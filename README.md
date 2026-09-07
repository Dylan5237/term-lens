# Term Lens 🐾

**Windows 托盘划词注释器**（不是终端透镜）。vibe coding 时 AI 爱冒英文术语？划选文本，双击 `Ctrl`，鼠标旁立刻弹出人话注释。

> **产品冻结**：不支持终端取词。取词靠模拟复制；剪贴板相对快照没变就失败，绝不拿旧内容去查、更不上云。默认无云端；没配好 loopback / https 白名单就不发外网。密钥走 Windows 凭据管理器，不要写进 `config.toml`。

```
"独立 smoke 的 Runtime 链路已走到预期受控失败"
        ↓ 划选 + 双击 Ctrl
  ┌─────────────────────────────┐
  │ smoke  [AI]        [devops] │
  │ 冒烟测试                     │
  │ 上线前的最小化功能验证        │
  │ [采纳] [修改] [否决]         │
  └─────────────────────────────┘
```

## 它解决什么问题

大模型输出的中文技术文本里混着大量英文术语（smoke / token / guardrail / hallucination…），
通用翻译工具要么整句机翻、要么不认识这些词的行业通行译法。Term Lens 只做一件事：
**把你划选里的英文术语注释成行业通行中文**，并且越用越准——你的每次裁决都会沉淀。

## 核心特性

- **两级流水线**：本地 SQLite 术语表（<200ms，内置 AI/IT 通行译法）→ 未命中且已配置合法 endpoint 才走云端兜底（默认 3s）
- **裁决闭环**：✅采纳 / ✏️修改 / ❌否决。云端结果先落「待确认」，你确认后才转正。否决只伤个人层，种子「词元」还在
- **一词多译**：token 在 LLM 语境锁定「词元」、安全语境→令牌；分不出域则并列候选，绝不瞎猜
- **隐私优先**：默认不上云。开启后兜底**只上传术语单词本身**。密钥不进 toml
- **系统托盘**：右键菜单分组：词库 / 提示词 / 大模型连接 / 界面与热键
- **可配置**：`%APPDATA%\term-lens\config.toml` 改模型/超时/热键，`fallback_prompt.md` 改提示词，保存即生效

## 安装

前置：**Windows 10/11**、[MSVC 工具链](https://visualstudio.microsoft.com/visual-cpp-build-tools/)（安装「使用 C++ 的桌面开发」）、系统自带或引导安装 [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) Runtime。

1. Releases 下载 `TermLens_0.1.2_x64-setup.exe` 安装（免管理员权限，currentUser）
2. 运行后进程常驻托盘——**默认就是安静的、不上云的**
3. 在普通桌面窗口划选包含英文术语的文本，双击 `Ctrl`

> 首次运行 Windows SmartScreen 可能拦截（未签名），点「仍要运行」即可。不支持 Windows Terminal / ConPTY 取词。

## 使用

| 操作 | 说明 |
|------|------|
| 双击 `Ctrl` | 划选取词（默认；托盘或 `config.toml` 可切 `Alt+T`） |
| 点击英文词 | 划选含多个术语时循环切换 |
| 采纳 / 修改 / 否决 | 裁决写入个人层；否决不改官方种子 |
| 托盘图标 左键 | 显示悬浮窗 |
| 托盘图标 右键 | 词库统计/导出/导入、提示词、模型配置、弹窗位置与触发方式 |
| `Esc` / 点别处 | 隐藏悬浮窗 |

剪贴板没变时会提示失败，**不会查询、不会上云、不写 query_log**。

## 配置云端兜底（可选）

不配也能用（只有本地词表）。默认 `base_url` 为空。若要解锁新词兜底：

1. 把 API Key 放进 **Windows 凭据管理器**（目标名 `TermLens/api_key`），或环境变量 `TERM_LENS_API_KEY`。**不要把密钥写进 toml。** 若旧版 toml 里有 `api_key`，启动时会尽量迁出后删掉。
2. 编辑 `%APPDATA%\term-lens\config.toml`：

```toml
[provider]
base_url = "https://api.deepseek.com"   # OpenAI 兼容；不要用 /anthropic
model    = "deepseek-v4-flash"
timeout_ms = 3000
```

本地代理也可以：`base_url = "http://127.0.0.1:10100/v1"`。loopback 明文 http 允许；其它公网必须 https。

不允许的 URL（任意主机的 http、非 http(s) 等）会直接失败且不发请求。loopback 绕过系统代理；外网走系统代理。

「恢复默认配置」写入的内容与仓库 `src-tauri/src/default_config.toml` 一致：`base_url = ""`、`timeout_ms = 3000`。

从 0.1.1 升级：仅当仍是未改过的默认文件（`timeout_ms = 15000` 且未配置密钥）时，才会一次性关掉默认公网 DeepSeek，并写入 `cloud_default_cleared`。之后在 toml 里显式填写 `https://api.deepseek.com` 会保留。toml 里的 `api_key` 只有成功写入凭据管理器并回读一致后才会从文件删除；失败则留在 toml。

## 命令行

```powershell
$env:TERMLENS_CLI = "1"   # 可选；--export/--stats/--rescan 会自动跳过单实例互斥
term-lens.exe --export          # 导出词库 CSV
term-lens.exe --stats           # 命中率 / 真正 95 分位延迟
term-lens.exe --rescan xx.csv   # 导入外部词库（永不覆盖个人裁决层；官方 rejected 可恢复）
```

## 词库从哪来

内置种子 = `data/seed_terms.csv`（含 agentic-cn 强制映射）+ `data/ai_terms_latest.csv`。两份合并后 `(en,domain,layer)` unique。
你的个人层完全私有；`--export` 导出的活跃词可发给别人 `--rescan` 导入。

## 构建（Windows / PowerShell）

```powershell
# 前置: rustup + Visual Studio Build Tools (MSVC) + WebView2
cd D:\_projects\tools\term-lens
npm install
cd src-tauri
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release            # 绿色版 exe: target\release\term-lens.exe
cd ..
npx tauri build                  # NSIS 安装包
# 产物: src-tauri\target\release\bundle\nsis\TermLens_0.1.2_x64-setup.exe
```

`package.json` 脚本：`npm test`（cargo test）、`npm run dev`、`npm run build`。

## 设计文档

`DESIGN.md`（第一性原理、架构、验收指标；未实施项已标注）· `AGENTS.md`（AI 代理协作规范）

运行时提示词文件是 `%APPDATA%\term-lens\fallback_prompt.md`，内置默认在 `src-tauri/src/fallback_prompt.md`。仓库里没有 `prompts/fallback.md`。

## License

MIT
