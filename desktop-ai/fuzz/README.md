# fuzz - cargo-fuzz 模糊测试

本目录是对 desktop-ai 中纯解析函数做深度模糊测试的独立 cargo-fuzz 子项目。

## 项目定位

- 为什么存在：解析类代码的输入空间无法穷举，人工用例只能覆盖已知形态。这里用覆盖率引导的
  fuzzer 长时间探索清洗、分块、URL / HTML / robots / HTTP 解析这些无 IO、无副作用的纯函数，
  目标是把 panic、越界、死循环逼出来。
- 与 `src/redteam.rs` 的关系：`redteam.rs` 是稳定侧的守门员，用固定的内置种子语料做确定性
  回归，每个用例对应一次真实的攻击尝试，通过即防护生效、失败即找到绕过路径。本目录则是持续
  深挖：跑出新的崩溃后，把它沉淀为 `redteam.rs` 中的一条确定性用例。两者互补，前者快而确定，
  后者慢而广阔。

## 目录形态

- 本目录是独立的 cargo 工作区：`Cargo.toml` 末尾自带一个空的 `[workspace]`，使 `fuzz/` 不被
  并入主 crate 的成员集合。
- 主 crate 在 `desktop-ai/` 下执行 `cargo build` / `cargo test` 以及 CI 时不会编译本目录；
  反过来本目录的构建也不影响主 crate，两者互不干扰。
- `host/`：desktop-ai 的 rlib-only 垫片。cargo-fuzz 用 `-Zsanitizer=fuzzer,address` 编译
  整个依赖图，并要求每个链接产物都携带 libFuzzer 的 `main` 符号；主 crate 因 Android 交付
  需要 `crate-type = ["rlib", "cdylib"]`，其中 cdylib 没有 `main`，链接必失败
  （LNK2001）。垫片以 `desktop-ai-fuzz-host` 包名编译同一份 `../../src/lib.rs`，对外仍
  是 `desktop_ai`，但只产出 rlib。**主包 `[dependencies]` 增删时必须同步
  `host/Cargo.toml`**——不同步会在 fuzz 构建时显性报错（E0432/E0433），不会静默。

## 前置条件

```bash
rustup toolchain install nightly
cargo install cargo-fuzz --locked
```

### Windows：AddressSanitizer 运行时

默认构建启用 ASan，产物动态链接 `clang_rt.asan_dynamic-x86_64.dll`。该 DLL 随
Visual Studio 的 "C++ AddressSanitizer" 组件发布，不在 exe 旁边，直接运行会报
`STATUS_DLL_NOT_FOUND (0xc0000135)`。运行前拷一次即可：

```powershell
$dll = Get-ChildItem "C:\Program Files\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\clang_rt.asan_dynamic-x86_64.dll" |
  Select-Object -First 1
Copy-Item $dll.FullName "target\x86_64-pc-windows-msvc\release\" -Force
```

## 模糊测试目标

| 名称 | 被测函数 | 锁定的不变式 |
| --- | --- | --- |
| chunk_text | `desktop_ai::chunk_text`（chunk_size / overlap 参数） | 不 panic；每个输出块的字符数 <= chunk_size |
| clean_text | `desktop_ai::clean_text`（raw + format: html / markdown / md / text / 未知） | 不 panic |
| ssrf_url | `desktop_ai::fuzz_api::is_ssrf_url` | 不 panic；内网地址形态（127.0.0.1、localhost、[::1]、十进制 2130706433 等）恒判私网 |
| extract_links | `desktop_ai::fuzz_api::extract_links` | 不 panic；提取链接数 <= 50 |
| resolve_url | `desktop_ai::fuzz_api::resolve_url` | 不 panic；绝对 http(s) 链接原样返回 |
| ddg_html | `desktop_ai::fuzz_api::parse_ddg_html` | 不 panic；成功时结果 <= 5 条 |
| http_parser | `desktop_ai::fuzz_api::parse_http_validated` | 不 panic（本机 API 服务器手写 HTTP 解析器） |
| robots_rules | `desktop_ai::fuzz_api::robots_is_allowed` | 不 panic；空 robots.txt 一律放行 |

## 使用方法

```bash
cargo +nightly fuzz build                          # 首次编译较慢
cargo +nightly fuzz run <target> -- -runs=10000    # 冒烟：跑 1 万次
cargo +nightly fuzz run <target>                   # 持续运行，Ctrl-C 停止
```

- 语料自动写入 `fuzz/corpus/<target>/`，已被仓库根 `.gitignore` 忽略。

## 崩溃处理

- 触发失败的输入落在 `fuzz/artifacts/<target>/`。
- 用 `cargo +nightly fuzz fmt <artifact 路径>` 复现并查看具体输入。
- 修复后，在 `src/redteam.rs` 补一条确定性回归用例，再重跑 fuzz 确认。

## 新增 target

1. 在 `fuzz_targets/` 新建 `<name>.rs`。
2. 在 `fuzz/Cargo.toml` 增加对应的 `[[bin]]` 条目。
3. 若被测函数不是 `pub`，先在 `src/lib.rs` 的 `fuzz_api` 模块（`#[cfg(fuzzing)]` 门控）加一个薄包装。
