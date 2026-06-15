use serde::{Deserialize, Serialize};
use chrono::Utc;

use crate::config::conversations_dir;

fn sanitize_id(id: &str) -> bool {
    id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn safe_path(id: &str) -> Option<std::path::PathBuf> {
    if !sanitize_id(id) { return None; }
    let path = conversations_dir().join(format!("{}.json", id));
    if !path.starts_with(conversations_dir()) { return None; }
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

impl Conversation {
    pub fn new() -> Self {
        let id = Utc::now().format("%Y%m%d_%H%M%S_%f").to_string();
        Self { id, messages: vec![] }
    }

    pub fn load(id: &str) -> Option<Self> {
        let path = safe_path(id)?;
        let data = std::fs::read_to_string(&path).ok()?;
        let conv: ConversationData = serde_json::from_str(&data).ok()?;
        Some(Self { id: conv.id, messages: conv.messages })
    }

    pub fn save(&self) {
        let title = self.messages.iter()
            .find(|m| m.role == "user")
            .map(|m| {
                let t: String = m.content.chars().take(50).collect();
                if m.content.len() > 50 { format!("{}...", t) } else { t }
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
            if c.is_ascii_alphanumeric() || c == '_' { filename.push(c); }
        }
        if filename.is_empty() { return; }
        let path = dir.join(format!("{}.json", filename));
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            if let Err(e) = std::fs::write(&path, &json) {
                log::warn!("failed to save conversation {}: {}", self.id, e);
            }
        }
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(Message { role: role.into(), content: content.into() });
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
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "json"))
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
            msgs.push(Message { role: "system".into(), content: sp.into() });
        }
        let start = if self.messages.len() > max { self.messages.len() - max } else { 0 };
        msgs.extend(self.messages[start..].to_vec());
        msgs
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
            c.messages.push(Message { role: "user".into(), content: format!("msg{}", i) });
        }
        let ctx = c.context_messages(None, 5);
        assert_eq!(ctx.len(), 5);
        assert_eq!(ctx[0].content, "msg10");
    }
}
