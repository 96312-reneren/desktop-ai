//! 红队对抗性用例集（2026-10 第三方渗透测试）
//!
//! 每个用例都是一次真实的攻击尝试，断言"防护应当拦下"：
//!   - 通过 = 防护生效
//!   - 失败 = 找到了一条可绕过的路径（失败信息会给出具体 payload）
//!
//! 覆盖范围仅限 `pub(crate)` 可达的防护边界。`api_server` 的 HTTP 解析器
//! 已有 25 个内联用例（请求走私、重复 Content-Length、控制字符等），
//! 此处不重复；其 DNS 固定逻辑见 `crawler::pin_host`，由本模块的 URL 用例覆盖。
//!
//! web 搜索出口面（URL 构建 / 重定向复检 / 结果页解析）自 2026-10 审查
//! 起纳入第 4 节——search 曾是全项目唯一未做逐跳重定向校验的网络出口。
//!
//! 所有用例只使用回环/保留地址与临时目录，不产生任何对外网络流量。

use crate::rag::chunker::chunk_text;
use crate::rag::cleaner::clean_text;
use crate::rag::crawler::{extract_links, is_ssrf_url, resolve_url, validate_redirect_target};
use crate::rag::search::{build_search_url, parse_ddg_html};
use crate::rag::vector_store::VectorStore;
use crate::server::api_server::parse_http_validated;
use crate::server::sandbox::Sandbox;

// ═══════════════════════════════════════════════════════════════
// 1. SSRF：IP 黑名单的网段覆盖度
// ═══════════════════════════════════════════════════════════════

/// 云元数据服务是 SSRF 的首要目标，优先针对**字面 IP** 形态。
///
/// 注意：形如 `metadata.tencentyun.com` 的主机名不在本用例范围内——它是
/// 域名而非 IP，字符串规则无法也不应判断它，必须由 `crawler::pin_host`
/// 在 DNS 解析后校验。该主机名解析到 169.254.0.23（link-local），会被
/// `pin_host` 拦下，属于第二层防护的职责。
#[test]
fn redteam_ssrf_blocks_cloud_metadata_literal_ips() {
    let cases: [(&str, &str); 11] = [
        (
            "AWS/GCP/Azure 元数据",
            "http://169.254.169.254/latest/meta-data/",
        ),
        (
            "阿里云元数据(CGNAT)",
            "http://100.100.100.200/latest/meta-data/",
        ),
        ("CGNAT 下界", "http://100.64.0.1/"),
        ("CGNAT 上界", "http://100.127.255.254/"),
        ("基准测试段 198.18/15", "http://198.18.0.1/"),
        ("IPv4 组播段", "http://224.0.0.1/"),
        ("保留段 240/4", "http://240.0.0.1/"),
        ("受限广播", "http://255.255.255.255/"),
        ("IETF 协议段 192.0.0/24", "http://192.0.0.1/"),
        ("私网 10/8", "http://10.1.2.3/"),
        ("私网 172.16/12", "http://172.20.1.1/"),
    ];
    let mut escaped = Vec::new();
    for (name, url) in cases {
        if !is_ssrf_url(url) {
            escaped.push(format!("  {:<28} {}", name, url));
        }
    }
    assert!(
        escaped.is_empty(),
        "\nSSRF 防护未覆盖以下目标（{} 项）：\n{}\n",
        escaped.len(),
        escaped.join("\n")
    );
}

/// IPv6 侧的隧道与地址转换写法：Teredo 与 NAT64 都能把流量送回 IPv4 私网。
#[test]
fn redteam_ssrf_blocks_ipv6_tunnel_and_nat64() {
    let cases: [(&str, &str); 8] = [
        ("IPv6 回环(压缩)", "http://[::1]/"),
        ("IPv6 回环(全写)", "http://[0:0:0:0:0:0:0:1]/"),
        ("IPv4 映射 10.x", "http://[::ffff:10.0.0.1]/"),
        ("IPv4 映射 127.x", "http://[::ffff:127.0.0.1]/"),
        ("IPv4 映射 169.254", "http://[::ffff:169.254.169.254]/"),
        ("Teredo 隧道 2001::/32", "http://[2001::1]/"),
        ("NAT64 → 127.0.0.1", "http://[64:ff9b::7f00:1]/"),
        ("IPv6 组播 ff00::/8", "http://[ff02::1]/"),
    ];
    let mut escaped = Vec::new();
    for (name, url) in cases {
        if !is_ssrf_url(url) {
            escaped.push(format!("  {:<28} {}", name, url));
        }
    }
    assert!(
        escaped.is_empty(),
        "\nSSRF 防护未覆盖以下 IPv6 形态（{} 项）：\n{}\n",
        escaped.len(),
        escaped.join("\n")
    );
}

/// 等价写法绕过：十进制 / 八进制 / 十六进制 / 混合 / 用户信息段前缀。
/// 这组用例是"应当被拦下"的，用来确认已有防护没有回归。
#[test]
fn redteam_ssrf_blocks_alternate_spellings() {
    let cases: [(&str, &str); 10] = [
        ("十进制整数", "http://2130706433/"),
        ("十六进制", "http://0x7f000001/"),
        ("八进制", "http://0177.0.0.1/"),
        ("两段式", "http://127.1/"),
        ("四段式", "http://127.0.0.1/"),
        ("userinfo 前缀", "http://evil.com@127.0.0.1/"),
        ("FQDN 尾点", "http://localhost./"),
        ("大写", "http://LOCALHOST/"),
        ("回环 C 段尾", "http://127.255.255.254/"),
        ("link-local", "http://169.254.1.1/"),
    ];
    let mut escaped = Vec::new();
    for (name, url) in cases {
        if !is_ssrf_url(url) {
            escaped.push(format!("  {:<20} {}", name, url));
        }
    }
    assert!(
        escaped.is_empty(),
        "\n等价写法绕过（{} 项）：\n{}\n",
        escaped.len(),
        escaped.join("\n")
    );
}

// ═══════════════════════════════════════════════════════════════
// 2. 文件沙盒：路径穿越
// ═══════════════════════════════════════════════════════════════

/// 各类穿越写法都不得把文件写到沙盒外。
#[test]
fn redteam_sandbox_blocks_path_traversal() {
    let tmp = tempfile::TempDir::new().unwrap();
    let box_dir = tmp.path().join("box");
    let sb = Sandbox::new(box_dir.clone());

    let outside_rel = tmp.path().join("escape_rel.txt");
    let outside_back = tmp.path().join("escape_backslash.txt");
    let outside_abs = tmp.path().join("escape_abs.txt");
    let abs_vec = outside_abs.to_string_lossy().replace('\\', "/");

    let vectors: Vec<(&str, &str)> = vec![
        ("正斜杠上跳", "../escape_rel.txt"),
        ("反斜杠上跳", "..\\escape_backslash.txt"),
        ("深层上跳", "a/b/../../../escape_rel.txt"),
        ("绝对路径", abs_vec.as_str()),
        ("file 前缀", "file://../escape_rel.txt"),
        ("多重斜杠上跳", "....//....//escape_rel.txt"),
        ("点斜杠混合", "./a/../../escape_rel.txt"),
        ("盘符绝对", "C:/Windows/Temp/da_redteam_probe.txt"),
    ];
    for (_name, v) in &vectors {
        let _ = sb.write(v, "pwned");
    }

    let mut breaches = Vec::new();
    for p in [&outside_rel, &outside_back, &outside_abs] {
        if p.exists() {
            breaches.push(p.display().to_string());
        }
    }
    assert!(
        breaches.is_empty(),
        "\n沙盒逃逸：文件被写到沙盒外：\n  {}\n",
        breaches.join("\n  ")
    );
}

/// 符号链接逃逸：沙盒内放一个指向外部的链接。
///
/// 这是本组最关键的一条。`safe_path` 的文档声称"不存在的路径会先
/// canonicalize 父目录再比较"，但该分支未实现——非存在路径只做了
/// `starts_with` 逻辑比较，而 Windows 的 `File::create` 会跟随符号链接。
#[test]
fn redteam_sandbox_blocks_symlink_escape_on_new_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let box_dir = tmp.path().join("box");
    let outside_dir = tmp.path().join("outside");
    std::fs::create_dir_all(&box_dir).unwrap();
    std::fs::create_dir_all(&outside_dir).unwrap();

    let link = box_dir.join("escape_link");
    let linked = make_dir_link(&outside_dir, &link);

    if !linked {
        eprintln!("[跳过] 当前环境不允许创建目录符号链接或联接");
        return;
    }

    // 目标文件尚不存在 → 走"非存在路径"分支
    let _ = sb_write_through_link(&box_dir, "escape_link/pwned.txt");

    let landed = outside_dir.join("pwned.txt");
    assert!(
        !landed.exists(),
        "\n符号链接/联接逃逸成功：通过沙盒内的链接把文件写到了沙盒外\n  链接: {}\n  落点: {}\n",
        link.display(),
        landed.display()
    );
}

/// 在沙盒内建立指向外部的目录链接。
///
/// Windows 上创建符号链接需要开发者模式或管理员权限，但**目录联接
/// （junction）不需要任何特权**——它对 `File::create` 的效果与符号链接
/// 一致，都会被文件系统透明跟随，因此是更现实的攻击路径。
fn make_dir_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() && link.exists() {
            return true;
        }
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        ok && link.exists()
    }
    #[cfg(unix)]
    {
        return std::os::unix::fs::symlink(target, link).is_ok() && link.exists();
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (target, link);
        false
    }
}

/// 通过沙盒写入（拆分出来便于上面的用例保持线性可读）。
fn sb_write_through_link(box_dir: &std::path::Path, rel: &str) -> Result<(), String> {
    let sb = Sandbox::new(box_dir.to_path_buf());
    sb.write(rel, "pwned")
}

// ═══════════════════════════════════════════════════════════════
// 3. 知识库全文检索：注入与语法健壮性
// ═══════════════════════════════════════════════════════════════

/// SQL 注入：查询走参数绑定，注入应无任何效果。
/// 这是"防护应当生效"的正面确认。
#[test]
fn redteam_fts_sql_injection_has_no_effect() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = VectorStore::new(tmp.path());

    let payloads = [
        "' OR 1=1 --",
        "'); DROP TABLE chunks_fts;--",
        "\" UNION SELECT 1,2,3 --",
        "x'; DELETE FROM documents;--",
        "'; ATTACH DATABASE 'owned.db' AS x;--",
    ];
    for p in payloads {
        let _ = store.search_text(p, 5);
    }
    // 表结构必须仍然可用（注意查询词本身要用纯字母，避免混入 FTS 元字符）
    assert!(
        store.search_text("alive", 5).is_ok(),
        "注入载荷执行后 chunks_fts / documents 表已不可用"
    );
}

/// FTS5 语法健壮性：MATCH 的查询语法接受 `* ^ : ( ) AND OR NOT NEAR` 等
/// 元字符。当前实现只中和了双引号，其余元字符会直接进入查询语法。
#[test]
fn redteam_fts_hostile_syntax_does_not_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = VectorStore::new(tmp.path());

    let hostile = [
        "(",
        ")",
        "()",
        "*",
        "^",
        ":",
        "{",
        "}",
        "[",
        "]",
        "-",
        "+",
        "AND",
        "OR",
        "NOT",
        "AND AND AND",
        "a AND",
        "NOT NOT",
        "NEAR(",
        "NEAR(a b)",
        "C++",
        "a*(",
        "a:b",
        "^^^",
        "(((",
        ")))",
    ];
    let mut errors = Vec::new();
    for q in hostile {
        if let Err(e) = store.search_text(q, 5) {
            errors.push(format!("  {:<14} → {}", format!("{:?}", q), e));
        }
    }
    assert!(
        errors.is_empty(),
        "\n以下输入触发 FTS5 语法错误（应转义或兜底为空结果）（{} 项）：\n{}\n",
        errors.len(),
        errors.join("\n")
    );
}

// ═══════════════════════════════════════════════════════════════
// 4. 检索出口面：web 搜索的 URL 构建 / 重定向 / 结果页解析
// ═══════════════════════════════════════════════════════════════

/// 搜索查询直接拼入固定 URL。若编码不完整，输入中的 `& ? # / :` 等
/// 字符可以注入额外参数或改写路径。本用例锁定"查询串只出现在
/// `q=` 之后、且完全由百分号编码构成"这一性质。
#[test]
fn redteam_search_query_injection_has_no_effect() {
    const PREFIX: &str = "https://html.duckduckgo.com/html/?q=";
    let hostile: Vec<(&str, String)> = vec![
        ("参数注入", "q=1&kl=wa-nf".into()),
        ("路径穿越", "../../etc/passwd".into()),
        ("锚点注入", "x#fragment".into()),
        ("协议注入", "file:///etc/passwd".into()),
        ("元数据字面量", "169.254.169.254".into()),
        ("控制字符", "a b\tc\rd\ne".into()),
        ("HTML 标签", "<script>alert(1)</script>".into()),
        ("引号反斜杠", "\"'`\\".into()),
        ("百分号原文", "%41%42%zz".into()),
        ("加号语义", "a+b+c".into()),
        ("中文", "中文查询".into()),
        ("Emoji", "🦀🚀".into()),
        ("超长输入", "x".repeat(10_000)),
    ];
    let mut escaped = Vec::new();
    for (name, q) in &hostile {
        let url = build_search_url(q);
        let tail = match url.strip_prefix(PREFIX) {
            Some(t) => t,
            None => {
                escaped.push(format!("  {:<12} 前缀被改写 → {}", name, url));
                continue;
            }
        };
        if let Some(why) = find_unencoded_byte(tail) {
            escaped.push(format!("  {:<12} {} → 尾段 {:?}", name, why, tail));
        }
    }
    assert!(
        escaped.is_empty(),
        "\n查询串存在未编码字节（{} 项）：\n{}\n",
        escaped.len(),
        escaped.join("\n")
    );
}

/// 编码后的查询段只允许 URL 非保留字符、`+`（空格）与完整的 `%XX`。
/// 返回 Some(原因) 表示发现违规字节。
fn find_unencoded_byte(tail: &str) -> Option<String> {
    let b = tail.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            c if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~' | b'+') => {
                i += 1;
            }
            b'%' => {
                if i + 2 >= b.len()
                    || !b[i + 1].is_ascii_hexdigit()
                    || !b[i + 2].is_ascii_hexdigit()
                {
                    return Some(format!("不完整转义 @{}", i));
                }
                i += 3;
            }
            c => return Some(format!("未编码字节 0x{:02X} @{}", c, i)),
        }
    }
    None
}

/// 重定向 `Location` 完全由响应方控制，是检索出口面最危险的通道。
/// 2026-10 审查发现 search 曾未禁用自动重定向（reqwest 默认跟随且
/// 不复查 SSRF），是全项目唯一漏掉逐跳校验的网络出口。本用例锁定
/// "每一跳必须复检"；同时确认合法跳转不被误杀——否则搜索会在
/// DDG 正常 302 时中断。
#[test]
fn redteam_search_redirect_targets_private_rejected() {
    let base = "https://html.duckduckgo.com/html/?q=x";
    let hostile = [
        ("云元数据", "https://169.254.169.254/latest/meta-data/"),
        ("阿里元数据", "http://100.100.100.200/latest/meta-data/"),
        ("IPv4 回环", "http://127.0.0.1:8080/"),
        ("IPv6 回环", "http://[::1]/"),
        ("NAT64 回环", "http://[64:ff9b::7f00:1]/"),
        ("十进制回环", "http://2130706433/"),
        ("十六进制回环", "http://0x7f000001/"),
        ("八进制回环", "http://0177.0.0.1/"),
        ("userinfo 伪装", "http://evil.com@10.0.0.1/"),
        ("协议相对私网", "//10.0.0.1/x"),
        ("CGNAT 段", "http://100.64.0.1/"),
        ("localhost 主机名", "http://localhost/x"),
    ];
    let mut escaped = Vec::new();
    for (name, loc) in hostile {
        if validate_redirect_target(loc, base).is_ok() {
            escaped.push(format!("  {:<20} Location: {}", name, loc));
        }
    }
    assert!(
        escaped.is_empty(),
        "\n重定向复检放行了私网目标（{} 项）：\n{}\n",
        escaped.len(),
        escaped.join("\n")
    );

    // 合法跳转不得误杀（否则搜索功能会在 DDG 正常跳转时中断）
    let legit = [
        ("绝对公网", "https://duckduckgo.com/html/?q=x"),
        ("根相对", "/html/?q=y"),
        ("协议相对公网", "//duckduckgo.com/html"),
    ];
    let mut blocked = Vec::new();
    for (name, loc) in legit {
        if validate_redirect_target(loc, base).is_err() {
            blocked.push(format!("  {:<20} Location: {}", name, loc));
        }
    }
    assert!(
        blocked.is_empty(),
        "\n误杀合法重定向，搜索会在正常跳转时中断：\n{}\n",
        blocked.join("\n")
    );
}

/// 搜索结果解析器直接消费不可信 HTML（DDG 被劫持或中间人篡改时，
/// 页面内容完全受控）。敌意语料不得 panic；正向对照确保合法结果块
/// 仍能被解析出来，防止"永远 Err"造成的假绿。
#[test]
fn redteam_search_parse_hostile_html_no_panic() {
    let marker = r#"class="result__body""#;
    let corpus: Vec<(&str, String)> = vec![
        ("空输入", String::new()),
        ("巨量重复 marker", marker.repeat(5000)),
        ("marker 在串尾", format!("x {} y", marker)),
        ("深层嵌套未闭合", "<div>".repeat(100_000)),
        (
            "snippet 实体轰炸",
            format!(
                r#"{}<a class="result__snippet">{}&amp;lt;</a>"#,
                marker,
                "&amp;".repeat(50_000)
            ),
        ),
        (
            "属性值含闭合锚",
            format!(r#"{}<a class="result__a" title="</a>">T</a>"#, marker),
        ),
        (
            "百 KB 标题",
            format!(r#"{}<a class="result__a">{}"#, marker, "T".repeat(100_000)),
        ),
        (
            "marker 交错",
            r#"class="result__snippet"class="result__a"class="result__body""#.into(),
        ),
        ("二进制残渣", "\u{0}\u{1}\u{FFFD}garbage\u{7F}".into()),
    ];
    for (_name, html) in &corpus {
        // 唯一断言：不 panic（返回 Ok/Err 均可接受）
        let _ = parse_ddg_html(html);
    }

    // 正向对照：合法结果块必须恰好产出 1 条且字段正确。
    let ok = concat!(
        r#"<div class="result__body">"#,
        r#"<a class="result__a" href="https://example.com/a">Example Title</a>"#,
        r#"<a class="result__url" href="https://example.com/a">example.com/a</a>"#,
        r#"<a class="result__snippet">Example snippet</a>"#,
        "</div>",
    );
    let results = parse_ddg_html(ok).expect("合法结果块解析失败");
    assert_eq!(results.len(), 1, "合法结果块应恰好产出 1 条");
    assert_eq!(results[0].title, "Example Title");
    assert_eq!(results[0].url, "https://example.com/a");
    assert_eq!(results[0].snippet, "Example snippet");
}

// ═══════════════════════════════════════════════════════════════
// 5. NTFS ADS / 设备保留名 / Win32 名字规范化（沙盒数据藏匿面）
// ═══════════════════════════════════════════════════════════════
//
// 差距注记（2026-10 审查）：`sandbox::safe_path` 只拦 `..` 与沙盒外
// canonicalize，对三类 Win32 特殊名字语义无感知。这些语义**不会逃逸
// 沙盒**（写入都落在沙盒内或被设备吞掉），但构成"数据藏匿面"——
// 沙盒里存着 list 看不见、read 读不到的内容。当前 GUI/API 没有任何
// sandbox.write 调用点（kb_panel 只读展示），此面暂不可达；待 Agent
// 协议接入 sandbox.write 后，应实现：拒绝冒号流语法、设备保留名
//（CON/PRN/AUX/NUL/COM1-9/LPT1-9，含带扩展名形态）、尾点/尾空格名。
// 本节用例钉住现状：防止回归，也防止"已修复"的错觉。

/// NTFS 备用数据流：`ads_probe.txt:hidden` 不含 `..`，safe_path 放行；
/// Windows 上写入进入命名流——主流不受污染、目录列举看不到流。非
/// Windows 上冒号只是普通文件名字符，落成沙盒内一个独立文件。两平台
/// 都不得逃逸沙盒。
#[test]
fn redteam_sandbox_ads_write_confined_not_listed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let box_dir = tmp.path().join("box");
    let sb = Sandbox::new(box_dir.clone());

    let _ = sb.write("ads_probe.txt:hidden", "stowaway");
    let _ = sb.write("ads_probe.txt", "main");
    // 主流不被 ADS 写入污染（两平台一致）
    assert_eq!(sb.read("ads_probe.txt").unwrap(), "main");

    #[cfg(windows)]
    {
        let entries: Vec<String> = std::fs::read_dir(&box_dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            entries.iter().all(|n| n != "ads_probe.txt:hidden"),
            "ADS 名不应出现在目录列举: {:?}",
            entries
        );
    }
    // 沙盒外无新增文件（藏匿面 ⊆ 沙盒内）
    assert!(!tmp.path().join("ads_probe.txt").exists());
    assert!(!tmp.path().join("ads_probe.txt:hidden").exists());
}

/// Windows 设备保留名不含 `..`，会被 safe_path 放行——钉住现状：路径
/// 关卡对这些名字既不 panic 也不做拦截。本用例**只探关卡、不打开
/// 设备**（真实打开设备的行为由 NUL 用例单独覆盖）。
#[test]
fn redteam_sandbox_device_names_pass_path_gate() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sb = Sandbox::new(tmp.path().join("box"));
    for name in [
        "CON", "PRN", "AUX", "NUL", "COM1", "LPT2", "CON.txt", "COM1.txt",
    ] {
        let r = sb.safe_path(name);
        assert!(r.is_ok(), "safe_path({name}) 不应 Err: {:?}", r);
    }
}

/// 写 NUL 设备：Windows 语义是丢弃写入。断言不 panic、且沙盒目录内
/// 不出现名为 NUL 的真实文件（设备不占目录项）。
#[test]
#[cfg(windows)]
fn redteam_sandbox_nul_write_swallowed_and_confined() {
    let tmp = tempfile::TempDir::new().unwrap();
    let box_dir = tmp.path().join("box");
    let sb = Sandbox::new(box_dir.clone());

    let _ = sb.write("NUL", "swallowed");
    let entries: Vec<String> = std::fs::read_dir(&box_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        !entries.iter().any(|n| n.eq_ignore_ascii_case("nul")),
        "NUL 不应在沙盒内变成真实文件: {:?}",
        entries
    );
}

/// Win32 名字规范化剥掉尾点/尾空格：`dot.txt.` 与 `dot.txt` 是同一文件。
/// 断言写路径不 panic、（Windows 上）两种写法命中同一文件、沙盒外无残留。
#[test]
fn redteam_sandbox_trailing_dot_names_no_panic() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sb = Sandbox::new(tmp.path().join("box"));

    let _ = sb.write("dot.txt.", "a");
    let _ = sb.write("sp.txt ", "b");
    let _ = sb.write("dot.txt", "c");

    #[cfg(windows)]
    {
        // 尾点/空格被规范化：与 dot.txt 是同一文件，后写覆盖
        assert_eq!(sb.read("dot.txt").unwrap(), "c");
    }
    assert!(!tmp.path().join("dot.txt").exists());
}

// ═══════════════════════════════════════════════════════════════
// 6. 模糊测试种子（零依赖 PRNG · 纯函数鲁棒性）
// ═══════════════════════════════════════════════════════════════
//
// 第 1-5 节是手工构造的攻击向量；本节用固定种子的伪随机补充
// "没想到的形状"：随机字节、随机长度、语料变异（替换/插入/删除）。
// 覆盖对象全部是**零外联纯函数**（清洗/分块/URL 解析/HTML 解析/
// HTTP 解析），断言只有一条：不 panic。种子固定 → 失败可精确复现。

/// xorshift64：测试内嵌 PRNG，零依赖、可复现。种子必须非零。
struct FuzzRng(u64);

impl FuzzRng {
    fn new(seed: u64) -> Self {
        assert!(seed != 0, "xorshift 种子必须非零");
        FuzzRng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn byte(&mut self) -> u8 {
        (self.next_u64() >> 32) as u8
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
    fn random_string(&mut self, max_len: usize) -> String {
        let len = self.below(max_len);
        let bytes: Vec<u8> = (0..len).map(|_| self.byte()).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// 随机字节 → clean_text（html/md/txt 三格式）：清洗管线不 panic。
#[test]
fn redteam_fuzz_clean_text_random_bytes() {
    let mut rng = FuzzRng::new(0x9E3779B97F4A7C15);
    for fmt in ["html", "md", "txt"] {
        for _ in 0..500 {
            let s = rng.random_string(2048);
            let _ = clean_text(&s, fmt);
        }
    }
}

/// 随机长文本 → chunk_text：每段字符数不超上限（字符边界安全）。
#[test]
fn redteam_fuzz_chunker_random_text_respects_char_limit() {
    let mut rng = FuzzRng::new(0x123456789ABCDEF0);
    for _ in 0..300 {
        let s = rng.random_string(30_000);
        for c in chunk_text(&s, 500, 50) {
            assert!(
                c.chars().count() <= 500,
                "chunk 超过 500 字符上限: {}",
                c.chars().count()
            );
        }
    }
}

/// 合法 DDG 结果页语料 → 结构化变异（单字节替换/插入/删除 ×1..8 次）：
/// 解析器不得 panic（与第 4 节静态敌意语料互补）。
#[test]
fn redteam_fuzz_ddg_parser_mutations() {
    let corpus = concat!(
        r#"<div class="result__body">"#,
        r#"<a class="result__a" href="https://example.com/a">Example Title</a>"#,
        r#"<a class="result__url" href="https://example.com/a">example.com/a</a>"#,
        r#"<a class="result__snippet">Example snippet</a>"#,
        "</div>",
    );
    let mut rng = FuzzRng::new(0xDEADBEEFCAFEF00D);
    for _ in 0..1000 {
        let mut bytes = corpus.as_bytes().to_vec();
        for _ in 0..(1 + rng.below(8)) {
            if bytes.is_empty() {
                break;
            }
            let pos = rng.below(bytes.len());
            match rng.below(3) {
                0 => bytes[pos] = rng.byte(),
                1 => bytes.insert(pos, rng.byte()),
                _ => {
                    bytes.remove(pos);
                }
            }
        }
        let html = String::from_utf8_lossy(&bytes);
        let _ = parse_ddg_html(&html);
    }
}

/// 随机串 → is_ssrf_url / resolve_url / extract_links：三个 URL 面
/// 纯函数在任意输入（含裸控制字符、残缺 URL）下不 panic。
#[test]
fn redteam_fuzz_url_parsers_random_strings() {
    let mut rng = FuzzRng::new(0xF00DBEEF12345678);
    let bases = ["https://example.com/a/b", "http://example.com", ""];
    for _ in 0..1000 {
        let s = rng.random_string(512);
        for base in bases {
            let _ = is_ssrf_url(&s);
            let _ = resolve_url(&s, base);
            let _ = extract_links(&s, base);
        }
    }
}

/// 合法 HTTP 请求语料 → 结构化变异：api_server 解析器（25 个内联用例
/// 之外的随机覆盖）不得 panic。变异可破坏请求行/头区/长度字段任意处。
#[test]
fn redteam_fuzz_http_parser_mutations() {
    let corpus = concat!(
        "POST /v1/chat/completions HTTP/1.1\r\n",
        "Host: 127.0.0.1\r\n",
        "Content-Type: application/json\r\n",
        "Content-Length: 7\r\n",
        "Origin: http://localhost:3000\r\n",
        "\r\n",
        "{\"x\":1}",
    );
    let mut rng = FuzzRng::new(0xC0FFEEC0DE202610);
    for _ in 0..1000 {
        let mut bytes = corpus.as_bytes().to_vec();
        for _ in 0..(1 + rng.below(8)) {
            if bytes.is_empty() {
                break;
            }
            let pos = rng.below(bytes.len());
            match rng.below(3) {
                0 => bytes[pos] = rng.byte(),
                1 => bytes.insert(pos, rng.byte()),
                _ => {
                    bytes.remove(pos);
                }
            }
        }
        let raw = String::from_utf8_lossy(&bytes);
        let _ = parse_http_validated(&raw);
    }
}
