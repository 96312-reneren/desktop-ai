use crate::store::config::ModelInfo;
use std::sync::mpsc;

/// Remote JSON catalog URL. The file must be a JSON array of ModelInfo-shaped
/// objects. Fetched in a background thread on app start; on failure the
/// hardcoded defaults below are used.
const CATALOG_URL: &str =
    "https://raw.githubusercontent.com/96312-reneren/desktop-ai/main/model_catalog.json";

/// 返回内置的六模型兜底清单。
///
/// 网络目录（[`CATALOG_URL`]）拉取失败或无缓存时使用。每个条目给出下载
/// `url`、本地 `filename`、整体 `expected_sha256` 以及分卷信息 `parts`
/// （单文件模型 `parts` 为空；分卷模型由下载器逐卷校验后合并）。
pub(crate) fn default_catalog() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "qwen2.5-0.5b".into(),
            name: "Qwen2.5-0.5B".into(),
            desc: "超微型模型，2GB内存即可运行，适合低配/老旧设备".into(),
            size_gb: 0.35,
            tags: vec!["超轻量".into(), "低配救星".into()],
            url: "https://hf-mirror.com/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-0.5b-instruct-q4_k_m.gguf".into(),
            expected_sha256: Some("74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db".into()),
            parts: vec![],
        },
        ModelInfo {
            id: "qwen3-1.7b".into(),
            name: "Qwen3-1.7B".into(),
            desc: "超轻量模型，4GB内存即可流畅运行，响应速度极快".into(),
            size_gb: 1.71,
            tags: vec!["轻量".into(), "快速".into(), "低配首选".into()],
            url: "https://hf-mirror.com/Qwen/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q8_0.gguf".into(),
            filename: "Qwen3-1.7B-Q8_0.gguf".into(),
            expected_sha256: Some("061b54daade076b5d3362dac252678d17da8c68f07560be70818cace6590cb1a".into()),
            parts: vec![],
        },
        ModelInfo {
            id: "qwen2.5-3b".into(),
            name: "Qwen2.5-3B".into(),
            desc: "轻量均衡模型，兼顾速度与质量，6GB内存推荐".into(),
            size_gb: 2.0,
            tags: vec!["均衡".into(), "推荐".into()],
            url: "https://hf-mirror.com/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-3b-instruct-q4_k_m.gguf".into(),
            expected_sha256: Some("626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d".into()),
            parts: vec![],
        },
        ModelInfo {
            id: "qwen2.5-7b".into(),
            name: "Qwen2.5-7B".into(),
            desc: "经典7B模型，综合能力强，8GB内存推荐".into(),
            size_gb: 4.7,
            tags: vec!["经典".into(), "综合".into()],
            // 官方仓库 q4_k_m 为双分卷，无单文件版本；下载器逐卷校验
            // SHA-256 后自动合并为完整 GGUF。
            url: "https://hf-mirror.com/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf".into(),
            filename: "qwen2.5-7b-instruct-q4_k_m.gguf".into(),
            expected_sha256: Some("0bb400ccfe4201fecfc96ed6e4442fe1106d1e003c09e024bd7981c4b69ef899".into()),
            parts: vec![
                crate::store::config::ModelPart {
                    url: "https://hf-mirror.com/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf".into(),
                    filename: "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf".into(),
                    sha256: Some("dfce12e3862a5283ccfb88221b48480e58745165de856439950d0f22590580db".into()),
                },
                crate::store::config::ModelPart {
                    url: "https://hf-mirror.com/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf".into(),
                    filename: "qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf".into(),
                    sha256: Some("539cf93f78e887edea1c04e2d7d8cdaca9d01dae9c9025bcb8accbe29df3d72a".into()),
                },
            ],
        },
        ModelInfo {
            id: "qwen2.5-coder-7b".into(),
            name: "Qwen2.5-Coder-7B".into(),
            desc: "代码专用模型，擅长编程、代码生成与解释".into(),
            size_gb: 4.7,
            tags: vec!["编程".into(), "代码".into()],
            url: "https://hf-mirror.com/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/qwen2.5-coder-7b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-coder-7b-instruct-q4_k_m.gguf".into(),
            expected_sha256: Some("509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c".into()),
            parts: vec![],
        },
        ModelInfo {
            id: "qwen3-8b".into(),
            name: "Qwen3-8B".into(),
            desc: "最新一代8B模型，推理能力更强，10GB内存推荐".into(),
            size_gb: 4.7,
            tags: vec!["最新".into(), "高性能".into()],
            url: "https://hf-mirror.com/Qwen/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf".into(),
            filename: "Qwen3-8B-Q4_K_M.gguf".into(),
            expected_sha256: Some("d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785".into()),
            parts: vec![],
        },
    ]
}

/// 按 `id` 在目录中查找模型条目，未命中返回 `None`。
pub fn find_model<'a>(catalog: &'a [ModelInfo], id: &str) -> Option<&'a ModelInfo> {
    catalog.iter().find(|m| m.id == id)
}

/// Fetch the remote model catalog in a background thread.
///
/// Returns a receiver that will deliver the fetched catalog (or an error
/// message) exactly once. The caller should poll with `try_recv()` and, on
/// success, merge the result into `config.model_catalog` via
/// [`merge_catalog`].
///
/// The fetch has a 10-second connect timeout and 30-second total timeout.
/// On any failure (network, parse, empty list) the error string is sent.
pub(crate) fn fetch_remote_catalog() -> mpsc::Receiver<Result<Vec<ModelInfo>, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = fetch_catalog_sync();
        let _ = tx.send(result);
    });
    rx
}

/// 同步拉取并解析远程模型目录（阻塞，仅应在本模块的后台线程中调用）。
///
/// 使用禁用重定向的 HTTP 客户端，带 10 秒连接超时与 30 秒总超时。要求响应
/// 为 `ModelInfo` 数组的 JSON；非 2xx 状态、读取或解析失败、以及空列表均
/// 以错误字符串返回。
fn fetch_catalog_sync() -> Result<Vec<ModelInfo>, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("HTTP client 构建失败: {e}"))?;

    let resp = client
        .get(CATALOG_URL)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body = resp.text().map_err(|e| format!("读取响应失败: {e}"))?;
    let catalog: Vec<ModelInfo> =
        serde_json::from_str(&body).map_err(|e| format!("JSON 解析失败: {e}"))?;

    if catalog.is_empty() {
        return Err("远程目录为空".into());
    }

    Ok(catalog)
}

/// Merge a remote catalog into the local one.
///
/// - Entries with the same `id` are replaced by the remote version.
/// - New remote entries are appended after existing ones.
/// - Local-only entries (not in remote) are kept unchanged.
///
/// Returns the number of entries added or updated.
pub(crate) fn merge_catalog(local: &mut Vec<ModelInfo>, remote: Vec<ModelInfo>) -> usize {
    let mut changed = 0;
    for entry in remote {
        if let Some(existing) = local.iter_mut().find(|m| m.id == entry.id) {
            *existing = entry;
        } else {
            local.push(entry);
        }
        changed += 1;
    }
    changed
}
