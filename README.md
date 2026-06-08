# 桌面AI — 开发历史

## v5.0 Rust 完整功能版 (当前版本)
- 语言：Rust 1.96 + egui 原生 GUI，二进制 6.2 MB
- **安全加固**：CString防护、Arc生命周期(UAF修复)、路径遍历修复、下载校验、DLL校验
- **全功能设置**：主题实时切换、CPU线程数、系统提示词、模型管理、数据清理
- 6 款模型可选、同时下载多个、断点续传、深/浅主题
- Release: LTO + strip + panic=abort

## v4.0 Rust 安全性加固版
- 基于安全审计的修复版本
- 6项CRITICAL漏洞修复

## v3.0 Rust 原生版
- 语言：Rust 1.96 + egui 原生 GUI
- 特性：6 款模型可选、同时下载多个、生成中可切对话、中文渲染、硬件检测

## v2.0 Python 优化版
- 语言：Python 3.13 + CustomTkinter，~70 MB
- 优化：streaming + 一次性 Markdown 渲染
- 多模型选择、断点续传、深色/浅色主题

## v1.0 Python 初版
- 单模型 Qwen2.5-7B，首次启动自动下载

---

## 文件结构
```
桌面AI/
├── v2_python/     # Python 版源码
├── v3_rust/       # Rust 版源码 (v5.0)
└── release/       # 最新可执行文件
    ├── 桌面AI.exe
    ├── llama.dll
    └── README.md
```
