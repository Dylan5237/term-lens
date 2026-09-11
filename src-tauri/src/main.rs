// Term Lens — Windows 托盘划词注释器 (不是终端透镜)
// 设计依据: ../DESIGN.md §0 第一性原理锚点 (P1-P4, H1-H2)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rusqlite::Connection;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use term_lens::*;

struct AppState {
    db: Mutex<Connection>,
    last_selection: Mutex<String>,
}

static LAST_CTRL_UP: AtomicU64 = AtomicU64::new(0);
static TRIGGER_AT: AtomicU64 = AtomicU64::new(0);
static HOOK_APP: Mutex<Option<AppHandle>> = Mutex::new(None);
static HOTKEY_MODE: AtomicU8 = AtomicU8::new(0);
static GRAB_BUSY: AtomicBool = AtomicBool::new(false);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn tl_log(msg: &str) {
    use std::io::Write;
    let path = data_dir().join("term-lens.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{} {}", now_ms(), msg);
    }
}

fn ensure_single_instance(skip: bool) {
    if skip {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        #[allow(deprecated)]
        unsafe {
            let name: Vec<u16> = OsStr::new("Local\\TermLens-Main")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let _handle = windows::Win32::System::Threading::CreateMutexW(
                None,
                false,
                windows::core::PCWSTR(name.as_ptr()),
            );
            if windows::Win32::Foundation::ERROR_ALREADY_EXISTS
                == windows::Win32::Foundation::GetLastError()
            {
                tl_log("another instance running, exiting");
                native_box(
                    "TermLens",
                    "Term Lens 已在运行（托盘区）。本实例退出，避免热键/悬浮窗双开。",
                    false,
                );
                std::process::exit(0);
            }
        }
    }
}

struct GrabGuard;
impl Drop for GrabGuard {
    fn drop(&mut self) {
        GRAB_BUSY.store(false, Ordering::SeqCst);
    }
}

fn spawn_grab(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || grab_selection_and_show(&app));
}

fn grab_selection_and_show(app: &AppHandle) {
    if GRAB_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let _busy = GrabGuard;
    wait_modifiers_released();

    let selected = match read_os_selection() {
        Some(s) => s,
        None => {
            tl_log("os selection empty, abort lookup");
            emit_selection_failed(
                app,
                "未能读取当前选区。请先划选再取词；终端等应用暂不支持。剪贴板里若是截图/文件则无法回退复制。",
            );
            return;
        }
    };
    if let Some(st) = app.try_state::<AppState>() {
        *st.last_selection.lock().unwrap() = selected.clone();
    }

    let (terms, omitted) = extract_report(&selected);
    show_overlay(app, &terms, omitted);
}

fn emit_selection_failed(app: &AppHandle, msg: &str) {
    let (px, py) = get_cursor_pos();
    let fixed = load_config().ui.position == "fixed";
    let win: Option<WebviewWindow> = app.get_webview_window("main");
    let msg = msg.to_string();
    tauri::async_runtime::spawn(async move {
        if let Some(w) = win {
            position_window(&w, px, py, fixed);
            let _ = w.emit("selection-failed", &msg);
            let _ = w.show();
            let _ = w.set_focus();
        }
    });
}

#[derive(Serialize)]
struct TermsPayload {
    terms: Vec<String>,
    omitted: usize,
}

fn show_overlay(app: &AppHandle, terms: &[String], omitted: usize) {
    let app2 = app.clone();
    let (px, py) = get_cursor_pos();
    let fixed = load_config().ui.position == "fixed";
    let win: Option<WebviewWindow> = app2.get_webview_window("main");
    let payload = TermsPayload {
        terms: terms.to_vec(),
        omitted,
    };
    tauri::async_runtime::spawn(async move {
        if let Some(w) = win {
            position_window(&w, px, py, fixed);
            let _ = w.emit("terms-requested", &payload);
            let _ = w.show();
            let _ = w.set_focus();
        }
    });
}

const POPUP_MAX_LOGICAL_H: f64 = 640.0;
const POPUP_MIN_LOGICAL_H: f64 = 120.0;
const POPUP_WORK_FRAC: f64 = 2.0 / 3.0;

fn cursor_work_area(w: &WebviewWindow, px: f64, py: f64, scale: f64) -> (f64, f64, f64, f64) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::{POINT, RECT};
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        unsafe {
            let mon = MonitorFromPoint(
                POINT {
                    x: px as i32,
                    y: py as i32,
                },
                MONITOR_DEFAULTTONEAREST,
            );
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                rcMonitor: RECT::default(),
                rcWork: RECT::default(),
                dwFlags: 0,
            };
            if GetMonitorInfoW(mon, &mut info).as_bool() {
                let r = info.rcWork;
                return (
                    r.left as f64,
                    r.top as f64,
                    (r.right - r.left) as f64,
                    (r.bottom - r.top) as f64,
                );
            }
        }
    }
    let mon = w
        .monitor_from_point(px, py)
        .ok()
        .flatten()
        .or_else(|| w.current_monitor().ok().flatten())
        .or_else(|| w.primary_monitor().ok().flatten());
    mon.as_ref()
        .map(|m| {
            let wa = m.work_area();
            (
                wa.position.x as f64,
                wa.position.y as f64,
                wa.size.width as f64,
                wa.size.height as f64,
            )
        })
        .unwrap_or((0.0, 0.0, 1920.0 * scale, 1080.0 * scale))
}

fn pin_overlay_pos(
    x: f64,
    y: f64,
    pw: f64,
    ph: f64,
    wx: f64,
    wy: f64,
    ww: f64,
    wh: f64,
    scale: f64,
) -> (f64, f64) {
    let pad = 12.0 * scale;
    let min_x = wx + pad;
    let min_y = wy + pad;
    let max_x = (wx + ww - pw - pad).max(min_x);
    let max_y = (wy + wh - ph - pad).max(min_y);
    (x.max(min_x).min(max_x), y.max(min_y).min(max_y))
}

fn clamp_overlay_pos(
    px: f64,
    py: f64,
    pw: f64,
    ph: f64,
    wx: f64,
    wy: f64,
    ww: f64,
    wh: f64,
    scale: f64,
    fixed: bool,
) -> (f64, f64) {
    let pad = 12.0 * scale;
    let (mut fx, mut fy) = if fixed {
        (wx + ww - pw - 24.0 * scale, wy + wh - ph - 60.0 * scale)
    } else {
        (px + pad, py + 16.0 * scale)
    };
    if fy + ph > wy + wh - pad {
        fy = py - ph - pad;
    }
    if fx + pw > wx + ww - pad {
        fx = px - pw - pad;
    }
    let min_x = wx + pad;
    let min_y = wy + pad;
    let max_x = (wx + ww - pw - pad).max(min_x);
    let max_y = (wy + wh - ph - pad).max(min_y);
    (fx.max(min_x).min(max_x), fy.max(min_y).min(max_y))
}

fn position_window(w: &WebviewWindow, px: f64, py: f64, fixed: bool) {
    let scale = w.scale_factor().unwrap_or(1.0);
    let (pw, ph) = w
        .outer_size()
        .map(|s| (s.width as f64, s.height as f64))
        .unwrap_or((360.0 * scale, 230.0 * scale));
    let (wx, wy, ww, wh) = cursor_work_area(w, px, py, scale);
    let max_ph = (wh * POPUP_WORK_FRAC).min(POPUP_MAX_LOGICAL_H * scale);
    let ph = ph.min(max_ph).max(POPUP_MIN_LOGICAL_H * scale);
    let _ = w.set_size(tauri::LogicalSize::new(pw / scale, ph / scale));
    let (fx, fy) = clamp_overlay_pos(px, py, pw, ph, wx, wy, ww, wh, scale, fixed);
    let _ = w.set_position(tauri::PhysicalPosition::new(fx, fy));
}

#[tauri::command]
fn place_overlay(
    app: tauri::AppHandle,
    width: f64,
    height: f64,
    reanchor: bool,
) -> Result<f64, String> {
    let w = app
        .get_webview_window("main")
        .ok_or_else(|| "no overlay window".to_string())?;
    let (px, py) = get_cursor_pos();
    let scale = w.scale_factor().map_err(|e| e.to_string())?;
    let (wx, wy, ww, wh) = cursor_work_area(&w, px, py, scale);
    let max_h = (wh / scale * POPUP_WORK_FRAC)
        .min(POPUP_MAX_LOGICAL_H)
        .max(POPUP_MIN_LOGICAL_H);
    let h = height.min(max_h).max(POPUP_MIN_LOGICAL_H);
    let pw = width * scale;
    let ph = h * scale;
    let fixed = load_config().ui.position == "fixed";
    w.set_size(tauri::LogicalSize::new(width, h))
        .map_err(|e| e.to_string())?;
    let (fx, fy) = if reanchor {
        clamp_overlay_pos(px, py, pw, ph, wx, wy, ww, wh, scale, fixed)
    } else if let Ok(pos) = w.outer_position() {
        pin_overlay_pos(pos.x as f64, pos.y as f64, pw, ph, wx, wy, ww, wh, scale)
    } else {
        clamp_overlay_pos(px, py, pw, ph, wx, wy, ww, wh, scale, fixed)
    };
    w.set_position(tauri::PhysicalPosition::new(fx, fy))
        .map_err(|e| e.to_string())?;
    Ok(h)
}

fn wait_modifiers_released() {
    #[cfg(target_os = "windows")]
    {
        use std::time::Instant;
        let t0 = Instant::now();
        while t0.elapsed().as_millis() < 500 {
            #[allow(deprecated)]
            let down = unsafe {
                use windows::Win32::UI::Input::KeyboardAndMouse::{
                    GetAsyncKeyState, VK_CONTROL, VK_MENU,
                };
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

#[cfg(target_os = "windows")]
unsafe extern "system" fn ll_hook_proc(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_LCONTROL, VK_RCONTROL};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, WM_KEYUP,
    };
    if code >= 0 && HOTKEY_MODE.load(Ordering::SeqCst) == 0 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let injected = (kb.flags.0 & LLKHF_INJECTED.0) != 0;
        let vk = kb.vkCode;
        let is_ctrl =
            vk == VK_LCONTROL.0 as u32 || vk == VK_RCONTROL.0 as u32 || vk == VK_CONTROL.0 as u32;
        if !injected && is_ctrl && wparam.0 as u32 == WM_KEYUP {
            let now = now_ms();
            let last = LAST_CTRL_UP.swap(now, Ordering::SeqCst);
            if last != 0 && now - last < 350 && now - TRIGGER_AT.load(Ordering::SeqCst) > 350 {
                TRIGGER_AT.store(now, Ordering::SeqCst);
                LAST_CTRL_UP.store(0, Ordering::SeqCst);
                let app = HOOK_APP.lock().unwrap().clone();
                if let Some(app) = app {
                    tl_log("double_ctrl triggered");
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
            DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, MSG, WH_KEYBOARD_LL,
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

#[derive(Serialize)]
struct LookupResult {
    hit: Option<Term>,
    candidates: Vec<Term>,
}

#[tauri::command]
fn lookup(state: State<AppState>, en: String) -> Result<LookupResult, String> {
    let ctx = state
        .last_selection
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let t0 = std::time::Instant::now();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let (hit, candidates) = lookup_terms(&conn, &en, &ctx).map_err(|e| e.to_string())?;
    let latency = t0.elapsed().as_millis() as i64;
    if let Some(h) = &hit {
        let _ = bump_hit_count(&conn, &h.en);
        let _ = log_query(&conn, &h.en, &h.layer, latency);
    } else {
        let _ = log_query(&conn, &en, "miss", latency);
    }
    Ok(LookupResult { hit, candidates })
}

#[derive(Serialize)]
struct LookupItem {
    en: String,
    hit: Option<Term>,
    candidates: Vec<Term>,
}

#[tauri::command]
fn lookup_many(state: State<AppState>, ens: Vec<String>) -> Result<Vec<LookupItem>, String> {
    let ctx = state
        .last_selection
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(ens.len().min(MAX_EXTRACT));
    for en in ens.into_iter().take(MAX_EXTRACT) {
        let t0 = std::time::Instant::now();
        let (hit, candidates) = lookup_terms(&conn, &en, &ctx).map_err(|e| e.to_string())?;
        let latency = t0.elapsed().as_millis() as i64;
        if let Some(h) = &hit {
            let _ = bump_hit_count(&conn, &h.en);
            let _ = log_query(&conn, &h.en, &h.layer, latency);
        } else {
            let _ = log_query(&conn, &en, "miss", latency);
        }
        out.push(LookupItem {
            en,
            hit,
            candidates,
        });
    }
    Ok(out)
}

#[derive(Serialize)]
struct FallbackResp {
    result: Option<Term>,
    offline: bool,
    error: Option<String>,
}

#[tauri::command]
async fn fallback(app: AppHandle, en: String) -> Result<FallbackResp, String> {
    validate_fallback_term(&en)?;
    let r = cloud_lookup(&en).await;
    let st = app.state::<AppState>();
    Ok(match r {
        Ok(Some(t)) => {
            let conn = st.db.lock().map_err(|e| e.to_string())?;
            upsert(&conn, &t).map_err(|e| e.to_string())?;
            FallbackResp {
                result: Some(t),
                offline: false,
                error: None,
            }
        }
        Ok(None) => FallbackResp {
            result: None,
            offline: false,
            error: None,
        },
        Err(e) => FallbackResp {
            result: None,
            offline: true,
            error: Some(e),
        },
    })
}

#[tauri::command]
fn adopt(
    state: State<AppState>,
    en: String,
    zh: String,
    domain: String,
    note: String,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    adopt_term(&conn, &en, &zh, &domain, &note).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn fix(state: State<AppState>, en: String, zh: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    fix_term(&conn, &en, &zh).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn reject(state: State<AppState>, en: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    reject_term(&conn, &en).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn export_terms(state: State<AppState>) -> Result<ExportOutcome, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let out_path = data_dir().join("export_terms.csv");
    export_db(&conn, &out_path)
}

#[tauri::command]
fn stats(state: State<AppState>) -> Result<serde_json::Value, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    Ok(stats_db(&conn))
}

fn mi(item: &impl tauri::menu::IsMenuItem<tauri::Wry>) -> &dyn tauri::menu::IsMenuItem<tauri::Wry> {
    item
}

fn native_box(title: &str, text: &str, yesno: bool) -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO,
            MESSAGEBOX_RESULT,
        };
        let enc = |s: &str| {
            s.encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        };
        let t = enc(title);
        let b = enc(text);
        let style = if yesno {
            MB_YESNO | MB_ICONQUESTION
        } else {
            MB_OK | MB_ICONINFORMATION
        };
        let r: MESSAGEBOX_RESULT = unsafe {
            MessageBoxW(
                None,
                windows::core::PCWSTR(b.as_ptr()),
                windows::core::PCWSTR(t.as_ptr()),
                style,
            )
        };
        if yesno {
            Some(r == IDYES)
        } else {
            Some(r != MESSAGEBOX_RESULT(0))
        }
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
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let enc = |s: &str| {
            s.encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        };
        let t = enc(&format!("TermLens - {title}"));
        let b = enc(text);
        unsafe {
            MessageBoxW(
                None,
                windows::core::PCWSTR(b.as_ptr()),
                windows::core::PCWSTR(t.as_ptr()),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(target_os = "windows"))]
    println!("[TermLens - {title}] {text}");
}
fn confirm_box(title: &str, text: &str) -> bool {
    native_box(&format!("TermLens - {title}"), text, true).unwrap_or(false)
}

fn open_in_notepad(path: &std::path::Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("notepad")
            .arg(path)
            .spawn()
            .is_ok()
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
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .is_ok()
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
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .is_ok()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        false
    }
}

fn provider_summary() -> String {
    let cfg = load_config();
    let key = resolve_api_key(&cfg.provider.api_key);
    let pr = data_dir().join("fallback_prompt.md");
    let pr_chars = std::fs::read_to_string(&pr)
        .map(|s| s.chars().count())
        .unwrap_or(0);
    format!(
        "产品    : Windows 托盘划词注释器（不支持终端取词）\n\
         Base URL : {}\nModel    : {}\nAPI Key  : {}\n\
         Timeout  : {} ms\n\n\
         默认不上云。base_url 为空则零外网请求。\n密钥走 Windows 凭据管理器，不要写进 toml。\n\n\
         提示词文件: fallback_prompt.md ({} 字符)\n数据目录: {}",
        if cfg.provider.base_url.is_empty() {
            "(空，不上云)"
        } else {
            cfg.provider.base_url.as_str()
        },
        cfg.provider.model,
        mask_api_key(&key),
        default_timeout_ms(cfg.provider.timeout_ms),
        pr_chars,
        data_dir().display(),
    )
}

fn stats_text() -> String {
    let conn = Connection::open(data_dir().join(DB_NAME)).ok();
    match conn {
        Some(c) => {
            let s = stats_db(&c);
            let total: i64 = c
                .query_row("SELECT COUNT(*) FROM terms", [], |r| r.get(0))
                .unwrap_or(0);
            let pending: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM terms WHERE status='pending'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let personal: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM terms WHERE layer='personal'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let by_domain: i64 = c
                .query_row(
                    "SELECT COUNT(DISTINCT en) FROM terms WHERE status='active'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            format!(
                "词条总数   : {} (活动 {} 个)\n个人层     : {} 个\n待确认     : {} 个 (云端兜底结果, 可在悬浮窗\"采纳/否决\")\n\n本地命中率 : {:.2}%\n查询总次数 : {}  本地 P95: {} ms\n\n去重词条   : {}\n\n导出文件   : export_terms.csv\n数据目录   : {}",
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

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let cfg = load_config();
    let hk_alt = cfg.hotkey.mode == "alt_t";
    let pos_fixed = cfg.ui.position == "fixed";

    let m_translate =
        MenuItem::with_id(app, "act_translate", "翻译当前选中内容", true, None::<&str>)?;
    let m_show = MenuItem::with_id(app, "act_show", "显示悬浮窗", true, None::<&str>)?;
    let m_quit = MenuItem::with_id(app, "quit", "退出 TermLens", true, None::<&str>)?;
    let sep_a = PredefinedMenuItem::separator(app)?;
    let sep_b = PredefinedMenuItem::separator(app)?;

    let lib_stats = MenuItem::with_id(app, "lib_stats", "查看词库统计", true, None::<&str>)?;
    let lib_export = MenuItem::with_id(
        app,
        "lib_export",
        "导出词库 CSV (并打开所在目录)",
        true,
        None::<&str>,
    )?;
    let lib_rescan = MenuItem::with_id(
        app,
        "lib_rescan",
        "从数据目录导入 CSV 词条",
        true,
        None::<&str>,
    )?;
    let sep_lib = PredefinedMenuItem::separator(app)?;
    let lib_open = MenuItem::with_id(app, "lib_open", "打开词库数据目录", true, None::<&str>)?;
    let sub_lib = Submenu::with_items(
        app,
        "词库",
        true,
        &[
            mi(&lib_stats),
            mi(&lib_export),
            mi(&lib_rescan),
            mi(&sep_lib),
            mi(&lib_open),
        ],
    )?;

    let pr_view = MenuItem::with_id(app, "pr_view", "查看 / 编辑提示词文件", true, None::<&str>)?;
    let pr_reset = MenuItem::with_id(app, "pr_reset", "恢复默认提示词", true, None::<&str>)?;
    let sub_pr = Submenu::with_items(
        app,
        "提示词 (云端兜底词典)",
        true,
        &[mi(&pr_view), mi(&pr_reset)],
    )?;

    let cn_view = MenuItem::with_id(app, "cn_view", "查看当前连接配置", true, None::<&str>)?;
    let cn_test = MenuItem::with_id(app, "cn_test", "测试云端连接", true, None::<&str>)?;
    let cn_edit = MenuItem::with_id(
        app,
        "cn_edit",
        "编辑配置文件 config.toml",
        true,
        None::<&str>,
    )?;
    let cn_reset = MenuItem::with_id(app, "cn_reset", "恢复默认配置", true, None::<&str>)?;
    let sub_cn = Submenu::with_items(
        app,
        "大模型连接",
        true,
        &[mi(&cn_view), mi(&cn_test), mi(&cn_edit), mi(&cn_reset)],
    )?;

    let t_pos = MenuItem::with_id(app, "t_pos", "弹窗位置", false, None::<&str>)?;
    let pos_cursor = CheckMenuItem::with_id(
        app,
        "pos_cursor",
        "跟随鼠标 (默认)",
        true,
        !pos_fixed,
        None::<&str>,
    )?;
    let pos_fixed_c = CheckMenuItem::with_id(
        app,
        "pos_fixed",
        "固定屏幕右下角",
        true,
        pos_fixed,
        None::<&str>,
    )?;
    let sep_ui = PredefinedMenuItem::separator(app)?;
    let t_hk = MenuItem::with_id(app, "t_hk", "触发方式", false, None::<&str>)?;
    let hk_double =
        CheckMenuItem::with_id(app, "hk_double", "双击 Ctrl", true, !hk_alt, None::<&str>)?;
    let hk_alt_c = CheckMenuItem::with_id(app, "hk_alt", "Alt + T", true, hk_alt, None::<&str>)?;
    let sep_ui2 = PredefinedMenuItem::separator(app)?;
    let ui_size = MenuItem::with_id(
        app,
        "ui_size",
        "悬浮窗尺寸: 改 config.toml [ui]",
        false,
        None::<&str>,
    )?;
    let sub_ui = Submenu::with_items(
        app,
        "界面与热键",
        true,
        &[
            mi(&t_pos),
            mi(&pos_cursor),
            mi(&pos_fixed_c),
            mi(&sep_ui),
            mi(&t_hk),
            mi(&hk_double),
            mi(&hk_alt_c),
            mi(&sep_ui2),
            mi(&ui_size),
        ],
    )?;

    Menu::with_items(
        app,
        &[
            mi(&m_translate),
            mi(&m_show),
            mi(&sep_a),
            mi(&sub_lib),
            mi(&sub_pr),
            mi(&sub_cn),
            mi(&sub_ui),
            mi(&sep_b),
            mi(&m_quit),
        ],
    )
}

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

fn tray_menu_event(app: &AppHandle, id: &str) {
    let st = app.state::<AppState>();
    match id {
        "act_translate" => spawn_grab(app),
        "act_show" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        "lib_stats" => info_box("词库统计", &stats_text()),
        "lib_export" => match export_db(&st.db.lock().unwrap(), &data_dir().join("export_terms.csv"))
        {
            Ok(out) => {
                let opened = explorer_select(&out.path);
                info_box(
                    "词库导出",
                    &format!(
                        "已导出 {} 条活动词条 →\n{}\n\n{}",
                        out.written,
                        out.path.display(),
                        if opened {
                            "已打开所在文件夹。"
                        } else {
                            "请手动到数据目录查看。"
                        }
                    ),
                );
            }
            Err(e) => err_box("导出失败", &e),
        },
        "lib_rescan" => {
            let dir = data_dir();
            let mut total = 0usize;
            let mut files: Vec<(String, usize)> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                let mut names: Vec<PathBuf> = rd
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().map(|x| x == "csv").unwrap_or(false))
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n != "export_terms.csv")
                            .unwrap_or(false)
                    })
                    .collect();
                names.sort();
                for p in names {
                    match import_csv(&st.db.lock().unwrap(), &p) {
                        Ok(n) if n > 0 => {
                            total += n;
                            files.push((
                                p.file_name()
                                    .map(|f| f.to_string_lossy().to_string())
                                    .unwrap_or_default(),
                                n,
                            ));
                        }
                        Ok(_) => {}
                        Err(e) => err_box("导入失败", &format!("{}: {e}", p.display())),
                    }
                }
            }
            if files.is_empty() {
                info_box(
                    "词库导入",
                    "没有新增词条。\n\n说明: 把 CSV 放到数据目录后再次点击此项即可导入; 已存在的词条不会被覆盖 (只增不改)。官方层 rejected 可恢复。",
                );
            } else {
                let list = files
                    .iter()
                    .map(|(f, n)| format!("  {f}: +{n}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                info_box(
                    "词库导入",
                    &format!("共导入 {total} 条新词条:\n{list}\n\n数据目录: {}", dir.display()),
                );
            }
        }
        "lib_open" => {
            if !open_dir(&data_dir()) {
                err_box(
                    "打开目录",
                    &("无法打开资源管理器, 请手动访问:\n".to_owned() + &data_dir().display().to_string()),
                );
            }
        }
        "pr_view" => {
            let p = data_dir().join("fallback_prompt.md");
            if !open_in_notepad(&p) {
                err_box(
                    "打开提示词",
                    &("无法调用记事本, 请手动编辑:\n".to_owned() + &p.display().to_string()),
                );
            }
        }
        "pr_reset" => {
            if confirm_box(
                "恢复默认提示词",
                "将把 fallback_prompt.md 重置为内置默认词典规则。\n你自己的修改会丢失, 确认继续?",
            ) {
                match write_default_prompt() {
                    Ok(()) => info_box(
                        "恢复默认提示词",
                        "已恢复默认。下一次云端兜底请求即使用默认提示词。",
                    ),
                    Err(e) => err_box("恢复失败", &e),
                }
            }
        }
        "cn_view" => info_box("大模型连接", &provider_summary()),
        "cn_test" => {
            tauri::async_runtime::spawn(async move {
                let cfg = load_config();
                if let Err(e) = cloud_preflight(&cfg.provider.base_url) {
                    err_box("连接测试", &format!("{e}\n\n默认不上云。请在 config.toml 填写允许的 base_url（https、loopback http 或内网 RFC1918 http）。密钥走凭据管理器。"));
                    return;
                }
                let url = format!(
                    "{}/chat/completions",
                    cfg.provider.base_url.trim_end_matches('/')
                );
                let body = serde_json::json!({
                    "model": cfg.provider.model,
                    "messages": [{"role":"user","content":"ping"}],
                    "max_tokens": 1,
                });
                let client = match http_client_for(&cfg.provider.base_url) {
                    Ok(c) => c,
                    Err(e) => {
                        err_box("连接测试", &e);
                        return;
                    }
                };
                let key = resolve_api_key(&cfg.provider.api_key);
                let r = client
                    .post(&url)
                    .header("Authorization", format!("Bearer {key}"))
                    .json(&body)
                    .timeout(std::time::Duration::from_millis(default_timeout_ms(
                        cfg.provider.timeout_ms,
                    )))
                    .send()
                    .await;
                match r {
                    Ok(resp) if resp.status().is_success() => {
                        info_box("连接测试", &format!("✓ 连接成功\n\nBase URL : {}\nModel    : {}\n\n响应状态: HTTP {}\n\n说明: 云端兜底仅在本地词库未命中时使用。", cfg.provider.base_url, cfg.provider.model, resp.status().as_u16()));
                    }
                    Ok(resp) => err_box("连接测试", &format!("云端返回 HTTP {}\n\nBase URL: {}\n请检查 config.toml 的 base_url / model，以及凭据管理器中的密钥。", resp.status().as_u16(), cfg.provider.base_url)),
                    Err(e) => err_box("连接测试", &format!("请求失败: {}\n\nBase URL: {}", err_chain(&e), cfg.provider.base_url)),
                }
            });
        }
        "cn_edit" => {
            let p = data_dir().join("config.toml");
            if !open_in_notepad(&p) {
                err_box(
                    "打开配置",
                    &("无法调用记事本, 请手动编辑:\n".to_owned() + &p.display().to_string()),
                );
            }
        }
        "cn_reset" => {
            if confirm_box(
                "恢复默认配置",
                "将 config.toml 重置为内置默认值：默认不上云（base_url 为空），timeout_ms=3000，密钥不写进 toml。\n你配置的 base_url/model/热键会被覆盖, 确认继续?",
            ) {
                match write_default_config() {
                    Ok(()) => {
                        HOTKEY_MODE.store(0, Ordering::SeqCst);
                        let sc = Shortcut::new(Some(Modifiers::ALT), Code::KeyT);
                        let _ = app.global_shortcut().unregister(sc);
                        refresh_tray_menu(app);
                        info_box(
                            "恢复默认配置",
                            "已恢复默认（与写入的 default_config.toml 一致）：\n- base_url 为空，不上云\n- 超时 3000ms\n- 触发方式: 双击 Ctrl",
                        );
                    }
                    Err(e) => err_box("恢复失败", &e),
                }
            }
        }
        "pos_cursor" | "pos_fixed" => {
            let v = if id == "pos_fixed" { "fixed" } else { "cursor" };
            match patch_config_file("position", v) {
                Ok(()) => {
                    refresh_tray_menu(app);
                    info_box(
                        "弹窗位置",
                        if v == "fixed" {
                            "已切换为: 固定屏幕右下角。\n下一次触发悬浮即生效。"
                        } else {
                            "已切换为: 跟随鼠标。\n下一次触发悬浮即生效。"
                        },
                    );
                }
                Err(e) => err_box("切换失败", &e),
            }
        }
        "hk_double" | "hk_alt" => {
            let alt = id == "hk_alt";
            let sc = Shortcut::new(Some(Modifiers::ALT), Code::KeyT);
            if alt {
                match app.global_shortcut().register(sc) {
                    Ok(()) => {
                        let prev = HOTKEY_MODE.swap(1, Ordering::SeqCst);
                        if let Err(e) = patch_config_file("mode", "alt_t") {
                            HOTKEY_MODE.store(prev, Ordering::SeqCst);
                            let _ = app.global_shortcut().unregister(sc);
                            err_box("切换失败", &e);
                            return;
                        }
                    }
                    Err(e) => {
                        err_box(
                            "注册失败",
                            &format!("Alt+T 注册失败: {e}\n未改变当前触发方式。可能被其它程序占用。"),
                        );
                        return;
                    }
                }
            } else {
                let _ = app.global_shortcut().unregister(sc);
                let prev = HOTKEY_MODE.swap(0, Ordering::SeqCst);
                if let Err(e) = patch_config_file("mode", "double_ctrl") {
                    HOTKEY_MODE.store(prev, Ordering::SeqCst);
                    err_box("切换失败", &e);
                    return;
                }
            }
            refresh_tray_menu(app);
            info_box(
                "触发方式",
                if alt {
                    "已切换为 Alt+T, 即时生效。\n\n说明: Alt+T 需在目标程序内先划选文本, 再按组合键取词翻译。不支持终端取词。"
                } else {
                    "已切换为 双击 Ctrl, 即时生效。\n\n说明: 在任意程序划选文本后快速按两下 Ctrl 即可取词翻译。不支持终端取词。"
                },
            );
        }
        "quit"
            if confirm_box(
                "退出",
                "确定退出 TermLens?\n\n退出后热键与托盘将失效; 重新打开安装目录下的 TermLens.exe 即可恢复。",
            ) =>
        {
            app.exit(0);
        }
        _ => {}
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let handle = app.handle();
    let menu = build_menu(handle)?;
    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .tooltip("Term Lens — Windows 划词注释器（不支持终端取词，默认不上云）")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_string();
            tray_menu_event(app, &id);
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
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
    let arg = std::env::args().nth(1).unwrap_or_default();
    let is_cli = matches!(arg.as_str(), "--export" | "--stats" | "--rescan");
    ensure_single_instance(is_cli);

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
    let conn = init_db(&db_path).expect("open sqlite");
    let seeded = apply_official_seeds(&conn).unwrap_or(0);
    if seeded > 0 {
        println!("[term-lens] official seeds applied ({seeded} rows scanned)");
    }

    if is_cli {
        if arg == "--export" {
            match export_db(&conn, &data_dir().join("export_terms.csv")) {
                Ok(out) => println!("exported {} rows -> {}", out.written, out.path.display()),
                Err(e) => eprintln!("{e}"),
            }
            return;
        }
        if arg == "--stats" {
            let s = stats_db(&conn);
            println!("{s:#}");
            return;
        }
        if arg == "--rescan" {
            let path = std::env::args()
                .nth(2)
                .map(PathBuf::from)
                .unwrap_or(data_dir().join("seed_terms.csv"));
            match import_csv(&conn, &path) {
                Ok(n) => println!("imported {n} rows from {}", path.display()),
                Err(e) => eprintln!("{e}"),
            }
            return;
        }
    }

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

    let state = AppState {
        db: Mutex::new(conn),
        last_selection: Mutex::new(String::new()),
    };

    let shortcut = Shortcut::new(Some(Modifiers::ALT), Code::KeyT);

    tauri::Builder::default()
        .manage(state)
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, _sc, ev| {
                    if ev.state() == ShortcutState::Pressed {
                        spawn_grab(app);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            let hk_alt = load_config().hotkey.mode == "alt_t";
            if hk_alt {
                match app.global_shortcut().register(shortcut) {
                    Ok(()) => HOTKEY_MODE.store(1, Ordering::SeqCst),
                    Err(e) => {
                        tl_log(&format!(
                            "alt_t register failed, staying on double_ctrl: {e}"
                        ));
                        HOTKEY_MODE.store(0, Ordering::SeqCst);
                    }
                }
            } else {
                HOTKEY_MODE.store(0, Ordering::SeqCst);
            }
            #[cfg(target_os = "windows")]
            start_double_ctrl_hook(app.handle().clone());
            build_tray(app)?;
            tl_log("tray ready");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            lookup,
            lookup_many,
            fallback,
            adopt,
            fix,
            reject,
            export_terms,
            stats,
            place_overlay
        ])
        .run(tauri::generate_context!())
        .expect("error while running TermLens");
}
