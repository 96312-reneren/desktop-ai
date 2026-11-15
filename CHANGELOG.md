# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式，
版本号遵循语义化版本。最新版本号以 `desktop-ai/Cargo.toml` 为准。

## [6.2.0] — 2026-11-15

### 新增
- RAG 知识库文档来源元数据（粘贴 / 文件 / 抓取 + 原始 URL），schema v2 自动迁移，检索结果与引用处展示来源徽标
- RAG 混合检索：向量语义 + FTS5 关键词双路召回，RRF 融合去重（设置中可开关，默认开）
- RAG 规则化查询重写：短问题指代消解，自动结合上一轮提问检索（仅影响检索，不改原文；可开关，默认开）
- RAG prompt token 预算与历史截断，生成参数可配置
- 回答下方展示检索来源；KB 管线迁移至 rag 模块
- UI：侧边栏状态持久化、toast 通知、自适应气泡、Esc 停止生成
- 动态模型目录 + 真虚拟滚动；代码块复制按钮
- cargo-fuzz 落地：8 个纯解析器 fuzz 目标
- 全模块简体中文注释

### 安全
- RAG 不可信上下文围栏；robots 策略与 crawler 钉扎加固（第 3 轮）
- API 面、下载、KB 输入加固（第 2 轮）；搜索响应大小上限与重定向逐跳校验（第 1 轮）
- 检索分数诚实化；`/ready` 不再暴露模型名；CLI token 随机化
- WAL 持久性、会话 id 碰撞、下载暂存等多项加固
- FTS 重建孤儿感知与删除一致性修复

### 修复
- 修复远程模型目录 404：仓库根新增 `model_catalog.json`，客户端可拉取远程目录
- 修复 `clippy::needless_bool`（Rust 1.100 工具链）

### 文档
- 根 README 重写为落地页（徽章、应用截图、三分钟上手）
- desktop-ai/README 重写精简：删除法律章节，更新 RAG 新特性与测试数（254）
- 新增应用截图 3 张（`assets/screenshots/`）
- 评估报告类文档移出仓库（`docs/` 已加入 .gitignore）

## [6.1.6] — 2026-10

### 新增
- 桌面端 Material Design 3 主题（紫色基线，与安卓端统一设计语言）
- 安卓端 WebView UI 重做为 M3 风格；会话管理（新建/切换/删除）与 Markdown 渲染
- 爬虫支持 robots.txt（RFC 9309：最长匹配、通配符、逐跳检查、按 origin 缓存）
- 模型选择窗口支持标题栏关闭按钮；面板尺寸按主窗口比例固定

### 安全
- 安卓 API token 改为每次启动随机生成（经 JNI 传入，不再硬编码于 APK）
- 安卓 API 端口动态分配（`bind 127.0.0.1:0`），防端口劫持
- JS 桥 URL 白名单；WebView 关闭 file 访问；`allowBackup=false`
- 网络明文仅放行回环地址（networkSecurityConfig）；APK v2+v3 签名
- 修复 cleaner/crawler Unicode 字节边界 panic（恶意网页 DoS）
- crawler 每跳 DNS 钉扎防 rebinding；sandbox 写入 256 MiB 硬上限
- 空 api_token 回填；llama 库 SHA-256 基线比对
- keystore 移出仓库，口令随机化（keystore-mgr.py）

### 变更
- 运行时库迁至 `vendor/windows/`（含 SHA-256 清单）；删除入库的 Linux `.so`（CI 现场构建）
- `ui::app` 排除出安卓构建，eframe 移出安卓依赖（libdesktop_ai.so 12.5 MB → 1.9 MB）
- 移除 `lib.rs` 扁平别名，调用点全部改用领域路径（`crate::store::config` 等）
- 移除零引用依赖 `anyhow` / `egui_extras`；`tracing-log` 特性显式启用
- 版本号单点化：窗口标题、APK versionName 均读取 `Cargo.toml`
- 仓库历史重写：剔除 196 MB 构建产物历史（88 次提交保留）

### 修复
- Windows 发布包补齐 8 个运行时 DLL（此前只打包 llama.dll，发布包不可用）
- API 服务端：OPTIONS 预检免认证、`Origin: null` 仅安卓放行、
  `Access-Control-Allow-Private-Network` 支持
- 更新 CI 工具链下 `fetch_update` 弃用与浮点字面量回退错误

## [6.1.4] — 2026-08

- 修复 Qwen3-8B 输出异常：上下文参数对齐 `llama_context_default_params`
- 修复退化循环：EOS/EOT 停止符 + 重复惩罚
- Windows 运行时 DLL 集合重建（llama + ggml + MinGW 运行库）
- USB/便携模式模型加载、下载后自动加载

## [6.1.0] — 2026-07

- SQLite 存储迁移（知识库与对话，WAL 事务、崩溃不丢、增量写入）
- 会话增量落盘；模型下载断点续传；每日滚动日志
- API token 密码学随机化；ChatML 注入消毒扩展；SSRF 防护补强
- 正式支持 Linux（x86_64, glibc ≥ 2.35），CI 双平台发布
