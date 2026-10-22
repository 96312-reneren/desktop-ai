//! LLM 领域模块：封装 llama.cpp 的 FFI 绑定、推理、向量化、模型目录与模型下载。
//!
//! 对外：被上层 UI 与 RAG 检索流程调用；向下经 `ffi` 子模块访问 llama.cpp 的 C ABI。
//! 子模块分工：
//! - `ffi`：llama.cpp 原始绑定的安全包装（加载模型、建上下文、解码、取 embedding）；
//! - `inference`：文本生成（流式 token 推理）；
//! - `embedding`：语义向量化，供知识库相似度检索使用；
//! - `model_catalog` / `downloader`：模型清单与下载（含分卷合并与 SHA-256 校验）。

pub(crate) mod downloader;
pub(crate) mod embedding;
pub(crate) mod ffi;
pub(crate) mod inference;
pub(crate) mod model_catalog;
