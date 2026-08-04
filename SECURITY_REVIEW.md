# 桌面AI 安全审查报告

- 审查日期：2026-08-04
- 审查对象：`desktop-ai`（Rust 桌面端源码）＋ `android/build/`（安卓分支构建产物，无源码）
- 审查范围：Rust 核心模块（api_server / ffi / config / sandbox / downloader / crawler / db / vector_store / inference / conversation / model_catalog / app）、安卓 APK（manifest、dex 反编译、assets、原生库符号与字符串、签名）、CI/CD 工作流、发布产物
- 审查方法：源码审阅 + 构建产物静态分析（AXML 解析、class 反编译字符串提取、ELF 字符串/符号分析、APK 签名结构检查）

---

## 一、总体结论

桌面端 Rust 代码的整体安全水平**明显高于同类本地应用**：本地 API 服务有绑定回环地址、随机 token、常量时间比较、CORS 白名单、HTTP 解析器加固、请求体/并发上限、ChatML 注入消毒、SSRF 防护、SQL 参数化、路径遍历防护、依赖审计（cargo-audit）等完整防护链，且均有对应单元测试。

**安卓分支存在系统性认证与隐私设计缺陷**（硬编码 token、固定端口、可被任意应用劫持的本地服务），与本产品"数据不出手机"的隐私承诺直接冲突，属于需优先修复的问题。

另有一个**工程一致性问题**：当前仓库源码与已发布的 APK v6.1.4 不一致（详见 §4.4），本次对安卓的审查结论以实际产物为准。

---

## 二、问题清单（按严重程度排序）

### P0 / 高危

| # | 位置 | 问题 | 影响 |
|---|------|------|------|
| A1 | 安卓 `MainActivity$onCreate$3`（AndroidBridge） | **硬编码 API 认证 token `Bearer desktopai`**（同时存在于 `libdesktop_ai.so` 中） | Android 上任何应用**无需任何权限**即可连接 `127.0.0.1:11434`，从 APK 提取硬编码 token 后即可完全冒充合法客户端：读写对话上下文、消耗设备算力。认证形同虚设。 |
| A2 | `chat.html` + 安卓启动逻辑 | **固定端口 11434（Ollama 默认端口），无端口占用/冲突检测** | 恶意应用可抢先绑定 `127.0.0.1:11434`，WebView 的全部对话内容将流向恶意应用（**对话隐私泄露**），并可伪造 AI 响应实施诱导。直接破坏"完全离线、数据不出手机"的承诺。 |

### P1 / 中危

| # | 位置 | 问题 | 影响 |
|---|------|------|------|
| A3 | `AndroidBridge.postJson(url, body, tag)` | **桥接方法允许任意 URL**，JS 传入即 `HttpURLConnection.openConnection(url)` | 一旦 chat.html 存在任何可执行 JS 的路径（XSS、注入内容、降级页面），可借桥发起 SSRF 与任意数据外传。桥应限定 URL 白名单（仅 `http://127.0.0.1:<port>/...`）。 |
| A4 | `MainActivity.onCreate` | `setJavaScriptEnabled(true)` + `setAllowFileAccess` + 加载 `file:///android_asset/chat.html` | file:// 页面在 WebView 中可读取同源 file:// 资源（其他本地文件），若叠加 JS 注入，可读取设备本地文件并外传。建议 `setAllowFileAccess(false)`（资产页不需要跨文件访问），并使用 `WebViewAssetLoader`。 |
| A5 | `AndroidManifest.xml` → application | `android:allowBackup="true"` | adb backup / 云备份可提取全部应用数据（对话记录以明文存储）。建议 `allowBackup="false"`（或配置 `dataExtractionRules` 排除敏感数据）。 |
| A6 | `AndroidManifest.xml` → application | `android:usesCleartextTraffic="true"` | 全局允许明文 HTTP。本地回环 API 确需明文，但应改用 `networkSecurityConfig` 仅放行 `127.0.0.1`，而非全局开关。 |
| A7 | APK 签名 | **仅 v1 (JAR) 签名，无 APK Signature Scheme v2/v3**（未发现 `APK Sig Block 42`） | v1 签名存在已知弱点：条目可增删/重排后重签，易受降级与篡改攻击。建议 `apksigner` 同时输出 v2（Android 7.0+ 全部支持）。 |
| A8 | `android/build/desktopai.keystore` | **签名密钥与构建产物同目录存放**（虽已加入 .gitignore，但构建目录整体可被复制/分发） | 密钥一旦外泄可伪造签名版本。建议密钥移出仓库/构建目录，存入受控的密码管理器或 CI 密钥库；确认密钥口令强度。 |
| D1 | `api_server.rs` `origin_allowed()` | `Origin: null` 一律放行 | 该放行是为安卓 file:// 页面妥协，但桌面端同样生效：无 Origin / sandboxed 上下文请求可绕过 CORS 白名单。`/v1/*` 仍有 token 保护，风险可控；建议按平台收紧（安卓保留，桌面拒绝）。 |

### P2 / 低危

| # | 位置 | 问题 |
|---|------|------|
| A9 | `AndroidManifest.xml` → activity | `exported="true"` 且带 MAIN/LAUNCHER（常规配置，风险低）；无 `android:usesCleartextTraffic` 之外的敏感能力 |
| A10 | `chat.html` | 对话记录明文存 `localStorage`（`desktopai.convs.v1`），无加密；备份/取证可直接读取 |
| D2 | `release/` 目录 | **构建产物（exe、so、zip、tar.gz）已提交至 git**：仓库膨胀、二进制内含本地路径等元数据，且难以审计供应链；建议发布产物改用 Git Releases |
| D3 | API 服务 | 无 TLS（回环可接受）、OPTIONS 预检不认证（符合规范）、`/health`/`/ready` 无认证（有意的存活探测） |

### 已确认的正面防护（未发现问题）

- API server 仅绑定 `127.0.0.1`，`/v1/*` 强制 Bearer token，token 由 `getrandom` 生成 16 字节随机值并持久化于 0600 权限的 config 文件（Unix）
- 常量时间 token 比较、CORS 白名单含 userinfo 绕过防护（`http://localhost:evil@attacker.com` 被拒）与子域伪装防护
- HTTP 解析器加固：方法白名单、HTTP 版本校验、URI/头数量/头值/头区总长度上限、重复 `Content-Length` 拒绝、控制字符/非法头名拒绝、慢速连接 30s 读超时 + 60s 总时限
- 并发连接上限 16（503 拒绝）、请求体 1 MiB 上限、消息数 ≤64、单条内容 ≤64KB、输出截断
- ChatML 控制 token 消毒（`<|im_start|>` 等全集合）+ RAG 历史统一消毒
- 爬虫 SSRF 防护：种子 URL 校验（拒绝 localhost/私有/回环/IP 编码绕过，如 `2130706433`）+ 重定向逐跳校验
- SQL 全参数化（`?1` 占位符）；sandbox 路径遍历防护（拒绝 `..` + canonicalize + 组件级 starts_with）；下载路径校验 + SHA-256 校验 + 证书校验（`danger_accept_invalid_certs(false)`）
- 模型目录 URL 全为 HTTPS；FFI 层 DLL 完整性检查（大小 + 符号探测 + SHA-256 审计日志）
- CI 含 `cargo audit`（明确豁免说明）、rustfmt/clippy 检查

---

## 三、安卓分支专项

### 3.1 审查对象

仓库中**不存在安卓 Kotlin/Java 源码**，仅存在构建产物：

```
android/build/
├── DesktopAI-v6.1.4.apk          # 发布 APK（v1 签名）
├── base.apk / aligned.apk        # 中间产物
├── desktopai.keystore            # 签名密钥（PKCS12，2574B）
├── classes.dex / classes/        # 可反编译的 dex/class
└── lib/arm64-v8a/*.so            # libdesktop_ai + llama/ggml 原生库
```

本次通过反编译 class、解析 AXML manifest、分析 ELF 符号与字符串完成了等效审查。

### 3.2 应用结构

- `MainActivity`：加载 `file:///android_asset/chat.html`，启用 JS/DOM Storage/FileAccess，注册 `AndroidBridge` JS 桥，`System.loadLibrary("desktop_ai")` 后调用 `startRust(internalDir, externalDir, port)`
- `AndroidBridge.postJson`：在单线程 executor 上向 JS 传入的 URL 发 `POST`（`Content-Type: application/json`、`Authorization: Bearer desktopai`），响应经 `window.dispatchEvent(new CustomEvent('aiResp', ...))` 回传
- manifest：`minSdk 24 / targetSdk 34`，仅 `INTERNET` 权限（良好）；`allowBackup=true`、`usesCleartextTraffic=true`、MainActivity `exported=true`
- Rust 侧：API server 绑定 `127.0.0.1`，`/v1/*` 需 Bearer；模型存 `getExternalFilesDir` 下的 `DesktopAI/models`；数据根为应用私有 `HOME`

### 3.3 安卓攻击面推演

1. **本地端口嗅探/抢占**（A2）：任意应用可 `bind(127.0.0.1:11434)`，截获 WebView 全部对话 → 隐私泄露；或先启动监听冒充服务。
2. **认证绕过**（A1）：token 硬编码于 APK（`Bearer desktopai`），攻击应用直接携带即可调用 `/v1/chat/completions` 与 `/v1/models`，窃取上下文、消耗算力。
3. **桥滥用**（A3/A4）：若 chat.html 被注入 JS（如经伪造服务的响应或未来功能引入不可信内容），`postJson` 可向任意地址外发数据（包括读取到的本地文件内容）。

---

## 四、工程与流程问题

### 4.1 源码-产物不一致（重要）

- `lib.rs` 声明 `#[cfg(target_os = "android")] pub mod android_service;`，但 `src/android_service.rs` **不存在**；`main.rs` 注释声称 `android_main in lib.rs`，实际不存在；JNI 导出（`Java_com_desktopai_android_MainActivity_startRust`）在源码中无对应实现。
- APK 的 `.so` 使用硬编码 `desktopai` token，而当前源码的 API server 使用 config 中的随机 token —— 两者行为不同。
- 结论：**当前仓库源码无法重现 APK v6.1.4 的安卓构建**，安卓源码可能在分支/历史中或已丢失。建议立即归档安卓构建源（Kotlin + Rust 安卓分支）并恢复可复现构建（补全 `android_service.rs`、`android_main`、JNI 层、构建脚本）。

### 4.2 发布流程

- `release.yml` 仅构建 Windows/Linux；安卓为本地零 Gradle 管道手动构建，无 CI 可追溯性。
- `release/` 二进制产物入库（见 D2）。

### 4.3 供应链

- llama.cpp b7700 由 CI 从 GitHub 克隆构建（可复现）；模型文件下载均校验 SHA-256 且为 HTTPS（良好）。
- `Cargo.lock` 已提交（良好）；cargo-audit 有 2 条豁免并有说明（quick-xml，影响面为 build-time/本地 D-Bus 解析，可接受）。

---

## 五、修复建议（按优先级）

| 优先级 | 事项 |
|--------|------|
| P0 | 1) 安卓：改为启动时随机生成 API token（复用现有 `default_api_token()`），通过 bridge 参数传入 JS；移除硬编码 `desktopai`。2) 安卓：端口动态分配（`TcpListener::bind("127.0.0.1:0")` 取实际端口）并回传 WebView，或至少检测 11434 被占时改用随机端口。 |
| P1 | 3) `postJson` 桥增加 URL 白名单校验（仅允许 `http://127.0.0.1:<port>/v1/*`）。4) `setAllowFileAccess(false)` + `WebViewAssetLoader`。5) `allowBackup=false` 或 `dataExtractionRules`。6) 用 `networkSecurityConfig` 收窄明文放行至回环地址。7) APK 启用 v2 签名；keystore 移出构建目录。8) `origin_allowed` 的 `null` 放行按平台收紧。 |
| P2 | 9) 对话数据落盘加密或至少混淆（安卓 WebView localStorage）。10) 停止提交 `release/` 产物，改用 GitHub Releases。11) 补全并归档安卓源码，恢复可复现构建；后续版本安全审查以源码为准。 |

---

## 六、审查结论

桌面端（Rust）安全工程成熟度良好，核心防护完备且测试覆盖到位；**安卓分支存在 2 项高危（认证 token 硬编码、固定端口可劫持）与多项中危（桥 URL 无白名单、file 访问、备份/明文流量、v1 签名）问题**，直接威胁用户对话隐私，应作为下个版本的首要修复目标。同时建议尽快恢复安卓构建源的可追溯性，消除源码-产物不一致带来的供应链与维护风险。

*注：本报告基于 2026-08-04 工作区代码与构建产物。由于安卓无源码，A1-A10 的结论来自产物静态分析；修复后建议对安卓分支做一次基于源码的复查。*
