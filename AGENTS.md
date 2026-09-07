# AGENTS.md — Term Lens 项目代理工作规范

> 本文件适用于所有在此仓库工作的 AI 编码代理（Claude Code / Codex / WorkBuddy 等）。
> 依据：DESIGN.md §0 第一性原理锚点。改代码前先读本文件。

## 方向性原则（不可妥协）

本质命题：**AI 输出中的未知符号阻碍理解，工具的职责是以最小摩擦把"未知"变成"已知"。**

- **P1 触发摩擦最小化**：零触发 > 一键 > 多步。任何新增交互先问它增加还是减少摩擦。
- **P2 映射正确性**：本地术语表不是词典，是**已裁决的决策日志**。一词多义必须带 domain 多候选，禁止 1:1 唯一映射假设；语境无法裁决时并列展示候选，绝不猜一个答案。
- **P3 延迟分层**：本地路径 p95 <200ms 是生命线。0.1.2 热路径走索引 SQLite（禁止 `en_variants LIKE` 全表扫描）。**内存 LRU：不做**。云端默认关闭；开启后超时默认 3s，必须可预期、可静默降级。
- **P4 沉淀复利**：每次查询/采纳/裁决都要让术语库变聪明。所有写入路径最终落 SQLite 并可经 `export` 导出为 CSV（md/agents 格式：**未实施**）。

## 硬约束

- **H1 数据出境最小化（合规红线）**：本项目运行在公司笔记本上，AI 输出可能含内部代码/业务信息。**默认无云端**（`base_url` 空或未通过允许名单则零请求）。开启后**仅传术语单词**。带上下文上云：**未实施**（不要加回托盘死开关）。日志/遥测禁止记录划选原文，只记归一化后的术语。
- **H2 资产中立**：禁止锁定私有格式。SQLite 为准，0.1.2 可 `export` 为 CSV。微软 TBX / L3：**未实施**。
- **云端结果必经 pending 确认**才可转 active 写入词表（防 LLM 幻觉污染）。
- **密钥只走 Windows 凭据管理器**（或环境变量 `TERM_LENS_API_KEY`），禁止把 api_key 当作 config.toml 的正路。启动时可一次性从 toml 迁出后删除文件中的 key。
- **剪贴板未变禁止查询**：模拟复制后若剪贴板相对快照没有变化，中止，不 lookup、不 fallback、不写 query_log。

## 产品冻结（0.1.2）

这是 **Windows 托盘划词注释器**，不是终端透镜。不支持终端取词。取词靠模拟复制。默认无云端。

明确不做：浏览器扩展、ConPTY、内存 LRU、前端虚拟化、微软 TBX、agents 多格式导出、Linux/macOS、流式跟翻、整篇翻译、覆盖旧 tag。

## 工程约定

- 技术栈：Tauri 2.x + Rust（msvc 工具链）+ SQLite(WAL) + 原生 JS（无打包器）。
- 云端 provider：默认空。可选 loopback（如 `http://127.0.0.1:10100/v1`）或 https；只走环境变量/凭据解析，不硬编码密钥。明文 http 仅允许 loopback host。
- 系统提示词：内置默认 `src-tauri/src/fallback_prompt.md`；运行时读写 `%APPDATA%\term-lens\fallback_prompt.md`（每次调用时读取，文件即配置=热重载）。**仓库里没有 `prompts/fallback.md`。**
- 术语种子：`data/seed_terms.csv` + `data/ai_terms_latest.csv`（规范：`en,zh,domain,ctx_hints,keep_policy,note,layer,source`）。两份合并后 `(en,domain,layer)` 必须 unique；`token`+`llm` 锁定「词元」，security 域保留「令牌」。
- 官方层启动：`INSERT OR IGNORE` 或按层 upsert，**废除 `seed_if_empty` 作为唯一策略**，永不碰 personal。
- 范围纪律：**不做**整篇文档翻译、云端流式跟翻、终端(ConPTY)取词、跨平台。砍需求优先于加需求。
- 验收看 DESIGN.md §9 指标表；p95 必须是真正 95 分位。达不到门槛先查数据模型，别先堆功能。
- 术语裁决默认规范（L1 未覆盖时）：Agent→智能体（禁用"代理"）、Tool Use→工具使用、token 在 LLM 语境锁定「词元」。
- Git：提交信息 conventional commits；`src-tauri/target/` 不入库；DESIGN.md 与 AGENTS.md 是权威源，改行为先改文档。
- 版本号：Cargo.toml、tauri.conf.json、README 安装说明统一 **0.1.2**。
