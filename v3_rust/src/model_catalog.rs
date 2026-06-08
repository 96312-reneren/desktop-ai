use crate::config::ModelInfo;

pub fn default_catalog() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "qwen2.5-0.5b".into(),
            name: "Qwen2.5-0.5B".into(),
            desc: "超微型模型，2GB内存即可运行，适合低配/老旧设备".into(),
            size_gb: 0.35,
            tags: vec!["超轻量".into(), "低配救星".into()],
            url: "https://hf-mirror.com/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-0.5b-instruct-q4_k_m.gguf".into(),
            expected_sha256: None,
        },
        ModelInfo {
            id: "qwen3-1.7b".into(),
            name: "Qwen3-1.7B".into(),
            desc: "超轻量模型，4GB内存即可流畅运行，响应速度极快".into(),
            size_gb: 1.2,
            tags: vec!["轻量".into(), "快速".into(), "低配首选".into()],
            url: "https://hf-mirror.com/Qwen/Qwen3-1.7B-GGUF/resolve/main/qwen3-1.7b-instruct-q4_k_m.gguf".into(),
            filename: "qwen3-1.7b-instruct-q4_k_m.gguf".into(),
            expected_sha256: None,
        },
        ModelInfo {
            id: "qwen2.5-3b".into(),
            name: "Qwen2.5-3B".into(),
            desc: "轻量均衡模型，兼顾速度与质量，6GB内存推荐".into(),
            size_gb: 2.0,
            tags: vec!["均衡".into(), "推荐".into()],
            url: "https://hf-mirror.com/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-3b-instruct-q4_k_m.gguf".into(),
            expected_sha256: None,
        },
        ModelInfo {
            id: "qwen2.5-7b".into(),
            name: "Qwen2.5-7B".into(),
            desc: "经典7B模型，综合能力强，8GB内存推荐".into(),
            size_gb: 4.7,
            tags: vec!["经典".into(), "综合".into()],
            url: "https://hf-mirror.com/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-7b-instruct-q4_k_m.gguf".into(),
            expected_sha256: None,
        },
        ModelInfo {
            id: "qwen2.5-coder-7b".into(),
            name: "Qwen2.5-Coder-7B".into(),
            desc: "代码专用模型，擅长编程、代码生成与解释".into(),
            size_gb: 4.7,
            tags: vec!["编程".into(), "代码".into()],
            url: "https://hf-mirror.com/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/qwen2.5-coder-7b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-coder-7b-instruct-q4_k_m.gguf".into(),
            expected_sha256: None,
        },
        ModelInfo {
            id: "qwen3-8b".into(),
            name: "Qwen3-8B".into(),
            desc: "最新一代8B模型，推理能力更强，10GB内存推荐".into(),
            size_gb: 5.5,
            tags: vec!["最新".into(), "高性能".into()],
            url: "https://hf-mirror.com/Qwen/Qwen3-8B-GGUF/resolve/main/qwen3-8b-instruct-q4_k_m.gguf".into(),
            filename: "qwen3-8b-instruct-q4_k_m.gguf".into(),
            expected_sha256: None,
        },
    ]
}

pub fn find_model<'a>(catalog: &'a [ModelInfo], id: &str) -> Option<&'a ModelInfo> {
    catalog.iter().find(|m| m.id == id)
}
