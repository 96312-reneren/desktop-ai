# 桌面AI v5.3 — RAG增强版

> 双击运行，本地 AI 对话。**~6.5 MB 双文件分发，毫秒启动。**

## v5.3 新功能: 搜索增强RAG

开启「搜索引擎」后，每次发送消息时自动调用 DuckDuckGo 搜索用户问题，将
搜索结果注入到 LLM 的上下文 prompt 中，实现本地 RAG (检索增强生成)。

**工作原理:**
1. 用户发送消息
2. 自动搜索 DuckDuckGo (不阻塞 UI)
3. 搜索结果格式化为系统提示注入 prompt
4. LLM 基于实时搜索结果回答问题

**状态栏显示 "RAG" 标识表示搜索增强已启用。**

## 快速开始

1. 确保 `桌面AI.exe` 和 `llama.dll` 在同一目录
2. 双击 `桌面AI.exe`
3. 设置中开启「搜索引擎」
4. 选择模型 → 对话

## 6 款可选模型

| 模型 | 大小 | 内存 | 适用 |
|------|------|------|------|
| Qwen2.5-0.5B | 0.35 GB | 2 GB | 超低配/老旧设备 |
| Qwen3-1.7B | 1.2 GB | 4 GB | 低配电脑首选 |
| Qwen2.5-3B | 2.0 GB | 6 GB | 日常对话 |
| Qwen2.5-7B | 4.7 GB | 8 GB | 综合能力 |
| Qwen2.5-Coder-7B | 4.7 GB | 8 GB | 编程专用 |
| Qwen3-8B | 5.5 GB | 10 GB | 最新最强 |

## 系统要求

- Windows 10/11 64-bit
- VC++ Redistributable

## 技术栈

Rust 1.96 + egui/eframe + llama.cpp FFI + pulldown-cmark + reqwest + DuckDuckGo
