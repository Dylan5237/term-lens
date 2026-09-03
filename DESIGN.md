# Term Lens 设计文档

> AI 输出中英混杂术语实时注释工具 · v0.2
> 日期：2026-09-03 · 作者：森几 × 小墨
> v0.2 变更：按第一性原理审核修订——多候选 schema 与决策日志语义、数据出境硬约束、浏览器零触发注释升为一等公民（二期）、术语表→agent 词表导出闭环、可证伪验收指标、导入源置信度分级

---

## 0. 第一性原理锚点（方向性原则，不可妥协）

本质命题：**AI 输出中的未知符号阻碍理解，工具的职责是以最小摩擦把"未知"变成"已知"。** 所有设计决策必须可追溯到以下四条约束之一，否则不做：

- **P1 触发摩擦最小化**：用户为搞懂一个词付出的动作要趋近于零。零触发 > 一键 > 多步。
- **P2 映射正确性**：含义依赖语境。工具输出的不是"翻译"，是**"在已裁决规则下的最优解释"**。
- **P3 延迟分层**：本地路径 <200ms 是体验生命线；云端路径 ~1s 可接受但必须可预期、可降级。
- **P4 沉淀复利**：每次查询都应让系统变聪明（采纳闭环），且资产可导出、可复用、可开源。

附加硬约束：

- **H1 数据出境最小化**：本项目运行在公司笔记本上。云端兜底**默认仅传输术语单词本身**；任何携带上下文的请求必须用户显式开启。AI 输出可能包含内部代码与业务信息，此条为合规红线。
- **H2 资产中立**：术语库随时可整体导出为开放格式（Markdown/CSV），不锁定私有存储。微软 TBX 派生数据若未来开源发布须可剥离（署名要求）。

## 1. 背景与问题

Vibe coding 过程中，AI 输出大量中英混杂的技术表述，例如：

> "独立 smoke 的 Runtime 链路本身已走到预期受控失败"

smoke 应理解为"冒烟测试"，Runtime 应理解为"程序运行时环境"。整段丢通用翻译器效果差（场景歧义），逐词查文档打断心流。

## 2. 目标与非目标

**目标**
- 常用术语注释延迟 < 200ms，新术语云端兜底 p95 < 1.2s
- 术语准确率优先：一词多义按语境/域裁决，不确定时给多候选而非瞎猜
- 覆盖浏览器 + 桌面应用（热键悬浮窗 + 浏览器内联自动注释）
- 术语库持续沉淀 → 反哺 agent 词表 → 可开源

**非目标**
- 整页/整篇文档翻译（沉浸式翻译已覆盖）
- 云端整段流式跟翻（延迟与质量双输）
- 终端取词（Windows Terminal/ConPTY 无 DOM，模拟复制有 Ctrl+C 冲突，二期单独立项评估）
- 跨平台（只做 Windows）

## 3. 形态与技术栈

**形态：托盘常驻小工具 + 浏览器辅助扩展**。
理由：兼容面（网页 ChatGPT、桌面客户端）的共同点是"屏幕上的文字"，最小摩擦路径是①零触发内联注释（浏览器扩展，仅本地词表）②全局热键悬浮窗（覆盖一切窗口）。

| 项 | 选型 | 理由 |
|----|------|------|
| 应用框架 | Tauri 2.x（Rust + WebView） | 安装包小、内存低、系统级能力（全局钩子/托盘/无边框窗）成熟 |
| 术语存储 | SQLite（WAL 模式）+ 内存 LRU | 单文件零运维；高频词常驻内存保 P3 |
| 云端兜底 | opencodex（127.0.0.1:10100）→ deepseek-v4-flash | 快、便宜、多 provider 可切换 |
| 密钥存储 | Windows 凭据管理器 | 不落明文 |
| 资产格式 | SQLite 为准，CSV/Markdown 双向导出 | H2 |

## 4. 核心架构

```
┌─ 零触发路径（浏览器扩展，二期）
│    AI 输出停稳 → 本地词表匹配已知术语 → 内联括注/下划线（纯本地，不碰云端）
│
├─ 热键路径（MVP）
│    划词 + Alt+T → 模拟复制 → 剪贴板快照恢复 → 预提取英文片段
│        ├─ 一级：本地术语表（归一化 + 域裁决）    命中 <200ms
│        └─ 二级：opencodex 云端兜底（仅传术语，H1）~1s
│              └─ 结果标记 pending → 悬浮窗 → 采纳/修改/否决 → 沉淀个人层
└─ 反哺路径（P4/H2）
     术语表导出 → ① agent 词表（CLAUDE.md / AGENTS.md 段落，约束自家 AI 输出格式）
                → ② 开放格式发布（剥离 L3 派生数据后可开源）
```

关键行为约定：
- **剪贴板快照恢复**：取词后恢复原剪贴板，不污染用户复制
- **云端降级**：超时 3s / opencodex 不可达 → 静默仅本地结果，悬浮窗标注"离线"
- **多候选裁决**（P2）：命中多域候选时按上下文关键词打分选域；分不出则悬浮窗并列展示候选让用户点选（点选本身即一次裁决，计入个人层）
- **上下文开关**：默认关闭（H1）；开启时仅传划选文本前后各一句，且首次开启弹合规确认

## 5. 数据模型（v0.2 重写：一词多候选 + 决策日志语义）

本地表不是词典，是**裁决过的决策日志**：记录"你在什么语境下把什么词裁决成什么译法"。

```sql
CREATE TABLE terms (
  id INTEGER PRIMARY KEY,
  en TEXT NOT NULL,                 -- 归一化 lowercase
  en_variants TEXT,                 -- JSON 词形变体 ["smoke tests","Smoke-Test"]
  zh TEXT NOT NULL,                 -- 译法
  domain TEXT NOT NULL DEFAULT 'general',  -- general|llm|frontend|security|devops|windows|...
  ctx_hints TEXT,                   -- JSON: 选此域的上下文线索词 ["prompt","agent","token"]
  keep_policy TEXT DEFAULT 'translate',    -- translate|keep|note
  note TEXT,                        -- 释义/译注
  layer TEXT NOT NULL,              -- personal|ai|ms   (高层压过低层)
  source TEXT,                      -- 来源标识，关联 confidence
  status TEXT DEFAULT 'active',     -- active|pending|conflict|rejected
  hit_count INTEGER DEFAULT 0,
  created_at TEXT, updated_at TEXT,
  UNIQUE(en, domain, layer)
);

CREATE TABLE sources (              -- 导入源置信度分级（仲裁加权用）
  id TEXT PRIMARY KEY,              -- agentic-cn / dongshuyan / csdn / ms-tbx / manual / cloud-adopted
  confidence INTEGER,               -- 人工校对流程>聚合内容: agentic-cn=90 csdn=60 ms-tbx=80 manual=100
  license_note TEXT
);

CREATE TABLE decisions (            -- 裁决日志：冲突的最终归属
  id INTEGER PRIMARY KEY, en TEXT, chosen_term_id INTEGER, reason TEXT, decided_at TEXT
);
```

仲裁规则：同 (en, domain) 多来源冲突 → 按 source.confidence 加权，最高者胜出但入 conflict 队列待人工确认；无裁决记录时悬浮窗并列展示，用户点选即写入 decisions。

## 6. 术语库：三层结构 + 沉淀扩展点

| 层 | 内容 | 演进 |
|----|------|------|
| L1 个人层 | 用户拍板的译法与裁决 | 采纳/修改/点选闭环持续沉淀 |
| L2 AI 术语层 | agent 时代术语（token/hallucination/guardrail…） | 开源源导入 + 滚动补充（无权威库，本层即差异化资产） |
| L3 经典底座 | 微软术语库 TBX ~3 万条 | 一次性导入；开源发布时可剥离（H2） |

**扩展点（开放接口）：**
1. `ImportSource` trait：TbxImporter / MarkdownTableImporter / CsvImporter 内置，社区新源即插即用
2. 冲突仲裁队列（见 §5）
3. 采纳闭环：cloud-adopted 默认落 pending，需用户确认才转 active（防云端幻觉污染词表）
4. 导出矩阵：→ agent 词表段落（CLAUDE.md/AGENTS.md 格式）｜→ Markdown/CSV/TBX｜→ 沉浸式翻译自定义术语
5. 使用统计：hit_count 驱动高频置顶与零命中清理

## 7. 冷启动数据源

| 来源 | 置信度 | 用途 |
|------|--------|------|
| canisn/agentic-design-patterns-cn rules.md 强制映射表 | 90（有 PR 审核） | L2 主源 + 译名决策规则 |
| Microsoft Terminology Collection（TBX） | 80 | L3 底座 |
| dongshuyan/LLM-Fundamentals 常见名词篇 | 75 | L2 |
| spec-kit-cn TERMINOLOGY.md | 75 | L2 + "保留英文"规则样例 |
| CSDN《大模型应用开发术语中英对照表》 | 60（聚合内容需甄别） | L2 补充 |

## 8. 模块划分（Tauri）

- **capture**：global-shortcut(Alt+T，启动时冲突检测) + enigo 模拟 Ctrl+C + arboard 剪贴板读写快照恢复
- **glossary**：正则英文片段提取 + 词形归一 + SQLite/内存 LRU 查询 + 域裁决打分
- **fallback**：opencodex OpenAI 兼容调用；提示词来自 `prompt.md` 文件（每次读取=热重载）；超时/降级
- **overlay**：无边框置顶不抢焦点窗（WS_EX_NOACTIVATE）、失焦即隐、多屏 DPI、采纳/修改/否决按钮
- **tray/config**：托盘菜单、`config.toml`（provider/热键/上下文开关）、凭据管理器取 key
- **export**（P4）：CLI 子命令 `term-lens export --format md|csv|agents`（agents = 生成词表段落贴进 CLAUDE.md）

## 9. 验收指标（可证伪）

| 指标 | MVP 门槛 | 一个月目标 | 测量方式 |
|------|----------|------------|----------|
| 本地命中率 | >70% | >85% | 用真实 AI 输出语料回放测 query_log |
| 本地路径延迟 p95 | <200ms | <100ms | query_log.latency |
| 云端兜底 p95 | <1.2s | <1.0s | 计时日志 |
| 悬浮窗取词→显示 | <200ms（命中时） | — | 手动秒表抽查 |
| 周均查询量 | >50 次/周 | — | 存活验证（没人用就砍项目） |

## 10. 里程碑

- **M0（本次交付）**：Tauri 最小闭环——热键→取词→本地表（含 L2 种子词）→云端兜底→悬浮窗→采纳回写→CSV 导出
- **M1**：TbxImporter + 冲突仲裁 UI + 浏览器扩展（零触发内联注释）
- **M2**：使用统计面板、agent 词表导出、终端取词评估
- **M3（候选）**：术语表开源发布、团队共享

## 11. 风险与对策

- 译名分歧（Agent→智能体/代理）：决策日志 + 仲裁队列，个人层永久压过来源层（默认采纳 agentic-cn 规范"智能体"）
- 云端幻觉污染词表：cloud-adopted 必经 pending 确认
- 数据出境：H1 默认仅术语；带上下文首次开启需合规确认
- MSVC/Tauri 构建链在公司笔记本受限：已验证 rustc 1.97 + VS2022 Professional 可用
- opencodex 单点：静默降级已内置
