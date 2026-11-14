use std::path::Path;
use std::process::Command;

use spur_graph::extract::languages::Language;
use spur_graph::extract::tree_sitter::{BytesExtractor, ExtractedSymbol};
use spur_graph::git_walk::{
    file_changes_for_commit, merge_symbol_changes, symbol_changes_for_commit,
    symbol_changes_from_extracted, FileChange, FileChangeKind, SymbolChange, SymbolDiffCtx,
};
use spur_graph::schema::{ChangeKind, GitPath, RenamePrev};
use tempfile::TempDir;

#[test]
fn preparsed_matching_keeps_last_left_duplicate_identity() {
    let earlier = symbol("overload", 0, "earlier", &["earlier"]);
    let last = symbol("overload", 20, "last", &["last"]);
    let change = file_change("lib.rs", FileChangeKind::Modified, Some("parent"));

    let (changes, diagnostics) = symbol_changes_from_extracted(
        "current",
        &change,
        &[earlier.clone(), last.clone()],
        std::slice::from_ref(&last),
    );
    assert!(
        changes.is_empty(),
        "the last left overload is the identity match"
    );
    assert!(diagnostics.is_empty());

    let (changes, _) =
        symbol_changes_from_extracted("current", &change, &[earlier.clone(), last], &[earlier]);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].change_kind, ChangeKind::Modified);
}

#[test]
fn preparsed_matching_matches_commit_walker_across_history() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    write(
        dir.path(),
        "lib.rs",
        b"pub fn overload(x: u32) -> u32 { x + 1 }\npub fn overload(x: u64) -> u64 { x + 2 }\npub fn removed() -> u32 { 77 }\n",
    );
    let root = commit(dir.path(), "root with duplicate identities");
    git(dir.path(), &["branch", "side"]);
    write(
        dir.path(),
        "lib.rs",
        b"pub fn overload(x: u32) -> u32 { x + 1 }\npub fn overload(x: u64) -> u64 { x + 3 }\n",
    );
    let modified = commit(dir.path(), "modify overload and delete symbol");
    git(dir.path(), &["mv", "lib.rs", "renamed.rs"]);
    let renamed = commit(dir.path(), "rename file");
    git(dir.path(), &["checkout", "-q", "side"]);
    write(
        dir.path(),
        "side.rs",
        b"pub fn side_symbol() -> u32 { 20 }\n",
    );
    let side = commit(dir.path(), "side branch");
    git(dir.path(), &["checkout", "-q", "main"]);
    git(
        dir.path(),
        &["merge", "--no-ff", "-q", "side", "-m", "merge"],
    );
    let merged = revision(dir.path(), "HEAD");

    for sha in [&root, &modified, &renamed, &side, &merged] {
        let file_changes = file_changes_for_commit(dir.path(), sha).unwrap();
        if sha == &root {
            assert!(file_changes
                .iter()
                .all(|change| change.parent_sha.is_none()));
        }
        if sha == &renamed {
            assert!(file_changes
                .iter()
                .any(|change| matches!(change.kind, FileChangeKind::Renamed { .. })));
        }
        if sha == &merged {
            let mut parents: Vec<_> = file_changes.iter().map(|c| &c.parent_sha).collect();
            parents.sort();
            parents.dedup();
            assert_eq!(parents.len(), 2);
        }
        assert_preparsed_matches_walker(dir.path(), sha, &file_changes);
    }
}

#[test]
fn rename_uses_current_path_language_and_current_sha_for_predecessor() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    write(dir.path(), "old.go", b"pub fn preserved() -> u32 { 1 }\n");
    let parent = commit(dir.path(), "rust source at old extension");
    git(dir.path(), &["mv", "old.go", "new.rs"]);
    let sha = commit(dir.path(), "change extension");
    let change = file_change(
        "new.rs",
        FileChangeKind::Renamed {
            from: GitPath::from("old.go"),
        },
        Some(&parent),
    );

    let changes = assert_preparsed_matches_walker(dir.path(), &sha, &[change]);
    assert_eq!(changes.len(), 1);
    let ChangeKind::RenamedFrom(RenamePrev::Symbol(previous)) = &changes[0].change_kind else {
        panic!("expected rename: {changes:?}");
    };
    assert_eq!(previous.commit, sha);
    assert_eq!(changes[0].snapshot.key.commit, sha);
}

#[test]
fn preparsed_rename_tier_preserves_duplicate_input_order() {
    let first = symbol("overload", 0, "first", &["first"]);
    let second = symbol("overload", 20, "second", &["second"]);
    let change = file_change(
        "new.rs",
        FileChangeKind::Renamed {
            from: GitPath::from("old.rs"),
        },
        Some("parent"),
    );
    let left = [first, second];
    let (changes, diagnostics) = symbol_changes_from_extracted(
        "current",
        &change,
        &left,
        &[left[1].clone(), left[0].clone()],
    );
    assert!(diagnostics.is_empty());
    assert_eq!(changes.len(), 2);
    let (old, _) = symbol_changes_from_extracted(
        "current",
        &file_change("old.rs", FileChangeKind::Added, None),
        &[],
        &left,
    );
    for (renamed, predecessor) in changes.iter().zip(old) {
        assert_eq!(
            renamed.change_kind,
            ChangeKind::RenamedFrom(RenamePrev::Symbol(predecessor.snapshot.key))
        );
    }
    assert_eq!(changes[0].snapshot.anchor_hash, "second");
}

#[test]
fn preparsed_matching_returns_ambiguity_diagnostics_and_current_deleted_sha() {
    let left = [
        symbol("before_a", 0, "a", &["same", "body"]),
        symbol("before_b", 20, "b", &["same", "body"]),
    ];
    let right = [symbol("after", 40, "c", &["same", "body"])];
    let change = file_change("lib.rs", FileChangeKind::Modified, Some("parent"));
    let (changes, diagnostics) = symbol_changes_from_extracted("current", &change, &left, &right);
    assert_eq!(changes.len(), 3);
    assert_eq!(diagnostics.len(), 5);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.starts_with("ambiguous_rename:"))
            .count(),
        4
    );
    assert!(diagnostics
        .iter()
        .any(|d| d == "merge_collision: file=lib.rs candidate=after"));
    assert_eq!(
        changes
            .iter()
            .filter(|c| c.change_kind == ChangeKind::Deleted)
            .count(),
        2
    );
    assert!(changes.iter().all(|c| c.snapshot.key.commit == "current"));
}

#[test]
fn preparsed_matching_skips_unsupported_paths_and_gitlinks() {
    let symbols = [symbol("ignored", 0, "anchor", &["body"])];
    for change in [
        file_change("file.unknown", FileChangeKind::Added, None),
        file_change(
            "submodule.rs",
            FileChangeKind::Gitlink {
                old_oid: None,
                new_oid: Some("oid".to_owned()),
            },
            None,
        ),
    ] {
        let (changes, diagnostics) =
            symbol_changes_from_extracted("current", &change, &symbols, &symbols);
        assert!(changes.is_empty());
        assert!(diagnostics.is_empty());
    }
}

#[test]
fn merge_is_parent_scoped_and_retains_first_snapshot_payload() {
    let (changes, _) = symbol_changes_from_extracted(
        "current",
        &file_change("lib.rs", FileChangeKind::Added, Some("parent_a")),
        &[],
        &[symbol("item", 0, "first", &["first"])],
    );
    let first = changes[0].clone();
    let mut conflicting = first.clone();
    conflicting.snapshot.anchor_hash = "later payload".to_owned();
    conflicting.change_kind = ChangeKind::Deleted;
    let mut other_parent = conflicting.clone();
    other_parent.parent_sha = Some("parent_b".to_owned());
    let mut root = first.clone();
    root.parent_sha = None;

    let merged = merge_symbol_changes([
        first.clone(),
        first.clone(),
        conflicting,
        other_parent.clone(),
        root.clone(),
    ]);
    assert_eq!(merged.len(), 3);
    assert_eq!(merged[0].snapshot, first.snapshot);
    assert_eq!(merged[0].change_kind, ChangeKind::Modified);
    assert_eq!(merged[1], other_parent);
    assert_eq!(merged[2], root);
    assert_eq!(
        merge_symbol_changes([first.clone(), first.clone()]),
        vec![first]
    );
    assert!(merge_symbol_changes([]).is_empty());
}

fn assert_preparsed_matches_walker(
    dir: &Path,
    sha: &str,
    files: &[FileChange],
) -> Vec<SymbolChange> {
    let mut parsed_changes = Vec::new();
    let mut parsed_diagnostics = Vec::new();
    for change in files {
        let language = Language::from_path(&change.path.to_path_buf()).unwrap();
        let mut extractor = BytesExtractor::for_language(language).unwrap();
        let left_path = match &change.kind {
            FileChangeKind::Added | FileChangeKind::Gitlink { .. } => None,
            FileChangeKind::Renamed { from } => Some(from),
            _ => Some(&change.path),
        };
        let left = match (change.parent_sha.as_deref(), left_path) {
            (Some(parent), Some(path)) => extractor
                .extract(&path.to_path_buf(), &blob(dir, parent, path))
                .unwrap(),
            _ => Vec::new(),
        };
        let right = if matches!(
            change.kind,
            FileChangeKind::Deleted | FileChangeKind::Gitlink { .. }
        ) {
            Vec::new()
        } else {
            extractor
                .extract(&change.path.to_path_buf(), &blob(dir, sha, &change.path))
                .unwrap()
        };
        let (changes, diagnostics) = symbol_changes_from_extracted(sha, change, &left, &right);
        parsed_changes.extend(changes);
        parsed_diagnostics.extend(diagnostics);
    }
    let mut ctx = SymbolDiffCtx::new();
    let walker = symbol_changes_for_commit(dir, sha, files, &mut ctx).unwrap();
    let parsed = merge_symbol_changes(parsed_changes);
    assert_eq!(
        canonical_changes(&parsed),
        canonical_changes(&walker),
        "commit {sha}"
    );
    parsed_diagnostics.sort();
    let mut walker_diagnostics = ctx.diagnostics().to_vec();
    walker_diagnostics.sort();
    assert_eq!(parsed_diagnostics, walker_diagnostics, "commit {sha}");
    parsed
}

fn canonical_changes(changes: &[SymbolChange]) -> Vec<String> {
    let mut changes: Vec<_> = changes.iter().map(|c| format!("{c:?}")).collect();
    changes.sort();
    changes
}

fn symbol(name: &str, start: usize, anchor: &str, tokens: &[&str]) -> ExtractedSymbol {
    ExtractedSymbol {
        entity_name: name.to_owned(),
        symbol_kind: "function".to_owned(),
        enclosing_scope: None,
        byte_range: [start, start + 10],
        line_range: [start + 1, start + 2],
        anchor_hash: anchor.to_owned(),
        tokens: tokens.iter().map(|token| (*token).to_owned()).collect(),
    }
}

fn file_change(path: &str, kind: FileChangeKind, parent: Option<&str>) -> FileChange {
    FileChange {
        path: GitPath::from(path),
        kind,
        parent_sha: parent.map(str::to_owned),
    }
}

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(
        dir,
        &["config", "user.email", "temporal-test@example.invalid"],
    );
    git(dir, &["config", "user.name", "Temporal test"]);
}

fn write(dir: &Path, path: &str, bytes: &[u8]) {
    std::fs::write(dir.join(path), bytes).unwrap();
}

fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    revision(dir, "HEAD")
}

fn revision(dir: &Path, revision: &str) -> String {
    String::from_utf8(git(dir, &["rev-parse", revision]))
        .unwrap()
        .trim()
        .to_owned()
}

fn blob(dir: &Path, sha: &str, path: &GitPath) -> Vec<u8> {
    git(dir, &["show", &format!("{sha}:{}", path.to_string_lossy())])
}

fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
