#![allow(unsafe_code)] // macOS task_info FFI + MaybeUninit::assume_init for bench RSS metrics.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use spur_graph::temporal::{
    resolve_symbol_at, resolve_symbol_at_indexed, symbol_history, Resolution, TemporalIndex,
};
use spur_graph::{
    build_facts_for_paths, compute_graph_content_hash, current_manifest_version, ChangeKind,
    CommitArtifact, CommitIndexArtifact, EdgeEndpoint, GitPath, GraphIndexArtifact,
    GraphIndexHeader, RelationKind, RenamePrev, SnapshotKey, SymbolSnapshotArtifact,
    TemporalEdgeArtifact, WalkStrategy, WriteOptions, GRAPH_INDEX_VERSION_TEMPORAL,
};
use tempfile::TempDir;

const DEFAULT_FILE_COUNT: usize = 100_000;
const DEFAULT_CHANGE_SET: usize = 1_000;
const DEFAULT_DIRTY_MODS: usize = 100;
const BENCH_FILE_COUNT_ENV: &str = "SPUR_GRAPH_BENCH_FILES";
const BENCH_CHANGE_SET_ENV: &str = "SPUR_GRAPH_BENCH_CHANGE_SET";
const BENCH_DIRTY_MODS_ENV: &str = "SPUR_GRAPH_BENCH_DIRTY_MODS";
const BENCH_GIT_WALK_1K_COMMITS_ENV: &str = "SPUR_GRAPH_BENCH_GIT_WALK_1K_COMMITS";
const BENCH_GIT_WALK_20K_COMMITS_ENV: &str = "SPUR_GRAPH_BENCH_GIT_WALK_20K_COMMITS";
const HISTORY_WALK_ASSERT_MS_ENV: &str = "SPUR_GRAPH_HISTORY_WALK_ASSERT_MS";
const HISTORY_WALK_ASSERT_ITERATIONS_ENV: &str = "SPUR_GRAPH_HISTORY_WALK_ASSERT_ITERATIONS";
#[allow(dead_code)]
const SNAPSHOT_GROWTH_SMALL_COMMITS_ENV: &str = "SPUR_GRAPH_SNAPSHOT_GROWTH_SMALL_COMMITS";
#[allow(dead_code)]
const SNAPSHOT_GROWTH_LARGE_COMMITS_ENV: &str = "SPUR_GRAPH_SNAPSHOT_GROWTH_LARGE_COMMITS";
const QUICK_GIT_WALK_1K_COMMITS: usize = 100;
const QUICK_GIT_WALK_20K_COMMITS: usize = 250;
#[allow(dead_code)]
const SNAPSHOT_GROWTH_SMALL_COMMITS: usize = 50;
#[allow(dead_code)]
const SNAPSHOT_GROWTH_LARGE_COMMITS: usize = 500;
const HISTORY_WALK_SNAPSHOT_COUNT: usize = 50_000;
const HISTORY_WALK_TARGET_CHAIN: usize = 8;
const HISTORY_WALK_ASSERT_ITERATIONS: usize = 1_000;
const HISTORY_WALK_ASSERT_MS: usize = 250;
const RESOLVE_SYMBOL_AT_SNAPSHOT_COUNT: usize = 50_000;
const RESOLVE_SYMBOL_AT_TARGET_CHAIN: usize = 8;
const RESOLVE_SYMBOL_AT_ASSERT_ITERATIONS: usize = 10;
const RESOLVE_SYMBOL_AT_ASSERT_MIN_SPEEDUP: u128 = 5;
const SYNTHETIC_SYMBOL_COUNT: usize = 16;
const SYNTHETIC_RENAME_RATE: f32 = 0.04;
// T15 baseline, captured for the 20k synthetic merge fixture on 2026-05-21:
// peak RSS 1,250,000,000 bytes; artifact JSON 166,666,667 bytes.
// The hardening guard allows 1.2x drift: 1.5 GB RSS and 200 MB artifact JSON.
const FULL_WALK_20K_PEAK_RSS_BASELINE_BYTES: u64 = 1_250_000_000;
const FULL_WALK_20K_ARTIFACT_SIZE_BASELINE_BYTES: u64 = 166_666_667;
#[cfg(test)]
#[allow(dead_code)]
const SYNTHETIC_BUDGET_MERGE_DENSITY: f32 = 0.90;
const SYNTHETIC_START_TIME: i64 = 1_700_000_000;

#[derive(Clone, Copy, Debug, Default)]
struct PhaseTimes {
    total: Duration,
    ls_files: Duration,
    blake3: Duration,
    extraction: Duration,
}

impl PhaseTimes {
    fn checked_avg(self, iters: u64) -> Self {
        if iters == 0 {
            return self;
        }
        Self {
            total: self.total / iters as u32,
            ls_files: self.ls_files / iters as u32,
            blake3: self.blake3 / iters as u32,
            extraction: self.extraction / iters as u32,
        }
    }
}

impl std::ops::AddAssign for PhaseTimes {
    fn add_assign(&mut self, rhs: Self) {
        self.total += rhs.total;
        self.ls_files += rhs.ls_files;
        self.blake3 += rhs.blake3;
        self.extraction += rhs.extraction;
    }
}

#[derive(Clone, Debug)]
struct SummaryRow {
    scenario: &'static str,
    files: usize,
    changed_paths: usize,
    cache_mode: &'static str,
    phases: PhaseTimes,
}

#[derive(Clone, Debug)]
struct ContentEntry {
    path: String,
    content_oid: String,
    extractable: bool,
}

#[derive(Clone, Debug)]
struct FullWalkMeasurement {
    elapsed: Duration,
    symbol_snapshots: usize,
    temporal_edges: usize,
    commits: usize,
    artifact_bytes: u64,
    peak_rss_bytes: Option<u64>,
}

impl FullWalkMeasurement {
    #[allow(dead_code)]
    fn for_test(peak_rss_bytes: u64, artifact_bytes: u64) -> Self {
        Self {
            elapsed: Duration::ZERO,
            symbol_snapshots: 0,
            temporal_edges: 0,
            commits: 0,
            artifact_bytes,
            peak_rss_bytes: Some(peak_rss_bytes),
        }
    }
}

struct SyntheticRepo {
    _temp: TempDir,
    root: PathBuf,
    git_common_dir: PathBuf,
}

impl SyntheticRepo {
    fn new(file_count: usize, label: &str) -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join(label);
        fs::create_dir(&root).expect("create repo dir");
        run_git(&root, &["init"]);
        run_git(
            &root,
            &["config", "user.email", "spur-graph@example.invalid"],
        );
        run_git(&root, &["config", "user.name", "Spur Graph Benchmark"]);
        run_git(&root, &["config", "core.autocrlf", "false"]);
        run_git(&root, &["config", "gc.auto", "0"]);

        write_rust_files(&root, 0, file_count, 0);
        run_git(&root, &["add", "src"]);
        run_git(&root, &["commit", "-m", "baseline"]);

        let git_common_dir = git_stdout(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        );
        Self {
            _temp: temp,
            root,
            git_common_dir: PathBuf::from(git_common_dir.trim_end()),
        }
    }

    fn baseline(&self) -> BTreeMap<String, String> {
        let entries = discover_content_entries(&self.root)
            .expect("discover baseline content entries")
            .entries;
        entries
            .into_iter()
            .map(|entry| (entry.path, entry.content_oid))
            .collect()
    }

    fn commit_rewrites(&self, count: usize) {
        write_rust_files(&self.root, 0, count, 1);
        run_git(&self.root, &["add", "src"]);
        run_git(&self.root, &["commit", "-m", "rewrite change set"]);
    }

    fn dirty_rewrites(&self, count: usize) {
        write_rust_files(&self.root, 0, count, 2);
    }

    fn canonical_dir(&self) -> PathBuf {
        self.git_common_dir
            .join("spur-graph")
            .join("artifacts")
            .join(current_manifest_version())
    }

    fn clear_canonical(&self) {
        let dir = self.canonical_dir();
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("clear canonical benchmark dir");
        }
    }

    fn canonical_marker(&self, graph_hash: &str) -> PathBuf {
        self.canonical_dir().join(format!("{graph_hash}.json"))
    }

    fn ensure_marker(&self, graph_hash: &str) {
        let marker = self.canonical_marker(graph_hash);
        if let Some(parent) = marker.parent() {
            fs::create_dir_all(parent).expect("create canonical benchmark dir");
        }
        fs::write(marker, b"benchmark cache marker\n").expect("write canonical marker");
    }

    fn marker_exists(&self, graph_hash: &str) -> bool {
        self.canonical_marker(graph_hash).exists()
    }
}

struct DiscoveryResult {
    entries: Vec<ContentEntry>,
    ls_files: Duration,
}

#[derive(Clone, Debug)]
struct SyntheticSymbol {
    slot: usize,
    name: String,
    generation: usize,
    rename_generation: usize,
}

#[derive(Clone, Debug)]
struct SyntheticRng {
    state: u64,
}

struct FastImportCommit<'a> {
    branch: &'a str,
    mark: usize,
    parent: Option<usize>,
    merges: &'a [usize],
    summary: &'a str,
    lib_rs: &'a str,
    commit_index: usize,
}

impl SyntheticRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.state
    }

    fn usize(&mut self, upper: usize) -> usize {
        assert!(upper > 0, "upper bound must be non-zero");
        (self.next_u64() as usize) % upper
    }

    fn chance(&mut self, probability: f32) -> bool {
        let probability = probability.clamp(0.0, 1.0) as f64;
        let sample = ((self.next_u64() >> 11) as f64) / ((1_u64 << 53) as f64);
        sample < probability
    }
}

fn build_synthetic_repo(dir: &Path, n_commits: usize, merge_density: f32) -> Vec<String> {
    assert!(n_commits > 0, "synthetic repo must contain commits");
    fs::create_dir_all(dir).expect("create synthetic repo dir");
    run_git(dir, &["init"]);
    run_git(dir, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    run_git(dir, &["config", "user.email", "spur-graph@example.invalid"]);
    run_git(dir, &["config", "user.name", "Spur Graph Benchmark"]);
    run_git(dir, &["config", "core.autocrlf", "false"]);
    run_git(dir, &["config", "gc.auto", "0"]);

    let mut importer = Command::new("git")
        .args(["fast-import", "--quiet", "--date-format=raw"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn git fast-import");

    {
        let stdin = importer.stdin.take().expect("git fast-import stdin");
        let mut stream = BufWriter::new(stdin);
        let mut rng = SyntheticRng::new(0x005e_ed5e_ed14);
        let mut symbols = initial_synthetic_symbols();
        let mut commit_count = 0usize;
        let mut next_mark = 1usize;

        let baseline = synthetic_rust_source(&symbols);
        write_fast_import_commit(
            &mut stream,
            FastImportCommit {
                branch: "refs/heads/main",
                mark: next_mark,
                parent: None,
                merges: &[],
                summary: "synthetic baseline",
                lib_rs: &baseline,
                commit_index: commit_count,
            },
        );
        let mut main_mark = next_mark;
        next_mark += 1;
        commit_count += 1;

        while commit_count < n_commits {
            let can_merge = commit_count + 2 <= n_commits;
            if can_merge && rng.chance(merge_density) {
                let mut side_symbols = symbols.clone();
                mutate_synthetic_symbol(&mut side_symbols, &mut rng);
                let side_mark = next_mark;
                let side_ref = format!("refs/heads/spur-synthetic-side/{side_mark}");
                let side_source = synthetic_rust_source(&side_symbols);
                let side_summary = format!("synthetic side churn {side_mark}");
                write_fast_import_commit(
                    &mut stream,
                    FastImportCommit {
                        branch: &side_ref,
                        mark: side_mark,
                        parent: Some(main_mark),
                        merges: &[],
                        summary: &side_summary,
                        lib_rs: &side_source,
                        commit_index: commit_count,
                    },
                );
                next_mark += 1;
                commit_count += 1;

                let merge_mark = next_mark;
                let merge_summary = format!("merge synthetic side {side_mark}");
                write_fast_import_commit(
                    &mut stream,
                    FastImportCommit {
                        branch: "refs/heads/main",
                        mark: merge_mark,
                        parent: Some(main_mark),
                        merges: &[side_mark],
                        summary: &merge_summary,
                        lib_rs: &side_source,
                        commit_index: commit_count,
                    },
                );
                next_mark += 1;
                commit_count += 1;
                main_mark = merge_mark;
                symbols = side_symbols;
            } else {
                mutate_synthetic_symbol(&mut symbols, &mut rng);
                let source = synthetic_rust_source(&symbols);
                let summary = format!("synthetic churn {next_mark}");
                write_fast_import_commit(
                    &mut stream,
                    FastImportCommit {
                        branch: "refs/heads/main",
                        mark: next_mark,
                        parent: Some(main_mark),
                        merges: &[],
                        summary: &summary,
                        lib_rs: &source,
                        commit_index: commit_count,
                    },
                );
                main_mark = next_mark;
                next_mark += 1;
                commit_count += 1;
            }
        }

        stream.write_all(b"done\n").expect("finish fast-import");
        stream.flush().expect("flush fast-import stream");
    }

    let output = importer
        .wait_with_output()
        .expect("wait for git fast-import");
    assert!(
        output.status.success(),
        "git fast-import failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    run_git(dir, &["checkout", "-q", "-f", "main"]);
    let shas: Vec<_> = git_stdout(dir, &["rev-list", "--topo-order", "--reverse", "main"])
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    assert_eq!(shas.len(), n_commits, "synthetic commit count mismatch");
    shas
}

fn initial_synthetic_symbols() -> Vec<SyntheticSymbol> {
    (0..SYNTHETIC_SYMBOL_COUNT)
        .map(|slot| SyntheticSymbol {
            slot,
            name: format!("symbol_{slot:03}"),
            generation: 0,
            rename_generation: 0,
        })
        .collect()
}

fn mutate_synthetic_symbol(symbols: &mut [SyntheticSymbol], rng: &mut SyntheticRng) {
    let index = rng.usize(symbols.len());
    let symbol = &mut symbols[index];
    symbol.generation += 1;
    if rng.chance(SYNTHETIC_RENAME_RATE) {
        symbol.rename_generation += 1;
        symbol.name = format!("symbol_{:03}_r{:03}", symbol.slot, symbol.rename_generation);
    }
}

fn synthetic_rust_source(symbols: &[SyntheticSymbol]) -> String {
    let mut source = String::with_capacity(symbols.len() * 240);
    source.push_str("// Synthetic Rust fixture for spur-graph temporal walk benches.\n\n");
    for (index, symbol) in symbols.iter().enumerate() {
        let callee = &symbols[(index + 1) % symbols.len()].name;
        let rotate = (symbol.generation % 31) + 1;
        let salt = symbol.slot.wrapping_mul(97) + symbol.generation.wrapping_mul(31);
        source.push_str(&format!(
            "pub fn {}(input: usize) -> usize {{\n    let base = input.wrapping_add({}usize).rotate_left({});\n    let churn = base ^ {}usize;\n    if input == 0 {{\n        churn\n    }} else {{\n        churn.wrapping_add({}(input - 1))\n    }}\n}}\n\n",
            symbol.name,
            symbol.slot + symbol.generation,
            rotate,
            salt,
            callee
        ));
    }
    source
}

fn write_fast_import_commit<W: Write>(stream: &mut W, commit: FastImportCommit<'_>) {
    let timestamp = SYNTHETIC_START_TIME + commit.commit_index as i64;
    writeln!(stream, "commit {}", commit.branch).expect("write fast-import commit");
    writeln!(stream, "mark :{}", commit.mark).expect("write fast-import mark");
    writeln!(
        stream,
        "author Spur Synthetic <spur-graph@example.invalid> {timestamp} +0000"
    )
    .expect("write fast-import author");
    writeln!(
        stream,
        "committer Spur Synthetic <spur-graph@example.invalid> {timestamp} +0000"
    )
    .expect("write fast-import committer");
    write_fast_import_data(stream, commit.summary.as_bytes());
    if let Some(parent) = commit.parent {
        writeln!(stream, "from :{parent}").expect("write fast-import parent");
    }
    for merge in commit.merges {
        writeln!(stream, "merge :{merge}").expect("write fast-import merge parent");
    }
    writeln!(stream, "M 100644 inline src/lib.rs").expect("write fast-import file command");
    write_fast_import_data(stream, commit.lib_rs.as_bytes());
    writeln!(stream).expect("separate fast-import commit");
}

fn write_fast_import_data<W: Write>(stream: &mut W, bytes: &[u8]) {
    writeln!(stream, "data {}", bytes.len()).expect("write fast-import data header");
    stream.write_all(bytes).expect("write fast-import data");
    stream
        .write_all(b"\n")
        .expect("terminate fast-import data block");
}

fn benchmark_incremental(c: &mut Criterion) {
    if !criterion_filter_allows(&[
        "incremental",
        "clean cold",
        "clean warm",
        "clean change set",
        "dirty unstaged mods",
    ]) {
        return;
    }

    let file_count = env_usize(BENCH_FILE_COUNT_ENV, DEFAULT_FILE_COUNT);
    let change_set = env_usize(BENCH_CHANGE_SET_ENV, DEFAULT_CHANGE_SET).min(file_count);
    let dirty_mods = env_usize(BENCH_DIRTY_MODS_ENV, DEFAULT_DIRTY_MODS).min(file_count);

    eprintln!(
        "spur-graph incremental benchmark fixture: files={file_count}, change_set={change_set}, dirty_mods={dirty_mods}"
    );

    let clean = SyntheticRepo::new(file_count, "clean");
    let clean_hash = graph_hash_for_setup(&clean.root);
    clean.ensure_marker(&clean_hash);

    let changed = SyntheticRepo::new(file_count, "changed");
    let changed_baseline = changed.baseline();
    changed.commit_rewrites(change_set);

    let dirty = SyntheticRepo::new(file_count, "dirty");
    let dirty_baseline = dirty.baseline();
    dirty.dirty_rewrites(dirty_mods);

    let rows: Arc<Mutex<Vec<SummaryRow>>> = Arc::default();
    let mut group = c.benchmark_group("incremental");

    bench_scenario(
        &mut group,
        rows.clone(),
        SummaryRow {
            scenario: "clean cold",
            files: file_count,
            changed_paths: file_count,
            cache_mode: "miss",
            phases: PhaseTimes::default(),
        },
        || clean.clear_canonical(),
        || measure_build(&clean, None, true),
    );

    bench_scenario(
        &mut group,
        rows.clone(),
        SummaryRow {
            scenario: "clean warm",
            files: file_count,
            changed_paths: 0,
            cache_mode: "hit",
            phases: PhaseTimes::default(),
        },
        || clean.ensure_marker(&clean_hash),
        || measure_build(&clean, None, false),
    );

    bench_scenario(
        &mut group,
        rows.clone(),
        SummaryRow {
            scenario: "clean change set",
            files: file_count,
            changed_paths: change_set,
            cache_mode: "miss",
            phases: PhaseTimes::default(),
        },
        || changed.clear_canonical(),
        || measure_build(&changed, Some(&changed_baseline), true),
    );

    bench_scenario(
        &mut group,
        rows.clone(),
        SummaryRow {
            scenario: "dirty unstaged mods",
            files: file_count,
            changed_paths: dirty_mods,
            cache_mode: "miss",
            phases: PhaseTimes::default(),
        },
        || dirty.clear_canonical(),
        || measure_build(&dirty, Some(&dirty_baseline), true),
    );

    group.finish();
    print_summary(&rows.lock().expect("summary rows").clone());
}

fn bench_full_walk_1k(c: &mut Criterion) {
    const BENCH_NAME: &str = "git_walk full 1k linear";
    const BENCH_FILTER: &str = "bench_full_walk_1k";
    if !criterion_filter_allows(&[BENCH_FILTER, BENCH_NAME]) {
        return;
    }

    let dir = tempfile::TempDir::new().unwrap();
    let commit_count = synthetic_bench_commits(
        BENCH_GIT_WALK_1K_COMMITS_ENV,
        1_000,
        QUICK_GIT_WALK_1K_COMMITS,
    );
    eprintln!("{BENCH_NAME} synthetic commits={commit_count}");
    let shas = build_synthetic_repo(dir.path(), commit_count, 0.0);
    let mut group = c.benchmark_group(BENCH_FILTER);
    group.throughput(Throughput::Elements(commit_count as u64));
    for jobs in [1, 2, 4, 8] {
        let config = spur_graph::git_walk::GitWalkConfig {
            temporal_jobs: NonZeroUsize::new(jobs).unwrap(),
            ..spur_graph::git_walk::GitWalkConfig::default()
        };
        group.bench_with_input(
            BenchmarkId::new(BENCH_NAME, format!("jobs={jobs}")),
            &config,
            |b, config| {
                b.iter(|| {
                    let (graph, commits) =
                        spur_graph::git_walk::run_full_walk_into(dir.path(), config, None, None)
                            .unwrap();
                    black_box((
                        graph.symbol_snapshots.len(),
                        graph.temporal_edges.len(),
                        commits.commits.len(),
                        shas.len(),
                    ));
                });
            },
        );
    }
    group.finish();
}

fn bench_full_walk_20k_merges(c: &mut Criterion) {
    const BENCH_NAME: &str = "git_walk full 20k merges";
    if !criterion_filter_allows(&[BENCH_NAME]) {
        return;
    }

    let dir = tempfile::TempDir::new().unwrap();
    let commit_count = synthetic_bench_commits(
        BENCH_GIT_WALK_20K_COMMITS_ENV,
        20_000,
        QUICK_GIT_WALK_20K_COMMITS,
    );
    eprintln!("{BENCH_NAME} synthetic commits={commit_count}");
    let shas = build_synthetic_repo(dir.path(), commit_count, 0.30);
    let artifact_dir = tempfile::TempDir::new().unwrap();
    let artifact_base = artifact_dir.path().join("graph");
    let latest_measurement = Arc::new(Mutex::new(None::<FullWalkMeasurement>));
    c.bench_function(BENCH_NAME, |b| {
        let latest_measurement = latest_measurement.clone();
        b.iter(|| {
            let metrics = measure_full_walk_once(dir.path(), &artifact_base).unwrap();
            assert_full_walk_20k_budget(&metrics);
            black_box((
                metrics.symbol_snapshots,
                metrics.temporal_edges,
                metrics.commits,
                shas.len(),
                metrics.artifact_bytes,
                metrics.peak_rss_bytes,
            ));
            *latest_measurement.lock().expect("full walk measurement") = Some(metrics);
        });
    });
    let metrics = latest_measurement
        .lock()
        .expect("full walk measurement")
        .clone();
    if let Some(metrics) = metrics {
        print_full_walk_measurement(BENCH_NAME, &metrics);
    }
}

fn bench_history_walk_50k_snapshots(c: &mut Criterion) {
    const BENCH_NAME: &str = "history walk 50k snapshots";
    if !criterion_filter_allows(&[BENCH_NAME, "history"]) {
        return;
    }

    assert_history_walk_budget();

    let (graph, commits, target_symbol) =
        synthetic_history_artifact(HISTORY_WALK_SNAPSHOT_COUNT, HISTORY_WALK_TARGET_CHAIN);
    let index = TemporalIndex::new(Arc::new(graph));
    c.bench_function(BENCH_NAME, |b| {
        b.iter(|| {
            black_box(symbol_history(
                black_box(&index),
                black_box(&commits),
                black_box(&target_symbol),
            ))
        });
    });
}

fn bench_resolve_symbol_at(c: &mut Criterion) {
    const BENCH_NAME: &str = "resolve_symbol_at raw vs indexed";
    if !criterion_filter_allows(&[BENCH_NAME, "resolve_symbol_at"]) {
        return;
    }

    assert_resolve_symbol_at_indexed_speedup();

    let (graph, commits, symbol, anchor, target) = synthetic_resolve_artifact(
        RESOLVE_SYMBOL_AT_SNAPSHOT_COUNT,
        RESOLVE_SYMBOL_AT_TARGET_CHAIN,
    );
    let index = TemporalIndex::new(Arc::new(graph.clone()));
    let mut group = c.benchmark_group("resolve_symbol_at");
    group.bench_function("raw", |b| {
        b.iter(|| {
            black_box(resolve_symbol_at(
                black_box(&graph),
                black_box(&commits),
                black_box(&symbol),
                black_box(&anchor),
                black_box(&target),
            ))
        });
    });
    group.bench_function("indexed", |b| {
        b.iter(|| {
            black_box(resolve_symbol_at_indexed(
                black_box(&index),
                black_box(&commits),
                black_box(&symbol),
                black_box(&anchor),
                black_box(&target),
            ))
        });
    });
    group.finish();
}

fn assert_history_walk_budget() {
    let iterations = env_usize(
        HISTORY_WALK_ASSERT_ITERATIONS_ENV,
        HISTORY_WALK_ASSERT_ITERATIONS,
    );
    let max_ms = env_usize(HISTORY_WALK_ASSERT_MS_ENV, HISTORY_WALK_ASSERT_MS);
    let (graph, commits, target_symbol) =
        synthetic_history_artifact(HISTORY_WALK_SNAPSHOT_COUNT, HISTORY_WALK_TARGET_CHAIN);
    let index = TemporalIndex::new(Arc::new(graph));

    let start = Instant::now();
    let mut total_events = 0usize;
    for _ in 0..iterations {
        total_events += symbol_history(&index, &commits, &target_symbol).len();
    }
    let elapsed = start.elapsed();
    black_box(total_events);

    assert_eq!(
        total_events,
        iterations * HISTORY_WALK_TARGET_CHAIN,
        "history walk returned an unexpected number of events"
    );
    assert!(
        elapsed <= Duration::from_millis(max_ms as u64),
        "history walk budget exceeded: {} for {iterations} walks over {HISTORY_WALK_SNAPSHOT_COUNT} snapshots, budget={} (set {HISTORY_WALK_ASSERT_MS_ENV} to tune for CI)",
        fmt_duration(elapsed),
        fmt_duration(Duration::from_millis(max_ms as u64))
    );
}

fn assert_resolve_symbol_at_indexed_speedup() {
    let (graph, commits, symbol, anchor, target) = synthetic_resolve_artifact(
        RESOLVE_SYMBOL_AT_SNAPSHOT_COUNT,
        RESOLVE_SYMBOL_AT_TARGET_CHAIN,
    );
    let index = TemporalIndex::new(Arc::new(graph.clone()));

    let raw_start = Instant::now();
    let mut raw_result = None;
    for _ in 0..RESOLVE_SYMBOL_AT_ASSERT_ITERATIONS {
        raw_result = Some(resolve_symbol_at(
            &graph, &commits, &symbol, &anchor, &target,
        ));
    }
    let raw_elapsed = raw_start.elapsed();

    let indexed_start = Instant::now();
    let mut indexed_result = None;
    for _ in 0..RESOLVE_SYMBOL_AT_ASSERT_ITERATIONS {
        indexed_result = Some(resolve_symbol_at_indexed(
            &index, &commits, &symbol, &anchor, &target,
        ));
    }
    let indexed_elapsed = indexed_start.elapsed();

    assert_eq!(
        raw_result, indexed_result,
        "raw and indexed resolve_symbol_at must return identical results"
    );
    assert!(
        matches!(indexed_result, Some(Resolution::Found { .. })),
        "synthetic resolve fixture should resolve the rename chain"
    );

    let raw_nanos = raw_elapsed.as_nanos();
    let indexed_nanos = indexed_elapsed.as_nanos().max(1);
    assert!(
        raw_nanos >= indexed_nanos * RESOLVE_SYMBOL_AT_ASSERT_MIN_SPEEDUP,
        "resolve_symbol_at_indexed speedup below {RESOLVE_SYMBOL_AT_ASSERT_MIN_SPEEDUP}x: raw={} indexed={} ratio={:.2}x",
        fmt_duration(raw_elapsed),
        fmt_duration(indexed_elapsed),
        raw_elapsed.as_secs_f64() / indexed_elapsed.as_secs_f64().max(f64::EPSILON)
    );
}

fn measure_full_walk_once(
    worktree: &Path,
    artifact_base: &Path,
) -> anyhow::Result<FullWalkMeasurement> {
    let rss_before = peak_rss_bytes();
    let start = Instant::now();
    let (graph, commits) = spur_graph::git_walk::run_full_walk_into(
        worktree,
        &spur_graph::git_walk::GitWalkConfig::default(),
        None,
        None,
    )?;
    let artifact_dir = spur_graph::store::write_artifact_parquet(
        &graph,
        artifact_base,
        WriteOptions::default(),
        Vec::new(),
    )?;
    spur_graph::store::write_current_pointer(worktree, &artifact_dir)?;
    let artifact_bytes = artifact_dir_size(&artifact_dir)?;
    let peak_rss_bytes = [rss_before, peak_rss_bytes()].into_iter().flatten().max();

    Ok(FullWalkMeasurement {
        elapsed: start.elapsed(),
        symbol_snapshots: graph.symbol_snapshots.len(),
        temporal_edges: graph.temporal_edges.len(),
        commits: commits.commits.len(),
        artifact_bytes,
        peak_rss_bytes,
    })
}

fn artifact_dir_size(path: &Path) -> anyhow::Result<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total = total.saturating_add(artifact_dir_size(&entry.path())?);
        } else if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

fn assert_full_walk_20k_budget(metrics: &FullWalkMeasurement) {
    let violations = full_walk_20k_budget_violations(metrics);
    assert!(
        violations.is_empty(),
        "git_walk full 20k merges budget exceeded:\n{}",
        violations.join("\n")
    );
}

fn full_walk_20k_budget_violations(metrics: &FullWalkMeasurement) -> Vec<String> {
    let mut violations = Vec::new();
    let peak_rss_limit = budget_limit_bytes(FULL_WALK_20K_PEAK_RSS_BASELINE_BYTES);
    let artifact_limit = budget_limit_bytes(FULL_WALK_20K_ARTIFACT_SIZE_BASELINE_BYTES);

    match metrics.peak_rss_bytes {
        Some(peak_rss_bytes) if peak_rss_bytes > peak_rss_limit => violations.push(format!(
            "peak RSS {} > 1.2x baseline {} (limit {})",
            fmt_bytes(peak_rss_bytes),
            fmt_bytes(FULL_WALK_20K_PEAK_RSS_BASELINE_BYTES),
            fmt_bytes(peak_rss_limit)
        )),
        Some(_) => {}
        None => violations.push("peak RSS unavailable on this platform".to_owned()),
    }

    if metrics.artifact_bytes > artifact_limit {
        violations.push(format!(
            "artifact {} > 1.2x JSON baseline {} (limit {})",
            fmt_bytes(metrics.artifact_bytes),
            fmt_bytes(FULL_WALK_20K_ARTIFACT_SIZE_BASELINE_BYTES),
            fmt_bytes(artifact_limit)
        ));
    }

    violations
}

fn budget_limit_bytes(baseline_bytes: u64) -> u64 {
    baseline_bytes.saturating_mul(12) / 10
}

fn print_full_walk_measurement(bench_name: &str, metrics: &FullWalkMeasurement) {
    eprintln!(
        "{bench_name} metrics: elapsed={} commits={} snapshots={} temporal_edges={} artifact={} peak_rss={}",
        fmt_duration(metrics.elapsed),
        metrics.commits,
        metrics.symbol_snapshots,
        metrics.temporal_edges,
        fmt_bytes(metrics.artifact_bytes),
        metrics
            .peak_rss_bytes
            .map(fmt_bytes)
            .unwrap_or_else(|| "unavailable".to_owned())
    );
}

#[cfg(target_os = "linux")]
fn peak_rss_bytes() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    proc_status_bytes(&status, "VmHWM:").or_else(|| proc_status_bytes(&status, "VmRSS:"))
}

#[cfg(target_os = "linux")]
fn proc_status_bytes(status: &str, label: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with(label))?;
    let mut parts = line.split_whitespace();
    parts.next()?;
    let value = parts.next()?.parse::<u64>().ok()?;
    let unit = parts.next().unwrap_or("kB");
    match unit {
        "kB" => Some(value.saturating_mul(1024)),
        "mB" | "MB" => Some(value.saturating_mul(1024 * 1024)),
        "B" => Some(value),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn peak_rss_bytes() -> Option<u64> {
    type KernReturn = i32;
    type MachMsgTypeNumber = u32;
    type MachPort = u32;
    type TaskFlavor = i32;

    const KERN_SUCCESS: KernReturn = 0;
    const MACH_TASK_BASIC_INFO: TaskFlavor = 20;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct TimeValue {
        seconds: i32,
        microseconds: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MachTaskBasicInfo {
        virtual_size: u64,
        resident_size: u64,
        resident_size_max: u64,
        user_time: TimeValue,
        system_time: TimeValue,
        policy: i32,
        suspend_count: i32,
    }

    unsafe extern "C" {
        fn mach_task_self() -> MachPort;
        fn task_info(
            target_task: MachPort,
            flavor: TaskFlavor,
            task_info_out: *mut i32,
            task_info_out_count: *mut MachMsgTypeNumber,
        ) -> KernReturn;
    }

    let mut info = std::mem::MaybeUninit::<MachTaskBasicInfo>::zeroed();
    let mut count = (std::mem::size_of::<MachTaskBasicInfo>() / std::mem::size_of::<i32>())
        as MachMsgTypeNumber;
    let result = unsafe {
        task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            info.as_mut_ptr().cast::<i32>(),
            &mut count,
        )
    };
    if result == KERN_SUCCESS {
        Some(unsafe { info.assume_init() }.resident_size_max)
    } else {
        None
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn peak_rss_bytes() -> Option<u64> {
    None
}

fn synthetic_history_artifact(
    snapshot_count: usize,
    target_chain_len: usize,
) -> (GraphIndexArtifact, CommitIndexArtifact, String) {
    assert!(target_chain_len > 0);
    assert!(snapshot_count >= target_chain_len);

    let mut graph = GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.into(),
            content_hash_blake3: None,
        },
        manifest_version: String::new(),
        graph_content_hash: String::new(),
        file_manifests: vec![],
        files: vec![],
        file_node_ids: vec![],
        symbols: vec![],
        symbol_node_ids: vec![],
        edges: vec![],
        tombstones: vec![],
        diagnostics: vec![],
        commits: vec![],
        symbol_snapshots: Vec::with_capacity(snapshot_count),
        temporal_edges: Vec::with_capacity(snapshot_count),
    };
    let mut commits = Vec::with_capacity(snapshot_count);
    let target_symbol = "history-target".to_owned();
    let mut previous_sha: Option<String> = None;

    for index in 0..snapshot_count {
        let sha = format!("history-{index:05}");
        let commit = CommitArtifact {
            sha: sha.clone(),
            parents: previous_sha.iter().cloned().collect(),
            author_time: SYNTHETIC_START_TIME + index as i64,
            author_name: String::new(),
            author_email: String::new(),
            summary: format!("history snapshot {index}"),
        };
        graph.commits.push(commit.clone());
        commits.push(commit);

        let stable_symbol_id = if index < target_chain_len {
            target_symbol.clone()
        } else {
            format!("unrelated-symbol-{index:05}")
        };
        let key = SnapshotKey {
            stable_symbol_id: stable_symbol_id.clone(),
            commit: sha.clone(),
        };
        graph.symbol_snapshots.push(SymbolSnapshotArtifact {
            key: key.clone(),
            file_path: GitPath::from_bytes(format!("src/{stable_symbol_id}.rs").into_bytes()),
            entity_name: stable_symbol_id.clone(),
            symbol_kind: "function".into(),
            enclosing_scope: None,
            byte_range: [0, 10],
            line_range: [1, 1],
            anchor_hash: format!("anchor-{index:05}"),
            tokens: vec![],
        });
        graph.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit { sha: sha.clone() },
            target: EdgeEndpoint::Snapshot { key },
            relation: RelationKind::Touches,
            parent: previous_sha.clone(),
            change_kind: Some(if index == 0 {
                ChangeKind::Added
            } else {
                ChangeKind::Modified
            }),
        });

        previous_sha = Some(sha);
    }

    let commit_index = CommitIndexArtifact {
        schema_version: 1,
        commits,
        refs: [("main".into(), previous_sha.unwrap())].into(),
        indexed_at: "2026-05-21T00:00:00Z".into(),
        walk_strategy: WalkStrategy::Reachable,
    };
    (graph, commit_index, target_symbol)
}

fn synthetic_resolve_artifact(
    snapshot_count: usize,
    target_chain_len: usize,
) -> (
    GraphIndexArtifact,
    CommitIndexArtifact,
    String,
    String,
    String,
) {
    assert!(target_chain_len > 1);
    assert!(snapshot_count >= target_chain_len);

    let mut graph = GraphIndexArtifact {
        rust_evidence: None,
        macro_invocations: None,
        header: GraphIndexHeader {
            graph_index_version: GRAPH_INDEX_VERSION_TEMPORAL.into(),
            content_hash_blake3: None,
        },
        manifest_version: String::new(),
        graph_content_hash: String::new(),
        file_manifests: vec![],
        files: vec![],
        file_node_ids: vec![],
        symbols: vec![],
        symbol_node_ids: vec![],
        edges: vec![],
        tombstones: vec![],
        diagnostics: vec![],
        commits: vec![],
        symbol_snapshots: Vec::with_capacity(snapshot_count),
        temporal_edges: Vec::with_capacity(snapshot_count + target_chain_len),
    };
    let mut commits = Vec::with_capacity(snapshot_count);
    let initial_symbol = "resolve-target-r000".to_owned();
    let mut previous_sha: Option<String> = None;
    let mut previous_target_key: Option<SnapshotKey> = None;
    let mut anchor = String::new();
    let mut target = String::new();

    for index in 0..snapshot_count {
        let sha = format!("resolve-{index:05}");
        let commit = CommitArtifact {
            sha: sha.clone(),
            parents: previous_sha.iter().cloned().collect(),
            author_time: SYNTHETIC_START_TIME + index as i64,
            author_name: String::new(),
            author_email: String::new(),
            summary: format!("resolve snapshot {index}"),
        };
        graph.commits.push(commit.clone());
        commits.push(commit);

        let stable_symbol_id = if index < target_chain_len {
            format!("resolve-target-r{index:03}")
        } else {
            format!("resolve-unrelated-{index:05}")
        };
        let key = SnapshotKey {
            stable_symbol_id: stable_symbol_id.clone(),
            commit: sha.clone(),
        };
        graph.symbol_snapshots.push(SymbolSnapshotArtifact {
            key: key.clone(),
            file_path: GitPath::from_bytes(format!("src/{stable_symbol_id}.rs").into_bytes()),
            entity_name: stable_symbol_id.clone(),
            symbol_kind: "function".into(),
            enclosing_scope: None,
            byte_range: [0, 10],
            line_range: [1, 1],
            anchor_hash: format!("resolve-anchor-{index:05}"),
            tokens: vec![],
        });

        let rename_prev = (index > 0 && index < target_chain_len)
            .then(|| previous_target_key.clone().expect("previous target key"));
        let change_kind = rename_prev
            .clone()
            .map(|prev| ChangeKind::RenamedFrom(RenamePrev::Symbol(prev)))
            .unwrap_or(ChangeKind::Added);
        graph.temporal_edges.push(TemporalEdgeArtifact {
            source: EdgeEndpoint::Commit { sha: sha.clone() },
            target: EdgeEndpoint::Snapshot { key: key.clone() },
            relation: RelationKind::Touches,
            parent: previous_sha.clone(),
            change_kind: Some(change_kind),
        });
        if let Some(prev) = rename_prev {
            graph.temporal_edges.push(TemporalEdgeArtifact {
                source: EdgeEndpoint::Snapshot { key: prev.clone() },
                target: EdgeEndpoint::Snapshot { key: key.clone() },
                relation: RelationKind::Touches,
                parent: previous_sha.clone(),
                change_kind: Some(ChangeKind::RenamedFrom(RenamePrev::Symbol(prev))),
            });
        }

        if index == 0 {
            anchor = sha.clone();
        }
        if index < target_chain_len {
            previous_target_key = Some(key);
        }
        target = sha.clone();
        previous_sha = Some(sha);
    }

    let commit_index = CommitIndexArtifact {
        schema_version: 1,
        commits,
        refs: [("main".into(), target.clone())].into(),
        indexed_at: "2026-05-21T00:00:00Z".into(),
        walk_strategy: WalkStrategy::Reachable,
    };
    (graph, commit_index, initial_symbol, anchor, target)
}

fn synthetic_bench_commits(env_name: &str, default: usize, quick_default: usize) -> usize {
    let default = if criterion_quick_mode() {
        quick_default
    } else {
        default
    };
    env_usize(env_name, default)
}

fn criterion_quick_mode() -> bool {
    env::args().any(|arg| arg == "--quick")
}

fn criterion_filter_allows(candidates: &[&str]) -> bool {
    let filters: Vec<_> = env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    filters.is_empty()
        || filters.iter().any(|filter| {
            candidates
                .iter()
                .any(|candidate| candidate.contains(filter))
        })
}

fn bench_scenario<Setup, Measure>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    rows: Arc<Mutex<Vec<SummaryRow>>>,
    row: SummaryRow,
    mut setup: Setup,
    mut measure: Measure,
) where
    Setup: FnMut(),
    Measure: FnMut() -> anyhow::Result<PhaseTimes>,
{
    group.bench_with_input(BenchmarkId::new(row.scenario, row.files), &row, |b, row| {
        b.iter_custom(|iters| {
            let mut total = PhaseTimes::default();
            for _ in 0..iters {
                setup();
                let phases = measure().expect("measure spur-graph incremental scenario");
                total += phases;
            }
            let averaged = total.checked_avg(iters);
            upsert_summary(
                &rows,
                SummaryRow {
                    phases: averaged,
                    ..row.clone()
                },
            );
            total.total
        });
    });
}

fn measure_build(
    repo: &SyntheticRepo,
    baseline: Option<&BTreeMap<String, String>>,
    populate_cache_on_miss: bool,
) -> anyhow::Result<PhaseTimes> {
    let total_start = Instant::now();
    let discovery = discover_content_entries(&repo.root)?;

    let blake3_start = Instant::now();
    let graph_hash = compute_graph_content_hash(
        discovery
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.content_oid.as_str())),
    );
    let blake3 = blake3_start.elapsed();

    let cache_hit = repo.marker_exists(&graph_hash);
    let mut extraction = Duration::ZERO;
    if !cache_hit {
        let extract_paths = changed_extract_paths(&repo.root, &discovery.entries, baseline);
        let extraction_start = Instant::now();
        let facts = build_facts_for_paths(&repo.root, &extract_paths)?;
        black_box(facts.nodes.len() + facts.edges.len() + facts.spans.len());
        extraction = extraction_start.elapsed();
        if populate_cache_on_miss {
            repo.ensure_marker(&graph_hash);
        }
    }

    Ok(PhaseTimes {
        total: total_start.elapsed(),
        ls_files: discovery.ls_files,
        blake3,
        extraction,
    })
}

fn discover_content_entries(root: &Path) -> anyhow::Result<DiscoveryResult> {
    let dirty_entries = spur_graph::git::status_dirty_paths(root)?;
    let dirty_paths: BTreeMap<String, spur_graph::DirtyEntry> = dirty_entries
        .into_iter()
        .filter(|entry| is_rust_path(&entry.path))
        .map(|entry| (entry.path.clone(), entry))
        .collect();

    let ls_start = Instant::now();
    let tracked = spur_graph::git::ls_files_with_oids(root)?;
    let ls_files = ls_start.elapsed();

    let mut entries = BTreeMap::new();
    for tracked in tracked {
        if !is_rust_path(&tracked.path) {
            continue;
        }
        let content_oid = if tracked.is_gitlink {
            tracked.content_oid
        } else if dirty_paths.contains_key(&tracked.path) {
            match read_worktree_content_oid(root, &tracked.path)? {
                Some(content_oid) => content_oid,
                None => continue,
            }
        } else {
            tracked.content_oid
        };
        entries.insert(
            tracked.path.clone(),
            ContentEntry {
                path: tracked.path,
                content_oid,
                extractable: !tracked.is_gitlink,
            },
        );
    }

    for dirty in dirty_paths.values() {
        if entries.contains_key(&dirty.path) {
            continue;
        }
        let Some(content_oid) = read_worktree_content_oid(root, &dirty.path)? else {
            continue;
        };
        entries.insert(
            dirty.path.clone(),
            ContentEntry {
                path: dirty.path.clone(),
                content_oid,
                extractable: true,
            },
        );
    }

    Ok(DiscoveryResult {
        entries: entries.into_values().collect(),
        ls_files,
    })
}

fn changed_extract_paths(
    root: &Path,
    entries: &[ContentEntry],
    baseline: Option<&BTreeMap<String, String>>,
) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|entry| entry.extractable)
        .filter(|entry| {
            baseline.is_none_or(|baseline| {
                baseline
                    .get(&entry.path)
                    .is_none_or(|content_oid| content_oid != &entry.content_oid)
            })
        })
        .map(|entry| root.join(&entry.path))
        .collect()
}

fn graph_hash_for_setup(root: &Path) -> String {
    let entries = discover_content_entries(root)
        .expect("discover setup content entries")
        .entries;
    compute_graph_content_hash(
        entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.content_oid.as_str())),
    )
}

fn read_worktree_content_oid(root: &Path, path: &str) -> anyhow::Result<Option<String>> {
    match fs::read(root.join(path)) {
        Ok(bytes) => Ok(Some(spur_graph::git_blob_oid(&bytes))),
        Err(err) if matches!(err.kind(), std::io::ErrorKind::NotFound) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn write_rust_files(root: &Path, start: usize, count: usize, generation: usize) {
    for index in start..start + count {
        let path = rust_file_path(root, index);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture dir");
        }
        fs::write(path, rust_source(index, generation)).expect("write rust fixture");
    }
}

fn rust_file_path(root: &Path, index: usize) -> PathBuf {
    root.join("src")
        .join(format!("mod_{:03}", index / 1_000))
        .join(format!("file_{index:06}.rs"))
}

fn rust_source(index: usize, generation: usize) -> String {
    format!("pub fn f_{index:06}() -> usize {{\n    {index}usize + {generation}usize\n}}\n")
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn is_rust_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
}

fn upsert_summary(rows: &Arc<Mutex<Vec<SummaryRow>>>, row: SummaryRow) {
    let mut rows = rows.lock().expect("summary rows");
    if let Some(existing) = rows
        .iter_mut()
        .find(|existing| existing.scenario == row.scenario)
    {
        *existing = row;
    } else {
        rows.push(row);
    }
}

fn print_summary(rows: &[SummaryRow]) {
    eprintln!();
    eprintln!("spur-graph incremental benchmark summary");
    eprintln!("| scenario | files | changed | cache | total | ls-files | blake3 | extraction |");
    eprintln!("|---|---:|---:|---|---:|---:|---:|---:|");
    for row in rows {
        eprintln!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            row.scenario,
            row.files,
            row.changed_paths,
            row.cache_mode,
            fmt_duration(row.phases.total),
            fmt_duration(row.phases.ls_files),
            fmt_duration(row.phases.blake3),
            fmt_duration(row.phases.extraction)
        );
    }
}

fn fmt_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();
    if nanos >= 1_000_000_000 {
        format!("{:.3}s", duration.as_secs_f64())
    } else if nanos >= 1_000_000 {
        format!("{:.3}ms", nanos as f64 / 1_000_000.0)
    } else if nanos >= 1_000 {
        format!("{:.3}us", nanos as f64 / 1_000.0)
    } else {
        format!("{nanos}ns")
    }
}

fn fmt_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.2}GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.2}MiB", bytes / MIB)
    } else {
        format!("{}B", bytes as u64)
    }
}

fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|err| panic!("failed to run git {args:?}: {err}"));
    assert!(
        output.status.success(),
        "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|err| panic!("failed to run git {args:?}: {err}"));
    assert!(
        output.status.success(),
        "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git stdout UTF-8")
}

#[test]
fn snapshot_growth_budget() {
    let small_commits = env_usize(
        SNAPSHOT_GROWTH_SMALL_COMMITS_ENV,
        SNAPSHOT_GROWTH_SMALL_COMMITS,
    );
    let large_commits = env_usize(
        SNAPSHOT_GROWTH_LARGE_COMMITS_ENV,
        SNAPSHOT_GROWTH_LARGE_COMMITS,
    );
    assert!(
        large_commits > small_commits,
        "snapshot growth budget requires large fixture > small fixture"
    );

    let d1 = tempfile::TempDir::new().unwrap();
    build_synthetic_repo(d1.path(), small_commits, SYNTHETIC_BUDGET_MERGE_DENSITY);
    let budget_config = snapshot_growth_budget_config();
    let (g1, _) =
        spur_graph::git_walk::run_full_walk_into(d1.path(), &budget_config, None, None).unwrap();

    let d2 = tempfile::TempDir::new().unwrap();
    build_synthetic_repo(d2.path(), large_commits, SYNTHETIC_BUDGET_MERGE_DENSITY);
    let (g2, _) =
        spur_graph::git_walk::run_full_walk_into(d2.path(), &budget_config, None, None).unwrap();

    let ratio = g2.symbol_snapshots.len() as f64 / g1.symbol_snapshots.len() as f64;
    let commit_ratio = large_commits as f64 / small_commits as f64;
    eprintln!(
        "snapshot_growth_budget: small({small_commits})={} large({large_commits})={} ratio={ratio:.3}",
        g1.symbol_snapshots.len(),
        g2.symbol_snapshots.len()
    );
    assert!(
        ratio <= 1.5 * commit_ratio,
        "snapshot growth {ratio} > 1.5x/{commit_ratio:.1}x budget; needs sharded persistence before merge"
    );
}

#[allow(dead_code)]
fn snapshot_growth_budget_config() -> spur_graph::git_walk::GitWalkConfig {
    spur_graph::git_walk::GitWalkConfig {
        walk_strategy: spur_graph::WalkStrategy::Reachable,
        ..spur_graph::git_walk::GitWalkConfig::default()
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn full_walk_20k_budget_rejects_values_above_one_point_two_baseline() {
        let mut metrics = FullWalkMeasurement::for_test(
            FULL_WALK_20K_PEAK_RSS_BASELINE_BYTES * 12 / 10,
            FULL_WALK_20K_ARTIFACT_SIZE_BASELINE_BYTES * 12 / 10,
        );
        assert!(full_walk_20k_budget_violations(&metrics).is_empty());

        metrics.peak_rss_bytes = Some(FULL_WALK_20K_PEAK_RSS_BASELINE_BYTES * 12 / 10 + 1);
        let violations = full_walk_20k_budget_violations(&metrics);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains("peak RSS")),
            "expected peak RSS violation, got {violations:?}"
        );

        metrics.peak_rss_bytes = Some(FULL_WALK_20K_PEAK_RSS_BASELINE_BYTES);
        metrics.artifact_bytes = FULL_WALK_20K_ARTIFACT_SIZE_BASELINE_BYTES * 12 / 10 + 1;
        let violations = full_walk_20k_budget_violations(&metrics);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains("artifact")),
            "expected artifact violation, got {violations:?}"
        );
    }

    #[test]
    fn full_walk_measurement_reports_artifact_size_and_peak_rss() {
        let dir = tempfile::TempDir::new().unwrap();
        build_synthetic_repo(dir.path(), 12, 0.30);
        let artifact_base = dir.path().join(".spur/bench-artifacts");

        let metrics = measure_full_walk_once(dir.path(), &artifact_base).unwrap();

        assert!(metrics.artifact_bytes > 0);
        assert!(artifact_base.exists());
        assert!(dir.path().join(".spur/graph/CURRENT").exists());
        assert!(
            metrics.peak_rss_bytes.is_some_and(|bytes| bytes > 0),
            "expected RSS measurement on supported benchmark platforms"
        );
    }

    #[test]
    fn snapshot_growth_budget_config_walks_reachable_dag() {
        assert_eq!(
            snapshot_growth_budget_config().walk_strategy,
            spur_graph::WalkStrategy::Reachable
        );
    }
}

criterion_group!(
    benches,
    benchmark_incremental,
    bench_full_walk_1k,
    bench_full_walk_20k_merges,
    bench_history_walk_50k_snapshots,
    bench_resolve_symbol_at
);
criterion_main!(benches);
