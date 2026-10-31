use std::path::{Path, PathBuf};

use anyhow::Context as _;
use ignore::{DirEntry, Error as IgnoreError, WalkBuilder};

/// Root-relative graph-owned outputs must never become graph source inputs.
/// Keep unrelated hidden/JSON sources, including nested `.spur` fixtures.
pub(crate) fn is_graph_owned_path(path: &Path) -> bool {
    use crate::store::cache::{COMMIT_INDEX_POINTER_PATH, POINTER_PATH, WORKTREE_ARTIFACT_PATH};
    path.starts_with(WORKTREE_ARTIFACT_PATH)
        || path == Path::new(POINTER_PATH)
        || path == Path::new(COMMIT_INDEX_POINTER_PATH)
}

pub fn discover_files(root: &Path, allowed_extensions: &[&str]) -> anyhow::Result<Vec<PathBuf>> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize `{}`", root.display()))?;
    let mut files = Vec::new();

    for entry in WalkBuilder::new(&root)
        .standard_filters(true)
        .hidden(false)
        .filter_entry({
            let root = root.clone();
            move |entry| {
                should_descend(entry)
                    && !entry
                        .path()
                        .strip_prefix(&root)
                        .is_ok_and(is_graph_owned_path)
            }
        })
        .build()
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                let path = walk_error_path(&err)
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "<unknown>".to_owned());
                tracing::warn!(
                    root = %root.display(),
                    path = %path,
                    error = %err,
                    "spur-graph: skipping entry (walk failed)"
                );
                continue;
            }
        };
        if entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
            && entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    allowed_extensions
                        .iter()
                        .any(|allowed| extension.eq_ignore_ascii_case(allowed))
                })
        {
            files.push(entry.into_path());
        }
    }

    files.sort();
    Ok(files)
}

fn walk_error_path(err: &IgnoreError) -> Option<&Path> {
    match err {
        IgnoreError::Partial(errs) => errs.iter().find_map(walk_error_path),
        IgnoreError::WithLineNumber { err, .. } => walk_error_path(err),
        IgnoreError::WithPath { path, .. } => Some(path.as_path()),
        IgnoreError::WithDepth { err, .. } => walk_error_path(err),
        IgnoreError::Loop { child, .. } => Some(child.as_path()),
        _ => None,
    }
}

fn should_descend(entry: &DirEntry) -> bool {
    let Some(file_name) = entry.file_name().to_str() else {
        return true;
    };
    if file_name == "target" || file_name == ".git" || file_name == "node_modules" {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::discover_files;
    use std::fs;

    #[test]
    fn discover_files_includes_dot_github_yaml_and_skips_git_and_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().canonicalize().expect("canonical temp root");
        fs::create_dir_all(root.join(".github/workflows")).expect("mkdir workflows");
        fs::create_dir_all(root.join(".git")).expect("mkdir .git");
        fs::create_dir_all(root.join("target")).expect("mkdir target");
        fs::create_dir_all(root.join("src")).expect("mkdir src");
        fs::write(
            root.join(".github/workflows/ci.yml"),
            "name: ci\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n",
        )
        .expect("write ci.yml");
        fs::write(root.join("src/app.yaml"), "interface:\n  name: app\n").expect("write app.yaml");
        fs::write(root.join(".git/ignored.yml"), "secret: 1\n").expect("write .git yml");
        fs::write(root.join("target/out.yml"), "artifact: 1\n").expect("write target yml");

        let files = discover_files(&root, &["yml", "yaml"]).expect("discover");
        let relative: Vec<String> = files
            .iter()
            .map(|path| {
                path.strip_prefix(&root)
                    .expect("under root")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();

        assert!(
            relative
                .iter()
                .any(|path| path == ".github/workflows/ci.yml"),
            "tracked hidden source dir must be indexed, got {relative:?}"
        );
        assert!(
            relative.iter().any(|path| path == "src/app.yaml"),
            "normal yaml must be indexed, got {relative:?}"
        );
        assert!(
            !relative.iter().any(|path| path.contains(".git/")),
            ".git must stay skipped, got {relative:?}"
        );
        assert!(
            !relative.iter().any(|path| path.starts_with("target/")),
            "target/ must stay skipped, got {relative:?}"
        );
    }
}
