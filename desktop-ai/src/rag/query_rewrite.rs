//! 规则化查询重写：纯字符串处理，零推理成本，永不 panic。
//!
//! 在聊天发送前把用户当前输入改写成一条更适合检索（embedding / FTS）的查询串。
//! 本模块不调用任何模型，只做规范化与简单的指代消解；用户看到的原文不变。

use crate::store::config::strip_zero_width;
use crate::store::conversation::Message;

/// 指代词：问题里出现任一即可能指代上文，需要结合上一轮 user 提问补全。
const PRONOUNS: [&str; 11] = [
    "它", "他", "她", "这个", "那个", "这", "那", "上面", "上述", "该", "此",
];

/// 指代消解时上一问题的最大保留字符数，避免拼接后查询过长稀释 embedding 语义。
const CONTEXT_CHARS: usize = 50;

/// 短问题阈值：归一化后字符数小于该值才考虑拼接上文。
const SHORT_QUERY_CHARS: usize = 20;

/// 规则化查询重写：纯字符串处理，零推理成本，永不 panic。
///
/// `current` 是用户当前输入；`history` 是最近几轮对话（role/content 对，
/// 按时间正序，最后一条通常就是 current 对应的 user 消息）。
/// 返回重写后的检索查询；规则未命中时返回 `current` 的规范化形式。
pub(crate) fn rewrite_query(current: &str, history: &[Message]) -> String {
    let normalized = normalize(current);
    let rewritten = resolve_reference(&normalized, history).unwrap_or_else(|| normalized.clone());
    if !rewritten.is_empty() {
        return rewritten;
    }
    // 空结果保护：规范化可能把纯零宽/空白输入清空，此时回退到未经处理的原文。
    current.to_string()
}

/// 规范化：去除零宽字符，折叠连续空白（含换行、制表符）为单个空格并去首尾空白。
fn normalize(text: &str) -> String {
    let cleaned = strip_zero_width(text);
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 判断问题是否包含任一指代词。
fn contains_pronoun(text: &str) -> bool {
    PRONOUNS.iter().any(|p| text.contains(p))
}

/// 按 char 边界安全截断，避免在多字节中文中间切断导致 panic。
fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 在历史中倒序查找当前问题之前最近的一条 user 消息。
///
/// 跳过规范化后与当前问题相同的那条（history 末条通常就是 current 本身），
/// 同时跳过空消息，确保拿到的是真正可用的上文。
fn previous_user_query(normalized_current: &str, history: &[Message]) -> Option<String> {
    history
        .iter()
        .rev()
        .filter(|m| m.role == "user")
        .map(|m| normalize(&m.content))
        .find(|content| !content.is_empty() && content != normalized_current)
}

/// 指代消解：短问题含指代词且存在上一 user 消息时，拼接为「上一问题 当前问题」。
fn resolve_reference(normalized_current: &str, history: &[Message]) -> Option<String> {
    if normalized_current.is_empty()
        || normalized_current.chars().count() >= SHORT_QUERY_CHARS
        || !contains_pronoun(normalized_current)
    {
        return None;
    }
    let previous = previous_user_query(normalized_current, history)?;
    let context = truncate_chars(&previous, CONTEXT_CHARS);
    Some(format!("{} {}", context, normalized_current))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一条测试消息。
    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.into(),
            content: content.into(),
        }
    }

    #[test]
    fn test_normalize_collapses_whitespace_and_zero_width() {
        let raw = "  什么\u{200B}是\n\nRust\t?  ";
        assert_eq!(normalize(raw), "什么是 Rust ?");
    }

    #[test]
    fn test_short_pronoun_joins_previous_user_message() {
        let history = vec![
            msg("user", "Rust 的所有权是什么"),
            msg("assistant", "所有权是一组规则。"),
            msg("user", "它有什么用"),
        ];
        assert_eq!(
            rewrite_query("它有什么用", &history),
            "Rust 的所有权是什么 它有什么用"
        );
    }

    #[test]
    fn test_short_pronoun_without_history_returns_normalized_current() {
        assert_eq!(rewrite_query("这个  怎么用", &[]), "这个 怎么用");
    }

    #[test]
    fn test_long_query_is_not_joined_even_with_pronoun() {
        let current = "这是一个很长很长的问题它已经超过了二十个字符所以不需要拼接";
        let history = vec![msg("user", "上一轮的问题")];
        assert_eq!(rewrite_query(current, &history), current);
    }

    #[test]
    fn test_history_last_equals_current_is_skipped() {
        let history = vec![
            msg("user", "第一个问题是什么"),
            msg("assistant", "回答。"),
            msg("user", "它呢"),
        ];
        assert_eq!(rewrite_query("它呢", &history), "第一个问题是什么 它呢");
    }

    #[test]
    fn test_previous_message_truncated_at_char_boundary() {
        let previous = "中".repeat(60);
        let history = vec![msg("user", &previous), msg("user", "它是什么")];
        let expected = format!("{} {}", "中".repeat(CONTEXT_CHARS), "它是什么");
        assert_eq!(rewrite_query("它是什么", &history), expected);
    }

    #[test]
    fn test_empty_input_returns_empty() {
        assert_eq!(rewrite_query("", &[]), "");
    }
}
