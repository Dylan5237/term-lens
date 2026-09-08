//! 取词：优先 UI Automation / 原生 Edit；失败时短暂 Ctrl+C 并立即还原剪贴板。

#[cfg(not(target_os = "windows"))]
pub fn read_os_selection() -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
pub fn read_os_selection() -> Option<String> {
    read_selection_uia()
        .or_else(read_selection_native_edit)
        .or_else(read_selection_clipboard_restore)
}

#[cfg(target_os = "windows")]
fn read_selection_uia() -> Option<String> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let should_uninit = hr.is_ok();
        let result = (|| -> Option<String> {
            let auto: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let mut roots = Vec::new();
            if let Ok(focused) = auto.GetFocusedElement() {
                roots.push(focused);
            }
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_ok() {
                if let Ok(at_point) = auto.ElementFromPoint(pt) {
                    roots.push(at_point);
                }
            }
            for root in &roots {
                if let Some(s) = collect_text_selection(&auto, root) {
                    return Some(s);
                }
            }
            None
        })();
        if should_uninit {
            CoUninitialize();
        }
        result
    }
}

#[cfg(target_os = "windows")]
fn collect_text_selection(
    auto: &windows::Win32::UI::Accessibility::IUIAutomation,
    start: &windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> Option<String> {
    if let Some(s) = text_pattern_selection(start) {
        return Some(s);
    }
    // 只对起点做 Subtree 搜索。祖先上 FindAll 会扫整个 Cursor/Chrome 树，过慢。
    if let Some(s) = find_descendant_text_selection(auto, start) {
        return Some(s);
    }
    let mut el = start.clone();
    for _ in 0..12 {
        let Ok(walker) = (unsafe { auto.RawViewWalker() }) else {
            break;
        };
        match unsafe { walker.GetParentElement(&el) } {
            Ok(parent) => el = parent,
            Err(_) => break,
        }
        if let Some(s) = text_pattern_selection(&el) {
            return Some(s);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn find_descendant_text_selection(
    auto: &windows::Win32::UI::Accessibility::IUIAutomation,
    start: &windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> Option<String> {
    use windows::core::VARIANT;
    use windows::Win32::UI::Accessibility::{
        TreeScope_Subtree, UIA_IsTextPatternAvailablePropertyId,
    };

    unsafe {
        let cond = auto
            .CreatePropertyCondition(UIA_IsTextPatternAvailablePropertyId, &VARIANT::from(true))
            .ok()?;
        let arr = start.FindAll(TreeScope_Subtree, &cond).ok()?;
        let n = arr.Length().ok()?.min(64);
        for i in 0..n {
            if let Ok(found) = arr.GetElement(i) {
                if let Some(s) = text_pattern_selection(&found) {
                    return Some(s);
                }
            }
        }
        None
    }
}

#[cfg(target_os = "windows")]
fn text_pattern_selection(
    el: &windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> Option<String> {
    use windows::core::Interface;
    use windows::Win32::UI::Accessibility::{IUIAutomationTextPattern, UIA_TextPatternId};

    unsafe {
        let unk = el.GetCurrentPattern(UIA_TextPatternId).ok()?;
        let pattern: IUIAutomationTextPattern = unk.cast().ok()?;
        let arr = pattern.GetSelection().ok()?;
        let n = arr.Length().ok()?;
        let mut acc = String::new();
        for i in 0..n {
            if let Ok(range) = arr.GetElement(i) {
                if let Ok(bstr) = range.GetText(-1) {
                    acc.push_str(&bstr.to_string());
                }
            }
        }
        crate::capture::accept_selection_text(&acc)
    }
}

#[cfg(target_os = "windows")]
fn read_selection_native_edit() -> Option<String> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetGUIThreadInfo, SendMessageW, GUITHREADINFO, WM_GETTEXT, WM_GETTEXTLENGTH,
    };

    const EM_GETSEL: u32 = 0x00B0;

    unsafe {
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(0, &mut info).ok()?;
        let hwnd = info.hwndFocus;
        if hwnd.is_invalid() {
            return None;
        }
        let mut class_buf = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class_buf);
        let class = String::from_utf16_lossy(&class_buf[..n as usize]).to_ascii_lowercase();
        let looks_edit = class.contains("edit")
            || class.contains("richedit")
            || class.contains("scintilla")
            || class == "notepadtexteditor";
        if !looks_edit {
            return None;
        }

        let mut start: u32 = 0;
        let mut end: u32 = 0;
        SendMessageW(
            hwnd,
            EM_GETSEL,
            WPARAM(&mut start as *mut u32 as usize),
            LPARAM(&mut end as *mut u32 as isize),
        );
        if end <= start {
            return None;
        }
        let len = SendMessageW(hwnd, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0;
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; (len as usize) + 1];
        SendMessageW(
            hwnd,
            WM_GETTEXT,
            WPARAM(buf.len()),
            LPARAM(buf.as_mut_ptr() as isize),
        );
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        let s = start as usize;
        let e = (end as usize).min(full.chars().count());
        if s >= e {
            return None;
        }
        crate::capture::accept_selection_text(&full.chars().skip(s).take(e - s).collect::<String>())
    }
}

/// Electron/Cursor 等常无可用 TextPattern：写入探针 → Ctrl+C → 立刻还原用户剪贴板。
/// 同一词再划仍能成功（不依赖「剪贴板是否碰巧等于选区」）。
#[cfg(target_os = "windows")]
fn read_selection_clipboard_restore() -> Option<String> {
    use enigo::{
        Direction::{Press, Release},
        Enigo, Key, Keyboard, Settings,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    let saved = arboard::Clipboard::new()
        .ok()
        .and_then(|mut c| c.get_text().ok());
    // 非文本剪贴板（截图/文件）不走这条，避免毁掉原内容
    let snap = saved?;

    let sentinel = format!(
        "\u{2060}tl-sel-{}\u{2060}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );

    {
        let mut c = arboard::Clipboard::new().ok()?;
        c.set_text(&sentinel).ok()?;
    }

    struct Restore(String);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Ok(mut c) = arboard::Clipboard::new() {
                let _ = c.set_text(self.0.clone());
            }
        }
    }
    let _restore = Restore(snap);

    let mut enigo = Enigo::new(&Settings::default()).ok()?;
    let _ = enigo.key(Key::Control, Press);
    let _ = enigo.key(Key::Unicode('c'), Press);
    let _ = enigo.key(Key::Unicode('c'), Release);
    let _ = enigo.key(Key::Control, Release);

    let mut found = None;
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let cur = arboard::Clipboard::new()
            .ok()
            .and_then(|mut c| c.get_text().ok());
        if let Some(s) = crate::capture::clipboard_after_probe(&sentinel, cur.as_deref()) {
            found = Some(s);
            break;
        }
    }
    found
}
