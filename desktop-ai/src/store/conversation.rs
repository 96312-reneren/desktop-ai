use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::store::config::conversations_dir;
use crate::store::db::Db;

fn sanitize_id(id: &str) -> bool {
    id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ConversationMeta {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConversationData {
    id: String,
    title: String,
    created_at: String,
    messages: Vec<Message>,
}

#[derive(Debug, Clone)]
pub struct Conversation {
    pub id: String,
    pub messages: Vec<Message>,
    /// How many of `messages` are already persisted; `save()` only inserts
    /// the tail beyond this, so long conversations stay O(1) per message.
    persisted_count: usize,
}

/// Process-wide SQLite conversation store. One connection, guarded by a
/// mutex, shared by all conversation operations (see [`crate::store::db`]).
static CONV_DB: once_cell::sync::Lazy<Option<Db>> = once_cell::sync::Lazy::new(|| {
    let path = conversations_dir().join("conversations.db");
    let db = match crate::store::db::open(&path) {
        Ok(db) => db,
        Err(e) => {
            log::error!("failed to open conversations.db: {}", e);
            return None;
        }
    };
    if let Err(e) = db.with_conn(|c| {
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations (
                id         TEXT PRIMARY KEY,
                title      TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                conv_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                role    TEXT NOT NULL,
                content TEXT NOT NULL,
                seq     INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(conv_id, seq);
            ",
        )?;
        let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            if let Err(e) = migrate_from_json(c) {
                log::error!("conversation JSON migration failed: {}", e);
            }
            c.pragma_update(None, "user_version", 1)?;
        }
        Ok(())
    }) {
        log::error!("conversation store init failed: {}", e);
        return None;
    }
    Some(db)
});

fn conversation_title(messages: &[Message]) -> String {
    messages
        .iter()
        .find(|m| m.role == "user")
        .map(|m| {
            let t: String = m.content.chars().take(50).collect();
            if m.content.len() > 50 {
                format!("{}...", t)
            } else {
                t
            }
        })
        .unwrap_or_else(|| "新对话".into())
}

#[allow(clippy::new_without_default)]
impl Conversation {
    pub fn new() -> Self {
        // 微秒时间戳 + 进程内序列号：紧密循环或并发创建时也不会撞 id。
        // （仅用 %f 微秒时，同一微秒内创建的两个会话会共享 id，其消息按
        // seq 互相插入同一会话，属于数据污染。）
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!(
            "{}_{}",
            Utc::now().format("%Y%m%d_%H%M%S_%f"),
            seq % 1_000_000
        );
        Self {
            id,
            messages: vec![],
            persisted_count: 0,
        }
    }

    pub fn load(id: &str) -> Option<Self> {
        if !sanitize_id(id) {
            return None;
        }
        let db = CONV_DB.as_ref()?;
        let exists: bool = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?1)",
                    params![id],
                    |r| r.get(0),
                )
            })
            .ok()
            .unwrap_or(false);
        if !exists {
            return None;
        }
        let messages: Vec<Message> = db
            .with_conn(|c| {
                let mut stmt = c.prepare(
                    "SELECT role, content FROM messages WHERE conv_id = ?1 ORDER BY seq",
                )?;
                let rows = stmt.query_map(params![id], |r| {
                    Ok(Message {
                        role: r.get(0)?,
                        content: r.get(1)?,
                    })
                })?;
                rows.collect()
            })
            .ok()?;
        let persisted_count = messages.len();
        Some(Self {
            id: id.into(),
            messages,
            persisted_count,
        })
    }

    pub(crate) fn save(&mut self) {
        if !sanitize_id(&self.id) {
            log::warn!("refusing to save conversation with unsafe id: {}", self.id);
            return;
        }
        let Some(db) = CONV_DB.as_ref() else { return };
        let title = conversation_title(&self.messages);
        let now = Utc::now().to_rfc3339();
        // Only the messages beyond `persisted_count` are new; the meta row
        // is upserted (O(1), no delete → no CASCADE wipe) so title and
        // updated_at stay fresh.
        let new_messages: Vec<Message> =
            self.messages[self.persisted_count.min(self.messages.len())..].to_vec();
        if new_messages.is_empty() {
            return;
        }
        let result = db.with_conn(|c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO conversations (id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                     title = excluded.title,
                     updated_at = excluded.updated_at",
                params![self.id, title, now, now],
            )?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO messages (conv_id, role, content, seq) VALUES (?1, ?2, ?3, ?4)",
                )?;
                for (i, m) in new_messages.iter().enumerate() {
                    stmt.execute(params![
                        self.id,
                        m.role,
                        m.content,
                        self.persisted_count + i
                    ])?;
                }
            }
            tx.commit()
        });
        match result {
            Ok(()) => self.persisted_count = self.messages.len(),
            Err(e) => log::warn!("failed to save conversation {}: {}", self.id, e),
        }
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(Message {
            role: role.into(),
            content: content.into(),
        });
        self.save();
    }

    pub fn delete(id: &str) {
        if !sanitize_id(id) {
            return;
        }
        if let Some(db) = CONV_DB.as_ref() {
            let _ = db.with_conn(|c| {
                c.execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
                Ok(())
            });
        }
    }

    /// Delete every conversation (both the SQLite store and any leftover
    /// legacy JSON files).
    pub(crate) fn delete_all() {
        if let Some(db) = CONV_DB.as_ref() {
            let _ = db.with_conn(|c| {
                c.execute("DELETE FROM messages", [])?;
                c.execute("DELETE FROM conversations", [])?;
                Ok(())
            });
        }
        let dir = conversations_dir();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|ext| ext == "json") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    pub(crate) fn list_all() -> Vec<ConversationMeta> {
        let Some(db) = CONV_DB.as_ref() else {
            return Vec::new();
        };
        db.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT c.id, c.title, c.created_at, COUNT(m.id)
                 FROM conversations c
                 LEFT JOIN messages m ON m.conv_id = c.id
                 GROUP BY c.id
                 ORDER BY c.updated_at DESC",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(ConversationMeta {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    created_at: r.get(2)?,
                    message_count: r.get::<_, i64>(3)? as usize,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default()
    }

    pub(crate) fn context_messages(&self, system_prompt: Option<&str>, max: usize) -> Vec<Message> {
        let mut msgs = vec![];
        if let Some(sp) = system_prompt {
            msgs.push(Message {
                role: "system".into(),
                content: sp.into(),
            });
        }
        let start = if self.messages.len() > max {
            self.messages.len() - max
        } else {
            0
        };
        msgs.extend(self.messages[start..].to_vec());
        msgs
    }

    /// Export this conversation as a pretty-printed JSON string.
    /// Includes id, title, created_at, and messages. Suitable for
    /// backup/migration. Returns Err with a descriptive message on failure.
    pub(crate) fn export_json(&self) -> Result<String, String> {
        let data = ConversationData {
            id: self.id.clone(),
            title: conversation_title(&self.messages),
            created_at: Utc::now().to_rfc3339(),
            messages: self.messages.clone(),
        };
        serde_json::to_string_pretty(&data).map_err(|e| format!("序列化失败: {}", e))
    }

    /// Import a conversation from JSON produced by export_json.
    /// Returns a fully populated Conversation. Returns Err with a
    /// descriptive message on parse failure.
    pub(crate) fn import_json(json: &str) -> Result<Self, String> {
        let conv: ConversationData =
            serde_json::from_str(json).map_err(|e| format!("解析失败: {}", e))?;
        // Validate the imported id is safe (sanitized) before use
        if !sanitize_id(&conv.id) {
            return Err(format!("非法对话 id: {}", conv.id));
        }
        Ok(Self {
            id: conv.id,
            messages: conv.messages,
            persisted_count: 0,
        })
    }
}

/// Import legacy `*.json` conversation files into SQLite (one-time).
fn migrate_from_json(c: &mut Connection) -> Result<(), String> {
    #[derive(Deserialize)]
    struct Legacy {
        id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        created_at: String,
        #[serde(default)]
        messages: Vec<Message>,
    }

    let dir = conversations_dir();
    let mut imported = 0usize;
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| format!("read conversations dir: {}", e))?
        .flatten();
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            files.push(path);
        }
    }
    if files.is_empty() {
        return Ok(());
    }

    let tx = c.transaction().map_err(|e| e.to_string())?;
    // 注意：遗留对话 JSON 文件位于 conversations_dir()（data_dir/conversations），
    // 而沙箱目录为 data_dir/sandbox，两者不在同一路径下，
    // 因此不强制使用沙箱读取，避免破坏现有迁移逻辑。
    for path in files {
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let legacy: Legacy = match serde_json::from_str(&raw) {
            Ok(l) => l,
            Err(_) => continue,
        };
        if !sanitize_id(&legacy.id) {
            continue;
        }
        let title = if legacy.title.is_empty() {
            "新对话".to_string()
        } else {
            legacy.title
        };
        let created_at = if legacy.created_at.is_empty() {
            "1970-01-01T00:00:00Z".to_string()
        } else {
            legacy.created_at.clone()
        };
        let updated_at = created_at.clone();
        tx.execute(
            "INSERT OR IGNORE INTO conversations (id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![legacy.id, title, created_at, updated_at],
        )
        .map_err(|e| e.to_string())?;

        // 幂等保护：该对话若已有消息（上次迁移中断后又重跑），跳过重复导入。
        // 否则 messages 表没有 (conv_id, seq) 唯一约束，重复导入会产生重复消息。
        let existing: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE conv_id = ?1",
                params![legacy.id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if existing > 0 {
            continue;
        }

        let mut stmt = tx
            .prepare("INSERT INTO messages (conv_id, role, content, seq) VALUES (?1, ?2, ?3, ?4)")
            .map_err(|e| e.to_string())?;
        for (i, m) in legacy.messages.iter().enumerate() {
            stmt.execute(params![legacy.id, m.role, m.content, i as i64])
                .map_err(|e| e.to_string())?;
        }
        imported += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
    if imported > 0 {
        log::info!("migrated {} conversations from JSON", imported);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_valid_ids() {
        assert!(sanitize_id("20250608_120000_123456"));
        assert!(sanitize_id("abc123"));
        assert!(!sanitize_id("../etc/passwd"));
        assert!(!sanitize_id("C:\\Windows\\evil"));
        assert!(!sanitize_id("a b"));
        assert!(!sanitize_id("a.b"));
    }

    #[test]
    fn test_new_conversation_has_valid_id() {
        let c = Conversation::new();
        assert!(sanitize_id(&c.id));
        assert!(!c.id.is_empty());
    }

    #[test]
    fn test_context_messages_truncation() {
        let mut c = Conversation::new();
        for i in 0..15 {
            c.messages.push(Message {
                role: "user".into(),
                content: format!("msg{}", i),
            });
        }
        let ctx = c.context_messages(None, 5);
        assert_eq!(ctx.len(), 5);
        assert_eq!(ctx[0].content, "msg10");
    }

    #[test]
    fn test_export_import_roundtrip() {
        let mut c = Conversation::new();
        c.messages.push(Message {
            role: "user".into(),
            content: "你好".into(),
        });
        c.messages.push(Message {
            role: "assistant".into(),
            content: "您好！有什么我可以帮您的吗？".into(),
        });
        let json = c.export_json().expect("export failed");
        let parsed = Conversation::import_json(&json).expect("import failed");
        assert_eq!(parsed.id, c.id);
        assert_eq!(parsed.messages.len(), c.messages.len());
        assert_eq!(parsed.messages[0].content, "你好");
        assert_eq!(parsed.messages[1].content, "您好！有什么我可以帮您的吗？");
    }

    #[test]
    fn test_import_rejects_invalid_json() {
        assert!(Conversation::import_json("not json").is_err());
        assert!(Conversation::import_json("{}").is_err());
    }

    #[test]
    fn test_import_rejects_unsafe_id() {
        // The importer must reject ids that contain path separators.
        let json = r#"{
            "id": "../etc/passwd",
            "title": "x",
            "created_at": "2026-01-01T00:00:00Z",
            "messages": []
        }"#;
        assert!(Conversation::import_json(json).is_err());
    }

    #[test]
    fn test_ids_unique_in_tight_loop() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10_000 {
            assert!(seen.insert(Conversation::new().id), "紧凑循环产生重复 id");
        }
    }

    #[test]
    fn test_ids_unique_across_threads() {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..1000)
                        .map(|_| Conversation::new().id)
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        for h in handles {
            for id in h.join().unwrap() {
                assert!(seen.insert(id), "并发创建产生重复 id");
            }
        }
    }

    #[test]
    fn test_incremental_persistence_no_duplicates() {
        crate::test_util::temp_data_dir();
        let mut conv = Conversation::new();
        conv.add_message("user", "a");
        conv.add_message("assistant", "b");
        conv.save(); // 重复保存不应重复落盘
        let r = Conversation::load(&conv.id).expect("load");
        assert_eq!(r.messages.len(), 2);
        conv.add_message("user", "c");
        let r = Conversation::load(&conv.id).expect("load");
        assert_eq!(r.messages.len(), 3);
        assert_eq!(r.messages[2].content, "c");
        Conversation::delete(&conv.id);
    }

    #[test]
    fn test_interleaved_conversations_keep_order() {
        crate::test_util::temp_data_dir();
        let mut c1 = Conversation::new();
        let mut c2 = Conversation::new();
        c1.add_message("user", "1-a");
        c2.add_message("user", "2-a");
        c1.add_message("user", "1-b");
        c2.add_message("user", "2-b");
        let r1 = Conversation::load(&c1.id).unwrap();
        let r2 = Conversation::load(&c2.id).unwrap();
        assert_eq!(
            r1.messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>(),
            vec!["1-a", "1-b"]
        );
        assert_eq!(
            r2.messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>(),
            vec!["2-a", "2-b"]
        );
        Conversation::delete(&c1.id);
        Conversation::delete(&c2.id);
    }

    #[test]
    fn test_long_conversation_persists_and_truncates() {
        crate::test_util::temp_data_dir();
        let mut conv = Conversation::new();
        for i in 0..300 {
            conv.add_message("user", &format!("m{}", i));
        }
        let r = Conversation::load(&conv.id).unwrap();
        assert_eq!(r.messages.len(), 300);
        let ctx = r.context_messages(Some("sys"), 50);
        assert_eq!(ctx.len(), 51);
        assert_eq!(ctx[1].content, "m250");
        assert_eq!(ctx.last().unwrap().content, "m299");
        Conversation::delete(&conv.id);
    }

    #[test]
    fn test_json_migration_is_idempotent() {
        crate::test_util::temp_data_dir();
        let dir = conversations_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = r#"{"id":"legacy_mig_1","title":"旧","created_at":"2025-01-01T00:00:00Z","messages":[{"role":"user","content":"hi"},{"role":"assistant","content":"yo"}]}"#;
        let legacy_path = dir.join("legacy_mig_1.json");
        std::fs::write(&legacy_path, legacy).unwrap();
        let db_path = dir.join("mig_idempotency_test.db");
        let _ = std::fs::remove_file(&db_path);
        {
            let mut c = Connection::open(&db_path).unwrap();
            c.execute_batch(
                "CREATE TABLE IF NOT EXISTS conversations (
                    id TEXT PRIMARY KEY, title TEXT NOT NULL,
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    conv_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                    role TEXT NOT NULL, content TEXT NOT NULL, seq INTEGER NOT NULL);",
            )
            .unwrap();
            migrate_from_json(&mut c).unwrap();
            migrate_from_json(&mut c).unwrap(); // 第二遍不得重复导入
            let msgs: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE conv_id = 'legacy_mig_1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(msgs, 2, "迁移重复执行产生了重复消息");
            let docs: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM conversations WHERE id = 'legacy_mig_1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(docs, 1);
        }
        let _ = std::fs::remove_file(&legacy_path);
        let _ = std::fs::remove_file(&db_path);
    }
}
