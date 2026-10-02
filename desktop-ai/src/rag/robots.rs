//! robots.txt support for the crawler (RFC 9309).
//!
//! Rules are fetched once per origin, parsed into user-agent groups, and
//! evaluated with the standard longest-match semantics (Allow wins ties).
//! Fetch failures (404/5xx/network) are treated as "no restrictions", the
//! common lenient policy for small crawlers.

use reqwest::blocking::Client;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// User-agent token used for robots matching. The full UA header is
/// "Mozilla/5.0 (Windows NT 10.0; Win64; x64) DesktopAI/5.7"; robots.txt
/// files that target us will use `User-agent: DesktopAI`.
const UA_TOKEN: &str = "DesktopAI";

/// The User-Agent header the crawler sends; also used for robots matching.
pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) DesktopAI/5.7";

#[derive(Debug, Clone)]
struct Rule {
    allow: bool,
    path: String,
}

#[derive(Debug, Default, Clone)]
struct Group {
    agents: Vec<String>,
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

    fn rules_for(&self, client: &Client, origin: &str) -> Arc<RobotsRules> {
        if let Some(r) = self.entries.lock().unwrap().get(origin) {
            return Arc::clone(r);
        }
        let robots_url = format!("{}/robots.txt", origin);
        let text = match client.get(&robots_url).send() {
            Ok(resp) if resp.status().is_success() => {
                let limited: String = resp
                    .text()
                    .unwrap_or_default()
                    .chars()
                    .take(500_000)
                    .collect();
                limited
            }
            // 4xx/5xx/network error: lenient policy — no restrictions.
            _ => String::new(),
        };
        let rules = Arc::new(RobotsRules::parse(&text));
        self.entries
            .lock()
            .unwrap()
            .insert(origin.to_string(), Arc::clone(&rules));
        rules
    }
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
}
