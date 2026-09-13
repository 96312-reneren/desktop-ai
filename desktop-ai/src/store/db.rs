//! SQLite storage helper.
//!
//! Concurrency rules (deadlock avoidance):
//! - One `Mutex<Connection>` per database file; all access goes through
//!   [`Db::with_conn`] which locks for the duration of a single short call.
//! - **Never** hold the lock across slow operations (embedding inference,
//!   network I/O, GUI). Callers must do heavy work *before* taking the lock;
//!   inside the lock only fast SQL statements + byte copies.
//! - Never acquire the lock of another store while holding this one
//!   (each store owns its own `Db`, so cross-store nesting is impossible).
//! - WAL journal + `busy_timeout` make concurrent writers wait instead of
//!   failing with `SQLITE_BUSY`.

use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

pub(crate) struct Db {
    conn: Mutex<Connection>,
}

pub(crate) fn open(path: &Path) -> Result<Db, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {:?}: {}", parent, e))?;
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(|e| format!("open {:?}: {}", path, e))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("enable WAL: {}", e))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| format!("set synchronous: {}", e))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| format!("enable foreign keys: {}", e))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("set busy_timeout: {}", e))?;
    Ok(Db {
        conn: Mutex::new(conn),
    })
}

impl Db {
    /// Run a short transaction under the connection lock.
    pub(crate) fn with_conn<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        let mut guard = self.lock();
        f(&mut guard).map_err(|e| e.to_string())
    }

    /// Lock the connection directly. Prefer [`Db::with_conn`]; use this only
    /// when several statements must share one transaction. Keep the guard
    /// alive for as short as possible and never call other `Db` methods
    /// while holding it.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().expect("db mutex poisoned")
    }
}
