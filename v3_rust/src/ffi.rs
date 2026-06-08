use libloading::{Library, Symbol};
use once_cell::sync::OnceCell;
use std::ffi::{c_char, c_void, CStr, CString};

// ─── C types ──────────────────────────────────────────

#[repr(C)] #[derive(Clone)]
pub struct LlamaModelParams {
    pub n_gpu_layers: i32, pub split_mode: i32, pub main_gpu: i32,
    pub tensor_split: *const f32,
    pub progress_callback: *const c_void, pub progress_callback_user_data: *const c_void,
    pub kv_overrides: *const c_void,
    pub vocab_only: bool, pub use_mmap: bool, pub use_mlock: bool, pub check_tensors: bool,
}

impl Default for LlamaModelParams { fn default() -> Self { unsafe { std::mem::zeroed() } } }

#[repr(C)] #[derive(Clone)]
pub struct LlamaContextParams {
    pub seed: u32, pub n_ctx: u32, pub n_batch: u32, pub n_ubatch: u32,
    pub n_seq_max: u32, pub n_threads: u32, pub n_threads_batch: u32,
    pub rope_scaling_type: i32, pub pooling_type: i32,
    pub rope_freq_base: f32, pub rope_freq_scale: f32,
    pub yarn_ext_factor: f32, pub yarn_attn_factor: f32,
    pub yarn_beta_fast: f32, pub yarn_beta_slow: f32, pub yarn_orig_ctx: u32,
    pub defrag_thold: f32,
    pub logits_all: bool, pub embeddings: bool, pub offload_kqv: bool,
    pub flash_attn: bool, pub no_perf: bool,
}

impl Default for LlamaContextParams { fn default() -> Self { unsafe { std::mem::zeroed() } } }

pub type LlamaToken = i32;
pub type LlamaModel = c_void;
pub type LlamaContext = c_void;

#[repr(C)] pub struct LlamaBatch {
    pub n_tokens: i32, pub token: *mut LlamaToken, pub embd: *mut f32,
    pub pos: *mut i32, pub n_seq_id: *mut i32, pub seq_id: *mut *mut i32, pub logits: *mut i8,
}

// ─── API function pointer types ───────────────────────

type PfnModelDefaultParams    = unsafe extern "C" fn() -> LlamaModelParams;
type PfnContextDefaultParams  = unsafe extern "C" fn() -> LlamaContextParams;
type PfnLoadModelFromFile    = unsafe extern "C" fn(*const c_char, LlamaModelParams) -> *mut LlamaModel;
type PfnNewContextWithModel  = unsafe extern "C" fn(*mut LlamaModel, LlamaContextParams) -> *mut LlamaContext;
type PfnFreeModel            = unsafe extern "C" fn(*mut LlamaModel);
type PfnFree                = unsafe extern "C" fn(*mut LlamaContext);
type PfnNVocab              = unsafe extern "C" fn(*const LlamaModel) -> i32;
type PfnTokenize            = unsafe extern "C" fn(*const LlamaModel, *const c_char, i32, *mut LlamaToken, i32, bool, bool) -> i32;
type PfnTokenToPiece        = unsafe extern "C" fn(*const LlamaModel, LlamaToken, *mut c_char, i32, i32, bool) -> i32;
type PfnBatchGetOne          = unsafe extern "C" fn(*mut LlamaToken, i32) -> LlamaBatch;
type PfnBatchFree            = unsafe extern "C" fn(LlamaBatch);
type PfnDecode              = unsafe extern "C" fn(*mut LlamaContext, LlamaBatch) -> i32;
type PfnSampleTokenGreedy   = unsafe extern "C" fn(*mut LlamaContext, *mut LlamaToken) -> LlamaToken;

// ─── DLL integrity ────────────────────────────────────

const LLAMA_DLL_MIN_SIZE: u64 = 1_000_000;

fn verify_dll(path: &str) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot access {}: {}", path, e))?;
    if meta.len() < LLAMA_DLL_MIN_SIZE {
        return Err(format!("llama.dll appears corrupted (size {} < {} bytes)", meta.len(), LLAMA_DLL_MIN_SIZE));
    }
    Ok(())
}

// ─── Global API ───────────────────────────────────────

static LLAMA_LIB: OnceCell<Library> = OnceCell::new();

fn lib() -> &'static Library { LLAMA_LIB.get().expect("llama.dll not loaded") }

/// Load llama.dll with integrity verification. Must be called once before any other function.
pub unsafe fn init() -> Result<(), String> {
    LLAMA_LIB.get_or_try_init(|| {
        verify_dll("llama.dll")?;
        Library::new("llama.dll").map_err(|e| format!("load llama.dll: {}", e))
    }).map(|_| ())
}

macro_rules! call {
    ($name:ident, $type:ty, $($arg:expr),*) => {{
        let sym: Symbol<$type> = match unsafe { lib().get(stringify!($name).as_bytes()) } {
            Ok(s) => s,
            Err(_) => {
                log::error!("missing FFI symbol: {}", stringify!($name));
                return Default::default();
            }
        };
        unsafe { sym($($arg),*) }
    }};
}

fn to_cstring_safe(s: &str) -> CString {
    let filtered: String = s.chars().map(|c| if c == '\0' { ' ' } else { c }).collect();
    CString::new(filtered).unwrap_or_else(|_| CString::new("").unwrap())
}

// ─── Safe wrapper functions ───────────────────────────

pub unsafe fn load_model(path: &str) -> *mut LlamaModel {
    let c_path = to_cstring_safe(path);
    let mut params = LlamaModelParams::default();
    params.use_mmap = true;
    params.use_mlock = false;
    params.n_gpu_layers = 0;
    call!(llama_load_model_from_file, PfnLoadModelFromFile, c_path.as_ptr(), params)
}

pub unsafe fn new_context(model: *mut LlamaModel, n_ctx: u32, n_threads: u32) -> *mut LlamaContext {
    let mut params = LlamaContextParams::default();
    params.n_ctx = n_ctx;
    params.n_batch = 512;
    params.n_ubatch = 512;
    params.n_seq_max = 1;
    params.n_threads = n_threads;
    params.n_threads_batch = n_threads;
    params.no_perf = true;
    call!(llama_new_context_with_model, PfnNewContextWithModel, model, params)
}

pub unsafe fn free_model(model: *mut LlamaModel) { call!(llama_free_model, PfnFreeModel, model); }
pub unsafe fn free_context(ctx: *mut LlamaContext) { call!(llama_free, PfnFree, ctx); }

pub unsafe fn n_vocab(model: *const LlamaModel) -> i32 {
    call!(llama_n_vocab, PfnNVocab, model)
}

pub unsafe fn tokenize(model: *const LlamaModel, text: &str, add_special: bool) -> Vec<LlamaToken> {
    let c_text = to_cstring_safe(text);
    let max_tokens = (text.len() * 2).max(256);
    let mut tokens = vec![0i32; max_tokens];
    let count = call!(llama_tokenize, PfnTokenize,
        model, c_text.as_ptr(), text.len() as i32, tokens.as_mut_ptr(), tokens.len() as i32, add_special, true);
    if count < 0 { return vec![1, 2]; }
    let count = (count as usize).min(tokens.len());
    tokens.truncate(count);
    tokens
}

pub unsafe fn token_to_piece(model: *const LlamaModel, token: LlamaToken) -> String {
    let mut buf = vec![0i8; 512];
    let len = call!(llama_token_to_piece, PfnTokenToPiece,
        model, token, buf.as_mut_ptr() as *mut c_char, buf.len() as i32, 0, true);
    if len <= 0 { return String::new(); }
    CStr::from_bytes_until_nul(std::slice::from_raw_parts(buf.as_ptr() as *const u8, len as usize))
        .unwrap_or_default().to_string_lossy().into_owned()
}

pub unsafe fn decode(ctx: *mut LlamaContext, token: LlamaToken) {
    let mut t = token;
    let _batch = call!(llama_batch_get_one, PfnBatchGetOne, &mut t, 1);
    call!(llama_decode, PfnDecode, ctx, _batch);
    // batch is consumed by decode, no free needed since llama_decode manages it
}

pub unsafe fn sample_greedy(ctx: *mut LlamaContext) -> LlamaToken {
    let mut tok: LlamaToken = 0;
    call!(llama_sample_token_greedy, PfnSampleTokenGreedy, ctx, &mut tok);
    tok
}
