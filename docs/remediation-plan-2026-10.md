# 本地文件读取加固改造计划（2026-10）

- **依据**：`docs/redteam-report-2026-10.md` §6 遗留问题 1（`:315`，展开在 `:322-332`）
- **范围**：`read_local_file` 及其同族的全部本地文件读取路径
- **状态**：阶段一待实施（可立即执行）；阶段二在 harness/agent 落地时执行
- **决策记录**：修复时机 = 两阶段（现在做统一出入口 + 加固，harness/agent 落地时只切换信任来源）；保留手输绝对路径入口，但加显式确认并回显规范化后的真实路径；范围收敛到本地文件读取这一条线；交付形式 = 本文件 + `docs/README.md` 索引
- **约束**：零新依赖（只用 `std::path` / `std::fs`）；三项 CI 门禁（`cargo test` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`）必须保持绿

## 1. 问题定义

报告原文（`docs/redteam-report-2026-10.md:315`，展开在 `:322-332`）：

> `read_local_file` 对本地文件无任何路径限制，与 `Sandbox` 的严谨校验形成鲜明反差 …… 建议让 `read_local_file` 走 `Sandbox` 校验，或至少限制在数据目录与用户显式选择的文件范围内。

本计划采纳后半句（**用户显式选择 + 显式确认 + 回显真实路径**），并说明为什么不采纳"走 Sandbox 校验"：

| 方案 | 结论 | 理由 |
|------|------|------|
| 让 `read_local_file` 直接调用 `Sandbox::read` | 否决 | `Sandbox::safe_path`（`desktop-ai/src/server/sandbox.rs:69-118`）只接受**沙盒根下的相对路径**，而"让知识库索引用户自己的文档"天然是沙盒外的任意绝对路径。强行套用只有两种结果：功能缩水成"只能索引数据目录内的文件"，或者给 `Sandbox` 开一个"任意绝对路径"的后门，把校验变成装饰。 |
| 限制在用户显式选择的文件范围内 | 采纳 | 与产品语义一致，且这正是未来 harness/agent 需要区分的那条线。 |
| 新增统一出入口 + 信任等级参数 | 采纳（本计划主体） | 把"调用方是谁"变成代码里的一等公民，使 harness/agent 落地时只需新增一个信任等级，而不是重写校验。 |

## 2. 现状：本地文件读取一共有三条路径

| # | 入口 | 调用链 | 现有校验 | 大小上限 |
|---|------|--------|----------|----------|
| A | 文件对话框 | `pick_and_index_file`（`desktop-ai/src/ui/app/mod.rs:435`）→ `start_kb_job(KbIndexJob::File)`（`:507`、`:546`）→ `desktop-ai/src/rag/kb_job.rs:64-84` → `std::fs::read_to_string` | 扩展名白名单 `txt/md/pdf`（`mod.rs:487-490`）+ 前 10 字节二进制嗅探（`:493-504`） | **无** |
| B | 手输路径 → "载入文件" | 同上；注意取消文件对话框后会静默回退到 `kb_title` 里的旧文本（`mod.rs:450-459`） | 同上 | **无** |
| C | 手输绝对路径 → "爬取" | `crawl_url_to_kb`（`ui/app/mod.rs:528`）→ `crawl_with_depth` / `crawl_url`（`rag/crawler.rs:661` / `:585`）→ `crawl_single`（`:591`）→ `is_file` 分支（`:607`）→ `read_local_file`（`rag/crawler.rs:289`） | 仅 pdf 走 `extract_pdf_safe`；无扩展名白名单 | 5 MB，但**在整文件读入之后**才判断（`crawler.rs:289` 内） |
| D | 手输相对路径 → "爬取" | `crawl_single` 的 else 分支（`rag/crawler.rs:616-623`）→ `read_local_file` | 无（`PathBuf::from(src).exists()` 即读） | 同 C |

路径 D 是报告未提及的一条：它使 `../../` 形式的相对路径也能命中文件读取，且完全不经过 A/B 的扩展名白名单。

另一处关键事实：`desktop-ai/src/server/sandbox.rs` 的 `read` / `write` / `list` / `write_bytes` / `read_bytes` / `delete` / `exists`（`sandbox.rs:127`–`:270`）**全部带 #[allow(dead_code)] 且无生产调用方**；生产代码只用到 `root_path()` 与 `list("")`（`ui/app/kb_panel.rs:227`、`:230`，纯展示）。也就是说，"与 Sandbox 的严谨校验形成鲜明反差"的实质是：**沙盒校验早就写好了，但文件读取链路根本没接上去**。

## 3. 同族问题（本次一并处理）

| 编号 | 问题 | 位置 |
|------|------|------|
| P1 | 三条读取路径的校验逻辑各写各的，没有单点 | `ui/app/mod.rs:487-504` vs `rag/crawler.rs:289` vs `rag/kb_job.rs:64-84` |
| P2 | `KbIndexJob::File` 分支完全没有大小上限（内存 DoS） | `rag/kb_job.rs:64-84` |
| P3 | 相对路径 + 目录遍历可以命中文件读取 | `rag/crawler.rs:616-623` |
| P4 | 扩展名白名单不一致：UI 只允许 `txt/md/pdf`，面板页脚却宣称支持 html | `ui/app/mod.rs:487-490` vs `ui/app/kb_panel.rs:255` |
| P5 | 取消文件对话框后静默回退到 `kb_title` 里的旧字符串，会把上一次粘贴的标题当路径再读一次 | `ui/app/mod.rs:450-459`（Android 分支 `:462-474`） |
| P6 | 5 MB 上限的判定发生在整文件读入之后，等于没有上限 | `rag/crawler.rs:289` |

## 4. 为什么必须留到 harness/agent 才能收尾

真正的分界线不是"加一句路径校验"，而是**调用方的信任等级**（用户手势 ≠ 模型输出）：

- **今天**：`read_local_file` 的 `src` 只来自 UI——用户在文件对话框里选，或在输入框里敲。能读到的文件用户本来就能读，风险是自伤，不是越权。
- **harness/agent 落地后**：同一个参数会多出"模型生成"这一来源。此时链路才完整：网页/文档内容 → prompt 注入 → 模型生成路径 → 读取任意文件 → 内容入库 → 随对话返回给模型 → 外泄。今天这条链路缺的只是"模型能决定路径"这一环。
- **结论**：现在要做的不能只是"把校验写死"，而要把信任等级做成一等公民——新增 `FileOrigin` 参数，今天的三个调用点全部传 `UserGesture`，未来 agent 工具传 `AgentTool`。这样阶段二的改动量是"新增一个分支 + 一个调用点"，而不是重写校验。这也是 §1 中"新增统一出入口"这一行被采纳的原因。

## 5. 目标模型

新增 `desktop-ai/src/rag/fs_access.rs`，并在 `desktop-ai/src/rag/mod.rs` 末尾（第 8 行 `vector_store` 之后）注册 `pub(crate) mod fs_access;`。零新依赖。

```rust
//! 本地文件读取的唯一出入口。所有读取路径必须经过这里。

/// 可索引的扩展名白名单（UI 文案与校验共用此常量）。
pub(crate) const INDEXABLE_EXTS: &[&str] = &["txt", "md", "pdf", "html", "htm"];
/// 单个待索引文件的字节上限：读之前用 metadata 判定。
pub(crate) const MAX_INDEX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// 调用方的信任等级。
pub(crate) enum FileOrigin {
    /// 用户在 UI 中通过文件对话框选择，或在输入框显式键入并确认。
    UserGesture,
    /// 阶段二：Agent 工具调用。当前无调用方。
    AgentTool,
}

pub(crate) struct IndexedFile {
    /// 与 crawl_single 现有约定一致的格式标记（实现时照抄现有取值，不要新造）。
    pub(crate) format: String,
    /// 原始内容（pdf 已抽取为文本）。
    pub(crate) raw: String,
    /// canonicalize 之后的真实路径，供 UI 回显与日志。
    pub(crate) resolved: std::path::PathBuf,
    /// 非致命提示（如"路径经过符号链接/联接"）。
    pub(crate) warnings: Vec<String>,
}

/// 用户手势 / Agent 共用的读取入口。
pub(crate) fn read_for_index(path: &str, origin: FileOrigin) -> Result<IndexedFile, String>;

/// 仅 Agent 分支使用：沙盒内相对路径读取。
pub(crate) fn read_for_agent(sandbox: &Sandbox, rel: &str) -> Result<String, String>;
```

`FileOrigin` 的规则表：

| 规则 | `UserGesture` | `AgentTool`（阶段二） |
|------|---------------------|------------------------------|
| scheme | 允许空 scheme 与 `file:`；出现 `http:` / `https:` 以外的其它 scheme（`ftp:`、`smb:`、`data:` …）一律拒绝 | 不适用（只接受沙盒内相对路径） |
| UNC / 设备命名空间 | **对 canonicalize 之前的原始字符串**判定，拒绝 `\\` 与 `//` 开头，以及 `\\?\` 与 `\\.\` 前缀 | 由 `Sandbox::safe_path` 负责 |
| 相对路径 | 拒绝，必须绝对路径 | 必须相对路径 |
| 存在性与类型 | `canonicalize` 必须成功；`metadata().is_file()` 必须为真 | 同左（`Sandbox::read` 已有） |
| 扩展名 | 以 **canonicalize 之后的真实目标**为准，转小写后必须命中 `INDEXABLE_EXTS` | 无扩展名限制（阶段二议题） |
| 大小上限 | 读之前用 `metadata().len() > MAX_INDEX_FILE_BYTES` 拒绝 | 沿用 `Sandbox`（当前 `MAX_FILE_SIZE = 500_000`，见 §11） |
| 重解析点（符号链接 / 联接） | 只**警告**并回显真实路径，不硬拒（理由见 §8.2） | 硬拒（`Sandbox` 的 canonicalize + `starts_with` 已覆盖越界） |

## 6. 阶段一实施清单

### T1 新建 `fs_access` 模块
实现 §5 的模型与规则；在 `desktop-ai/src/rag/mod.rs` 注册。
**验收**：可编译，`clippy -D warnings` 无新告警，无新依赖。

### T2 三条读取路径改道
- `desktop-ai/src/rag/crawler.rs:289`：`read_local_file` 正文替换为对 `fs_access::read_for_index(path, FileOrigin::UserGesture)` 的调用。
- `desktop-ai/src/rag/kb_job.rs:64-84`：`KbIndexJob::File` 分支改为 `fs_access::read_for_index(...)`，删除本地的 pdf / `read_to_string` 分支。
- `desktop-ai/src/rag/crawler.rs:616-623`：相对路径 else 分支改为返回 `Err`（"本地文件请使用『载入文件』入口并输入绝对路径"）。

**验收**：`grep -n "read_to_string" desktop-ai/src/rag` 只剩 `fs_access.rs` 一处。

### T3 拆分"爬网页"与"索引本地文件"两个入口（关键改动）
- `desktop-ai/src/rag/crawler.rs:585` 的 `crawl_url` 收窄为**只接受 http/https**，任何其它输入返回 `Err`。
- 抽出 `crawl_single`（`crawler.rs:591`）中 `read_local_file` 之后的公共后处理（清洗、长度截断、U+FFFD 守卫等）为 `fn page_from_raw(src: &str, format: &str, raw: &str) -> Result<CrawledPage, String>`，由 `crawl_url` 与新的 `pub(crate) fn index_local_file(path: &str) -> Result<CrawledPage, String>` 共用。**实现时先读 `crawler.rs:591-660` 确认后处理步骤的实际顺序，逐条搬移，不要凭记忆重写。**

**验收**：`crawl_url("C:\\tmp\\a.txt")` 与 `crawl_url("../../a.txt")` 均返回 `Err`；`index_local_file` 对同一文件返回与改造前等价的 `CrawledPage`。

> ⚠️ 迁移既有用例：`redteam_hostile_files_via_crawl_url_no_panic`（`rag/crawler.rs:775`）与 `redteam_pdf_bombs_no_process_kill`（`:823`）正是通过 `crawl_url` 喂本地文件，入口收窄后它们会失效，必须一并迁移到 `index_local_file`。动手前先 `grep -n "crawl_url(" desktop-ai/src` 盘点全部调用点（含测试）。

### T4 扩展名白名单单点化
`ui/app/mod.rs:487-490` 与 `ui/app/kb_panel.rs:255` 统一引用 `fs_access::INDEXABLE_EXTS`；同时决定 html 是否真的支持（推荐放开校验让文案与实现一致，否则改文案）。
**验收**：两处不再出现硬编码扩展名列表（P4 消解）。

### T5 UI 确认框
给 `ConfirmAction`（`ui/app/mod.rs:214`）增加一个携带真实路径的变体（如 `IndexExternalFile { resolved: PathBuf }`），在 `ui/app/settings.rs:312` 的 `render_confirm_dialog` 里处理：`:316-337` 的 `match` 目前硬编码成 `(title, msg, is_danger)` 静态三元组，需先让它能容纳动态内容，再在 `:349-383` 的按钮分支里加确认后的 `start_kb_job` 路径。**不要破坏其它四个变体的行为。**
**验收**：任意一次本地文件索引都会先弹确认框，并显示 `resolved` 真实路径。

### T6 去掉取消回退
`ui/app/mod.rs:450-459`（以及 Android 分支 `:462-474`）在用户取消文件对话框时直接 `return`，不再回退到 `kb_title`。手输路径改为与"粘贴标题"分离的独立输入/按钮（`ui/app/kb_panel.rs:83` 与 `:128` 目前共用 `kb_title`，语义重叠）。
**验收**：点"载入文件"后取消，不产生任何索引任务（P5 消解）。

### T7 回显真实路径
索引成功/失败时把 `IndexedFile.resolved` 与 `warnings` 显示在知识库面板状态区，让用户看到"实际读了哪个文件"。

### T8 `redteam.rs` 新增第 3 节"本地文件读取"
沿用现有风格（收集 `escaped: Vec<String>` 后 `assert!(escaped.is_empty(), ...)`；新增 `use crate::rag::fs_access::{...};`）。用例清单见 §9。
**验收**：先写用例，确认 UNC / 相对遍历 / 超限三类在改造前**会失败**（能红），再改实现转绿。

### T9 正向用例（防止过度拦截）
用已有 dev-dep `tempfile` 写入合法 `.txt` / `.md`，断言 `read_for_index` 成功且 `resolved == canonicalize(输入)`；pdf 用例可沿用 `crawler.rs:823` 的最小合法 PDF 构造方式。

### T10 文档
本文件入索引（`docs/README.md` 表格新增一行，并在"已落实的结论"里注明"遗留项 1 的改造计划见本文件，待实施"）；在 `crawler.rs` / `kb_job.rs` 相关函数上补注释指向 `fs_access`。

## 7. 阶段二：harness/agent 落地时

1. Agent 工具签名的第一个参数固定为 `&ToolCtx`（携带沙盒与信任等级），**禁止**从模型参数直接取路径读取。
2. `AgentTool` 分支只走 `read_for_agent` → `Sandbox`；任何绝对路径请求一律拒绝，要求模型给出沙盒内相对路径。
3. 文件对话框返回值改为**不透明句柄** `UserSelectedFile`（内部持有已 canonicalize 的 `PathBuf`），UI 不再传递裸 `String` 路径——这样"模型生成的字符串"在类型上就无法伪装成"用户选择"。
4. CI 增加 grep 式门禁：断言 `std::fs::read_to_string` / `File::open` / `read_local_file` 只出现在 `fs_access.rs` 的白名单内。
5. 复审 `Sandbox::MAX_FILE_SIZE = 500_000`（`server/sandbox.rs:7`）是否满足 agent 读取需求。

## 8. Windows 实现陷阱（必须遵守）

1. **`canonicalize` 的 \\?\ 前缀**：Windows 上 `std::fs::canonicalize` 返回扩展长度路径（形如 `\\?\C:\…`）。因此 UNC / 设备命名空间检查**必须在 canonicalize 之前对原始输入串做**；若对 canonicalize 的结果判定，会把每一个正常路径都误杀。反过来，`\\?\` 前缀在 `resolved` 回显给用户前应剥掉。
2. **不要对用户手势硬拒"祖先重解析点"**：OneDrive 的"已知文件夹移动"、重定向的用户目录、`C:\Users\<user>\Documents` 整体都可能是重解析点，硬拒会误伤正常使用。改为在 `warnings` 里提示"路径经过链接/联接，真实位置为 X"。真正的硬边界放在 (a) 真实目标的扩展名与类型判定，(b) 阶段二的 Agent 分支。
3. **非 UTF-8 路径**：`to_string_lossy()` 会把非法字节替换成 U+FFFD，再 canonicalize 就找不到文件。`read_for_index` 应接受 `&Path`（或对失败给出"路径包含非法字符"的明确错误），不要让 UI 的 `to_string_lossy` 结果静默退化成"文件不存在"。
4. `symlink_metadata().file_type().is_symlink()` 在 Windows 上对目录联接（junction）同样返回 true，**不需要**为 junction 单独写 `FILE_ATTRIBUTE_REPARSE_POINT` 判断。
5. **目录**（而非文件）被传入时必须报"不是普通文件"，不能落到 `read_to_string` 的 `Is a directory` 系统错误上，错误信息要可读。

## 9. 回归用例清单（`desktop-ai/src/redteam.rs` 第 3 节）

| 用例 | 输入 | 期望 |
|------|------|------|
| UNC 反斜杠 | `\\server\share\a.txt` | `Err` |
| UNC 正斜杠 | `//server/share/a.txt` | `Err` |
| 设备命名空间 | `\\?\C:\Windows\win.ini`、`\\.\NUL` | `Err` |
| 相对路径 | `..\..\secret.txt`、`secret.txt` | `Err` |
| `file:` 相对 / 遍历 | `file://../../a.txt` | `Err` |
| 其它 scheme | `ftp://host/a.txt`、`data:text/plain,x` | `Err` |
| 白名单外扩展名 | `…\id_rsa`、`a.txt.exe`、无扩展名 | `Err` |
| 真实目标扩展名 | `link.txt` 指向 `id_rsa` | `Err`（按 canonicalize 后的真实目标判定） |
| 超限 | `MAX_INDEX_FILE_BYTES + 1` 字节的 `.txt` | `Err`，且未整文件读入 |
| 目录 | 指向目录的路径 | `Err`，且错误信息可读 |
| 入口一致性 | 上述任一 payload 分别经 `index_local_file` 与 `KbIndexJob::File` 分支 | 均 `Err`，错误语义一致 |
| 爬取入口收窄 | `crawl_url("C:\\tmp\\a.txt")`、`crawl_url("../../a.txt")` | `Err` |
| 正向：合法文本 | tempfile 写入 `.txt` / `.md` | `Ok`，`resolved == canonicalize(输入)` |
| 正向：合法 PDF | 最小合法 PDF | `Ok`，`format` 与改造前一致 |
| 链接警告（不硬拒） | 指向白名单内真实文件的符号链接 | `Ok` 且 `warnings` 非空 |
| 沙盒回归 | `redteam_sandbox_blocks_path_traversal`（`redteam.rs:125`） | 继续通过 |

## 10. 验收标准与命令

```powershell
cd E:\AI可视化\桌面AI
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

手测清单：

1. 文件对话框选 `%USERPROFILE%\.ssh\id_rsa` → 必须被拒（扩展名）。
2. 选任意正常 `.md` → 弹确认框并显示 canonical 真实路径 → 确认后入库成功。
3. 点"载入文件"后取消 → 无任何索引任务（改造前这里会读 `kb_title` 里的旧文本）。
4. 在"爬取"框里输入 `C:\Windows\win.ini` → 报错并提示改用"载入文件"。
5. 在"文件路径"框输入 `..\..\secret.txt` → 报错。
6. 造一个 6 MB 的 `.txt` → 报错，且进程内存不出现尖峰。

Definition of Done：

- [ ] 全仓库只有 `fs_access.rs` 直接调用 `std::fs::read_to_string` / `File::open`（爬虫的 HTTP 部分除外）
- [ ] `FileOrigin` 已存在于三个调用点，且都传 `UserGesture`
- [ ] `crawl_url` 不再接受非 http(s) 输入，既有红队用例已迁移
- [ ] `redteam.rs` 第 3 节用例正反向都覆盖
- [ ] 三项 CI 门禁绿
- [ ] 零新依赖

## 11. 风险与取舍

| 项 | 取舍 |
|------|------|
| 行为变更：本地路径不能再从"爬取"入口进入 | **有意为之**。这是把两条信任链路分开的前提。UI 文案改为"仅支持 http/https"，错误信息指向"载入文件"。 |
| 行为变更：`KbIndexJob::File` 新增 5 MB 上限 | **有意为之**（内存 DoS）。此前能索引的超大 txt/md 会被拒。后续如需支持大文件，应走分块流式读取，另开任务（§12）。 |
| 白名单新增 `html/htm` | 需要产品决定：若面板确实宣称支持 html，就让校验与文案一致（推荐放开）；否则改文案。 |
| 确认框多一次点击 | 用户已确认采纳（"保留手输路径但加显式确认"）。 |
| `Sandbox::MAX_FILE_SIZE = 500_000` 对 agent 偏小 | 阶段二议题，本阶段不改，避免同时动两处语义。 |

## 12. 不在本计划范围

- 报告 §6 的其它遗留项：2（FTS5 命中 score 恒 1.0）、3（`/ready` 未认证返回模型名）、4（`examples/api_serve.rs` 默认弱 token `"da-local"`）、5/6（`target/`、`android/build/` 磁盘占用）。
- 大文件的分块 / 流式索引。
- 把 `Sandbox` 从 `server/` 迁到独立的访问层模块（本计划只在 `fs_access` 里引用它）。
- URL 抓取侧的 SSRF 加固——已在上一轮修复，见 `docs/redteam-report-2026-10.md`。
