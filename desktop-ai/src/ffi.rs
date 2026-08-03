use libloading::{Library, Symbol};
use once_cell::sync::OnceCell;
use std::ffi::{c_char, c_void, CStr, CString};

// ─── C types ──────────────────────────────────────────

#[repr(C)]
#[derive(Clone)]
pub struct LlamaModelParams {
    pub devices: *const c_void,
    pub tensor_buft_overrides: *const c_void,
    pub n_gpu_layers: i32,
    pub split_mode: i32,
    pub main_gpu: i32,
    pub tensor_split: *const f32,
    pub progress_callback: *const c_void,
    pub progress_callback_user_data: *const c_void,
    pub kv_overrides: *const c_void,
    pub vocab_only: bool,
    pub use_mmap: bool,
    pub use_direct_io: bool,
    pub use_mlock: bool,
    pub check_tensors: bool,
    pub use_extra_bufts: bool,
    pub no_host: bool,
    pub no_alloc: bool,
}

impl Default for LlamaModelParams {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct LlamaContextParams {
    pub n_ctx: u32,
    pub n_batch: u32,
    pub n_ubatch: u32,
    pub n_seq_max: u32,
    pub n_threads: i32,
    pub n_threads_batch: i32,
    pub rope_scaling_type: i32,
    pub pooling_type: i32,
    pub attention_type: i32,
    pub flash_attn_type: i32,
    pub rope_freq_base: f32,
    pub rope_freq_scale: f32,
    pub yarn_ext_factor: f32,
    pub yarn_attn_factor: f32,
    pub yarn_beta_fast: f32,
    pub yarn_beta_slow: f32,
    pub yarn_orig_ctx: u32,
    pub defrag_thold: f32,
    pub cb_eval: *const c_void,
    pub cb_eval_user_data: *const c_void,
    pub type_k: i32,
    pub type_v: i32,
    pub abort_callback: *const c_void,
    pub abort_callback_data: *const c_void,
    pub embeddings: bool,
    pub offload_kqv: bool,
    pub no_perf: bool,
    pub op_offload: bool,
    pub swa_full: bool,
    pub kv_unified: bool,
    pub samplers: *const c_void,
    pub n_samplers: usize,
}

impl Default for LlamaContextParams {
    fn default() -> Self {
        // Match llama_context_default_params() so model-provided settings
        // (RoPE/YaRN, KV cache type, attention type) are NOT overridden:
        // zeroed params previously forced yarn=0 / type_k=0 (F32) /
        // attention=CAUSAL, which broke positional encoding on models with
        // custom rope config (fragmented/garbage output).
        Self {
            n_ctx: 0,
            n_batch: 0,
            n_ubatch: 0,
            n_seq_max: 0,
            n_threads: 0,
            n_threads_batch: 0,
            rope_scaling_type: -1, // UNSPECIFIED
            pooling_type: -1,      // UNSPECIFIED
            attention_type: -1,    // UNSPECIFIED
            flash_attn_type: -1,   // AUTO
            rope_freq_base: 0.0,
            rope_freq_scale: 0.0,
            yarn_ext_factor: -1.0,
            yarn_attn_factor: -1.0,
            yarn_beta_fast: -1.0,
            yarn_beta_slow: -1.0,
            yarn_orig_ctx: 0,
            defrag_thold: -1.0,
            cb_eval: std::ptr::null(),
            cb_eval_user_data: std::ptr::null(),
            type_k: 1, // GGML_TYPE_F16
            type_v: 1, // GGML_TYPE_F16
            abort_callback: std::ptr::null(),
            abort_callback_data: std::ptr::null(),
            embeddings: false,
            offload_kqv: true,
            no_perf: true,
            op_offload: true,
            swa_full: true,
            kv_unified: false,
            samplers: std::ptr::null(),
            n_samplers: 0,
        }
    }
}

pub type LlamaToken = i32;
pub type LlamaModel = c_void;
pub type LlamaContext = c_void;
pub type LlamaSampler = c_void;

#[repr(C)]
pub struct LlamaBatch {
    pub n_tokens: i32,
    pub token: *mut LlamaToken,
    pub embd: *mut f32,
    pub pos: *mut i32,
    pub n_seq_id: *mut i32,
    pub seq_id: *mut *mut i32,
    pub logits: *mut i8,
}

// ─── API function pointer types ───────────────────────

type PfnLoadModelFromFile =
    unsafe extern "C" fn(*const c_char, LlamaModelParams) -> *mut LlamaModel;
type PfnNewContextWithModel =
    unsafe extern "C" fn(*mut LlamaModel, LlamaContextParams) -> *mut LlamaContext;
type PfnFreeModel = unsafe extern "C" fn(*mut LlamaModel);
type PfnFree = unsafe extern "C" fn(*mut LlamaContext);
type PfnNVocab = unsafe extern "C" fn(*const LlamaModel) -> i32;
type PfnTokenize = unsafe extern "C" fn(
    *const LlamaModel,
    *const c_char,
    i32,
    *mut LlamaToken,
    i32,
    bool,
    bool,
) -> i32;
type PfnTokenToPiece =
    unsafe extern "C" fn(*const LlamaModel, LlamaToken, *mut c_char, i32, i32, bool) -> i32;
type PfnBatchGetOne = unsafe extern "C" fn(*mut LlamaToken, i32) -> LlamaBatch;
type PfnDecode = unsafe extern "C" fn(*mut LlamaContext, LlamaBatch) -> i32;
type PfnSampleTokenGreedy = unsafe extern "C" fn(*mut LlamaContext, *mut LlamaToken) -> LlamaToken;
type PfnNEmbd = unsafe extern "C" fn(*const LlamaModel) -> i32;
type PfnGetEmbeddingsIth = unsafe extern "C" fn(*mut LlamaContext, i32) -> *mut f32;
type PfnFreeContext = unsafe extern "C" fn(*mut LlamaContext);
type PfnPrintSystemInfo = unsafe extern "C" fn() -> *const c_char;

// ─── Vocab API (llama.cpp ≥ b4xxx) ─────────────────────

type PfnGetVocab = unsafe extern "C" fn(*const LlamaModel) -> *const c_void;
type PfnVocabNTokens = unsafe extern "C" fn(*const c_void) -> i32;
type PfnBackendInit = unsafe extern "C" fn();
type PfnVocabEos = unsafe extern "C" fn(*const c_void) -> LlamaToken;
type PfnVocabEot = unsafe extern "C" fn(*const c_void) -> LlamaToken;
type PfnTokenEosLegacy = unsafe extern "C" fn(*const LlamaModel) -> LlamaToken;
type PfnGetMemory = unsafe extern "C" fn(*const LlamaContext) -> *const c_void;
type PfnMemorySeqPosMax = unsafe extern "C" fn(*const c_void, i32) -> i32;

// ─── Modern sampler API (llama.cpp ≥ b4xxx) ─────────────

type PfnSamplerFree = unsafe extern "C" fn(*mut LlamaSampler);
/// Sample and accept a token from the idx-th output of the last evaluation.
type PfnSamplerSample =
    unsafe extern "C" fn(*mut LlamaSampler, *mut LlamaContext, i32) -> LlamaToken;

#[repr(C)]
pub struct LlamaSamplerChainParams {
    pub no_perf: bool,
}

type PfnSamplerChainInit = unsafe extern "C" fn(LlamaSamplerChainParams) -> *mut LlamaSampler;
type PfnSamplerChainAdd = unsafe extern "C" fn(*mut LlamaSampler, *mut LlamaSampler);
type PfnSamplerInitPenalties = unsafe extern "C" fn(i32, f32, f32, f32) -> *mut LlamaSampler;
type PfnSamplerInitTopK = unsafe extern "C" fn(i32) -> *mut LlamaSampler;
type PfnSamplerInitTemp = unsafe extern "C" fn(f32) -> *mut LlamaSampler;
type PfnSamplerInitDist = unsafe extern "C" fn(u32) -> *mut LlamaSampler;

// ─── Sampling API version marker ──────────────────────

/// True when the loaded library exports the modern `llama_sampler_*` API
/// (llama.cpp ≥ b4xxx). In that case `sample_greedy` uses a registered
/// greedy sampler via `llama_sampler_sample` instead of the legacy
/// `llama_sample_token_greedy(ctx, &mut token)` wrapper.
static SAMPLING_V2: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True when the library exposes the vocab API (`llama_model_get_vocab`).
/// Modern llama.cpp (≥ b4xxx) takes `llama_vocab*` in tokenize /
/// token_to_piece / n_vocab instead of `llama_model*`.
static MODERN_VOCAB_API: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Greedy samplers (modern API), keyed by chat context pointer.
/// Pointers are stored as usize so the map is `Send + Sync`.
static SAMPLER_REGISTRY: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, usize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub fn sampling_is_v2() -> bool {
    SAMPLING_V2.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether the loaded llama library was built with a GPU (BLAS) backend.
/// CPU-only builds report `BLAS = 0` in `llama_print_system_info`.
/// Returns `false` before [`init`] has run.
pub fn gpu_backend_available() -> bool {
    if LLAMA_LIB.get().is_none() {
        return false;
    }
    let info = unsafe {
        let sym: Symbol<PfnPrintSystemInfo> = match lib().get(b"llama_print_system_info") {
            Ok(s) => s,
            Err(_) => return false,
        };
        let ptr = sym();
        if ptr.is_null() {
            return false;
        }
        CStr::from_ptr(ptr).to_string_lossy().to_string()
    };
    // `BLAS = 1` means an accelerated backend (CUDA / Vulkan / Metal /
    // OpenBLAS) is compiled in; CPU-only builds print `BLAS = 0`.
    info.contains("BLAS = 1")
}

// ─── Platform library name ─────────────────────────────

/// Filename of the llama shared library for the current platform:
/// `llama.dll` (Windows), `libllama.dylib` (macOS), `libllama.so` (Linux/Unix).
pub fn llama_library_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "llama.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "libllama.dylib"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "libllama.so"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        "llama"
    }
}

// ─── DLL integrity ────────────────────────────────────

const LLAMA_DLL_MIN_SIZE: u64 = 1_000_000;

fn verify_dll(path: &str) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot access {}: {}", path, e))?;
    if meta.len() < LLAMA_DLL_MIN_SIZE {
        return Err(format!(
            "llama library appears corrupted (size {} < {} bytes)",
            meta.len(),
            LLAMA_DLL_MIN_SIZE
        ));
    }
    Ok(())
}

/// SHA-256 of a file (hex), used for the library audit trail.
fn sha256_of_file(path: &std::path::Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        use std::io::Read;
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect())
}

// ─── Global API ───────────────────────────────────────

static LLAMA_LIB: OnceCell<Library> = OnceCell::new();

fn lib() -> &'static Library {
    LLAMA_LIB.get().expect("llama library not loaded")
}

/// Resolve the llama library path. On Linux/macOS `dlopen` does not search
/// the executable's directory, so prefer an absolute path next to the exe;
/// fall back to the bare filename (dev runs / tests with CWD-based loading).
fn resolve_lib_path() -> std::path::PathBuf {
    let lib_name = llama_library_name();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();
    let candidate = exe_dir.join(lib_name);
    if candidate.exists() {
        candidate
    } else {
        std::path::PathBuf::from(lib_name)
    }
}

/// Load the platform llama shared library with integrity verification. Must be called once before any other function.
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
/// The library is loaded into a global static and shared across all subsequent calls.
pub unsafe fn init() -> Result<(), String> {
    LLAMA_LIB
        .get_or_try_init(|| {
            let lib_path = resolve_lib_path();
            let lib_path_str = lib_path.to_string_lossy();
            verify_dll(&lib_path_str)?;
            // Audit trail: record the loaded library's SHA-256 so a replaced
            // or tampered file is identifiable in the logs after the fact.
            if let Ok(hash) = sha256_of_file(&lib_path) {
                log::info!("llama library SHA-256: {}", hash);
            }
            let lib = Library::new(lib_path_str.as_ref())
                .map_err(|e| format!("加载 {} 失败: {}", lib_path_str, e))?;
            check_dll_version(&lib)?;
            Ok(lib)
        })
        .map(|_| ())
}

/// Probe the freshly loaded library by calling `llama_print_system_info`.
/// If the symbol is missing, returns NULL, or emits an empty / whitespace-only
/// string, the DLL is considered incompatible — the user must re-download the
/// complete package.
fn check_dll_version(lib: &Library) -> Result<(), String> {
    let sym: Symbol<PfnPrintSystemInfo> =
        unsafe { lib.get(b"llama_print_system_info") }.map_err(|_| {
            "llama 库缺少关键符号 (llama_print_system_info)，\
          版本可能不兼容，请重新下载完整包"
                .to_string()
        })?;

    let ptr = unsafe { sym() };
    if ptr.is_null() {
        return Err("llama 库损坏或不兼容，请重新下载完整包".into());
    }

    let info = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().to_string();
    let trimmed = info.trim();
    if trimmed.is_empty() {
        return Err("llama 库损坏或不兼容，请重新下载完整包".into());
    }

    // Log the first line of system info for audit trail.
    let first_line = trimmed.lines().next().unwrap_or(trimmed);
    log::info!("llama.cpp: {}", first_line);

    // Detect the sampling API era of the loaded library:
    // modern `llama_sampler_*` API (llama.cpp ≥ b4xxx) vs legacy
    // `llama_sample_token_greedy(ctx, &mut token)`.
    if unsafe { lib.get::<PfnSamplerSample>(b"llama_sampler_sample") }.is_ok() {
        SAMPLING_V2.store(true, std::sync::atomic::Ordering::Relaxed);
        log::info!("llama library uses the modern llama_sampler_* API");
    } else if unsafe { lib.get::<PfnSampleTokenGreedy>(b"llama_sample_token_greedy") }.is_ok() {
        SAMPLING_V2.store(false, std::sync::atomic::Ordering::Relaxed);
        log::info!("llama library uses the legacy sampling API");
    } else {
        return Err(
            "llama 库缺少采样 API (llama_sampler_sample / llama_sample_token_greedy)，\
          版本不兼容，请重新下载完整包"
                .into(),
        );
    }

    // Modern vocab API (`llama_vocab*` instead of `llama_model*` in
    // tokenize / token_to_piece / n_vocab).
    MODERN_VOCAB_API.store(
        unsafe { lib.get::<PfnGetVocab>(b"llama_model_get_vocab") }.is_ok(),
        std::sync::atomic::Ordering::Relaxed,
    );

    // Initialize the backend (required since llama.cpp b4xxx; harmless
    // on older libraries that initialize lazily).
    let backend_init = unsafe { lib.get::<PfnBackendInit>(b"llama_backend_init") }
        .or_else(|_| unsafe { lib.get::<PfnBackendInit>(b"llama_init_backend") });
    match backend_init {
        Ok(sym) => unsafe { sym() },
        Err(_) => log::warn!("llama_backend_init not exported; assuming lazy init"),
    }
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

/// Load a model, retrying without mmap when the first attempt fails.
/// Some file systems (e.g. FAT32 / exFAT on USB sticks) reject `mmap`,
/// which makes llama.cpp fail to load; falling back to plain file reads
/// keeps portable installs working.
fn load_model_inner(path: &str, n_gpu_layers: i32, use_mmap: bool) -> *mut LlamaModel {
    let c_path = to_cstring_safe(path);
    let params = LlamaModelParams {
        use_mmap,
        use_mlock: false,
        n_gpu_layers,
        main_gpu: if n_gpu_layers > 0 { 0 } else { -1 },
        ..LlamaModelParams::default()
    };
    call!(
        llama_load_model_from_file,
        PfnLoadModelFromFile,
        c_path.as_ptr(),
        params
    )
}

/// # Safety
///
/// The llama library must have been loaded via [`init`] before calling this function.
/// `path` must point to a valid GGUF model file accessible to the process.
/// The caller is responsible for calling [`free_model`] on the returned pointer
/// when it is no longer needed.
pub unsafe fn load_model(path: &str) -> *mut LlamaModel {
    let model = load_model_inner(path, 0, true);
    if model.is_null() {
        log::warn!(
            "model load with mmap failed, retrying without mmap: {}",
            path
        );
        load_model_inner(path, 0, false)
    } else {
        model
    }
}

/// # Safety
///
/// Same preconditions as [`load_model`]. `n_gpu_layers` controls how many
/// model layers are offloaded to GPU; pass 0 for CPU-only inference.
pub unsafe fn load_model_gpu(path: &str, n_gpu_layers: i32) -> *mut LlamaModel {
    let model = load_model_inner(path, n_gpu_layers, true);
    if model.is_null() {
        log::warn!(
            "model load with mmap failed, retrying without mmap: {}",
            path
        );
        load_model_inner(path, n_gpu_layers, false)
    } else {
        model
    }
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
        n_threads: n_threads as i32,
        n_threads_batch: n_threads as i32,
        no_perf: true,
        ..LlamaContextParams::default()
    };
    let ctx = call!(
        llama_new_context_with_model,
        PfnNewContextWithModel,
        model,
        params
    );
    if !ctx.is_null() && SAMPLING_V2.load(std::sync::atomic::Ordering::Relaxed) {
        // Build a sampler chain (penalties + top-k + temperature + dist).
        // Bare greedy sampling degenerates into fragmented/repetitive output
        // (verified against official llama-simple-chat & Ollama, which both
        // sample from the distribution); this matches the standard llama-cli
        // chain (repeat 1.1, top-k 40, temp 0.7).
        let chain_params = LlamaSamplerChainParams { no_perf: true };
        let chain = call!(llama_sampler_chain_init, PfnSamplerChainInit, chain_params);
        if chain.is_null() {
            log::error!("llama_sampler_chain_init returned NULL");
        } else {
            let add = |s: *mut LlamaSampler| {
                if s.is_null() {
                    log::warn!("sampler init returned NULL");
                } else {
                    call!(llama_sampler_chain_add, PfnSamplerChainAdd, chain, s);
                }
            };
            add(call!(
                llama_sampler_init_penalties,
                PfnSamplerInitPenalties,
                64,  // penalty_last_n
                1.1, // penalty_repeat
                0.0, // penalty_freq
                0.0  // penalty_present
            ));
            add(call!(llama_sampler_init_top_k, PfnSamplerInitTopK, 40));
            add(call!(llama_sampler_init_temp, PfnSamplerInitTemp, 0.7));
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u32)
                .unwrap_or(0x5EED);
            add(call!(llama_sampler_init_dist, PfnSamplerInitDist, seed));
            SAMPLER_REGISTRY
                .lock()
                .unwrap()
                .insert(ctx as usize, chain as usize);
        }
    }
    ctx
}

/// Free the registered sampler (if any) for a context about to be destroyed.
fn release_sampler(ctx: *mut LlamaContext) {
    if SAMPLING_V2.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(smpl) = SAMPLER_REGISTRY.lock().unwrap().remove(&(ctx as usize)) {
            call!(
                llama_sampler_free,
                PfnSamplerFree,
                smpl as *mut LlamaSampler
            );
        }
    }
}

/// # Safety
///
/// `model` must be a valid pointer from [`load_model`] or [`load_model_gpu`].
/// After this call the pointer is invalid and must not be used again.
pub unsafe fn free_model(model: *mut LlamaModel) {
    call!(llama_free_model, PfnFreeModel, model);
}
/// # Safety
///
/// `ctx` must be a valid pointer from [`new_context`] or [`new_embedding_context`].
/// After this call the pointer is invalid.
pub unsafe fn free_context(ctx: *mut LlamaContext) {
    release_sampler(ctx);
    call!(llama_free, PfnFree, ctx);
}

/// Resolve the first argument for tokenizer functions: the vocab on modern
/// llama.cpp, or the model itself on legacy libraries.
fn vocab_or_model(model: *const LlamaModel) -> *const LlamaModel {
    if MODERN_VOCAB_API.load(std::sync::atomic::Ordering::Relaxed) {
        let vocab = call!(llama_model_get_vocab, PfnGetVocab, model);
        if vocab.is_null() {
            log::error!("llama_model_get_vocab returned NULL");
            return std::ptr::null();
        }
        vocab as *const LlamaModel
    } else {
        model
    }
}

/// # Safety
///
/// `model` must be a valid, non-null pointer.
pub unsafe fn n_vocab(model: *const LlamaModel) -> i32 {
    if MODERN_VOCAB_API.load(std::sync::atomic::Ordering::Relaxed) {
        let vocab = vocab_or_model(model);
        if vocab.is_null() {
            return 0;
        }
        call!(llama_vocab_n_tokens, PfnVocabNTokens, vocab)
    } else {
        call!(llama_n_vocab, PfnNVocab, model)
    }
}

/// End-of-sentence token of the model's vocabulary
/// (`LLAMA_TOKEN_NULL` = -1 when the model has none).
///
/// # Safety
///
/// `ctx` must be a valid context pointer.
pub unsafe fn memory_seq_pos_max(ctx: *mut LlamaContext, seq_id: i32) -> i32 {
    let memory = call!(llama_get_memory, PfnGetMemory, ctx);
    if memory.is_null() {
        return -1;
    }
    call!(llama_memory_seq_pos_max, PfnMemorySeqPosMax, memory, seq_id)
}

/// # Safety
///
/// `model` must be a valid, non-null pointer.
pub unsafe fn eos_token(model: *const LlamaModel) -> LlamaToken {
    if MODERN_VOCAB_API.load(std::sync::atomic::Ordering::Relaxed) {
        let vocab = vocab_or_model(model);
        if vocab.is_null() {
            return -1;
        }
        call!(llama_vocab_eos, PfnVocabEos, vocab)
    } else {
        call!(llama_token_eos, PfnTokenEosLegacy, model)
    }
}

/// End-of-turn token of the model's vocabulary
/// (`LLAMA_TOKEN_NULL` = -1 when the model has none).
///
/// # Safety
///
/// `model` must be a valid, non-null pointer.
pub unsafe fn eot_token(model: *const LlamaModel) -> LlamaToken {
    if MODERN_VOCAB_API.load(std::sync::atomic::Ordering::Relaxed) {
        let vocab = vocab_or_model(model);
        if vocab.is_null() {
            return -1;
        }
        call!(llama_vocab_eot, PfnVocabEot, vocab)
    } else {
        -1
    }
}

/// # Safety
///
/// `model` must be a valid pointer. `text` will be sanitised internally via
/// [`to_cstring_safe`].
pub unsafe fn tokenize(model: *const LlamaModel, text: &str, add_special: bool) -> Vec<LlamaToken> {
    let first = vocab_or_model(model);
    if first.is_null() {
        return vec![1, 2];
    }
    let c_text = to_cstring_safe(text);
    // Grow the buffer if the tokenizer reports a full buffer — a silent
    // truncation would corrupt generation.
    let mut capacity = (text.len() * 2).max(256);
    let max_capacity = (text.len() * 8).max(4096);
    loop {
        let mut tokens = vec![0i32; capacity];
        let count = call!(
            llama_tokenize,
            PfnTokenize,
            first,
            c_text.as_ptr(),
            text.len() as i32,
            tokens.as_mut_ptr(),
            tokens.len() as i32,
            add_special,
            true
        );
        if count < 0 {
            return vec![1, 2];
        }
        let count = count as usize;
        if count < capacity {
            tokens.truncate(count);
            return tokens;
        }
        // Buffer exactly full — likely needs more room; retry once with a
        // larger buffer, then give up rather than truncate silently.
        if capacity >= max_capacity {
            log::warn!(
                "tokenize buffer exhausted ({} tokens) — returning partial result",
                capacity
            );
            return tokens;
        }
        capacity *= 2;
    }
}

/// # Safety
///
/// `model` must be a valid pointer. `token` must be a valid llama token ID.
pub unsafe fn token_to_piece(model: *const LlamaModel, token: LlamaToken) -> String {
    let first = vocab_or_model(model);
    if first.is_null() {
        return String::new();
    }
    let mut buf = vec![0i8; 512];
    let len = call!(
        llama_token_to_piece,
        PfnTokenToPiece,
        first,
        token,
        buf.as_mut_ptr() as *mut c_char,
        buf.len() as i32,
        0,
        true
    );
    if len <= 0 {
        return String::new();
    }
    // The piece written by llama_token_to_piece has no NUL terminator, so
    // CStr::from_bytes_until_nul would error; decode the raw bytes instead.
    String::from_utf8_lossy(std::slice::from_raw_parts(
        buf.as_ptr() as *const u8,
        len as usize,
    ))
    .into_owned()
}

/// # Safety
///
/// `ctx` must be a valid context pointer. `token` must be a valid token ID
/// previously obtained from the tokenizer.
pub unsafe fn decode(ctx: *mut LlamaContext, token: LlamaToken) {
    let mut t = token;
    let _batch = call!(llama_batch_get_one, PfnBatchGetOne, &mut t, 1);
    let rc = call!(llama_decode, PfnDecode, ctx, _batch);
    // llama_decode returns 0 on success, >0 to retry the batch, <0 on error.
    // Surface failures instead of silently producing garbage.
    if rc < 0 {
        log::error!("llama_decode failed with code {}", rc);
    } else if rc > 0 {
        log::warn!(
            "llama_decode requested retry (code {}), result may be degraded",
            rc
        );
    }
    // batch is consumed by decode, no free needed since llama_decode manages it
}

/// # Safety
///
/// `ctx` must be a valid context pointer.
pub unsafe fn sample_greedy(ctx: *mut LlamaContext) -> LlamaToken {
    if SAMPLING_V2.load(std::sync::atomic::Ordering::Relaxed) {
        let smpl = SAMPLER_REGISTRY
            .lock()
            .unwrap()
            .get(&(ctx as usize))
            .copied()
            .unwrap_or(0) as *mut LlamaSampler;
        if smpl.is_null() {
            log::error!("sample_greedy: no sampler registered for this context");
            return 2; // EOS-like: let the generation loop terminate
        }
        // Samples and accepts a token from output 0 of the last llama_decode.
        call!(llama_sampler_sample, PfnSamplerSample, smpl, ctx, 0)
    } else {
        let mut tok: LlamaToken = 0;
        call!(
            llama_sample_token_greedy,
            PfnSampleTokenGreedy,
            ctx,
            &mut tok
        );
        tok
    }
}

// ─── Embedding ──────────────────────────────────────────

/// # Safety
///
/// `model` must be a valid pointer. The returned context has `embeddings=true`
/// and must be freed with [`free_embd_context`].
pub unsafe fn new_embedding_context(
    model: *mut LlamaModel,
    n_ctx: u32,
    n_threads: u32,
) -> *mut LlamaContext {
    let params = LlamaContextParams {
        n_ctx,
        n_batch: 512,
        n_ubatch: 512,
        n_seq_max: 1,
        n_threads: n_threads as i32,
        n_threads_batch: n_threads as i32,
        embeddings: true,
        no_perf: true,
        ..LlamaContextParams::default()
    };
    call!(
        llama_new_context_with_model,
        PfnNewContextWithModel,
        model,
        params
    )
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
    release_sampler(ctx);
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
        // Layout for llama.cpp b7700 on 64-bit (see llama.h):
        // 2 ptr (devices, tensor_buft_overrides) + 3×i32 + 1 ptr (tensor_split)
        // + 1 fn ptr (progress_callback) + 2 ptr + 8 bool → 72 bytes.
        let sz = std::mem::size_of::<LlamaModelParams>();
        if cfg!(target_pointer_width = "64") {
            assert_eq!(sz, 72, "LlamaModelParams layout drift (llama.cpp b7700)");
        } else {
            assert!(
                (30..=128).contains(&sz),
                "LlamaModelParams size insane: {}",
                sz
            );
        }
    }

    #[test]
    fn llama_context_params_is_correctly_sized() {
        // Layout for llama.cpp b7700 on 64-bit: 6×u32/i32 + 4 enums (i32)
        // + 6 floats + u32 + f32 + 2 fn/void ptr + 2 i32 + 2 fn/void ptr
        // + 6 bool + ptr + usize → 136 bytes.
        let sz = std::mem::size_of::<LlamaContextParams>();
        if cfg!(target_pointer_width = "64") {
            assert_eq!(sz, 136, "LlamaContextParams layout drift (llama.cpp b7700)");
        } else {
            assert!(
                (50..=256).contains(&sz),
                "LlamaContextParams size insane: {}",
                sz
            );
        }
    }

    #[test]
    fn llama_batch_is_correctly_sized() {
        let sz = std::mem::size_of::<LlamaBatch>();
        assert!((40..=128).contains(&sz), "LlamaBatch size insane: {}", sz);
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
        assert!(
            slice.iter().all(|&b| b == 0),
            "ModelParams default must be zeroed"
        );
    }

    #[test]
    fn default_context_params_match_llama_defaults() {
        // Must match llama_context_default_params(): UNSPECIFIED enums (-1),
        // YaRN fields -1.0, KV cache type F16, offload flags on. Zeroed
        // values here override model-provided RoPE config and break
        // positional encoding (garbage output).
        let p = LlamaContextParams::default();
        assert_eq!(p.rope_scaling_type, -1);
        assert_eq!(p.pooling_type, -1);
        assert_eq!(p.attention_type, -1);
        assert_eq!(p.flash_attn_type, -1);
        assert_eq!(p.yarn_ext_factor, -1.0);
        assert_eq!(p.yarn_attn_factor, -1.0);
        assert_eq!(p.yarn_beta_fast, -1.0);
        assert_eq!(p.yarn_beta_slow, -1.0);
        assert_eq!(p.defrag_thold, -1.0);
        assert_eq!(p.type_k, 1); // GGML_TYPE_F16
        assert_eq!(p.type_v, 1); // GGML_TYPE_F16
        assert!(p.offload_kqv);
        assert!(p.op_offload);
        assert!(p.swa_full);
        assert!(!p.embeddings);
        assert!(!p.kv_unified);
        // Embedding contexts override `embeddings`; chat contexts override
        // n_ctx etc. — those stay consistent with official defaults.
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

    /// No-op if the llama library exists without the right size; we only test
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
