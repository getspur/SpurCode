use std::path::{Path, PathBuf};

use beads_rust::model::{Issue, IssueType, Priority, Status};
use beads_rust::storage::sqlite::SqliteStorage;
use chrono::Utc;
use tempfile::TempDir;

pub struct TestBeadsWorkspace {
    _dir: TempDir,
    pub path: PathBuf,
    pub storage: SqliteStorage,
}

impl TestBeadsWorkspace {
    pub fn init() -> Self {
        let dir = TempDir::new().expect("create temp beads workspace");
        let path = dir.path().to_path_buf();
        let db_path = path.join("beads.db");
        let storage = SqliteStorage::open(&db_path).expect("open beads test database");

        Self {
            _dir: dir,
            path,
            storage,
        }
    }

    pub fn create_issue(&mut self, title: &str) -> String {
        self.create_with_type(title, IssueType::Task)
    }

    pub fn create_issue_with_id(&mut self, id: &str, title: &str) -> String {
        let issue = build_issue(id.to_string(), title.to_string(), IssueType::Task);
        self.storage
            .create_issue(&issue, "test")
            .expect("create test issue with explicit id");
        id.to_string()
    }

    pub fn create_epic(&mut self, title: &str) -> String {
        self.create_with_type(title, IssueType::Epic)
    }

    pub fn add_label(&mut self, id: &str, label: &str) {
        self.storage
            .add_label(id, label, "test")
            .expect("add test issue label");
    }

    pub fn close_issue(&mut self, id: &str) {
        let update = beads_rust::storage::sqlite::IssueUpdate {
            status: Some(Status::Closed),
            ..Default::default()
        };
        self.storage
            .update_issue(id, &update, "test")
            .expect("close test issue");
    }

    pub fn add_dep(&mut self, child: &str, parent: &str) {
        self.storage
            .add_dependency(child, parent, "blocks", "test")
            .expect("add test issue dependency");
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Copy `beads.db` plus its WAL/SHM sidecars to `<dst_dir>/beads.db…`.
    ///
    /// beads_rust's `SqliteStorage` opens the database in WAL mode and
    /// explicitly skips checkpoint on `Drop` (see `beads_rust::storage::sqlite`
    /// `impl Drop` — comment cites the read-only-CLI lock-contention bug).
    /// Periodic auto-checkpoint only fires every 50 mutations, so a typical
    /// test (a handful of issues + labels) leaves all of its data in
    /// `beads.db-wal` with `beads.db` effectively empty.
    ///
    /// Copying just `beads.db` therefore loses every uncheckpointed write.
    /// The reader opens an empty database and trivial assertions like
    /// `actions.is_empty()` pass for the wrong reason. beads_rust does NOT
    /// expose a public checkpoint API to downstream crates (`execute_raw` is
    /// `pub(crate)`, `execute_test_sql` is `#[cfg(test)]`), so we mirror the
    /// production read path: copy all three SQLite files. `SqliteStorage`
    /// at the destination then reads main + WAL exactly as it does at the
    /// source.
    pub fn copy_db_to(&self, dst_dir: &Path) {
        for sidecar in ["beads.db", "beads.db-wal", "beads.db-shm"] {
            let src = self.path.join(sidecar);
            if !src.exists() {
                continue;
            }
            std::fs::copy(&src, dst_dir.join(sidecar))
                .unwrap_or_else(|e| panic!("copy {sidecar}: {e}"));
        }
    }

    fn create_with_type(&mut self, title: &str, issue_type: IssueType) -> String {
        self.create_with_type_at(title, issue_type, Utc::now())
    }

    fn create_with_type_at(
        &mut self,
        title: &str,
        issue_type: IssueType,
        now: chrono::DateTime<Utc>,
    ) -> String {
        let id = beads_rust::util::id::IdGenerator::with_defaults().generate(
            title,
            None,
            Some("test"),
            now,
            self.storage
                .count_issues()
                .expect("count test issues")
                .saturating_add(1),
            |candidate| {
                self.storage
                    .id_exists(candidate)
                    .expect("check test issue ID")
            },
        );
        let issue = build_issue(id.clone(), title.to_string(), issue_type);
        self.storage
            .create_issue(&issue, "test")
            .expect("create test issue");
        id
    }
}

fn build_issue(id: String, title: String, issue_type: IssueType) -> Issue {
    let now = Utc::now();
    Issue {
        id,
        title,
        description: None,
        status: Status::Open,
        priority: Priority::MEDIUM,
        issue_type,
        created_at: now,
        updated_at: now,
        assignee: None,
        owner: None,
        estimated_minutes: None,
        due_at: None,
        defer_until: None,
        external_ref: None,
        ephemeral: false,
        content_hash: None,
        design: None,
        acceptance_criteria: None,
        notes: None,
        created_by: Some("test".to_string()),
        closed_at: None,
        close_reason: None,
        closed_by_session: None,
        source_system: None,
        source_repo: None,
        deleted_at: None,
        deleted_by: None,
        delete_reason: None,
        original_type: None,
        compaction_level: None,
        compacted_at: None,
        compacted_at_commit: None,
        original_size: None,
        sender: None,
        pinned: false,
        is_template: false,
        labels: Vec::new(),
        dependencies: Vec::new(),
        comments: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_generation_handles_existing_candidate_id() {
        let mut w = TestBeadsWorkspace::init();
        let now = Utc::now();
        let existing = beads_rust::util::generate_id("collision", None, Some("test"), now);
        w.create_issue_with_id(&existing, "existing issue");
        let next = w.create_with_type_at("collision", IssueType::Task, now);
        assert_ne!(next, existing);
        assert_eq!(w.storage.count_issues().unwrap(), 2);
    }

    #[test]
    fn workspace_init_and_create_round_trip() {
        let mut w = TestBeadsWorkspace::init();
        let id = w.create_issue("Hello");
        assert!(!id.is_empty());

        let issue = w.storage.get_issue(&id).unwrap().expect("present");
        assert_eq!(issue.title, "Hello");

        w.add_label(&id, "test-label");
        w.close_issue(&id);

        let epic_id = w.create_epic("Epic");
        let child = w.create_issue("Child");
        w.add_dep(&child, &epic_id);
    }

    #[test]
    fn copy_db_to_preserves_uncheckpointed_wal_data() {
        let mut w = TestBeadsWorkspace::init();
        let epic = w.create_epic("Epic");
        for i in 0..3 {
            let task = w.create_issue(&format!("Task {i}"));
            w.add_label(&task, "spur:plan-id:CP-TEST");
            w.close_issue(&task);
        }
        w.add_label(&epic, "spur:plan-complete");
        w.close_issue(&epic);

        let original_count = w.storage.count_issues().unwrap();
        assert_eq!(original_count, 4, "setup precondition");

        let dst = TempDir::new().unwrap();
        w.copy_db_to(dst.path());

        let copy_storage =
            beads_rust::storage::sqlite::SqliteStorage::open(&dst.path().join("beads.db")).unwrap();
        let copy_count = copy_storage.count_issues().unwrap();
        assert_eq!(
            copy_count, original_count,
            "copy_db_to must preserve all data — bare `fs::copy(beads.db)` would lose WAL data"
        );
    }
}
