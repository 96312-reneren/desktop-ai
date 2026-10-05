# 桌面AI

本地大模型聊天应用 — 纯 Rust 实现，绿色免安装。

→ [项目源码与文档](./desktop-ai/)

## 仓库内容（单一 main 分支）

| 目录 | 内容 |
| ---- | ---- |
| `desktop-ai/` | 桌面版（Windows / Linux）：egui 界面 + llama.cpp 推理 + RAG 知识库 |
| `android/` | 安卓混合架构版：Kotlin Activity + Rust .so（JNI）+ WebView 聊天界面 |
| `vendor/` | 第三方运行时库（llama.cpp 的 DLL），见 [vendor/README.md](./vendor/README.md) |
| `.github/` | CI 与发布工作流 |

## 安卓版结构

- `android/app/` — Kotlin 源码、WebView UI（`assets/chat.html`）、manifest
- `android/build-apk.bat` — 零 Gradle 的 APK 构建链（kotlinc + d8 + aapt2 + apksigner）
- `desktop-ai/src/platform/android.rs` — Rust 服务端 JNI 入口（模型加载 + OpenAI 兼容 API server）
