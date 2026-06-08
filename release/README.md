# 桌面AI v5.0 — 完整功能版

> 双击运行，本地 AI 对话。**~8 MB 双文件分发，毫秒启动。**

## v5 更新

### 安全性加固 (继承自 v4)
- CString空字节防护、Arc生命周期管理(UAF修复)、路径遍历修复
- 下载完整性验证、DLL校验、配置边界检查
- Release: LTO+strip+panic=abort

### 新功能
- **主题实时切换**：点击即生效，启动自动应用
- **设置全面升级**：
  - CPU 线程数选择 (auto / 2-16)
  - 系统提示词编辑器
  - 已下载模型列表 + 单独删除
  - 切换模型快捷入口
- **数据管理**：
  - 删除所有模型 / 删除所有对话 / 重置应用
  - 所有危险操作二次确认

## 快速开始

1. 确保 `桌面AI.exe` 和 `llama.dll` 在同一目录
2. 双击 `桌面AI.exe`
3. 选择模型 → 下载 → 自动加载 → 对话

## 6 款可选模型

| 模型 | 大小 | 内存 | 适用 |
|------|------|------|------|
| Qwen2.5-0.5B | 0.35 GB | 2 GB | 超低配/老旧设备 |
| Qwen3-1.7B | 1.2 GB | 4 GB | 低配电脑首选 |
| Qwen2.5-3B | 2.0 GB | 6 GB | 日常对话 |
| Qwen2.5-7B | 4.7 GB | 8 GB | 综合能力 |
| Qwen2.5-Coder-7B | 4.7 GB | 8 GB | 编程专用 |
| Qwen3-8B | 5.5 GB | 10 GB | 最新最强 |

## 系统要求

- Windows 10/11 64-bit
- VC++ Redistributable (https://aka.ms/vs/17/release/vc_redist.x64.exe)

## 技术栈

Rust 1.96 + egui/eframe + llama.cpp FFI + pulldown-cmark + reqwest
