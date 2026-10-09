//! Transactional, cross-process graph change tracking. No reader consumes rows.
//! One row per affected ID coalesces edits, retaining deletion evidence.

use rusqlite::Connection;

pub(super) fn install(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS spur_graph_clock (
             id INTEGER PRIMARY KEY CHECK(id=1),
             revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=0)
         );
         INSERT OR IGNORE INTO spur_graph_clock VALUES (1,0);
         CREATE TABLE IF NOT EXISTS spur_graph_changes (
             issue_id TEXT PRIMARY KEY,
             revision INTEGER NOT NULL,
             node_revision INTEGER NOT NULL,
             edge_revision INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS spur_graph_changes_revision
             ON spur_graph_changes(revision);",
    )?;

    let fields = [
        "id",
        "title",
        "status",
        "priority",
        "issue_type",
        "assignee",
        "created_at",
        "updated_at",
        "due_at",
        "content_hash",
        "is_template",
    ];
    let issue_changed = fields
        .iter()
        .map(|f| format!("OLD.{f} IS NOT NEW.{f}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    for (table, guard, old_ids, new_ids, node) in [
        ("issues", issue_changed.as_str(), vec!["OLD.id"], vec!["NEW.id"], true),
        ("labels", "OLD.issue_id IS NOT NEW.issue_id OR OLD.label IS NOT NEW.label",
            vec!["OLD.issue_id"], vec!["NEW.issue_id"], true),
        ("dependencies", "OLD.issue_id IS NOT NEW.issue_id OR OLD.depends_on_id IS NOT NEW.depends_on_id OR OLD.type IS NOT NEW.type",
            vec!["OLD.issue_id", "OLD.depends_on_id"], vec!["NEW.issue_id", "NEW.depends_on_id"], false),
    ] {
        for operation in ["insert", "update", "delete"] {
            let ids = match operation {
                "insert" => new_ids.clone(),
                "delete" => old_ids.clone(),
                _ => old_ids.iter().chain(&new_ids).copied().collect(),
            };
            let when = if operation == "update" { format!(" WHEN {guard}") } else { String::new() };
            let mut body = String::from("UPDATE spur_graph_clock SET revision=revision+1 WHERE id=1;");
            for id in ids {
                body.push_str(&mark(&format!("SELECT {id} AS changed_id"), node));
            }
            tx.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS spur_graph_{table}_{operation}
                 AFTER {operation} ON {table}{when} BEGIN {body} END;"
            ))?;
        }
    }

    // SQLite REPLACE may delete a different issue through the external_ref
    // unique index without firing DELETE triggers (recursive_triggers=OFF).
    // Record that victim before INSERT/UPDATE; a failing statement rolls this
    // back too. INSERT OR IGNORE can conservatively record a harmless delta.
    for (name, event, own_id) in [
        ("insert", "INSERT", "NEW.id"),
        ("update", "UPDATE OF external_ref", "OLD.id"),
    ] {
        let victims = format!(
            "SELECT id AS changed_id FROM issues WHERE external_ref=NEW.external_ref AND id<>{own_id}"
        );
        let body = mark(&victims, true);
        tx.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS spur_graph_replace_{name}
             BEFORE {event} ON issues WHEN EXISTS({victims})
             BEGIN
                 UPDATE spur_graph_clock SET revision=revision+1 WHERE id=1;
                 {body}
             END;"
        ))?;
    }
    tx.commit()?;
    Ok(())
}

fn mark(ids: &str, node: bool) -> String {
    let revision = "(SELECT revision FROM spur_graph_clock WHERE id=1)";
    let (node_revision, edge_revision) = if node {
        (revision, "0")
    } else {
        ("0", revision)
    };
    // The subquery's WHERE avoids SQLite's INSERT SELECT / ON CONFLICT parser ambiguity.
    format!(
        "INSERT INTO spur_graph_changes(issue_id,revision,node_revision,edge_revision)
         SELECT changed_id,{revision},{node_revision},{edge_revision}
         FROM ({ids}) WHERE true
         ON CONFLICT(issue_id) DO UPDATE SET
           revision=excluded.revision,
           node_revision=MAX(spur_graph_changes.node_revision,excluded.node_revision),
           edge_revision=MAX(spur_graph_changes.edge_revision,excluded.edge_revision);"
    )
}
