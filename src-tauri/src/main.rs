// Term Lens — AI 输出中英混杂术语实时注释工具 (Windows MVP)
// 设计依据: ../DESIGN.md §0 第一性原理锚点 (P1-P4, H1-H2)

// release 构建以窗口子系统运行(无黑框); CLI 子命令启动时再 AttachConsole 回终端
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use enigo::{Enigo, Keyboard, Settings as EnigoSettings};
use regex::Regex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const DB_NAME: &str = "terms.db";

// 单实例保护: 先抢命名互斥体, 已有人持有则本进程立即退出
// (防止开机自启+手动双击撞车 → 悬浮窗双开/热键失效); CLI 子命令设 TERMLENS_CLI=1 跳过
fn ensure_single_instance() {
    if std::env::var("TERMLENS_CLI").is_ok() {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        #[allow(deprecated)]
        unsafe {
            let name: Vec<u16> = OsStr::new("Local\\TermLens-Main").encode_wide().chain(std::iter::once(0)).collect();
            let _handle = windows::Win32::System::Threading::CreateMutexW(
                None,
                false,
                windows::core::PCWSTR(name.as_ptr()),
            );
            if windows::Win32::Foundation::ERROR_ALREADY_EXISTS == windows::Win32::Foundation::GetLastError() {
                std::process::exit(0);
            }
        }
    }
}

// ---------- 配置 ----------

#[derive(Debug, Deserialize)]
#[serde(default)]
struct Config {
    provider: ProviderCfg,
    ui: UiCfg,
    hotkey: HotkeyCfg,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct HotkeyCfg {
    /// "double_ctrl" = 双击 Ctrl (默认) | "alt_t" = Alt+T
    mode: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct ProviderCfg {
    base_url: String,
    api_key: String,
    model: String,
    timeout_ms: u64,
    /// H1 数据出境最小化: 兜底默认仅传术语单词; 带上下文必须显式开启
    send_context: bool,
    context_chars: usize,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct UiCfg {
    popup_width: f64,
    popup_height: f64,
    /// "cursor" = 跟随鼠标(默认); "fixed" = 固定屏幕右下角
    position: String,
}

impl Default for Config {
    fn default() -> Self {
        Config { provider: ProviderCfg::default(), ui: UiCfg::default(), hotkey: HotkeyCfg::default() }
    }
}
impl Default for HotkeyCfg {
    fn default() -> Self {
        HotkeyCfg { mode: "double_ctrl".into() }
    }
}
impl Default for ProviderCfg {
    fn default() -> Self {
        ProviderCfg {
            base_url: "http://127.0.0.1:10100/v1".into(),
            api_key: "opencodex-local".into(),
            model: "opencode-go/deepseek-v4-flash".into(),
            timeout_ms: 15000,
            send_context: false, // H1
            context_chars: 0,
        }
    }
}
impl Default for UiCfg {
    fn default() -> Self {
        UiCfg { popup_width: 360.0, popup_height: 230.0, position: "cursor".into() }
    }
}

// ---------- 数据结构 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Term {
    en: String,
    zh: String,
    domain: String,
    ctx_hints: Vec<String>,
    keep_policy: String,
    note: String,
    layer: String,
    source: String,
    status: String,
}

struct AppState {
    db: Mutex<Connection>,
    last_selection: Mutex<String>,
    client: reqwest::Client,
}

// ---------- 路径 ----------

fn data_dir() -> PathBuf {
    let p = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("term-lens");
    fs::create_dir_all(&p).ok();
    p
}

fn write_if_absent(path: &PathBuf, content: &str) {
    if !path.exists() {
        fs::write(path, content).ok();
    }
}

const DEFAULT_CONFIG: &str = "# Term Lens 配置 (修改后自动生效: 每次兜底请求时重读)\n\
     # H1 硬约束: send_context 默认 false, 仅传术语单词; 开启前请确认合规\n\
     # 云端兜底可接任意 OpenAI 兼容 API: 改 base_url/model/api_key 即可\n\
     #   例: base_url = \"https://api.deepseek.com/v1\"  model = \"deepseek-chat\"\n\
     # 没有可用 API 也能用: 仅本地词表, 兜底会显示离线徽标\n\n\
     [provider]\n\
     base_url = \"http://127.0.0.1:10100/v1\"\n\
     api_key = \"opencodex-local\"\n\
     model = \"opencode-go/deepseek-v4-flash\"\n\
     timeout_ms = 15000\n\
     send_context = false\n\
     context_chars = 0\n\n\
     [ui]\n\
     popup_width = 360.0\n\
     popup_height = 230.0\n\
     # \"cursor\"=跟随鼠标(默认)  \"fixed\"=固定屏幕右下角\n\
     position = \"cursor\"\n\n\
     [hotkey]\n\
     # \"double_ctrl\"=双击Ctrl(默认)  \"alt_t\"=Alt+T\n\
     mode = \"double_ctrl\"\n";

fn load_config() -> Config {
    let path = data_dir().join("config.toml");
    write_if_absent(&path, DEFAULT_CONFIG);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

const FALLBACK_PROMPT: &str = "\
你是计算机/AI 领域术语词典。用户给出一个在中文技术语境(常见于 AI 编程助手的输出)中\
出现的英文术语，请给出最贴切的中文译法。要求：\n\
1. 优先采用行业通行译名(如 Agent→智能体, 禁用\"代理\"; hallucination→幻觉; \
smoke 在测试语境→冒烟测试; token 在 LLM 语境→词元, 安全语境→令牌)\n\
2. 若该词业内通常保留英文, keep_policy 用 \"keep\", zh 给出中文释义而非翻译\n\
3. note 用一句话解释该词在技术语境中的含义\n\
4. domain 从 llm/frontend/backend/security/devops/database/general 中选一个\n\
只输出一个 JSON 对象, 不要其它文字, 格式:\n\
{\"en\":\"\",\"zh\":\"\",\"domain\":\"\",\"note\":\"\",\"keep_policy\":\"translate|keep|note\"}\n";

fn load_prompt() -> String {
    let path = data_dir().join("fallback_prompt.md");
    if !path.exists() {
        fs::write(&path, FALLBACK_PROMPT).ok();
    }
    fs::read_to_string(&path).unwrap_or_else(|_| FALLBACK_PROMPT.to_string())
}

// ---------- 数据库 ----------

fn init_db(db_path: &PathBuf) -> Connection {
    let conn = Connection::open(db_path).expect("open sqlite");
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS terms (
           id INTEGER PRIMARY KEY,
           en TEXT NOT NULL,
           en_variants TEXT,
           zh TEXT NOT NULL,
           domain TEXT NOT NULL DEFAULT 'general',
           ctx_hints TEXT,
           keep_policy TEXT DEFAULT 'translate',
           note TEXT DEFAULT '',
           layer TEXT NOT NULL,
           source TEXT DEFAULT 'manual',
           status TEXT DEFAULT 'active',
           hit_count INTEGER DEFAULT 0,
           created_at TEXT DEFAULT (datetime('now','localtime')),
           updated_at TEXT DEFAULT (datetime('now','localtime')),
           UNIQUE(en, domain, layer)
         );
         CREATE INDEX IF NOT EXISTS idx_terms_en ON terms(en);
         CREATE TABLE IF NOT EXISTS query_log (
           id INTEGER PRIMARY KEY,
           en TEXT NOT NULL,
           ts TEXT DEFAULT (datetime('now','localtime')),
           layer_hit TEXT,
           latency_ms INTEGER
         );",
    )
    .expect("init schema");
    conn
}

const LAYER_RANK: &[&str] = &["ms", "ai", "personal"];
fn layer_rank(layer: &str) -> i32 {
    LAYER_RANK.iter().position(|l| *l == layer).unwrap_or(0) as i32
}

fn row_to_term(row: &rusqlite::Row) -> rusqlite::Result<Term> {
    // ctx_hints 可能为 NULL (adopt/fix 未填该列) —— 必须按 Option 读, 否则整行被静默丢弃
    let hints: Option<String> = row.get(4)?;
    let ctx_hints: Vec<String> = hints
        .and_then(|h| serde_json::from_str(&h).ok())
        .unwrap_or_default();
    Ok(Term {
        en: row.get(0)?,
        zh: row.get(1)?,
        domain: row.get(2)?,
        keep_policy: row.get(3)?,
        ctx_hints,
        note: row.get(5)?,
        layer: row.get(6)?,
        source: row.get(7)?,
        status: row.get(8)?,
    })
}

const SELECT_COLS: &str =
    "SELECT en, zh, domain, keep_policy, ctx_hints, note, layer, source, status FROM terms";

fn lookup_local(conn: &Connection, en: &str) -> Vec<Term> {
    let key = en.to_lowercase();
    let mut stmt = conn
        .prepare(&format!(
            "{SELECT_COLS} WHERE (en = ?1 OR en_variants LIKE '%\"' || ?1 || '\"%') AND status IN ('active','pending') \
             ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END, hit_count DESC"
        ))
        .unwrap();
    let rows = stmt
        .query_map([&key], |r| row_to_term(r))
        .unwrap()
        .filter_map(|x| x.ok())
        .collect();
    rows
}

fn pick_term(terms: &mut Vec<Term>, context: &str) -> Option<Term> {
    if terms.is_empty() {
        return None;
    }
    terms.sort_by_key(|t| -layer_rank(&t.layer));
    if terms.len() == 1 {
        return Some(terms.remove(0));
    }
    let ctx = context.to_lowercase();
    let scored: Vec<(i32, usize)> = terms
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let score = if t.domain == "general" { 0 } else { t.ctx_hints.iter().filter(|h| ctx.contains(&h.to_lowercase())).count() as i32 };
            (score, i)
        })
        .collect();
    let (best, idx) = *scored.iter().max_by_key(|(s, _)| *s).unwrap();
    if best > 0 {
        Some(terms.remove(idx))
    } else {
        Some(terms.remove(0))
    }
}

fn upsert(conn: &Connection, t: &Term) {
    conn.execute(
        "INSERT INTO terms (en, zh, domain, ctx_hints, keep_policy, note, layer, source, status)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(en, domain, layer) DO UPDATE SET
           zh = excluded.zh, note = excluded.note, keep_policy = excluded.keep_policy,
           status = excluded.status, updated_at = datetime('now','localtime')",
        rusqlite::params![
            t.en.to_lowercase(), t.zh, t.domain,
            serde_json::to_string(&t.ctx_hints).unwrap_or("[]".into()),
            t.keep_policy, t.note, t.layer, t.source, t.status
        ],
    )
    .ok();
}

// 种子词库编译期内嵌 (分享版无仓库路径也能完整初始化)
const SEED_CLASSIC: &str = include_str!("../../data/seed_terms.csv");
const SEED_AI: &str = include_str!("../../data/ai_terms_latest.csv");

fn seed_if_empty(conn: &Connection) -> usize {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM terms", [], |r| r.get(0))
        .unwrap_or(0);
    if count > 0 {
        return 0;
    }
    // 落盘一份到数据目录 (便于用户查看/编辑), 但入库读内嵌内容
    let _ = fs::write(data_dir().join("seed_terms.csv"), format!("{SEED_CLASSIC}{SEED_AI}"));
    let mut n = 0usize;
    for line in SEED_CLASSIC.lines().skip(1).chain(SEED_AI.lines().skip(1)) {
        if line.trim().is_empty() {
            continue;
        }
        // CSV: en,zh,domain,ctx_hints(json),keep_policy,note,layer,source
        if let Some(t) = parse_seed_line(line) {
            upsert(conn, &t);
            n += 1;
        }
    }
    n
}

fn parse_seed_line(line: &str) -> Option<Term> {
    // 标准 CSV 单行解析（引号状态机; 字段内无换行）
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => { cur.push('"'); chars.next(); }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    parts.push(cur);
    if parts.len() < 8 {
        return None;
    }
    let ctx_hints: Vec<String> = serde_json::from_str(&parts[3]).unwrap_or_default();
    Some(Term {
        en: parts[0].trim().to_string(),
        zh: parts[1].trim().to_string(),
        domain: parts[2].trim().to_string(),
        ctx_hints,
        keep_policy: parts[4].trim().to_string(),
        note: parts[5].trim().to_string(),
        layer: parts[6].trim().to_string(),
        source: parts[7].trim().to_string(),
        status: "active".into(),
    })
}

// ---------- 取词与文本提取 ----------

fn extract_terms(text: &str) -> Vec<String> {
    let re = Regex::new(r"(?i)\b([A-Za-z][A-Za-z0-9]*(?:[ _\-][A-Za-z][A-Za-z0-9]*)?)\b").unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for cap in re.captures_iter(text) {
        let w = cap[1].to_string();
        let lw = w.to_lowercase();
        if lw.len() < 2 || seen.contains(&lw) {
            continue;
        }
        seen.insert(lw.clone());
        out.push(w);
    }
    out.truncate(8);
    out
}

fn normalize(en: &str) -> String {
    let s = en.trim().to_lowercase();
    if s.ends_with("s") && !s.ends_with("ss") {
        let singular = s.trim_end_matches('s').to_string();
        if !singular.is_empty() {
            return singular;
        }
    }
    s
}

// ---------- 云端兜底 (H1: 仅传术语) ----------

#[derive(Serialize)]
struct ChatReq {
    model: String,
    messages: Vec<Msg>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Msg {
    role: String,
    content: String,
}

async fn cloud_lookup(client: &reqwest::Client, en: &str) -> Result<Option<Term>, String> {
    let cfg = load_config(); // 热重载: 每次读取
    let prompt = load_prompt();
    let body = ChatReq {
        model: cfg.provider.model.clone(),
        messages: vec![
            Msg { role: "system".into(), content: prompt },
            Msg { role: "user".into(), content: en.to_string() }, // H1: 仅术语单词
        ],
        max_tokens: Some(300),
    };
    let url = format!("{}/chat/completions", cfg.provider.base_url.trim_end_matches('/'));
    // 上游路由偶发慢响应: 失败自动重试一次, 平滑超时尖峰
    let mut last_err = String::new();
    for attempt in 0..2 {
        let r = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", cfg.provider.api_key))
            .json(&body)
            .timeout(std::time::Duration::from_millis(cfg.provider.timeout_ms))
            .send()
            .await;
        match r {
            Ok(resp) => {
                if !resp.status().is_success() {
                    last_err = format!("HTTP {}", resp.status());
                    continue;
                }
                let v: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(e) => { last_err = format!("响应解析失败: {e}"); continue; }
                };
                let content = match v["choices"][0]["message"]["content"].as_str() {
                    Some(s) => s.to_string(),
                    None => { last_err = "响应格式错误".into(); continue; }
                };
                // 提取 JSON (容忍包裹 ```json)
                let json_str = match content
                    .find('{')
                    .and_then(|i| content[i..].rfind('}').map(|j| &content[i..i + j + 1]))
                {
                    Some(s) => s,
                    None => { last_err = "无 JSON".into(); continue; }
                };
                #[derive(Deserialize)]
                struct CloudTerm {
                    zh: String,
                    #[serde(default)]
                    domain: String,
                    #[serde(default)]
                    note: String,
                    #[serde(default)]
                    keep_policy: String,
                }
                let ct: CloudTerm = match serde_json::from_str(json_str) {
                    Ok(t) => t,
                    Err(e) => { last_err = format!("JSON 解析失败: {e}"); continue; }
                };
                let _ = attempt;
                return Ok(Some(Term {
                    en: en.to_string(),
                    zh: ct.zh,
                    domain: if ct.domain.is_empty() { "general".into() } else { ct.domain },
                    ctx_hints: vec![],
                    keep_policy: if ct.keep_policy.is_empty() { "translate".into() } else { ct.keep_policy },
                    note: ct.note,
                    layer: "personal".into(),
                    source: "cloud-adopted".into(),
                    status: "pending".into(), // 云端必经确认, 防幻觉污染
                }));
            }
            Err(e) => { last_err = format!("请求失败: {}", err_chain(&e)); continue; }
        }
    }
    Err(last_err)
}

// ---------- 双击 Ctrl 低级键盘钩子 ----------

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

static LAST_CTRL_UP: AtomicU64 = AtomicU64::new(0);
static TRIGGER_AT: AtomicU64 = AtomicU64::new(0);
static HOOK_APP: Mutex<Option<AppHandle>> = Mutex::new(None);
// 触发方式全局开关: 0=双击 Ctrl  1=Alt+T
// 托盘菜单可运行时切换 (写 config.toml + 热注册/注销), 无需重启
static HOTKEY_MODE: AtomicU8 = AtomicU8::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(target_os = "windows")]
fn tl_log(msg: &str) {
    use std::io::Write;
    let path = data_dir().join("term-lens.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {}", now_ms(), msg);
    }
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn ll_hook_proc(code: i32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_LCONTROL, VK_RCONTROL};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, WM_KEYUP,
    };
    if code >= 0 {
        // Alt+T 模式激活时, 双击 Ctrl 不应响应 (托盘可即时切换)
        if HOTKEY_MODE.load(Ordering::SeqCst) != 0 {
            return CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam);
        }
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        // 跳过注入事件: 我们自己模拟的 Ctrl+C 不能触发钩子 (否则复制→又触发→死循环)
        let injected = (kb.flags.0 & LLKHF_INJECTED.0) != 0;
        // 低级钩子报告具体的左右键码 (VK_LCONTROL/VK_RCONTROL), 通用 VK_CONTROL 收不到左 Ctrl
        let vk = kb.vkCode;
        let is_ctrl = vk == VK_LCONTROL.0 as u32 || vk == VK_RCONTROL.0 as u32 || vk == VK_CONTROL.0 as u32;
        if !injected && is_ctrl && wparam.0 as u32 == WM_KEYUP {
            let now = now_ms();
            let last = LAST_CTRL_UP.swap(now, Ordering::SeqCst);
            // 连按三次(第三下距触发<350ms)不重复触发
            if last != 0 && now - last < 350 && now - TRIGGER_AT.load(Ordering::SeqCst) > 350 {
                TRIGGER_AT.store(now, Ordering::SeqCst);
                LAST_CTRL_UP.store(0, Ordering::SeqCst);
                let app = HOOK_APP.lock().unwrap().clone();
                if let Some(app) = app {
                    tl_log("double_ctrl triggered");
                    // 钩子回调里不能做耗时操作(阻塞全局输入), 丢给独立线程
                    std::thread::spawn(move || grab_selection_and_show(&app));
                }
            }
        }
    }
    CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam)
}

#[cfg(target_os = "windows")]
fn start_double_ctrl_hook(app: AppHandle) {
    std::thread::spawn(move || {
        use windows::Win32::Foundation::HINSTANCE;
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, MSG,
            WH_KEYBOARD_LL,
        };
        *HOOK_APP.lock().unwrap() = Some(app);
        unsafe {
            let hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(ll_hook_proc),
                HINSTANCE(std::ptr::null_mut()),
                0,
            );
            match hook {
                Ok(_h) => {
                    tl_log("hook installed, pumping");
                    let mut msg = MSG::default();
                    // 消息泵: 低级钩子回调依赖安装线程持续泵消息
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                Err(e) => tl_log(&format!("hook install FAILED: {e}")),
            }
        }
    });
}

// ---------- Tauri 命令 ----------

#[derive(Serialize)]
struct LookupResult {
    hit: Option<Term>,
    candidates: Vec<Term>,
}

#[tauri::command]
fn lookup(state: State<AppState>, en: String) -> LookupResult {
    let ctx = state.last_selection.lock().unwrap().clone();
    let t0 = std::time::Instant::now();
    let mut found = lookup_local(&state.db.lock().unwrap(), en.trim());
    // 归一化重试 (复数)
    if found.is_empty() {
        let norm = normalize(&en);
        if norm != en.to_lowercase() {
            found = lookup_local(&state.db.lock().unwrap(), &norm);
        }
    }
    let candidates = found.clone();
    let hit = pick_term(&mut found.to_vec(), &ctx);
    let latency = t0.elapsed().as_millis() as i64;
    if let Some(h) = &hit {
        let conn = state.db.lock().unwrap();
        conn.execute("UPDATE terms SET hit_count = hit_count + 1 WHERE en = ?1", rusqlite::params![h.en.to_lowercase()]).ok();
        conn.execute("INSERT INTO query_log(en, layer_hit, latency_ms) VALUES (?1,?2,?3)", rusqlite::params![h.en.to_lowercase(), h.layer, latency]).ok();
    } else {
        let conn = state.db.lock().unwrap();
        conn.execute("INSERT INTO query_log(en, layer_hit, latency_ms) VALUES (?1,'miss',?2)", rusqlite::params![en.to_lowercase(), latency]).ok();
    }
    LookupResult { hit, candidates }
}

#[derive(Serialize)]
struct FallbackResp {
    result: Option<Term>,
    offline: bool,
    error: Option<String>,
}

#[tauri::command]
async fn fallback(app: AppHandle, en: String) -> Result<FallbackResp, String> {
    let st = app.state::<AppState>();
    let client = st.client.clone();
    let r = cloud_lookup(&client, &en).await;
    Ok(match r {
        Ok(Some(t)) => {
            // 云端结果落库为 pending: 同词再查直接本地命中"待确认", 不重复打云端 (DESIGN §pending 语义)
            let conn = st.db.lock().unwrap();
            upsert(&conn, &t);
            FallbackResp { result: Some(t), offline: false, error: None }
        }
        Ok(None) => FallbackResp { result: None, offline: false, error: None },
        Err(e) => FallbackResp { result: None, offline: true, error: Some(e) },
    })
}

#[tauri::command]
fn adopt(state: State<AppState>, en: String, zh: String, domain: String, note: String) {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "INSERT INTO terms (en, zh, domain, keep_policy, note, layer, source, status)
         VALUES (?1,?2,?3,'translate',?4,'personal','user-adopted','active')
         ON CONFLICT(en, domain, layer) DO UPDATE SET
           zh = excluded.zh, note = excluded.note, updated_at = datetime('now','localtime')",
        rusqlite::params![en.to_lowercase(), zh, domain, note],
    )
    .ok();
}

#[tauri::command]
fn fix(state: State<AppState>, en: String, zh: String) {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "INSERT INTO terms (en, zh, domain, keep_policy, note, layer, source, status)
         VALUES (?1,?2,'general','translate','用户手工裁决','personal','user-fixed','active')
         ON CONFLICT(en, domain, layer) DO UPDATE SET
           zh = excluded.zh, note = excluded.note, status='active', updated_at = datetime('now','localtime')",
        rusqlite::params![en.to_lowercase(), zh],
    )
    .ok();
}

#[tauri::command]
fn reject(state: State<AppState>, en: String) {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "UPDATE terms SET status='rejected', updated_at=datetime('now','localtime') WHERE en=?1 AND status IN ('active','pending')",
        rusqlite::params![en.to_lowercase()],
    )
    .ok();
    conn.execute(
        "INSERT INTO terms (en, zh, domain, layer, source, status) VALUES (?1,'(已否决)','general','personal','user-rejected','rejected')
         ON CONFLICT(en, domain, layer) DO UPDATE SET status='rejected'",
        rusqlite::params![en.to_lowercase()],
    )
    .ok();
}

fn export_db(conn: &Connection) -> String {
    let out_path = data_dir().join("export_terms.csv");
    let mut w = std::io::BufWriter::new(fs::File::create(&out_path).unwrap());
    use std::io::Write;
    writeln!(w, "en,zh,domain,ctx_hints,keep_policy,note,layer,source").ok();
    let mut stmt = conn
        .prepare("SELECT en, zh, domain, ctx_hints, keep_policy, note, layer, source FROM terms WHERE status='active' ORDER BY en")
        .unwrap();
    let mut rows = stmt.query_map([], |r| {
        let s: (String, String, String, String, String, String, String, String) =
            (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?);
        Ok(s)
    }).unwrap();
    while let Some(Ok(r)) = rows.next() {
        let esc = |s: String| if s.contains(',') || s.contains('"') { format!("\"{}\"", s.replace('"', "\"\"")) } else { s };
        writeln!(w, "{},{},{},{},{},{},{},{}", esc(r.0), esc(r.1), esc(r.2), esc(r.3), esc(r.4), esc(r.5), esc(r.6), esc(r.7)).ok();
    }
    out_path.to_string_lossy().to_string()
}

fn import_csv(conn: &Connection, path: &PathBuf) -> usize {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let mut n = 0usize;
    for line in content.lines().skip(1) {
        if line.trim().is_empty() { continue; }
        if let Some(t) = parse_seed_line(line) {
            if t.layer == "personal" {
                continue; // 外部导入永远不得写入/覆盖个人裁决层
            }
            // 已存在则跳过 (防新库译名覆盖旧裁决, 如 token→词元); 只增不改
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM terms WHERE en=?1 AND domain=?2 AND layer=?3",
                rusqlite::params![t.en.to_lowercase(), t.domain, t.layer],
                |r| r.get(0)).unwrap_or(0);
            if exists > 0 { continue; }
            let mut t = t;
            t.status = "active".into();
            upsert(conn, &t);
            n += 1;
        }
    }
    n
}

fn stats_db(conn: &Connection) -> serde_json::Value {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM query_log", [], |r| r.get(0)).unwrap_or(0);
    let hits: i64 = conn.query_row("SELECT COUNT(*) FROM query_log WHERE layer_hit NOT IN ('miss','')", [], |r| r.get(0)).unwrap_or(0);
    let terms: i64 = conn.query_row("SELECT COUNT(*) FROM terms WHERE status='active'", [], |r| r.get(0)).unwrap_or(0);
    let p95: i64 = conn.query_row(
        "SELECT latency_ms FROM query_log WHERE layer_hit!='miss' ORDER BY latency_ms LIMIT 1 OFFSET MAX((SELECT COUNT(*) FROM query_log WHERE layer_hit!='miss')/20, 0)",
        [], |r| r.get(0)).unwrap_or(0);
    serde_json::json!({
        "queries_total": total,
        "local_hit_rate": if total > 0 { (hits as f64 / total as f64 * 10000.0).round() / 10000.0 } else { 0.0 },
        "active_terms": terms,
        "local_p95_ms": p95,
        "export_path": data_dir().join("export_terms.csv").to_string_lossy(),
    })
}

#[tauri::command]
fn export_terms(state: State<AppState>) -> String {
    let conn = state.db.lock().unwrap();
    export_db(&conn)
}

#[tauri::command]
fn stats(state: State<AppState>) -> serde_json::Value {
    let conn = state.db.lock().unwrap();
    stats_db(&conn)
}

// ---------- 悬浮窗与热键 ----------

fn get_cursor_pos() -> (f64, f64) {
    #[cfg(target_os = "windows")]
    {
        #[allow(deprecated)]
        unsafe {
            let mut pt = windows::Win32::Foundation::POINT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
            return (pt.x as f64, pt.y as f64);
        }
    }
    #[allow(unreachable_code)]
    (400.0, 300.0)
}

/// 等待 Alt/Ctrl 修饰键物理释放 (热键触发时 Alt 还按着, 直接发 Ctrl+C 会变成 Alt+Ctrl+C 导致复制失败)
fn wait_modifiers_released() {
    #[cfg(target_os = "windows")]
    {
        use std::time::Instant;
        let t0 = Instant::now();
        while t0.elapsed().as_millis() < 500 {
            #[allow(deprecated)]
            let down = unsafe {
                use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_MENU};
                (GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000 != 0)
                    || (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0)
            };
            if !down {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

/// reqwest 错误链展开, 便于定位连接层根因
fn err_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(x) = src {
        s.push_str(&format!(" | {x}"));
        src = x.source();
    }
    s
}

fn grab_selection_and_show(app: &AppHandle) {
    // 0. 等修饰键松开
    wait_modifiers_released();

    // 1. 保存剪贴板快照
    let saved = arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .ok();

    // 2. 模拟 Ctrl+C 复制选中文本
    let mut enigo = Enigo::new(&EnigoSettings::default()).unwrap();
    use enigo::Direction::{Press, Release};
    enigo.key(enigo::Key::Control, Press);
    enigo.key(enigo::Key::Unicode('c'), Press);
    enigo.key(enigo::Key::Unicode('c'), Release);
    enigo.key(enigo::Key::Control, Release);

    // 3. 轮询读新剪贴板 (最多 400ms, 慢应用复制有延迟)
    let mut selected = String::new();
    for _ in 0..13 {
        std::thread::sleep(std::time::Duration::from_millis(30));
        let cur = arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .unwrap_or_default();
        if cur != saved.clone().unwrap_or_default() {
            selected = cur;
            break;
        }
        if saved.is_none() && !cur.is_empty() {
            selected = cur;
            break;
        }
        selected = cur; // 兜底: 剪贴板没变也用当前值
    }

    // 4. 恢复剪贴板 (P1: 不污染用户复制)
    if let Some(old) = saved {
        if let Ok(mut c) = arboard::Clipboard::new() {
            let _ = c.set_text(old);
        }
    }

    // 记录划选文本作为语境（供多候选域裁决；H1: 仅存本地，绝不上云）
    if let Some(st) = app.try_state::<AppState>() {
        *st.last_selection.lock().unwrap() = selected.clone();
    }

    let terms = extract_terms(&selected);
    let app2 = app.clone();
    let (px, py) = get_cursor_pos(); // 物理像素
    let fixed = load_config().ui.position == "fixed";
    let win: Option<WebviewWindow> = app2.get_webview_window("main");

    tauri::async_runtime::spawn(async move {
        if let Some(w) = win {
            let scale = w.scale_factor().unwrap_or(1.0);
            let (pw, ph) = w.outer_size()
                .map(|s| (s.width as f64, s.height as f64))
                .unwrap_or((360.0 * scale, 230.0 * scale));
            let (sw, sh) = w.primary_monitor().ok().flatten()
                .map(|m| {
                    let sz = m.size();
                    (sz.width as f64, sz.height as f64)
                })
                .unwrap_or((1920.0 * scale, 1080.0 * scale));
            // 目标物理坐标: fixed=右下角(留边距), cursor=鼠标右下偏移
            let (fx, fy) = if fixed {
                (sw - pw - 24.0 * scale, sh - ph - 60.0 * scale)
            } else {
                (px + 8.0 * scale, py + 16.0 * scale)
            };
            // 夹取到屏幕内, 再换算为逻辑像素 (高 DPI 屏上物理/逻辑混用会跑偏)
            let cx = fx.max(0.0).min((sw - pw).max(0.0)) / scale;
            let cy = fy.max(0.0).min((sh - ph).max(0.0)) / scale;
            let _ = w.set_position(tauri::LogicalPosition::new(cx, cy));
            let _ = w.emit("terms-requested", &terms);
            let _ = w.show();
            let _ = w.set_focus();
        }
    });
}

// ================= 系统托盘 (分组右键菜单) =================
//
// 菜单结构 (所有"查看/修改"入口按配置域分组):
//   翻译当前选中内容             → 与热键同一条触发链路, 即时取词
//   显示悬浮窗
//   ─ 词库           查看统计 / 导出CSV / 从数据目录导入CSV / 打开数据目录
//   ─ 提示词         查看编辑 (fallback_prompt.md) / 恢复默认
//   ─ 大模型连接     查看配置 / 测试连接 / 编辑配置 (config.toml) / 恢复默认
//   ─ 界面与热键     弹窗位置(勾选) / 触发方式(勾选) / 弹窗尺寸提示
//   ─ 退出
//
// 即时生效原则 (改完即用, 无需重启):
//   * provider.* + fallback_prompt.md → 每次云端请求前重读 (cloud_lookup/load_prompt)
//   * ui.position                     → 每次触发悬浮时重读 (grab_selection_and_show)
//   * hotkey.mode                     → HOTKEY_MODE 开关 + 全局快捷键注册/注销, 运行时切换
//   所有变更同时落盘 config.toml, 重启后保持。

fn mi<'a>(item: &'a impl tauri::menu::IsMenuItem<tauri::Wry>) -> &'a dyn tauri::menu::IsMenuItem<tauri::Wry> {
    item
}

/// 不丢注释地改写 config.toml 中某字段值 (保注释 = 用户自己写的说明不会被清掉)
fn patch_config_field(key: &str, value: &str) -> Result<(), String> {
    let path = data_dir().join("config.toml");
    let raw = fs::read_to_string(&path).map_err(|e| format!("读取配置失败: {e}"))?;
    let re = Regex::new(&format!(r#"(?m)^([ \t]*{0}[ \t]*=[ \t]*)"[^"]*""#, regex::escape(key)))
        .map_err(|e| e.to_string())?;
    if !re.is_match(&raw) {
        return Err(format!("配置中找不到 {key} 字段, 请手动编辑 config.toml"));
    }
    let out = re.replace(&raw, format!("$1\"{value}\"")).to_string();
    fs::write(&path, out).map_err(|e| format!("写回配置失败: {e}"))
}

// 保注释重写整个配置文件 (恢复默认用)
fn write_default_config() -> Result<(), String> {
    let path = data_dir().join("config.toml");
    fs::write(&path, DEFAULT_CONFIG).map_err(|e| e.to_string())
}

fn write_default_prompt() -> Result<(), String> {
    let path = data_dir().join("fallback_prompt.md");
    fs::write(&path, FALLBACK_PROMPT).map_err(|e| e.to_string())
}

// ---------- 原生提示框 (零新依赖: 复用已引入的 windows crate) ----------

fn native_box(title: &str, text: &str, yesno: bool) -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDNO, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION,
            MB_OK, MB_YESNO, MESSAGEBOX_STYLE, MESSAGEBOX_RESULT,
        };
        let enc = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let t = enc(title);
        let b = enc(text);
        let style = if yesno { MB_YESNO | MB_ICONQUESTION } else { MB_OK | MB_ICONINFORMATION };
        let r: MESSAGEBOX_RESULT = unsafe {
            MessageBoxW(None, windows::core::PCWSTR(b.as_ptr()), windows::core::PCWSTR(t.as_ptr()), style)
        };
        return if yesno { Some(r == IDYES) } else { Some(r != MESSAGEBOX_RESULT(0)) };
    }
    #[cfg(not(target_os = "windows"))]
    {
        println!("[{title}] {text}");
        Some(true)
    }
}

fn info_box(title: &str, text: &str) {
    native_box(&format!("TermLens - {title}"), text, false);
}
fn err_box(title: &str, text: &str) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK, MESSAGEBOX_STYLE};
        let enc = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let t = enc(&format!("TermLens - {title}"));
        let b = enc(text);
        unsafe {
            MessageBoxW(None, windows::core::PCWSTR(b.as_ptr()), windows::core::PCWSTR(t.as_ptr()), MB_OK | MB_ICONERROR);
        }
    }
    #[cfg(not(target_os = "windows"))]
    println!("[TermLens - {title}] {text}");
}
fn confirm_box(title: &str, text: &str) -> bool {
    native_box(&format!("TermLens - {title}"), text, true).unwrap_or(false)
}

// ---------- 文件/目录打开 ----------

fn open_in_notepad(path: &std::path::Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("notepad").arg(path).spawn().is_ok();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        false
    }
}

fn explorer_select(path: &std::path::Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .is_ok();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        false
    }
}

fn open_dir(path: &std::path::Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("explorer").arg(path).spawn().is_ok();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        false
    }
}

// ---------- 分组菜单构建 ----------

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let cfg = load_config();
    let hk_alt = cfg.hotkey.mode == "alt_t";
    let pos_fixed = cfg.ui.position == "fixed";

    // 顶层
    let m_translate = MenuItem::with_id(app, "act_translate", "翻译当前选中内容", true, None::<&str>)?;
    let m_show = MenuItem::with_id(app, "act_show", "显示悬浮窗", true, None::<&str>)?;
    let m_quit = MenuItem::with_id(app, "quit", "退出 TermLens", true, None::<&str>)?;
    let sep_a = PredefinedMenuItem::separator(app)?;
    let sep_b = PredefinedMenuItem::separator(app)?;

    // ─ 词库
    let lib_stats = MenuItem::with_id(app, "lib_stats", "查看词库统计", true, None::<&str>)?;
    let lib_export = MenuItem::with_id(app, "lib_export", "导出词库 CSV (并打开所在目录)", true, None::<&str>)?;
    let lib_rescan = MenuItem::with_id(app, "lib_rescan", "从数据目录导入 CSV 词条", true, None::<&str>)?;
    let sep_lib = PredefinedMenuItem::separator(app)?;
    let lib_open = MenuItem::with_id(app, "lib_open", "打开词库数据目录", true, None::<&str>)?;
    let sub_lib = Submenu::with_items(app, "词库", true, &[
        mi(&lib_stats), mi(&lib_export), mi(&lib_rescan), mi(&sep_lib), mi(&lib_open),
    ])?;

    // ─ 提示词
    let pr_view = MenuItem::with_id(app, "pr_view", "查看 / 编辑提示词文件", true, None::<&str>)?;
    let pr_reset = MenuItem::with_id(app, "pr_reset", "恢复默认提示词", true, None::<&str>)?;
    let sub_pr = Submenu::with_items(app, "提示词 (云端兜底词典)", true, &[mi(&pr_view), mi(&pr_reset)])?;

    // ─ 大模型连接
    let cn_view = MenuItem::with_id(app, "cn_view", "查看当前连接配置", true, None::<&str>)?;
    let cn_test = MenuItem::with_id(app, "cn_test", "测试云端连接", true, None::<&str>)?;
    let cn_edit = MenuItem::with_id(app, "cn_edit", "编辑配置文件 config.toml", true, None::<&str>)?;
    let cn_reset = MenuItem::with_id(app, "cn_reset", "恢复默认配置", true, None::<&str>)?;
    let sub_cn = Submenu::with_items(app, "大模型连接", true, &[
        mi(&cn_view), mi(&cn_test), mi(&cn_edit), mi(&cn_reset),
    ])?;

    // ─ 界面与热键
    let t_pos = MenuItem::with_id(app, "t_pos", "弹窗位置", false, None::<&str>)?;
    let pos_cursor = CheckMenuItem::with_id(app, "pos_cursor", "跟随鼠标 (默认)", true, !pos_fixed, None::<&str>)?;
    let pos_fixed_c = CheckMenuItem::with_id(app, "pos_fixed", "固定屏幕右下角", true, pos_fixed, None::<&str>)?;
    let sep_ui = PredefinedMenuItem::separator(app)?;
    let t_hk = MenuItem::with_id(app, "t_hk", "触发方式", false, None::<&str>)?;
    let hk_double = CheckMenuItem::with_id(app, "hk_double", "双击 Ctrl", true, !hk_alt, None::<&str>)?;
    let hk_alt_c = CheckMenuItem::with_id(app, "hk_alt", "Alt + T", true, hk_alt, None::<&str>)?;
    let sep_ui2 = PredefinedMenuItem::separator(app)?;
    let ui_size = MenuItem::with_id(app, "ui_size", "悬浮窗尺寸: 改 config.toml [ui]", false, None::<&str>)?;
    let sub_ui = Submenu::with_items(app, "界面与热键", true, &[
        mi(&t_pos), mi(&pos_cursor), mi(&pos_fixed_c), mi(&sep_ui),
        mi(&t_hk), mi(&hk_double), mi(&hk_alt_c), mi(&sep_ui2), mi(&ui_size),
    ])?;

    let menu = Menu::with_items(app, &[
        mi(&m_translate), mi(&m_show), mi(&sep_a),
        mi(&sub_lib), mi(&sub_pr), mi(&sub_cn), mi(&sub_ui),
        mi(&sep_b), mi(&m_quit),
    ])?;
    Ok(menu)
}

/// 托盘菜单重建 (勾选状态跟随 config 变化) —— 切换后即见新状态
fn refresh_tray_menu(app: &AppHandle) {
    match build_menu(app) {
        Ok(menu) => {
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_menu(Some(menu));
            }
        }
        Err(e) => tl_log(&format!("rebuild tray menu failed: {e}")),
    }
}

fn mask_api_key(k: &str) -> String {
    if k.len() <= 8 {
        return "****".into();
    }
    format!("{}…{}", &k[..4], &k[k.len() - 2..])
}

fn provider_summary() -> String {
    let cfg = load_config();
    let pr = data_dir().join("fallback_prompt.md");
    let pr_chars = fs::read_to_string(&pr).map(|s| s.chars().count()).unwrap_or(0);
    format!(
        "Base URL : {}\nModel    : {}\nAPI Key  : {}\n\nSend Ctx : {}  (H1 硬约束, 默认 false 仅传术语单词)\nTimeout  : {} ms\n\n提示词文件: fallback_prompt.md ({} 字符)\n修改后每次请求自动重读, 无需重启。\n数据目录: {}",
        cfg.provider.base_url,
        cfg.provider.model,
        mask_api_key(&cfg.provider.api_key),
        cfg.provider.send_context,
        cfg.provider.timeout_ms,
        pr_chars,
        data_dir().display(),
    )
}

fn stats_text() -> String {
    let conn = Connection::open(data_dir().join(DB_NAME)).ok();
    match conn {
        Some(c) => {
            let s = stats_db(&c);
            let total: i64 = c.query_row("SELECT COUNT(*) FROM terms", [], |r| r.get(0)).unwrap_or(0);
            let pending: i64 = c.query_row("SELECT COUNT(*) FROM terms WHERE status='pending'", [], |r| r.get(0)).unwrap_or(0);
            let personal: i64 = c.query_row("SELECT COUNT(*) FROM terms WHERE layer='personal'", [], |r| r.get(0)).unwrap_or(0);
            let by_domain: i64 = c.query_row("SELECT COUNT(DISTINCT en) FROM terms WHERE status='active'", [], |r| r.get(0)).unwrap_or(0);
            format!(
                "词条总数   : {} (活动 {} 个)\n  经典层 ms / AI 层 ai / 个人层 personal = {} 个个人词条\n待确认     : {} 个 (云端兜底结果, 可在悬浮窗\"采纳/否决\")\n\n本地命中率 : {:.2}%\n查询总次数 : {}  本地 P95: {} ms\n\n去重词条   : {}\n\n导出文件   : export_terms.csv\n数据目录   : {}",
                total,
                s["active_terms"].as_i64().unwrap_or(0),
                personal,
                pending,
                s["local_hit_rate"].as_f64().unwrap_or(0.0) * 100.0,
                s["queries_total"].as_i64().unwrap_or(0),
                s["local_p95_ms"].as_i64().unwrap_or(0),
                by_domain,
                data_dir().display(),
            )
        }
        None => "无法打开词库数据库".into(),
    }
}

// ---------- 菜单动作分发 ----------

fn tray_menu_event(app: &AppHandle, id: &str) {
    let st = app.state::<AppState>();
    match id {
        "act_translate" => grab_selection_and_show(app),
        "act_show" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        // ─ 词库
        "lib_stats" => info_box("词库统计", &stats_text()),
        "lib_export" => {
            let n: i64 = st.db.lock().unwrap()
                .query_row("SELECT COUNT(*) FROM terms WHERE status='active'", [], |r| r.get(0)).unwrap_or(0);
            let path = export_db(&st.db.lock().unwrap());
            let opened = explorer_select(&std::path::Path::new(&path));
            info_box("词库导出", &format!("已导出 {n} 条活动词条 →\n{path}\n\n{}", if opened { "已打开所在文件夹。" } else { "请手动到数据目录查看。" }));
        }
        "lib_rescan" => {
            let dir = data_dir();
            let mut total = 0usize;
            let mut files: Vec<(String, usize)> = Vec::new();
            if let Ok(rd) = fs::read_dir(&dir) {
                let mut names: Vec<std::path::PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().map(|x| x == "csv").unwrap_or(false))
                    .filter(|p| p.file_name().and_then(|n| n.to_str()).map(|n| n != "export_terms.csv").unwrap_or(false))
                    .collect();
                names.sort();
                for p in names {
                    let n = import_csv(&st.db.lock().unwrap(), &p);
                    if n > 0 {
                        total += n;
                        files.push((p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(), n));
                    }
                }
            }
            if files.is_empty() {
                info_box("词库导入", "没有新增词条。\n\n说明: 把 CSV 放到数据目录后再次点击此项即可导入; 已存在的词条不会被覆盖 (只增不改)。");
            } else {
                let list = files.iter().map(|(f, n)| format!("  {f}: +{n}")).collect::<Vec<_>>().join("\n");
                info_box("词库导入", &format!("共导入 {total} 条新词条:\n{list}\n\n数据目录: {}", dir.display()));
            }
        }
        "lib_open" => {
            if !open_dir(&data_dir()) {
                err_box("打开目录", &("无法打开资源管理器, 请手动访问:\n".to_owned() + &data_dir().display().to_string()));
            }
        }
        // ─ 提示词
        "pr_view" => {
            let p = data_dir().join("fallback_prompt.md");
            if !open_in_notepad(&p) {
                err_box("打开提示词", &("无法调用记事本, 请手动编辑:\n".to_owned() + &p.display().to_string()));
            }
        }
        "pr_reset" => {
            if confirm_box("恢复默认提示词", "将把 fallback_prompt.md 重置为内置默认词典规则。\n你自己的修改会丢失, 确认继续?") {
                match write_default_prompt() {
                    Ok(()) => info_box("恢复默认提示词", "已恢复默认。下一次云端兜底请求即使用默认提示词。"),
                    Err(e) => err_box("恢复失败", &e),
                }
            }
        }
        // ─ 大模型连接
        "cn_view" => info_box("大模型连接", &provider_summary()),
        "cn_test" => {
            // 独立线程发最小 chat 请求, 避免卡菜单事件循环
            let client = st.client.clone();
            tauri::async_runtime::spawn(async move {
                let cfg = load_config();
                let url = format!("{}/chat/completions", cfg.provider.base_url.trim_end_matches('/'));
                let body = serde_json::json!({
                    "model": cfg.provider.model,
                    "messages": [{"role":"user","content":"ping"}],
                    "max_tokens": 1,
                });
                let r = client.post(&url)
                    .header("Authorization", format!("Bearer {}", cfg.provider.api_key))
                    .json(&body)
                    .timeout(std::time::Duration::from_millis(cfg.provider.timeout_ms))
                    .send()
                    .await;
                match r {
                    Ok(resp) if resp.status().is_success() => {
                        info_box("连接测试", &format!("✓ 连接成功\n\nBase URL : {}\nModel    : {}\n\n响应状态: HTTP {}\n\n说明: 云端兜底仅在本地词库未命中时使用。", cfg.provider.base_url, cfg.provider.model, resp.status().as_u16()));
                    }
                    Ok(resp) => err_box("连接测试", &format!("云端返回 HTTP {}\n\nBase URL: {}\n请检查 config.toml 的 base_url / api_key / model。", resp.status().as_u16(), cfg.provider.base_url)),
                    Err(e) => err_box("连接测试", &format!("请求失败: {}\n\nBase URL: {}\n\n常见原因:\n1. 本地代理(opencodex)未启动\n2. base_url 不可达 (需要外网/代理)\n3. 网络中断", err_chain(&e), cfg.provider.base_url)),
                }
            });
        }
        "cn_edit" => {
            let p = data_dir().join("config.toml");
            if !open_in_notepad(&p) {
                err_box("打开配置", &("无法调用记事本, 请手动编辑:\n".to_owned() + &p.display().to_string()));
            }
        }
        "cn_reset" => {
            if confirm_box("恢复默认配置", "将 config.toml 重置为内置默认值 (本地 opencodex 代理)。\n你配置的 base_url/api_key/model 会被覆盖, 确认继续?") {
                match write_default_config() {
                    Ok(()) => {
                        HOTKEY_MODE.store(0, Ordering::SeqCst);
                        refresh_tray_menu(app);
                        info_box("恢复默认配置", "已恢复默认。\n- 大模型连接 / 弹窗位置: 下次使用即时生效\n- 触发方式: 已切回 双击 Ctrl (即时生效)");
                    }
                    Err(e) => err_box("恢复失败", &e),
                }
            }
        }
        // ─ 界面与热键 (写盘 + 重建菜单勾选 → 即时生效)
        "pos_cursor" | "pos_fixed" => {
            let v = if id == "pos_fixed" { "fixed" } else { "cursor" };
            match patch_config_field("position", v) {
                Ok(()) => {
                    refresh_tray_menu(app);
                    info_box("弹窗位置", if v == "fixed" { "已切换为: 固定屏幕右下角。\n下一次触发悬浮即生效。" } else { "已切换为: 跟随鼠标。\n下一次触发悬浮即生效。" });
                }
                Err(e) => err_box("切换失败", &e),
            }
        }
        "hk_double" | "hk_alt" => {
            let alt = id == "hk_alt";
            // 1) 落盘 (重启后保持)
            if let Err(e) = patch_config_field("mode", if alt { "alt_t" } else { "double_ctrl" }) {
                err_box("切换失败", &e);
                return;
            }
            // 2) 内存开关 + 全局快捷键注册/注销 (即时生效)
            let sc = Shortcut::new(Some(Modifiers::ALT), Code::KeyT);
            if alt {
                HOTKEY_MODE.store(1, Ordering::SeqCst);
                match app.global_shortcut().register(sc) {
                    Ok(()) => {}
                    Err(e) => err_box("注册失败", &format!("Alt+T 注册失败: {e}\n可能被其它程序占用。可保持双击 Ctrl 使用。")),
                }
            } else {
                HOTKEY_MODE.store(0, Ordering::SeqCst);
                let _ = app.global_shortcut().unregister(sc);
            }
            refresh_tray_menu(app);
            info_box("触发方式", if alt { "已切换为 Alt+T, 即时生效。\n\n说明: Alt+T 需在目标程序内先划选文本, 再按组合键取词翻译。" } else { "已切换为 双击 Ctrl, 即时生效。\n\n说明: 在任意程序划选文本后快速按两下 Ctrl 即可取词翻译。" });
        }
        "quit" => {
            if confirm_box("退出", "确定退出 TermLens?\n\n退出后热键与托盘将失效; 重新打开安装目录下的 TermLens.exe 即可恢复。") {
                app.exit(0);
            }
        }
        _ => {}
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let handle = app.handle();
    let menu = build_menu(handle)?;
    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_string();
            tray_menu_event(app, &id);
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击托盘 → 显示悬浮窗 (右键已弹出菜单)
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                if let Some(w) = tray.app_handle().get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(handle)?;
    Ok(())
}

fn main() {
    // CLI 子命令先于互斥体判断
    let arg = std::env::args().nth(1).unwrap_or_default();
    let is_cli = matches!(arg.as_str(), "--export" | "--stats" | "--rescan");
    if is_cli {
        std::env::set_var("TERMLENS_CLI", "1");
    }
    ensure_single_instance();

    // CLI 模式: 窗口子系统下 stdout 不接终端, 需在任何 println 前 AttachConsole 回父终端
    #[cfg(target_os = "windows")]
    {
        if is_cli {
            use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
            unsafe {
                let _ = AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }
    }

    let db_path = data_dir().join(DB_NAME);
    let conn = init_db(&db_path);
    let seeded = seed_if_empty(&conn);
    if seeded > 0 {
        println!("[term-lens] seeded {seeded} terms");
    }
    // 启动即落盘配置与提示词文件, 保证"可配置"立刻可见
    if is_cli {
        if arg == "--export" {
            let out = export_db(&conn);
            println!("exported -> {out}");
            return;
        }
        if arg == "--stats" {
            let s = stats_db(&conn);
            println!("{s:#}");
            return;
        }
        if arg == "--rescan" {
            // 重导种子/外部 CSV: term-lens --rescan <path.csv>
            let path = std::env::args().nth(2).map(PathBuf::from).unwrap_or(data_dir().join("seed_terms.csv"));
            let n = import_csv(&conn, &path);
            println!("imported {n} rows from {}", path.display());
            return;
        }
    }
    // GUI 模式: 窗口子系统本无黑框; debug 构建(控制台子系统)时隐藏
    #[cfg(all(target_os = "windows", debug_assertions))]
    {
        use windows::Win32::System::Console::GetConsoleWindow;
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        unsafe {
            let hwnd = GetConsoleWindow();
            if !hwnd.is_invalid() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
    load_config();
    load_prompt();

    // 系统代理绕过: reqwest 默认读 http_proxy(本机 Clash 会劫持 localhost), 必须关掉
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap();

    let state = AppState {
        db: Mutex::new(conn),
        last_selection: Mutex::new(String::new()),
        client,
    };

    let shortcut = Shortcut::new(Some(Modifiers::ALT), Code::KeyT);

    tauri::Builder::default()
        .manage(state)
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, _sc, ev| {
                    // 快捷键事件只在 Alt+T 已注册时产生; 双击 Ctrl 走低级钩子, 不经过这里
                    if ev.state() == ShortcutState::Pressed {
                        grab_selection_and_show(app);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            // 按 config 决定初始触发方式 (托盘菜单可运行时切换)
            let hk_alt = load_config().hotkey.mode == "alt_t";
            HOTKEY_MODE.store(if hk_alt { 1 } else { 0 }, Ordering::SeqCst);
            if hk_alt {
                if let Err(e) = app.global_shortcut().register(shortcut) {
                    tl_log(&format!("alt_t register failed: {e}"));
                }
            }
            // 双击 Ctrl 钩子线程常驻 (内部按 HOTKEY_MODE 自行决定是否响应), 保证可随时切回
            #[cfg(target_os = "windows")]
            start_double_ctrl_hook(app.handle().clone());
            // 系统托盘 (分组右键菜单)
            build_tray(app)?;
            tl_log("tray ready");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            lookup, fallback, adopt, fix, reject, export_terms, stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running TermLens");
}
