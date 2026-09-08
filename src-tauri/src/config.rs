//! 配置、URL 允许名单、密钥解析。toml 不再作为 api_key 正路。

use regex::Regex;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_CONFIG: &str = include_str!("default_config.toml");
pub const FALLBACK_PROMPT: &str = include_str!("fallback_prompt.md");
pub const DB_NAME: &str = "terms.db";
pub const CRED_TARGET: &str = "TermLens/api_key";

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Config {
    pub provider: ProviderCfg,
    pub ui: UiCfg,
    pub hotkey: HotkeyCfg,
    pub migrate: MigrateCfg,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(default)]
pub struct MigrateCfg {
    /// 一次性：0.1.1 默认公网 DeepSeek 已处理。之后用户再填同一 URL 不得清空。
    pub cloud_default_cleared: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct HotkeyCfg {
    pub mode: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct ProviderCfg {
    pub base_url: String,
    /// 仅作一次性迁移源；运行时不要依赖此字段。
    #[serde(default)]
    pub api_key: String,
    pub model: String,
    pub timeout_ms: u64,
    pub reasoning_off: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct UiCfg {
    pub popup_width: f64,
    pub popup_height: f64,
    pub position: String,
}

impl Default for HotkeyCfg {
    fn default() -> Self {
        HotkeyCfg {
            mode: "double_ctrl".into(),
        }
    }
}
impl Default for ProviderCfg {
    fn default() -> Self {
        ProviderCfg {
            base_url: String::new(),
            api_key: String::new(),
            model: "deepseek-v4-flash".into(),
            timeout_ms: 3000,
            reasoning_off: true,
        }
    }
}
impl Default for UiCfg {
    fn default() -> Self {
        UiCfg {
            popup_width: 360.0,
            popup_height: 230.0,
            position: "cursor".into(),
        }
    }
}

pub fn data_dir() -> PathBuf {
    let p = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("term-lens");
    let _ = fs::create_dir_all(&p);
    p
}

pub fn write_if_absent(path: &Path, content: &str) {
    if !path.exists() {
        let _ = fs::write(path, content);
    }
}

pub fn is_loopback_host(host: &str) -> bool {
    let h = host
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    matches!(
        h.as_str(),
        "127.0.0.1" | "localhost" | "::1" | "0:0:0:0:0:0:0:1"
    )
}

/// 默认只允许 https；明文 http 仅 loopback host；拒绝非 http(s)。
pub fn validate_base_url(raw: &str) -> Result<(), String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("base_url 为空，不上云".into());
    }
    let (scheme, rest) = if let Some(r) = strip_scheme(s, "https://") {
        ("https", r)
    } else if let Some(r) = strip_scheme(s, "http://") {
        ("http", r)
    } else {
        return Err("仅允许 http(s) URL".into());
    };
    if rest.starts_with('/') || rest.is_empty() {
        return Err("拒绝无 host 的 URL".into());
    }
    if rest.contains('@') {
        return Err("拒绝带用户信息的 URL".into());
    }
    let hostport = rest.split('/').next().unwrap_or("");
    if hostport.is_empty() || hostport == "*" || hostport.starts_with('.') {
        return Err("拒绝任意 URL".into());
    }
    let host = hostport
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(hostport);
    let host = if host.starts_with('[') {
        host.split(']').next().unwrap_or("").trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or("")
    };
    if host.is_empty() || host == "*" || host == "0.0.0.0" {
        return Err("拒绝任意 URL".into());
    }
    if scheme == "http" && !is_loopback_host(host) {
        return Err("明文 http 仅允许 loopback host".into());
    }
    Ok(())
}

fn strip_scheme<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

pub fn should_send_cloud_request(base_url: &str) -> bool {
    validate_base_url(base_url).is_ok()
}

pub fn url_is_loopback(base_url: &str) -> bool {
    let s = base_url.trim();
    let rest = strip_scheme(s, "https://")
        .or_else(|| strip_scheme(s, "http://"))
        .unwrap_or("");
    let hostport = rest.split('/').next().unwrap_or("");
    let host = if hostport.starts_with('[') {
        hostport
            .split(']')
            .next()
            .unwrap_or("")
            .trim_start_matches('[')
    } else {
        hostport.split(':').next().unwrap_or("")
    };
    is_loopback_host(host)
}

pub fn http_client_for(base_url: &str) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder();
    if url_is_loopback(base_url) {
        b = b.no_proxy();
    }
    b.build().map_err(|e| format!("构建 HTTP 客户端失败: {e}"))
}

pub fn default_timeout_ms(ms: u64) -> u64 {
    if ms == 0 {
        3000
    } else {
        ms
    }
}

/// 从 toml 文本抽出非空 api_key，并返回擦除后的文本。
pub fn strip_toml_api_key(raw: &str) -> (String, Option<String>) {
    let re = match Regex::new(r#"(?m)^([ \t]*api_key[ \t]*=[ \t]*)"([^"]*)""#) {
        Ok(r) => r,
        Err(_) => return (raw.to_string(), None),
    };
    if let Some(c) = re.captures(raw) {
        let key = c.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();
        let out = re.replace(raw, r#"$1"""#).to_string();
        if key.is_empty() {
            (out, None)
        } else {
            (out, Some(key))
        }
    } else {
        (raw.to_string(), None)
    }
}

pub fn patch_config_field(config_text: &str, key: &str, value: &str) -> Result<String, String> {
    let re = Regex::new(&format!(
        r#"(?m)^([ \t]*{0}[ \t]*=[ \t]*)"[^"]*""#,
        regex::escape(key)
    ))
    .map_err(|e| e.to_string())?;
    if !re.is_match(config_text) {
        return Err(format!("配置中找不到 {key} 字段, 请手动编辑 config.toml"));
    }
    Ok(re
        .replace(config_text, format!("$1\"{value}\""))
        .to_string())
}

pub fn patch_config_file(key: &str, value: &str) -> Result<(), String> {
    let path = data_dir().join("config.toml");
    let raw = fs::read_to_string(&path).map_err(|e| format!("读取配置失败: {e}"))?;
    let out = patch_config_field(&raw, key, value)?;
    fs::write(&path, out).map_err(|e| format!("写回配置失败: {e}"))
}

pub fn write_default_config() -> Result<(), String> {
    let path = data_dir().join("config.toml");
    fs::write(&path, DEFAULT_CONFIG).map_err(|e| e.to_string())
}

pub fn write_default_prompt() -> Result<(), String> {
    let path = data_dir().join("fallback_prompt.md");
    fs::write(&path, FALLBACK_PROMPT).map_err(|e| e.to_string())
}

pub fn load_prompt() -> String {
    let path = data_dir().join("fallback_prompt.md");
    if !path.exists() {
        let _ = fs::write(&path, FALLBACK_PROMPT);
    }
    fs::read_to_string(&path).unwrap_or_else(|_| FALLBACK_PROMPT.to_string())
}

pub fn parse_config_toml(s: &str) -> Config {
    toml::from_str(s).unwrap_or_default()
}

/// 0.1.1 写入的产品默认公网端点（非用户显式选择）。
/// 规范化：去空白、忽略大小写、去尾斜杠、可选 `/v1`。
pub fn is_legacy_product_default_base_url(raw: &str) -> bool {
    let s = raw.trim().to_ascii_lowercase();
    let s = s.trim_end_matches('/');
    let s = s.strip_suffix("/v1").unwrap_or(s).trim_end_matches('/');
    s == "https://api.deepseek.com"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCloudUrlAction {
    /// 已打标，不再改 URL。
    Skip,
    /// 0.1.1 未配密钥的产品默认：清空 URL 并打标。
    ClearAndMark,
    /// 用户已显式使用云端（有密钥）或 URL 已非默认：只打标。
    MarkOnly,
}

/// `has_secret`：toml 或凭据管理器里已有非空密钥。
/// `timeout_ms == 15000` 才像未改过的 0.1.1 默认文件；0.1.2 默认是 3000。
pub fn plan_legacy_cloud_url(
    already_cleared: bool,
    base_url: &str,
    has_secret: bool,
    timeout_ms: u64,
) -> LegacyCloudUrlAction {
    if already_cleared {
        return LegacyCloudUrlAction::Skip;
    }
    if is_legacy_product_default_base_url(base_url) && !has_secret && timeout_ms == 15000 {
        LegacyCloudUrlAction::ClearAndMark
    } else {
        LegacyCloudUrlAction::MarkOnly
    }
}

pub fn upsert_cloud_default_cleared(raw: &str, value: bool) -> String {
    let re = Regex::new(r"(?m)^([ \t]*cloud_default_cleared[ \t]*=[ \t]*)(true|false)\s*$")
        .expect("cloud_default_cleared regex");
    if re.is_match(raw) {
        return re.replace(raw, format!("$1{value}")).into_owned();
    }
    format!(
        "{}\n\n[migrate]\ncloud_default_cleared = {value}\n",
        raw.trim_end()
    )
}

/// 一次性迁移：无密钥的 0.1.1 默认 DeepSeek 才清空；之后同一 URL 视为用户选择。
pub fn rewrite_legacy_cloud_migration(raw: &str, has_cred_secret: bool) -> Option<String> {
    let cfg = parse_config_toml(raw);
    let has_secret = has_cred_secret || !cfg.provider.api_key.trim().is_empty();
    match plan_legacy_cloud_url(
        cfg.migrate.cloud_default_cleared,
        &cfg.provider.base_url,
        has_secret,
        cfg.provider.timeout_ms,
    ) {
        LegacyCloudUrlAction::Skip => None,
        LegacyCloudUrlAction::ClearAndMark => {
            let patched = patch_config_field(raw, "base_url", "").ok()?;
            Some(upsert_cloud_default_cleared(&patched, true))
        }
        LegacyCloudUrlAction::MarkOnly => Some(upsert_cloud_default_cleared(raw, true)),
    }
}

/// 凭据回读必须等于原文才允许从 toml 删 key。
pub fn key_migration_committed(
    store_result: Result<(), String>,
    roundtrip: Option<&str>,
    original: &str,
) -> bool {
    store_result.is_ok() && roundtrip == Some(original)
}

pub fn load_config() -> Config {
    let path = data_dir().join("config.toml");
    write_if_absent(&path, DEFAULT_CONFIG);
    migrate_api_key_file(&path);
    migrate_legacy_default_base_url(&path);
    fs::read_to_string(&path)
        .ok()
        .map(|s| parse_config_toml(&s))
        .unwrap_or_default()
}

fn migrate_legacy_default_base_url(path: &Path) {
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    let has_cred = read_api_key().map(|s| !s.is_empty()).unwrap_or(false);
    if let Some(out) = rewrite_legacy_cloud_migration(&raw, has_cred) {
        if out != raw {
            let _ = fs::write(path, out);
        }
    }
}

fn migrate_api_key_file(path: &Path) {
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    let (stripped, key) = strip_toml_api_key(&raw);
    let Some(k) = key else {
        return;
    };
    let stored = store_api_key(&k);
    let roundtrip = read_api_key();
    if key_migration_committed(stored, roundtrip.as_deref(), &k) && stripped != raw {
        let _ = fs::write(path, stripped);
    }
}

pub fn mask_api_key(k: &str) -> String {
    if k.len() <= 8 {
        return "****".into();
    }
    format!("{}…{}", &k[..4], &k[k.len() - 2..])
}

pub fn resolve_api_key(toml_key: &str) -> String {
    if let Ok(v) = std::env::var("TERM_LENS_API_KEY") {
        let t = v.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    if let Some(k) = read_api_key() {
        if !k.is_empty() {
            return k;
        }
    }
    // 仅迁移窗口：内存中仍可用 toml 里读到的值，但不作为正路写入
    toml_key.trim().to_string()
}

pub fn store_api_key(key: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_cred_write(CRED_TARGET, key)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = key;
        Err("非 Windows，凭据管理器不可用".into())
    }
}

pub fn read_api_key() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        windows_cred_read(CRED_TARGET)
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_cred_write(target: &str, secret: &str) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::Security::Credentials::{
        CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    let mut target_w: Vec<u16> = OsStr::new(target)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut user_w: Vec<u16> = OsStr::new("TermLens")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // Windows 通用凭据惯例是 UTF-16 LE；写 UTF-8 回读常为空。
    let mut blob: Vec<u8> = secret
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let cred = CREDENTIALW {
        Flags: Default::default(),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target_w.as_mut_ptr()),
        Comment: PWSTR::null(),
        LastWritten: FILETIME::default(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(user_w.as_mut_ptr()),
    };
    unsafe { CredWriteW(&cred, 0) }.map_err(|e| format!("写入凭据失败: {e}"))
}

#[cfg(target_os = "windows")]
fn windows_cred_read(target: &str) -> Option<String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };

    let target_w: Vec<u16> = OsStr::new(target)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        if CredReadW(PCWSTR(target_w.as_ptr()), CRED_TYPE_GENERIC, 0, &mut cred).is_err()
            || cred.is_null()
        {
            return None;
        }
        let c = &*cred;
        let bytes = if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
        };
        CredFree(cred as *const std::ffi::c_void);
        decode_cred_blob(&bytes)
    }
}

fn decode_cred_blob(bytes: &[u8]) -> Option<String> {
    let utf16_le_ascii = bytes.len() >= 2
        && bytes.len().is_multiple_of(2)
        && bytes.chunks_exact(2).all(|c| c[1] == 0);
    if utf16_le_ascii {
        let u16s: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0)
            .collect();
        if let Ok(s) = String::from_utf16(&u16s) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    String::from_utf8(bytes.to_vec())
        .ok()
        .map(|s| s.trim().trim_end_matches('\0').to_string())
        .filter(|s| !s.is_empty())
}
