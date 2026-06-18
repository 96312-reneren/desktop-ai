pub fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    let paragraphs: Vec<&str> = text.split_inclusive(|c| c == '\n').collect();
    let mut chunks = Vec::new();
    let mut current = String::new();

    for para in &paragraphs {
        let trimmed = para.trim();
        if trimmed.is_empty() { continue; }

        if current.len() + trimmed.len() > chunk_size && !current.is_empty() {
            chunks.push(current.trim().to_string());
            // Overlap: keep last 'overlap' chars from previous chunk
            if overlap > 0 && chunks.len() > 0 {
                let prev = &chunks[chunks.len() - 1];
                let start = prev.char_indices()
                    .rev().take(overlap).last()
                    .map(|(i, _)| i).unwrap_or(0);
                current = prev[start..].to_string();
            } else {
                current.clear();
            }
        }

        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(trimmed);

        // If a single paragraph exceeds chunk_size, force split
        while current.len() > chunk_size {
            let split_at = current.char_indices()
                .take(chunk_size).last()
                .map(|(i, _)| i).unwrap_or(chunk_size);
            chunks.push(current[..split_at].trim().to_string());
            let rest: String = current[split_at..].to_string();
            current = rest;
        }
    }

    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_chunking() {
        let text = "段落一内容\n段落二内容\n段落三内容";
        let chunks = chunk_text(text, 50, 0);
        assert!(chunks.len() >= 1);
    }

    #[test]
    fn test_empty() {
        assert!(chunk_text("", 512, 0).is_empty());
        assert!(chunk_text("   \n\n  ", 512, 0).is_empty());
    }

    #[test]
    fn test_long_text() {
        let text = "A".repeat(2000);
        let chunks = chunk_text(&text, 512, 64);
        assert!(chunks.len() > 2);
        for c in &chunks {
            assert!(c.len() <= 512);
        }
    }
}
