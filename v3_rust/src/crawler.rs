use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub struct CrawledPage {
    pub url: String,
    pub title: String,
    pub text: String,
    pub text_size: usize,
}

pub struct CrawlConfig {
    pub max_depth: u32,
    pub max_pages: usize,
    pub max_size_per_page: usize,
    pub timeout_secs: u64,
    pub stop_flag: Option<Arc<AtomicBool>>,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_pages: 20,
            max_size_per_page: 3_000_000,
            timeout_secs: 15,
            stop_flag: None,
        }
    }
}

fn is_stopped(cfg: &CrawlConfig) -> bool {
    cfg.stop_flag.as_ref().map(|f| f.load(Ordering::Relaxed)).unwrap_or(false)
}

fn is_url(src: &str) -> bool {
    src.starts_with("http://") || src.starts_with("https://")
}

fn is_file(src: &str) -> bool {
    if src.starts_with("file://") { return true; }
    let p = PathBuf::from(src);
    if p.is_absolute() && p.exists() { return true; }
    false
}

fn strip_file_prefix(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("file://") { rest }
    else if let Some(rest) = s.strip_prefix("file:") { rest }
    else { s }
}

fn read_local_file(path: &str) -> Result<(String, String), String> {
    let file_path = PathBuf::from(strip_file_prefix(path));
    if !file_path.exists() {
        return Err(format!("文件不存在: {}", path));
    }
    let ext = file_path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let raw = if ext == "pdf" {
        pdf_extract::extract_text(&file_path)
            .map_err(|e| format!("PDF解析失败: {}", e))?
    } else {
        std::fs::read_to_string(&file_path)
            .map_err(|e| format!("读取失败: {}", e))?
    };

    if raw.len() > 5_000_000 {
        return Err("文件过大(>5MB)".into());
    }

    let format = if ext == "html" || ext == "htm" { "html" } else { "text" };
    Ok((format.to_string(), raw))
}

fn fetch_url(url: &str, cfg: &CrawlConfig) -> Result<(String, String), String> {
    if is_stopped(cfg) { return Err("已取消".into()); }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(cfg.timeout_secs))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) DesktopAI/5.7")
        .build()
        .map_err(|e| format!("连接失败: {}", e))?;

    let response = client.get(url).send()
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }

    let content_type = response.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let raw = response.text()
        .map_err(|e| format!("读取失败: {}", e))?;

    if raw.len() > cfg.max_size_per_page {
        return Err(format!("页面过大(>{:.0}MB)", cfg.max_size_per_page as f64 / 1e6));
    }

    Ok((content_type, raw))
}

fn is_html_content(ct: &str) -> bool {
    ct.contains("text/html") || ct.is_empty()
}

fn extract_links(html: &str, base_url: &str) -> Vec<String> {
    let mut links = Vec::new();
    let lower = html.to_lowercase();
    let mut search_from = 0usize;

    while let Some(pos) = lower[search_from..].find("href") {
        let abs_pos = search_from + pos;
        let rest = &html[abs_pos + 4..];
        // Skip past = and optional whitespace + opening quote
        let after_eq = rest.trim_start();
        if !after_eq.starts_with('=') { search_from = abs_pos + 4; continue; }
        let after_eq = after_eq[1..].trim_start();

        let (quote_char, content_start) = if after_eq.starts_with('"') {
            ('"', 1)
        } else if after_eq.starts_with('\'') {
            ('\'', 1)
        } else {
            (' ', 0)
        };

        let link_start = abs_pos + 4 + (rest.len() - after_eq.len()) + content_start;
        let rest2 = &html[link_start..];
        let end = if quote_char == ' ' {
            rest2.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(rest2.len())
        } else {
            rest2.find(quote_char).unwrap_or(rest2.len())
        };

        let raw_link = &rest2[..end].trim();
        if !raw_link.is_empty()
            && !raw_link.starts_with('#')
            && !raw_link.starts_with("javascript:")
        {
            let resolved = resolve_url(raw_link, base_url);
            if is_url(&resolved) && !links.contains(&resolved) {
                if links.len() >= 50 { break; }
                links.push(resolved);
            }
        }

        search_from = link_start + end + 1;
    }

    links
}

fn resolve_url(link: &str, base: &str) -> String {
    if link.starts_with("http://") || link.starts_with("https://") {
        return link.to_string();
    }
    if link.starts_with("//") {
        let proto = if base.starts_with("https://") { "https:" } else { "http:" };
        return format!("{}{}", proto, link);
    }
    if link.starts_with('/') {
        // Absolute path
        if let Some(domain_end) = base.find("://") {
            let after_proto = &base[domain_end + 3..];
            if let Some(slash) = after_proto.find('/') {
                return format!("{}{}", &base[..domain_end + 3 + slash], link);
            }
            return format!("{}{}", base.trim_end_matches('/'), link);
        }
    }
    // Relative path
    let base_dir = if let Some(slash) = base.rfind('/') {
        if slash > 8 { &base[..slash] } else { base }
    } else {
        base
    };
    format!("{}/{}", base_dir, link.trim_start_matches("./"))
}

pub fn crawl_url(src: &str) -> Result<CrawledPage, String> {
    let cfg = CrawlConfig::default();
    crawl_single(src, &cfg)
}

fn crawl_single(src: &str, cfg: &CrawlConfig) -> Result<CrawledPage, String> {
    if is_stopped(cfg) { return Err("已取消".into()); }

    let (format, raw) = if is_file(src) {
        read_local_file(src)?
    } else if is_url(src) {
        let (ct, raw) = fetch_url(src, cfg)?;
        let format = if is_html_content(&ct) { "html".to_string() } else { "text".to_string() };
        (format, raw)
    } else {
        // Try as local file path
        let p = PathBuf::from(src);
        if p.exists() {
            read_local_file(src)?
        } else {
            return Err(format!("无法识别的路径: {}", src));
        }
    };

    let (title, text) = if format == "html" {
        crate::cleaner::clean_text(&raw, "html")
    } else {
        crate::cleaner::clean_text(&raw, "text")
    };

    if text.len() < 50 {
        return Err("提取文本少于50字符".into());
    }

    Ok(CrawledPage {
        url: src.to_string(),
        title: if title.is_empty() { src.to_string() } else { title },
        text_size: text.len(),
        text,
    })
}

pub fn crawl_with_depth(start_url: &str, config: CrawlConfig) -> Vec<Result<CrawledPage, String>> {
    let mut results = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut to_visit: Vec<(String, u32)> = vec![(start_url.to_string(), 0)];

    while let Some((url, depth)) = to_visit.pop() {
        if is_stopped(&config) { break; }
        if results.len() >= config.max_pages { break; }
        if visited.contains(&url) { continue; }
        visited.insert(url.clone());

        let is_html = is_url(&url);

        match crawl_single(&url, &config) {
            Ok(page) => {
                let new_depth = depth + 1;
                // Extract and queue links from HTML pages
                if is_html && new_depth <= config.max_depth {
                    let body = if is_file(&url) {
                        std::fs::read_to_string(strip_file_prefix(&url)).unwrap_or_default()
                    } else if is_url(&url) {
                        // We already have the raw from crawl_single, but we need it again for links
                        // Re-fetch only if we're following links
                        fetch_url(&url, &config).map(|(_, raw)| raw).unwrap_or_default()
                    } else {
                        String::new()
                    };

                    let links = extract_links(&body, &url);
                    for link in links.into_iter().rev() {
                        if !visited.contains(&link) && to_visit.len() < config.max_pages {
                            to_visit.push((link, new_depth));
                        }
                    }
                }
                results.push(Ok(page));
            }
            Err(e) => {
                results.push(Err(e));
            }
        }
    }

    results
}

pub fn crawl_multiple(urls: &[String]) -> Vec<Result<CrawledPage, String>> {
    urls.iter().map(|url| crawl_url(url)).collect()
}
