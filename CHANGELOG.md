# Changelog

## 0.1.2 — 2026-09-07

Windows 托盘划词注释器发版门槛：可构建、有门禁、主路径不再写坏数据/误出境。

### 产品

- 定位冻结：划词注释器，**不是**终端透镜；不支持终端取词
- 空窗 / 托盘 / README 写明：模拟复制取词；剪贴板未变则失败；默认不上云

### 正确性

- 剪贴板相对快照未变 → 失败提示，零 lookup / fallback / query_log
- 取词后 cap 16KB；取词单飞；Alt+T 与托盘翻译在后台线程
- `reject` 只伤 personal；种子 `token`+`llm` 锁定「词元」
- `adopt` 冲突更新 `status='active', source='user-adopted'`
- 导出按 `Option` 读 `ctx_hints`，单行错误不截断，条数以实际写入为准
- IPC 返回 `Result`，前端 try/catch，SQL 失败不再假成功
- 默认 `base_url` 为空；未配置或 URL 不在允许名单则不发请求
- 从 0.1.1 升级：若 `config.toml` 仍是产品默认公网 `https://api.deepseek.com`（含无尾斜杠、大小写、可选 `/v1`），启动时清空并写回；自定义 endpoint 不动。需要云端请显式再配
- 密钥迁入 Windows 凭据管理器；toml 不再作为正路
- CSP：`default-src 'self'`，`connect-src` 仅 ipc；删除 `send_context` 死开关
- `extract_terms` ASCII 词边界（CJK 紧贴可抽）；停用词不作为第一优先
- `pick_term` 在 `best==0` 且多候选时不猜
- 热键切换：注册成功后再改开关；seqId 在入口发放
- 默认超时 3s；p95 为真正 95 分位（升序 OFFSET n×19/20）
- 官方层启动 `INSERT OR IGNORE`（锁定词 upsert），废除 `seed_if_empty` 唯一策略
- 两份 CSV unique `(en,domain,layer)`；schema_version 启动迁移

### 工程

- 版本号统一 0.1.2
- Windows GitHub Actions：fmt + clippy -D warnings + test + build
- `package-lock.json` 纳入版本库
- 单实例互斥撞车弹出提示，不再静默 `exit(0)`

### 明确不做

浏览器扩展、ConPTY、内存 LRU、微软 TBX、agents 多格式导出、Linux/macOS、流式跟翻、整篇翻译。
