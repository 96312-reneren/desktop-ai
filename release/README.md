# 桌面AI v3.0 — Rust 重写版

> 双击运行，本地 AI 对话。**~10 MB 单文件分发，毫秒启动。**

## v3 Rust vs v2 Python

| | Python v2 | **Rust v3** |
|---|---|---|
| 体积 | 70 MB | **~10 MB** |
| 启动 | 1-3 秒 | **毫秒** |
| UI | tkinter，GIL 限制 | **egui，60fps 原生渲染** |
| 内存 | +80MB Python 运行时 | **零额外开销** |
| 分发 | 需 70MB 文件夹 | **exe+dll 两个文件** |

## 快速开始

1. 确保 `桌面AI.exe` 和 `llama.dll` 在同一目录
2. 双击 `桌面AI.exe`
3. 选择模型 → 下载 → 自动加载 → 对话

## 5 款可选模型

| 模型 | 大小 | 内存 | 适用 |
|------|------|------|------|
| Qwen3-1.7B | 1.2 GB | 4 GB | 低配电脑首选 |
| Qwen2.5-3B | 2.0 GB | 6 GB | 日常对话 |
| Qwen2.5-7B | 4.7 GB | 8 GB | 综合能力 |
| Qwen2.5-Coder-7B | 4.7 GB | 8 GB | 编程专用 |
| Qwen3-8B | 5.5 GB | 10 GB | 最新最强 |

## 文件说明

- `桌面AI.exe` → 主程序（7.5 MB）
- `llama.dll` → 推理引擎（2.5 MB，必须同目录）
- 模型下载到 `%APPDATA%\DesktopAI\models\`
- 对话保存到 `%APPDATA%\DesktopAI\conversations\`

## 系统要求

- Windows 10/11 64-bit
- VC++ Redistributable (https://aka.ms/vs/17/release/vc_redist.x64.exe)

## 技术栈

Rust 1.96 + egui/eframe + llama.cpp FFI + pulldown-cmark + reqwest
