# 桌面AI — 开发历史

## v4.0 Rust 安全性加固版 (当前版本)
- 语言：Rust 1.96 + egui 原生 GUI
- 体积：~6.2 MB (exe) + ~2.5 MB (llama.dll)
- 特性：6 款模型可选、同时下载多个、生成中可切对话、中文渲染、硬件检测
- **安全加固**：CString空字节防护、Arc生命周期管理、路径遍历修复、下载完整性验证、DLL校验、配置边界检查、Release优化

## v3.0 Rust 原生版
- 语言：Rust 1.96 + egui 原生 GUI
- 体积：~9 MB (exe) + ~3 MB (llama.dll)
- 特性：6 款模型可选、同时下载多个、生成中可切对话、中文渲染、硬件检测

## v2.0 Python 优化版
- 语言：Python 3.13 + CustomTkinter
- 体积：~70 MB
- 优化：streaming 纯文本追加，完成后一次性 Markdown 渲染
- 特性：多模型选择、断点续传、深色/浅色主题

## v1.0 Python 初版
- 单模型 Qwen2.5-7B
- 首次启动自动下载
- 基础对话功能

---

## 文件结构
```
桌面AI/
├── v2_python/     # Python 优化版源码
├── v3_rust/       # Rust 原生版源码 (v4.0)
└── release/       # 最新可执行文件
    ├── 桌面AI.exe
    ├── llama.dll
    └── README.md
```
