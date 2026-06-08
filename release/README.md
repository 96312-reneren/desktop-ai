# 桌面AI v4.0 — 安全性加固版

> 双击运行，本地 AI 对话。**~8 MB 双文件分发，毫秒启动。**

## v4 安全性更新

本版本基于安全审计报告对以下漏洞进行了修补：

| 修复项 | 严重度 | 描述 |
|--------|--------|------|
| CString空字节崩溃 | CRITICAL | 输入含`\0`不再导致应用panic崩溃 |
| 裸指针跨线程UAF | CRITICAL | 推理引擎使用Arc生命周期管理，消除use-after-free |
| 对话路径遍历 | CRITICAL | ID严格校验，防止任意文件读写 |
| 下载文件验证 | CRITICAL | 下载后验证文件大小，防止损坏/恶意文件 |
| llama.dll完整性 | HIGH | 加载前验证DLL文件完整性 |
| 配置文件校验 | MEDIUM | 反序列化后验证所有字段边界 |
| Release加固 | MEDIUM | LTO+strip+panic=abort，二进制缩小35% |

## v3 Rust vs v2 Python

| | Python v2 | **Rust v4** |
|---|---|---|
| 体积 | 70 MB | **~8 MB** |
| 启动 | 1-3 秒 | **毫秒** |
| UI | tkinter，GIL 限制 | **egui，60fps 原生渲染** |
| 内存 | +80MB Python 运行时 | **零额外开销** |
| 安全 | 无审计 | **全面安全加固** |

## 快速开始

1. 确保 `桌面AI.exe` 和 `llama.dll` 在同一目录
2. 双击 `桌面AI.exe`
3. 选择模型 → 下载 → 自动加载 → 对话

## 6 款可选模型

| 模型 | 大小 | 内存 | 适用 |
|------|------|------|------|
| Qwen2.5-0.5B | 0.35 GB | 2 GB | 超低配/老旧设备 |
| Qwen3-1.7B | 1.2 GB | 4 GB | 低配电脑首选 |
| Qwen2.5-3B | 2.0 GB | 6 GB | 日常对话 |
| Qwen2.5-7B | 4.7 GB | 8 GB | 综合能力 |
| Qwen2.5-Coder-7B | 4.7 GB | 8 GB | 编程专用 |
| Qwen3-8B | 5.5 GB | 10 GB | 最新最强 |

## 文件说明

- `桌面AI.exe` → 主程序（6.2 MB）
- `llama.dll` → 推理引擎（2.5 MB，必须同目录，加载前验证完整性）
- 模型下载到 `%APPDATA%\DesktopAI\models\`
- 对话保存到 `%APPDATA%\DesktopAI\conversations\`

## 系统要求

- Windows 10/11 64-bit
- VC++ Redistributable (https://aka.ms/vs/17/release/vc_redist.x64.exe)

## 技术栈

Rust 1.96 + egui/eframe + llama.cpp FFI + pulldown-cmark + reqwest
