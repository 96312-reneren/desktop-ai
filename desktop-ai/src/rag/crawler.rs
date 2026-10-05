use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[allow(dead_code)]
pub(crate) struct CrawledPage {
    pub url: String,
    pub title: String,
    pub text: String,
    pub text_size: usize,
}

pub(crate) struct CrawlConfig {
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
    cfg.stop_flag
        .as_ref()
        .map(|f| f.load(Ordering::Relaxed))
        .unwrap_or(false)
}

fn is_url(src: &str) -> bool {
    src.starts_with("http://") || src.starts_with("https://")
}

fn is_file(src: &str) -> bool {
    if src.starts_with("file://") {
        return true;
    }
    let p = PathBuf::from(src);
    if p.is_absolute() && p.exists() {
        return true;
    }
    false
}

fn strip_file_prefix(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("file://") {
        rest
    } else if let Some(rest) = s.strip_prefix("file:") {
        rest
    } else {
        s
    }
}

/// 提取 PDF 文本，panic 安全（pdf_extract 在损坏文件上可能 panic）。
/// 供爬虫与知识库文件索引共用。
pub(crate) fn extract_pdf_safe(path: &std::path::Path) -> Result<String, String> {
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
/// hostname checks only (DNS-rebinding is handled by `pin_host`); covers
/// `http://127.0.0.1`, `http://localhost`, `http://192.168.x.x`,
/// `http://169.254.169.254`, and every equivalent spelling of a loopback
/// IPv6 literal (`http://[0:0:0:0:0:0:0:1]`, `http://[::0001]`, …).
pub(crate) fn is_ssrf_url(url: &str) -> bool {
    let host = match extract_host(url) {
        Some(h) => h,
        None => return false,
    };
    // FQDN 尾点与裸名等价（localhost. / 127.0.0.1.），先归一化
    let host = host.trim_end_matches('.');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Some(ip) = parse_ipv4(host) {
        return is_private_ipv4(ip);
    }
    // IPv6 字面量交给标准库解析，覆盖压缩/完整/映射/兼容等全部等价写法。
    // （旧实现只匹配 "::1"/"::ffff:" 前缀，`http://[0:0:0:0:0:0:0:1]` 可绕过）
    if let Ok(v6) = host.parse::<std::net::Ipv6Addr>() {
        return is_private_ipv6(v6);
    }
    false
}

/// Private / loopback / link-local / ULA IPv6. IPv4-mapped and legacy
/// IPv4-compatible forms are unwrapped and checked against the IPv4 rules
/// (otherwise `http://[::ffff:10.0.0.1]/` would slip through).
fn is_private_ipv6(v6: std::net::Ipv6Addr) -> bool {
    if v6.is_loopback() || v6.is_unspecified() {
        return true;
    }
    if let Some(v4) = v6.to_ipv4() {
        return is_private_ipv4(v4.octets());
    }
    let s = v6.segments();
    // fc00::/7 唯一本地地址; fe80::/10 链路本地
    (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
}

fn extract_host(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    // 用户信息段（user:pass@host）按 URL 规范取最后一个 '@' 之后的部分；
    // 否则 "http://x@[::1]/" 的主机会被误解析成 "x@["。
    let rest = match rest.rsplit_once('@') {
        Some((_, after)) => after,
        None => rest,
    };
    let host_end = if rest.starts_with('[') {
        // bracketed IPv6 literal, e.g. [::1]:8080
        rest.find(']').map(|i| i + 1).unwrap_or(rest.len())
    } else {
        rest.find(['/', ':', '?', '#']).unwrap_or(rest.len())
    };
    Some(&rest[..host_end])
}

/// 按 inet_aton 语义解析 IPv4：1~4 段，每段接受十进制 / 十六进制(0x) /
/// 八进制(0 前缀)。覆盖 `2130706433`、`127.1`、`0x7f000001` 等全部等价写法。
fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = s.split('.').collect();
    match parts.len() {
        1 => {
            let n = parse_u32_radix(parts[0])?;
            Some(n.to_be_bytes())
        }
        2 => {
            let a = parse_u32_radix(parts[0])?;
            let b = parse_u32_radix(parts[1])?;
            if a > 0xff || b > 0xff_ffff {
                return None;
            }
            Some(((a << 24) | b).to_be_bytes())
        }
        3 => {
            let a = parse_u32_radix(parts[0])?;
            let b = parse_u32_radix(parts[1])?;
            let c = parse_u32_radix(parts[2])?;
            if a > 0xff || b > 0xff || c > 0xffff {
                return None;
            }
            Some(((a << 24) | (b << 16) | c).to_be_bytes())
        }
        4 => {
            let mut out = [0u8; 4];
            for (i, p) in parts.iter().enumerate() {
                let v = parse_u32_radix(p)?;
                if v > 0xff {
                    return None;
                }
                out[i] = v as u8;
            }
            Some(out)
        }
        _ => None,
    }
}

/// Parse one integer component, accepting decimal, hex (0x prefix), and
/// octal (0 prefix — e.g. 0177 = 127).
fn parse_u32_radix(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    if s.len() > 1 && s.starts_with('0') {
        return u32::from_str_radix(s, 8).ok();
    }
    s.parse().ok()
}

fn is_private_ipv4(ip: [u8; 4]) -> bool {
    if ip[0] == 0 {
        return true;
    } // 0.0.0.0/8
    if ip[0] == 10 {
        return true;
    } // 10.0.0.0/8
    if ip[0] == 127 {
        return true;
    } // 127.0.0.0/8 loopback
    if ip[0] == 169 && ip[1] == 254 {
        return true;
    } // 169.254.0.0/16 link-local
    if ip[0] == 172 && (ip[1] & 0xf0) == 16 {
        return true;
    } // 172.16.0.0/12
    if ip[0] == 192 && ip[1] == 168 {
        return true;
    } // 192.168.0.0/16
    false
}

fn read_local_file(path: &str) -> Result<(String, String), String> {
    let file_path = PathBuf::from(strip_file_prefix(path));
    if !file_path.exists() {
        return Err(format!("文件不存在: {}", path));
    }
    let ext = file_path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let raw = if ext == "pdf" {
        extract_pdf_safe(&file_path)?
    } else {
        std::fs::read_to_string(&file_path).map_err(|e| format!("读取失败: {}", e))?
    };

    if raw.len() > 5_000_000 {
        return Err("文件过大(>5MB)".into());
    }

    let format = if ext == "html" || ext == "htm" {
        "html"
    } else {
        "text"
    };
    Ok((format.to_string(), raw))
}

/// Resolve `host` once, reject private/loopback results, and pin the
/// host→IP mapping into the client builder. Pinning defeats DNS rebinding:
/// the connection uses the validated address, not a re-resolved one.
fn pin_host(
    mut builder: reqwest::blocking::ClientBuilder,
    url: &str,
) -> Result<reqwest::blocking::ClientBuilder, String> {
    let host = extract_host(url).ok_or("无法解析 URL 主机名")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if parse_ipv4(host).is_some() || host.parse::<std::net::Ipv6Addr>().is_ok() {
        // 字面 IP：is_ssrf_url 已完成校验
        return Ok(builder);
    }
    use std::net::ToSocketAddrs;
    let addrs: Vec<std::net::SocketAddr> = format!("{}:443", host)
        .to_socket_addrs()
        .map_err(|e| format!("DNS 解析失败: {}", e))?
        .collect();
    if addrs.is_empty() {
        return Err("DNS 解析无结果".into());
    }
    for a in &addrs {
        let private = match a.ip() {
            std::net::IpAddr::V4(v4) => is_private_ipv4(v4.octets()),
            std::net::IpAddr::V6(v6) => is_private_ipv6(v6),
        };
        if private {
            return Err(format!("DNS 解析到内网地址，已拦截: {}", a.ip()));
        }
    }
    for a in addrs {
        builder = builder.resolve(host, a);
    }
    Ok(builder)
}

fn fetch_url(
    url: &str,
    cfg: &CrawlConfig,
    robots: Option<&super::robots::RobotsCache>,
) -> Result<(String, String), String> {
    if is_stopped(cfg) {
        return Err("已取消".into());
    }
    // SSRF: validate the initial URL before the first request.
    if is_ssrf_url(url) {
        return Err("禁止访问内网或回环地址".into());
    }

    let mut current_url = url.to_string();
    let mut hops: u32 = 0;
    let max_hops: u32 = 5;

    loop {
        if is_stopped(cfg) {
            return Err("已取消".into());
        }
        if hops >= max_hops {
            return Err("重定向次数过多".into());
        }

        // Build a fresh client per hop so each target host is DNS-pinned
        // (single resolution, validated, then pinned — rebinding-proof).
        let builder = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .user_agent(super::robots::USER_AGENT)
            // P0-5: disable auto-redirect so we can re-validate each hop.
            .redirect(reqwest::redirect::Policy::none())
            // Explicit TLS verification (default, but stated for consistency
            // with downloader.rs / search.rs and to prevent silent regressions).
            .danger_accept_invalid_certs(false)
            .danger_accept_invalid_hostnames(false);
        let client = pin_host(builder, &current_url)?
            .build()
            .map_err(|e| format!("连接失败: {}", e))?;

        // robots.txt: honor the origin's rules for every hop (the redirect
        // target may live on a different host). The client is already
        // DNS-pinned, so the robots fetch itself is SSRF-safe.
        if let Some(rc) = robots {
            rc.check(&client, &current_url)?;
        }

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
            let location = response
                .headers()
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

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        // 网络安全加固：先按字节上限流式读取，再解码。
        // 旧实现 `response.text()` 会把任意大的响应体一次性读进内存
        // （恶意服务器可无视 Content-Length 持续输出），3MB 限制实际
        // 落在整段读取之后，存在明显的内存 DoS 窗口。
        let mut raw_bytes = Vec::with_capacity(64 * 1024);
        response
            .take(cfg.max_size_per_page as u64 + 1)
            .read_to_end(&mut raw_bytes)
            .map_err(|e| format!("读取失败: {}", e))?;
        if raw_bytes.len() > cfg.max_size_per_page {
            return Err(format!(
                "页面过大(>{:.0}MB)",
                cfg.max_size_per_page as f64 / 1e6
            ));
        }

        // 本构建未启用 reqwest 的 `charset` 特性，原 `text()` 即为 UTF-8
        // 有损解码；此处保持一致的行为。
        let raw = String::from_utf8_lossy(&raw_bytes).into_owned();

        return Ok((content_type, raw));
    }
}

fn is_html_content(ct: &str) -> bool {
    ct.contains("text/html") || ct.is_empty()
}

fn extract_links(html: &str, base_url: &str) -> Vec<String> {
    let mut links = Vec::new();
    // to_ascii_lowercase keeps byte offsets identical to the original string
    // (only ASCII a-z changes; multi-byte UTF-8 is untouched), so slicing
    // the original `html` with offsets found in `lower` is always in bounds.
    let lower = html.to_ascii_lowercase();
    let mut search_from = 0usize;

    while let Some(pos) = lower[search_from..].find("href") {
        let abs_pos = search_from + pos;
        let rest = &html[abs_pos + 4..];
        // Skip past = and optional whitespace + opening quote
        let after_eq = rest.trim_start();
        if !after_eq.starts_with('=') {
            search_from = abs_pos + 4;
            continue;
        }
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
            rest2
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(rest2.len())
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
                if links.len() >= 50 {
                    break;
                }
                links.push(resolved);
            }
        }

        // 攻击面修复：href 值未闭合且落在文档末尾时，end 取到 rest2.len()，
        // link_start + end + 1 会等于 len+1，下一轮 `lower[search_from..]`
        // 直接越界 panic（恶意网页可稳定触发）。钳制到串长即可。
        search_from = (link_start + end + 1).min(html.len());
    }

    links
}

pub(crate) fn resolve_url(link: &str, base: &str) -> String {
    if link.starts_with("http://") || link.starts_with("https://") {
        return link.to_string();
    }
    if link.starts_with("//") {
        let proto = if base.starts_with("https://") {
            "https:"
        } else {
            "http:"
        };
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
        if slash > 8 {
            &base[..slash]
        } else {
            base
        }
    } else {
        base
    };
    format!("{}/{}", base_dir, link.trim_start_matches("./"))
}

pub(crate) fn crawl_url(src: &str) -> Result<CrawledPage, String> {
    let cfg = CrawlConfig::default();
    let robots = super::robots::RobotsCache::new();
    crawl_single(src, &cfg, Some(&robots)).map(|(page, _)| page)
}

fn crawl_single(
    src: &str,
    cfg: &CrawlConfig,
    robots: Option<&super::robots::RobotsCache>,
) -> Result<(CrawledPage, String), String> {
    if is_stopped(cfg) {
        return Err("已取消".into());
    }
    // P0-4: the seed URL itself must pass SSRF validation — the old code
    // only checked extracted sub-links, allowing a direct crawl of
    // `http://127.0.0.1` or `http://169.254.169.254`.
    if is_url(src) && is_ssrf_url(src) {
        return Err("禁止访问内网或回环地址".into());
    }

    let (format, raw) = if is_file(src) {
        read_local_file(src)?
    } else if is_url(src) {
        let (ct, raw) = fetch_url(src, cfg, robots)?;
        let format = if is_html_content(&ct) {
            "html".to_string()
        } else {
            "text".to_string()
        };
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
        crate::rag::cleaner::clean_text(&raw, "html")
    } else {
        crate::rag::cleaner::clean_text(&raw, "text")
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

    Ok((
        CrawledPage {
            url: src.to_string(),
            title: if title.is_empty() {
                src.to_string()
            } else {
                title
            },
            text_size: text.len(),
            text,
        },
        raw,
    ))
}

pub(crate) fn crawl_with_depth(
    start_url: &str,
    config: CrawlConfig,
) -> Vec<Result<CrawledPage, String>> {
    let mut results = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut to_visit: Vec<(String, u32)> = vec![(start_url.to_string(), 0)];
    // One robots.txt cache per crawl session (per-origin memoization).
    let robots = super::robots::RobotsCache::new();

    while let Some((url, depth)) = to_visit.pop() {
        if is_stopped(&config) {
            break;
        }
        if results.len() >= config.max_pages {
            break;
        }
        if visited.contains(&url) {
            continue;
        }
        visited.insert(url.clone());

        let is_html = is_url(&url);

        match crawl_single(&url, &config, Some(&robots)) {
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
pub(crate) fn crawl_multiple(urls: &[String]) -> Vec<Result<CrawledPage, String>> {
    urls.iter().map(|url| crawl_url(url)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 攻击语料：恶意/畸形 HTML 链接解析不得 panic。
    /// （修复前：文档末尾不闭合的 href 会让 search_from 越界 1 字节，稳定 panic）
    #[test]
    fn redteam_extract_links_hostile_html_no_panic() {
        let cases = [
            "<a href=\"x",                 // 结尾未闭合双引号
            "<a href='x",                  // 结尾未闭合单引号
            "<a href=foo",                 // 无引号且无终止符
            "<a href=",                    // 空值
            "<a href",                     // 没有等号
            "href",                        // 只有关键字
            "<a href='x'><a href=\"y",     // 前一个正常、后一个未闭合
            "\u{4e2d}\u{6587}<a href=\"x", // 多字节 + 未闭合
        ];
        for h in cases {
            let links = extract_links(h, "http://example.com/");
            eprintln!("CORPUS extract_links {:?} -> {} links", h, links.len());
        }
    }

    /// 攻击语料：resolve_url 对任意输入不得 panic。
    #[test]
    fn redteam_resolve_url_hostile_no_panic() {
        let links = [
            "",
            "/",
            "//",
            "//evil",
            "http://x",
            "../../../",
            "\u{4e2d}\u{6587}",
            "?q=#f",
            "\\\\server\\share",
            ":",
            "::",
        ];
        let bases = [
            "",
            "http://",
            "http://a",
            "http://a/b",
            "http://[::1]/x",
            "https://a?q=1",
        ];
        for l in links {
            for b in bases {
                let _ = resolve_url(l, b);
            }
        }
    }

    /// 攻击语料：恶意本地文件经 KB/爬虫共用入口 crawl_url 不得 panic。
    #[test]
    fn redteam_hostile_files_via_crawl_url_no_panic() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut files: Vec<(&str, Vec<u8>)> = vec![
            ("empty.txt", vec![]),
            ("one.txt", b"x".to_vec()),
            ("nul.txt", vec![0u8; 1024]),
            ("garbage.bin", (0..=255u8).cycle().take(4097).collect()),
            ("over5mb.txt", vec![b'A'; 5_000_001]),
            ("hugeline.txt", vec![b'B'; 4_900_000]),
            ("fffd.txt", "\u{FFFD}".repeat(2000).into_bytes()),
            ("utf16.txt", {
                let mut v = vec![0xFF, 0xFE];
                for u in "hello 中文".encode_utf16() {
                    v.extend_from_slice(&u.to_le_bytes());
                }
                v
            }),
            ("gbk.txt", vec![0xB0, 0xA1, 0xC4, 0xE3, 0xBA, 0xC3, 0x0A]),
            ("deep.html", "<div>".repeat(100_000).into_bytes()),
            ("unclosed.html", b"<a href=\"x".to_vec()),
            ("entities.html", "&amp;".repeat(100_000).into_bytes()),
            (
                "script.html",
                format!("<script>{}</script>boss", "<".repeat(50_000)).into_bytes(),
            ),
            ("nul.html", vec![0u8; 2048]),
        ];
        for (name, data) in files.drain(..) {
            std::fs::write(dir.path().join(name), &data).unwrap();
        }

        let mut panicked: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let path = entry.unwrap().path();
            let src = path.to_string_lossy().to_string();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            match std::panic::catch_unwind(|| crawl_url(&src)) {
                Ok(Ok(page)) => eprintln!("CORPUS {} OK text_len={}", name, page.text.len()),
                Ok(Err(e)) => eprintln!("CORPUS {} Err({})", name, e),
                Err(_) => panicked.push(name),
            }
        }
        assert!(panicked.is_empty(), "恶意语料触发 panic: {:?}", panicked);
    }

    /// 攻击语料：PDF 炸弹（垃圾头、空文件、百万层嵌套数组）。
    /// 栈溢出无法被 catch_unwind 捕获，需单独运行本测试观察进程是否被打崩。
    #[test]
    fn redteam_pdf_bombs_no_process_kill() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("garbage.pdf"), b"%PDF-1.4\ngarbage").unwrap();
        std::fs::write(dir.path().join("empty.pdf"), b"").unwrap();
        std::fs::write(dir.path().join("nested.pdf"), build_nested_pdf(1_000_000)).unwrap();
        std::fs::write(
            dir.path().join("truncated.pdf"),
            &build_nested_pdf(10)[..120],
        )
        .unwrap();

        for name in ["garbage.pdf", "empty.pdf", "truncated.pdf", "nested.pdf"] {
            let p = dir.path().join(name);
            let res = crawl_url(&p.to_string_lossy());
            eprintln!(
                "PDF {} -> {:?}",
                name,
                res.map(|pg| pg.text.len())
                    .map_err(|e| e.chars().take(60).collect::<String>())
            );
        }
    }

    /// 构造带合法 xref 的最小 PDF，对象 4 为 depth 层嵌套数组。
    fn build_nested_pdf(depth: usize) -> String {
        let nested = format!("{}{}{}", "[".repeat(depth), "0", "]".repeat(depth));
        let mut pdf = String::from("%PDF-1.4\n");
        let objs = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n"
                .to_string(),
            format!("4 0 obj\n{}\nendobj\n", nested),
        ];
        let mut offsets = Vec::new();
        for o in &objs {
            offsets.push(pdf.len());
            pdf.push_str(o);
        }
        let xref_pos = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n", objs.len() + 1));
        pdf.push_str("0000000000 65535 f \n");
        for off in &offsets {
            pdf.push_str(&format!("{:010} 00000 n \n", off));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objs.len() + 1,
            xref_pos
        ));
        pdf
    }

    /// 红队回归：所有回环/内网等价写法都必须被拦截（纯字符串判定，不发起连接）。
    /// 其中 `[0:0:0:0:0:0:0:1]` 完整写法曾实测绕过防护并成功连上本机服务。
    #[test]
    fn redteam_loopback_forms_all_blocked() {
        let blocked = [
            "http://127.0.0.1/",
            "http://127.0.0.1:8080/admin",
            "http://127.1/",
            "http://127.0.1/",
            "http://0177.0.0.1/",
            "http://0x7f.0.0.1/",
            "http://0x7f000001/",
            "http://2130706433/",
            "http://0/",
            "http://1/",
            "http://0.0.0.0/",
            "http://127.0.0.1./",
            "http://localhost/",
            "http://localhost./",
            "http://LOCALHOST/",
            "http://user:pass@localhost/",
            "http://user@127.0.0.1/",
            "http://[::1]/",
            "http://[::0001]/",
            "http://[0::1]/",
            "http://[0:0:0:0:0:0:0:1]/",
            "http://[0:0:0:0:0:0:0:1]:8080/admin",
            "http://[::]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[0:0:0:0:0:ffff:127.0.0.1]/",
            "http://x@[::1]/",
            "http://user:pass@[0:0:0:0:0:0:0:1]:9000/",
            "http://169.254.169.254/latest/meta-data/",
            "http://169.254.0.1/",
            "http://10.0.0.1/",
            "http://10.255.255.255/",
            "http://172.16.0.1/",
            "http://172.31.255.254/",
            "http://192.168.0.1/",
            "http://192.168.255.255/",
            "http://[fe80::1]/",
            "http://[fc00::1]/",
            "http://[fd12:3456:789a::1]/",
        ];
        for u in blocked {
            assert!(is_ssrf_url(u), "red-team: {} 未被拦截", u);
        }
    }

    /// 修复前缀误杀回归：以 fc/fd/fe80 开头的正常公网域名必须放行。
    #[test]
    fn redteam_public_hosts_not_false_positive() {
        assert!(!is_ssrf_url("http://fc2.com/"));
        assert!(!is_ssrf_url("https://fda.gov/"));
        assert!(!is_ssrf_url("http://fe80host.example/"));
        // userinfo 里的 127.0.0.1 不是主机名
        assert!(!is_ssrf_url("http://127.0.0.1@evil.example/"));
        assert!(!is_ssrf_url("http://[2606:4700::1111]/")); // 公网 IPv6
        assert!(!is_ssrf_url("http://[::ffff:8.8.8.8]/")); // 映射公网
    }

    /// 红队实弹 1：完整写法 IPv6 回环必须被拦截，且不产生任何连接。
    #[test]
    fn redteam_ipv6_loopback_never_connected() {
        let listener = match std::net::TcpListener::bind("[::1]:0") {
            Ok(l) => l,
            Err(_) => {
                eprintln!("SKIP: 本机不支持 IPv6 回环");
                return;
            }
        };
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://[0:0:0:0:0:0:0:1]:{}/secret", port);
        let res = crawl_url(&url);
        assert!(res.is_err(), "IPv6 回环必须被拒绝");
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            listener.accept().is_err(),
            "red-team 回归：防护放行了到本机 IPv6 服务的连接"
        );
    }

    /// 红队实弹 2：经典 IPv4 回环（含 userinfo 变体）同样必须零连接。
    #[test]
    fn redteam_loopback_never_connected() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        for url in [
            format!("http://127.0.0.1:{}/secret", port),
            format!("http://user@127.0.0.1:{}/secret", port),
        ] {
            let res = crawl_url(&url);
            assert!(res.is_err(), "{} 必须被拒绝", url);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            listener.accept().is_err(),
            "red-team 回归：防护放行了到本机服务的连接"
        );
    }

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
        assert!(is_ssrf_url("http://[fc00::1]/"));
        assert!(is_ssrf_url("http://[fe80::1]/"));
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
        assert_eq!(
            extract_host("https://example.com/path"),
            Some("example.com")
        );
        assert_eq!(extract_host("http://localhost:8080/x"), Some("localhost"));
        assert_eq!(extract_host("http://127.0.0.1:9000"), Some("127.0.0.1"));
        assert_eq!(extract_host("not a url"), None);
    }

    #[test]
    fn test_parse_ipv4_edges() {
        assert_eq!(parse_ipv4("1.2.3.4"), Some([1, 2, 3, 4]));
        assert_eq!(parse_ipv4("256.0.0.0"), None);
        // inet_aton 短写法：a.b → a.0.0.b，a.b.c → a.b.0.c
        assert_eq!(parse_ipv4("127.1"), Some([127, 0, 0, 1]));
        assert_eq!(parse_ipv4("127.0.1"), Some([127, 0, 0, 1]));
        assert_eq!(parse_ipv4("1.2.3"), Some([1, 2, 0, 3]));
        assert_eq!(parse_ipv4("a.b.c.d"), None);
        // Alternate representations
        assert_eq!(parse_ipv4("2130706433"), Some([127, 0, 0, 1])); // decimal
        assert_eq!(parse_ipv4("0x7f000001"), Some([127, 0, 0, 1])); // hex
        assert_eq!(parse_ipv4("0177.0.0.1"), Some([127, 0, 0, 1])); // octal octet
        assert_eq!(parse_ipv4("0x7f.0.0.1"), Some([127, 0, 0, 1])); // hex octet
    }

    #[test]
    fn test_ssrf_blocks_alternate_encodings() {
        assert!(is_ssrf_url("http://2130706433/")); // decimal 127.0.0.1
        assert!(is_ssrf_url("http://0x7f000001/")); // hex 127.0.0.1
        assert!(is_ssrf_url("http://0177.0.0.1/")); // octal 127.0.0.1
        assert!(is_ssrf_url("http://0x7f.0.0.1/")); // hex octet
        assert!(is_ssrf_url("http://1/")); // decimal 0.0.0.1 (0/8 network)
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
        let garbage: String = "\u{FFFD}".repeat(20);
        let mixed = format!("{}{}", prefix, garbage);

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("garbled.txt");
        std::fs::write(&path, &mixed).ok();

        let result = crawl_url(&path.to_string_lossy());

        assert!(
            result.is_err(),
            "dirty data with dense U+FFFD must be rejected"
        );
    }

    #[test]
    fn test_dirty_data_filter_allows_clean_text() {
        let clean = "这是一段正常的中文文本，用于测试清洗管道是否正确放行。".repeat(5);
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("clean.txt");
        std::fs::write(&path, &clean).ok();

        let result = crawl_url(&path.to_string_lossy());

        assert!(result.is_ok(), "clean Chinese text must pass the guard");
    }
}
