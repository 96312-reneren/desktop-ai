//! robots.txt support for the crawler (RFC 9309).
//!
//! Rules are fetched once per origin, parsed into user-agent groups, and
//! evaluated with the standard longest-match semantics (Allow wins ties).
//!
//! Status handling (RFC 9309 §2.2.3.1):
//! - 2xx: parse the body (capped at 500 KiB, streamed — never buffered whole).
//! - 401/403: full disallow — the server explicitly does not want robots.
//! - other 4xx (404 etc.): allow all, per RFC 9309 §2.2.3.1.5.
//! - 5xx: full disallow — server-side trouble means "don't crawl now"
//!   (RFC 9309 §2.2.3.1.3 recommends treating unreachable as full disallow;
//!   also prevents a hostile server from gating us permissive with 503s).
//! - 3xx (we do not follow robots redirects; the pinned client has
//!   Policy::none) and network errors: allow all — RFC 9309 permits the
//!   lenient reading for temporary unavailability, which is friendlier for
//!   a user-initiated desktop crawler on flaky networks. SSRF safety never
//!   depends on robots.txt, so leniency here is not a security hole.

use reqwest::blocking::Client;
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};

/// Cap for a robots.txt body: RFC 9309 §2.2.3.1.1 — parsers SHOULD use at
/// least 500 KiB of the content. We read at most 500 KiB, streaming, so a
/// hostile origin cannot make us buffer an unbounded "robots.txt".
const MAX_ROBOTS_BYTES: usize = 500 * 1024;

/// Body parsed when the server signals "robots not welcome" (401/403) or
/// server-side failure (5xx): a star group that disallows everything.
const FULL_DISALLOW_TEXT: &str = "User-agent: *\nDisallow: /\n";

/// User-agent token used for robots matching. The full UA header is
/// "Mozilla/5.0 (Windows NT 10.0; Win64; x64) DesktopAI/5.7"; robots.txt
/// files that target us will use `User-agent: DesktopAI`.
const UA_TOKEN: &str = "DesktopAI";

/// The User-Agent header the crawler sends; also used for robots matching.
pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) DesktopAI/5.7";

/// 单条 `Allow`/`Disallow` 指令。
#[derive(Debug, Clone)]
struct Rule {
    /// 为真表示 `Allow`，否则表示 `Disallow`。
    allow: bool,
    /// 指令路径模式，可能含 `*`/`$` 通配；大小写与末尾 `$` 均保留原文。
    path: String,
}

/// 一个 `User-agent` 组：共享同一批规则的 UA 模式集合。
#[derive(Debug, Default, Clone)]
struct Group {
    /// 该组声明的 UA 模式（已转小写），`*` 代表任意爬虫。
    agents: Vec<String>,
    /// 该组的规则，评估时按最长匹配优先、同长 `Allow` 胜出。
    rules: Vec<Rule>,
}

/// Parsed robots.txt.
#[derive(Debug, Default, Clone)]
pub(crate) struct RobotsRules {
    groups: Vec<Group>,
}

impl RobotsRules {
    /// Parse robots.txt content. Unknown directives are ignored.
    pub(crate) fn parse(text: &str) -> Self {
        let mut groups: Vec<Group> = Vec::new();
        let mut agents: Vec<String> = Vec::new();
        let mut rules: Vec<Rule> = Vec::new();

        for raw_line in text.lines() {
            // Strip comments and whitespace.
            let line = match raw_line.find('#') {
                Some(i) => &raw_line[..i],
                None => raw_line,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();

            match key.as_str() {
                "user-agent" => {
                    // A new User-agent line after rules starts a new group.
                    if !rules.is_empty() {
                        groups.push(Group {
                            agents: std::mem::take(&mut agents),
                            rules: std::mem::take(&mut rules),
                        });
                    }
                    agents.push(value.to_ascii_lowercase());
                }
                "allow" | "disallow" => {
                    if agents.is_empty() {
                        continue; // rule before any user-agent: ignore
                    }
                    // Empty Disallow means "allow everything" (no rule needed);
                    // empty Allow is a no-op too.
                    if value.is_empty() {
                        continue;
                    }
                    rules.push(Rule {
                        allow: key == "allow",
                        path: value.to_string(),
                    });
                }
                _ => {}
            }
        }
        if !agents.is_empty() || !rules.is_empty() {
            groups.push(Group { agents, rules });
        }
        Self { groups }
    }

    /// Whether `path` may be fetched by our crawler.
    pub(crate) fn is_allowed(&self, path: &str) -> bool {
        let group = self
            .groups
            .iter()
            .find(|g| g.agents.iter().any(|a| ua_matches(a)))
            .or_else(|| {
                self.groups
                    .iter()
                    .find(|g| g.agents.iter().any(|a| a == "*"))
            });
        let Some(group) = group else {
            return true;
        };
        // Longest matching rule wins; Allow wins ties (RFC 9309 §2.2.2).
        let mut best: Option<&Rule> = None;
        for rule in &group.rules {
            if !path_pattern_matches(&rule.path, path) {
                continue;
            }
            match best {
                None => best = Some(rule),
                Some(b)
                    if rule.path.len() > b.path.len()
                        || (rule.path.len() == b.path.len() && rule.allow && !b.allow) =>
                {
                    best = Some(rule);
                }
                _ => {}
            }
        }
        best.map(|r| r.allow).unwrap_or(true)
    }
}

/// 判断 robots.txt 的一条 UA 模式是否命中我们的爬虫。
///
/// 按约定做大小写不敏感的子串匹配：同时对照完整 `USER_AGENT` 与短 token
/// `UA_TOKEN`，使 `User-agent: DesktopAI` 这类写法也能命中。通配 `*` 不作为
/// 具体匹配（返回 `false`），由 `is_allowed` 作为兜底单独处理。
fn ua_matches(pattern: &str) -> bool {
    // robots UA tokens match case-insensitively as substrings of our UA.
    let p = pattern.trim().to_ascii_lowercase();
    if p == "*" {
        return false; // handled as fallback
    }
    USER_AGENT.to_ascii_lowercase().contains(&p) || UA_TOKEN.to_ascii_lowercase().contains(&p)
}

/// robots path pattern matching: `*` matches any run of characters, a
/// trailing `$` anchors the end. Without `$` the pattern is a prefix
/// pattern (an implicit trailing `*`).
fn path_pattern_matches(pattern: &str, path: &str) -> bool {
    let anchored = pattern.ends_with('$');
    let body = if anchored {
        &pattern[..pattern.len() - 1]
    } else {
        pattern
    };
    // Non-anchored patterns behave as prefix patterns.
    let pattern_chars: Vec<char> = if anchored {
        body.chars().collect()
    } else {
        let mut v: Vec<char> = body.chars().collect();
        v.push('*');
        v
    };
    let text: Vec<char> = path.chars().collect();

    // Classic iterative wildcard match with backtracking.
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < text.len() {
        if pi < pattern_chars.len() && pattern_chars[pi] == text[ti] {
            pi += 1;
            ti += 1;
        } else if pi < pattern_chars.len() && pattern_chars[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < pattern_chars.len() && pattern_chars[pi] == '*' {
        pi += 1;
    }
    pi == pattern_chars.len()
}

/// Split a URL into its origin (`scheme://host[:port]`) and path.
fn split_origin_path(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    if !rest.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return None;
    }
    match rest.find('/') {
        Some(i) => {
            let origin = format!("{}://{}", scheme, &rest[..i]);
            let path = &rest[i..];
            Some((origin, path.to_string()))
        }
        None => Some((format!("{}://{}", scheme, rest), "/".to_string())),
    }
}

/// Per-origin robots.txt cache (single crawl session).
pub(crate) struct RobotsCache {
    entries: Mutex<HashMap<String, Arc<RobotsRules>>>,
}

impl RobotsCache {
    /// 创建空的按源缓存。一个爬取会话通常复用一个实例，以摊薄每个源一次抓取。
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Verify the URL against the origin's robots.txt.
    /// The client must already have its target host DNS-pinned (SSRF-safe).
    pub(crate) fn check(&self, client: &Client, url: &str) -> Result<(), String> {
        let Some((origin, path)) = split_origin_path(url) else {
            return Ok(()); // cannot parse; let normal request handling deal with it
        };
        let rules = self.rules_for(client, &origin);
        if rules.is_allowed(&path) {
            Ok(())
        } else {
            Err(format!("robots.txt 不允许爬取: {}", url))
        }
    }

    /// 返回 `origin` 的规则：命中缓存直接复用，否则抓取一次 `robots.txt` 并缓存。
    ///
    /// `client` 必须已对目标主机做好 DNS 固定（SSRF 安全）且禁用自动重定向
    /// （配合 `rules_from_status` 对 3xx 的放行语义）。抓取过程中的网络错误按
    /// "临时不可用"处理，放行全部路径；状态码到规则的映射见 `rules_from_status`。
    /// 缓存在 `Mutex` 下更新，可跨线程共享。
    fn rules_for(&self, client: &Client, origin: &str) -> Arc<RobotsRules> {
        if let Some(r) = self.entries.lock().unwrap().get(origin) {
            return Arc::clone(r);
        }
        let robots_url = format!("{}/robots.txt", origin);
        let rules = Arc::new(match client.get(&robots_url).send() {
            Ok(resp) => rules_from_status(resp.status().as_u16(), || parse_robots_bytes(resp)),
            // Network error: allow all (see module doc for the rationale).
            Err(_) => RobotsRules::parse(""),
        });
        self.entries
            .lock()
            .unwrap()
            .insert(origin.to_string(), Arc::clone(&rules));
        rules
    }
}

/// Map an HTTP status from the robots.txt fetch to parsed rules.
/// `body` is only consulted for 2xx responses. Pure function (testable).
fn rules_from_status(status: u16, body: impl FnOnce() -> RobotsRules) -> RobotsRules {
    match status {
        s if (200..300).contains(&s) => body(),
        401 | 403 => RobotsRules::parse(FULL_DISALLOW_TEXT),
        s if (400..500).contains(&s) => RobotsRules::parse(""),
        s if (500..600).contains(&s) => RobotsRules::parse(FULL_DISALLOW_TEXT),
        // 3xx / 1xx / anything else: we did not follow redirects; treat as
        // temporarily unavailable → allow all (documented leniency).
        _ => RobotsRules::parse(""),
    }
}

/// Stream the robots.txt body with a hard byte cap. The response is never
/// buffered whole: we read at most `MAX_ROBOTS_BYTES` (RFC 9309 advises
/// using the first 500 KiB of oversized files). Truncation that cuts a
/// multi-byte UTF-8 char is healed with the lossy replacement char.
fn parse_robots_bytes(resp: reqwest::blocking::Response) -> RobotsRules {
    let mut buf = Vec::new();
    // take() caps the read stream; +1 lets us detect oversize cheaply if
    // needed later, while truncate() enforces the exact cap.
    let _ = resp
        .take((MAX_ROBOTS_BYTES + 1) as u64)
        .read_to_end(&mut buf);
    if buf.len() > MAX_ROBOTS_BYTES {
        buf.truncate(MAX_ROBOTS_BYTES);
    }
    RobotsRules::parse(&String::from_utf8_lossy(&buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_disallow() {
        let r = RobotsRules::parse("User-agent: *\nDisallow: /private/\n");
        assert!(!r.is_allowed("/private/x"));
        assert!(r.is_allowed("/public"));
        assert!(r.is_allowed("/"));
    }

    #[test]
    fn allow_overrides_longer_match() {
        let r = RobotsRules::parse("User-agent: *\nDisallow: /private/\nAllow: /private/public/\n");
        assert!(!r.is_allowed("/private/secret"));
        assert!(r.is_allowed("/private/public/page"));
    }

    #[test]
    fn wildcard_and_dollar() {
        let r = RobotsRules::parse("User-agent: *\nDisallow: /*.pdf$\nDisallow: /tmp\n");
        assert!(!r.is_allowed("/file.pdf"));
        assert!(r.is_allowed("/file.pdf.html")); // $ anchors the end
        assert!(!r.is_allowed("/tmp/x")); // prefix semantics
        assert!(!r.is_allowed("/a/b/c.pdf"));
    }

    #[test]
    fn specific_agent_wins_over_star() {
        let r = RobotsRules::parse("User-agent: *\nDisallow: /\nUser-agent: DesktopAI\nAllow: /\n");
        assert!(r.is_allowed("/anything"));
    }

    #[test]
    fn empty_or_comment_lines_ignored() {
        let r = RobotsRules::parse("# comment\n\nUser-agent: *\n#x\nDisallow:\n");
        assert!(r.is_allowed("/anything")); // empty Disallow = allow all
    }

    #[test]
    fn no_groups_allows_everything() {
        let r = RobotsRules::parse("");
        assert!(r.is_allowed("/a"));
    }

    #[test]
    fn split_origin_path_works() {
        assert_eq!(
            split_origin_path("https://example.com/a/b?q=1"),
            Some(("https://example.com".into(), "/a/b?q=1".into()))
        );
        assert_eq!(
            split_origin_path("http://example.com"),
            Some(("http://example.com".into(), "/".into()))
        );
    }

    // ── Status mapping (RFC 9309 §2.2.3.1) ──

    #[test]
    fn status_5xx_and_401_403_mean_full_disallow() {
        for status in [401u16, 403, 500, 502, 503, 504] {
            let r = rules_from_status(status, || RobotsRules::parse(""));
            assert!(
                !r.is_allowed("/"),
                "status {status} should disallow everything"
            );
            assert!(!r.is_allowed("/any/path"), "status {status}");
        }
    }

    #[test]
    fn status_404_and_other_4xx_mean_allow_all() {
        for status in [400u16, 404, 410, 451] {
            let r = rules_from_status(status, || panic!("body must not be read"));
            assert!(r.is_allowed("/"), "status {status} should allow all");
            assert!(r.is_allowed("/private"), "status {status}");
        }
    }

    #[test]
    fn status_3xx_unfollowed_means_allow_all() {
        // We never follow robots redirects (pinned client, Policy::none);
        // documented leniency for temporary unavailability.
        for status in [301u16, 302, 307, 308] {
            let r = rules_from_status(status, || panic!("body must not be read"));
            assert!(r.is_allowed("/"), "status {status}");
        }
    }

    #[test]
    fn status_2xx_uses_parsed_body() {
        let r = rules_from_status(200, || {
            RobotsRules::parse("User-agent: *\nDisallow: /private/\n")
        });
        assert!(!r.is_allowed("/private/x"));
        assert!(r.is_allowed("/public"));
        // Body fn must not even be called for non-2xx.
        let mut called = false;
        let r = rules_from_status(503, || {
            called = true;
            RobotsRules::parse("")
        });
        assert!(!called);
        assert!(!r.is_allowed("/"));
    }

    #[test]
    fn oversized_robots_body_cap_truncates_without_panic() {
        // Simulate the byte-cap logic directly: an oversized body is cut to
        // MAX_ROBOTS_BYTES and any partial UTF-8 tail is healed lossily.
        let huge = "a".repeat(MAX_ROBOTS_BYTES + 4096);
        let capped = huge.as_bytes();
        let mut buf = capped.to_vec();
        if buf.len() > MAX_ROBOTS_BYTES {
            buf.truncate(MAX_ROBOTS_BYTES);
        }
        let text = String::from_utf8_lossy(&buf);
        assert_eq!(text.len(), MAX_ROBOTS_BYTES);
        // Multi-byte truncation: cutting mid-char must not panic.
        let multibyte = "多".repeat(MAX_ROBOTS_BYTES / 3 + 100);
        let mut buf = multibyte.as_bytes().to_vec();
        let cut = MAX_ROBOTS_BYTES;
        if cut % 3 != 0 {
            buf.truncate(cut); // lands mid-char on purpose
        }
        let text = String::from_utf8_lossy(&buf);
        assert!(!text.is_empty());
    }
}
