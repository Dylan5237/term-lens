# Term Lens 设计文档

> Windows 托盘划词注释器 · v0.1.3
> 日期：2026-09-08 · 作者：森几 × 小墨
> v0.1.3 变更：UIA 优先取词、标识符/短短语整段查询、多义平铺；产品仍冻结为划词注释器（非终端透镜），默认无云端

---

## 0. 第一性原理锚点（方向性原则，不可妥协）

本质命题：**AI 输出中的未知符号阻碍理解，工具的职责是以最小摩擦把"未知"变成"已知"。** 所有设计决策必须可追溯到以下四条约束之一，否则不做：

- **P1 触发摩擦最小化**：用户为搞懂一个词付出的动作要趋近于零。零触发 > 一键 > 多步。
- **P2 映射正确性**：含义依赖语境。工具输出的不是"翻译"，是**"在已裁决规则下的最优解释"**。一词多义必须带 domain 多候选；语境无法裁决时并列展示，**绝不猜一个答案**。
- **P3 延迟分层**：本地路径 p95 <200ms 是生命线。0.1.2 热路径走索引 SQLite（`en` 等值查询）。**内存 LRU：未实施**（本版本明确不做）。云端路径默认关闭；用户配置合法 endpoint 后超时默认 3s，必须可预期、可静默降级。
- **P4 沉淀复利**：每次查询都应让系统变聪明（采纳闭环），且资产可导出、可复用。0.1.2 导出格式为 **CSV**。Markdown / agents 词表段落 / TBX：**未实施**。

附加硬约束：

- **H1 数据出境最小化**：本项目运行在公司笔记本上。**默认无云端**（`base_url` 为空则零请求）。开启后**仅传输术语单词本身**。带上下文请求：**未实施**（已删除死开关，不提供 `send_context`）。日志/遥测禁止记录划选原文，只记归一化后的术语。
- **H2 资产中立**：术语库随时可导出为开放 CSV，不锁定私有存储。微软 TBX 派生数据（L3）：**未实施**。

## 1. 背景与问题

Vibe coding 过程中，AI 输出大量中英混杂的技术表述，例如：

> "独立 smoke 的 Runtime 链路本身已走到预期受控失败"

smoke 应理解为"冒烟测试"，Runtime 应理解为"程序运行时环境"。整段丢通用翻译器效果差（场景歧义），逐词查文档打断心流。

## 2. 目标与非目标

**产品冻结（0.1.2）**：这是 **Windows 托盘划词注释器**，不是终端透镜。不支持终端取词。取词优先 UI Automation / 原生 Edit；Electron 等读不到时才短暂 Ctrl+C，立刻还原剪贴板（非文本剪贴板不回退）。读不到选区才失败；同一词再划一次仍查询。默认无云端；没配好 loopback/白名单就不发外网。

**目标**
- 常用术语注释延迟 < 200ms；云端仅在用户显式配置合法 endpoint 后启用，超时默认 3s
- 术语准确率优先：一词多义按语境/域裁决，不确定时给多候选而非瞎猜
- 全局热键 / 托盘触发悬浮窗（覆盖普通桌面窗口的划选）
- 术语库持续沉淀 → CSV 导出

**非目标（0.1.2 明确不做）**
- 整页/整篇文档翻译
- 云端整段流式跟翻
- 终端取词（Windows Terminal / ConPTY）
- 浏览器扩展 / 零触发内联注释
- 内存 LRU
- 微软 TBX 导入（L3）
- agents / Markdown / 多格式导出（仅 CSV）
- Linux / macOS
- 覆盖旧 Git tag
- `send_context` 带上下文上云

## 3. 形态与技术栈

**形态：Windows 托盘常驻划词注释器**（热键或托盘菜单 → 读系统选区 → 悬浮窗）。

| 项 | 选型 | 理由 |
|----|------|------|
| 应用框架 | Tauri 2.x（Rust + WebView） | 安装包小、内存低、托盘/无边框窗成熟 |
| 术语存储 | SQLite（WAL 模式） | 单文件零运维；`UNIQUE(en,domain,layer)`；热路径 `en` 等值 + 索引 |
| 云端兜底 | 默认关闭。可选：loopback OpenAI 兼容（如 127.0.0.1:10100）或 **https** 公网 | 未配置 / URL 不在允许名单 → 不发请求 |
| 密钥存储 | Windows 凭据管理器（toml 不是正路） | 不落明文 |
| 资产格式 | SQLite 为准，CSV 导出 | H2；其它格式未实施 |

## 4. 核心架构

```
┌─ 零触发路径（浏览器扩展）—— 未实施
│
├─ 热键/托盘路径（0.1.2）
│    划词 + 双击 Ctrl（或 Alt+T）→ UIA / 原生 Edit；失败则探针 Ctrl+C 并还原剪贴板
│        无选区 → 失败提示，零 lookup / 零 fallback / 不写 query_log
│        有选区（含同一词再划）→ cap 8–16KB → 预提取英文片段（CJK 紧贴可抽；snake/kebab/camel 整段一词；短选区≤4 词段整段优先；停用词不作为第一优先）
│        ├─ 一级：本地术语表（归一化 + 域裁决）    命中 <200ms
│        │         多候选且分差为 0 → hit=None，并列展示，不上云
│        └─ 二级：云端兜底（仅传术语，H1；未配置则失败且不发请求）
│              └─ 结果标记 pending → 悬浮窗 → 采纳/修改/否决 → 沉淀个人层
└─ 反哺路径（P4/H2）
     术语表导出 CSV（agents/Markdown/TBX 导出：未实施）
```

关键行为约定：
- **取词**：UI Automation TextPattern + 原生 Edit/RichEdit；仍读不到时探针 Ctrl+C 并立刻还原。禁止把未变化剪贴板当选区。同一词再划仍查询
- **云端降级**：`base_url` 空 / URL 非法 / 超时 3s / 不可达 → 静默仅本地结果，悬浮窗标注"离线"
- **多候选裁决**（P2）：命中多域候选时按上下文关键词打分选域；`best==0` 且多候选则 **不猜**，并列展示
- **上下文开关**：删除。云端永远只传术语单词
- **取词单飞**：同时只跑一次 grab；Alt+T / 托盘翻译必须在后台线程，禁止在事件线程同步 sleep
- **热键切换**：注册成功后再改内存开关；失败回滚
- **seqId**：`showTerms` / `renderTerm` 入口发放；lookup 与 fallback 返回后都校验

## 5. 数据模型（v0.2 重写：一词多候选 + 决策日志语义）

本地表不是词典，是**裁决过的决策日志**：记录"你在什么语境下把什么词裁决成什么译法"。

```sql
CREATE TABLE terms (
  id INTEGER PRIMARY KEY,
  en TEXT NOT NULL,                 -- 归一化 lowercase
  en_variants TEXT,                 -- 0.1.2 不参与热路径查询（空列，禁止 LIKE 全表扫描）
  zh TEXT NOT NULL,                 -- 译法
  domain TEXT NOT NULL DEFAULT 'general',
  ctx_hints TEXT,                   -- JSON；读取必须按 Option，NULL 不得截断导出
  keep_policy TEXT DEFAULT 'translate',
  note TEXT,
  layer TEXT NOT NULL,              -- personal|ai|ms   (高层压过低层)
  source TEXT,
  status TEXT DEFAULT 'active',     -- active|pending|conflict|rejected
  hit_count INTEGER DEFAULT 0,
  created_at TEXT, updated_at TEXT,
  UNIQUE(en, domain, layer)
);

CREATE TABLE query_log (
  id INTEGER PRIMARY KEY,
  en TEXT NOT NULL,
  ts TEXT DEFAULT (datetime('now','localtime')),
  layer_hit TEXT,
  latency_ms INTEGER
);

CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- schema_version=1
```

`sources` / `decisions` 表与冲突仲裁 UI：**未实施**。

仲裁规则（0.1.2 已实施部分）：同 (en, domain) 多候选按 ctx_hints 打分；分不出则并列平铺全部释义，不点选、不猜。有语境命中时仍平铺，并标注「当前语境」。按 `source.confidence` 加权的仲裁队列：**未实施**。

**层隔离**
- `reject` 只 UPDATE `layer='personal'`，禁止改种子层 status
- `adopt` 的 `ON CONFLICT` 必须 `SET status='active', source='user-adopted'`
- 官方层启动时 `INSERT OR IGNORE`（锁定词例外 upsert），永不碰 personal
- 导入：非 personal 且 `rejected` 可恢复为 active；personal-rejected 只覆盖展示，不改种子

**锁定译名（官方层）**：`token` + domain `llm` → **词元**（security 域保留「令牌」）。

## 6. 术语库：三层结构 + 沉淀扩展点

| 层 | 内容 | 0.1.2 |
|----|------|--------|
| L1 个人层 | 用户拍板的译法与裁决 | 已实施：采纳/修改/否决 |
| L2 AI 术语层 | agent 时代术语 | 已实施：两份 CSV 种子，unique `(en,domain,layer)` |
| L3 经典底座 | 微软术语库 TBX | **未实施** |

**扩展点：**
1. `ImportSource` trait / TbxImporter：**未实施**。0.1.2 为 CSV 解析导入
2. 冲突仲裁队列：**未实施**
3. 采纳闭环：cloud-adopted 默认落 pending，需用户确认才转 active — **已实施**
4. 导出矩阵：CSV — **已实施**；agent 词表 / Markdown / TBX — **未实施**
5. 使用统计：query_log + **真正 95 分位**（升序 `OFFSET n*19/20`）

## 7. 冷启动数据源

| 来源 | 置信度 | 用途 |
|------|--------|------|
| `data/seed_terms.csv`（含 agentic-cn 强制映射） | 90 | L2 主源；与 `ai_terms_latest.csv` 合并后 unique |
| `data/ai_terms_latest.csv`（LLM-Fundamentals） | 75 | L2 补充；冲突键以 seed 为准 |
| Microsoft Terminology Collection（TBX） | 80 | **未实施** |
| 其它社区源 | — | **未实施** |

启动策略：**不是** `seed_if_empty` 一次性。每次启动对官方层 `INSERT OR IGNORE`（锁定词 upsert），personal 不动。`schema_version` 启动迁移。

## 8. 模块划分（Tauri）

- **capture**：global-shortcut / 双击 Ctrl；UIA / 原生 Edit 优先；失败才探针 Ctrl+C 并还原
- **glossary**：ASCII 词边界提取（CJK 紧贴可抽；`blocked_field` / `huge-doge` / `blockedField` 整段一词；选区本身像术语则整段优先，部件可随后轮询）+ 保守归一 + SQLite 等值查询 + 域裁决（不分则不猜）
- **fallback**：OpenAI 兼容调用；提示词来自 `%APPDATA%\term-lens\fallback_prompt.md`（每次读取=热重载）；IPC 有最大长度/字符类约束；未配置不发请求
- **overlay**：无边框置顶窗、失焦即隐、采纳/修改/否决；CSP：`default-src 'self'`，`connect-src` 仅 ipc
- **tray/config**：托盘菜单、`config.toml`（provider/热键，**不含 api_key 正路**）、凭据管理器取 key。0.1.1 默认公网 DeepSeek **只清一次**（无密钥且 timeout 仍为 15000）；打上 `migrate.cloud_default_cleared` 后用户再填同一 URL 保留。toml 中的 api_key 仅在凭据回读成功后删除。
- **export**：`--export` 输出 CSV（`--format md|agents`：**未实施**）

## 9. 验收指标（可证伪）

| 指标 | MVP 门槛 | 一个月目标 | 测量方式 |
|------|----------|------------|----------|
| 本地命中率 | >70% | >85% | 用真实 AI 输出语料回放测 query_log |
| 本地路径延迟 p95 | <200ms | <100ms | query_log.latency_ms，**真正 95 分位**（升序 OFFSET n×19/20） |
| 云端兜底 p95 | <1.2s（仅当已配置） | <1.0s | 计时日志 |
| 悬浮窗取词→显示 | <200ms（命中时） | — | 手动秒表抽查 |
| 周均查询量 | >50 次/周 | — | 存活验证（没人用就砍项目） |

## 10. 里程碑

- **M0 / 0.1.2（本次交付）**：Windows 托盘划词闭环——热键→读系统选区→本地表→可选云端兜底→悬浮窗→采纳回写→CSV 导出；默认零外网
- **M1（未实施）**：TbxImporter + 冲突仲裁 UI + 浏览器扩展
- **M2（未实施）**：使用统计面板、agent 词表导出、终端取词评估（评估 ≠ 做）
- **M3（候选，未实施）**：术语表开源发布、团队共享

## 11. 风险与对策

- 译名分歧（Agent→智能体/代理）：个人层永久压过来源层（默认采纳 agentic-cn 规范「智能体」）
- 云端幻觉污染词表：cloud-adopted 必经 pending 确认
- 数据出境：H1 默认无云端；开启后仅术语单词；loopback 走 no_proxy，外网走系统代理
- 覆盖用户剪贴板：UIA 失败才 Ctrl+C，写入探针后立刻还原；非文本剪贴板不回退；同一词再划不得误报失败
- MSVC/Tauri 构建链在公司笔记本受限：已验证 rustc + VS2022 Professional 可用
- opencodex / 公网 API 单点：未配置或失败则静默降级
