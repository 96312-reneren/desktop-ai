#![no_main]
//! fuzz `parse_ddg_html`：DuckDuckGo 结果页解析器对抗意 HTML
//! （标记截断、属性泄漏防护）。锁定：不 panic、成功时结果 ≤ 5 条。
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    if let Ok(results) = desktop_ai::fuzz_api::parse_ddg_html(&html) {
        assert!(results.len() <= 5, "result cap violated: {}", results.len());
    }
});
