# 桌面AI — 可维护性与文件架构评审

评审对象：`E:\AI可视化\桌面AI`（main 分支，版本 6.1.6，75 次提交）
评审视角：一个新人接手这个仓库后，会在哪里卡住、会在哪里踩坑。

---

## 一句话判断

**代码本体是健康的，病灶不在代码里，而在仓库管理。**

373 KB 的 Rust 源码被组织成 6 个领域模块、有 31 处条件编译门支撑三平台、有 CI、有 14 个测试模块——这套底子不算差。真正的问题是这个 git 仓库被当成了网盘用：335 MB 的仓库里 99.9% 是历史垃圾，同时三套产品的产物、三份互相矛盾的文档、两个版权人不同的 LICENSE 挤在同一个仓库根。

代码层面最实在的欠账只有一处：主界面文件 1827 行、主结构体 44 个字段，而且**已经拆了一半又停下了**。

按"改动收益 ÷ 改动成本"排序如下。

---

## P0 · 仓库体积：改起来最快，收益最大

仓库 335 MB，源码 373 KB，占比 0.1%。`.git` 目录自己就占 318 MB。

原因不在当前文件，而在历史。查出历史上最大的一批对象：

| 文件 | 单份体积 | 进入历史次数 |
|---|---|---|
| `release/linux/desktop-ai` | 18.8–19.3 MB | 6 次 |
| `release/portable/DesktopAI-Portable.zip` | 16.6–19.0 MB | 3 次 |
| `android/build/lib/arm64-v8a/libllama.so` | 29.2 MB | — |
| `android/build/DesktopAI-v6.1.4.apk` | 15.6 MB | — |
| `android/build/lib/arm64-v8a/libdesktop_ai.so` | 11.9 MB | — |

现在的 `.gitignore` 里已经补上了 `release/`（第 48 行）和 `android/build/`（第 39 行），工作区也确实干净了——但**从工作区删掉和从历史里删掉是两件事**。二进制内容不会重复压缩，所以那 6 份可执行文件各占一份完整体积。

一个佐证：`.git` 里有 852 个松散对象、0 个 pack。说明这个仓库从未执行过 `git gc`，连基本的打包回收都没做过。

代价很直接：任何人 clone 一次要拖 300 多 MB，而项目对外宣称的是"12 MB 绿色免安装"。这两件事放在一起看，会让人觉得作者不太在意仓库卫生。

**建议**：用 `git filter-repo` 或 BFG 重写历史剔除 `release/`、`android/build/`、`*.apk`、`*.zip`，然后 `git gc --aggressive --prune=now`。注意这会改写所有 commit id，需要所有协作者重新 clone。

---

## P1 · 12 个预编译二进制躺在源码目录里

Windows 的 6 个 `.dll` 和 Linux 的 4 个 `.so`（外加 2 个重复计数）直接放在 `desktop-ai/` 根下，和 `Cargo.toml`、`build.rs` 平级。

问题有三层：

1. **没有归档结构**。两个平台的二进制混在同一个目录，没有 `vendor/windows/`、`vendor/linux/` 这类区分。
2. **没有版本清单**。这是个 1.1–3.6 MB 的第三方推理运行时（llama.cpp 的产物），但仓库里没有任何文件记录它的版本号和 SHA-256。构建来源只写在某条提交信息和 `release.yml` 的注释里（`llama.cpp b7700`）。要升级或审计时，得靠考古。
3. **入库的 Linux `.so` 对 CI 是死重**。`release.yml` 在 Linux 构建时会**重新从 llama.cpp b7700 编译** `libllama.so`，然后 `cp -f build/bin/libllama.so.0 ../libllama.so` 覆盖过去。也就是说 CI 根本不读入库的那份。入库的唯一目的是让本地 Linux 开发者能直接 `cargo build`——而这恰恰制造了漂移风险：本地那份和 CI 编出来的那份没有任何机制保证一致。

`.gitignore` 里忽略的是 `*.lib` / `*.exe` / `*.pdb`，**没有忽略 `*.dll` 和 `*.so`**，所以它们被正常跟踪着。

**建议**：移到 `vendor/{windows,linux}/`，新增一份 `BINARIES.md` 记录版本 + SHA-256 + 下载来源，加一个校验脚本进 CI（`SECURITY_REVIEW.md` 里已经做过 DLL SHA-256 审计，把结论固化下来即可）。

---

## P1+ · Windows 发布包缺少依赖库，实际不可用（实测发现）

这是本次评审最严重的发现，比体积问题更影响项目本身的价值。

`release.yml` 的 Windows job 只往打包目录拷两个文件：

```pwsh
Copy-Item target/release/desktop-ai.exe staging/桌面AI.exe
Copy-Item llama.dll staging/llama.dll
Compress-Archive -Path staging/* -DestinationPath 桌面AI-v$ver-windows-x86_64.zip -Force
```

而对 `llama.dll` 的导入表做二进制字符串检查，结果是：

| 被依赖的库 | 是否存在 |
|---|---|
| `ggml.dll` | ✅ 依赖 |
| `ggml-base.dll` | ✅ 依赖 |
| `libstdc++-6.dll` | ✅ 依赖 |
| `libgcc_s_seh-1.dll` | ✅ 依赖 |
| `ggml-cpu.dll` | 未在导入表出现（由 ggml 侧动态加载） |

而 `build.rs` 里登记需要拷贝的运行时库一共 **8 个**——`release.yml` 只搬了其中 1 个。

也就是说：在干净机器上解压这个 zip，`llama.dll` 会因为找不到 `ggml-base.dll` 而加载失败。`ffi.rs` 的 `resolve_lib_path()` 是在 exe 同目录找 `llama.dll`，找到了也会在 `dlopen` 阶段因依赖缺失而报错，模型功能直接不可用。

这条结论和"项目能跑"的认知是矛盾的，最可能的解释是：**历次对外发布其实是手工打的包，而不是走这个 workflow**。这也解释了为什么历史里会出现 `release/portable/DesktopAI-Portable.zip`（16–19 MB，里面装了全套 DLL）——那是人工组装的结果。

但无论如何，**按当前提交的 workflow 跑出来的 Windows 包是不可用的**。这是个应该在重建仓库时顺手修掉的坑。

修法（假设二进制已迁到 `vendor/windows/`）：

```pwsh
$ver = "${{ github.ref_name }}" -replace '^v',''
New-Item -ItemType Directory -Force -Path staging | Out-Null
Copy-Item target/release/desktop-ai.exe staging/桌面AI.exe
Copy-Item vendor/windows/*.dll staging/
Compress-Archive -Path staging/* -DestinationPath 桌面AI-v$ver-windows-x86_64.zip -Force
```

**验收方式**：在干净目录（最好是一台没装过开发工具的机器或全新虚拟机）解压产物，双击运行，确认能加载模型。仅在本机测试是测不出来的——本机 `PATH` 或系统目录里可能恰好有这些 DLL。

---

## P2 · `lib.rs` 的门面把"领域分层"架空了

上一次重构（提交 `be935f8` "restructure src into domain modules"）把 `src/` 拆成了六个领域目录，方向完全正确。但紧接着的 `aef1981` 在 `lib.rs` 里加了一段扁平别名：

```rust
pub(crate) use store::{config, conversation, db};
pub(crate) use rag::{chunker, cleaner, crawler, search, vector_store};
// ...共 5 行
```

后果可以量化：

| 引用方式 | 出现次数 |
|---|---|
| 新的领域路径 `crate::store::config::...` | **1 次** |
| 旧的扁平路径 `crate::config::...` | **73 次** |

目录分好了，调用方却全走回头路。实际代码里写的是 `crate::vector_store::SearchHit`、`crate::conversation::ConversationMeta`——新人打开一个文件，看到 `crate::vector_store`，无法判断它到底住在 `rag/` 还是顶层。**领域结构在文件系统上成立，在代码里不成立。**

**建议**：删掉 `lib.rs` 里那 5 行 `use`，全量替换成领域路径。这是纯机械改动，但能让那次重构真正生效。

---

## P2 · 单 crate 扛三平台，且 CI 只测其中一平台

全仓库有 **31 处** `cfg(target_os = "android")` / `cfg(windows)` / `cfg(not(windows))` 散落在 14 个文件里。

最值得说的一处，是 `Cargo.toml` 自己承认的：

```toml
# On Android the app uses a WebView UI + JNI service entry instead,
# but eframe stays in the dependency graph so the UI modules compile
# on the Android target too.
eframe = { version = "0.31", features = ["android-native-activity"] }
```

翻译过来：安卓端用 WebView 做界面，桌面 GUI 框架根本用不上，但**为了让它能编译过，硬留在依赖图里**。这是被架构逼出来的妥协——为了让一个模块"能编译"，付出了让安卓包背一整套桌面 GUI 栈的代价。正确做法是把 `ui` 模块整体 cfg 掉。

而 CI 的覆盖情况更值得注意：

- `ci.yml` 只跑 `windows-latest`，没有任何 Linux job，也没有 Android job。它做 `fmt` / `test` / `clippy -D warnings` / `cargo audit`。
- `release.yml` 才构建 Linux，而且只在**打 tag 或推 main** 时触发。
- **Android 从头到尾没有 CI**。

结果是：`android/` 那套"零 Gradle"的 Python + bat 构建链一旦某处坏掉，没有任何自动检测会发现，只能靠人工试。顺带一提，`ci.yml` 里写的是 `cargo test --workspace`，但这个项目根本没有定义 `[workspace]`——这行是复制来的，留着也是个提醒信号。

**建议**：`ci.yml` 加一个 `ubuntu-latest` job 至少跑 `cargo check`；`ui` 模块加 `cfg(not(target_os = "android"))`；clippy 加 `--all-targets`（现在测试代码没被 lint）。

### 附带发现：Android 构建链依赖机器临时目录

`android/build-apk.bat` 里有两行硬编码，指向**本机 Temp 目录**：

```bat
set "NDKOUT=C:\Users\TheUn\AppData\Local\Temp\opencode\androidout\jni\arm64-v8a"
set "LLAMA=C:\Users\TheUn\AppData\Local\Temp\opencode\androidout"
```

意思是：安卓构建需要的 `libdesktop_ai.so` 和 4 个 llama 库，取自一个**会被系统清理的临时目录**。再加上 `JAVA_HOME`、`KOTLINC`、`BT`（build-tools 34.0.0）、`PLATFORM` 也全是本机绝对路径，这个构建链在任何其它机器上都跑不起来。加上 Android 没有任何 CI，这套构建只能算"在作者机器上能跑"。

**这同时证明了一件事**：历史里那个 29 MB 的 `android/build/lib/arm64-v8a/libllama.so` 和 15 MB 的 APK，跟构建流程毫无关系——构建脚本每次运行都会先 `rmdir /s /q "%OUT%"` 再重建，而且输入取自 Temp 目录。它们纯粹是遗留垃圾，删除零风险。

另外一个小的版本漂移：APK 文件名 `DesktopAI-v6.1.6.apk` 是硬编码在 bat 里的，和 `Cargo.toml` 的 `version` 各改各的。

**建议**：把路径提成脚本顶部的一组变量（或支持环境变量覆盖），在 `android/README.md` 里写清前置条件；版本号改为读 `Cargo.toml`。

---

## P3 · 主界面文件 1827 行——拆到一半停下了

`desktop-ai/src/ui/app/mod.rs` 是最大的文件（1827 行，占全部源码 17%）。里面同时住着五类不相关的东西：

| 内容 | 行范围 | 说明 |
|---|---|---|
| 主题函数 `apply_theme` | 25–60 | 纯样式 |
| 硬件探测 | 467–560 | 用 `wmic` 查 GPU、调 Win32 API 查内存 |
| 知识库后台任务流水线 | 259–466 | `run_kb_job` / `index_content` / `finish_kb_job` |
| `struct DesktopAI` | 170–228 | **44 个字段** |
| `impl DesktopAI` | 562–1452 | **890 行** |
| `impl eframe::App` 的 `update()` | 1454–1827 | **374 行** |

关键判断是：**这不是"没拆"，是"拆了一半"**。

`chat.rs`(180) / `settings.rs`(327) / `sidebar.rs`(104) / `kb_panel.rs`(259) / `model_select.rs`(108) 都已经拆出去了，而且委托是通的——`update()` 里确实在调 `self.render_settings(ctx, ui)`、`self.render_kb_panel(ui)` 等，五个子模块各自持有 `impl DesktopAI` 块。这个模式没问题。

问题是拆分标准不一致，剩下的东西没归位：

- `mod.rs` 里还留着约 **60 行内联的"确认对话框" UI 代码**（删除模型 / 重置 / 卸载），它明显属于 `settings.rs` 那一类。
- 硬件探测放在 UI 模块里，但它和界面无关，属于 `platform`。
- 知识库任务流水线放在 `mod.rs`，但它属于 `rag` 领域的状态机。
- 44 个字段的结构体**没有按领域分组**，只靠注释分隔（`// Chat`、`// Downloads`、`// Knowledge base`……）。这等于把"分组"这件事交给读者的眼睛。

**建议**（按性价比）：① 确认对话框移入 `settings.rs`；② 硬件探测移入 `platform`；③ 44 个字段按 7 个领域拆成嵌套 struct，让 `// Chat` 这类注释变成真正的类型边界；④ `update()` 里每个面板分支压缩成一行委托调用。

---

## P3 · 文档三处副本，而且互相矛盾

这是"文件架构问题"里最容易被低估、但对项目可信度伤害最大的一块。

**（一）README 描述的分支模型是虚构的**

仓库根 `README.md` 写着：

> | `main` | 桌面版… |
> | `android` | 安卓混合架构版… |
> 安卓版从 `main` 分出…

实际情况：`android/` 目录**当前就在 main 分支里被跟踪**（`git ls-files android/` 有 8 个文件），而且 `android` 分支已经完全并入 main（`git log main..android` 为空，最后一次合并是 `978936f Merge branch 'android' into main`）。

也就是说文档描述的是"两个产品两条分支"，实际是"两个产品挤在一条 main 上"。**这份 README 描述的是一个不存在的架构。**

**（二）文档指向一个不存在的文件**

三处引用 `android_service.rs`，但该文件不存在（已改名为 `src/platform/android.rs`）：

- `README.md` 第 17 行：`desktop-ai/src/android_service.rs` — Rust 服务端 JNI 入口
- `desktop-ai/Cargo.toml` 第 12 行注释
- `desktop-ai/Cargo.toml` 第 41 行注释

（`SECURITY_REVIEW.md` 里也提了，且那份评审报告本身就记录过"安卓源码可能在分支/历史中或已丢失"——说明这个坑被记录过、但一直没修。）

**（三）两个 LICENSE，版权人不一样**

| 位置 | 版权声明 |
|---|---|
| `LICENSE`（根） | `Copyright (c) 2026 归去来兮` |
| `desktop-ai/LICENSE` | `Copyright (c) 2026 桌面AI` |

对一个对外发布的 MIT 项目，版权主体写得不一致是实打实的合规瑕疵——用户行使权利时该找谁，法理上是不清楚的。

**（四）安全评审文档有三份副本，内容各不相同**

| 位置 | 体积 |
|---|---|
| `SECURITY_REVIEW.md`（仓库根，受跟踪） | 13999 B |
| `.git/SECURITY_REVIEW.md` | 14161 B |
| `.git/project-evaluation-report.md` | 11463 B |

后两份躺在 `.git/` 目录内部。这是一个几乎不会被发现、也几乎一定会被误删的位置。考虑到提交 `ef29ccd` 的动作是"remove evaluation report from public repo"，推测是把要"下架"的文档随手挪进了 `.git/`——但 `.git/` 不是归档目录，它会在 `git gc` 或重建仓库时消失。

**建议**：README 改写成"单 main、含桌面与安卓两套"的真实模型；三处 `android_service.rs` 改成 `platform/android.rs`；两份 LICENSE 统一版权人；`.git/` 里那两份文档移出到 `docs/` 并纳入版本控制（或用子目录忽略），三份副本合并为一份。

---

## P3 · 测试摆放不统一

14 个生产文件里内联了 `#[cfg(test)] mod tests`。`api_server.rs` 1128 行里有接近 300 行是测试，`ffi.rs` 1075 行里约 200 行是测试。

这不影响运行，但影响阅读成本：想搞清楚 `api_server` 的公开行为，得先跳过大段测试。而 `tests/` 下只有孤零零一个 `integration.rs`(231 行)。

这一条不是错误，是风格选择——Rust 社区两种做法都有。但**同一仓库里要一致**：现在的状态是"单元测试内联、集成测试外置"，标准本身是清楚的，只是内联测试的体积已经大到妨碍导航了。

**建议**：把纯函数级测试（`constant_time_eq`、`parse_content_length`、`find_subslice` 这类无状态函数）迁到 `tests/`，保留涉及内部状态的测试内联。

---

## 值得肯定的部分

审这份代码时也要说清楚哪些是对的，避免把好设计一起否定掉：

- **六领域划分本身是干净的**。`llm` / `rag` / `store` / `server` / `ui` / `platform` 的边界合理，`lib.rs` 用 `pub(crate)` 收窄可见性、只 `pub use` 少量对外条目的思路是对的。
- **测试有真实覆盖**，而且测的是要害：FFI 结构体尺寸对齐（`llama_model_params_is_correctly_sized`）、Unicode 边界、CORS origin 绕过、常量时间比较、Content-Length 重复头。这些是安全相关的高价值测试，不是凑数的。
- **CI 里挂了 `cargo audit`**，并且对无解的两个 RUSTSEC 写了带理由的 `--ignore` 注释（说明为什么 local attack surface only）。这是成熟做法。
- **`build.rs` 的 `rerun-if-changed` 列全了**所有平台库，没有偷懒。
- **生产代码里 `unsafe` 集中在 `ffi.rs`**，边界清楚。
- 提交信息质量高（`security:` / `refactor:` / `fix:` 前缀 + 具体原因），75 次提交能看出演进脉络。

---

## 建议的执行顺序

| 优先级 | 动作 | 性质 | 备注 |
|---|---|---|---|
| P0 | 重写历史剔除构建产物 + `git gc` | 一次性 | 需协作者重新 clone |
| P1 | 二进制移到 `vendor/` + `BINARIES.md` + CI 校验 | 新增文件 | 无损 |
| P1 | 修 3 处 `android_service.rs` 引用、统一 LICENSE 版权人 | 改文字 | 无损，几分钟 |
| P2 | 删 `lib.rs` 扁平别名，全量改领域路径 | 机械替换 | 73 处，可脚本化 |
| P2 | CI 加 ubuntu job；`ui` 加 cfg 排除；clippy `--all-targets` | 改配置 | 无损 |
| P3 | `mod.rs` 确认对话框 / 硬件探测 / KB 流水线归位 | 重构 | 需回归测试 |
| P3 | 44 字段结构体按领域拆嵌套 struct | 重构 | 影响面广，最后做 |
| P3 | 合并三份安全评审副本到 `docs/` | 归档 | — |

---

## 附：完整问题清单

| # | 问题 | 位置 | 严重度 |
|---|---|---|---|
| 1 | `.git` 318 MB，含 6 份 19 MB 可执行文件 | `.git` 历史 | 致命 |
| 2 | 852 个松散对象、0 个 pack，从未 gc | `.git` | 高 |
| 3 | 12 个 `.dll`/`.so` 直接放在源码目录 | `desktop-ai/` 根 | 高 |
| 4 | 二进制无版本清单、无哈希记录 | `desktop-ai/` | 中 |
| 5 | 入库 Linux `.so` 被 `release.yml` 覆盖，存在漂移风险 | `release.yml` | 中 |
| 6 | `.gitignore` 未忽略 `*.dll` / `*.so` | `.gitignore` | 低 |
| 7 | `lib.rs` 扁平别名架空领域分层（73:1） | `src/lib.rs` | 高 |
| 8 | 31 处条件编译门散落 14 个文件 | `src/` | 中 |
| 9 | 安卓为迁就编译而保留 `eframe` 依赖 | `Cargo.toml` | 中 |
| 10 | CI 只跑 Windows；Linux 仅发版时构建；Android 无 CI | `.github/workflows/` | 高 |
| 11 | `cargo test --workspace` 但无 workspace 定义 | `ci.yml` | 低 |
| 12 | clippy 未加 `--all-targets`，测试代码未 lint | `ci.yml` | 低 |
| 13 | `ui/app/mod.rs` 1827 行，五类关注点混住 | `src/ui/app/` | 中 |
| 14 | `DesktopAI` 44 字段未分组 | `src/ui/app/mod.rs` | 中 |
| 15 | `update()` 374 行；60 行确认对话框内联 | `src/ui/app/mod.rs` | 中 |
| 16 | 硬件探测（`wmic`/Win32）放在 UI 模块 | `src/ui/app/mod.rs` | 中 |
| 17 | KB 任务流水线放在 UI 模块 | `src/ui/app/mod.rs` | 中 |
| 18 | README 分支模型与实际不符 | `README.md` | 高 |
| 19 | 3 处引用不存在的 `android_service.rs` | `README.md`, `Cargo.toml` | 中 |
| 20 | 两份 LICENSE 版权人不同 | `LICENSE`, `desktop-ai/LICENSE` | 高 |
| 21 | 3 份安全评审副本内容不一致，2 份藏在 `.git/` | `.git/`, 根目录 | 中 |
| 22 | 根 README 703 B 与 `desktop-ai/README.md` 18.6 KB 两套 | 根, `desktop-ai/` | 低 |
| 23 | 14 个生产文件内联测试模块；`tests/` 仅 1 个文件 | `src/`, `tests/` | 低 |
| 24 | `.gitignore` 首行带 UTF-8 BOM | `desktop-ai/.gitignore` | 低 |
| 25 | `android/` 构建链无文档（3 个 Python + 1 个 bat） | `android/` | 中 |
| 26 | **Windows 发布包漏拷 7 个运行时依赖 → 发布包不可用** | `release.yml` | 致命 |
| 27 | Android 构建链硬编码本机 Temp 路径与 SDK 绝对路径，不可复现 | `android/build-apk.bat` | 高 |
| 28 | APK 版本号硬编码在构建脚本，与 `Cargo.toml` 脱钩 | `android/build-apk.bat` | 低 |
| 29 | 历次 Windows 发布疑似手工打包，与 workflow 不一致 | `release.yml` + Release 附件 | 中 |

---

*评审基于 6.1.6 版本工作区与完整 git 历史（75 次提交）。所有结论均通过 `git ls-files` / `git cat-file --batch-check` / 源码定位核实，未执行任何修改操作。*

---
---

# 附录 B · 仓库重建行动方案（单人、无协作场景）

> 本附录回答一个问题：既然没人协作，能不能直接删库重建，只留源码和必要依赖？
> **答案是可以，而且对你这种情况，重建比重写历史更划算。**

---

## B0. 为什么"重建"适合你

清理一个被构建产物污染历史的仓库，只有两条路：

| 方案 | 优点 | 代价 | 适用场景 |
|---|---|---|---|
| `git filter-repo` / BFG 重写历史 | 保留 75 次提交演进；GitHub 上的 star / issue / Release / fork 全部保留 | 要装第三方工具；改写全部 commit id；操作失误可能损坏仓库；每个协作者都要重新 clone | 有协作、历史有参考价值 |
| **删库重建（orphan / fresh init）** | 10 分钟、零工具依赖、结果绝对干净、绝不残留 | 丢失提交历史；若同时删远程仓库，会丢 star / issue / Release | **单人、无协作** ← 你的情况 |

你已经明确"没人协作"，那么这 75 次提交对你就只剩考古价值，没有协作价值。更关键的是——**这个仓库的历史本身就是你要清理的那个东西**，保留它等于保留一半垃圾。

**结论：直接重建。**

重建前只需确认一件事：**这个 GitHub 仓库是不是你原创的？**

- **是你自己的** → 直接重建。只用记住会丢 star / issue / Release，重建前先记一下数量。
- **是从别人仓库 fork 或克隆来的** → **只重建你自己那一份**，并且必须保留 MIT LICENSE 原文与原作者署名。MIT 协议要求"在软件的所有副本中保留版权声明"，这不是可选项。你当前两份 LICENSE 署名不一致（根目录是 `Copyright (c) 2026 归去来兮`，`desktop-ai/LICENSE` 是 `Copyright (c) 2026 桌面AI`）——后者恰好把原作者署名抹掉了。重建正好是修正时机：合并为一份，原作者与你的署名都写上。

---

## B1. 先决定"留什么"

核心判据只有一条：**这份文件是我需要手工生成、丢了就得重新折腾的东西吗？**

### 必须保留

| 类别 | 文件 | 为什么不能丢 |
|---|---|---|
| 源码 | `desktop-ai/src/**`、`tests/`、`Cargo.toml`、`Cargo.lock`、`build.rs` | 项目本体 |
| 文档 | 根 `README.md`、`desktop-ai/README.md`、`LICENSE`、`SECURITY_REVIEW.md` | 均为孤本 |
| CI | `.github/workflows/ci.yml`、`release.yml` | 手工编写 |
| Android | `android/app/**`、`android/*.py`、`android/build-apk.bat` | 源码，保留（脚本本身有问题，见 B6） |
| 配置 | `.gitattributes`、`.gitignore` | 手工编写，行尾规则依赖它 |
| **Windows 运行时库 ×8** | `llama.dll`(3.6 MB)、`ggml.dll`、`ggml-base.dll`、`ggml-cpu.dll`、`libgcc_s_seh-1.dll`、`libstdc++-6.dll`、`libwinpthread-1.dll`、`libgomp-1.dll` | **绝对不能丢**：`build.rs` 从仓库根拷贝到 `target/`，`release.yml` 也直接从仓库根取。丢了本地无法 `cargo build`，也发不了版 |

### 可以丢弃

| 类别 | 文件 | 体积 | 为什么可以丢 |
|---|---|---|---|
| 发布产物 | `release/**` | ~110 MB | GitHub Release 才是它的家，不该进 git |
| Android 构建产物 | `android/build/**` | ~57 MB | **构建脚本每次先 `rmdir /s /q` 再重建，且输入取自别处**（见 P2 附带发现），删除零风险 |
| Linux 运行时库 ×4 | `libllama.so`、`libggml*.so.0` | ~4.8 MB | `release.yml` 在 Linux job 里**从 llama.cpp b7700 现场编译并覆盖**，根本不读入库的这份。你只在 Windows 开发，用不上 |
| 旧历史文档 | `.git/SECURITY_REVIEW.md`、`.git/project-evaluation-report.md` | 25 KB | 位置错误。有价值就先捞出来，别留在 `.git/` |
| 重复 LICENSE | 根 `LICENSE` 与 `desktop-ai/LICENSE` 二选一 | 1 KB | 两份署名互相矛盾，本身就是问题 |
| 编译中间件 | `desktop-ai/target/` | 数 GB | 纯缓存 |

**净结果**：仓库从 335 MB 降到 **约 10 MB**，其中 9.5 MB 是那 8 个必需的 Windows DLL，源码只有 373 KB。这才配得上 README 里"12 MB 绿色免安装"这句话。

---

## B2. 新 `.gitignore`（先写它，再放文件）

你的顺序判断是对的。正确次序是：**建空仓库 → 写 `.gitignore` → 再放文件 → 最后 `git add`**。这样任何遗漏都会在 `git status` 里立刻现形，而不是等 commit 进去之后再补救。

仓库根 `.gitignore` 全文（可直接使用）：

```gitignore
# ═══════════════════════════════════════════════
# 桌面AI — 仓库忽略规则
# 原则：只收源码、手工文档、CI 配置
#       一切"能重新生成的东西"都不进仓库
# ═══════════════════════════════════════════════

# ── Rust 编译产物 ──
target/
**/*.rs.bk
mutants.out*/

# ── 可执行文件与链接产物 ──
*.exe
*.pdb
*.lib
*.exp
*.ilk

# ── 第三方运行时库：默认全禁，仅放行 vendor/ 下登记过的 ──
*.dll
*.so
*.so.*
*.dylib
!vendor/**/*.dll
!vendor/**/*.so*

# ── 发布产物 ──
release/
dist/
staging/
*.apk
*.aab
*.jar
*.zip
*.tar
*.tar.gz
*.7z

# ── Android 构建链产物 ──
android/build/
android/.gradle/
android/local.properties
*.keystore
*.jks

# ── 模型与数据（体积大，绝不入库）──
*.gguf
*.bin
*.safetensors
*.onnx
*.db
*.db-wal
*.db-shm

# ── 日志 ──
*.log
logs/

# ── 私有文档 ──
*.docx
*.xlsx
*.pptx

# ── IDE ──
.vscode/
.idea/
*.swp
*.swo
.qoder/

# ── 操作系统 ──
Thumbs.db
Desktop.ini
$RECYCLE.BIN/
System Volume Information/

# ── 测试临时目录 ──
**/desktop_ai_*_test/
```

### 关于那两行否定规则

```gitignore
*.dll
!vendor/**/*.dll
```

Git 的否定规则要求父目录没被忽略，而 `vendor/` 不在忽略列表里，所以这行能生效。它的作用是：**任何位置的 DLL/SO 默认都不许入库，只有你亲自放进 `vendor/` 的例外**。这样下次你（或某个工具）往 `src/` 旁边丢个 DLL，`git status` 会直接忽略——从根上防止问题 3 复发。

这是与你当前 `.gitignore` 最本质的区别。现在的写法是"只禁 `*.lib`/`*.exe`/`*.pdb`"，**方向和意图是反的**：`*.dll` 和 `*.so` 处于放行状态，所以 12 个二进制才会入库。新规则改成"默认全禁 + 白名单"。

### 配套：二进制迁入 `vendor/`

推荐把 8 个 Windows DLL 移到 `vendor/windows/`，然后同步改两处引用：

- `desktop-ai/build.rs` 第 42 行：`join(lib_name)` → `join("vendor").join("windows").join(lib_name)`
- `release.yml` Windows job：`Copy-Item llama.dll` → `Copy-Item vendor/windows/*.dll`（同时修掉 B6 的漏拷问题）

**如果想零风险**：让 8 个 DLL 留在 `desktop-ai/` 根下不动，把那两行否定规则换成显式白名单即可：

```gitignore
!desktop-ai/llama.dll
!desktop-ai/ggml.dll
!desktop-ai/ggml-base.dll
!desktop-ai/ggml-cpu.dll
!desktop-ai/libgcc_s_seh-1.dll
!desktop-ai/libstdc++-6.dll
!desktop-ai/libwinpthread-1.dll
!desktop-ai/libgomp-1.dll
```

两种都行：前者结构干净、后者省事零风险。

---

## B3. 分步操作

### 第 0 步 · 备份（唯一不可跳过的一步）

```bash
cd /e/AI可视化
cp -r 桌面AI "桌面AI-backup-$(date +%Y%m%d)"
```

更专业的做法，打成一个可还原的单文件：

```bash
cd /e/AI可视化/桌面AI
git bundle create "../desktop-ai-history-$(date +%Y%m%d).bundle" --all
git bundle verify ../desktop-ai-history-*.bundle
```

`git bundle` 的好处：一个文件装下全部分支与 75 次提交。将来想考古，`git clone xxx.bundle` 就能完整还原。**做完这一步，后面怎么折腾都不怕。**

顺便把只存在于 `.git/` 里的两份文档捞出来（删库后必丢）：

```bash
mkdir -p /e/AI可视化/_keep
cp .git/SECURITY_REVIEW.md            /e/AI可视化/_keep/security-review-v2.md
cp .git/project-evaluation-report.md  /e/AI可视化/_keep/
```

### 第 1 步 · 在干净目录里组装新工作区

**不要原地删**——先在旁边搭好新的，确认能编译，再切换。原目录全程不动，随时可回退。

```bash
cd /e/AI可视化
mkdir -p 桌面AI-new
rsync -a --exclude='.git' --exclude='target' --exclude='release' \
      --exclude='android/build' --exclude='*.so' --exclude='*.so.*' \
      "桌面AI/" "./桌面AI-new/"
```

Git Bash 若没有 rsync，用复制 + 定向删除：

```bash
cd /e/AI可视化
cp -r 桌面AI 桌面AI-new
cd 桌面AI-new
rm -rf .git desktop-ai/target release android/build
rm -f desktop-ai/*.so desktop-ai/*.so.*
```

> 注意：`rm -rf` 只作用于你**刚复制出来的** `桌面AI-new`，**全程不要对原目录执行任何删除**。

### 第 2 步 · 建空仓库，先写 `.gitignore`

```bash
cd /e/AI可视化/桌面AI-new
git init -b main
```

把 B2 的 `.gitignore` 全文写入仓库根。**这一步必须在 `git add` 之前完成。**

### 第 3 步 · 盘点（本方案的关键，别跳过）

先不提交，只看到底哪些东西会被收进去：

```bash
git add -A
git status --short | wc -l                 # 总文件数，预期 60~70
git ls-files | head -80                     # 逐个过一遍

# 体积自检：任何单个文件超过 2 MB 都立刻停下来查
git ls-files -z | xargs -0 du -k | sort -rn | head -20
```

**预期结果**：最大的几个文件就是那 8 个 Windows DLL（`llama.dll` 3.6 MB 排第一），源码文件全在几十 KB 量级。

**如果这里出现了 19 MB 的可执行文件或 29 MB 的 `.so`，说明忽略规则没生效，回去补 `.gitignore`，不要继续往下走。**

### 第 4 步 · 确认体积后提交

```bash
du -sh .        # 预期 ~10 MB
du -sh .git     # 预期 < 1 MB

git commit -m "chore: rebuild repo — source + Windows runtime only

- 剔除历史中的 release/、android/build/、APK 与 Linux .so
- 新增 .gitignore：默认禁止 dll/so/exe/zip/apk，仅放行 vendor/
- 二进制迁入 vendor/windows/，新增 BINARIES.md 记录版本与 SHA-256
- 统一 LICENSE 署名，修正 android_service.rs 失效引用
- 修复 release.yml Windows 打包漏拷 ggml 依赖"

git rev-list --count HEAD    # 输出 1，这是预期结果
```

### 第 5 步 · 验证还能编译

```bash
cd desktop-ai
cargo build      # 首次会拉依赖，需要网络，几分钟
cargo test
ls target/debug/*.dll    # 应能看到 8 个 DLL 被 build.rs 拷过来
```

`build.rs` 会把 DLL 从仓库根拷到 `target/debug/`。跑起来之后确认 `target/debug/` 里有 `llama.dll` 和 `ggml*.dll`，就说明运行时依赖没漏。

### 第 6 步 · 接回远程

这里有两种做法，**做错会丢 star**：

**方式 A（推荐：保留 star / issue / Release）** — 不删远程仓库，直接强推覆盖：

```bash
git remote add origin git@github.com:<你的账号>/<仓库名>.git
git push --force origin main
git push origin --delete android      # 旧分支一并清掉
```

仓库页面、star、issue、Release 全都在，只有提交历史被换成那一个新的 commit。

**方式 B（彻底重建，会丢 star / issue / Release）** — 在 GitHub 上删掉仓库 → 新建同名空仓库（**不要勾选 Add README / .gitignore / License**）→ 再 push。

既然没人协作，**方式 A 更划算**：省掉重建仓库的麻烦，还不丢 star。除非你连历史都想彻底抹掉，才用方式 B。

---

## B4. 注意事项

**1. 备份必须是第一件事，而且要验证能还原。** `git bundle verify` 通过才算备份成功。重建的每一步都是"看起来没问题"的，直到你发现丢了某个只改过一次的文件。

**2. 不要在原目录上删。** 在旁边搭好新的、验证通过后再切换。原目录改名留档（如加 `-old` 后缀），确认一周没问题再处理——`rm -rf` 没有回收站。

**3. `.gitignore` 必须在第一次 `git add` 之前写好。** 已经 commit 进去的文件，事后加忽略规则**不会**自动移除（git 只忽略未跟踪文件）。这是最常见的返工原因——你现在的仓库就是这个活教训：`release/` 明明已经写进 `.gitignore` 了，历史里那 6 份 19 MB 文件一份没少。

**4. `*.dll` 的忽略方向要和意图一致。** 你现在只禁 `*.lib`/`*.exe`/`*.pdb`，把 `*.dll`/`*.so` 放行了，所以 12 个二进制才进得来。新规则要反过来写，别写反。

**5. `Cargo.lock` 一定要提交。** 这是二进制程序，锁定依赖版本才能保证三个月后还能编译出一样的东西。它在当前跟踪列表里，别顺手删掉。

**6. `.gitattributes` 必须保留。** 它定义了行尾规则（`*.bat` 强制 CRLF）。丢了之后 `build-apk.bat` 可能在 cmd 下因行尾问题抽风。

**7. Windows DLL 一个都不能少。** 已实测 `llama.dll` 硬依赖 `ggml.dll`、`ggml-base.dll`、`libstdc++-6.dll`、`libgcc_s_seh-1.dll`——少一个 `llama.dll` 就加载不起来，模型功能直接废掉。判断清单以 `build.rs` 为准（8 个），别凭印象删。

**8. 重建后到 GitHub 补 Release 附件。** 原 `release/` 里的 Linux 可执行文件和 Portable 压缩包会随删库消失。它们的正确归宿是 GitHub Release 页面（不占仓库体积，且有版本管理），不是 git 历史。

**9. 如果不是你原创的项目，署名必须保留。** MIT 不是"可以随便改署名"的协议。当前两份 LICENSE 一个写 `归去来兮`、一个写 `桌面AI`，重建正好合并修正：原作者与你自己都署名。

**10. `.git/` 不是文档仓库。** 那两份文档放在 `.git/` 里，`git gc` 或重建时必然消失，而且不会有人发现。要么移进 `docs/` 纳入版本管理，要么放到仓库外的归档目录，没有第三种选择。

**11. 重建只解决体积，不解决代码质量。** `lib.rs` 的 73 处扁平别名、1827 行的 `mod.rs`、CI 只测 Windows——这些不会因为换了新仓库而变好。**重建是"清地基"，不是"修房子"。** 建议顺序：先重建（10 分钟，降低后续所有操作的心理负担），再逐条改代码。

**12. 强推之前确认没有别人在克隆你的仓库。** 你说了没人协作，但只要有 fork 或有人在做二次开发，强推会让他们的仓库彻底对不上。哪怕只有一个 fork，也建议改用 `git filter-repo`。

---

## B5. 完成后的自检清单

新仓库 push 之后，逐条打勾：

```bash
# 1. 体积达标
du -sh .git                                   # 预期 < 1 MB
# 2. 只有一个 commit
git rev-list --count HEAD                     # 预期 1
# 3. 没有任何构建产物
git ls-files | grep -Ei "\.(exe|apk|aab|zip|pdb|lib|exp)$" | wc -l    # 预期 0
# 4. 二进制只剩那 8 个
git ls-files | grep -Ei "\.(dll|so)$"
# 5. 最大文件不到 4 MB
git ls-files -z | xargs -0 du -k | sort -rn | head -3
# 6. 没有该忽略却仍被跟踪的文件
git status --ignored --short | grep -v "^!!" | head
# 7. 克隆一次的真实体积（最直观的验收）
cd /tmp && git clone "/e/AI可视化/桌面AI-new" probe && du -sh probe
```

第 7 条是最诚实的验收：**克隆体积从 335 MB 掉到 10 MB 上下，就算成功了。**

---

## B6. 重建时必须顺手修的 3 个坑

这三处要在新仓库建好后立刻改，否则你就是把一个"干净但坏"的仓库推到线上。

### （1）`release.yml` 的 Windows 包是坏的 —— 高优先级

Windows job 只往打包目录拷两个文件，而 `llama.dll` 实测依赖 `ggml.dll`、`ggml-base.dll`、`libstdc++-6.dll`、`libgcc_s_seh-1.dll`。按当前 workflow 产出的 Windows 包，在干净机器上**模型功能不可用**。详见正文 `P1+` 小节。

修法：

```pwsh
$ver = "${{ github.ref_name }}" -replace '^v',''
New-Item -ItemType Directory -Force -Path staging | Out-Null
Copy-Item target/release/desktop-ai.exe staging/桌面AI.exe
Copy-Item vendor/windows/*.dll staging/
Compress-Archive -Path staging/* -DestinationPath 桌面AI-v$ver-windows-x86_64.zip -Force
```

若采用"二进制原地不动"方案，把第二行改成逐个列出 `build.rs` 里那 8 个 DLL。

**验收**：干净目录解压后运行，确认能加载模型。只在本机测试测不出来——本机系统目录里可能恰好有这些 DLL。

### （2）`android/build-apk.bat` 依赖机器 Temp 目录 —— 中优先级

脚本顶部两行硬编码指向本机 Temp，构建输入取自一个会被系统清理的目录；`JAVA_HOME`、`KOTLINC`、`BT`、`PLATFORM` 也全是本机绝对路径。修法：

- 把路径提成脚本顶部一组变量，并支持环境变量覆盖
- 新增 `android/README.md` 写清前置条件（JDK 17、kotlinc、build-tools 34.0.0、platform android-34、NDK 产物从哪来）
- 版本号 `DesktopAI-v6.1.6.apk` 改为读 `Cargo.toml` 的 `version`，消除与 `Cargo.toml` 的漂移

### （3）失效引用与署名

- `README.md` 第 17 行 `desktop-ai/src/android_service.rs` → `desktop-ai/src/platform/android.rs`
- `desktop-ai/Cargo.toml` 第 12、41 行注释里的 `android_service.rs` → 同上
- 两份 `LICENSE` 合并为一份，署名写全（原作者 + 你）
- 根 `README.md` 的"分支结构"表格删除——`android/` 就在 main 里，不存在独立分支

---

## B7. 建议的执行顺序（总表）

| 步骤 | 动作 | 耗时 | 风险 |
|---|---|---|---|
| 1 | `git bundle` 备份 + 捞出 `.git/` 里的两份文档 | 2 分钟 | 无 |
| 2 | 旁边搭新工作区，按 B1 清单取舍 | 10 分钟 | 无（原目录不动） |
| 3 | `git init` → **先写 `.gitignore`** → 再放文件 | 5 分钟 | 低 |
| 4 | `git add -A` 后**先看文件数与体积**，确认无大文件 | 5 分钟 | 低 |
| 5 | 提交，`cargo build` + `cargo test` 验证 | 10 分钟 | 中（首次拉依赖） |
| 6 | 修 B6 那三处 | 15 分钟 | 低 |
| 7 | `git push --force`（保留远程 star） | 2 分钟 | 中（不可逆，先确认无 fork） |

**总计约 50 分钟**，其中一半时间在等 `cargo build`。

---

*附录 B 基于正文的实测结论编写。其中 `llama.dll` 的依赖关系通过二进制导入表检查确认，`.git/` 内文档、Android 构建脚本的 Temp 路径依赖均通过直接读取文件内容核实。方案未执行，仅提供操作参考。*
