# Term Lens 🐾

**AI 输出中英混杂术语 · 实时注释小工具（Windows）**

 vibe coding 时 AI 爱冒英文术语？划选文本，双击 `Ctrl`，鼠标旁立刻弹出人话注释。

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

- **两级流水线**：本地 SQLite 术语表（<200ms，内置 160+ 条 AI/IT 通行译法）→ 未命中走云端 LLM 兜底（~3s）
- **裁决闭环**：✅采纳 / ✏️修改 / ❌否决 三键。云端结果先落"待确认"，你确认后才转正——防幻觉词污染词库
- **一词多译**：token 在 LLM 语境→词元、安全语境→令牌，按划选上下文自动选域
- **越用越快**：兜底结果自动沉淀，同一个词第二次秒出
- **隐私优先**：兜底默认**只上传术语单词本身**，不上传你划选的上下文；个人裁决层永不外发
- **一切可配置**：`%APPDATA%\term-lens\config.toml` 改模型/超时/热键，`fallback_prompt.md` 改提示词，保存即生效

## 安装

1. Releases 下载 `TermLens_0.1.0_x64-setup.exe` 安装（免管理员权限）
2. 运行后进程常驻，无窗口无任务栏图标——**默认就是安静的**
3. 划选任意包含英文术语的文本，双击 `Ctrl`

> 首次运行 Windows SmartScreen 可能拦截（未签名），点"仍要运行"即可。

## 使用

| 操作 | 说明 |
|------|------|
| 双击 `Ctrl` | 划选取词（默认；`config.toml` 可切回 `Alt+T`） |
| 点击英文词 | 划选含多个术语时循环切换 |
| 采纳 / 修改 / 否决 | 裁决写入个人层，下次秒出 |
| `Esc` / 点别处 | 隐藏悬浮窗 |

## 配置云端兜底（可选）

不配也能用（只有本地词表）。想解锁任意新词的兜底，编辑
`%APPDATA%\term-lens\config.toml`，指向任意 OpenAI 兼容 API：

```toml
[provider]
base_url = "https://api.deepseek.com/v1"   # 或任何兼容网关
api_key  = "sk-..."
model    = "deepseek-chat"
```

## 命令行

```bash
term-lens.exe --export          # 导出词库 CSV（可分享给同事导入）
term-lens.exe --stats           # 命中率/延迟统计
term-lens.exe --rescan xx.csv   # 导入外部词库（永不覆盖个人裁决层）
```

## 词库从哪来

内置种子 = 社区 AI 术语库（LLM-Fundamentals，2026-01）+ 常见工程术语手工裁决。
你的个人层完全私有；`--export` 导出的活跃词可发给别人 `--rescan` 导入，团队共建。

## 构建

```bash
cd src-tauri && cargo build --release        # 绿色版 exe
npx tauri build                              # NSIS 安装包 → src-tauri/target/release/bundle/nsis/
```

## 设计文档

`DESIGN.md`（第一性原理、架构、验收指标）· `AGENTS.md`（AI 代理协作规范）

## License

MIT
