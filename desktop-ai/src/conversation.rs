use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::config::conversations_dir;

fn sanitize_id(id: &str) -> bool {
    id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn safe_path(id: &str) -> Option<std::path::PathBuf> {
    if !sanitize_id(id) {
        return None;
    }
    let path = conversations_dir().join(format!("{}.json", id));
    if !path.starts_with(conversations_dir()) {
        return None;
    }
    Some(path)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
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
}

#[allow(clippy::new_without_default)]
impl Conversation {
    pub fn new() -> Self {
        let id = Utc::now().format("%Y%m%d_%H%M%S_%f").to_string();
        Self {
            id,
            messages: vec![],
        }
    }

    pub fn load(id: &str) -> Option<Self> {
        let path = safe_path(id)?;
        let data = std::fs::read_to_string(&path).ok()?;
        let conv: ConversationData = serde_json::from_str(&data).ok()?;
        Some(Self {
            id: conv.id,
            messages: conv.messages,
        })
    }

    pub fn save(&self) {
        let title = self
            .messages
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
            .unwrap_or_else(|| "新对话".into());

        let data = ConversationData {
            id: self.id.clone(),
            title,
            created_at: Utc::now().to_rfc3339(),
            messages: self.messages.clone(),
        };

        let dir = conversations_dir();
        let mut filename = String::new();
        for c in self.id.chars() {
            if c.is_ascii_alphanumeric() || c == '_' {
                filename.push(c);
            }
        }
        if filename.is_empty() {
            return;
        }
        let path = dir.join(format!("{}.json", filename));
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            if let Err(e) = std::fs::write(&path, &json) {
                log::warn!("failed to save conversation {}: {}", self.id, e);
            }
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
        if let Some(path) = safe_path(id) {
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("failed to delete conversation {}: {}", id, e);
            }
        }
    }

    pub fn list_all() -> Vec<ConversationMeta> {
        let dir = conversations_dir();
        let mut metas: Vec<ConversationMeta> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|e| {
                let data = std::fs::read_to_string(e.path()).ok()?;
                let conv: ConversationData = serde_json::from_str(&data).ok()?;
                Some(ConversationMeta {
                    id: conv.id,
                    title: conv.title,
                    created_at: conv.created_at,
                    message_count: conv.messages.len(),
                })
            })
            .collect();
        metas.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        metas
    }

    pub fn context_messages(&self, system_prompt: Option<&str>, max: usize) -> Vec<Message> {
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
    pub fn export_json(&self) -> Result<String, String> {
        let title = self
            .messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.content.chars().take(50).collect::<String>())
            .unwrap_or_else(|| "新对话".into());
        let data = ConversationData {
            id: self.id.clone(),
            title,
            created_at: Utc::now().to_rfc3339(),
            messages: self.messages.clone(),
        };
        serde_json::to_string_pretty(&data).map_err(|e| format!("序列化失败: {}", e))
    }

    /// Import a conversation from JSON produced by export_json.
    /// Returns a fully populated Conversation. Returns Err with a
    /// descriptive message on parse failure.
    pub fn import_json(json: &str) -> Result<Self, String> {
        let conv: ConversationData =
            serde_json::from_str(json).map_err(|e| format!("解析失败: {}", e))?;
        // Validate the imported id is safe (sanitized) before use
        if !sanitize_id(&conv.id) {
            return Err(format!("非法对话 id: {}", conv.id));
        }
        Ok(Self {
            id: conv.id,
            messages: conv.messages,
        })
    }
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
}
