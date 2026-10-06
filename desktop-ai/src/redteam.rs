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
//! 所有用例只使用回环/保留地址与临时目录，不产生任何对外网络流量。

use crate::rag::crawler::is_ssrf_url;
use crate::rag::vector_store::VectorStore;
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
