# Term Lens

AI 输出中英混杂术语实时注释工具（Windows）。

划选一段 AI 输出（如 "独立 smoke 的 Runtime 链路…"），按 `Alt+T`，鼠标旁弹出术语注释：
常用术语 <200ms 本地命中，新术语走 opencodex 云端兜底（~1s），结果一键采纳沉淀进本地术语表。

## 使用

1. 双击运行 `dist/term-lens.exe`（托盘无图标，进程常驻即生效）
2. 在任意窗口划选包含英文术语的文本，按 `Alt+T`
3. 悬浮窗显示：译法 + 域标签 + 释义；多术语时点击英文词循环切换
4. ✅采纳 / ✏️修改 / ❌否决 —— 你的裁决写入个人层，下次秒出
5. `Esc` 或点击其它窗口即隐藏

## 配置

- `config.toml` / `fallback_prompt.md` / `seed_terms.csv` / `terms.db` 位于 `%APPDATA%\term-lens\`
  （首次运行自动创建；改 prompt 和配置**保存即生效**，无需重启）
- 云端 provider 默认 opencodex `http://127.0.0.1:10100/v1` → `opencode-go/deepseek-v4-flash`
- H1 合规：兜底默认**仅传术语单词**，不上传划选上下文

## 数据与统计

- 术语表导出：悬浮窗无入口，用命令 `export_terms`（可在 devtools 触发）或运行目录脚本；导出到 `%APPDATA%\term-lens\export_terms.csv`
- 命中率/延迟统计：`stats` 命令，数据源 `query_log` 表

## 构建

```bash
cd src-tauri
cargo build --release
cp target/release/term-lens.exe ../dist/
```

## 设计

见 `DESIGN.md`（架构与验收指标）与 `AGENTS.md`（代理工作规范）。
