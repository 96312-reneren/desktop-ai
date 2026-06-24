use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[allow(dead_code)]
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

fn extract_pdf_safe(path: &std::path::Path) -> Result<String, String> {
    let path = path.to_path_buf();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        pdf_extract::extract_text(&path)
    }))
    .map_err(|_| "PDF解析时发生panic".to_string())?
    .map_err(|e| format!("PDF解析失败: {}", e))
}

/// True if the URL host points at a private / loopback / link-local address
/// that must NOT be fetched, to prevent SSRF via user-supplied URLs (the
/// crawler follows links from arbitrary pages). Literal-IP and obvious
/// hostname checks only (no DNS resolution, so DNS-rebinding is out of
/// scope); sufficient to block `http://127.0.0.1`, `http://localhost`,
/// `http://192.168.x.x`, `http://169.254.169.254`, etc.
fn is_ssrf_url(url: &str) -> bool {
    let host = match extract_host(url) {
        Some(h) => h,
        None => return false,
    };
    if host.eq_ignore_ascii_case("localhost") { return true; }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Some(ip) = parse_ipv4(host) {
        return is_private_ipv4(ip);
    }
    let lower = host.to_lowercase();
    if lower == "::1"
        || lower.starts_with("fc")
        || lower.starts_with("fd")
        || lower.starts_with("fe80")
        || lower.starts_with("::ffff:")
    {
        return true;
    }
    false
}

fn extract_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://"))?;
    let host_end = if rest.starts_with('[') {
        // bracketed IPv6 literal, e.g. [::1]:8080
        rest.find(']').map(|i| i + 1).unwrap_or(rest.len())
    } else {
        rest.find(['/', ':', '?', '#']).unwrap_or(rest.len())
    };
    Some(&rest[..host_end])
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    // Single decimal integer: http://2130706433/ → 127.0.0.1
    if let Ok(n) = s.parse::<u64>() {
        if n <= u32::MAX as u64 {
            let b = (n >> 24) as u8;
            let c = ((n >> 16) & 0xff) as u8;
            let d = ((n >> 8) & 0xff) as u8;
            let e = (n & 0xff) as u8;
            return Some([b, c, d, e]);
        }
    }
    // Hex: 0x7f000001
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        if let Ok(n) = u64::from_str_radix(hex, 16) {
            if n <= u32::MAX as u64 {
                let b = (n >> 24) as u8;
                let c = ((n >> 16) & 0xff) as u8;
                let d = ((n >> 8) & 0xff) as u8;
                let e = (n & 0xff) as u8;
                return Some([b, c, d, e]);
            }
        }
    }
    // Dotted notation: each octet may be decimal, hex, or octal
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 { return None; }
    let mut out = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        out[i] = parse_octet(p)?;
    }
    Some(out)
}

/// Parse one IPv4 octet, accepting decimal, hex (0x prefix), and
/// octal (0 prefix — e.g. 0177 = 127).
fn parse_octet(s: &str) -> Option<u8> {
    if s.is_empty() { return None; }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u8::from_str_radix(hex, 16).ok();
    }
    if s.len() > 1 && s.starts_with('0') {
        return u8::from_str_radix(s, 8).ok();
    }
    s.parse().ok()
}

fn is_private_ipv4(ip: [u8; 4]) -> bool {
    if ip[0] == 0 { return true; }                 // 0.0.0.0/8
    if ip[0] == 10 { return true; }                // 10.0.0.0/8
    if ip[0] == 127 { return true; }               // 127.0.0.0/8 loopback
    if ip[0] == 169 && ip[1] == 254 { return true; }// 169.254.0.0/16 link-local
    if ip[0] == 172 && (ip[1] & 0xf0) == 16 { return true; } // 172.16.0.0/12
    if ip[0] == 192 && ip[1] == 168 { return true; }         // 192.168.0.0/16
    false
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
        extract_pdf_safe(&file_path)?
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
    // SSRF: validate the initial URL before the first request.
    if is_ssrf_url(url) {
        return Err("禁止访问内网或回环地址".into());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(cfg.timeout_secs))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) DesktopAI/5.7")
        // P0-5: disable auto-redirect so we can re-validate each hop.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("连接失败: {}", e))?;

    let mut current_url = url.to_string();
    let mut hops: u32 = 0;
    let max_hops: u32 = 5;

    loop {
        if is_stopped(cfg) { return Err("已取消".into()); }
        if hops >= max_hops { return Err("重定向次数过多".into()); }

        // Exponential backoff on 429 / 503.
        let mut attempt = 0u32;
        let response = loop {
            match client.get(&current_url).send() {
                Ok(r) => {
                    let s = r.status().as_u16();
                    if (s == 429 || s == 503) && attempt < 3 {
                        attempt += 1;
                        let wait = Duration::from_millis(1000u64.saturating_mul(1 << attempt));
                        std::thread::sleep(wait);
                        continue;
                    }
                    break r;
                }
                Err(e) => return Err(format!("请求失败: {}", e)),
            }
        };

        let status = response.status();

        // Handle redirects manually — re-validate the target before following.
        if status.is_redirection() {
            let location = response.headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("重定向缺少 Location 头".to_string())?;

            // Resolve relative Location against the current URL.
            let next = resolve_url(location.trim(), &current_url);
            if is_ssrf_url(&next) {
                return Err(format!(
                    "重定向目标指向内网地址，已拦截: {} → {}",
                    current_url, next,
                ));
            }
            current_url = next;
            hops += 1;
            continue;
        }

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

        return Ok((content_type, raw));
    }
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
    crawl_single(src, &cfg).map(|(page, _)| page)
}

fn crawl_single(src: &str, cfg: &CrawlConfig) -> Result<(CrawledPage, String), String> {
    if is_stopped(cfg) { return Err("已取消".into()); }
    // P0-4: the seed URL itself must pass SSRF validation — the old code
    // only checked extracted sub-links, allowing a direct crawl of
    // `http://127.0.0.1` or `http://169.254.169.254`.
    if is_url(src) && is_ssrf_url(src) {
        return Err("禁止访问内网或回环地址".into());
    }

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
        return Err("页面可能需 JavaScript 才能正常显示，提取到的内容非常有限".into());
    }

    // Dirty-data guard: if more than 10 % of the text consists of Unicode
    // replacement characters (U+FFFD) the content is garbled (e.g. GBK
    // decoded as UTF-8) and must not enter the chunker / vector store.
    if !text.is_empty() {
        let repl_count = text.chars().filter(|&c| c == '\u{FFFD}').count();
        if repl_count * 10 > text.chars().count() {
            return Err("页面编码异常，文本无法正常解析".into());
        }
    }

    Ok((CrawledPage {
        url: src.to_string(),
        title: if title.is_empty() { src.to_string() } else { title },
        text_size: text.len(),
        text,
    }, raw))
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
            Ok((page, raw)) => {
                let new_depth = depth + 1;
                // Extract and queue links from HTML pages. Reuse the `raw`
                // body already fetched by `crawl_single` instead of issuing
                // a second HTTP request (the old code re-fetched the URL).
                if is_html && new_depth <= config.max_depth {
                    let links = extract_links(&raw, &url);
                    for link in links.into_iter().rev() {
                        if !visited.contains(&link)
                            && to_visit.len() < config.max_pages
                            && !is_ssrf_url(&link)
                        {
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

#[allow(dead_code)]
pub fn crawl_multiple(urls: &[String]) -> Vec<Result<CrawledPage, String>> {
    urls.iter().map(|url| crawl_url(url)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssrf_blocks_loopback_and_private() {
        assert!(is_ssrf_url("http://127.0.0.1/"));
        assert!(is_ssrf_url("http://127.0.1.5:8080/x"));
        assert!(is_ssrf_url("http://localhost/"));
        assert!(is_ssrf_url("http://localhost:3000/"));
        assert!(is_ssrf_url("http://192.168.1.1/"));
        assert!(is_ssrf_url("http://10.0.0.1/"));
        assert!(is_ssrf_url("http://172.16.0.1/"));
        assert!(is_ssrf_url("http://172.31.255.255/"));
        assert!(is_ssrf_url("http://169.254.169.254/latest/meta-data"));
        assert!(is_ssrf_url("http://[::1]/"));
        assert!(is_ssrf_url("http://fc00::1/"));
        assert!(is_ssrf_url("http://fe80::1/"));
    }

    #[test]
    fn test_ssrf_allows_public() {
        assert!(!is_ssrf_url("https://example.com/"));
        assert!(!is_ssrf_url("http://8.8.8.8/"));
        assert!(!is_ssrf_url("https://hf-mirror.com/x"));
        assert!(!is_ssrf_url("http://93.184.216.34/"));
    }

    #[test]
    fn test_ssrf_not_fooled_by_similar_names() {
        // Must not match "localhost" as a substring of another host.
        assert!(!is_ssrf_url("http://localhost.evil.com/"));
        assert!(!is_ssrf_url("http://mylocalhost.com/"));
        // 172.32 is NOT in 172.16/12 private range (172.16 - 172.31).
        assert!(!is_ssrf_url("http://172.32.0.1/"));
        // 11.x is public.
        assert!(!is_ssrf_url("http://11.0.0.1/"));
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(extract_host("https://example.com/path"), Some("example.com"));
        assert_eq!(extract_host("http://localhost:8080/x"), Some("localhost"));
        assert_eq!(extract_host("http://127.0.0.1:9000"), Some("127.0.0.1"));
        assert_eq!(extract_host("not a url"), None);
    }

    #[test]
    fn test_parse_ipv4_edges() {
        assert_eq!(parse_ipv4("1.2.3.4"), Some([1, 2, 3, 4]));
        assert_eq!(parse_ipv4("256.0.0.0"), None);
        assert_eq!(parse_ipv4("1.2.3"), None);
        assert_eq!(parse_ipv4("a.b.c.d"), None);
        // Alternate representations
        assert_eq!(parse_ipv4("2130706433"), Some([127, 0, 0, 1]));  // decimal
        assert_eq!(parse_ipv4("0x7f000001"), Some([127, 0, 0, 1]));  // hex
        assert_eq!(parse_ipv4("0177.0.0.1"), Some([127, 0, 0, 1]));  // octal octet
        assert_eq!(parse_ipv4("0x7f.0.0.1"), Some([127, 0, 0, 1])); // hex octet
    }

    #[test]
    fn test_ssrf_blocks_alternate_encodings() {
        assert!(is_ssrf_url("http://2130706433/"));       // decimal 127.0.0.1
        assert!(is_ssrf_url("http://0x7f000001/"));        // hex 127.0.0.1
        assert!(is_ssrf_url("http://0177.0.0.1/"));        // octal 127.0.0.1
        assert!(is_ssrf_url("http://0x7f.0.0.1/"));        // hex octet
        assert!(is_ssrf_url("http://1/"));                  // decimal 0.0.0.1 (0/8 network)
    }

    #[test]
    fn test_is_private_ipv4_ranges() {
        assert!(is_private_ipv4([127, 0, 0, 1]));
        assert!(is_private_ipv4([10, 255, 255, 255]));
        assert!(is_private_ipv4([192, 168, 0, 1]));
        assert!(is_private_ipv4([172, 16, 0, 1]));
        assert!(is_private_ipv4([172, 31, 255, 255]));
        assert!(is_private_ipv4([169, 254, 0, 1]));
        assert!(is_private_ipv4([0, 0, 0, 0]));
        assert!(!is_private_ipv4([8, 8, 8, 8]));
        assert!(!is_private_ipv4([172, 32, 0, 1]));
    }

    #[test]
    fn test_dirty_data_filter_rejects_high_replacement_chars() {
        // Build text where >10 % of chars are U+FFFD — must trigger the guard.
        let prefix = "Short normal text. ";
        let garbage: String = std::iter::repeat('\u{FFFD}').take(20).collect();
        let mixed = format!("{}{}", prefix, garbage);

        let dir = std::env::temp_dir().join("desktop_ai_dirty_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("garbled.txt");
        std::fs::write(&path, &mixed).ok();

        let result = crawl_url(&path.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_err(), "dirty data with dense U+FFFD must be rejected");
    }

    #[test]
    fn test_dirty_data_filter_allows_clean_text() {
        let clean = "这是一段正常的中文文本，用于测试清洗管道是否正确放行。".repeat(5);
        let dir = std::env::temp_dir().join("desktop_ai_clean_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("clean.txt");
        std::fs::write(&path, &clean).ok();

        let result = crawl_url(&path.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_ok(), "clean Chinese text must pass the guard");
    }
}
