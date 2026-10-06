# 安全政策

## 报告漏洞

请**不要**通过公开 issue 报告安全漏洞。使用 GitHub 的私密渠道:

👉 [Security Advisories](https://github.com/96312-reneren/desktop-ai/security/advisories/new)
(仓库页面 → Security → Report a vulnerability)

报告时请尽量包含:

- 影响的版本(以 `Cargo.toml` 的 `version` 为准)
- 复现步骤或最小复现样例
- 影响面评估(信息泄露 / 拒绝服务 / 本地提权等)

## 响应时间

这是个人维护的开源项目,尽力在 **7 天内**确认报告,修复时间视严重程度而定。

## 设计背景(帮助判断影响)

- 本应用是**完全本地运行**的桌面/安卓程序,API 仅绑定 `127.0.0.1` 并要求 token
- 桌面端 API token 为首次启动随机生成,存于用户数据目录的 config
- 安卓端 API token 每次启动在内存中随机生成,不落盘、不嵌入 APK
- 已知的本地攻击面(恶意本机进程、物理访问)不在威胁模型内;
  但**远程可达**的漏洞(爬虫/RAG 拉取恶意内容、模型下载链路)属于重点关注范围

## 历史评审

安全评审与整改记录见 [`docs/security-review.md`](docs/security-review.md)(含修复状态表)。
