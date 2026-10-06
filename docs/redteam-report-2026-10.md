# 桌面AI — 红队对抗测试报告

- 测试日期：2026-10-06
- 测试对象：`desktop-ai`（v6.1.6，90 次提交）
- 测试范围：文件沙盒、SSRF 防护、知识库全文检索、运行时库加载
- 测试方式：白盒（直接构造攻击载荷调用防护函数）+ 入口程序黑盒
- 用例位置：`desktop-ai/src/redteam.rs`（309 行，16 个对抗用例）

---

## 一句话结论

**找到一个已实测逃逸的沙盒漏洞、两处 SSRF 黑名单缺口，以及一个由上一轮仓库整改引入的回归（命令行入口完全不可用）。四处均已修复，`cargo test` / `clippy -D warnings` / `fmt --check` 三项门禁全部通过。**

同时有 12 组防护在攻击下站住了，包括全部经典绕过写法——这套防护的底子是扎实的，问题出在网段枚举的完整性，而不是思路。

---

## 一、测试环境与限制（先说清楚证据强度）

| 项 | 情况 | 对测试的影响 |
|---|---|---|
| GGUF 模型 | 本机无任何 `.gguf` 文件 | **API server 无法启动** → HTTP 层无法黑盒攻击 |
| 网络 | 受限（代理返回 502） | 无法下载微型模型补上这一环 |
| 已有构建产物 | `target/debug/` 完整（含 8 个运行时 DLL） | 可编译、可跑测试、可运行入口程序 |
| Rust 工具链 | 1.96.0 | 与 CI 一致 |

因此本轮采用：

- **白盒**：`src/redteam.rs` 直接调用 `crawler::is_ssrf_url`、`Sandbox`、
  `VectorStore::search_text` 等防护边界函数，构造攻击载荷。
- **黑盒**：对编译产出的 `api_serve` 入口程序做真实调用。

**未能覆盖的部分**（需在具备模型的环境补测）：`/v1/chat/completions` 与
`/v1/models` 的 HTTP 层。该层的解析器已有 25 个内联用例（请求走私、
重复 Content-Length、控制字符、超长头、非法方法/版本、CORS 回显等），
本轮以**代码审读**方式复核，结论见第四节。

---

## 二、攻击矩阵

### 站住的防护（12 组，共 40+ 载荷）

| 攻击面 | 载荷 | 结果 |
|---|---|---|
| IP 等价写法 | `http://2130706433/`、`http://0x7f000001/`、`http://0177.0.0.1/`、`http://127.1/` | 全部拦截 |
| userinfo 绕过 | `http://evil.com@127.0.0.1/` | 拦截（按最后一个 `@` 切分，取真实 host） |
| FQDN 尾点 / 大小写 | `http://localhost./`、`http://LOCALHOST/` | 拦截 |
| IPv6 全写形式 | `http://[0:0:0:0:0:0:0:1]/` | 拦截（交标准库解析，非前缀匹配） |
| IPv4 映射 | `http://[::ffff:169.254.169.254]/` | 拦截（解包后按 IPv4 规则判） |
| 路径穿越 | `../`、`..\`、`a/../../`、`....//`、`./a/../../`、绝对路径、`file://`、`C:/Windows/Temp/` | 全部拦截 |
| SQL 注入 | `'); DROP TABLE chunks_fts;--`、`' OR 1=1 --`、`UNION SELECT`、`ATTACH DATABASE` | 无效果（参数绑定，正面确认） |
| DNS 重绑定 | 每跳新建 client + `pin_host` 解析一次即固定 | 设计正确（未做真实外联验证） |
| 重定向链 | 关闭自动跳转，逐跳重新校验并钉 DNS | 设计正确 |
| 本地文件污染 | `nul.txt`、空字节、超 5MB、UTF-16、脚本注入 HTML | 全部被拒 |
| PDF 炸弹 | 空 / 截断 / 垃圾 / 嵌套 PDF | 解析失败但不崩进程 |
| 下载器回环 | 模型下载指向回环地址 | 拦截 |

### 被突破的项（4 组）

| # | 漏洞 | 严重度 | 状态 |
|---|---|---|---|
| 1 | 文件沙盒：目录联接 / 符号链接逃逸 | **高** | 已修复 |
| 2 | SSRF：IPv4 黑名单缺 6 类网段 | **中高** | 已修复 |
| 3 | SSRF：IPv6 隧道与 NAT64 未覆盖 | **中高** | 已修复 |
| 4 | 运行时库加载：命令行入口不可用 | **高** | 已修复 |
| 5 | FTS5 语法健壮性：元字符触发原始错误 | 中 | 已修复 |

---

## 三、漏洞详情

### 3.1 文件沙盒逃逸（已实测逃逸成功）

**这是本轮唯一一条被真实利用成功的漏洞。**

`safe_path` 对**尚不存在**的路径，只做了 `Path::starts_with` 的字符串层级
比较就放行。而 `write()` 随后调用 `File::create`，文件系统会**透明跟随**
符号链接与目录联接——于是沙盒内的一个链接就能把写入重定向到沙盒外。

更值得注意的是：代码注释明确声称"对不存在的路径会 canonicalize 父目录
再比较"，但该分支**并未实现这一步**。文档与实现脱节，是这条漏洞能长期
存在的原因——审阅者读到注释会认为已经处理过了。

实测证据（测试输出原文）：

```
符号链接/联接逃逸成功：通过沙盒内的链接把文件写到了沙盒外
  链接: C:\Users\TheUn\AppData\Local\Temp\.tmpqkD0kv\box\escape_link
  落点: C:\Users\TheUn\AppData\Local\Temp\.tmpqkD0kv\outside\pwned.txt
```

攻击要点：Windows 上创建符号链接需要开发者模式，但**目录联接（junction，
`mklink /J`）不需要任何特权**，对 `File::create` 效果完全一致。因此这是一条
现实中可用的攻击路径，而不是理论问题。

**可达性说明（重要）**：当前 API server 只暴露 `/v1/models` 与
`/v1/chat/completions`，**没有任何文件读写或爬取端点**，所以这条漏洞目前
**不可被远程触发**。它的真实风险在于：`sandbox.rs` 的注释写明这套 API 是
为"P1 阶段的 AI Agent 工具调用协议"预留的。一旦 Agent 协议落地、模型可以
发出 `write("...")` 工具调用，只要沙盒内存在一个链接（用户自己放的，
或解压包自带的），就可以被 prompt 注入诱导写出沙盒外。

**修复**：把注释承诺的那一步补上——对非存在路径，逐级向上找到最深的
已存在祖先目录，canonicalize 后与沙盒根比较，不在根内即拒绝。

```rust
// src/server/sandbox.rs
let mut probe = candidate.clone();
while !probe.exists() {
    match probe.parent() { Some(p) => probe = p.to_path_buf(), None => break }
}
let real_ancestor = std::fs::canonicalize(&probe)?;
if !real_ancestor.starts_with(&self.resolved_root) {
    return Err(format!("路径越界: {}", relative));
}
```

### 3.2 SSRF：IPv4 黑名单缺 6 类网段

`is_private_ipv4` 原本只覆盖 `0/8`、`10/8`、`127/8`、`169.254/16`、
`172.16/12`、`192.168/16`。以下网段全部漏检：

| 网段 | 说明 | 现实影响 |
|---|---|---|
| `100.64.0.0/10` (CGNAT) | RFC 6598 共享地址空间 | **阿里云元数据服务就在此段**（`100.100.100.200`），可读到实例凭据 |
| `198.18.0.0/15` | 基准测试段 | 非路由地址 |
| `224.0.0.0/4` | 组播 | 非单播目标 |
| `240.0.0.0/4` | 保留段（含 `255.255.255.255` 广播） | 非路由地址 |
| `192.0.0.0/24` | IETF 协议分配 | 内部用途 |
| `192.0.2/24`、`198.51.100/24`、`203.0.113/24` | TEST-NET 1/2/3 | 文档/测试段 |

`169.254.169.254`（AWS/GCP/Azure 元数据）原本就被拦住了——说明作者**想到了
元数据攻击**，只是国内云厂商用的 CGNAT 段没在视野里。这是一处典型的
"防护思路正确、枚举不完整"。

### 3.3 SSRF：IPv6 隧道与 NAT64 未覆盖

`is_private_ipv6` 原本只处理回环、未指定、IPv4 映射、`fc00::/7`、`fe80::/10`。
以下形态漏检：

| 形态 | 说明 |
|---|---|
| `2001::/32` | Teredo 隧道，内部封装 IPv4，可把流量送回私网 |
| `64:ff9b::/96` | NAT64（RFC 6052），低 32 位就是目标 IPv4，`64:ff9b::7f00:1` = `127.0.0.1` |
| `2002::/16` | 6to4（RFC 3056），第 2、3 段拼接为目标 IPv4 |
| `ff00::/8` | IPv6 组播 |
| `2001:db8::/32` | 文档地址 |

**修复**：新增 `embedded_ipv4()`，从这三类地址中解出内嵌的 IPv4 再按 IPv4
规则判断。这样 `64:ff9b::7f00:1`、`2002:0a00:0001::` 之类的写法会被自动
拦截，无需逐个枚举——比继续加网段更不容易随 RFC 演进而失效。

### 3.4 根因：第二层防护对字面 IP 直接放行

上面两条之所以是**真实可利用**而非"防御纵深的第一层不完善"，原因在
`pin_host` 的这一段：

```rust
if parse_ipv4(host).is_some() || host.parse::<Ipv6Addr>().is_ok() {
    // 字面 IP：is_ssrf_url 已完成校验
    return Ok(builder);          // ← 直接返回，不做任何复核
}
```

注释假设第一层已经拦住了字面 IP，于是字面 IP 形态**完全跳过** DNS 解析与
IP 校验。结果是：黑名单只要漏一个网段，两层防护就同时失效，
`http://100.100.100.200/` 可以直通。

**修复**：让 `pin_host` 对字面 IP 独立复核一次，不再依赖"第一层应该做过"。
这样两层的职责是重叠的，任一层有遗漏都还有另一层兜底。

**一个应当是设计如此的对照**：主机名 `metadata.tencentyun.com` 无法也不应
用字符串规则判断，它由 `pin_host` 在 DNS 解析后校验（该域名解析到
`169.254.0.23`，落在已拦截的 link-local 段）。这是第二层存在的意义，
我最初把它写进第一层的用例是断言过严，已修正用例。

### 3.5 运行时库加载：命令行入口完全不可用（**上一轮整改引入的回归**）

这条需要说明来源：**它是我上一轮建议的 `vendor/` 迁移带出来的副作用。**

黑盒测试 `api_serve`（`examples/api_serve.rs`，其文档注释明确写着
`cargo run --example api_serve -- <model.gguf>`）的实际表现：

```
$ api_serve.exe /nonexistent/x.gguf 11599 tok
模型加载失败: cannot access llama.dll: 系统找不到指定的文件。 (os error 2)
```

排查过程与结论：

1. `resolve_lib_path()` 只在 exe 同目录找 `llama.dll`。示例二进制位于
   `target/debug/examples/`，而 `build.rs` 把运行时库写在 `target/debug/` →
   **找不到**。
2. 补上"逐级上溯父目录"的搜索后，报错变成 `LoadLibraryExW failed`——
   库找到了但加载不了。
3. 用独立加载器验证：8 个 DLL 全部 `OK`，文件本身没问题。
4. 把工作目录切到 `target/debug` 再运行示例 → **成功**（报错变成模型文件
   不存在，即整条 DLL 栈已通）。

所以根因是 Windows 的加载语义：**解析一个 DLL 的导入表时，搜索的是可执行
文件所在目录与当前工作目录，而不是被加载 DLL 自己的目录。** 示例/测试
二进制在 `examples/` 子目录里，因此看不到 `target/debug/` 里的
`ggml*.dll` 与 MinGW 运行时。

**修复**：在 `ffi::init()` 里、`dlopen` 之前，把库所在目录设为进程 DLL
搜索目录（`SetDllDirectoryW`）。一次修复所有二进制布局（主程序、示例、
测试、将来的 Agent CLI），并且顺带把当前工作目录移出搜索路径，消除了
"从工作目录植入同名 DLL"的劫持面。

```rust
#[cfg(windows)]
fn add_dll_search_dir(lib_path: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    let Some(dir) = lib_path.parent().filter(|d| !d.as_os_str().is_empty()) else { return };
    let mut wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    wide.push(0);
    unsafe { windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW(wide.as_ptr()) };
}
```

修复后验证（从 crate 根运行，此前必失败）：

```
$ api_serve.exe /nonexistent/x.gguf 11599 tok
gguf_init_from_file: failed to open GGUF file '/nonexistent/x.gguf'   ← DLL 栈已通，只差模型
```

**这条的启示**：把二进制从源码目录移走，会连带改变运行时对它们的查找路径。
迁移类改动必须走一遍"实际启动一次"的验证，仅靠 `cargo test` 通过是不够的
——所有 177 个测试在我改之前就是通过的，没有一个覆盖"示例程序能否启动"。

### 3.6 FTS5 查询语法健壮性

`search_text` 原本只把双引号换成空格，其余 FTS5 元字符直接进入查询语法。
由于查询是**参数绑定**（`MATCH ?1`），这**不是 SQL 注入**——但会让正常输入
报错：

| 输入 | 实际错误 |
|---|---|
| `(`、`(((`、`)))` | `fts5: syntax error near ""` |
| `AND`、`OR`、`NOT` | `fts5: syntax error near "AND"` |
| `a:b` | `no such column: a` ← 被解释成列过滤 |
| `^`、`^^^` | `fts5: syntax error near "^"` |
| `C++` | `fts5: syntax error near "+"` |
| `a*(`、`NEAR(` | `fts5: syntax error near "("` |

用户在搜索框里输入 `C++` 或一个带冒号的词就会撞上原始 SQLite 错误。

**修复**：把元字符统一替换为**空格**（而非删除），既保证任何输入都不报错，
又保留分词效果——`GPT-4` → `GPT 4`，与内容侧 unicode61 分词器的结果一致；
再配合小写化中和 `AND`/`OR`/`NOT`/`NEAR` 这些裸运算符（FTS5 只认大写形式，
而 unicode61 本身大小写不敏感，故不影响检索结果）。

---

## 四、HTTP 层代码审读结论（无法黑盒测试，以审读替代）

对 API server 的认证、CORS、请求解析做了逐行复核，**未发现可绕过之处**：

| 检查点 | 实现 | 结论 |
|---|---|---|
| 认证 | `/v1/*` 强制 `Bearer` + `constant_time_eq` 常量时间比较；空 token 直接拒绝 | 正确 |
| 头部大小写 | 解析时 `to_lowercase()` 归一，`authorization` 匹配不受大小写影响 | 正确 |
| CORS 用户信息 | `rsplit('@').next()` 取真实 host，`http://evil@127.0.0.1` 不会被误判 | 正确 |
| CORS 端口 / IPv6 | 剥离端口、处理 `[::1]:8080` 方括号形式 | 正确 |
| CORS 后缀混淆 | `localhost.evil.com` 因精确匹配被拒（fail-closed） | 正确 |
| `Origin: null` | 桌面端拒绝、Android 端放行（cfg 门控且有注释说明） | 设计合理 |
| 未认证端点 | 仅 `/health` 与 `/ready` | 可接受（见下） |

两条低severity 观察：

1. `/ready` 未认证即返回**模型名称**。属轻微信息泄露，对本地回环服务影响
   很小，但如果将来把 API 暴露到局域网需要一并加固。
2. `examples/api_serve.rs` 的 token 默认值是 `"da-local"`（弱口令）。GUI 端
   已做「token 强随机化」，CLI 入口建议同样默认随机生成并在启动时打印一次，
   而不是给一个固定弱值。

---

## 五、修复清单与验证

改动共 6 个文件、新增 1 个测试文件：

| 文件 | 改动 |
|---|---|
| `src/rag/crawler.rs` | IPv4 补齐 6 类网段；新增 `embedded_ipv4()` 处理 NAT64/6to4/映射；IPv6 补 Teredo、组播、文档段；`pin_host` 对字面 IP 独立复核 |
| `src/server/sandbox.rs` | 非存在路径补上"canonicalize 最深已存在祖先并比较"（即注释早已承诺的一步） |
| `src/rag/vector_store.rs` | FTS5 查询元字符中和 + 小写化中和裸运算符 |
| `src/llm/ffi.rs` | `resolve_lib_path` 逐级上溯搜索；新增 `add_dll_search_dir` 修复依赖解析 |
| `Cargo.toml` | `windows-sys` 增加 `Win32_System_LibraryLoader` feature |
| `src/lib.rs` | 挂载 `#[cfg(test)] mod redteam;` |
| `src/redteam.rs` | **新增**：16 个对抗用例（SSRF 网段/形态/等价写法、沙盒穿越/链接逃逸、FTS5 注入/语法） |

验证结果（CI 三项门禁全绿）：

```
cargo test                                           177 passed; 0 failed  (+ 集成 8 passed)
cargo clippy --all-targets -- -D warnings             通过
cargo fmt --check                                     通过
cargo test --lib redteam                              16 passed; 0 failed
```

修复前后的对比很直观：修复前 4 个用例失败（3 个 SSRF/沙盒用例 + 1 个是我
自己写错的用例），修复后全部通过；期间 177 个既有测试无一阵亡，说明这些
修复没有改变既有行为。

---

## 六、本轮顺带发现的其它改进点

| # | 事项 | 位置 | 类型 | 严重度 |
|---|---|---|---|---|
| 1 | `read_local_file` 对本地文件**无任何路径限制**，与 `Sandbox` 的严谨校验形成鲜明反差 | `rag/crawler.rs` | 设计不一致 | 中（见下） |
| 2 | FTS5 命中的 `score` 恒为 `1.0`，在 UI 上会显示成「相似度 100%」 | `rag/vector_store.rs` | 展示误导 | 中 |
| 3 | `/ready` 未认证返回模型名 | `server/api_server.rs` | 信息泄露 | 低 |
| 4 | CLI 入口默认弱 token `"da-local"` | `examples/api_serve.rs` | 弱默认值 | 低 |
| 5 | `target/` 占 3.2 GB 磁盘（已正确忽略，非仓库问题） | `desktop-ai/target` | 磁盘占用 | 低 |
| 6 | `android/build/` 又堆积了 81 MB 产物（已正确忽略） | `android/build` | 磁盘占用 | 低 |

**第 1 条需要展开**：`read_local_file` 接受任意绝对路径，`C:\Users\...\.ssh\id_rsa`
也照读不误，读到的内容会进入知识库并可能随对话输出。好消息是它目前**不可
远程触发**——API server 没有爬取端点，只能由用户在知识库面板里手输。

但它和沙盒形成了危险的反差：项目为（尚未落地的）Agent 协议精心写了
`Sandbox` 路径校验，而爬虫却给了一条**绕过沙盒读任意文件**的旁路。当 P1 的
Agent 协议落地、模型可以自主决定爬什么时，这条旁路会立刻从"用户自己输路径
的脚枪"升级为"prompt 注入 → 任意文件读取 → 数据外泄"的完整链路。

**建议**：让 `read_local_file` 走 `Sandbox` 校验，或至少限制在数据目录与
用户显式选择的文件范围内。

---

## 七、结论

这一轮的性质和上一轮不同。上一轮的问题是"仓库管理乱"，这一轮的问题集中在
**防护逻辑的边界完整性**上：

- 思路都是对的。`pin_host` 的"解析一次即钉住"是防 DNS 重绑定的正确做法；
  手动逐跳校验重定向是对的；常量时间比较是对的；参数绑定是对的；
  `Sandbox` 的路径校验思路也是对的。
- 失误都出在**枚举与复核**上：网段列举不全、`pin_host` 假设第一层已完成
  校验、沙盒漏了注释里已经写明的那一步、加载路径假设了 exe 同目录布局。
- 其中一条（3.5）还是我上一轮的建议引入的，这提醒我：**迁移类改动必须补一次
  "实际启动"验证**，测试全绿不等于程序能跑。

修复后，`src/redteam.rs` 会留在仓库里作为**安全回归套件**。它守护的是四类
过去真实出过问题的边界，任何一条回归都会立刻让 CI 变红。这比一次性的人工
渗透有价值得多。

---

*本报告所有结论均来自实际执行的攻击用例与源码定位。唯一未能黑盒验证的是
HTTP 层（缺 GGUF 模型），该部分已明确标注为代码审读结论。*
