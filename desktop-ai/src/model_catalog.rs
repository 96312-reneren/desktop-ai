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
            expected_sha256: Some("74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db".into()),
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
        },
        ModelInfo {
            id: "qwen2.5-7b".into(),
            name: "Qwen2.5-7B".into(),
            desc: "经典7B模型，综合能力强，8GB内存推荐".into(),
            size_gb: 4.7,
            tags: vec!["经典".into(), "综合".into()],
            url: "https://hf-mirror.com/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf".into(),
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
            expected_sha256: Some("509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c".into()),
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
        },
    ]
}

pub fn find_model<'a>(catalog: &'a [ModelInfo], id: &str) -> Option<&'a ModelInfo> {
    catalog.iter().find(|m| m.id == id)
}
