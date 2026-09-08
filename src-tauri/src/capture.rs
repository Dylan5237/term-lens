//! 取词：非空选区即查询。禁止把未变化的剪贴板当选区。

pub const MAX_SELECTION_BYTES: usize = 16 * 1024;

/// 修剪并截断；空串视为没有选区。同一词再划一次仍可查询。
pub fn accept_selection_text(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        None
    } else {
        Some(cap_text(t))
    }
}

/// 探针写入后：剪贴板仍是探针 → 复制未发生；变成别的非空文本 → 才是选区。
pub fn clipboard_after_probe(sentinel: &str, current: Option<&str>) -> Option<String> {
    let cur = current?;
    if cur == sentinel {
        None
    } else {
        accept_selection_text(cur)
    }
}

pub fn cap_text(s: &str) -> String {
    if s.len() <= MAX_SELECTION_BYTES {
        return s.to_string();
    }
    let mut end = MAX_SELECTION_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}
