#![no_main]
//! fuzz `resolve_url`：绝对 / 协议相对 / 根相对 / 相对四分支 URL 解析。
//! 锁定：不 panic；绝对 http(s) 链接必须原样返回。
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    link: String,
    base: String,
}

fuzz_target!(|input: Input| {
    let resolved = desktop_ai::fuzz_api::resolve_url(&input.link, &input.base);
    if input.link.starts_with("http://") || input.link.starts_with("https://") {
        assert_eq!(resolved, input.link, "absolute link must round-trip");
    }
});
