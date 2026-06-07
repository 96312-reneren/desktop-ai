use crate::ffi;
use std::sync::{Arc, mpsc};

pub enum StreamToken {
    Text(String),
    Done,
    Error(String),
}

/// Wrapper that implements Send/Sync for raw FFI pointers
struct SendPtr<T>(*mut T);
unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}

pub struct LlamaInference {
    model: SendPtr<ffi::LlamaModel>,
    ctx: SendPtr<ffi::LlamaContext>,
}

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

        Ok(Self { model: SendPtr(model), ctx: SendPtr(ctx) })
    }

    /// Extract raw pointers for use in background threads.
    /// The caller must ensure the inference lives longer than the thread.
    pub fn raw_ptrs(&self) -> (*mut ffi::LlamaModel, *mut ffi::LlamaContext) {
        (self.model.0, self.ctx.0)
    }

    pub fn unload(&mut self) {
        if !self.ctx.0.is_null() {
            unsafe { ffi::free_context(self.ctx.0); }
            self.ctx = SendPtr(std::ptr::null_mut());
        }
        if !self.model.0.is_null() {
            unsafe { ffi::free_model(self.model.0); }
            self.model = SendPtr(std::ptr::null_mut());
        }
    }
}

impl Drop for LlamaInference {
    fn drop(&mut self) { self.unload(); }
}

fn format_chatml(messages: &[crate::conversation::Message]) -> String {
    let mut s = String::new();
    for msg in messages {
        s.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", msg.role, msg.content));
    }
    s.push_str("<|im_start|>assistant\n");
    s
}
