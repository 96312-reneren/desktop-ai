use crate::llm::ffi;

/// 语义向量引擎：持有一份 llama.cpp embedding 模型及其推理上下文。
///
/// 为文本产出 L2 归一化向量，供 `vector_store` 以余弦相似度检索。句柄为
/// 裸指针，销毁时经 [`EmbeddingEngine::unload`] 释放；不实现 `Sync`，须
/// 在单一时刻仅由一个线程使用。
pub(crate) struct EmbeddingEngine {
    /// llama.cpp 模型句柄，`null` 表示未加载。
    model: *mut ffi::LlamaModel,
    /// embedding 推理上下文，`null` 表示未创建。
    ctx: *mut ffi::LlamaContext,
    /// 向量维度，取自模型的 `n_embd`。
    dim: usize,
}

// The engine is constructed on a worker thread and moved to the UI thread
// via a channel; it is only ever used from one thread at a time, so `Send`
// is sound. `Sync` is intentionally NOT implemented.
unsafe impl Send for EmbeddingEngine {}

impl EmbeddingEngine {
    /// 加载指定 GGUF 模型并创建 embedding 上下文。
    ///
    /// 依次初始化 FFI、加载模型、校验 `n_embd` 为正、创建嵌入上下文；任一
    /// 步骤失败都会先释放已占用的模型句柄再返回错误，避免泄漏。成功后返回
    /// 的引擎通过 channel 跨线程移交，但同一时刻只应被一个线程使用。
    pub(crate) fn load(model_path: &str, n_ctx: u32, n_threads: u32) -> Result<Self, String> {
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

    /// 将文本编码为 L2 归一化向量，长度固定为 [`EmbeddingEngine::dim`]。
    ///
    /// 分词后按 512 token 分块解码（解码器模型的隐藏态依赖最近一次前向），
    /// 取最后一个 token 的隐藏态作为句向量。空输入或任一步失败时返回全零
    /// 向量而非报错，调用方需自行容忍该退化结果。
    pub(crate) fn embed(&self, text: &str) -> Vec<f32> {
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

            // Decoder-model embedding: use the hidden state of the LAST token
            // (index -1 = most recent embedding; a positive index would point
            // past the single output of the final llama_decode call).
            let embd_ptr = ffi::get_embeddings_ith(self.ctx, -1);
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

    /// 返回向量维度（`n_embd`）。目前尚无调用方（见 `#[allow(dead_code)]`）。
    #[allow(dead_code)]
    pub(crate) fn dim(&self) -> usize {
        self.dim
    }

    /// 释放上下文与模型句柄并将指针置空；可重复调用（幂等）。
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
    /// 析构时释放模型与上下文，确保 FFI 资源不泄漏。
    fn drop(&mut self) {
        self.unload();
    }
}
