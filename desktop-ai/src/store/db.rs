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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::path::PathBuf;

    /// WAL / 同步 / 外键 / busy_timeout 的 PRAGMA 基线必须生效。
    #[test]
    fn wal_and_durability_pragmas_applied() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = open(&dir.path().join("t.db")).unwrap();
        let (journal, sync, fk, busy): (String, i64, i64, i64) = db
            .with_conn(|c| {
                Ok((
                    c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?,
                    c.query_row("PRAGMA synchronous", [], |r| r.get(0))?,
                    c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?,
                    c.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?,
                ))
            })
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
        assert_eq!(sync, 1, "synchronous 应为 NORMAL(1)");
        assert_eq!(fk, 1, "外键约束必须开启");
        assert_eq!(busy, 5000, "busy_timeout 应等待 5 秒而不是立刻 SQLITE_BUSY");
    }

    /// 崩溃现场模拟：连接仍持有时复制 db+wal，重开副本必须能回放 WAL 不丢数据。
    #[test]
    fn wal_replays_after_crash_copy() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("crash.db");
        let copy_dir = dir.path().join("copy");
        {
            let db = open(&path).unwrap();
            db.with_conn(|c| {
                c.execute_batch("CREATE TABLE t(x INTEGER)")?;
                c.execute("INSERT INTO t VALUES (42)", params![])
            })
            .unwrap();
            // 不 checkpoint、不关闭连接，直接拷贝现场文件（等价于进程被强杀）
            std::fs::create_dir_all(&copy_dir).unwrap();
            for suffix in ["", "-wal", "-shm"] {
                let src = PathBuf::from(format!("{}{}", path.display(), suffix));
                if src.exists() {
                    std::fs::copy(&src, copy_dir.join(src.file_name().unwrap())).unwrap();
                }
            }
        }
        let db2 = open(&copy_dir.join("crash.db")).unwrap();
        let v: i64 = db2
            .with_conn(|c| c.query_row("SELECT x FROM t", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(v, 42, "WAL 副本必须能回放出崩溃前的已提交数据");
    }

    /// 写锁被占用时，另一连接应在 busy_timeout 内等待而不是直接失败。
    #[test]
    fn busy_timeout_waits_for_writer() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("busy.db");
        let db1 = open(&path).unwrap();
        db1.with_conn(|c| c.execute_batch("CREATE TABLE t(x INTEGER)"))
            .unwrap();

        let guard = db1.lock();
        guard.execute_batch("BEGIN IMMEDIATE").unwrap();
        guard
            .execute("INSERT INTO t VALUES (1)", params![])
            .unwrap();

        let path2 = path.clone();
        let t = std::thread::spawn(move || {
            let db2 = open(&path2).unwrap();
            db2.with_conn(|c| c.execute("INSERT INTO t VALUES (2)", params![]))
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        guard.execute_batch("COMMIT").unwrap();
        drop(guard);

        let res = t.join().unwrap();
        assert!(res.is_ok(), "busy_timeout 应等待锁释放: {:?}", res.err());
        let n: i64 = db1
            .with_conn(|c| c.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(n, 2);
    }

    /// checkpoint 应能把 WAL 内容并回主库并截断 WAL 文件。
    #[test]
    fn checkpoint_truncates_wal() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("cp.db");
        let db = open(&path).unwrap();
        db.with_conn(|c| {
            c.execute_batch("CREATE TABLE t(x INTEGER)")?;
            for i in 0..500 {
                c.execute("INSERT INTO t VALUES (?1)", params![i])?;
            }
            Ok(())
        })
        .unwrap();
        let wal = PathBuf::from(format!("{}-wal", path.display()));
        let before = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert!(before > 0, "写入后 WAL 应非空");

        let (busy, log, checkpointed): (i64, i64, i64) = db
            .with_conn(|c| {
                c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
            })
            .unwrap();
        assert_eq!(busy, 0, "checkpoint 不应被占用");
        assert!(log >= checkpointed);
        let after = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert_eq!(after, 0, "TRUNCATE checkpoint 后 WAL 应为空");

        let n: i64 = db
            .with_conn(|c| c.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(n, 500, "checkpoint 不得丢数据");
    }
}
