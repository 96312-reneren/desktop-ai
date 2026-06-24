use crate::ffi;
use std::sync::{Arc, Mutex};

pub enum StreamToken {
    Text(String),
    Done,
    Error(String),
}

pub struct LlamaInference {
    model: *mut ffi::LlamaModel,
    ctx: *mut ffi::LlamaContext,
}

// The raw pointers are only ever touched while the external `Mutex` is held
// (see `run_inference`). `Send` is therefore sound; we intentionally do NOT
// implement `Sync` — the underlying llama.cpp context is single-threaded and
// must be serialised by the caller via `Arc<Mutex<LlamaInference>>`.
unsafe impl Send for LlamaInference {}

impl LlamaInference {
    #[allow(dead_code)]
    pub fn load(model_path: &str, n_ctx: u32, n_threads: u32) -> Result<Self, String> {
        Self::load_ex(model_path, n_ctx, n_threads, 0)
    }

    pub fn load_ex(model_path: &str, n_ctx: u32, n_threads: u32, gpu_layers: i32) -> Result<Self, String> {
        unsafe { ffi::init()?; }

        let model = if gpu_layers > 0 {
            unsafe { ffi::load_model_gpu(model_path, gpu_layers) }
        } else {
            unsafe { ffi::load_model(model_path) }
        };
        if model.is_null() { return Err("failed to load model".into()); }

        let ctx = unsafe { ffi::new_context(model, n_ctx, n_threads) };
        if ctx.is_null() {
            unsafe { ffi::free_model(model); }
            return Err("failed to create context".into());
        }

        Ok(Self { model, ctx })
    }

    pub fn model_ctx(&self) -> (*mut ffi::LlamaModel, *mut ffi::LlamaContext) {
        (self.model, self.ctx)
    }

    fn unload(&mut self) {
        if !self.ctx.is_null() {
            unsafe { ffi::free_context(self.ctx); }
            self.ctx = std::ptr::null_mut();
        }
        if !self.model.is_null() {
            unsafe { ffi::free_model(self.model); }
            self.model = std::ptr::null_mut();
        }
    }
}

impl Drop for LlamaInference {
    fn drop(&mut self) { self.unload(); }
}

/// Run streaming inference. The `Arc<Mutex<LlamaInference>>` is locked for
/// the entire generation so concurrent callers (UI chat + API requests) are
/// serialised — llama.cpp contexts are not thread-safe.
pub fn run_inference(
    inf: Arc<Mutex<LlamaInference>>,
    prompt: String,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    tx: std::sync::mpsc::Sender<StreamToken>,
    max_tokens: u32,
) {
    let inf_guard = inf.lock().unwrap();
    let (model, ctx) = inf_guard.model_ctx();
    unsafe {
        let tokens = ffi::tokenize(model, &prompt, true);
        if tokens.is_empty() {
            let _ = tx.send(StreamToken::Error("tokenization failed".into()));
            return;
        }
        let vocab = ffi::n_vocab(model);
        for chunk in tokens.chunks(512) {
            for &t in chunk { ffi::decode(ctx, t); }
        }
        let mut count = 0u32;
        loop {
            if stop_flag.load(std::sync::atomic::Ordering::Relaxed) { break; }
            let token = ffi::sample_greedy(ctx);
            if token == 1 || token == 2 || token >= vocab { break; }
            count += 1;
            if count >= max_tokens { break; }
            let piece = ffi::token_to_piece(model, token);
            if piece.is_empty() { break; }
            if tx.send(StreamToken::Text(piece)).is_err() { break; }
            ffi::decode(ctx, token);
        }
        let _ = tx.send(StreamToken::Done);
    }
}

#[allow(dead_code)]
pub fn format_chatml(messages: &[crate::conversation::Message]) -> String {
    let mut s = String::new();
    for msg in messages {
        s.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", msg.role, msg.content));
    }
    s.push_str("<|im_start|>assistant\n");
    s
}

/// Build a RAG-augmented ChatML prompt.
/// Injects kb_context and/or search_context between the system prompt and the conversation history.
pub fn build_rag_prompt(
    base_messages: &[crate::conversation::Message],
    kb_context: Option<&str>,
    search_context: Option<&str>,
) -> String {
    let mut s = String::new();

    // 1. System message (if exists)
    let has_system = base_messages.first().map(|m| m.role == "system").unwrap_or(false);

    if has_system {
        let sys = &base_messages[0];
        let mut sys_content = sys.content.clone();

        // Inject KB context into system prompt
        if let Some(kb) = kb_context {
            sys_content.push_str("\n\n---\n以下为参考文档：\n\n");
            sys_content.push_str(&sanitize_chatml(kb));
            sys_content.push_str("\n---\n请基于以上文档回答用户问题。如果文档不包含相关信息，请如实说明。");
        }

        // Inject search context into system prompt
        if let Some(search) = search_context {
            sys_content.push_str("\n\n---\n以下为网络搜索结果：\n\n");
            sys_content.push_str(&sanitize_chatml(search));
            sys_content.push_str("\n---\n请优先基于这些搜索结果回答。");
        }

        s.push_str(&format!("<|im_start|>system\n{}<|im_end|>\n", sys_content));
    }

    // 2. Remaining messages (skip system if already handled)
    let start = if has_system { 1 } else { 0 };
    for msg in &base_messages[start..] {
        s.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", msg.role, msg.content));
    }

    s.push_str("<|im_start|>assistant\n");
    s
}

/// Neutralise ChatML control tokens coming from untrusted (crawled / search)
/// content so a malicious document cannot伪造 system / assistant turns by
/// embedding `<|im_start|>` or `<|im_end|>`.
pub fn sanitize_chatml(s: &str) -> String {
    s.replace("<|im_start|>", "<| im_start |>")
     .replace("<|im_end|>", "<| im_end |>")
}
