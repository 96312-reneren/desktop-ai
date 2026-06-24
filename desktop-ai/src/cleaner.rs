/// Unified text cleaning pipeline.
/// Converts raw input (HTML, markdown, plain text) into clean JSON-ready plain text.
pub fn clean_text(raw: &str, source_format: &str) -> (String, String) {
    let (title, body) = match source_format {
        "html" => {
            let t = extract_title(raw);
            let b = html_to_text(raw);
            (t, b)
        }
        "markdown" | "md" => {
            let t = extract_first_heading(raw);
            (t, raw.to_string())
        }
        _ => {
            let t = raw.lines().next().unwrap_or("未命名").to_string();
            (t, raw.to_string())
        }
    };
    (normalize_text(&title), normalize_text(&body))
}

fn html_to_text(html: &str) -> String {
    let mut output = String::new();
    let mut in_script = false;
    let mut in_style = false;
    let mut in_tag = false;
    let mut tag_name = String::new();
    let mut prev_was_newline = false;

    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag_name.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let tl = tag_name.to_lowercase();
                if tl == "script" || tl.starts_with("script ") {
                    in_script = true;
                } else if tl == "/script" {
                    in_script = false;
                } else if tl == "style" || tl.starts_with("style ") {
                    in_style = true;
                } else if tl == "/style" {
                    in_style = false;
                } else if tl == "p"
                    || tl == "/p"
                    || tl == "br"
                    || tl == "br/"
                    || tl.starts_with("br ")
                {
                    if !prev_was_newline {
                        output.push('\n');
                        prev_was_newline = true;
                    }
                } else if tl.starts_with("h") && tl.len() == 2
                    || tl == "li"
                    || tl == "/li"
                    || tl == "tr"
                    || tl == "/tr"
                    || tl == "/div"
                {
                    output.push('\n');
                }
            }
            _ if in_tag => {
                tag_name.push(ch);
            }
            _ if in_script || in_style => {}
            _ => {
                output.push(ch);
                if ch == '\n' {
                    prev_was_newline = true;
                } else if !ch.is_whitespace() {
                    prev_was_newline = false;
                }
            }
        }
    }

    decode_entities(&output).replace("\r", "")
}

fn extract_title(html: &str) -> String {
    let lower = html.to_lowercase();
    if let Some(start) = lower.find("<title") {
        if let Some(content_start) = html[start..].find('>') {
            let content = &html[start + content_start + 1..];
            if let Some(end) = content.to_lowercase().find("</title") {
                return html_to_text(&content[..end]);
            }
        }
    }
    String::new()
}

fn extract_first_heading(md: &str) -> String {
    for line in md.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return rest.to_string();
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            return rest.to_string();
        }
    }
    md.lines().next().unwrap_or("未命名").to_string()
}

pub fn normalize_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut prev_was_newline = false;
    let mut prev_was_space = false;

    for ch in text.chars() {
        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }
        match ch {
            '\n' => {
                if !prev_was_newline {
                    output.push('\n');
                    prev_was_newline = true;
                }
                prev_was_space = false;
            }
            '\t' | ' ' => {
                if !prev_was_space && !prev_was_newline {
                    output.push(' ');
                    prev_was_space = true;
                }
                prev_was_newline = false;
            }
            _ => {
                output.push(ch);
                prev_was_newline = false;
                prev_was_space = false;
            }
        }
    }

    // Collapse 3+ newlines to 2
    while output.contains("\n\n\n") {
        output = output.replace("\n\n\n", "\n\n");
    }

    output.trim().to_string()
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_cleaning() {
        let html = "<html><head><title>Test Page</title></head><body><p>Hello <b>World</b></p><script>evil()</script><p>More text</p></body></html>";
        let (title, body) = clean_text(html, "html");
        assert!(title.contains("Test Page"));
        assert!(body.contains("Hello World"));
        assert!(body.contains("More text"));
        assert!(!body.contains("evil"));
        assert!(!body.contains("<script>"));
    }

    #[test]
    fn test_normalize_whitespace() {
        let input = "hello   world\n\n\n\nfoo\tbar\r\nbaz";
        let out = normalize_text(input);
        assert!(!out.contains("   "));
        assert!(!out.contains("\n\n\n"));
        assert!(!out.contains("\t"));
        assert!(!out.contains("\r"));
    }

    #[test]
    fn test_entity_decoding() {
        let input = "A &amp; B &lt; C &gt; D &quot;E&quot;";
        let out = decode_entities(input);
        assert_eq!(out, "A & B < C > D \"E\"");
    }

    #[test]
    fn test_extract_title() {
        assert_eq!(extract_title("<title>My Page</title>"), "My Page");
        assert_eq!(extract_title("<TITLE>hello</TITLE>"), "hello");
    }
}
