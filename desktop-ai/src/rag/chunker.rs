/// 按句边界把长文本切分为带重叠的定长片段，供向量化与检索使用。
///
/// `chunk_size` 是每段允许的字符数上限（按 Unicode 字符计，非字节），
/// `overlap` 是相邻段之间从上一段末尾保留的字符数，用于避免上下文刚好被
/// 切在段边界处而丢失。输入为空或纯空白时返回空 `Vec`；这是 crate 对外
/// 导出的 API，对任意输入不 panic 且每个片段都不超过 `chunk_size` 字符。
/// Sentence-aware text chunking with character-based sizing.
/// Splits by sentence boundaries (。！？.!?\n), then assembles chunks
/// up to chunk_size chars with overlap between adjacent chunks.
pub fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    let sentences = split_sentences(text);
    let mut chunks = Vec::new();
    let mut current = String::new();

    for sent in &sentences {
        if sent.is_empty() {
            continue;
        }

        if char_len(&current) + char_len(sent) > chunk_size && !current.is_empty() {
            chunks.push(current.trim().to_string());

            if overlap > 0 && !chunks.is_empty() {
                current = build_overlap(&chunks, overlap);
            } else {
                current.clear();
            }
        }

        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(sent);

        while char_len(&current) > chunk_size {
            let split_at = find_split_point(&current, chunk_size);
            chunks.push(current[..split_at].trim().to_string());
            current = current[split_at..].trim().to_string();
        }
    }

    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }

    chunks
}

/// 返回字符串的字符数（Unicode 标量值个数），与字节长度无关。
fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// 按中英文句末标点及换行把文本切成句子，返回已去除首尾空白的非空句子列表。
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        current.push(ch);

        if matches!(ch, '。' | '！' | '？' | '.' | '!' | '?' | '\n') {
            while i + 1 < chars.len() && chars[i + 1].is_whitespace() && chars[i + 1] != '\n' {
                i += 1;
                current.push(chars[i]);
            }
            if i + 1 < chars.len() && chars[i + 1] == '\n' {
                i += 1;
                current.push(chars[i]);
            }
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        }
        i += 1;
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }
    sentences
}

/// 取上一段末尾 `overlap` 个字符作为新段的重叠前缀；`overlap` 为 0、
/// 无历史段或历史段不足 `overlap` 字符时，返回整段或空串。
fn build_overlap(chunks: &[String], overlap: usize) -> String {
    if overlap == 0 || chunks.is_empty() {
        return String::new();
    }
    let prev = &chunks[chunks.len() - 1];
    let cc = char_len(prev);
    if cc <= overlap {
        return prev.clone();
    }
    prev.chars().skip(cc - overlap).collect()
}

/// 在不超过 `max_chars` 字符的范围内为超长句寻找切分点，返回字节偏移：
/// 优先句末标点，其次空格，最后按字符硬切，保证始终落在 UTF-8 字符边界上。
fn find_split_point(text: &str, max_chars: usize) -> usize {
    let indices: Vec<(usize, char)> = text.char_indices().collect();
    if indices.len() <= max_chars {
        return text.len();
    }

    for &(i, ch) in indices[..max_chars].iter().rev() {
        if matches!(ch, '。' | '！' | '？' | '.' | '!' | '?' | '\n') {
            return i + ch.len_utf8();
        }
    }
    for &(i, ch) in indices[..max_chars].iter().rev() {
        if ch == ' ' {
            return i + 1;
        }
    }
    indices
        .get(max_chars - 1)
        .map(|&(i, ch)| i + ch.len_utf8())
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sentence_split() {
        let text = "第一句。第二句！第三句？第四句.第五句!第六句?";
        let sents = split_sentences(text);
        assert_eq!(sents.len(), 6);
    }

    #[test]
    fn test_chunk_with_overlap() {
        let text = "苹果是红色的。香蕉是黄色的。橘子是橙色的。葡萄是紫色的。西瓜是绿色的。";
        let chunks = chunk_text(text, 20, 10);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            let cc = char_len(c);
            assert!(cc <= 20, "chars {} > 20: {}", cc, c);
        }
    }

    #[test]
    fn test_empty() {
        assert!(chunk_text("", 500, 0).is_empty());
        assert!(chunk_text("   \n\n  ", 500, 0).is_empty());
    }

    #[test]
    fn test_long_single_sentence() {
        let text = "A".repeat(2000);
        let chunks = chunk_text(&text, 500, 50);
        assert!(chunks.len() >= 4);
        for c in &chunks {
            assert!(char_len(c) <= 500);
        }
    }

    #[test]
    fn test_chinese_text_chunking() {
        let text = "这是第一句话。这里是第二句话。这是第三句。";
        let chunks = chunk_text(text, 500, 0);
        assert_eq!(chunks.len(), 1);
    }
}
