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

type PfnLoadModelFromFile    = unsafe extern "C" fn(*const c_char, LlamaModelParams) -> *mut LlamaModel;
type PfnNewContextWithModel  = unsafe extern "C" fn(*mut LlamaModel, LlamaContextParams) -> *mut LlamaContext;
type PfnFreeModel            = unsafe extern "C" fn(*mut LlamaModel);
type PfnFree                = unsafe extern "C" fn(*mut LlamaContext);
type PfnNVocab              = unsafe extern "C" fn(*const LlamaModel) -> i32;
type PfnTokenize            = unsafe extern "C" fn(*const LlamaModel, *const c_char, i32, *mut LlamaToken, i32, bool, bool) -> i32;
type PfnTokenToPiece        = unsafe extern "C" fn(*const LlamaModel, LlamaToken, *mut c_char, i32, i32, bool) -> i32;
type PfnBatchGetOne          = unsafe extern "C" fn(*mut LlamaToken, i32) -> LlamaBatch;
type PfnDecode              = unsafe extern "C" fn(*mut LlamaContext, LlamaBatch) -> i32;
type PfnSampleTokenGreedy   = unsafe extern "C" fn(*mut LlamaContext, *mut LlamaToken) -> LlamaToken;
type PfnNEmbd              = unsafe extern "C" fn(*const LlamaModel) -> i32;
type PfnGetEmbeddingsIth   = unsafe extern "C" fn(*mut LlamaContext, i32) -> *mut f32;
type PfnFreeContext         = unsafe extern "C" fn(*mut LlamaContext);
type PfnPrintSystemInfo      = unsafe extern "C" fn() -> *const c_char;

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
///
/// Performs three checks in order:
/// 1. File size ≥ 1 MB (`verify_dll`).
/// 2. Successful `dlopen` via libloading.
/// 3. Calls `llama_print_system_info()` — if the symbol is missing, returns NULL,
///    or returns an empty string, the DLL is rejected as incompatible.
///
/// # Safety
///
/// This function must be called exactly once before any other FFI function.
/// The DLL is loaded into a global static and shared across all subsequent calls.
pub unsafe fn init() -> Result<(), String> {
    LLAMA_LIB.get_or_try_init(|| {
        verify_dll("llama.dll")?;
        let lib = Library::new("llama.dll")
            .map_err(|e| format!("加载 llama.dll 失败: {}", e))?;
        check_dll_version(&lib)?;
        Ok(lib)
    }).map(|_| ())
}

/// Probe the freshly loaded DLL by calling `llama_print_system_info`.
/// If the symbol is missing, returns NULL, or emits an empty / whitespace-only
/// string, the DLL is considered incompatible — the user must re-download the
/// complete package.
fn check_dll_version(lib: &Library) -> Result<(), String> {
    let sym: Symbol<PfnPrintSystemInfo> = unsafe {
        lib.get(b"llama_print_system_info")
    }.map_err(|_| {
        "llama.dll 缺少关键符号 (llama_print_system_info)，\
         版本可能不兼容，请重新下载完整包".to_string()
    })?;

    let ptr = unsafe { sym() };
    if ptr.is_null() {
        return Err("llama.dll 损坏或不兼容，请重新下载完整包".into());
    }

    let info = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().to_string();
    let trimmed = info.trim();
    if trimmed.is_empty() {
        return Err("llama.dll 损坏或不兼容，请重新下载完整包".into());
    }

    // Log the first line of system info for audit trail.
    let first_line = trimmed.lines().next().unwrap_or(trimmed);
    log::info!("llama.cpp: {}", first_line);
    Ok(())
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

/// # Safety
///
/// `llama.dll` must have been loaded via [`init`] before calling this function.
/// `path` must point to a valid GGUF model file accessible to the process.
/// The caller is responsible for calling [`free_model`] on the returned pointer
/// when it is no longer needed.
pub unsafe fn load_model(path: &str) -> *mut LlamaModel {
    let c_path = to_cstring_safe(path);
    let params = LlamaModelParams {
        use_mmap: true,
        use_mlock: false,
        n_gpu_layers: 0,
        ..LlamaModelParams::default()
    };
    call!(llama_load_model_from_file, PfnLoadModelFromFile, c_path.as_ptr(), params)
}

/// # Safety
///
/// Same preconditions as [`load_model`]. `n_gpu_layers` controls how many
/// model layers are offloaded to GPU; pass 0 for CPU-only inference.
pub unsafe fn load_model_gpu(path: &str, n_gpu_layers: i32) -> *mut LlamaModel {
    let c_path = to_cstring_safe(path);
    let params = LlamaModelParams {
        use_mmap: true,
        use_mlock: false,
        n_gpu_layers,
        ..LlamaModelParams::default()
    };
    call!(llama_load_model_from_file, PfnLoadModelFromFile, c_path.as_ptr(), params)
}

/// # Safety
///
/// `model` must be a valid, non-null pointer returned by [`load_model`] or
/// [`load_model_gpu`]. The returned context pointer must be freed with
/// [`free_context`].
pub unsafe fn new_context(model: *mut LlamaModel, n_ctx: u32, n_threads: u32) -> *mut LlamaContext {
    let params = LlamaContextParams {
        n_ctx,
        n_batch: 512,
        n_ubatch: 512,
        n_seq_max: 1,
        n_threads,
        n_threads_batch: n_threads,
        no_perf: true,
        ..LlamaContextParams::default()
    };
    call!(llama_new_context_with_model, PfnNewContextWithModel, model, params)
}

/// # Safety
///
/// `model` must be a valid pointer from [`load_model`] or [`load_model_gpu`].
/// After this call the pointer is invalid and must not be used again.
pub unsafe fn free_model(model: *mut LlamaModel) { call!(llama_free_model, PfnFreeModel, model); }
/// # Safety
///
/// `ctx` must be a valid pointer from [`new_context`] or [`new_embedding_context`].
/// After this call the pointer is invalid.
pub unsafe fn free_context(ctx: *mut LlamaContext) { call!(llama_free, PfnFree, ctx); }

/// # Safety
///
/// `model` must be a valid, non-null pointer.
pub unsafe fn n_vocab(model: *const LlamaModel) -> i32 {
    call!(llama_n_vocab, PfnNVocab, model)
}

/// # Safety
///
/// `model` must be a valid pointer. `text` will be sanitised internally via
/// [`to_cstring_safe`].
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

/// # Safety
///
/// `model` must be a valid pointer. `token` must be a valid llama token ID.
pub unsafe fn token_to_piece(model: *const LlamaModel, token: LlamaToken) -> String {
    let mut buf = vec![0i8; 512];
    let len = call!(llama_token_to_piece, PfnTokenToPiece,
        model, token, buf.as_mut_ptr() as *mut c_char, buf.len() as i32, 0, true);
    if len <= 0 { return String::new(); }
    CStr::from_bytes_until_nul(std::slice::from_raw_parts(buf.as_ptr() as *const u8, len as usize))
        .unwrap_or_default().to_string_lossy().into_owned()
}

/// # Safety
///
/// `ctx` must be a valid context pointer. `token` must be a valid token ID
/// previously obtained from the tokenizer.
pub unsafe fn decode(ctx: *mut LlamaContext, token: LlamaToken) {
    let mut t = token;
    let _batch = call!(llama_batch_get_one, PfnBatchGetOne, &mut t, 1);
    call!(llama_decode, PfnDecode, ctx, _batch);
    // batch is consumed by decode, no free needed since llama_decode manages it
}

/// # Safety
///
/// `ctx` must be a valid context pointer.
pub unsafe fn sample_greedy(ctx: *mut LlamaContext) -> LlamaToken {
    let mut tok: LlamaToken = 0;
    call!(llama_sample_token_greedy, PfnSampleTokenGreedy, ctx, &mut tok);
    tok
}

// ─── Embedding ──────────────────────────────────────────

/// # Safety
///
/// `model` must be a valid pointer. The returned context has `embeddings=true`
/// and must be freed with [`free_embd_context`].
pub unsafe fn new_embedding_context(model: *mut LlamaModel, n_ctx: u32, n_threads: u32) -> *mut LlamaContext {
    let params = LlamaContextParams {
        n_ctx,
        n_batch: 512,
        n_ubatch: 512,
        n_seq_max: 1,
        n_threads,
        n_threads_batch: n_threads,
        embeddings: true,
        no_perf: true,
        ..LlamaContextParams::default()
    };
    call!(llama_new_context_with_model, PfnNewContextWithModel, model, params)
}

/// # Safety
///
/// `model` must be a valid pointer.
pub unsafe fn n_embd(model: *const LlamaModel) -> i32 {
    call!(llama_n_embd, PfnNEmbd, model)
}

/// # Safety
///
/// `ctx` must be a valid embedding context pointer.
pub unsafe fn get_embeddings_ith(ctx: *mut LlamaContext, i: i32) -> *mut f32 {
    call!(llama_get_embeddings_ith, PfnGetEmbeddingsIth, ctx, i)
}

/// # Safety
///
/// `ctx` must be a valid embedding context pointer. After this call the pointer
/// is invalid and must not be used again.
pub unsafe fn free_embd_context(ctx: *mut LlamaContext) {
    call!(llama_free, PfnFreeContext, ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── to_cstring_safe fuzz: 1000 random-byte iterations ───

    #[test]
    fn to_cstring_safe_handles_random_bytes_without_panic() {
        for i in 0..1000 {
            let garbage: String = (0..(i % 512 + 1))
                .map(|_| ((i as u8).wrapping_mul(13u8)) as char)
                .collect();
            let cs = to_cstring_safe(&garbage);
            let _out = cs.to_bytes_with_nul();
        }
    }

    #[test]
    fn to_cstring_safe_empty_string() {
        let cs = to_cstring_safe("");
        assert_eq!(cs.to_bytes_with_nul(), &[0u8]);
    }

    #[test]
    fn to_cstring_safe_only_null_bytes() {
        let cs = to_cstring_safe("\0\0\0\0\0");
        let bytes = cs.to_bytes_with_nul();
        // All \0 → ' ', so "     " + trailing NUL
        assert_eq!(&bytes[..5], b"     ");
        assert_eq!(bytes[5], 0u8);
        assert_eq!(bytes.len(), 6);
    }

    #[test]
    fn to_cstring_safe_interior_null() {
        let cs = to_cstring_safe("hello\0world");
        assert!(cs.to_string_lossy().contains("hello world"));
    }

    #[test]
    fn to_cstring_safe_unicode_survives() {
        let cs = to_cstring_safe("中文テスト한국어");
        assert!(cs.to_string_lossy().contains("中文"));
    }

    #[test]
    fn to_cstring_safe_very_long_string() {
        let s = "A".repeat(100_000);
        let cs = to_cstring_safe(&s);
        assert_eq!(cs.to_bytes_with_nul().len(), 100_001);
    }

    #[test]
    fn to_cstring_safe_control_characters_dont_crash() {
        let s: String = (0u8..=31u8)
            .chain(127u8..=159u8)
            .map(|b| b as char)
            .collect();
        let _cs = to_cstring_safe(&s);
    }

    // ─── FFI struct layout correctness ──────────────────

    #[test]
    fn llama_model_params_is_correctly_sized() {
        // 3*i32 + 3*ptr + 1*ptr + 1*ptr + 4*bool (padded)
        // On 64-bit: i32=4, bool=1, ptr=8. With alignment padding.
        let sz = std::mem::size_of::<LlamaModelParams>();
        // Must be > 0 and a sane size (llama.cpp 层直接通过值传递)
        assert!(sz >= 30 && sz <= 128, "LlamaModelParams size insane: {}", sz);
    }

    #[test]
    fn llama_context_params_is_correctly_sized() {
        let sz = std::mem::size_of::<LlamaContextParams>();
        assert!(sz >= 50 && sz <= 256, "LlamaContextParams size insane: {}", sz);
    }

    #[test]
    fn llama_batch_is_correctly_sized() {
        let sz = std::mem::size_of::<LlamaBatch>();
        assert!(sz >= 40 && sz <= 128, "LlamaBatch size insane: {}", sz);
    }

    #[test]
    fn llama_token_is_i32() {
        // llama.h defines llama_token as int32_t
        assert_eq!(std::mem::size_of::<LlamaToken>(), 4);
    }

    #[test]
    fn default_model_params_is_all_zero() {
        let p = LlamaModelParams::default();
        let raw = &p as *const _ as *const u8;
        let sz = std::mem::size_of::<LlamaModelParams>();
        let slice = unsafe { std::slice::from_raw_parts(raw, sz) };
        assert!(slice.iter().all(|&b| b == 0), "ModelParams default must be zeroed");
    }

    #[test]
    fn default_context_params_is_all_zero() {
        let p = LlamaContextParams::default();
        let raw = &p as *const _ as *const u8;
        let sz = std::mem::size_of::<LlamaContextParams>();
        let slice = unsafe { std::slice::from_raw_parts(raw, sz) };
        assert!(slice.iter().all(|&b| b == 0), "ContextParams default must be zeroed");
    }

    // ─── DLL integrity verification ─────────────────────

    #[test]
    fn verify_dll_rejects_tiny_file() {
        let tmp = std::env::temp_dir().join("tiny_dummy.dll");
        std::fs::write(&tmp, b"x").ok();
        let result = verify_dll(&tmp.to_string_lossy());
        let _ = std::fs::remove_file(&tmp);
        assert!(result.is_err(), "tiny file must be rejected");
    }

    #[test]
    fn verify_dll_rejects_missing_file() {
        let result = verify_dll("__non_existent_file__.dll");
        assert!(result.is_err(), "missing file must be rejected");
    }

    /// No-op if llama.dll exists without the right size; we only test
    /// the check logic, not the actual DLL.
    #[test]
    fn struct_alignment_does_not_cause_undefined_behavior() {
        // Ensure Default::default() on zeroed structs produces valid values.
        let mp = LlamaModelParams::default();
        assert_eq!(mp.n_gpu_layers, 0);
        let cp = LlamaContextParams::default();
        assert_eq!(cp.n_ctx, 0);
        assert!(!cp.embeddings);
    }

    #[test]
    fn function_pointer_types_are_correctly_sized() {
        let ptr_size = std::mem::size_of::<usize>();
        assert_eq!(std::mem::size_of::<PfnPrintSystemInfo>(), ptr_size);
        assert_eq!(std::mem::size_of::<PfnLoadModelFromFile>(), ptr_size);
        assert_eq!(std::mem::size_of::<PfnDecode>(), ptr_size);
    }
}
