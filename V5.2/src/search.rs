use std::time::Duration;

pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

pub fn search_duckduckgo(query: &str) -> Result<Vec<SearchResult>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| format!("client: {}", e))?;

    let url = format!("https://html.duckduckgo.com/html/?q={}", urlencoding(query));
    let html = client.get(&url).send()
        .map_err(|e| format!("request: {}", e))?
        .text()
        .map_err(|e| format!("text: {}", e))?;

    parse_ddg_html(&html)
}

fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push('%');
                result.push(hex(byte >> 4));
                result.push(hex(byte & 0x0F));
            }
        }
    }
    result
}

fn hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + (n - 10)) as char,
    }
}

fn parse_ddg_html(html: &str) -> Result<Vec<SearchResult>, String> {
    let mut results = Vec::new();

    // Parse DDG HTML result blocks: each result is in a div with class "result"
    let mut search_from = 0usize;
    while let Some(start) = html[search_from..].find(r#"class="result__body""#) {
        let block_start = search_from + start;
        let block = &html[block_start..];

        let block_end = block.find(r#"class="result result--"#)
            .or_else(|| block.find(r#"<div class="nav-link"#))
            .unwrap_or(block.len());

        let block = &block[..block_end];

        // Extract title
        let title = extract_between(block, r#"class="result__a""#, "</a>")
            .map(|s| strip_html(s))
            .unwrap_or_default();

        // Extract URL
        let url = extract_between(block, r#"class="result__url""#, "</a>")
            .and_then(|s| extract_between(s, "href=\"", "\""))
            .map(|s| s.trim().to_string())
            .or_else(|| {
                extract_between(block, "class=\"result__a\"", ">")
                    .and_then(|s| extract_between(s, "href=\"", "\""))
                    .map(|s| s.trim().to_string())
            })
            .unwrap_or_default();

        // Extract snippet
        let snippet = extract_between(block, r#"class="result__snippet""#, "</a>")
            .map(|s| strip_html(s))
            .unwrap_or_default();

        if !title.is_empty() {
            results.push(SearchResult {
                title: title.trim().to_string(),
                url: url.trim().to_string(),
                snippet: snippet.trim().to_string(),
            });
        }

        search_from = block_start + block.len().min(1);
        if results.len() >= 5 { break; }
    }

    if results.is_empty() {
        Err("未找到搜索结果".into())
    } else {
        Ok(results)
    }
}

fn extract_between<'a>(s: &'a str, start_marker: &str, end_marker: &str) -> Option<&'a str> {
    let start = s.find(start_marker)? + start_marker.len();
    let slice = &s[start..];
    let end = slice.find(end_marker)?;
    Some(&slice[..end])
}

fn strip_html(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
        .replace("&quot;", "\"").replace("&#x27;", "'")
}
