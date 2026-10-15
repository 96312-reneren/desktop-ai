#![no_main]
//! fuzz robots.txt 解析 + RFC 9309 匹配（最长规则胜、Allow 平局胜、
//! `*` 通配、`$` 锚定、注释剥离）。锁定：任意文本不 panic，且
//! 空 robots.txt 一律放行。
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    text: String,
    path: String,
}

fuzz_target!(|input: Input| {
    let _allowed = desktop_ai::fuzz_api::robots_is_allowed(&input.text, &input.path);

    // 锚点：空 robots.txt 不构成任何限制
    assert!(desktop_ai::fuzz_api::robots_is_allowed("", "/anything"));
});
