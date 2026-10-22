//! 对外服务领域：OpenAI 兼容 API 服务器与文件沙盒。
//!
//! - `api_server`：仅绑定 127.0.0.1 的本地 `OpenAI` 兼容 HTTP 服务器。
//! - `sandbox`：把 AI 的文件读写限制在受控根目录内的本地沙盒。

pub(crate) mod api_server;
pub(crate) mod sandbox;
