# 桌面AI

**纯 Rust 本地大模型聊天应用** — 无需 Python / Node / CUDA / Docker，双击即可运行，支持 Windows 与 Linux。

零门槛中文 AI 桌面工具：选模型 → 对话，全程 GUI，支持 RAG 知识库 / 网络搜索 / OpenAI 兼容 API。

> **开源许可：** 本应用代码采用 [MIT 许可证](./LICENSE)。llama.cpp 使用 MIT 许可。所支持的千问模型使用 Apache 2.0 许可（由阿里云/通义提供）。

---

## 当前版本：v6.1.0

### 区别于上一版（v5.8）的关键变更
- **SQLite 存储迁移**：知识库与对话从 JSON 迁移至 SQLite（WAL 事务、崩溃不丢数据、增量写入），旧数据首次启动自动迁移
- **会话增量落盘**：只写入新增消息，长对话不再全量重写
- **模型下载断点续传**：中断后自动续传，无需重新下载
- **日志文件**：每日滚动日志写入数据目录，崩溃后可排查
- **API token 强随机化**：getrandom 密码学随机，替代时间戳方案
- **新版 llama.cpp API 适配**：采样器 / vocab / 后端初始化全适配，修复模型加载崩溃与空输出问题
- **ChatML 注入消毒扩展**：13 个特殊标记全覆盖，对话历史统一消毒
- **SSRF 防护补强**：种子 URL 校验、重定向逐跳校验、IP 编码绕过拦截
- **API 防护**：请求总超时防慢速滴流、CORS 白名单回显、路由查询串兼容
- **config 原子写入** + tokenize 缓冲扩容 + decode 错误可见
- **跨平台**：正式支持 Linux（x86_64，glibc ≥ 2.35），GitHub Actions 自动编译双平台产物并发布 Release
- **依赖安全升级**：pdf-extract 0.12（lopdf 0.42，修复 RUSTSEC-2026-0187）、anyhow、event-listener、memmap2 等

完整功能见下方[功能清单](#核心功能)。

---

## 核心功能

### 推理
- **多模型选择**：6 款 Qwen GGUF 模型（0.5B / 1.7B / 3B / 7B / 8B / Coder-7B），覆盖 2 GB - 16 GB 内存设备
- **流式生成**：token-by-token 实时显示，可中断停止
- **GPU 加速**：通过 `n_gpu_layers` 支持 CUDA / Vulkan，自动 GPU 检测
- **新版 llama.cpp 兼容**：运行时自动检测库的 API 时代（现代 `llama_sampler_*` / 旧版采样）

### RAG 三位一体
- **本地知识库 KB**：基于同一模型的 embedding 上下文做语义检索，SQLite 存储（WAL 事务，增量写入，自动迁移旧 JSON）
- **网络搜索**：DuckDuckGo HTML 抓取
- **网页爬虫**：深度 1-3，SSRF 防护拦截私网 / 回环地址（含 IP 替代编码、逐跳重定向校验）
- **统一 RAG 提示词**：`build_rag_prompt()` 将 KB 与搜索上下文注入系统提示词，并对全部 ChatML 特殊标记做注入消毒

### API 服务
- **OpenAI 兼容**：`/v1/chat/completions`、`/v1/models`、`/health`、`/ready`
- **Bearer Token 认证**：`/v1/*` 需携带 `Authorization: Bearer <token>`（设置中查看）
- **CORS 白名单**：仅允许 localhost / 127.0.0.1，响应回显白名单 Origin
- **DoS 防护**：最大 16 并发、30 秒读超时 + 60 秒请求总超时、1 MB 请求体上限

### 对话管理
- 多轮对话持久化（SQLite，增量落盘）
- 实时搜索过滤（Ctrl+F）
- **JSON 导入 / 导出**（备份与迁移）
- id 消毒（仅允许 `[a-zA-Z0-9_]`）

### UI / UX
- 完整 Markdown 渲染（含表格 / 链接 / 内联代码高亮）
- 可选中文本支持 Ctrl+C 复制
- 快捷键：Ctrl+N 新对话 · Ctrl+F 搜索 · Ctrl+Enter 发送 · Esc 关闭弹窗
- 深色 / 浅色主题实时切换
- 危险操作二次确认（删除模型 / 重置应用 / 卸载等）
- 硬件检测自动推荐合适模型

### 安全加固
- **新版 llama.cpp FFI 全适配**：结构布局、后端初始化、vocab API、现代采样器
- **SSRF 拦截**（种子 / 子链接 / 重定向 / IP 编码）+ **ChatML 注入消毒**（13 标记全覆盖）
- **API 认证** + 慢速滴流防护 + CORS 白名单
- **依赖漏洞修复**：pdf-extract 0.12 / lopdf 0.42（RUSTSEC-2026-0187）、anyhow、event-listener、memmap2
- Release profile：`opt-level=3` + `lto=true` + `strip="symbols"`

### 工程质量
- **85 个测试**全部通过（77 unit + 8 integration），覆盖 11 个模块
- **0 个 Clippy 错误、0 个 Clippy 警告**（pristine baseline）
- 全部 unsafe 函数含 `# Safety` 文档说明
- 结构化日志：`tracing-subscriber` + `tracing-log` 桥接 `log::*!` 宏，落盘到数据目录 `logs/`
- GitHub Actions 自动编译：push 构建双平台产物，tag 自动发布 Release

---

## 快速开始

### 1. 获取应用
从 [GitHub Releases](https://github.com/96312-reneren/desktop-ai/releases) 下载对应平台压缩包，或使用仓库 `release/` 目录：

- **Windows**：`release/windows/` 中的 `桌面AI.exe` 与 `llama.dll` 放在同一文件夹，双击 `桌面AI.exe`
- **Linux**：解压 `release/linux/` 或 `desktop-ai-<版本>-linux-x86_64.tar.gz`，运行 `desktop-ai`（需 glibc ≥ 2.35）

### 2. 首次使用
1. 应用启动后，点击左上角"切换模型"
2. 选择适合您硬件的 Qwen 模型（小内存选 0.5B / 1.7B，8GB+ 可选 3B-7B）
3. 等待下载完成（支持断点续传）
4. 输入消息，Ctrl+Enter 发送

### 3. 启用 RAG（可选）
- 设置面板中开启"网络搜索"或"知识库"
- 知识库面板可添加本地文件 / 网页 URL，自动分块向量化
- 推理时 RAG 上下文自动注入系统提示词

### 4. 启用 API 服务器（可选）
- 设置面板中开启"API 服务"，默认端口 11434
- 任何 OpenAI 客户端可指向 `http://127.0.0.1:11434/v1`，需携带 API token

---

## 项目结构

```
桌面AI/
├── desktop-ai/                    # Rust 源码
│   ├── src/
│   │   ├── main.rs                # 入口 + 字体加载 + 日志初始化
│   │   ├── config.rs              # 配置序列化 + 边界校验 + 原子写入
│   │   ├── db.rs                  # SQLite 连接封装（Mutex + WAL + busy_timeout）
│   │   ├── conversation.rs        # 对话 CRUD（SQLite 增量落盘）+ 导入导出
│   │   ├── ffi.rs                 # llama.cpp C FFI 绑定（unsafe，双 API 时代兼容）
│   │   ├── inference.rs           # 推理 + ChatML 消毒 + RAG 提示词
│   │   ├── embedding.rs           # 文本向量化
│   │   ├── vector_store.rs        # SQLite 向量存储 + 余弦检索 + JSON 迁移
│   │   ├── chunker.rs             # 句子感知分块器
│   │   ├── cleaner.rs             # HTML → 纯文本清洗
│   │   ├── crawler.rs             # 网页爬虫 + SSRF 防护
│   │   ├── search.rs              # DuckDuckGo 搜索
│   │   ├── api_server.rs          # OpenAI 兼容 HTTP API（token 认证 + DoS 防护）
│   │   ├── sandbox.rs             # 沙盒文件系统
│   │   ├── markdown.rs            # Markdown → egui 渲染
│   │   ├── downloader.rs          # 模型下载 + 断点续传 + SHA-256 校验
│   │   ├── model_catalog.rs       # 6 款 Qwen 模型元数据
│   │   ├── lib.rs                 # 库根（集成测试 + re-export）
│   │   └── app/                   # 主应用（6 个子模块）
│   │       ├── mod.rs             # 结构体 + 业务逻辑 + update()
│   │       ├── sidebar.rs         # 侧边栏 + 对话列表 + 导入导出
│   │       ├── chat.rs            # 聊天区 + 消息气泡 + 输入栏防御
│   │       ├── settings.rs        # 设置：主题/字号/GPU/API/数据
│   │       ├── model_select.rs    # 模型选择窗口
│   │       └── kb_panel.rs        # 知识库 + 搜索面板
│   ├── tests/
│   │   └── integration.rs         # 集成测试（GBK/concurrent/API）
│   ├── build.rs                   # 按平台复制 llama 库到输出目录
│   ├── Cargo.toml                 # 项目配置 (v6.1.0)
│   ├── Cargo.lock                 # 依赖版本锁定
│   ├── llama.dll                  # llama.cpp 预编译 (Windows)
│   ├── libllama.so / libggml*.so  # llama.cpp 预编译 (Linux)
│   └── README.md                  # 本文件
├── release/                       # 发布产物
│   ├── windows/                   # 桌面AI.exe + llama.dll
│   ├── linux/                     # desktop-ai + libllama.so + libggml*.so.0
│   └── desktop-ai-v6.1.0-linux-x86_64.tar.gz
└── .github/workflows/             # CI + 自动编译发布
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

# 运行所有 85 个测试
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
- 无需本地编译，发布流程：`git tag vX.Y.Z && git push origin vX.Y.Z`

### 关键依赖
| crate | 用途 |
|-------|------|
| `eframe` / `egui` 0.31 | 即时模式 GUI |
| `rusqlite` 0.37 (bundled) | SQLite 存储（自带编译，无系统依赖） |
| `pulldown-cmark` 0.12 | Markdown 解析 |
| `libloading` 0.8 | llama 库动态加载 |
| `reqwest` 0.12 (blocking) | HTTP 客户端 |
| `rfd` 0.15 | 原生文件对话框 |
| `pdf-extract` 0.12 | PDF 文本提取（lopdf 0.42） |
| `sha2` 0.10 | 模型 SHA-256 校验 |
| `tracing-subscriber` 0.3 | 结构化日志（stderr + 文件） |
| `getrandom` | API token 密码学随机 |

---

## 开发历史

### v6.1 (2026-08-02) — 自动编译与安全审计版
- SQLite 存储迁移（知识库 + 对话）+ 增量落盘
- 模型下载断点续传 + 日志文件 + API token 强随机
- 新版 llama.cpp API 全适配（采样器 / vocab / 后端初始化）
- ChatML 消毒扩展、SSRF 补强、API 慢速滴流防护、config 原子写
- 依赖安全升级（pdf-extract 0.12 / lopdf 0.42 等）
- GitHub Actions 自动编译 + 自动发布 Release
- 正式支持 Linux

### v5.8 (2026-06-22/23) — 工程质量与文档版
- **app.rs 拆分**：1722 行 → 6 子模块（app/mod.rs + sidebar/chat/settings/model_select/kb_panel）
- **Clippy 全清**：0 错误、0 警告
- Markdown 渲染重写支持表格 / 链接 / 内联代码
- 对话导入导出 + Ctrl+Enter 发送
- 修复 embedding.rs usize 比较失效 bug
- 测试从 27 扩至 43 项

### v5.7 — RAG 与安全加固版
- 网页爬虫 + 清洗管道 + GPU 推理 + 沙盒 P0
- ChatML 注入消毒 + SSRF 防护

### v5.6 — 文件选择与来源引用
- rfd 文件选择器 + pdf-extract + 索引进度条

### v5.5 — RAG 流水线重构
- build_rag_prompt() 线程安全快照 + 5 步推理流水线

### v5.4 — 本地 Embedding + 向量检索
- embedding.rs, chunker.rs, vector_store.rs

### v5.3 — RAG 搜索增强
- DuckDuckGo 结果自动注入提示词

### v5.2 — 工程质量提升
- log::warn! 错误处理 + 单元测试 + 键盘快捷键 + 对话搜索过滤

### v5.1 — OpenAI 兼容 API + 搜索
- /v1/chat/completions + DuckDuckGo

### v5.0 — 安全加固与全功能设置
- CString 防护、Arc UAF 修复、路径遍历修复、DLL 校验

### v4.0 — 安全加固版
### v3.0 — Rust 原生版（egui + 6 款模型）
### v2.0 — Python CustomTkinter 版
### v1.0 — Python 初版（单模型）

---

## 法律合规与用户须知 / Legal & Compliance Notice

> **以下内容具有法律约束力。使用本软件即表示您已阅读、理解并同意遵守全部条款。**
> **The following is legally binding. By using this software you confirm that you have read, understood, and agree to comply with all terms below.**

---

### 适用法律 / Applicable Laws

本软件的用户可能分布在全球各地。您必须遵守您所在国家或地区以及软件运行所涉及的全部适用法律。特别提示以下司法管辖区的关键法律要求：

Users of this software may be located worldwide. You must comply with all applicable laws in your jurisdiction and any jurisdiction where the software operates. Key requirements include, but are not limited to:

---

###   中华人民共和国法律 / PRC Laws

使用本软件不得违反中华人民共和国法律，包括但不限于：

1. **《中华人民共和国刑法》**
   - 第 285 条（非法侵入计算机信息系统罪）
   - 第 286 条（破坏计算机信息系统罪）
   - 第 287 条（利用计算机实施犯罪的定罪处罚）

2. **《中华人民共和国治安管理处罚法》** — 禁止利用计算机信息网络从事违法活动。

3. **《中华人民共和国网络安全法》** — 禁止利用网络从事危害网络安全的活动，不得提供专门用于危害网络安全的程序、工具。

4. **《中华人民共和国数据安全法》** — 处理数据须遵循合法、正当、必要原则，不得危害国家安全、公共利益。

5. **《中华人民共和国个人信息保护法》** — 收集、处理个人信息须取得用户同意，不得非法买卖、提供或公开他人个人信息。

6. **《中华人民共和国民法典》** — 尊重他人知识产权、名誉权、隐私权等人格权；不得利用本软件生成、传播侵犯他人合法权益的内容。

**用户不得利用本软件生成、传播以下内容：**

-   危害国家安全、泄露国家秘密、颠覆国家政权、破坏国家统一
-   损害国家荣誉和利益
-   煽动民族仇恨、民族歧视，破坏民族团结
-   破坏国家宗教政策，宣扬邪教和封建迷信
-   散布谣言，扰乱社会秩序，破坏社会稳定
-   散布淫秽、色情、赌博、暴力、凶杀、恐怖或者教唆犯罪
-   侮辱或者诽谤他人，侵害他人合法权益
-   其他违反中华人民共和国法律、行政法规的内容

---

###    United States Federal Law

This software is distributed via GitHub, a U.S.-based platform. Users must also comply with United States federal law, including but not limited to:

1. **Export Administration Regulations (EAR)** — This software may be subject to U.S. export controls. You may not download, export, or re-export the software (a) into any U.S. embargoed country, or (b) to anyone on the U.S. Treasury Department's Specially Designated Nationals List or the U.S. Commerce Department's Denied Persons List.

2. **Computer Fraud and Abuse Act (CFAA)** — Prohibits unauthorized access to protected computers and using computers to obtain information without authorization.

3. **Digital Millennium Copyright Act (DMCA)** — Users must respect copyright protection. This software must not be used to circumvent technological protection measures or to distribute copyrighted works without authorization.

4. **Children's Online Privacy Protection Act (COPPA)** — If collecting information from children under 13, compliance with COPPA is required.

5. **California Consumer Privacy Act (CCPA)** — Users handling personal data of California residents must comply with applicable privacy requirements.

---

### 模型许可 / Model Licensing

本软件支持下载、加载并运行以下开源模型。**模型文件本身不在本仓库中分发**——用户首次使用时通过软件内置下载功能从第三方镜像获取。

| 模型 | 许可 | 版权方 |
|------|------|--------|
| Qwen2.5-0.5B-Instruct | Apache 2.0 | Alibaba Cloud (通义) |
| Qwen2.5-1.7B-Instruct | Apache 2.0 | Alibaba Cloud (通义) |
| Qwen2.5-3B-Instruct | Apache 2.0 | Alibaba Cloud (通义) |
| Qwen2.5-7B-Instruct | Apache 2.0 | Alibaba Cloud (通义) |
| Qwen3-8B | Apache 2.0 | Alibaba Cloud (通义) |
| Qwen2.5-Coder-7B-Instruct | Apache 2.0 | Alibaba Cloud (通义) |

根据 Apache 2.0 许可条款，您使用这些模型时必须：

- 保留原始版权声明和许可声明
- 在修改后的文件中标注修改内容
- 不得使用商标或商业外观暗示与通义千问官方存在关联

模型通过第三方镜像（hf-mirror.com）下载。我们不对第三方镜像的可用性、完整性或许可合规性作任何保证。建议用户在必要时从 HuggingFace 官方源（huggingface.co）直接获取模型文件。

**免责声明：** 本软件仅提供模型加载与推理的工具能力。使用何种模型以及如何使用模型生成的内容，完全由用户自行决定并承担全部法律责任。我们不审查、不存储模型的输入或输出内容。

---

### 免责声明 / Disclaimer

THIS SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED. THE AUTHORS DISCLAIM ALL LIABILITY FOR ANY DAMAGES ARISING FROM THE USE OF THIS SOFTWARE. USERS BEAR SOLE RESPONSIBILITY FOR ENSURING THEIR USE COMPLIES WITH ALL APPLICABLE LAWS AND REGULATIONS.

本软件按"现状"提供，不附带任何明示或默示的担保。作者不对因使用本软件而产生的任何损害承担责任。用户自行负责确保其使用行为符合所有适用的法律法规。

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
