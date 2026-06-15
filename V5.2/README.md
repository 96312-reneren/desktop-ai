# 桌面AI v5.2 — 工程品质版

> 双击运行，本地 AI 对话。**~6.5 MB 双文件分发，毫秒启动。**

## 版本历史

```
v5.2  ← 当前 (工程品质改进)
v5.1  API服务 + DuckDuckGo搜索集成
v5.0  安全加固 + 全功能设置 + UI优化
v4.0  安全性加固版
v3.0  Rust 原生版
v2.0  Python 优化版
v1.0  Python 初版
```

## 核心功能

### 本地 AI 聊天
- 6 款 Qwen 模型 (0.35GB ~ 5.5GB)，覆盖低配到高性能设备
- 流式生成、多轮对话、对话管理
- 硬件检测 + 智能模型推荐

### API 服务
- 本地 HTTP 服务器，OpenAI 兼容格式
- 端点: `/v1/models`, `/v1/chat/completions`
- 可接入 Chatbox、Open WebUI 等任意前端
- 默认端口 11434，设置中可开关

### 搜索集成
- DuckDuckGo 网页搜索，无需 API Key
- 侧边栏搜索面板
- Ctrl+F 快捷键

### 设置面板
- 主题实时切换 (深色/浅色)
- 字号、上下文长度、CPU 线程数
- 系统提示词编辑器
- 已下载模型管理 (列表 + 单独删除)
- 数据管理: 批量删除 / 重置 / 卸载

### 安全加固
- CString 空字节防护
- Arc 生命周期管理 (UAF 修复)
- 路径遍历防护
- 下载完整性校验
- DLL 校验
- 配置边界验证
- Release: LTO + strip + panic=abort

### 工程品质
- 6 项单元测试
- 错误日志 (log::warn)
- 键盘快捷键 (Ctrl+N/F, Escape)
- 对话搜索过滤
- 零编译警告

## 文件说明

```
V5.2/
├── README.md           # 本文件
├── release/            # 可执行文件
│   ├── 桌面AI.exe      # 主程序 (6.5 MB)
│   ├── llama.dll       # 推理引擎 (2.5 MB)
│   └── README.md       # 用户指南
├── src/                # Rust 源码
│   ├── main.rs         # 入口
│   ├── app.rs          # UI + 状态管理
│   ├── config.rs       # 配置序列化
│   ├── conversation.rs # 对话管理
│   ├── downloader.rs   # 模型下载
│   ├── ffi.rs          # llama.cpp FFI
│   ├── inference.rs    # 推理封装
│   ├── markdown.rs     # Markdown 渲染
│   ├── model_catalog.rs# 模型目录
│   ├── api_server.rs   # HTTP API 服务
│   └── search.rs       # DuckDuckGo 搜索
├── Cargo.toml          # Rust 项目配置
├── Cargo.lock          # 依赖锁定
├── build.rs            # 构建脚本
└── .gitignore
```

## 6 款可选模型

| 模型 | 大小 | 内存 | 适用 |
|------|------|------|------|
| Qwen2.5-0.5B | 0.35 GB | 2 GB | 超低配/老旧设备 |
| Qwen3-1.7B | 1.2 GB | 4 GB | 低配电脑首选 |
| Qwen2.5-3B | 2.0 GB | 6 GB | 日常对话 |
| Qwen2.5-7B | 4.7 GB | 8 GB | 综合能力 |
| Qwen2.5-Coder-7B | 4.7 GB | 8 GB | 编程专用 |
| Qwen3-8B | 5.5 GB | 10 GB | 最新最强 |

## 快速开始

1. 确保 `桌面AI.exe` 和 `llama.dll` 在同一目录
2. 双击 `桌面AI.exe`
3. 选择模型 → 下载 → 自动加载 → 对话

## 系统要求

- Windows 10/11 64-bit
- VC++ Redistributable

## 构建

```bash
cargo build --release
# 输出: target/release/desktop-ai.exe (~6.5 MB)
```

## 技术栈

Rust 1.96 + egui/eframe + llama.cpp FFI + pulldown-cmark + reqwest
