use crate::ffi;
use std::sync::Arc;

pub enum StreamToken {
    Text(String),
    Done,
    Error(String),
}

pub struct LlamaInference {
    model: *mut ffi::LlamaModel,
    ctx: *mut ffi::LlamaContext,
}

unsafe impl Send for LlamaInference {}
unsafe impl Sync for LlamaInference {}

impl LlamaInference {
    pub fn load(model_path: &str, n_ctx: u32, n_threads: u32) -> Result<Self, String> {
        unsafe { ffi::init()?; }

        let model = unsafe { ffi::load_model(model_path) };
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

pub fn run_inference(
    inf: Arc<LlamaInference>,
    prompt: String,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    tx: std::sync::mpsc::Sender<StreamToken>,
    max_tokens: u32,
) {
    let (model, ctx) = inf.model_ctx();
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

pub fn format_chatml(messages: &[crate::conversation::Message]) -> String {
    let mut s = String::new();
    for msg in messages {
        s.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", msg.role, msg.content));
    }
    s.push_str("<|im_start|>assistant\n");
    s
}
