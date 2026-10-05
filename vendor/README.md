# vendor/ — 第三方运行时二进制清单

这些是 llama.cpp 的预编译运行时库。`desktop-ai/build.rs` 会把当前平台的库
拷贝到 `target/<profile>/`，运行时由 `ffi.rs` 动态加载。

**升级流程**：替换文件 → 更新本文件的 SHA-256 → 删除
`<data_root>/llama_library.sha256`（旧哈希基线），否则 ffi 层会报告
"MISMATCH"（这是有意的篡改检测）。

## vendor/windows/ — llama.cpp b7700 (Windows x86_64, MinGW 构建)

| 文件 | 字节数 | SHA-256 |
|------|--------|---------|
| `llama.dll` | 3762631 | `a639088f62fd01cc38b2ea29fc6faf55178749c8aa11a97fd4597a7455e104c2` |
| `ggml.dll` | 156082 | `dbf18c1bf35107a6491634b5a481c5d10bee8cc932160b43bf0fdf7c9908912e` |
| `ggml-base.dll` | 1061610 | `6db250c7317ff40492045ee71d00c5dd17db973512013e1340329788599d4a36` |
| `ggml-cpu.dll` | 1513594 | `c1dea7ec192b762bdb86dffb7c2dec6ed88faeda462d495b436ea6bb360c0c9b` |
| `libgcc_s_seh-1.dll` | 151732 | `caaaf78b238f4239e2e6ee1a6e96750c0c234a2ad8cfcbd4f6e7dc212329b18f` |
| `libstdc++-6.dll` | 2665504 | `860291d2ea6be6889731ec1b439c2c652bf8de002ffcee5e10ada6c50849d211` |
| `libwinpthread-1.dll` | 65955 | `6a661d5846d80a91394dbb9b2dab87ba3cc705eac80ad45fe73677bff70cd6d2` |
| `libgomp-1.dll` | 330359 | `94619d2ab423b978768223f3bb07e261627881022b66f4f7ec5d6a8da8224b44` |

依赖关系（实测导入表）：`llama.dll` 硬依赖 `ggml.dll`、`ggml-base.dll`、
`libstdc++-6.dll`、`libgcc_s_seh-1.dll`；`ggml-cpu.dll` 由 ggml 侧动态加载。
**发布打包必须整体携带这 8 个文件**，缺一即模型功能不可用。

## vendor/linux/ — 由 CI 现场构建（不入库）

`release.yml` 的 Linux job 从 llama.cpp `b7700` 源码编译并写入
`vendor/linux/`，随后打入发布包。本地 Linux 开发者如需 `cargo build`
可直接把自编译的 `.so` 放到该目录（build.rs 会自动拷贝）。
