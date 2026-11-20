//! 命令行入口：`--serve` 无界面启动本地模型与 OpenAI 兼容 API。
//!
//! 设计边界：本模块只做参数解析与装配（读配置 → 解析模型 → 加载推理 →
//! 启动 [`ApiServer`] → 常驻），不涉及任何 UI 代码。GUI 入口仍为
//! [`crate::ui::startup::run`]；两者共用同一份 `config.json` 与日志管道。
//!
//! 该模式的价值是「把 DesktopAI 当模型后端」：不打开窗口也能让 Chatbox、
//! Continue 一类客户端直接指向 `http://127.0.0.1:<port>/v1`。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::llm::inference::LlamaInference;
use crate::server::api_server::ApiServer;
use crate::store::config::{self, Config, ModelInfo};

/// `--serve` 的可选参数；未给出的项回落到 `config.json` 中的值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ServeOptions {
    /// 模型：目录 id（如 `qwen2.5-0.5b`）、models 目录内的文件名，或 `.gguf` 路径。
    pub model: Option<String>,
    /// 监听端口。
    pub port: Option<u16>,
    /// API 鉴权 token；缺省用配置中的随机 token。
    pub token: Option<String>,
    /// 上下文长度（token）。
    pub n_ctx: Option<u32>,
    /// 卸载到 GPU 的层数。
    pub gpu_layers: Option<i32>,
}

/// 解析后的命令行意图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliCommand {
    /// 无参数：正常启动图形界面。
    Gui,
    /// `serve` / `--serve`：无界面启动 API 服务。
    Serve(ServeOptions),
    /// `--help` / `-h`。
    Help,
    /// `--version` / `-V`。
    Version,
}

/// 帮助文本（`--help` 与参数错误时打印）。
pub(crate) fn usage() -> String {
    format!(
        "桌面AI v{}\n\
         \n\
         用法:\n\
         \x20 桌面AI                      启动图形界面\n\
         \x20 桌面AI serve [选项]         无界面启动本地模型 + OpenAI 兼容 API\n\
         \x20 桌面AI --help              显示本帮助\n\
         \x20 桌面AI --version           显示版本\n\
         \n\
         serve 选项:\n\
         \x20  --model <id|文件名|路径>   模型；缺省用配置里当前选中的模型\n\
         \x20  --port <端口>             监听端口；缺省用 config.json 的 api_port\n\
         \x20  --token <令牌>            API 鉴权令牌；缺省用 config.json 的 api_token\n\
         \x20  --n-ctx <长度>            上下文长度；缺省用配置值\n\
         \x20  --gpu-layers <层数>       卸载到 GPU 的层数；缺省用配置值\n\
         \n\
         示例:\n\
         \x20 桌面AI serve --port 8321\n\
         \x20 桌面AI serve --model qwen2.5-0.5b\n\
         \x20 桌面AI serve --model D:/models/qwen3-8b-q4.gguf --gpu-layers 99\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// 把 `std::env::args()` 之后的参数解析成 [`CliCommand`]。
///
/// 约定：`serve` 子命令与 `--serve` 等价；数值参数支持 `--port 8321` 与
/// `--port=8321` 两种写法；未知参数一律报错（不静默忽略，避免用户以为生效）。
pub(crate) fn parse_cli(args: &[String]) -> Result<CliCommand, String> {
    // 先归一化成 (key, inline_value) 序列，便于统一处理 `--k v` 与 `--k=v`。
    let mut pairs: Vec<(String, Option<String>)> = Vec::with_capacity(args.len());
    for arg in args {
        match arg.split_once('=') {
            Some((k, v)) => pairs.push((k.to_string(), Some(v.to_string()))),
            None => pairs.push((arg.clone(), None)),
        }
    }

    let Some((head, head_inline)) = pairs.first() else {
        return Ok(CliCommand::Gui);
    };
    match head.as_str() {
        "--help" | "-h" => return Ok(CliCommand::Help),
        "--version" | "-V" => return Ok(CliCommand::Version),
        "serve" | "--serve" => {}
        // 首个参数不是子命令：仍然按 GUI 启动，但打错的选项必须报错，
        // 否则用户会误以为参数生效。
        other if other.starts_with('-') => {
            return Err(format!("未知选项: {}", other));
        }
        _ => return Ok(CliCommand::Gui),
    }
    if head_inline.is_some() {
        return Err(format!("参数 {} 不接受取值", head));
    }

    let mut opts = ServeOptions::default();
    let mut rest = pairs[1..].iter().peekable();
    while let Some((key, inline)) = rest.next() {
        // 取当前选项的取值：`--k=v` 直接用 inline，`--k v` 吃掉下一个参数。
        let mut value = |key: &str, inline: &Option<String>| -> Result<String, String> {
            if let Some(v) = inline {
                return Ok(v.clone());
            }
            match rest.next() {
                Some((v, None)) => Ok(v.clone()),
                Some((v, Some(_))) => Err(format!("参数 {} 缺少取值（{} 不是取值）", key, v)),
                None => Err(format!("参数 {} 缺少取值", key)),
            }
        };

        match key.as_str() {
            "--model" | "-m" => opts.model = Some(value(key.as_str(), inline)?),
            "--token" | "-t" => opts.token = Some(value(key.as_str(), inline)?),
            "--port" | "-p" => {
                let raw = value(key.as_str(), inline)?;
                opts.port = Some(
                    raw.parse()
                        .map_err(|_| format!("--port 需要 1-65535 的整数，收到: {}", raw))?,
                );
            }
            "--n-ctx" => {
                let raw = value(key.as_str(), inline)?;
                opts.n_ctx = Some(
                    raw.parse()
                        .map_err(|_| format!("--n-ctx 需要正整数，收到: {}", raw))?,
                );
            }
            "--gpu-layers" => {
                let raw = value(key.as_str(), inline)?;
                opts.gpu_layers = Some(
                    raw.parse()
                        .map_err(|_| format!("--gpu-layers 需要整数，收到: {}", raw))?,
                );
            }
            "--help" | "-h" => return Ok(CliCommand::Help),
            other => return Err(format!("未知选项: {}", other)),
        }
    }

    Ok(CliCommand::Serve(opts))
}

/// 解析要加载的模型，返回 `(文件路径, 展示名)`。
///
/// 优先级：显式 `.gguf` 路径 > models 目录内文件名 > 模型目录 id >
/// 配置里当前选中的模型。全部失败时返回带可操作提示的错误。
pub(crate) fn resolve_model(
    spec: Option<&str>,
    selected_id: Option<&str>,
    catalog: &[ModelInfo],
    models_dir: &Path,
) -> Result<(PathBuf, String), String> {
    /// 由目录条目推导 models 目录下的路径。
    fn from_catalog(info: &ModelInfo, models_dir: &Path) -> (PathBuf, String) {
        (models_dir.join(&info.filename), info.name.clone())
    }

    /// 文件缺失时的提示：告诉用户怎么补。
    fn missing(path: &Path) -> String {
        format!(
            "模型文件不存在: {}\n提示：先用图形界面「选择模型」下载，或改用 --model 指定 .gguf 路径",
            path.display()
        )
    }

    /// 列出可用的 id，方便用户照着填。
    fn available_ids(catalog: &[ModelInfo]) -> String {
        catalog
            .iter()
            .map(|m| m.id.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }

    if let Some(spec) = spec {
        // 1) 当成路径（绝对/相对都试）
        let as_path = PathBuf::from(spec);
        if as_path.is_file() {
            let name = as_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| spec.to_string());
            return Ok((as_path, name));
        }
        // 2) 当成 models 目录内的文件名
        let in_models = models_dir.join(spec);
        if in_models.is_file() {
            let name = as_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| spec.to_string());
            return Ok((in_models, name));
        }
        // 3) 当成目录 id
        if let Some(info) = catalog.iter().find(|m| m.id == spec) {
            let (path, name) = from_catalog(info, models_dir);
            if path.is_file() {
                return Ok((path, name));
            }
            return Err(missing(&path));
        }
        return Err(format!(
            "无法识别的模型: {}\n可用 id: {}\n也可以用 --model 直接给 .gguf 路径",
            spec,
            available_ids(catalog)
        ));
    }

    // 4) 回落到配置里当前选中的模型
    let Some(id) = selected_id else {
        return Err(format!(
            "还没有选择过模型。用 --model 指定可用 id（{}），或先打开图形界面选一次",
            available_ids(catalog)
        ));
    };
    let Some(info) = catalog.iter().find(|m| m.id == id) else {
        return Err(format!(
            "配置中的模型 id 已不在目录里: {}\n可用 id: {}",
            id,
            available_ids(catalog)
        ));
    };
    let (path, name) = from_catalog(info, models_dir);
    if !path.is_file() {
        return Err(missing(&path));
    }
    Ok((path, name))
}

/// `n_threads` 配置解析：`auto` 用 CPU 物理并行度，其余按整数解析。
fn resolve_threads(cfg: &Config) -> u32 {
    if cfg.n_threads == "auto" {
        std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(4)
    } else {
        cfg.n_threads.parse().unwrap_or(4)
    }
}

/// `--serve` 主流程：装配并启动 API 服务，然后常驻直到进程被终止。
pub(crate) fn run_serve(opts: ServeOptions) -> Result<(), Box<dyn std::error::Error>> {
    // GUI 子系统进程默认没有控制台；挂到父进程控制台后 println 才有输出。
    crate::platform::console::attach_parent_console();

    // 与 GUI 共用日志管道：stderr + 数据目录下的每日滚动文件。
    let _log_guard = crate::logging::init();

    let cfg = config::load_config();
    let (model_path, model_name) = resolve_model(
        opts.model.as_deref(),
        cfg.selected_model_id.as_deref(),
        &cfg.model_catalog,
        &config::models_dir(),
    )?;

    let port = opts.port.unwrap_or(cfg.api_port);
    let token = opts.token.unwrap_or_else(|| cfg.api_token.clone());
    let n_ctx = opts.n_ctx.unwrap_or(cfg.n_ctx);
    let gpu_layers = opts.gpu_layers.unwrap_or(cfg.gpu_layers);
    let n_threads = resolve_threads(&cfg);

    println!("载入模型 {}（{}）...", model_name, model_path.display());
    log::info!(
        "serve: loading {} (n_ctx={}, n_threads={}, gpu_layers={})",
        model_path.display(),
        n_ctx,
        n_threads,
        gpu_layers
    );

    let inference =
        LlamaInference::load_ex(&model_path.to_string_lossy(), n_ctx, n_threads, gpu_layers)?;
    let inference = Arc::new(Mutex::new(inference));

    let mut server = ApiServer::start(Some(inference), port, model_name.clone(), token.clone());
    let Some(bound_port) = server.port() else {
        return Err(format!(
            "API 端口 {} 绑定失败：可能被其它程序占用（例如 Ollama），换一个 --port 再试",
            port
        )
        .into());
    };

    let info_path = config::data_root().join("serve.info");
    let info = serde_json::json!({
        "pid": std::process::id(),
        "port": bound_port,
        "token": token,
        "model": model_name,
        "model_path": model_path.to_string_lossy(),
    });
    if let Err(e) = std::fs::write(&info_path, format!("{}\n", info)) {
        log::warn!("write serve.info failed: {}", e);
    }

    println!();
    println!("桌面AI 服务已启动");
    println!("  模型: {}", model_name);
    println!("  API:  http://127.0.0.1:{}/v1", bound_port);
    println!("  Token: {}", token);
    println!("  信息文件: {}", info_path.display());
    println!("按 Ctrl+C 退出。");
    log::info!("serve: api server on 127.0.0.1:{}", bound_port);

    // 常驻：ApiServer 一旦被丢弃就会置停止标志并退出监听线程。
    loop {
        let _ = &mut server;
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn empty_args_start_gui() {
        assert_eq!(parse_cli(&s(&[])), Ok(CliCommand::Gui));
        assert!(parse_cli(&s(&["desktop-ai"])).is_ok());
    }

    #[test]
    fn serve_subcommand_and_flag_are_equivalent() {
        let a = parse_cli(&s(&["serve"])).unwrap();
        let b = parse_cli(&s(&["--serve"])).unwrap();
        assert_eq!(a, CliCommand::Serve(ServeOptions::default()));
        assert_eq!(b, a);
    }

    #[test]
    fn serve_parses_values_in_both_forms() {
        let cmd = parse_cli(&s(&[
            "serve",
            "--model",
            "qwen2.5-0.5b",
            "--port=8321",
            "--token",
            "da-test",
            "--n-ctx",
            "4096",
            "--gpu-layers",
            "99",
        ]))
        .unwrap();
        let CliCommand::Serve(opts) = cmd else {
            panic!("expected serve")
        };
        assert_eq!(opts.model.as_deref(), Some("qwen2.5-0.5b"));
        assert_eq!(opts.port, Some(8321));
        assert_eq!(opts.token.as_deref(), Some("da-test"));
        assert_eq!(opts.n_ctx, Some(4096));
        assert_eq!(opts.gpu_layers, Some(99));
    }

    #[test]
    fn unknown_or_incomplete_options_error() {
        assert!(parse_cli(&s(&["serve", "--nope"])).is_err());
        assert!(parse_cli(&s(&["serve", "--port"])).is_err());
        assert!(parse_cli(&s(&["serve", "--port", "abc"])).is_err());
        assert!(parse_cli(&s(&["--serve=1"])).is_err());
        assert!(parse_cli(&s(&["--wat"])).is_err());
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse_cli(&s(&["--help"])).unwrap(), CliCommand::Help);
        assert_eq!(parse_cli(&s(&["-h"])).unwrap(), CliCommand::Help);
        assert_eq!(parse_cli(&s(&["--version"])).unwrap(), CliCommand::Version);
        // 子命令之后出现 --help 也算帮助
        assert_eq!(
            parse_cli(&s(&["serve", "--help"])).unwrap(),
            CliCommand::Help
        );
    }

    fn catalog_fixture() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "qwen2.5-0.5b".into(),
                name: "Qwen2.5-0.5B".into(),
                desc: String::new(),
                size_gb: 0.35,
                tags: vec![],
                url: "https://example.com/a.gguf".into(),
                filename: "a.gguf".into(),
                expected_sha256: None,
                parts: vec![],
            },
            ModelInfo {
                id: "qwen3-8b".into(),
                name: "Qwen3-8B".into(),
                desc: String::new(),
                size_gb: 4.7,
                tags: vec![],
                url: "https://example.com/b.gguf".into(),
                filename: "b.gguf".into(),
                expected_sha256: None,
                parts: vec![],
            },
        ]
    }

    #[test]
    fn resolve_model_precedence() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.gguf"), b"x").unwrap();
        let catalog = catalog_fixture();

        // 目录 id → models 目录下文件
        let (path, name) = resolve_model(Some("qwen2.5-0.5b"), None, &catalog, dir.path()).unwrap();
        assert_eq!(path, dir.path().join("a.gguf"));
        assert_eq!(name, "Qwen2.5-0.5B");

        // models 目录内文件名
        let (path, name) = resolve_model(Some("a.gguf"), None, &catalog, dir.path()).unwrap();
        assert_eq!(path, dir.path().join("a.gguf"));
        assert_eq!(name, "a");

        // 回落到配置里选中的模型
        let (path, _) = resolve_model(None, Some("qwen2.5-0.5b"), &catalog, dir.path()).unwrap();
        assert_eq!(path, dir.path().join("a.gguf"));
    }

    #[test]
    fn resolve_model_reports_actionable_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let catalog = catalog_fixture();

        // 未下载
        let err = resolve_model(Some("qwen3-8b"), None, &catalog, dir.path()).unwrap_err();
        assert!(err.contains("模型文件不存在"), "{}", err);

        // 未知 id，且提示可用 id
        let err = resolve_model(Some("nope"), None, &catalog, dir.path()).unwrap_err();
        assert!(
            err.contains("无法识别的模型") && err.contains("qwen2.5-0.5b"),
            "{}",
            err
        );

        // 没有任何选择
        let err = resolve_model(None, None, &catalog, dir.path()).unwrap_err();
        assert!(err.contains("还没有选择过模型"), "{}", err);

        // 配置里的 id 过期
        let err = resolve_model(None, Some("gone"), &catalog, dir.path()).unwrap_err();
        assert!(err.contains("已不在目录里"), "{}", err);
    }

    #[test]
    fn resolve_model_accepts_explicit_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("custom-model.gguf");
        std::fs::write(&file, b"x").unwrap();
        let (path, name) =
            resolve_model(Some(&file.to_string_lossy()), None, &[], dir.path()).unwrap();
        assert_eq!(path, file);
        assert_eq!(name, "custom-model");
    }
}
