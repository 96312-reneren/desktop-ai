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

fn char_len(s: &str) -> usize {
    s.chars().count()
}

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
