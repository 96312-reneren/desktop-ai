#![no_main]
//! fuzz SSRF 黑名单谓词 `is_ssrf_url`：对任意 URL 字符串不 panic，
//! 并锚定核心内网形态恒判私网（防止覆盖率漂移悄悄翻转判定——
//! 该谓词是全部网络出口的第一道安全边界）。
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let url = String::from_utf8_lossy(data);
    let _ = desktop_ai::fuzz_api::is_ssrf_url(&url);

    assert!(desktop_ai::fuzz_api::is_ssrf_url("http://127.0.0.1/"));
    assert!(desktop_ai::fuzz_api::is_ssrf_url("http://localhost/admin"));
    assert!(desktop_ai::fuzz_api::is_ssrf_url("http://[::1]/"));
    assert!(desktop_ai::fuzz_api::is_ssrf_url("http://2130706433/"));
    assert!(!desktop_ai::fuzz_api::is_ssrf_url("http://example.com/"));
});
