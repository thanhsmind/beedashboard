//! Notification outbox — the durable at-least-once delivery queue for
//! status-change alerts (D1: ported from herdr-go's `store` module). This is
//! the only durable state the agent-terminal notification path persists: a
//! Telegram poll offset and pending/delivered notification rows. It never
//! stores terminal output, transcript content or any credential.
//!
//! Plain rusqlite, no async: `waggledance-core` stays async-runtime-free
//! (`bee::tests::no_web_framework_dependency_declared` asserts no
//! `tokio`/`axum`/`hyper` in this crate's `Cargo.toml`), so every call here
//! is a synchronous SQLite statement — the async watcher/notifier that live
//! in the `waggledance` binary crate call it directly, the same way
//! `repository::SqliteStore` is called from async handlers today: behind a
//! `Mutex<Connection>` (Send+Sync), never behind an async trait.

use crate::error::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

/// A pending notification obligation (e.g. "agent X is blocked"). Delivery is
/// at-least-once: recorded first, marked delivered only after a successful
/// send, so a crash between the two resends rather than loses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub id: i64,
    pub pane_id: String,
    pub kind: String,
    pub body: String,
    pub run_id: Option<String>,
    pub project_id: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS notify_kv (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS notifications (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    pane_id      TEXT NOT NULL,
    kind         TEXT NOT NULL,
    body         TEXT NOT NULL,
    created_at   INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    delivered_at INTEGER,
    run_id       TEXT,
    project_id   TEXT
);
"#;

pub struct NotifyStore {
    conn: Mutex<Connection>,
}

impl NotifyStore {
    /// Open (creating if needed) the outbox at `path`, applying the schema.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::from_conn(conn)
    }

    /// Open an in-memory outbox (tests).
    pub fn open_in_memory() -> Result<Self> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.execute_batch(SCHEMA)?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn migrate(conn: &Connection) -> Result<()> {
        if !has_column(conn, "notifications", "run_id")? {
            conn.execute("ALTER TABLE notifications ADD COLUMN run_id TEXT", [])?;
        }
        if !has_column(conn, "notifications", "project_id")? {
            conn.execute("ALTER TABLE notifications ADD COLUMN project_id TEXT", [])?;
        }
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_notifications_run_kind
             ON notifications(run_id, kind)",
            [],
        )?;
        Ok(())
    }

    /// Load the durable Telegram poll offset.
    pub fn poll_offset(&self) -> Result<i64> {
        let c = self.conn.lock().unwrap();
        let v: rusqlite::Result<String> = c.query_row(
            "SELECT value FROM notify_kv WHERE key='poll_offset'",
            [],
            |r| r.get(0),
        );
        match v {
            Ok(s) => Ok(s.parse().unwrap_or(0)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(e) => Err(e.into()),
        }
    }

    /// Persist the poll offset after a batch has been acted on.
    pub fn set_poll_offset(&self, offset: i64) -> Result<()> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO notify_kv(key,value) VALUES('poll_offset', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![offset.to_string()],
        )?;
        Ok(())
    }

    /// Queue a notification obligation; returns its id.
    pub fn enqueue_notification(&self, pane_id: &str, kind: &str, body: &str) -> Result<i64> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO notifications(pane_id,kind,body) VALUES(?1,?2,?3)",
            params![pane_id, kind, body],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// Queue a run-aware notification obligation (D5). If a notification for
    /// the same `(run_id, kind)` was already enqueued, the duplicate is
    /// swallowed and `Ok(None)` is returned.
    pub fn enqueue_run_notification(
        &self,
        run_id: &str,
        project_id: &str,
        pane_id: &str,
        kind: &str,
        body: &str,
    ) -> Result<Option<i64>> {
        let c = self.conn.lock().unwrap();
        let rows_affected = c.execute(
            "INSERT INTO notifications(pane_id,kind,body,run_id,project_id)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(run_id, kind) DO NOTHING",
            params![pane_id, kind, body, run_id, project_id],
        )?;
        if rows_affected == 0 {
            Ok(None)
        } else {
            Ok(Some(c.last_insert_rowid()))
        }
    }

    /// List notifications not yet marked delivered, oldest first.
    pub fn undelivered(&self) -> Result<Vec<Notification>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare(
            "SELECT id,pane_id,kind,body,run_id,project_id FROM notifications WHERE delivered_at IS NULL ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Notification {
                id: r.get(0)?,
                pane_id: r.get(1)?,
                kind: r.get(2)?,
                body: r.get(3)?,
                run_id: r.get(4)?,
                project_id: r.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Mark one notification delivered (only after a successful send).
    pub fn mark_delivered(&self, id: i64) -> Result<()> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "UPDATE notifications SET delivered_at=strftime('%s','now') WHERE id=?1",
            params![id],
        )?;
        Ok(())
    }
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        if r.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_round_trips() {
        let s = NotifyStore::open_in_memory().unwrap();
        assert_eq!(s.poll_offset().unwrap(), 0);
        s.set_poll_offset(42).unwrap();
        assert_eq!(s.poll_offset().unwrap(), 42);
    }

    #[test]
    fn offset_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        {
            let s = NotifyStore::open(&path).unwrap();
            s.set_poll_offset(99).unwrap();
        }
        let s2 = NotifyStore::open(&path).unwrap();
        assert_eq!(s2.poll_offset().unwrap(), 99);
    }

    #[test]
    fn notification_delivery_is_at_least_once() {
        let s = NotifyStore::open_in_memory().unwrap();
        let id = s
            .enqueue_notification("pane-1", "blocked", "agent needs you")
            .unwrap();
        let pending = s.undelivered().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        s.mark_delivered(id).unwrap();
        // A restart re-reads the same durable state — a delivered
        // notification never resurfaces as pending.
        assert!(s.undelivered().unwrap().is_empty());
    }

    #[test]
    fn reopen_is_idempotent_and_stays_usable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.sqlite");
        let _ = NotifyStore::open(&path).unwrap();
        // Reopening re-applies `CREATE TABLE IF NOT EXISTS` harmlessly.
        let s2 = NotifyStore::open(&path).unwrap();
        assert_eq!(s2.poll_offset().unwrap(), 0);
    }

    #[test]
    fn outbox_schema_has_no_terminal_output_or_credential_column() {
        // Guards herdr-go's never-store rule: the outbox's only columns are
        // an opaque pane id, a status kind, and a composed status body — no
        // screen/transcript text and no token/credential column exists to
        // write into. A future column named any of these must fail this test
        // before it can ship.
        let forbidden = [
            "screen",
            "transcript",
            "token",
            "credential",
            "password",
            "secret",
        ];
        let schema_lower = SCHEMA.to_lowercase();
        for word in forbidden {
            assert!(
                !schema_lower.contains(word),
                "notify_store SCHEMA must never declare a `{word}` column"
            );
        }
    }

    #[test]
    fn two_enqueues_for_same_run_and_status_leaves_one_row() {
        let s = NotifyStore::open_in_memory().unwrap();
        let id1 = s
            .enqueue_run_notification(
                "run-1",
                "proj-1",
                "pane-1",
                "blocked",
                "proj-1 pane-1 run-1",
            )
            .unwrap();
        assert!(id1.is_some());
        let id2 = s
            .enqueue_run_notification(
                "run-1",
                "proj-1",
                "pane-1",
                "blocked",
                "proj-1 pane-1 run-1",
            )
            .unwrap();
        assert!(id2.is_none());

        let pending = s.undelivered().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].run_id.as_deref(), Some("run-1"));
        assert_eq!(pending[0].project_id.as_deref(), Some("proj-1"));
        assert_eq!(pending[0].kind, "blocked");
    }

    #[test]
    fn same_run_at_different_status_adds_second_row() {
        let s = NotifyStore::open_in_memory().unwrap();
        let id1 = s
            .enqueue_run_notification(
                "run-1",
                "proj-1",
                "pane-1",
                "blocked",
                "proj-1 pane-1 run-1",
            )
            .unwrap();
        assert!(id1.is_some());
        let id2 = s
            .enqueue_run_notification(
                "run-1",
                "proj-1",
                "pane-1",
                "timeout",
                "proj-1 pane-1 run-1",
            )
            .unwrap();
        assert!(id2.is_some());

        let pending = s.undelivered().unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].kind, "blocked");
        assert_eq!(pending[1].kind, "timeout");
    }

    #[test]
    fn legacy_database_opens_migrates_and_returns_original_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE notify_kv (
                    key   TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                CREATE TABLE notifications (
                    id           INTEGER PRIMARY KEY AUTOINCREMENT,
                    pane_id      TEXT NOT NULL,
                    kind         TEXT NOT NULL,
                    body         TEXT NOT NULL,
                    created_at   INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                    delivered_at INTEGER
                );
                INSERT INTO notifications(pane_id, kind, body)
                VALUES('pane-legacy', 'blocked', 'legacy alert');
                "#,
            )
            .unwrap();
        }

        let s = NotifyStore::open(&path).unwrap();
        let pending = s.undelivered().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].pane_id, "pane-legacy");
        assert_eq!(pending[0].kind, "blocked");
        assert_eq!(pending[0].body, "legacy alert");
        assert_eq!(pending[0].run_id, None);
        assert_eq!(pending[0].project_id, None);

        let id2 = s
            .enqueue_run_notification(
                "run-new",
                "proj-new",
                "pane-new",
                "timeout",
                "proj-new pane-new run-new",
            )
            .unwrap();
        assert!(id2.is_some());
        assert_eq!(s.undelivered().unwrap().len(), 2);
    }

    #[test]
    fn pane_only_enqueue_still_works_and_drains() {
        let s = NotifyStore::open_in_memory().unwrap();
        let id = s
            .enqueue_notification("pane-1", "done", "claude agent pane-1 is done")
            .unwrap();
        let pending = s.undelivered().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        assert_eq!(pending[0].pane_id, "pane-1");
        assert_eq!(pending[0].run_id, None);
        assert_eq!(pending[0].project_id, None);

        s.mark_delivered(id).unwrap();
        assert!(s.undelivered().unwrap().is_empty());
    }
}
