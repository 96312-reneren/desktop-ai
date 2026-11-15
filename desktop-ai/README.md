# 桌面AI 桌面版

**纯 Rust 本地大模型聊天应用** — 无需 Python / Node / CUDA / Docker，双击即可运行，支持 Windows 与 Linux。

零门槛中文 AI 桌面工具：选模型 → 对话，全程 GUI，支持 RAG 知识库 / 联网搜索 / OpenAI 兼容 API。

> 开源许可：应用代码 [MIT](./LICENSE)；llama.cpp 为 MIT；Qwen 模型为 Apache 2.0（阿里云/通义）。

---

## 核心功能

### 推理

- **多模型选择**：6 款 Qwen GGUF 模型（0.5B / 1.7B / 3B / 7B / 8B / Coder-7B），覆盖 2 GB - 16 GB 内存设备，硬件检测自动推荐
- **流式生成**：token-by-token 实时显示，可中断停止
- **GPU 加速**：`n_gpu_layers` 支持 CUDA / Vulkan，自动 GPU 检测
- **新版 llama.cpp 兼容**：运行时自动检测库的 API 时代（现代 `llama_sampler_*` / 旧版采样）

### RAG 三位一体

- **本地知识库**：基于同一模型的 embedding 语义检索，SQLite 存储（WAL 事务、增量写入、旧 JSON 自动幂等迁移）
- **混合检索**：向量语义 + FTS5 关键词双路召回，RRF（Reciprocal Rank Fusion）融合去重，语义与专有名词两不误（可开关）
- **查询重写**：规则化指代消解——"它怎么样？"这类短问题自动结合上一轮提问检索，只影响检索、不改你的原文（可开关）
- **来源元数据**：每篇文档记录来源类型（📋 粘贴 / 📄 文件 / 🌐 抓取）与原始 URL，检索结果与引用处全程可见
- **网络搜索**：DuckDuckGo HTML 抓取
- **网页爬虫**：深度 1-3，SSRF 防护拦截私网 / 回环地址（含完整写法 IPv6、userinfo、IP 替代编码、逐跳重定向校验；响应体 3 MB 上限流式读取）
- **统一 RAG 提示词**：`build_rag_prompt_budgeted()` 按 token 预算注入 KB 与搜索上下文，超预算自动裁剪历史，并对全部 ChatML 特殊标记做注入消毒

### API 服务

- **OpenAI 兼容**：`/v1/chat/completions`、`/v1/models`、`/health`、`/ready`
- **Bearer Token 认证**：`/v1/*` 需携带 `Authorization: Bearer <token>`（设置中查看）
- **CORS 白名单**：仅允许 localhost / 127.0.0.1，响应回显白名单 Origin（带 `Vary: Origin`）
- **DoS 防护**：最大 16 并发、30 秒读超时 + 60 秒请求总超时、1 MB 请求体上限
- **无界面模式**：`cargo run --release --example api_serve -- <模型.gguf> [端口] [token]` 可脱离 GUI 启动服务

### 对话管理

- 多轮对话持久化（SQLite，增量落盘）
- 实时搜索过滤（Ctrl+F）
- **JSON 导入 / 导出**（备份与迁移）
- 会话 id 采用「时间戳 + 进程内序列号」生成，紧凑循环 / 并发创建不碰撞

### UI / UX

- 完整 Markdown 渲染（含表格 / 链接 / 内联代码高亮）
- 快捷键：Ctrl+N 新对话 · Ctrl+F 搜索 · Ctrl+Enter 发送 · Esc 关闭弹窗
- 深色 / 浅色主题实时切换
- 危险操作二次确认（删除模型 / 重置应用 / 卸载等）

### 安全加固

- **SSRF 拦截**（种子 / 子链接 / 重定向 / IP 编码 / 完整写法 IPv6 / userinfo）+ **ChatML 注入消毒**（13 标记全覆盖）
- **API 认证** + 慢速滴流防护 + CORS 白名单
- **下载完整性**：`.download` 暂存 + 原子改名 + SHA-256 + ETag/If-Range 续传校验
- **依赖漏洞修复**：pdf-extract 0.12 / lopdf 0.42（RUSTSEC-2026-0187）
- **本地数据权限**：config 文件 Unix 0600、日志目录 Unix 0700
- Release profile：`opt-level=3` + `lto=true` + `strip="symbols"`

### 工程质量

- **254 项自动化测试全部通过**（单元 + 集成，含红队回归语料与 WAL 崩溃回放）
- **0 个 Clippy 错误、0 个 Clippy 警告**
- 全部 unsafe 函数含 `# Safety` 文档说明
- 结构化日志落盘到数据目录 `logs/`，崩溃可排查
- GitHub Actions：push 构建双平台产物，tag 自动发布 Release

---

## 快速开始

### 1. 获取应用

从 [GitHub Releases](https://github.com/96312-reneren/desktop-ai/releases) 下载对应平台压缩包：

- **Windows**：`桌面AI.exe` 与 `llama.dll` 放在同一文件夹，双击 `桌面AI.exe`
- **Linux**：解压 `desktop-ai-<版本>-linux-x86_64.tar.gz`，运行 `desktop-ai`（需 glibc ≥ 2.35）

### 2. 首次使用

1. 应用启动后，点击左上角"切换模型"
2. 选择适合您硬件的 Qwen 模型（小内存选 0.5B / 1.7B，8GB+ 可选 3B-7B）
3. 等待下载完成（支持断点续传）
4. 输入消息，Ctrl+Enter 发送

### 3. 启用 RAG（可选）

- 设置面板中开启"网络搜索"或"知识库"（混合检索、查询重写默认开启，可单独关闭）
- 知识库面板可添加本地文件 / 网页 URL / 粘贴文本，自动分块向量化
- 推理时 RAG 上下文自动注入系统提示词

### 4. 启用 API 服务器（可选）

- 设置面板中开启"API 服务"，默认端口 11434
- 任何 OpenAI 客户端可指向 `http://127.0.0.1:11434/v1`，需携带 API token
- **无界面模式**（自动化 / 中转场景）：
  ```powershell
  cargo run --release --example api_serve -- <模型.gguf> [端口=11434] [token=da-local]
  ```

  服务只监听 `127.0.0.1`，退出按 Ctrl+C

---

## 项目结构

```
桌面AI/                            # 仓库根目录
├── .github/workflows/             # CI（test / clippy / fmt / audit）+ 自动发布
└── desktop-ai/                    # Rust 项目
    ├── src/
    │   ├── main.rs                # 入口 + 字体加载 + 日志初始化
    │   ├── lib.rs                 # crate facade
    │   ├── llm/                   # 推理领域
    │   │   ├── ffi.rs             # llama.cpp FFI（unsafe，运行时检测新旧 API）
    │   │   ├── inference.rs       # 推理 + ChatML 消毒 + RAG 提示词预算
    │   │   ├── embedding.rs       # 文本向量化
    │   │   ├── downloader.rs      # 下载（.download 暂存 + 续传 + SHA-256 + If-Range）
    │   │   └── model_catalog.rs   # 6 款 Qwen 模型元数据
    │   ├── rag/                   # 检索增强领域
    │   │   ├── crawler.rs         # 爬虫 + SSRF 防护 + 本地文件索引
    │   │   ├── cleaner.rs         # HTML / 文本清洗
    │   │   ├── chunker.rs         # 句子感知分块
    │   │   ├── vector_store.rs    # SQLite 向量库 + FTS + 混合检索(RRF) + 元数据
    │   │   ├── query_rewrite.rs   # 规则化查询重写（指代消解）
    │   │   ├── kb_job.rs          # 入库任务（粘贴 / 文件 / 抓取）
    │   │   └── search.rs          # DuckDuckGo 搜索
    │   ├── store/                 # 存储领域
    │   │   ├── config.rs          # 配置 + 原子写 + 数据目录 + token 生成
    │   │   ├── conversation.rs    # 对话 CRUD（增量落盘 + 导入导出）
    │   │   └── db.rs              # SQLite 封装（WAL + busy_timeout）
    │   ├── server/                # 服务领域
    │   │   ├── api_server.rs      # OpenAI 兼容 API（认证 + DoS 防护）
    │   │   └── sandbox.rs         # 沙盒文件系统
    │   ├── ui/                    # 界面领域
    │   │   ├── markdown.rs        # Markdown → egui 渲染
    │   │   ├── startup.rs         # 启动引导
    │   │   └── app/               # 主应用（mod + sidebar / chat / settings / model_select / kb_panel / handlers）
    │   └── platform/              # 平台层（Android 入口 + 桌面快捷方式）
    ├── examples/api_serve.rs      # 无界面 API 启动器
    ├── tests/integration.rs       # 集成测试（隔离数据目录）
    ├── build.rs                   # 按平台复制 llama 库到输出目录
    ├── Cargo.toml / Cargo.lock    # 全量锁版本
    ├── llama.dll                  # llama.cpp 预编译 (Windows)
    ├── libllama.so / libggml*.so  # llama.cpp 预编译 (Linux)
    ├── LICENSE                    # MIT
    └── README.md                  # 本文件
```

---

## 开发与构建

### 环境要求

- Rust stable（edition 2021）
- Windows / Linux（macOS 理论上可编译，未验证）
- Cargo 镜像推荐 TUNA：`sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/`

### 构建命令

```powershell
# 开发版（快速迭代）
cargo build

# Release 版（带 LTO + strip）
cargo build --release

# 运行全部测试
cargo test

# Clippy 静态扫描
cargo clippy
```

### Linux 额外依赖

```bash
sudo apt install build-essential cmake pkg-config patchelf libssl-dev \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libwayland-dev libgtk-3-dev libgl1-mesa-dev
```

### 自动编译发布

- 推送 `main` 分支：GitHub Actions 自动构建 Windows / Linux 产物（Actions artifact）
- 推送 `vX.Y.Z` tag：自动构建并创建 GitHub Release，附带双平台安装包与发布说明
- 发布流程：`git tag vX.Y.Z && git push origin vX.Y.Z`

### 关键依赖

| crate                       | 用途                                |
| --------------------------- | ----------------------------------- |
| `eframe` / `egui` 0.31  | 即时模式 GUI                        |
| `rusqlite` 0.37 (bundled) | SQLite 存储（自带编译，无系统依赖） |
| `pulldown-cmark` 0.12     | Markdown 解析                       |
| `libloading` 0.8          | llama 库动态加载                    |
| `reqwest` 0.12 (blocking) | HTTP 客户端                         |
| `rfd` 0.15                | 原生文件对话框                      |
| `pdf-extract` 0.12        | PDF 文本提取（lopdf 0.42）          |
| `sha2` 0.10               | 模型 SHA-256 校验                   |
| `tracing-subscriber` 0.3  | 结构化日志（stderr + 文件）         |
| `getrandom`               | API token 密码学随机                |

---

## 已知限制

- Linux 需要 glibc ≥ 2.35（Ubuntu 22.04+ 等 2022 年后发行版）
- 仅支持 GGUF 格式模型
- 单模型实例（切换需卸载）
- 无系统托盘最小化
- 无代码语法高亮（计划引入 syntect）

## 安全说明

- **PDF 解析**：v6.1 起已升级至 pdf-extract 0.12 / lopdf 0.42，修复了历史版本（0.34）的深层嵌套对象栈溢出漏洞（RUSTSEC-2026-0187）。所有解析均在本地完成，不涉及网络传输。
- **cargo audit 忽略项**：`RUSTSEC-2026-0194/0195`（quick-xml）因被 wayland-scanner（构建期工具，Windows 不运行）与 zbus_xml（系统 D-Bus 层）锁定且上游暂无修复版本而忽略，本地桌面场景无实际攻击面。

<details>
<summary><b>📜 开发历史（点击展开）</b></summary>

### v6.1.6+ (2026-09 起，未发版) — 安全审计与红队加固轮次

- SSRF：修复完整写法 IPv6 回环绕过（`[0:0:0:0:0:0:0:1]` 实测可连本机）、userinfo 解析、inet_aton 短写法；移除 `fc/fd/fe80` 前缀误杀
- 爬虫：修复恶意网页可触发的链接解析越界 panic；响应体按 3 MB 上限流式读取
- API：修复 accept 继承非阻塞导致延迟发送客户端被 400 掐断、并发上限失效；CORS 回显加 `Vary: Origin`
- 下载：`.download` 暂存 + 原子改名 + ETag/If-Range 续传校验
- 会话：id 加进程内序列号防碰撞；JSON 迁移幂等化
- 测试：新增红队回归 / WAL 崩溃回放 / 下载暂存 / token 强度等；测试数据隔离到 `target/test-data`
- RAG 升级：知识库文档来源元数据（schema v2）、混合检索（向量 + FTS5 关键词 RRF 融合）、规则化查询重写（指代消解）
- 新增无界面 API 启动器 `examples/api_serve.rs`

### v6.1 (2026-08-02) — 自动编译与安全审计版

- SQLite 存储迁移（知识库 + 对话）+ 增量落盘
- 模型下载断点续传 + 日志文件 + API token 强随机
- 新版 llama.cpp API 全适配（采样器 / vocab / 后端初始化）
- ChatML 消毒扩展、SSRF 补强、API 慢速滴流防护、config 原子写
- GitHub Actions 自动编译 + 自动发布 Release；正式支持 Linux

### v5.8 (2026-06) — 工程质量与文档版

- app.rs 拆分 1722 行 → 6 子模块；Clippy 全清；Markdown 渲染重写；测试 27 → 43 项

### v5.4 – v5.7 — RAG 成型

- 本地 Embedding + 向量检索 → RAG 流水线重构 → 网页爬虫 + GPU 推理 + SSRF 防护 → 文件选择与来源引用

### v5.0 – v5.2 — OpenAI API + 搜索 + 安全加固

- /v1/chat/completions + DuckDuckGo；CString 防护、Arc UAF 修复、路径遍历修复

### v1.0 – v4.0 — 演进

- Python 初版 → Python CustomTkinter 版 → Rust 原生版（egui + 6 款模型）→ 安全加固版

</details>
