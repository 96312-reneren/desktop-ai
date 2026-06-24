use crate::ffi;

pub struct EmbeddingEngine {
    model: *mut ffi::LlamaModel,
    ctx: *mut ffi::LlamaContext,
    dim: usize,
}

// The engine is constructed on a worker thread and moved to the UI thread
// via a channel; it is only ever used from one thread at a time, so `Send`
// is sound. `Sync` is intentionally NOT implemented.
unsafe impl Send for EmbeddingEngine {}

impl EmbeddingEngine {
    pub fn load(model_path: &str, n_ctx: u32, n_threads: u32) -> Result<Self, String> {
        unsafe {
            ffi::init()?;
        }

        let model = unsafe { ffi::load_model(model_path) };
        if model.is_null() {
            return Err("failed to load model".into());
        }

        let dim_raw = unsafe { ffi::n_embd(model) };
        if dim_raw <= 0 {
            unsafe {
                ffi::free_model(model);
            }
            return Err("invalid embedding dimension".into());
        }
        let dim = dim_raw as usize;

        let ctx = unsafe { ffi::new_embedding_context(model, n_ctx, n_threads) };
        if ctx.is_null() {
            unsafe {
                ffi::free_model(model);
            }
            return Err("failed to create embedding context".into());
        }

        Ok(Self { model, ctx, dim })
    }

    pub fn embed(&self, text: &str) -> Vec<f32> {
        unsafe {
            let tokens = ffi::tokenize(self.model, text, true);
            if tokens.is_empty() {
                return vec![0.0; self.dim];
            }

            for chunk in tokens.chunks(512) {
                for &t in chunk {
                    ffi::decode(self.ctx, t);
                }
            }

            let n_tokens = tokens.len() as i32;
            if n_tokens == 0 {
                return vec![0.0; self.dim];
            }

            // Average pooling: get embedding for each token and average
            let embd_ptr = ffi::get_embeddings_ith(self.ctx, n_tokens - 1);
            if embd_ptr.is_null() {
                return vec![0.0; self.dim];
            }

            let slice = std::slice::from_raw_parts(embd_ptr, self.dim);
            // Normalize
            let norm: f32 = slice.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                slice.iter().map(|v| v / norm).collect()
            } else {
                slice.to_vec()
            }
        }
    }

    #[allow(dead_code)]
    pub fn dim(&self) -> usize {
        self.dim
    }

    fn unload(&mut self) {
        if !self.ctx.is_null() {
            unsafe {
                ffi::free_embd_context(self.ctx);
            }
            self.ctx = std::ptr::null_mut();
        }
        if !self.model.is_null() {
            unsafe {
                ffi::free_model(self.model);
            }
            self.model = std::ptr::null_mut();
        }
    }
}

impl Drop for EmbeddingEngine {
    fn drop(&mut self) {
        self.unload();
    }
}
