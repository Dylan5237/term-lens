//! 剪贴板取词状态机：未变化则禁止查询。

pub const MAX_SELECTION_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrabDecision {
    Unchanged,
    Changed(String),
}

impl GrabDecision {
    pub fn should_lookup(&self) -> bool {
        matches!(self, GrabDecision::Changed(_))
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            GrabDecision::Changed(s) => Some(s),
            GrabDecision::Unchanged => None,
        }
    }
}

/// 仅当 current 相对 snapshot 发生变化时返回新文本。
pub fn selection_if_changed(snapshot: Option<&str>, current: Option<&str>) -> Option<String> {
    let cur = current?;
    match snapshot {
        Some(old) if old == cur => None,
        Some(_) => Some(cur.to_string()),
        None => {
            if cur.is_empty() {
                None
            } else {
                Some(cur.to_string())
            }
        }
    }
}

pub fn decide_grab(snapshot: Option<&str>, current: Option<&str>) -> GrabDecision {
    match selection_if_changed(snapshot, current) {
        Some(s) => GrabDecision::Changed(cap_text(&s)),
        None => GrabDecision::Unchanged,
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
