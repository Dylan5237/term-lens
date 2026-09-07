//! 云端兜底：未配置不发请求；IPC 术语约束。

use crate::config::{
    default_timeout_ms, http_client_for, load_config, load_prompt, resolve_api_key,
    should_send_cloud_request, validate_base_url,
};
use crate::glossary::Term;
use serde::{Deserialize, Serialize};

pub const MAX_FALLBACK_TERM_CHARS: usize = 64;

#[derive(Serialize)]
struct ChatReq {
    model: String,
    messages: Vec<Msg>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Thinking>,
}
#[derive(Serialize)]
struct Thinking {
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Serialize, Deserialize, Clone)]
struct Msg {
    role: String,
    content: String,
}

pub fn validate_fallback_term(en: &str) -> Result<(), String> {
    let t = en.trim();
    if t.is_empty() {
        return Err("空术语".into());
    }
    if t.chars().count() > MAX_FALLBACK_TERM_CHARS {
        return Err("术语过长，拒绝整段文档".into());
    }
    if t.contains('\n') || t.contains('\r') {
        return Err("禁止换行/整段文档".into());
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
    {
        return Err("术语含非法字符".into());
    }
    Ok(())
}

pub fn cloud_preflight(base_url: &str) -> Result<(), String> {
    if base_url.trim().is_empty() {
        return Err("未配置云端（base_url 为空），不发请求".into());
    }
    validate_base_url(base_url)
}

pub fn err_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(x) = src {
        s.push_str(&format!(" | {x}"));
        src = x.source();
    }
    s
}

pub async fn cloud_lookup(en: &str) -> Result<Option<Term>, String> {
    validate_fallback_term(en)?;
    let cfg = load_config();
    cloud_preflight(&cfg.provider.base_url)?;
    if !should_send_cloud_request(&cfg.provider.base_url) {
        return Err("URL 不允许，不发请求".into());
    }
    let api_key = resolve_api_key(&cfg.provider.api_key);
    let prompt = load_prompt();
    let body = ChatReq {
        model: cfg.provider.model.clone(),
        messages: vec![
            Msg {
                role: "system".into(),
                content: prompt,
            },
            Msg {
                role: "user".into(),
                content: en.trim().to_string(),
            },
        ],
        max_tokens: Some(300),
        thinking: if cfg.provider.reasoning_off {
            Some(Thinking {
                kind: "disabled".into(),
            })
        } else {
            None
        },
    };
    let url = format!(
        "{}/chat/completions",
        cfg.provider.base_url.trim_end_matches('/')
    );
    let client = http_client_for(&cfg.provider.base_url)?;
    let timeout = std::time::Duration::from_millis(default_timeout_ms(cfg.provider.timeout_ms));
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", err_chain(&e)))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| "响应格式错误".to_string())?
        .to_string();
    let json_str = content
        .find('{')
        .and_then(|i| content[i..].rfind('}').map(|j| &content[i..i + j + 1]))
        .ok_or_else(|| "无 JSON".to_string())?;
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
    let ct: CloudTerm =
        serde_json::from_str(json_str).map_err(|e| format!("JSON 解析失败: {e}"))?;
    Ok(Some(Term {
        en: en.trim().to_string(),
        zh: ct.zh,
        domain: if ct.domain.is_empty() {
            "general".into()
        } else {
            ct.domain
        },
        ctx_hints: vec![],
        keep_policy: if ct.keep_policy.is_empty() {
            "translate".into()
        } else {
            ct.keep_policy
        },
        note: ct.note,
        layer: "personal".into(),
        source: "cloud-adopted".into(),
        status: "pending".into(),
    }))
}
