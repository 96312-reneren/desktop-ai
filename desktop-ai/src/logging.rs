//! 日志初始化：stderr + 数据目录下的每日滚动文件。
//!
//! GUI 与 `--serve` 共用同一条日志管道；`tracing_subscriber::fmt().init()`
//! 全进程只能调用一次，因此两个入口都必须经由此函数，且各自持有返回的
//! guard（丢弃 guard 会截断文件写入）。

use tracing_subscriber::fmt::writer::MakeWriterExt;

/// 初始化日志并返回后台写线程的 guard；调用方须持有它直到进程退出。
pub(crate) fn init() -> tracing_appender::non_blocking::WorkerGuard {
    let log_dir = crate::store::config::log_dir();
    let file_appender = tracing_appender::rolling::daily(&log_dir, "desktop-ai.log");
    let (file_writer, log_guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_writer(std::io::stderr.and(file_writer))
        .init();
    log_guard
}
