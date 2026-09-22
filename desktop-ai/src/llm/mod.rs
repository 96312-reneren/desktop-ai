//! LLM 领域：llama.cpp FFI 绑定、推理、向量化、模型目录与模型下载。

pub(crate) mod downloader;
pub(crate) mod embedding;
pub(crate) mod ffi;
pub(crate) mod inference;
pub(crate) mod model_catalog;
