//! DuckDuckGo HTML 端点的联网搜索实现。
//!
//! `search_duckduckgo` 是主要入口（由 `ui::app::handlers` 调用）：构建固定端点
//! URL、抓取结果页、解析为 `SearchResult` 列表供 UI 展示与后续抓取使用；
//! `build_search_url` 与 `parse_ddg_html` 另暴露给 redteam 用例以锁定安全性质。
//!
//! 安全边界：本模块是项目直接访问外部网络的出口之一，因此复用 `crawler`
//! 的 SSRF 防护——种子校验、逐跳 `validate_redirect_target`、`pin_host`
//! 域名固定——并对响应体施加字节上限，防护策略与 `crawler::fetch_url`
//! 保持一致。`parse_ddg_html` 直接消费不可信 HTML，须对敌意输入健壮、不 panic。

use std::io::Read;
use std::time::Duration;

/// 单条搜索结果的规范化视图。
pub(crate) struct SearchResult {
    /// 结果标题（已剥离 HTML 标签与实体）。
    pub title: String,
    /// 结果链接（来自锚点 `href`，为原始字符串，未做安全过滤）。
    pub url: String,
    /// 结果摘要文本（已剥离 HTML 标签与实体）。
    pub snippet: String,
}

/// 搜索结果页的字节上限。DDG 的 HTML 结果页通常只有 50–200KB；
/// 恶意或异常响应可以无视 Content-Length 持续输出（同类问题与修复
/// 见 `crawler::fetch_url`），因此必须流式截断，不能 `response.text()`
/// 一次性读入。
const MAX_SEARCH_HTML_BYTES: usize = 2 * 1024 * 1024;

/// 红队审查（2026-10）指出的两个出口面缺口在此修复：
///   1. 旧实现 `resp.text()` 无大小上限——内存放大窗口；
///   2. 旧实现未禁用自动重定向——reqwest 默认跟随且不复查 SSRF，
///      曾是全项目唯一漏掉逐跳校验的网络出口（`crawler::fetch_url`
///      与 `downloader` 均为 Policy::none + 逐跳复检）。
pub(crate) fn search_duckduckgo(query: &str) -> Result<Vec<SearchResult>, String> {
    let url = build_search_url(query);
    let html = fetch_search_html(&url)?;
    parse_ddg_html(&html)
}

/// 固定搜索端点的 URL 构建：查询串全量百分号编码，无法注入额外参数、
/// 改写主机或路径。`pub(crate)` 供 redteam 用例锁定该性质。
pub(crate) fn build_search_url(query: &str) -> String {
    format!("https://html.duckduckgo.com/html/?q={}", urlencoding(query))
}

/// 抓取搜索端点 HTML 并返回解码后的正文。
///
/// 与本项目其他网络出口同构的防护：入参先做 SSRF 校验；每跳禁用自动重定向
/// 并重建 client（`pin_host` 固定 DNS，防 rebinding），`validate_redirect_target`
/// 复检跳转目标，最多 5 跳；429 映射为"过于频繁"，其余非 2xx 映射为服务异常；
/// 正文按 `MAX_SEARCH_HTML_BYTES` 流式截断后做 UTF-8 有损解码。
/// 返回 `Err` 时错误信息已经是可直接展示给用户的中文文案。
fn fetch_search_html(url: &str) -> Result<String, String> {
    // 种子校验：固定端点本应恒为公网，保留此检查使本函数对任意入参
    // 安全，防御未来调用方传入不可信 URL。
    if crate::rag::crawler::is_ssrf_url(url) {
        return Err("禁止访问内网或回环地址".into());
    }

    let mut current_url = url.to_string();
    let mut hops: u32 = 0;
    loop {
        if hops >= 5 {
            return Err("重定向次数过多".into());
        }

        // 每跳新建 client：DNS 单次解析、校验后 pin（防 rebinding），
        // 与 `crawler::fetch_url` 的逐跳防护一致。
        let builder = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .danger_accept_invalid_certs(false)
            .danger_accept_invalid_hostnames(false)
            // 禁自动重定向：每一跳都必须经过下方的 SSRF 复检。
            .redirect(reqwest::redirect::Policy::none());
        let client = crate::rag::crawler::pin_host(builder, &current_url)?
            .build()
            .map_err(|e| format!("client: {}", e))?;

        let resp = client.get(&current_url).send().map_err(|e| {
            if e.is_timeout() {
                "搜索超时，请稍后再试".into()
            } else {
                format!("搜索失败: {}", e)
            }
        })?;

        let status = resp.status().as_u16();
        if status == 429 {
            return Err("搜索过于频繁，请稍后再试".into());
        }

        if resp.status().is_redirection() {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("重定向缺少 Location 头".to_string())?;
            current_url = crate::rag::crawler::validate_redirect_target(location, &current_url)?;
            hops += 1;
            continue;
        }

        if !resp.status().is_success() {
            return Err(format!("搜索服务异常 (HTTP {})", status));
        }

        // 流式截断：先按字节上限读，再解码（与 `crawler::fetch_url`
        // 的修复一致——恶意服务器可无视 Content-Length 持续输出）。
        let mut raw_bytes = Vec::with_capacity(64 * 1024);
        resp.take(MAX_SEARCH_HTML_BYTES as u64 + 1)
            .read_to_end(&mut raw_bytes)
            .map_err(|e| format!("读取失败: {}", e))?;
        if raw_bytes.len() > MAX_SEARCH_HTML_BYTES {
            return Err("搜索结果页面异常过大".into());
        }
        // 本构建未启用 reqwest 的 `charset` 特性，原 `text()` 即为
        // UTF-8 有损解码；此处保持一致的行为。
        return Ok(String::from_utf8_lossy(&raw_bytes).into_owned());
    }
}

/// 对查询串做保守的百分号编码：仅保留 RFC 3986 未保留字符，空格编码为 `+`，
/// 其余字节逐字节大写十六进制转义。逐字节处理可安全覆盖多字节 UTF-8 输入。
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

/// 将半字节（0–15）转为大写十六进制字符，供 `urlencoding` 拼接转义序列。
fn hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + (n - 10)) as char,
    }
}

/// DDG 结果页解析。`pub(crate)`：解析器直接消费不可信 HTML，
/// redteam 模块需要锁定其"敌意输入不 panic"的性质。
pub(crate) fn parse_ddg_html(html: &str) -> Result<Vec<SearchResult>, String> {
    let mut results = Vec::new();

    // Parse DDG HTML result blocks: each result is in a div with class "result"
    let mut search_from = 0usize;
    while let Some(start) = html[search_from..].find(r#"class="result__body""#) {
        let block_start = search_from + start;
        let block = &html[block_start..];

        let block_end = block
            .find(r#"class="result result--"#)
            .or_else(|| block.find(r#"<div class="nav-link"#))
            .unwrap_or(block.len());

        let block = &block[..block_end];

        // Extract title. The marker may be followed by further attributes
        // (`href="..."`) before the opening tag closes; cut at the first
        // `>` so attribute text never leaks into the title. DDG's real
        // markup puts the class before the href, so without this guard
        // titles render as `href="URL"Title`.
        let title = extract_between(block, r#"class="result__a""#, "</a>")
            .and_then(|s| s.split_once('>').map(|(_, rest)| rest))
            .map(strip_html)
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

        // Extract snippet — snippets are anchors too
        // (`<a class="result__snippet" href="...">text</a>`), so apply the
        // same attribute-leak guard as titles.
        let snippet = extract_between(block, r#"class="result__snippet""#, "</a>")
            .and_then(|s| s.split_once('>').map(|(_, rest)| rest))
            .map(strip_html)
            .unwrap_or_default();

        if !title.is_empty() {
            results.push(SearchResult {
                title: title.trim().to_string(),
                url: url.trim().to_string(),
                snippet: snippet.trim().to_string(),
            });
        }

        search_from = block_start + block.len().min(1);
        if results.len() >= 5 {
            break;
        }
    }

    if results.is_empty() {
        Err("未搜索到相关内容，请尝试更换关键词".into())
    } else {
        Ok(results)
    }
}

/// 返回 `s` 中首个 `start_marker` 与紧随其后的首个 `end_marker` 之间的子串。
///
/// 任一标记缺失（或 `start_marker` 之后已无 `end_marker`）时返回 `None`。
/// 仅做朴素顺序查找、不处理嵌套；切片边界均由 `find` 的匹配位置决定，
/// 必落在字符边界上，因此对任意字节内容都不会 panic。
fn extract_between<'a>(s: &'a str, start_marker: &str, end_marker: &str) -> Option<&'a str> {
    let start = s.find(start_marker)? + start_marker.len();
    let slice = &s[start..];
    let end = slice.find(end_marker)?;
    Some(&slice[..end])
}

/// 剥离 HTML 标签并还原常见实体，得到纯文本。
///
/// 采用单遍状态机而非正则，能正确处理标签外的 `<`/`>`（如出现在文本中），
/// 也是"敌意 HTML 不 panic"的保证之一；实体集只覆盖 DDG 结果页实际出现的几种。
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
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
}
