use std::{
    env, fs,
    io::{self, BufRead as _, BufReader},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
};

mod coverage;
mod hygiene;

const CLI_SKILL_ASSETS: &str = "crates/spur-cli/assets/skills";

fn bundled_skill_assets_source(workspace_root: &Path) -> PathBuf {
    workspace_root.join(CLI_SKILL_ASSETS)
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let subcommand = args.next().unwrap_or_default();
    let extra: Vec<String> = args.collect();

    match subcommand.as_str() {
        "install" => install(extra),
        "coverage" => coverage_cmd(extra),
        "hygiene" => hygiene_cmd(extra),
        "dist" => dist(extra),
        "" | "help" | "--help" | "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("xtask: unknown subcommand {other:?}");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    eprintln!("usage: cargo xtask <subcommand>");
    eprintln!();
    eprintln!("subcommands:");
    eprintln!("  install [--debug] [--remote] [--local]   install spur to $CARGO_HOME/bin");
    eprintln!("             macOS default: compile on the build VM via `spur-cargo zigbuild`,");
    eprintln!("             then download the Mach-O artifact (S3) and install it.");
    eprintln!("             --local builds on this machine with plain cargo instead.");
    eprintln!();
    eprintln!("options:");
    eprintln!("  --debug    build/install debug artifacts for local installs");
    eprintln!("  --disable-notebook-mcp");
    eprintln!("             compile spur without brain-session notebook MCP injection");
    eprintln!("             (recovery for macOS SIGTTIN / suspended spur tui with codex).");
    eprintln!("             Equivalent: cargo xtask install -- --features disable-notebook-mcp");
    eprintln!("  --notebook-channel <auto|green>");
    eprintln!("             auto aliases green after the standalone notebook cutover");
    eprintln!("             green installs spur only and expects standalone getspur/spur-notebook");
    eprintln!(
        "  --remote   build Linux release binaries via scripts/cloud-build and fetch them back"
    );
    if cfg!(target_os = "linux") {
        eprintln!("             fetched into $CARGO_HOME/bin (native on this host)");
    } else {
        eprintln!(
            "             on a non-Linux host the Linux binaries are staged under target/remote-linux-bin"
        );
        eprintln!(
            "             instead of $CARGO_HOME/bin; they do not run here. macOS does not build SpurLab.app"
        );
    }
    eprintln!("  --force    with --remote on a non-Linux host, overwrite $CARGO_HOME/bin anyway");
    eprintln!();
    eprintln!(
        "  dist [--platforms linux,linux-x64,macos-arm64,macos-x64,windows] [--out <dir>] [--parallel]"
    );
    eprintln!("      build release spur binaries for every supported platform on the");
    eprintln!("      build VM (linux native, macOS per-arch via zigbuild, windows");
    eprintln!("      x86_64 via xwin), fetch them into <dir> (default dist/) with");
    eprintln!("      triple-suffixed names, package crates/spur-cli/assets/skills as");
    eprintln!("      spur-skills-<version>.tar.gz (share/spur/skills layout for the");
    eprintln!("      runtime SkillCatalog resolver), vendor DuckDB extensions as");
    eprintln!("      spur-duckdb-extensions-<version>.tar.gz, and write SHA256SUMS");
    eprintln!("      --parallel: run the platform legs concurrently, each in its own");
    eprintln!("        remote namespace (cargo's exclusive target-dir lock would");
    eprintln!("        serialize a shared dir; sccache still dedupes host compiles");
    eprintln!("        across the namespaces). Assumes the VM is already up — run");
    eprintln!("        scripts/cloud-build/spin.sh first so concurrent --auto-spin");
    eprintln!("        dispatches cannot race each other into duplicate VMs.");
    eprintln!();
    eprintln!("  coverage [--base <ref>] [--floor <pct>] [--diff-floor <pct>]");
    eprintln!("           [--output <path>] [--dry-run] [--measure-only | --gate-only]");
    eprintln!("      measure workspace + diff-vs-<ref> line coverage via cargo-llvm-cov");
    eprintln!("      and fail if either drops below its floor (default: base=main,");
    eprintln!("      floor=75, diff-floor=85, output=coverage/lcov.info)");
    eprintln!("      --measure-only: run cargo-llvm-cov, write the lcov file, skip the git");
    eprintln!("        diff gate (safe on the remote build VM, which has no git history)");
    eprintln!("      --gate-only: read an already-written lcov file and run the git diff");
    eprintln!("        gate, skip cargo-llvm-cov (must run locally, needs git history)");
    eprintln!();
    eprintln!("  hygiene");
    eprintln!("      enforce the spur-utilities family dependency-tree rules (spec");
    eprintln!("      2026-09-23, §5): default-features spur-mentions has no spur-acp/");
    eprintln!("      ratatui; --features code has no ratatui/spur-tui; spur-utilities");
    eprintln!("      depends directly on exactly spur-commands + spur-mentions.");
    eprintln!("      Crates not scaffolded yet are skipped with a note.");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotebookInstallChannel {
    Auto,
    Green,
}

impl NotebookInstallChannel {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "green" => Ok(Self::Green),
            "blue" => Err(
                "blue notebook install source was removed; install getspur/spur-notebook and use green"
                    .to_owned(),
            ),
            _ => Err(format!(
                "invalid --notebook-channel {value:?}; expected green or auto"
            )),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct InstallOptions {
    debug: bool,
    remote: bool,
    force: bool,
    /// Build on this machine with plain cargo instead of the default
    /// remote zigbuild + fetch flow (macOS hosts only; Linux always
    /// builds locally).
    local: bool,
    /// Compile out brain-session notebook MCP (`spur-core/disable-notebook-mcp`).
    disable_notebook_mcp: bool,
    notebook_channel: NotebookInstallChannel,
    cargo_args: Vec<String>,
}

fn parse_install_options(extra: Vec<String>) -> Result<InstallOptions, String> {
    let mut options = InstallOptions {
        debug: false,
        remote: false,
        force: false,
        local: false,
        disable_notebook_mcp: false,
        notebook_channel: NotebookInstallChannel::Green,
        cargo_args: Vec::new(),
    };
    let mut args = extra.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--debug" {
            options.debug = true;
        } else if arg == "--remote" {
            options.remote = true;
        } else if arg == "--force" {
            options.force = true;
        } else if arg == "--local" {
            options.local = true;
        } else if arg == "--disable-notebook-mcp" {
            options.disable_notebook_mcp = true;
        } else if arg == "--notebook-channel" {
            let value = args
                .next()
                .ok_or_else(|| "--notebook-channel requires green or auto".to_owned())?;
            if value.starts_with('-') {
                return Err("--notebook-channel requires green or auto".to_owned());
            }
            options.notebook_channel = NotebookInstallChannel::parse(&value)?;
        } else if let Some(value) = arg.strip_prefix("--notebook-channel=") {
            options.notebook_channel = NotebookInstallChannel::parse(value)?;
        } else {
            options.cargo_args.push(arg);
        }
    }
    if options.disable_notebook_mcp {
        ensure_cargo_feature(&mut options.cargo_args, "disable-notebook-mcp");
    }
    Ok(options)
}

/// Append `--features <name>` unless the feature is already requested in
/// `cargo_args` (either as `--features name` or `--features=name,...`).
fn ensure_cargo_feature(cargo_args: &mut Vec<String>, feature: &str) {
    let already = cargo_args.iter().enumerate().any(|(i, arg)| {
        if let Some(list) = arg.strip_prefix("--features=") {
            return list.split(',').any(|f| f.trim() == feature);
        }
        if arg == "--features" {
            return cargo_args
                .get(i + 1)
                .is_some_and(|list| list.split(',').any(|f| f.trim() == feature));
        }
        false
    });
    if !already {
        cargo_args.push("--features".to_owned());
        cargo_args.push(feature.to_owned());
    }
}

fn install(extra: Vec<String>) -> ExitCode {
    let options = match parse_install_options(extra) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("xtask: {err}");
            return ExitCode::FAILURE;
        }
    };
    let workspace_root = workspace_root();

    if options.remote {
        let host_is_linux = cfg!(target_os = "linux");
        let dest = remote_install_dest(&workspace_root, host_is_linux, options.force);
        if let Err(err) =
            install_remote_linux_binaries(&workspace_root, &dest, options.notebook_channel)
        {
            eprintln!("xtask: {err}");
            return ExitCode::FAILURE;
        }
        report_remote_install(&dest, host_is_linux, options.notebook_channel);
        return ExitCode::SUCCESS;
    }

    if cfg!(target_os = "macos") {
        if let Err(err) = install_macos_cli(
            &workspace_root,
            options.debug,
            options.local,
            &options.cargo_args,
        ) {
            eprintln!("xtask: {err}");
            return ExitCode::FAILURE;
        }
    } else {
        if let Err(err) = install_linux_binaries(
            &workspace_root,
            options.debug,
            &options.cargo_args,
            options.notebook_channel,
        ) {
            eprintln!("xtask: {err}");
            return ExitCode::FAILURE;
        }
    }
    print_green_notebook_install_guidance();
    ExitCode::SUCCESS
}

/// Continuous hygiene gate entry (see `hygiene` module docs): exit code only.
fn hygiene_cmd(extra: Vec<String>) -> ExitCode {
    if !extra.is_empty() {
        eprintln!("xtask: hygiene takes no options (got {extra:?})");
        return ExitCode::FAILURE;
    }
    match hygiene::run_hygiene(&workspace_root()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: {err}");
            ExitCode::FAILURE
        }
    }
}

fn coverage_cmd(extra: Vec<String>) -> ExitCode {
    let options = match coverage::parse_coverage_options(extra) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("xtask: {err}");
            return ExitCode::FAILURE;
        }
    };
    let workspace_root = workspace_root();

    if options.dry_run {
        eprintln!("==> dry run — no commands executed");
        if !options.gate_only {
            eprintln!(
                "would ensure installed: cargo {}",
                command_args(&coverage::install_llvm_cov_command()).join(" ")
            );
            eprintln!(
                "would run: cargo {}",
                command_args(&coverage::llvm_cov_clean_command(&workspace_root)).join(" ")
            );
            eprintln!(
                "would run: cargo {}",
                command_args(&coverage::llvm_cov_measure_command(
                    &workspace_root,
                    &options.output_path
                ))
                .join(" ")
            );
        }
        if !options.measure_only {
            eprintln!(
                "would run: git {}",
                command_args(&coverage::git_diff_command(&workspace_root, &options.base)).join(" ")
            );
            eprintln!(
                "floor={:.2}% diff-floor={:.2}% base={}",
                options.floor, options.diff_floor, options.base
            );
        }
        return ExitCode::SUCCESS;
    }

    let lcov_path = workspace_root.join(&options.output_path);

    if !options.gate_only {
        let mut version_check = coverage::llvm_cov_version_command();
        version_check
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let installed = version_check
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !installed {
            if let Err(err) = run_status(
                &mut coverage::install_llvm_cov_command(),
                "cargo install cargo-llvm-cov",
            ) {
                eprintln!("xtask: {err}");
                return ExitCode::FAILURE;
            }
        }

        if let Some(parent) = lcov_path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!("xtask: failed to create {}: {err}", parent.display());
                return ExitCode::FAILURE;
            }
        }

        let mut clean = coverage::llvm_cov_clean_command(&workspace_root);
        if let Err(err) = run_status(&mut clean, "cargo llvm-cov clean --workspace") {
            eprintln!("xtask: {err}");
            return ExitCode::FAILURE;
        }

        let mut measure = coverage::llvm_cov_measure_command(&workspace_root, &options.output_path);
        if let Err(err) = run_status(&mut measure, "cargo llvm-cov --workspace --lib --lcov") {
            eprintln!("xtask: {err}");
            return ExitCode::FAILURE;
        }
    }

    if options.measure_only {
        eprintln!("coverage measured: {}", lcov_path.display());
        return ExitCode::SUCCESS;
    }

    // The remote build syncs git-tracked file *contents* only (via `git
    // ls-files`, not a real clone), so there's no branch history on the VM to
    // diff against — this half of the gate must run wherever `git` has real
    // history, i.e. locally (see `--gate-only` docs on CoverageOptions).
    let lcov_text = match fs::read_to_string(&lcov_path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("xtask: failed to read {}: {err}", lcov_path.display());
            return ExitCode::FAILURE;
        }
    };
    let coverage_data = coverage::LineCoverage::parse_lcov(&lcov_text);

    let mut diff_cmd = coverage::git_diff_command(&workspace_root, &options.base);
    let diff_text = match run_output(&mut diff_cmd, "git diff (changed .rs lines)") {
        Ok(text) => text,
        Err(err) => {
            eprintln!("xtask: {err}");
            return ExitCode::FAILURE;
        }
    };
    let changed_lines = coverage::parse_changed_lines(&diff_text);

    let result = coverage::evaluate_gate(
        &coverage_data,
        &changed_lines,
        options.floor,
        options.diff_floor,
    );
    eprintln!("{}", result.report());

    if result.overall_pass() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn install_macos_cli(
    workspace_root: &Path,
    debug: bool,
    local: bool,
    extra: &[String],
) -> Result<(), String> {
    if local {
        let mut build = cargo_build_command(workspace_root, debug, &["spur-cli"], &[], extra);
        run_status(&mut build, "cargo build -p spur-cli")?;
        return install_built_binary(workspace_root, debug, "spur");
    }

    // Default macOS flow: compile on the build VM through the standard
    // zigbuild pattern (see docs/superpowers/specs/
    // 2026-07-07-zigbuild-macos-cross-poc.md), then download the Mach-O
    // artifact and install it. `--local` opts back into an on-host build.
    let triple = darwin_triple_for_arch(std::env::consts::ARCH)?;
    let mut build = zigbuild_install_build_command(workspace_root, debug, &triple, extra);
    run_status(&mut build, "scripts/spur-cargo zigbuild -p spur-cli")?;
    let mut fetch = zigbuild_install_fetch_command(workspace_root, &triple, debug);
    run_status(&mut fetch, "scripts/cloud-build/fetch.sh --via-s3 (spur)")?;

    let profile = if debug { "debug" } else { "release" };
    let fetched = workspace_root
        .join("target")
        .join(&triple)
        .join(profile)
        .join("spur");
    // S3 download does not preserve the executable bit.
    mark_executable(&fetched)?;
    install_binary_from(workspace_root, &fetched, "spur")
}

/// Map the host arch onto the darwin target triple that `spur-cargo
/// zigbuild` should produce for `cargo xtask install`.
fn darwin_triple_for_arch(arch: &str) -> Result<String, String> {
    match arch {
        "aarch64" => Ok("aarch64-apple-darwin".to_owned()),
        "x86_64" => Ok("x86_64-apple-darwin".to_owned()),
        other => Err(format!(
            "no darwin install target for host arch {other:?}; use --local"
        )),
    }
}

fn zigbuild_install_build_command(
    workspace_root: &Path,
    debug: bool,
    triple: &str,
    extra: &[String],
) -> Command {
    let mut cmd = Command::new(workspace_root.join("scripts/spur-cargo"));
    cmd.arg("zigbuild");
    if !debug {
        cmd.arg("--release");
    }
    cmd.args(["-p", "spur-cli", "--target", triple]);
    for arg in extra.iter().filter(|arg| !is_xtask_install_flag(arg)) {
        cmd.arg(arg);
    }
    cmd.current_dir(workspace_root);
    cmd
}

fn zigbuild_install_fetch_command(workspace_root: &Path, triple: &str, debug: bool) -> Command {
    let profile = if debug { "debug" } else { "release" };
    let artifact = format!("target/{triple}/{profile}/spur");
    let mut cmd = Command::new(workspace_root.join("scripts/cloud-build/fetch.sh"));
    cmd.arg("--via-s3")
        .arg("--to")
        .arg(workspace_root.join(&artifact))
        .arg(&artifact);
    cmd.current_dir(workspace_root);
    cmd
}

#[cfg(unix)]
fn mark_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .map_err(|err| format!("failed to mark {} executable: {err}", path.display()))
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// One platform in the `cargo xtask dist` build matrix. Every variant is
/// compiled on the build VM (see scripts/spur-cargo) and fetched back, so
/// dist works identically from any host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DistPlatform {
    /// Native VM target. The AWS Graviton builder emits aarch64 Linux
    /// binaries; the artifact suffix tracks that.
    LinuxAarch64,
    /// `x86_64` ELF via `spur-cargo zigbuild` (zig cross C/C++ + ELF link).
    /// Exists for npm parity: @getspur/spur-cli must serve linux x64.
    LinuxX64Gnu,
    /// arm64 Mach-O via `spur-cargo zigbuild`.
    MacAarch64,
    /// `x86_64` Mach-O via `spur-cargo zigbuild`.
    MacX64,
    /// PE32+ via `spur-cargo xwin`.
    WindowsX64,
}

impl DistPlatform {
    const ALL: [Self; 5] = [
        Self::LinuxAarch64,
        Self::LinuxX64Gnu,
        Self::MacAarch64,
        Self::MacX64,
        Self::WindowsX64,
    ];

    fn parse(value: &str) -> Result<Vec<Self>, String> {
        match value {
            "linux" => Ok(vec![Self::LinuxAarch64]),
            "linux-x64" | "linux-x86_64" => Ok(vec![Self::LinuxX64Gnu]),
            "macos" | "mac" | "darwin" => Ok(vec![Self::MacAarch64, Self::MacX64]),
            "macos-arm64" | "macos-aarch64" | "darwin-arm64" | "darwin-aarch64" => {
                Ok(vec![Self::MacAarch64])
            }
            "macos-x64" | "macos-x86_64" | "darwin-x64" | "darwin-x86_64" => {
                Ok(vec![Self::MacX64])
            }
            "windows" | "win" => Ok(vec![Self::WindowsX64]),
            other => Err(format!(
                "unknown dist platform {other:?}; expected linux, linux-x64, macos-arm64, macos-x64, macos, or windows"
            )),
        }
    }

    /// Artifact path relative to the remote target dir, as passed to
    /// scripts/cloud-build/fetch.sh.
    fn artifact_rel(self) -> &'static str {
        match self {
            Self::LinuxAarch64 => "target/release/spur",
            Self::LinuxX64Gnu => "target/x86_64-unknown-linux-gnu/release/spur",
            Self::MacAarch64 => "target/aarch64-apple-darwin/release/spur",
            Self::MacX64 => "target/x86_64-apple-darwin/release/spur",
            Self::WindowsX64 => "target/x86_64-pc-windows-msvc/release/spur.exe",
        }
    }

    /// Final artifact file name under the dist output directory.
    fn artifact_name(self, version: &str) -> String {
        match self {
            Self::LinuxAarch64 => format!("spur-{version}-aarch64-unknown-linux-gnu"),
            Self::LinuxX64Gnu => format!("spur-{version}-x86_64-unknown-linux-gnu"),
            Self::MacAarch64 => format!("spur-{version}-aarch64-apple-darwin"),
            Self::MacX64 => format!("spur-{version}-x86_64-apple-darwin"),
            Self::WindowsX64 => format!("spur-{version}-x86_64-pc-windows-msvc.exe"),
        }
    }

    fn build_label(self) -> &'static str {
        match self {
            Self::LinuxAarch64 => "spur-cargo build --release -p spur-cli (linux native)",
            Self::LinuxX64Gnu => "spur-cargo zigbuild --release -p spur-cli (linux x86_64)",
            Self::MacAarch64 => "spur-cargo zigbuild --release -p spur-cli (macOS arm64)",
            Self::MacX64 => "spur-cargo zigbuild --release -p spur-cli (macOS x86_64)",
            Self::WindowsX64 => "spur-cargo xwin build --release -p spur-cli (windows x86_64)",
        }
    }

    /// Short key used as the log prefix and remote-namespace suffix for
    /// parallel dist legs.
    fn namespace_key(self) -> &'static str {
        match self {
            Self::LinuxAarch64 => "linux",
            Self::LinuxX64Gnu => "linux-x64",
            Self::MacAarch64 => "macos-arm64",
            Self::MacX64 => "macos-x64",
            Self::WindowsX64 => "windows",
        }
    }

    /// `DuckDB` community/core extension CDN platform directory name.
    fn duckdb_platform(self) -> &'static str {
        match self {
            Self::LinuxAarch64 => "linux_arm64",
            Self::LinuxX64Gnu => "linux_amd64",
            Self::MacAarch64 => "osx_arm64",
            Self::MacX64 => "osx_amd64",
            Self::WindowsX64 => "windows_amd64",
        }
    }

    /// Remote namespace isolating this platform's parallel leg on the VM.
    /// Cargo takes an exclusive lock on the whole target dir, so parallel
    /// legs sharing /mnt/cargo/targets/spur/main would serialize on it;
    /// separate namespaces give each leg its own worktree + target dir while
    /// sccache still dedupes the host build-script/proc-macro compiles.
    fn remote_namespace(self) -> String {
        format!("spur-dist-{}", self.namespace_key())
    }
}

struct DistOptions {
    platforms: Vec<DistPlatform>,
    out_dir: Option<PathBuf>,
    parallel: bool,
}

fn parse_dist_options(extra: Vec<String>) -> Result<DistOptions, String> {
    let mut options = DistOptions {
        platforms: DistPlatform::ALL.to_vec(),
        out_dir: None,
        parallel: false,
    };
    let mut args = extra.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--parallel" {
            options.parallel = true;
        } else if let Some(value) = arg.strip_prefix("--platforms=") {
            options.platforms = parse_dist_platform_list(value)?;
        } else if arg == "--platforms" {
            let value = args
                .next()
                .ok_or_else(|| "--platforms requires a comma-separated list".to_owned())?;
            options.platforms = parse_dist_platform_list(&value)?;
        } else if let Some(value) = arg.strip_prefix("--out=") {
            options.out_dir = Some(PathBuf::from(value));
        } else if arg == "--out" {
            let value = args
                .next()
                .ok_or_else(|| "--out requires a directory".to_owned())?;
            options.out_dir = Some(PathBuf::from(value));
        } else {
            return Err(format!("unknown dist option {arg:?}"));
        }
    }
    Ok(options)
}

fn parse_dist_platform_list(value: &str) -> Result<Vec<DistPlatform>, String> {
    let mut platforms = Vec::new();
    for token in value.split(',').filter(|token| !token.is_empty()) {
        for platform in DistPlatform::parse(token.trim())? {
            if !platforms.contains(&platform) {
                platforms.push(platform);
            }
        }
    }
    if platforms.is_empty() {
        return Err(
            "--platforms requires at least one of linux, linux-x64, macos-arm64, macos-x64, macos, or windows"
                .to_owned(),
        );
    }
    Ok(platforms)
}

fn dist(extra: Vec<String>) -> ExitCode {
    let options = match parse_dist_options(extra) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("xtask: {err}");
            return ExitCode::FAILURE;
        }
    };
    let workspace_root = workspace_root();
    match run_dist(&workspace_root, &options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_dist(workspace_root: &Path, options: &DistOptions) -> Result<(), String> {
    let version = workspace_version(workspace_root)?;
    let out_dir = options
        .out_dir
        .clone()
        .unwrap_or_else(|| workspace_root.join("dist"));
    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;

    let parallel = options.parallel && options.platforms.len() > 1;
    let mut artifacts = if parallel {
        run_dist_platforms_parallel(workspace_root, &options.platforms, &version, &out_dir)?
    } else {
        let mut artifacts = Vec::new();
        for platform in &options.platforms {
            artifacts.push(run_dist_platform(
                workspace_root,
                *platform,
                &version,
                &out_dir,
                None,
            )?);
        }
        artifacts
    };

    // spur-cli embeds the skill tree, but retain the standalone archive for
    // npm and filesystem-package compatibility. Its share/spur/skills layout
    // matches install_bundled_skill_assets and package_asset_candidates.
    let skills_archive = package_dist_skill_assets(workspace_root, &out_dir, &version)?;
    artifacts.push(skills_archive);

    if env::var_os("SPUR_DIST_SKIP_DUCKDB_EXTENSIONS").is_none() {
        artifacts.push(package_dist_duckdb_extensions(
            workspace_root,
            &out_dir,
            &version,
            &options.platforms,
        )?);
    }

    let sums_path = out_dir.join("SHA256SUMS");
    write_sha256sums(&dist_artifacts_in(&out_dir)?, &sums_path)?;

    eprintln!("==> dist complete: {}", out_dir.display());
    for artifact in &artifacts {
        eprintln!("    {}", artifact.display());
    }
    eprintln!("    {}", sums_path.display());
    Ok(())
}

/// Archive name for the platform-independent skill asset bundle.
fn dist_skills_archive_name(version: &str) -> String {
    format!("spur-skills-{version}.tar.gz")
}

fn dist_duckdb_extensions_dir_name() -> &'static str {
    "duckdb-extensions"
}

/// Archive name for the platform-independent `DuckDB` extension sidecar.
/// GitHub release assets must be files; `gh release create dist/*` cannot
/// upload the unpacked `duckdb-extensions/` directory.
fn dist_duckdb_extensions_archive_name(version: &str) -> String {
    format!("spur-duckdb-extensions-{version}.tar.gz")
}

fn package_dist_duckdb_extensions(
    workspace_root: &Path,
    out_dir: &Path,
    version: &str,
    platforms: &[DistPlatform],
) -> Result<PathBuf, String> {
    let script = workspace_root.join("scripts/vendor-duckdb-extensions.sh");
    if !script.is_file() {
        return Err(format!("missing {}", script.display()));
    }

    let stage = out_dir.join(".duckdb-extensions-stage");
    remove_existing_path(&stage)?;
    // Runtime looks up `<prefix>/share/spur/duckdb-extensions` (next to
    // `share/spur/skills`) and `exe/duckdb-extensions`.
    let dest = stage
        .join("share")
        .join("spur")
        .join(dist_duckdb_extensions_dir_name());
    let mut command = Command::new("bash");
    command.arg(&script).arg("--out").arg(&dest);
    for platform in platforms {
        command.arg("--platform").arg(platform.duckdb_platform());
    }
    let status = command
        .status()
        .map_err(|err| format!("failed to run {}: {err}", script.display()))?;
    if !status.success() {
        let _ = remove_existing_path(&stage);
        return Err(format!(
            "{} failed with {status}; set SPUR_DIST_SKIP_DUCKDB_EXTENSIONS=1 to skip",
            script.display()
        ));
    }

    let archive_name = dist_duckdb_extensions_archive_name(version);
    let archive_path = out_dir.join(&archive_name);
    let stage_str = stage
        .to_str()
        .ok_or_else(|| format!("non-UTF8 stage path: {}", stage.display()))?;
    let archive_str = archive_path
        .to_str()
        .ok_or_else(|| format!("non-UTF8 archive path: {}", archive_path.display()))?;
    let tar_status = Command::new("tar")
        .args(["-czf", archive_str, "-C", stage_str, "share"])
        .status()
        .map_err(|err| format!("failed to spawn tar for duckdb extensions: {err}"))?;
    if !tar_status.success() {
        let _ = remove_existing_path(&stage);
        return Err(format!(
            "tar failed packaging duckdb extensions into {} (status {tar_status})",
            archive_path.display()
        ));
    }
    remove_existing_path(&stage)?;
    // Previous dist runs left an unpacked directory that `gh release create
    // dist/*` then tried to upload as a GitHub asset.
    remove_existing_path(&out_dir.join(dist_duckdb_extensions_dir_name()))?;

    if !archive_path.is_file() {
        return Err(format!(
            "duckdb extension archive missing after tar: {}",
            archive_path.display()
        ));
    }
    eprintln!("packaged duckdb extensions: {}", archive_path.display());
    Ok(archive_path)
}

/// Package `crates/spur-cli/assets/skills` into
/// `dist/spur-skills-<version>.tar.gz` with
/// archive-internal paths `share/spur/skills/<id>/...`.
///
/// That layout matches:
/// - local `cargo xtask install` → `$CARGO_HOME/../share/spur/skills`
/// - `package_asset_candidates()` → `<prefix>/share/spur/skills`
/// - npm postinstall extraction next to `bin/spur`
fn package_dist_skill_assets(
    workspace_root: &Path,
    out_dir: &Path,
    version: &str,
) -> Result<PathBuf, String> {
    let source = bundled_skill_assets_source(workspace_root);
    if !source.is_dir() {
        return Err(format!(
            "expected bundled skill assets at {}",
            source.display()
        ));
    }

    let archive_name = dist_skills_archive_name(version);
    let archive_path = out_dir.join(&archive_name);
    let stage = out_dir.join(".skills-stage");
    remove_existing_path(&stage)?;
    let staged_skills = stage.join("share/spur/skills");
    copy_dir_all(&source, &staged_skills)?;

    // Prefer GNU/BSD tar -czf; both accept -C and a single path operand.
    let stage_str = stage
        .to_str()
        .ok_or_else(|| format!("non-UTF8 stage path: {}", stage.display()))?;
    let archive_str = archive_path
        .to_str()
        .ok_or_else(|| format!("non-UTF8 archive path: {}", archive_path.display()))?;
    let status = Command::new("tar")
        .args(["-czf", archive_str, "-C", stage_str, "share"])
        .status()
        .map_err(|err| format!("failed to spawn tar for skill assets: {err}"))?;
    if !status.success() {
        let _ = remove_existing_path(&stage);
        return Err(format!(
            "tar failed packaging skill assets into {} (status {status})",
            archive_path.display()
        ));
    }
    remove_existing_path(&stage)?;

    if !archive_path.is_file() {
        return Err(format!(
            "skill asset archive missing after tar: {}",
            archive_path.display()
        ));
    }
    eprintln!("packaged skill assets: {}", archive_path.display());
    Ok(archive_path)
}

/// Build + fetch one platform leg. In parallel mode the build and fetch run
/// inside the platform's own remote namespace and every output line is
/// prefixed with the platform key so interleaved logs stay attributable.
fn run_dist_platform(
    workspace_root: &Path,
    platform: DistPlatform,
    version: &str,
    out_dir: &Path,
    parallel: Option<&str>,
) -> Result<PathBuf, String> {
    let mut build = dist_build_command(workspace_root, platform);
    if let Some(queue_slots) = parallel {
        apply_dist_namespace(&mut build, platform, queue_slots);
        run_status_prefixed(&mut build, platform.build_label(), platform.namespace_key())?;
    } else {
        run_status(&mut build, platform.build_label())?;
    }

    let dest = out_dir.join(platform.artifact_name(version));
    let mut fetch = dist_fetch_command(workspace_root, platform, &dest);
    if let Some(queue_slots) = parallel {
        apply_dist_namespace(&mut fetch, platform, queue_slots);
        run_status_prefixed(
            &mut fetch,
            "scripts/cloud-build/fetch.sh --via-s3 (dist)",
            platform.namespace_key(),
        )?;
    } else {
        run_status(&mut fetch, "scripts/cloud-build/fetch.sh --via-s3 (dist)")?;
    }
    // S3 download does not preserve the executable bit.
    mark_executable(&dest)?;
    Ok(dest)
}

/// Run every platform leg concurrently and collect all artifacts. Any leg
/// failure fails the whole dist, but the other legs run to completion first
/// so their work (and the shared sccache warm-up) is not wasted.
fn run_dist_platforms_parallel(
    workspace_root: &Path,
    platforms: &[DistPlatform],
    version: &str,
    out_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    eprintln!(
        "==> dist: building {} platform legs in parallel",
        platforms.len()
    );
    let queue_slots = dist_queue_slots(platforms.len());
    let queue_slots = queue_slots.as_str();
    let results: Vec<(DistPlatform, Result<PathBuf, String>)> = thread::scope(|scope| {
        let handles: Vec<_> = platforms
            .iter()
            .map(|platform| {
                let platform = *platform;
                (
                    platform,
                    scope.spawn(move || {
                        run_dist_platform(
                            workspace_root,
                            platform,
                            version,
                            out_dir,
                            Some(queue_slots),
                        )
                    }),
                )
            })
            .collect();
        handles
            .into_iter()
            .map(|(platform, handle)| {
                let result = handle
                    .join()
                    .unwrap_or_else(|_| Err("dist leg thread panicked".to_owned()));
                (platform, result)
            })
            .collect()
    });

    let mut artifacts = Vec::new();
    let mut failures = Vec::new();
    for (platform, result) in results {
        match result {
            Ok(dest) => artifacts.push(dest),
            Err(err) => failures.push(format!("{}: {err}", platform.namespace_key())),
        }
    }
    if !failures.is_empty() {
        return Err(format!("dist legs failed: {}", failures.join("; ")));
    }
    Ok(artifacts)
}

/// Point a dist leg's spur-cargo/fetch.sh subprocess at its per-platform
/// remote namespace (spur-cargo forwards a caller-set `SPUR_REMOTE_NAMESPACE`)
/// and size build.sh's per-VM admission queue so no leg waits on a slot.
fn apply_dist_namespace(cmd: &mut Command, platform: DistPlatform, queue_slots: &str) {
    cmd.env("SPUR_REMOTE_NAMESPACE", platform.remote_namespace());
    cmd.env("SPUR_BUILD_MAX_CONCURRENT", queue_slots);
}

/// build.sh admits `SPUR_BUILD_MAX_CONCURRENT` builds (default 3) per remote
/// builder, so parallel dist must raise the cap to the leg count or the extra
/// legs queue behind the others. An explicit caller override still wins.
fn dist_queue_slots(leg_count: usize) -> String {
    match env::var("SPUR_BUILD_MAX_CONCURRENT") {
        Ok(explicit) if !explicit.trim().is_empty() => explicit,
        _ => leg_count.to_string(),
    }
}

/// Like `run_status`, but pipes the child's stdout/stderr and re-emits every
/// line prefixed with `[<prefix>]` so concurrent legs stay readable.
fn run_status_prefixed(cmd: &mut Command, label: &str, prefix: &str) -> Result<(), String> {
    eprintln!("[{prefix}] ==> {label}");
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|err| format!("failed to spawn {label}: {err}"))?;
    let relay = |stream: Option<Box<dyn io::Read + Send>>, prefix: String| {
        stream.map(|stream| {
            thread::spawn(move || {
                for line in BufReader::new(stream).lines() {
                    match line {
                        Ok(line) => eprintln!("[{prefix}] {line}"),
                        Err(_) => break,
                    }
                }
            })
        })
    };
    let out_thread = relay(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn io::Read + Send>),
        prefix.to_owned(),
    );
    let err_thread = relay(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn io::Read + Send>),
        prefix.to_owned(),
    );
    let status = child
        .wait()
        .map_err(|err| format!("failed to wait on {label}: {err}"))?;
    if let Some(handle) = out_thread {
        let _ = handle.join();
    }
    if let Some(handle) = err_thread {
        let _ = handle.join();
    }
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed (status {status})"))
    }
}

fn dist_build_command(workspace_root: &Path, platform: DistPlatform) -> Command {
    let mut cmd = Command::new(workspace_root.join("scripts/spur-cargo"));
    match platform {
        DistPlatform::LinuxAarch64 => {
            cmd.args(["build", "--release", "-p", "spur-cli"]);
        }
        DistPlatform::LinuxX64Gnu => {
            cmd.args([
                "zigbuild",
                "--release",
                "-p",
                "spur-cli",
                "--target",
                "x86_64-unknown-linux-gnu",
            ]);
        }
        DistPlatform::MacAarch64 => {
            cmd.args([
                "zigbuild",
                "--release",
                "-p",
                "spur-cli",
                "--target",
                "aarch64-apple-darwin",
            ]);
        }
        DistPlatform::MacX64 => {
            cmd.args([
                "zigbuild",
                "--release",
                "-p",
                "spur-cli",
                "--target",
                "x86_64-apple-darwin",
            ]);
        }
        DistPlatform::WindowsX64 => {
            cmd.args([
                "xwin",
                "build",
                "--release",
                "-p",
                "spur-cli",
                "--target",
                "x86_64-pc-windows-msvc",
            ]);
        }
    }
    cmd.current_dir(workspace_root);
    cmd
}

fn dist_fetch_command(workspace_root: &Path, platform: DistPlatform, dest: &Path) -> Command {
    let mut cmd = Command::new(workspace_root.join("scripts/cloud-build/fetch.sh"));
    cmd.arg("--via-s3")
        .arg("--to")
        .arg(dest)
        .arg(platform.artifact_rel());
    cmd.current_dir(workspace_root);
    cmd
}

/// Read the `[workspace.package]` version from the workspace Cargo.toml.
/// Line-based on purpose: xtask stays dependency-free.
fn workspace_version(workspace_root: &Path) -> Result<String, String> {
    let manifest_path = workspace_root.join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;
    parse_workspace_version(&manifest).ok_or_else(|| {
        format!(
            "no [workspace.package] version in {}",
            manifest_path.display()
        )
    })
}

fn parse_workspace_version(manifest: &str) -> Option<String> {
    let mut in_workspace_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_workspace_package = line == "[workspace.package]";
            continue;
        }
        if !in_workspace_package {
            continue;
        }
        if let Some(rest) = line.strip_prefix("version") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().trim_matches('"').to_owned());
            }
        }
    }
    None
}

/// Every `spur-*` artifact currently in the dist output directory, sorted.
/// SHA256SUMS covers the whole directory rather than just this run's
/// platforms, so a `--platforms` subset re-run (e.g. after one platform
/// failed) refreshes the sums without dropping the other artifacts.
fn dist_artifacts_in(out_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(out_dir)
        .map_err(|err| format!("failed to list {}: {err}", out_dir.display()))?;
    let mut artifacts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to list {}: {err}", out_dir.display()))?;
        let path = entry.path();
        let is_artifact = path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("spur-"));
        if is_artifact {
            artifacts.push(path);
        }
    }
    artifacts.sort();
    Ok(artifacts)
}

/// Write a coreutils-compatible SHA256SUMS file (hash, two spaces, name).
/// Shells out to sha256sum (Linux) or shasum -a 256 (macOS).
fn write_sha256sums(artifacts: &[PathBuf], sums_path: &Path) -> Result<(), String> {
    use std::fmt::Write as _;

    let mut lines = String::new();
    for artifact in artifacts {
        let hash = sha256_file(artifact)?;
        let name = artifact
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("unrepresentable artifact name: {}", artifact.display()))?;
        writeln!(&mut lines, "{hash}  {name}").expect("writing to a String cannot fail");
    }
    fs::write(sums_path, lines)
        .map_err(|err| format!("failed to write {}: {err}", sums_path.display()))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let attempts: [(&str, &[&str]); 2] = [("sha256sum", &[]), ("shasum", &["-a", "256"])];
    for (program, args) in attempts {
        let output = Command::new(program).args(args).arg(path).output();
        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let hash = stdout
                    .split_whitespace()
                    .next()
                    .ok_or_else(|| format!("{program} produced no output for {}", path.display()))?
                    .to_owned();
                return Ok(hash);
            }
            _ => {}
        }
    }
    Err(format!(
        "no working sha256 tool (tried sha256sum, shasum -a 256) for {}",
        path.display()
    ))
}

fn install_linux_binaries(
    workspace_root: &Path,
    debug: bool,
    extra: &[String],
    notebook_channel: NotebookInstallChannel,
) -> Result<(), String> {
    let mut build = linux_install_build_command(workspace_root, debug, extra, notebook_channel);
    run_status(&mut build, linux_install_build_label(notebook_channel))?;
    install_built_binary(workspace_root, debug, "spur")?;
    Ok(())
}

fn linux_install_build_command(
    workspace_root: &Path,
    debug: bool,
    extra: &[String],
    _notebook_channel: NotebookInstallChannel,
) -> Command {
    cargo_build_command(workspace_root, debug, &["spur-cli"], &[], extra)
}

fn linux_install_build_label(_notebook_channel: NotebookInstallChannel) -> &'static str {
    "cargo build -p spur-cli"
}

/// Where `install --remote` deposits the fetched Linux release binaries.
struct RemoteInstallDest {
    dir: PathBuf,
    /// True when we steered the foreign Linux binaries into a staging directory
    /// instead of clobbering `$CARGO_HOME/bin` (the default on a non-Linux host).
    staged: bool,
}

/// Decide where the fetched Linux binaries land.
///
/// On a Linux host the binaries are native, so install them into
/// `$CARGO_HOME/bin` as usual. On any other host they are foreign ELF binaries
/// that would shadow, and break, the working native `spur`, so default to a
/// staging dir under `target/`. `--force` opts back into clobbering
/// `$CARGO_HOME/bin` (useful when the host PATH targets a Linux box over a mount).
fn remote_install_dest(
    workspace_root: &Path,
    host_is_linux: bool,
    force: bool,
) -> RemoteInstallDest {
    if host_is_linux || force {
        RemoteInstallDest {
            dir: cargo_home_bin(),
            staged: false,
        }
    } else {
        RemoteInstallDest {
            dir: workspace_root.join("target/remote-linux-bin"),
            staged: true,
        }
    }
}

fn install_remote_linux_binaries(
    workspace_root: &Path,
    dest: &RemoteInstallDest,
    notebook_channel: NotebookInstallChannel,
) -> Result<(), String> {
    let mut build = remote_install_build_command(workspace_root, notebook_channel);
    run_status(
        &mut build,
        "scripts/cloud-build/build.sh remote release build",
    )?;
    let mut fetch = remote_install_fetch_command(workspace_root, &dest.dir, notebook_channel);
    run_status(&mut fetch, remote_install_fetch_label(notebook_channel))?;
    install_bundled_skill_assets(workspace_root, &dest.dir)
}

fn remote_install_build_command(
    workspace_root: &Path,
    _notebook_channel: NotebookInstallChannel,
) -> Command {
    let mut cmd = Command::new(workspace_root.join("scripts/cloud-build/build.sh"));
    cmd.arg("--auto-spin").arg("--").args([
        "build",
        "--release",
        "-p",
        "spur-cli",
        "--no-default-features",
        "--features",
        "duckdb-bundled",
    ]);
    cmd.arg("--locked").current_dir(workspace_root);
    cmd
}

fn remote_install_fetch_command(
    workspace_root: &Path,
    dest_dir: &Path,
    _notebook_channel: NotebookInstallChannel,
) -> Command {
    let mut cmd = Command::new(workspace_root.join("scripts/cloud-build/fetch.sh"));
    cmd.arg("--to")
        .arg(dest_dir.join("spur"))
        .arg("target/release/spur");
    cmd.current_dir(workspace_root);
    cmd
}

fn remote_install_fetch_label(_notebook_channel: NotebookInstallChannel) -> &'static str {
    "scripts/cloud-build/fetch.sh target/release/spur"
}

fn report_remote_install(
    dest: &RemoteInstallDest,
    host_is_linux: bool,
    _notebook_channel: NotebookInstallChannel,
) {
    if dest.staged {
        let spur = dest.dir.join("spur");
        eprintln!();
        eprintln!("fetched Linux release binary to a staging dir (not $CARGO_HOME/bin):");
        eprintln!("  {}", spur.display());
        eprintln!();
        eprintln!("this is a Linux ELF binary and will not run on this host. Copy it");
        eprintln!("to a Linux box, or re-run with --force to overwrite $CARGO_HOME/bin.");
    } else {
        if !host_is_linux {
            eprintln!();
            eprintln!("warning: --force installed Linux binaries into $CARGO_HOME/bin on a");
            eprintln!("warning: non-Linux host; they will not run here. Re-run `cargo xtask");
            eprintln!("warning: install` (no --remote) to restore the native binaries.");
        }
    }
    print_green_notebook_install_guidance();
}

fn cargo_build_command(
    workspace_root: &Path,
    debug: bool,
    packages: &[&str],
    features: &[&str],
    extra: &[String],
) -> Command {
    let mut cmd = Command::new(cargo());
    cmd.arg("build").current_dir(workspace_root);
    if !debug {
        cmd.arg("--release");
    }
    for package in packages {
        cmd.arg("-p").arg(package);
    }
    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }
    for arg in extra.iter().filter(|arg| !is_xtask_install_flag(arg)) {
        cmd.arg(arg);
    }
    apply_macos_rustc_wrapper_workaround(&mut cmd);
    cmd
}

fn is_xtask_install_flag(arg: &str) -> bool {
    matches!(
        arg,
        "--debug" | "--remote" | "--force" | "--local" | "--disable-notebook-mcp"
    ) || arg == "--notebook-channel"
        || arg.starts_with("--notebook-channel=")
}

fn green_notebook_install_guidance() -> String {
    [
        "notebook channel green selected; notebook source is owned by getspur/spur-notebook.",
        "Install the standalone notebook from https://github.com/getspur/spur-notebook.",
        "Linux expects $CARGO_HOME/bin/spur-notebook or another spur-notebook on PATH.",
        "macOS expects ~/Applications/SpurLab.app/Contents/MacOS/SpurLab.",
        "Set SPUR_NOTEBOOK_CHANNEL=green and, if needed, SPUR_NOTEBOOK_BIN=/path/to/spur-notebook.",
    ]
    .join("\n")
}

fn print_green_notebook_install_guidance() {
    eprintln!();
    for line in green_notebook_install_guidance().lines() {
        eprintln!("{line}");
    }
}

fn install_built_binary(
    workspace_root: &Path,
    debug: bool,
    binary_name: &str,
) -> Result<(), String> {
    let profile = if debug { "debug" } else { "release" };
    let built_binary = workspace_root
        .join("target")
        .join(profile)
        .join(binary_name);
    install_binary_from(workspace_root, &built_binary, binary_name)
}

fn install_binary_from(
    workspace_root: &Path,
    built_binary: &Path,
    binary_name: &str,
) -> Result<(), String> {
    if !built_binary.is_file() {
        return Err(format!(
            "expected built binary at {}",
            built_binary.display()
        ));
    }

    let bin_dir = cargo_home_bin();
    fs::create_dir_all(&bin_dir)
        .map_err(|err| format!("failed to create {}: {err}", bin_dir.display()))?;
    let installed_binary = bin_dir.join(binary_name);
    remove_existing_path(&installed_binary)?;
    fs::copy(built_binary, &installed_binary).map_err(|err| {
        format!(
            "failed to copy {} to {}: {err}",
            built_binary.display(),
            installed_binary.display()
        )
    })?;
    eprintln!("installed {binary_name}: {}", installed_binary.display());
    install_bundled_skill_assets(workspace_root, &bin_dir)?;
    Ok(())
}

fn install_bundled_skill_assets(workspace_root: &Path, bin_dir: &Path) -> Result<(), String> {
    let source = bundled_skill_assets_source(workspace_root);
    if !source.is_dir() {
        return Err(format!(
            "expected bundled skill assets at {}",
            source.display()
        ));
    }
    let prefix = bin_dir.parent().ok_or_else(|| {
        format!(
            "failed to resolve install prefix from {}",
            bin_dir.display()
        )
    })?;
    let dest = prefix.join("share/spur/skills");
    remove_existing_path(&dest)?;
    copy_dir_all(&source, &dest)?;
    eprintln!("installed skill assets: {}", dest.display());
    Ok(())
}

fn copy_dir_all(source: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest)
        .map_err(|err| format!("failed to create {}: {err}", dest.display()))?;
    for entry in
        fs::read_dir(source).map_err(|err| format!("failed to read {}: {err}", source.display()))?
    {
        let entry = entry.map_err(|err| format!("failed to read {}: {err}", source.display()))?;
        let source_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to inspect {}: {err}", source_path.display()))?;
        if file_type.is_dir() {
            copy_dir_all(&source_path, &dest_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &dest_path).map_err(|err| {
                format!(
                    "failed to copy {} to {}: {err}",
                    source_path.display(),
                    dest_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn remove_existing_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let result = if metadata.file_type().is_dir() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };
            result.map_err(|err| format!("failed to remove existing {}: {err}", path.display()))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "failed to inspect existing {}: {err}",
            path.display()
        )),
    }
}

fn apply_macos_rustc_wrapper_workaround(cmd: &mut Command) {
    if should_strip_rustc_wrapper(cfg!(target_os = "macos")) {
        // macOS Sequoia/Tahoe stamps files with com.apple.provenance based on the
        // creating process. sccache-written intermediates can then fail later rustc
        // overwrites with "Operation not permitted"; Linux keeps its wrapper.
        cmd.env_remove("RUSTC_WRAPPER");
    }
}

fn should_strip_rustc_wrapper(is_macos: bool) -> bool {
    is_macos
}

fn run_status(cmd: &mut Command, label: &str) -> Result<(), String> {
    eprintln!("==> {label}");
    let status = cmd
        .status()
        .map_err(|err| format!("failed to spawn {label}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed (status {status})"))
    }
}

fn run_output(cmd: &mut Command, label: &str) -> Result<String, String> {
    eprintln!("==> {label}");
    let output = cmd
        .output()
        .map_err(|err| format!("failed to spawn {label}: {err}"))?;
    if !output.status.success() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        return Err(format!("{label} failed (status {})", output.status));
    }
    String::from_utf8(output.stdout)
        .map_err(|err| format!("{label} produced non-UTF8 output: {err}"))
}

pub(crate) fn command_args(command: &Command) -> Vec<String> {
    command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

pub(crate) fn cargo() -> PathBuf {
    env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"))
}

fn workspace_root() -> PathBuf {
    workspace_root_from_env(env::var_os("CARGO_MANIFEST_DIR"))
}

/// Resolve from the invoking cargo's runtime environment, never from
/// compile-time `env!()`: the xtask binary can be a stale cache hit compiled in
/// a different copy of the workspace (remote per-run dirs share one target
/// dir), and a baked-in manifest path would silently point every subcommand
/// at that older tree.
fn workspace_root_from_env(manifest_dir: Option<std::ffi::OsString>) -> PathBuf {
    manifest_dir
        .map(PathBuf::from)
        .and_then(|dir| dir.parent().map(PathBuf::from))
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn cargo_home_bin() -> PathBuf {
    env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .unwrap_or_else(|| PathBuf::from(".cargo"))
        .join("bin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_prefers_runtime_manifest_dir() {
        let root = workspace_root_from_env(Some("/copy-b/xtask".into()));
        assert_eq!(root, PathBuf::from("/copy-b"));
    }

    #[test]
    fn workspace_root_falls_back_to_current_dir_without_env() {
        let root = workspace_root_from_env(None);
        assert_eq!(root, env::current_dir().expect("current dir"));
    }

    #[test]
    fn cargo_build_command_includes_requested_features() {
        let root = PathBuf::from("/workspace");
        let extra = vec![
            "--locked".to_owned(),
            "--debug".to_owned(),
            "--remote".to_owned(),
        ];

        let command = cargo_build_command(
            &root,
            true,
            &["spur-cli"],
            &["spur-cli/example-feature"],
            &extra,
        );

        assert_eq!(
            command_args(&command),
            vec![
                "build".to_owned(),
                "-p".to_owned(),
                "spur-cli".to_owned(),
                "--features".to_owned(),
                "spur-cli/example-feature".to_owned(),
                "--locked".to_owned(),
            ]
        );
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
    }

    #[test]
    fn remote_install_build_command_defaults_to_green_cli_only_build() {
        let root = PathBuf::from("/workspace");

        let command = remote_install_build_command(&root, NotebookInstallChannel::Auto);

        assert_eq!(
            command.get_program(),
            root.join("scripts/cloud-build/build.sh").as_os_str()
        );
        assert_eq!(
            command_args(&command),
            vec![
                "--auto-spin".to_owned(),
                "--".to_owned(),
                "build".to_owned(),
                "--release".to_owned(),
                "-p".to_owned(),
                "spur-cli".to_owned(),
                "--no-default-features".to_owned(),
                "--features".to_owned(),
                "duckdb-bundled".to_owned(),
                "--locked".to_owned(),
            ]
        );
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
    }

    #[test]
    fn remote_install_build_command_green_builds_cli_only() {
        let root = PathBuf::from("/workspace");

        let command = remote_install_build_command(&root, NotebookInstallChannel::Green);

        assert_eq!(
            command_args(&command),
            vec![
                "--auto-spin".to_owned(),
                "--".to_owned(),
                "build".to_owned(),
                "--release".to_owned(),
                "-p".to_owned(),
                "spur-cli".to_owned(),
                "--no-default-features".to_owned(),
                "--features".to_owned(),
                "duckdb-bundled".to_owned(),
                "--locked".to_owned(),
            ]
        );
    }

    #[test]
    fn remote_install_fetch_command_defaults_to_green_spur_only() {
        let root = PathBuf::from("/workspace");
        let dest = PathBuf::from("/workspace/target/remote-linux-bin");

        let command = remote_install_fetch_command(&root, &dest, NotebookInstallChannel::Auto);

        assert_eq!(
            command.get_program(),
            root.join("scripts/cloud-build/fetch.sh").as_os_str()
        );
        assert_eq!(
            command_args(&command),
            vec![
                "--to".to_owned(),
                dest.join("spur").to_string_lossy().into_owned(),
                "target/release/spur".to_owned(),
            ]
        );
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
    }

    #[test]
    fn remote_install_fetch_command_green_fetches_spur_only() {
        let root = PathBuf::from("/workspace");
        let dest = PathBuf::from("/workspace/target/remote-linux-bin");

        let command = remote_install_fetch_command(&root, &dest, NotebookInstallChannel::Green);
        let args = command_args(&command);

        assert_eq!(
            args,
            vec![
                "--to".to_owned(),
                dest.join("spur").to_string_lossy().into_owned(),
                "target/release/spur".to_owned(),
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--bins"));
        assert!(!args.iter().any(|arg| arg.contains("spur-notebook")));
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
    }

    #[test]
    fn remote_install_fetch_label_uses_cloud_build() {
        assert_eq!(
            remote_install_fetch_label(NotebookInstallChannel::Auto),
            "scripts/cloud-build/fetch.sh target/release/spur"
        );
    }

    #[test]
    fn remote_install_dest_uses_cargo_bin_on_linux_host() {
        let root = PathBuf::from("/workspace");

        let dest = remote_install_dest(&root, true, false);

        assert!(!dest.staged);
        assert_eq!(dest.dir, cargo_home_bin());
    }

    #[test]
    fn remote_install_dest_stages_foreign_binaries_off_linux() {
        let root = PathBuf::from("/workspace");

        let dest = remote_install_dest(&root, false, false);

        assert!(dest.staged);
        assert_eq!(dest.dir, root.join("target/remote-linux-bin"));
    }

    #[test]
    fn remote_install_dest_force_clobbers_cargo_bin_off_linux() {
        let root = PathBuf::from("/workspace");

        let dest = remote_install_dest(&root, false, true);

        assert!(!dest.staged);
        assert_eq!(dest.dir, cargo_home_bin());
    }

    #[test]
    #[cfg(unix)]
    fn remote_install_binaries_copies_skill_assets_to_staged_prefix_share() {
        let workspace = temp_test_dir("remote-workspace");
        let scripts = workspace.join("scripts/cloud-build");
        fs::create_dir_all(&scripts).unwrap();
        let build_script = scripts.join("build.sh");
        fs::write(&build_script, "#!/bin/sh\nexit 0\n").unwrap();
        make_executable(&build_script);
        let fetch_script = scripts.join("fetch.sh");
        fs::write(
            &fetch_script,
            "#!/bin/sh\n\
             while [ \"$#\" -gt 0 ]; do\n\
             \tif [ \"$1\" = \"--to\" ]; then\n\
             \t\tdest=\"$2\"\n\
             \t\tshift 2\n\
             \telse\n\
             \t\tshift\n\
             \tfi\n\
             done\n\
             mkdir -p \"$(dirname \"$dest\")\"\n\
             printf 'fake remote spur' > \"$dest\"\n",
        )
        .unwrap();
        make_executable(&fetch_script);
        let asset = workspace.join("crates/spur-cli/assets/skills/package-skill/SKILL.md");
        fs::create_dir_all(asset.parent().unwrap()).unwrap();
        fs::write(
            &asset,
            "---\nname: package-skill\ndescription: packaged\n---\nbody\n",
        )
        .unwrap();
        let dest = RemoteInstallDest {
            dir: workspace.join("target/remote-linux-bin"),
            staged: true,
        };

        install_remote_linux_binaries(&workspace, &dest, NotebookInstallChannel::Green).unwrap();

        assert_eq!(
            fs::read_to_string(dest.dir.join("spur")).unwrap(),
            "fake remote spur"
        );
        let installed_asset = workspace.join("target/share/spur/skills/package-skill/SKILL.md");
        assert_eq!(
            fs::read_to_string(installed_asset).unwrap(),
            "---\nname: package-skill\ndescription: packaged\n---\nbody\n"
        );
    }

    #[test]
    fn force_flag_is_not_forwarded_to_cargo() {
        assert!(is_xtask_install_flag("--force"));
    }

    #[test]
    fn install_built_binary_copies_skill_assets_to_cargo_home_share() {
        let workspace = temp_test_dir("workspace");
        let cargo_home = temp_test_dir("cargo-home");
        let built = workspace.join("target/debug/spur");
        fs::create_dir_all(built.parent().unwrap()).unwrap();
        fs::write(&built, "fake spur binary").unwrap();
        let asset = workspace.join("crates/spur-cli/assets/skills/package-skill/SKILL.md");
        fs::create_dir_all(asset.parent().unwrap()).unwrap();
        fs::write(
            &asset,
            "---\nname: package-skill\ndescription: packaged\n---\nbody\n",
        )
        .unwrap();

        let output = Command::new(env::current_exe().unwrap())
            .arg("tests::install_built_binary_child_asserts_skill_asset_copy")
            .arg("--exact")
            .arg("--nocapture")
            .env("SPUR_XTASK_INSTALL_ASSET_CHILD", "1")
            .env("SPUR_XTASK_WORKSPACE", &workspace)
            .env("CARGO_HOME", &cargo_home)
            .output()
            .expect("child test process should run");

        assert!(
            output.status.success(),
            "child install test failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("1 passed"),
            "expected exactly one child test to run, got stdout:\n{stdout}"
        );
    }

    #[test]
    fn install_built_binary_child_asserts_skill_asset_copy() {
        if env::var_os("SPUR_XTASK_INSTALL_ASSET_CHILD").is_none() {
            return;
        }
        let workspace = PathBuf::from(env::var_os("SPUR_XTASK_WORKSPACE").unwrap());

        install_built_binary(&workspace, true, "spur").unwrap();

        let installed_asset = cargo_home_bin()
            .parent()
            .unwrap()
            .join("share/spur/skills/package-skill/SKILL.md");
        assert_eq!(
            fs::read_to_string(installed_asset).unwrap(),
            "---\nname: package-skill\ndescription: packaged\n---\nbody\n"
        );
    }

    #[test]
    fn dist_workspace_includes_skill_assets_for_archives_and_installers() {
        let raw = fs::read_to_string(workspace_root().join("dist-workspace.toml")).unwrap();

        assert!(
            raw.contains("include = [\"crates/spur-cli/assets\"]"),
            "dist-workspace.toml must include the spur-cli asset tree"
        );
    }

    #[test]
    fn spur_cli_cargo_package_includes_build_script_and_skill_assets() {
        let raw = fs::read_to_string(workspace_root().join("crates/spur-cli/Cargo.toml")).unwrap();

        assert!(
            raw.contains(
                "include = [\"src/**\", \"tests/**\", \"build.rs\", \"assets/skills/**\"]"
            ),
            "spur-cli's Cargo package whitelist must preserve build.rs and bundled skills"
        );
    }

    #[test]
    fn dist_skills_archive_name_is_versioned_and_platform_independent() {
        assert_eq!(
            dist_skills_archive_name("1.15.0"),
            "spur-skills-1.15.0.tar.gz"
        );
    }

    #[test]
    fn dist_duckdb_extensions_archive_name_is_versioned_and_platform_independent() {
        assert_eq!(
            dist_duckdb_extensions_archive_name("1.21.0"),
            "spur-duckdb-extensions-1.21.0.tar.gz"
        );
    }

    #[test]
    fn dist_platforms_map_to_duckdb_extension_cdn_names() {
        assert_eq!(DistPlatform::MacAarch64.duckdb_platform(), "osx_arm64");
        assert_eq!(DistPlatform::MacX64.duckdb_platform(), "osx_amd64");
        assert_eq!(DistPlatform::LinuxAarch64.duckdb_platform(), "linux_arm64");
        assert_eq!(DistPlatform::LinuxX64Gnu.duckdb_platform(), "linux_amd64");
        assert_eq!(DistPlatform::WindowsX64.duckdb_platform(), "windows_amd64");
    }

    #[test]
    fn vendor_duckdb_extensions_script_fetches_community_and_core() {
        let script =
            fs::read_to_string(workspace_root().join("scripts/vendor-duckdb-extensions.sh"))
                .unwrap();
        assert!(script.contains("community-extensions.duckdb.org"));
        assert!(script.contains("extensions.duckdb.org"));
        assert!(script.contains("onager"));
        assert!(script.contains("duckpgq"));
        assert!(script.contains("SPUR_DUCKDB_EXTENSION_DIR"));
    }

    /// GitHub `gh release create dist/*` can only upload files. The vendor
    /// tree must land as `spur-duckdb-extensions-<ver>.tar.gz`, not a directory.
    #[cfg(unix)]
    #[test]
    fn package_dist_duckdb_extensions_ships_tarball_not_directory() {
        let workspace = temp_test_dir("dist-duckdb-workspace");
        let out_dir = temp_test_dir("dist-duckdb-out");
        let script = workspace.join("scripts/vendor-duckdb-extensions.sh");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(
            &script,
            r#"#!/usr/bin/env bash
set -euo pipefail
OUT=""
PLATFORMS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --platform) PLATFORMS+=("$2"); shift 2 ;;
    --version) shift 2 ;;
    *) shift ;;
  esac
done
mkdir -p "${OUT}/v1.4.4/${PLATFORMS[0]}"
printf 'dummy-ext\n' > "${OUT}/v1.4.4/${PLATFORMS[0]}/lance.duckdb_extension"
"#,
        )
        .unwrap();
        make_executable(&script);

        let archive = package_dist_duckdb_extensions(
            &workspace,
            &out_dir,
            "9.9.9",
            &[DistPlatform::LinuxX64Gnu],
        )
        .expect("package duckdb extensions");
        assert_eq!(
            archive.file_name().and_then(|n| n.to_str()),
            Some("spur-duckdb-extensions-9.9.9.tar.gz")
        );
        assert!(
            archive.is_file(),
            "GitHub release upload requires a file, got {}",
            archive.display()
        );
        assert!(
            !out_dir.join("duckdb-extensions").exists(),
            "unpacked duckdb-extensions dir must not remain in dist/"
        );
        assert!(
            !out_dir.join(".duckdb-extensions-stage").exists(),
            "stage dir should be removed after packaging"
        );

        let listing = Command::new("tar")
            .args(["-tzf"])
            .arg(&archive)
            .output()
            .expect("tar -tzf");
        assert!(
            listing.status.success(),
            "tar -tzf failed: {}",
            String::from_utf8_lossy(&listing.stderr)
        );
        let listing = String::from_utf8_lossy(&listing.stdout);
        assert!(
            listing
                .contains("share/spur/duckdb-extensions/v1.4.4/linux_amd64/lance.duckdb_extension"),
            "archive must use share/spur/duckdb-extensions layout; got:\n{listing}"
        );
    }

    /// Dist ships macos-x64, but DuckDB 1.4.4 does not publish every extension
    /// for `osx_amd64` (lance and onager 404). Vendoring must skip those files
    /// instead of failing the whole `xtask dist` job.
    #[cfg(unix)]
    #[test]
    fn vendor_duckdb_extensions_skips_cdn_404_and_keeps_available_files() {
        let bin = temp_test_dir("fake-curl-bin");
        let out = temp_test_dir("duckdb-ext-out");
        let curl = bin.join("curl");
        fs::write(
            &curl,
            r#"#!/usr/bin/env bash
set -euo pipefail
out=""
url=""
write_http=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o|--output) out="$2"; shift 2 ;;
    -w|--write-out) write_http=1; shift 2 ;;
    -s|-S|-f|-L|-sS|-fsSL|-sSL|-fsS|--fail|--location|--silent|--show-error) shift ;;
    http*) url="$1"; shift ;;
    *) shift ;;
  esac
done
if [[ -z "$url" ]]; then
  echo "fake curl: missing url" >&2
  exit 2
fi
missing=0
case "$url" in
  *osx_amd64/lance.duckdb_extension.gz*|*osx_amd64/onager.duckdb_extension.gz*) missing=1 ;;
esac
if [[ "$missing" -eq 1 ]]; then
  if [[ "$write_http" -eq 1 ]]; then
    echo -n 404
    exit 0
  fi
  echo "curl: (22) The requested URL returned error: 404" >&2
  exit 22
fi
payload=$'FAKE-EXT\n'
if [[ -n "$out" ]]; then
  printf '%s' "$payload" | gzip > "$out"
else
  printf '%s' "$payload" | gzip
fi
if [[ "$write_http" -eq 1 ]]; then
  echo -n 200
fi
"#,
        )
        .unwrap();
        make_executable(&curl);

        let script = workspace_root().join("scripts/vendor-duckdb-extensions.sh");
        let path = format!("{}:{}", bin.display(), env::var("PATH").unwrap());
        let output = Command::new("bash")
            .arg(&script)
            .arg("--out")
            .arg(&out)
            .arg("--platform")
            .arg("osx_amd64")
            .env("PATH", path)
            .output()
            .expect("vendor script should run");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "vendor script should skip CDN 404s\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );

        let plat = out.join("v1.4.4/osx_amd64");
        assert!(
            !plat.join("lance.duckdb_extension").exists(),
            "missing CDN file must not be vendored"
        );
        assert!(
            !plat.join("onager.duckdb_extension").exists(),
            "missing CDN file must not be vendored"
        );
        for name in ["fts", "icu", "duckpgq"] {
            assert!(
                plat.join(format!("{name}.duckdb_extension")).is_file(),
                "{name} should still be vendored"
            );
        }
        let combined = format!("{stdout}{stderr}");
        assert!(
            combined.contains("skip 404"),
            "expected skip 404 log, got:\n{combined}"
        );
    }

    #[test]
    fn package_dist_skill_assets_ships_share_spur_skills_layout() {
        let workspace = temp_test_dir("dist-skills-workspace");
        let out_dir = temp_test_dir("dist-skills-out");
        let asset = workspace.join("crates/spur-cli/assets/skills/package-skill/SKILL.md");
        fs::create_dir_all(asset.parent().unwrap()).unwrap();
        fs::write(
            &asset,
            "---\nname: package-skill\ndescription: packaged\n---\nbody\n",
        )
        .unwrap();

        let archive =
            package_dist_skill_assets(&workspace, &out_dir, "9.9.9").expect("package skills");
        assert_eq!(
            archive.file_name().and_then(|n| n.to_str()),
            Some("spur-skills-9.9.9.tar.gz")
        );
        assert!(
            archive.is_file(),
            "archive should exist at {}",
            archive.display()
        );
        // Staging dir must not leak into dist/.
        assert!(
            !out_dir.join(".skills-stage").exists(),
            "stage dir should be removed after packaging"
        );

        let listing = Command::new("tar")
            .args(["-tzf"])
            .arg(&archive)
            .output()
            .expect("tar -tzf");
        assert!(
            listing.status.success(),
            "tar -tzf failed: {}",
            String::from_utf8_lossy(&listing.stderr)
        );
        let listing = String::from_utf8_lossy(&listing.stdout);
        assert!(
            listing.contains("share/spur/skills/package-skill/SKILL.md"),
            "archive must use share/spur/skills layout for SkillCatalog package candidates; got:\n{listing}"
        );

        // Extract into a fake install prefix and confirm package-relative path.
        let prefix = temp_test_dir("dist-skills-prefix");
        let status = Command::new("tar")
            .args(["-xzf"])
            .arg(&archive)
            .arg("-C")
            .arg(&prefix)
            .status()
            .expect("tar -xzf");
        assert!(status.success());
        let extracted = prefix.join("share/spur/skills/package-skill/SKILL.md");
        assert_eq!(
            fs::read_to_string(extracted).unwrap(),
            "---\nname: package-skill\ndescription: packaged\n---\nbody\n"
        );
    }

    #[test]
    fn dist_artifacts_in_includes_skills_tarball() {
        let out_dir = temp_test_dir("dist-artifacts-skills");
        fs::write(out_dir.join("spur-1.0.0-aarch64-unknown-linux-gnu"), b"bin").unwrap();
        fs::write(out_dir.join("spur-skills-1.0.0.tar.gz"), b"skills").unwrap();
        fs::write(
            out_dir.join("spur-duckdb-extensions-1.0.0.tar.gz"),
            b"duckdb",
        )
        .unwrap();
        fs::write(out_dir.join("SHA256SUMS"), b"ignore").unwrap();
        fs::write(out_dir.join("notes.txt"), b"ignore").unwrap();
        fs::create_dir(out_dir.join("duckdb-extensions")).unwrap();

        let names: Vec<String> = dist_artifacts_in(&out_dir)
            .unwrap()
            .into_iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "spur-1.0.0-aarch64-unknown-linux-gnu".to_owned(),
                "spur-duckdb-extensions-1.0.0.tar.gz".to_owned(),
                "spur-skills-1.0.0.tar.gz".to_owned(),
            ]
        );
    }

    #[test]
    fn notebook_channel_parser_accepts_flag_and_equals_forms() {
        let parsed = parse_install_options(vec![
            "--notebook-channel".to_owned(),
            "green".to_owned(),
            "--locked".to_owned(),
        ])
        .expect("green channel should parse");

        assert_eq!(parsed.notebook_channel, NotebookInstallChannel::Green);
        assert_eq!(parsed.cargo_args, vec!["--locked".to_owned()]);

        let parsed = parse_install_options(vec!["--notebook-channel=auto".to_owned()])
            .expect("auto channel should parse");

        assert_eq!(parsed.notebook_channel, NotebookInstallChannel::Auto);
        assert!(parsed.cargo_args.is_empty());
    }

    #[test]
    fn notebook_channel_parser_defaults_to_green_and_rejects_invalid_values() {
        let parsed = parse_install_options(vec![]).expect("default options should parse");
        assert_eq!(parsed.notebook_channel, NotebookInstallChannel::Green);

        let error =
            parse_install_options(vec!["--notebook-channel".to_owned(), "purple".to_owned()])
                .expect_err("invalid channel should fail");

        assert!(error.contains("--notebook-channel"));
        assert!(error.contains("green"));
        assert!(error.contains("auto"));
    }

    #[test]
    fn notebook_channel_parser_rejects_removed_blue_source_install() {
        let error = parse_install_options(vec!["--notebook-channel=blue".to_owned()])
            .expect_err("blue channel source install should be removed");

        assert!(error.contains("blue notebook install source was removed"));
        assert!(error.contains("getspur/spur-notebook"));
    }

    #[test]
    fn install_options_parser_accepts_local_flag() {
        let parsed = parse_install_options(vec!["--local".to_owned(), "--locked".to_owned()])
            .expect("--local should parse");

        assert!(parsed.local);
        assert_eq!(parsed.cargo_args, vec!["--locked".to_owned()]);

        let parsed = parse_install_options(vec![]).expect("default options should parse");
        assert!(!parsed.local);
        assert!(!parsed.disable_notebook_mcp);
    }

    #[test]
    fn install_options_parser_accepts_disable_notebook_mcp_flag() {
        let parsed = parse_install_options(vec!["--disable-notebook-mcp".to_owned()])
            .expect("--disable-notebook-mcp should parse");

        assert!(parsed.disable_notebook_mcp);
        assert_eq!(
            parsed.cargo_args,
            vec!["--features".to_owned(), "disable-notebook-mcp".to_owned()]
        );

        // Already requested via cargo args — do not double-append.
        let parsed = parse_install_options(vec![
            "--disable-notebook-mcp".to_owned(),
            "--features".to_owned(),
            "disable-notebook-mcp".to_owned(),
        ])
        .expect("dedupe features");
        assert_eq!(
            parsed.cargo_args,
            vec!["--features".to_owned(), "disable-notebook-mcp".to_owned()]
        );
    }

    #[test]
    fn darwin_triple_maps_host_arches_and_rejects_others() {
        assert_eq!(
            darwin_triple_for_arch("aarch64").expect("aarch64 maps"),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            darwin_triple_for_arch("x86_64").expect("x86_64 maps"),
            "x86_64-apple-darwin"
        );
        assert!(darwin_triple_for_arch("riscv64").is_err());
    }

    #[test]
    fn zigbuild_install_build_command_dispatches_spur_cargo_zigbuild() {
        let root = PathBuf::from("/workspace");
        let extra = vec!["--locked".to_owned(), "--local".to_owned()];

        let command = zigbuild_install_build_command(&root, false, "aarch64-apple-darwin", &extra);

        assert_eq!(
            command.get_program(),
            root.join("scripts/spur-cargo").as_os_str()
        );
        assert_eq!(
            command_args(&command),
            vec![
                "zigbuild".to_owned(),
                "--release".to_owned(),
                "-p".to_owned(),
                "spur-cli".to_owned(),
                "--target".to_owned(),
                "aarch64-apple-darwin".to_owned(),
                "--locked".to_owned(),
            ],
            "xtask install flags must not leak into the zigbuild argv"
        );
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
    }

    #[test]
    fn zigbuild_install_build_command_debug_omits_release() {
        let root = PathBuf::from("/workspace");

        let command = zigbuild_install_build_command(&root, true, "aarch64-apple-darwin", &[]);

        let args = command_args(&command);
        assert!(!args.iter().any(|arg| arg == "--release"));
        assert_eq!(args[0], "zigbuild");
    }

    #[test]
    fn zigbuild_install_fetch_command_pulls_macho_via_s3() {
        let root = PathBuf::from("/workspace");

        let command = zigbuild_install_fetch_command(&root, "aarch64-apple-darwin", false);

        assert_eq!(
            command.get_program(),
            root.join("scripts/cloud-build/fetch.sh").as_os_str()
        );
        assert_eq!(
            command_args(&command),
            vec![
                "--via-s3".to_owned(),
                "--to".to_owned(),
                root.join("target/aarch64-apple-darwin/release/spur")
                    .to_string_lossy()
                    .into_owned(),
                "target/aarch64-apple-darwin/release/spur".to_owned(),
            ]
        );
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
    }

    #[test]
    fn dist_options_default_to_all_platforms() {
        let options = parse_dist_options(Vec::new()).expect("no args parse");
        assert_eq!(
            options.platforms,
            vec![
                DistPlatform::LinuxAarch64,
                DistPlatform::LinuxX64Gnu,
                DistPlatform::MacAarch64,
                DistPlatform::MacX64,
                DistPlatform::WindowsX64,
            ]
        );
        assert!(options.out_dir.is_none());
        assert!(!options.parallel);
    }

    #[test]
    fn dist_options_accept_parallel_flag() {
        let options = parse_dist_options(vec![
            "--parallel".to_owned(),
            "--platforms".to_owned(),
            "linux,windows".to_owned(),
        ])
        .expect("--parallel parses");
        assert!(options.parallel);
        assert_eq!(
            options.platforms,
            vec![DistPlatform::LinuxAarch64, DistPlatform::WindowsX64]
        );
    }

    #[test]
    fn dist_namespaces_isolate_each_platform_leg() {
        assert_eq!(
            DistPlatform::LinuxAarch64.remote_namespace(),
            "spur-dist-linux"
        );
        assert_eq!(
            DistPlatform::MacAarch64.remote_namespace(),
            "spur-dist-macos-arm64"
        );
        assert_eq!(
            DistPlatform::MacX64.remote_namespace(),
            "spur-dist-macos-x64"
        );
        assert_eq!(
            DistPlatform::WindowsX64.remote_namespace(),
            "spur-dist-windows"
        );

        let mut cmd = dist_build_command(&PathBuf::from("/workspace"), DistPlatform::WindowsX64);
        apply_dist_namespace(&mut cmd, DistPlatform::WindowsX64, "4");
        let env_of = |key: &str| {
            cmd.get_envs()
                .find(|(k, _)| *k == std::ffi::OsStr::new(key))
                .and_then(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
        };
        assert_eq!(
            env_of("SPUR_REMOTE_NAMESPACE").as_deref(),
            Some("spur-dist-windows")
        );
        assert_eq!(env_of("SPUR_BUILD_MAX_CONCURRENT").as_deref(), Some("4"));
    }

    #[test]
    fn dist_queue_slots_match_leg_count_unless_overridden() {
        let saved = env::var("SPUR_BUILD_MAX_CONCURRENT").ok();

        env::remove_var("SPUR_BUILD_MAX_CONCURRENT");
        assert_eq!(dist_queue_slots(4), "4");

        env::set_var("SPUR_BUILD_MAX_CONCURRENT", "2");
        assert_eq!(dist_queue_slots(4), "2");

        env::set_var("SPUR_BUILD_MAX_CONCURRENT", "");
        assert_eq!(dist_queue_slots(3), "3");

        match saved {
            Some(value) => env::set_var("SPUR_BUILD_MAX_CONCURRENT", value),
            None => env::remove_var("SPUR_BUILD_MAX_CONCURRENT"),
        }
    }

    #[test]
    fn dist_options_accept_platform_subset_and_out_dir() {
        let options = parse_dist_options(vec![
            "--platforms".to_owned(),
            "windows,macos".to_owned(),
            "--out=/tmp/spur-dist".to_owned(),
        ])
        .expect("subset parses");
        assert_eq!(
            options.platforms,
            vec![
                DistPlatform::WindowsX64,
                DistPlatform::MacAarch64,
                DistPlatform::MacX64,
            ]
        );
        assert_eq!(options.out_dir, Some(PathBuf::from("/tmp/spur-dist")));

        let options = parse_dist_options(vec!["--platforms".to_owned(), "linux-x64".to_owned()])
            .expect("linux-x64 parses");
        assert_eq!(options.platforms, vec![DistPlatform::LinuxX64Gnu]);

        let options = parse_dist_options(vec![
            "--platforms".to_owned(),
            "macos-x64,macos-arm64".to_owned(),
        ])
        .expect("architecture-specific macOS platforms parse");
        assert_eq!(
            options.platforms,
            vec![DistPlatform::MacX64, DistPlatform::MacAarch64]
        );

        assert!(parse_dist_options(vec!["--platforms".to_owned(), "beos".to_owned()]).is_err());
        assert!(parse_dist_options(vec!["--platforms".to_owned(), ",".to_owned()]).is_err());
        assert!(parse_dist_options(vec!["--frobnicate".to_owned()]).is_err());
    }

    #[test]
    fn dist_build_commands_dispatch_spur_cargo_per_platform() {
        let root = PathBuf::from("/workspace");

        for (platform, expected) in [
            (
                DistPlatform::LinuxAarch64,
                vec!["build", "--release", "-p", "spur-cli"],
            ),
            (
                DistPlatform::LinuxX64Gnu,
                vec![
                    "zigbuild",
                    "--release",
                    "-p",
                    "spur-cli",
                    "--target",
                    "x86_64-unknown-linux-gnu",
                ],
            ),
            (
                DistPlatform::MacAarch64,
                vec![
                    "zigbuild",
                    "--release",
                    "-p",
                    "spur-cli",
                    "--target",
                    "aarch64-apple-darwin",
                ],
            ),
            (
                DistPlatform::MacX64,
                vec![
                    "zigbuild",
                    "--release",
                    "-p",
                    "spur-cli",
                    "--target",
                    "x86_64-apple-darwin",
                ],
            ),
            (
                DistPlatform::WindowsX64,
                vec![
                    "xwin",
                    "build",
                    "--release",
                    "-p",
                    "spur-cli",
                    "--target",
                    "x86_64-pc-windows-msvc",
                ],
            ),
        ] {
            let command = dist_build_command(&root, platform);
            assert_eq!(
                command.get_program(),
                root.join("scripts/spur-cargo").as_os_str()
            );
            assert_eq!(
                command_args(&command),
                expected
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<String>>()
            );
            assert_eq!(command.get_current_dir(), Some(root.as_path()));
        }
    }

    #[test]
    fn dist_fetch_command_pulls_platform_artifact_to_dest() {
        let root = PathBuf::from("/workspace");
        let dest = PathBuf::from("/workspace/dist/spur-1.7.0-x86_64-pc-windows-msvc.exe");

        let command = dist_fetch_command(&root, DistPlatform::WindowsX64, &dest);

        assert_eq!(
            command.get_program(),
            root.join("scripts/cloud-build/fetch.sh").as_os_str()
        );
        assert_eq!(
            command_args(&command),
            vec![
                "--via-s3".to_owned(),
                "--to".to_owned(),
                dest.to_string_lossy().into_owned(),
                "target/x86_64-pc-windows-msvc/release/spur.exe".to_owned(),
            ]
        );
    }

    #[test]
    fn dist_artifact_names_carry_version_and_triple() {
        assert_eq!(
            DistPlatform::LinuxAarch64.artifact_name("1.7.0"),
            "spur-1.7.0-aarch64-unknown-linux-gnu"
        );
        assert_eq!(
            DistPlatform::LinuxX64Gnu.artifact_name("1.7.0"),
            "spur-1.7.0-x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            DistPlatform::MacAarch64.artifact_name("1.7.0"),
            "spur-1.7.0-aarch64-apple-darwin"
        );
        assert_eq!(
            DistPlatform::MacX64.artifact_name("1.7.0"),
            "spur-1.7.0-x86_64-apple-darwin"
        );
        assert_eq!(
            DistPlatform::WindowsX64.artifact_name("1.7.0"),
            "spur-1.7.0-x86_64-pc-windows-msvc.exe"
        );
    }

    #[test]
    fn workspace_version_reads_workspace_package_section_only() {
        let manifest = r#"
[workspace]
members = ["crates/spur-cli"]

[workspace.package]
edition = "2021"
version = "1.7.0"

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
"#;
        assert_eq!(parse_workspace_version(manifest).as_deref(), Some("1.7.0"));

        let no_version = "[workspace.dependencies]\ntokio = { version = \"1\" }\n";
        assert_eq!(parse_workspace_version(no_version), None);
    }

    #[test]
    fn linux_install_build_command_defaults_to_green_cli_only_build() {
        let root = PathBuf::from("/workspace");
        let extra = vec!["--locked".to_owned()];

        let command =
            linux_install_build_command(&root, false, &extra, NotebookInstallChannel::Auto);

        assert_eq!(
            command_args(&command),
            vec![
                "build".to_owned(),
                "--release".to_owned(),
                "-p".to_owned(),
                "spur-cli".to_owned(),
                "--locked".to_owned(),
            ]
        );
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
    }

    #[test]
    fn linux_install_build_command_green_builds_cli_only() {
        let root = PathBuf::from("/workspace");
        let extra = vec!["--locked".to_owned()];

        let command =
            linux_install_build_command(&root, false, &extra, NotebookInstallChannel::Green);

        assert_eq!(
            command_args(&command),
            vec![
                "build".to_owned(),
                "--release".to_owned(),
                "-p".to_owned(),
                "spur-cli".to_owned(),
                "--locked".to_owned(),
            ]
        );
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
    }

    #[test]
    fn green_channel_reports_standalone_notebook_guidance() {
        let guidance = green_notebook_install_guidance();

        assert!(guidance.contains("SPUR_NOTEBOOK_CHANNEL=green"));
        assert!(guidance.contains("SPUR_NOTEBOOK_BIN"));
        assert!(guidance.contains("getspur/spur-notebook"));
        assert!(guidance.contains("SpurLab.app"));
        assert!(guidance.contains("$CARGO_HOME/bin/spur-notebook"));
    }

    #[test]
    fn rustc_wrapper_strip_is_macos_gated() {
        assert!(should_strip_rustc_wrapper(true));
        assert!(!should_strip_rustc_wrapper(false));
    }

    #[test]
    fn release_dist_npm_publish_uses_trusted_publisher_oidc() {
        // Sat model sol_dbb5643359a94bb1 / sol_e8b89417bf904b3b: the live
        // @getspur/spur-cli publish must use GitHub Actions OIDC (Node 24,
        // id-token: write, GitHub-hosted runner) and must not set NODE_AUTH_TOKEN.
        let yaml = fs::read_to_string(workspace_root().join(".github/workflows/release-dist.yml"))
            .expect("release-dist.yml");

        assert!(
            yaml.contains("id-token: write"),
            "npm trusted publishing requires permissions.id-token: write"
        );
        assert!(
            yaml.contains("runs-on: ubuntu-24.04"),
            "npm trusted publishing requires a GitHub-hosted runner; do not publish from the Spot VM"
        );

        let setup = yaml
            .split("actions/setup-node@")
            .nth(1)
            .expect("setup-node step for npm publish")
            .split("\n      - name:")
            .next()
            .expect("setup-node with: block");
        assert!(
            setup.contains("node-version: \"24\"") || setup.contains("node-version: '24'"),
            "official trusted-publisher recipe pins Node 24 so bundled npm is >= 11.5.1; got:\n{setup}"
        );
        assert!(
            setup.contains("package-manager-cache: false"),
            "release publishes must disable setup-node package-manager cache; got:\n{setup}"
        );
        assert!(
            setup.contains("registry-url: \"https://registry.npmjs.org\"")
                || setup.contains("registry-url: 'https://registry.npmjs.org'"),
            "setup-node must target registry.npmjs.org; got:\n{setup}"
        );

        let npm = yaml
            .split("Publish npm package")
            .nth(1)
            .expect("Publish npm package step");
        assert!(
            npm.contains("npm publish --access public"),
            "expected public npm publish of @getspur/spur-cli; got:\n{npm}"
        );
        let npm_env = npm
            .split("env:")
            .nth(1)
            .expect("npm step env")
            .split("run:")
            .next()
            .expect("env before run");
        assert!(
            !npm_env.contains("NODE_AUTH_TOKEN") && !npm_env.contains("NPM_TOKEN"),
            "trusted publishing must not set NODE_AUTH_TOKEN; npm CLI uses OIDC. env:\n{npm_env}"
        );
        assert!(
            !yaml.contains("secrets.NPM_TOKEN         —"),
            "required-config comment must not list secrets.NPM_TOKEN as a publish credential"
        );
    }

    #[test]
    fn cargo_dist_release_yml_does_not_token_publish() {
        // npmjs.com allows one trusted publisher per package. Tag publishes go
        // through release-dist.yml; cargo-dist must not keep a NODE_AUTH_TOKEN
        // npm job that could republish @getspur/spur-cli.
        let yaml = fs::read_to_string(workspace_root().join(".github/workflows/release.yml"))
            .expect("release.yml");
        assert!(
            yaml.contains("on:\n  pull_request:"),
            "cargo-dist release.yml must stay PR-plan-only after the release-dist cutover"
        );
        assert!(
            !yaml.contains("NODE_AUTH_TOKEN:"),
            "leftover cargo-dist npm publish must not set NODE_AUTH_TOKEN"
        );
        let dist = fs::read_to_string(workspace_root().join("dist-workspace.toml"))
            .expect("dist-workspace.toml");
        assert!(
            !dist.contains("publish-jobs = [\"npm\"]"),
            "cargo-dist must not generate npm publish jobs; trusted publishing lives in release-dist.yml"
        );
    }

    #[test]
    fn npm_spur_cli_repository_matches_provenance_source_repo() {
        // npm E422 on v1.22.0: Sigstore provenance is issued for getspur/spur
        // (the Actions workflow repo). package.json repository.url must match.
        let pkg = fs::read_to_string(workspace_root().join("npm/spur-cli/package.json"))
            .expect("npm/spur-cli/package.json");
        assert!(
            pkg.contains("\"url\": \"git+https://github.com/getspur/spur.git\""),
            "repository.url must be getspur/spur so npm provenance verifies; got:\n{pkg}"
        );
        assert!(
            pkg.contains("\"homepage\": \"https://github.com/getspur/spur\""),
            "homepage must match the provenance source repo; got:\n{pkg}"
        );
        assert!(
            !pkg.contains("github.com/getspur/spur-releases.git"),
            "repository.url must not point at the binary-hosting repo"
        );
    }

    fn temp_test_dir(label: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = env::temp_dir().join(format!("spur-xtask-{label}-{}-{n}", std::process::id()));
        fs::create_dir(&path).unwrap();
        path
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}
