// Term Lens — AI 输出中英混杂术语实时注释工具 (Windows MVP)
// 设计依据: ../DESIGN.md §0 第一性原理锚点 (P1-P4, H1-H2)

use enigo::{Enigo, Keyboard, Settings as EnigoSettings};
use regex::Regex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

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
            timeout_ms: 8000,
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

fn load_config() -> Config {
    let path = data_dir().join("config.toml");
    write_if_absent(
        &path,
        "# Term Lens 配置 (修改后自动生效: 每次兜底请求时重读)\n\
         # H1 硬约束: send_context 默认 false, 仅传术语单词; 开启前请确认合规\n\n\
         [provider]\n\
         base_url = \"http://127.0.0.1:10100/v1\"\n\
         api_key = \"opencodex-local\"\n\
         model = \"opencode-go/deepseek-v4-flash\"\n\
         timeout_ms = 8000\n\
         send_context = false\n\
         context_chars = 0\n\n\
         [ui]\n\
         popup_width = 360.0\n\
         popup_height = 230.0\n\
         # \"cursor\"=跟随鼠标(默认)  \"fixed\"=固定屏幕右下角\n\
         position = \"cursor\"\n\n\
         [hotkey]\n\
         # \"double_ctrl\"=双击Ctrl(默认)  \"alt_t\"=Alt+T\n\
         mode = \"double_ctrl\"\n",
    );
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
            "{SELECT_COLS} WHERE (en = ?1 OR en_variants LIKE '%\"' || ?1 || '\"%') AND status='active' \
             ORDER BY hit_count DESC"
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

fn seed_if_empty(conn: &Connection) -> usize {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM terms", [], |r| r.get(0))
        .unwrap_or(0);
    if count > 0 {
        return 0;
    }
    let path = data_dir().join("seed_terms.csv");
    if !path.exists() {
        // 仓库内种子文件优先
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../data/seed_terms.csv");
        if repo.exists() {
            fs::copy(&repo, &path).ok();
        }
    }
    if !path.exists() {
        return 0;
    }
    let content = fs::read_to_string(&path).unwrap_or_default();
    let mut n = 0usize;
    for line in content.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        // CSV: en,zh,domain,ctx_hints(json),keep_policy,note,layer,source  (note 内允许逗号, 用简单解析)
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
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", cfg.provider.api_key))
        .json(&body)
        .timeout(std::time::Duration::from_millis(cfg.provider.timeout_ms))
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", err_chain(&e)))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("响应格式错误")?
        .to_string();
    // 提取 JSON (容忍包裹 ```json)
    let json_str = content
        .find('{')
        .and_then(|i| content[i..].rfind('}').map(|j| &content[i..i + j + 1]))
        .ok_or("无 JSON")?;
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
    let ct: CloudTerm = serde_json::from_str(json_str).map_err(|e| format!("JSON 解析失败: {e}"))?;
    Ok(Some(Term {
        en: en.to_string(),
        zh: ct.zh,
        domain: if ct.domain.is_empty() { "general".into() } else { ct.domain },
        ctx_hints: vec![],
        keep_policy: if ct.keep_policy.is_empty() { "translate".into() } else { ct.keep_policy },
        note: ct.note,
        layer: "personal".into(),
        source: "cloud-adopted".into(),
        status: "pending".into(), // 云端必经确认, 防幻觉污染
    }))
}

// ---------- 双击 Ctrl 低级键盘钩子 ----------

use std::sync::atomic::{AtomicU64, Ordering};

static LAST_CTRL_UP: AtomicU64 = AtomicU64::new(0);
static TRIGGER_AT: AtomicU64 = AtomicU64::new(0);
static HOOK_APP: Mutex<Option<AppHandle>> = Mutex::new(None);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn ll_hook_proc(code: i32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, WM_KEYUP,
    };
    if code >= 0 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        // 跳过注入事件: 我们自己模拟的 Ctrl+C 不能触发钩子 (否则复制→又触发→死循环)
        let injected = (kb.flags.0 & LLKHF_INJECTED.0) != 0;
        if !injected && kb.vkCode == VK_CONTROL.0 as u32 && wparam.0 as u32 == WM_KEYUP {
            let now = now_ms();
            let last = LAST_CTRL_UP.swap(now, Ordering::SeqCst);
            // 连按三次(第三下距触发<350ms)不重复触发
            if last != 0 && now - last < 350 && now - TRIGGER_AT.load(Ordering::SeqCst) > 350 {
                TRIGGER_AT.store(now, Ordering::SeqCst);
                LAST_CTRL_UP.store(0, Ordering::SeqCst);
                let app = HOOK_APP.lock().unwrap().clone();
                if let Some(app) = app {
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
                    let mut msg = MSG::default();
                    // 消息泵: 低级钩子回调依赖安装线程持续泵消息
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                Err(e) => eprintln!("[term-lens] 键盘钩子安装失败: {e}"),
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
        Ok(Some(t)) => FallbackResp { result: Some(t), offline: false, error: None },
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
            let mut t = t;
            if t.layer != "personal" {
                // 外部导入不得覆盖用户裁决
                upsert(conn, &{ t.status = "active".into(); t });
                n += 1;
            }
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

fn main() {
    // CLI 子命令先于互斥体判断
    let arg = std::env::args().nth(1).unwrap_or_default();
    let is_cli = matches!(arg.as_str(), "--export" | "--stats" | "--rescan");
    if is_cli {
        std::env::set_var("TERMLENS_CLI", "1");
    }
    ensure_single_instance();

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
    load_config();
    load_prompt();

    // GUI 模式隐藏控制台窗口 (explorer/Run 键启动时不闪黑框; CLI 模式保留 stdout)
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::System::Console::{
            AttachConsole, FreeConsole, GetConsoleWindow, ATTACH_PARENT_PROCESS,
        };
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        unsafe {
            // 若控制台来自父进程(我们在终端里启动 GUI), 不隐藏; 否则隐藏独立黑框
            if AttachConsole(ATTACH_PARENT_PROCESS).is_ok() {
                let _ = FreeConsole();
            } else {
                let hwnd = GetConsoleWindow();
                if !hwnd.is_invalid() {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
        }
    }

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

    let mode = load_config().hotkey.mode.clone();
    let shortcut = Shortcut::new(Some(Modifiers::ALT), Code::KeyT);

    tauri::Builder::default()
        .manage(state)
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, _sc, ev| {
                    if ev.state() == ShortcutState::Pressed {
                        grab_selection_and_show(app);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            if mode == "alt_t" {
                let _ = app.global_shortcut().register(shortcut);
            } else {
                #[cfg(target_os = "windows")]
                start_double_ctrl_hook(app.handle().clone());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            lookup, fallback, adopt, fix, reject, export_terms, stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running TermLens");
}
