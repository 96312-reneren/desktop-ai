# 桌面AI

本地大模型聊天应用 — 纯 Rust 实现，12 MB 绿色免安装。

→ [项目源码与文档](./desktop-ai/)

## 分支结构

| 分支 | 内容 |
| ---- | ---- |
| `main` | 桌面版（Windows / Linux）：egui 界面 + llama.cpp 推理 + RAG 知识库 |
| `android` | 安卓混合架构版：Kotlin Activity + Rust .so（JNI）+ WebView 聊天界面 |

安卓版从 `main` 分出，包含：

- `android/` — 零 Gradle 的 APK 构建链（kotlinc + d8 + aapt2 + apksigner）
- `desktop-ai/src/android_service.rs` — Rust 服务端 JNI 入口（模型加载 + OpenAI 兼容 API server）
- WebView 聊天 UI（会话管理、深色主题、Markdown 渲染）
