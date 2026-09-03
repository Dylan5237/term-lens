# AGENTS.md — Term Lens 项目代理工作规范

> 本文件适用于所有在此仓库工作的 AI 编码代理（Claude Code / Codex / WorkBuddy 等）。
> 依据：DESIGN.md §0 第一性原理锚点。改代码前先读本文件。

## 方向性原则（不可妥协）

本质命题：**AI 输出中的未知符号阻碍理解，工具的职责是以最小摩擦把"未知"变成"已知"。**

- **P1 触发摩擦最小化**：零触发 > 一键 > 多步。任何新增交互先问它增加还是减少摩擦。
- **P2 映射正确性**：本地术语表不是词典，是**已裁决的决策日志**。一词多义必须带 domain 多候选，禁止 1:1 唯一映射假设；语境无法裁决时并列展示候选，绝不猜一个答案。
- **P3 延迟分层**：本地路径 p95 <200ms 是生命线，热路径禁止引入磁盘 I/O 阻塞（用内存 LRU）；云端路径 ~1s 必须可预期、可静默降级。
- **P4 沉淀复利**：每次查询/采纳/裁决都要让术语库变聪明。所有写入路径最终落 SQLite 并可经 `export` 导出为开放格式。

## 硬约束

- **H1 数据出境最小化（合规红线）**：本项目运行在公司笔记本上，AI 输出可能含内部代码/业务信息。云端兜底**默认仅传术语单词**；任何"带上下文"功能必须用户显式开启且首次开启弹合规确认。日志/遥测禁止记录划选原文，只记归一化后的术语。
- **H2 资产中立**：禁止锁定私有格式。SQLite 为准，必须随时可 `export --format md|csv|agents`。微软 TBX 派生数据（L3 层）须保持 `source='ms-tbx'` 标记以便开源时剥离。
- **云端结果必经 pending 确认**才可转 active 写入词表（防 LLM 幻觉污染）。
- **密钥只走 Windows 凭据管理器**，禁止写入 config.toml 或代码。

## 工程约定

- 技术栈：Tauri 2.x + Rust（msvc 工具链）+ SQLite(WAL) + 原生 JS（无打包器）。
- 云端 provider：opencodex（127.0.0.1:10100，OpenAI 兼容）→ deepseek-v4-flash；只走环境变量/凭据解析，不硬编码。
- 系统提示词放 `prompts/fallback.md`，每次调用时读取（文件即配置=热重载）。
- 术语种子：`data/seed_terms.csv`（MarkdownTableImporter 规范：`en,zh,domain,ctx_hints,keep_policy,note,layer,source`）。
- 范围纪律：**不做**整篇文档翻译、云端流式跟翻、终端(ConPTY)取词（M2 前）、跨平台。砍需求优先于加需求。
- 验收看 DESIGN.md §9 指标表，用 query_log 回放测量；达不到门槛先查数据模型，别先堆功能。
- 术语裁决默认规范（L1 未覆盖时）：Agent→智能体（禁用"代理"）、Tool Use→工具使用、token 保留英文或"词元"视语境；参照 agentic-design-patterns-cn rules.md 的强制映射。
- Git：提交信息 conventional commits；`src-tauri/target/` 不入库；DESIGN.md 与 AGENTS.md 是权威源，改行为先改文档。
