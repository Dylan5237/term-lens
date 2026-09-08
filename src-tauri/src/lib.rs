//! Term Lens 可单测核心逻辑（glossary / capture / config / fallback）。

pub mod capture;
pub mod config;
pub mod fallback;
pub mod glossary;
pub mod selection;

pub use capture::*;
pub use config::*;
pub use fallback::{
    cloud_lookup, cloud_preflight, err_chain, validate_fallback_term, MAX_FALLBACK_TERM_CHARS,
};
pub use glossary::*;
pub use selection::read_os_selection;

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;

    fn db() -> rusqlite::Connection {
        memory_db().expect("memory db")
    }

    #[test]
    fn extract_cjk_adjacent_agent() {
        let terms = extract_terms("一个Agent实例");
        assert!(
            terms.iter().any(|t| t.eq_ignore_ascii_case("Agent")),
            "CJK 紧贴应抽出 Agent, got {terms:?}"
        );
    }

    #[test]
    fn extract_snake_and_kebab_as_one_term() {
        assert_eq!(
            extract_terms("blocked_field"),
            vec![
                "blocked_field".to_string(),
                "blocked".into(),
                "field".into()
            ]
        );
        assert_eq!(extract_terms("huge-doge")[0], "huge-doge");
        assert_eq!(extract_terms("blocked_field_name")[0], "blocked_field_name");
        let cjk = extract_terms("一个blocked_field实例");
        assert_eq!(cjk[0], "blocked_field");
        let phrase = extract_terms("tool use");
        assert_eq!(phrase[0], "tool use");
        assert!(phrase.iter().any(|t| t == "tool"));
    }

    #[test]
    fn extract_camel_case_as_one_term() {
        let terms = extract_terms("blockedField");
        assert_eq!(terms[0], "blockedField");
        assert!(terms.iter().any(|t| t == "blocked"));
        assert!(terms.iter().any(|t| t == "Field"));
        let xml = extract_terms("XMLHttpRequest");
        assert_eq!(xml[0], "XMLHttpRequest");
        assert!(xml.iter().any(|t| t == "XML"));
        assert!(xml.iter().any(|t| t == "Http"));
        assert!(xml.iter().any(|t| t == "Request"));
        assert_eq!(extract_terms("HTTPS")[0], "HTTPS");
    }

    #[test]
    fn extract_short_selection_phrase_first() {
        assert_eq!(extract_terms("Common Template")[0], "Common Template");
        assert_eq!(
            extract_terms("Host-specific Fragment")[0],
            "Host-specific Fragment"
        );
        assert_eq!(extract_terms("Tool Use")[0], "Tool Use");
        assert_eq!(extract_terms("context window")[0], "context window");
        assert_eq!(extract_terms("  Common   Template  ")[0], "Common Template");
        let sentence = extract_terms("The Runtime is ready");
        assert_ne!(sentence[0], "The Runtime is ready");
        assert!(sentence.iter().any(|t| t.eq_ignore_ascii_case("Runtime")));
        let listed = extract_terms("foo, bar, baz");
        assert_ne!(listed.first().map(String::as_str), Some("foo, bar, baz"));
        assert!(listed.iter().any(|t| t == "foo"));
        let over = extract_terms("one two three four five");
        assert_ne!(over[0], "one two three four five");
    }

    #[test]
    fn extract_skips_leading_stopword_the() {
        let terms = extract_terms("The Runtime is ready");
        assert!(!terms.is_empty(), "应抽出 Runtime");
        assert!(
            !terms[0].eq_ignore_ascii_case("the"),
            "停用词 The 不得作为第一优先, got {terms:?}"
        );
        assert!(
            terms.iter().any(|t| t.eq_ignore_ascii_case("Runtime")),
            "应包含 Runtime, got {terms:?}"
        );
    }

    #[test]
    fn token_llm_seed_is_ciyuan() {
        csv_unique_keys(SEED_CLASSIC, SEED_AI).expect("CSV unique");
        let conn = db();
        apply_official_seeds(&conn).unwrap();
        let zh: String = conn
            .query_row(
                "SELECT zh FROM terms WHERE en='token' AND domain='llm' AND layer!='personal'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(zh, "词元");
        let sec: String = conn
            .query_row(
                "SELECT zh FROM terms WHERE en='token' AND domain='security'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sec, "令牌");

        conn.execute(
            "UPDATE terms SET zh='标记' WHERE en='token' AND domain='llm' AND layer='ai'",
            [],
        )
        .unwrap();
        apply_official_seeds(&conn).unwrap();
        let zh2: String = conn
            .query_row(
                "SELECT zh FROM terms WHERE en='token' AND domain='llm' AND layer='ai'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(zh2, "词元");
    }

    #[test]
    fn reject_only_personal_layer() {
        let conn = db();
        apply_official_seeds(&conn).unwrap();
        reject_term(&conn, "token").unwrap();
        let official: String = conn
            .query_row(
                "SELECT status FROM terms WHERE en='token' AND domain='llm' AND layer='ai'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(official, "active");
        let zh: String = conn
            .query_row(
                "SELECT zh FROM terms WHERE en='token' AND domain='llm' AND layer='ai'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(zh, "词元");
        let personal: String = conn
            .query_row(
                "SELECT status FROM terms WHERE en='token' AND layer='personal'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(personal, "rejected");
        let (hit, _) = lookup_terms(&conn, "token", "context window bpe").unwrap();
        assert_eq!(hit.as_ref().map(|t| t.zh.as_str()), Some("词元"));
    }

    #[test]
    fn adopt_sets_active_and_source() {
        let conn = db();
        apply_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO terms (en, zh, domain, layer, source, status)
             VALUES ('foo','福','general','personal','cloud-adopted','pending')",
            [],
        )
        .unwrap();
        adopt_term(&conn, "foo", "福", "general", "note").unwrap();
        let (status, source): (String, String) = conn
            .query_row(
                "SELECT status, source FROM terms WHERE en='foo' AND layer='personal'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "active");
        assert_eq!(source, "user-adopted");
    }

    #[test]
    fn export_null_ctx_hints_not_truncated() {
        let conn = db();
        conn.execute(
            "INSERT INTO terms (en, zh, domain, ctx_hints, layer, source, status)
             VALUES ('alpha','甲','general',NULL,'ai','t','active')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO terms (en, zh, domain, ctx_hints, layer, source, status)
             VALUES ('zeta','癸','general','[]','ai','t','active')",
            [],
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("tl-export-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("out.csv");
        let out = export_db(&conn, &path).expect("export");
        assert_eq!(out.written, 2);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("alpha"),
            "NULL ctx_hints 行不得被截断:\n{text}"
        );
        assert!(text.contains("zeta"), "后续行必须在:\n{text}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn normalize_class_not_clas() {
        assert_eq!(normalize("class"), "class");
        assert_eq!(normalize("tokens"), "token");
        assert_eq!(normalize("Class"), "class");
    }

    #[test]
    fn p95_is_true_percentile() {
        let xs: Vec<i64> = (1..=100).collect();
        assert_eq!(p95_offset(100), 95);
        assert_eq!(p95_value(xs), 96); // 0-based index 95 → value 96
        assert_eq!(p95_value(vec![]), 0);
        let conn = db();
        for ms in [10_i64, 20, 30, 40, 50] {
            log_query(&conn, "x", "ai", ms).unwrap();
        }
        let s = stats_db(&conn);
        let p = s["local_p95_ms"].as_i64().unwrap();
        assert_eq!(p, p95_value(vec![10, 20, 30, 40, 50]));
    }

    #[test]
    fn accept_selection_allows_same_word_twice() {
        assert_eq!(accept_selection_text("api_key"), Some("api_key".into()));
        assert_eq!(accept_selection_text("  api_key  "), Some("api_key".into()));
        assert!(accept_selection_text("   ").is_none());
    }

    #[test]
    fn clipboard_probe_ignores_sentinel_and_keeps_same_word() {
        let sentinel = "\u{2060}tl-sel-probe\u{2060}";
        assert!(clipboard_after_probe(sentinel, Some(sentinel)).is_none());
        assert!(clipboard_after_probe(sentinel, None).is_none());
        assert_eq!(
            clipboard_after_probe(sentinel, Some("Automation")),
            Some("Automation".into())
        );
        assert_eq!(
            clipboard_after_probe(sentinel, Some("  Automation  ")),
            Some("Automation".into())
        );
        assert!(clipboard_after_probe(sentinel, Some("   ")).is_none());
    }

    #[test]
    fn csv_keys_are_unique() {
        csv_unique_keys(SEED_CLASSIC, SEED_AI).expect("两份 CSV 的 (en,domain,layer) 必须 unique");
    }

    #[test]
    fn empty_base_url_does_not_send() {
        assert!(!should_send_cloud_request(""));
        assert!(!should_send_cloud_request("   "));
        assert!(cloud_preflight("").is_err());
        assert!(validate_base_url("http://example.com/v1").is_err());
        assert!(validate_base_url("ftp://127.0.0.1/v1").is_err());
        assert!(validate_base_url("http://127.0.0.1:10100/v1").is_ok());
        assert!(validate_base_url("http://localhost:10100/v1").is_ok());
        assert!(validate_base_url("https://api.deepseek.com/v1").is_ok());
        assert!(should_send_cloud_request("http://127.0.0.1:10100/v1"));
        assert!(!should_send_cloud_request("http://evil.example/v1"));
    }

    #[test]
    fn pick_does_not_guess_when_tied() {
        let terms = vec![
            Term {
                en: "token".into(),
                zh: "词元".into(),
                domain: "llm".into(),
                ctx_hints: vec!["bpe".into()],
                keep_policy: "translate".into(),
                note: String::new(),
                layer: "ai".into(),
                source: "s".into(),
                status: "active".into(),
            },
            Term {
                en: "token".into(),
                zh: "令牌".into(),
                domain: "security".into(),
                ctx_hints: vec!["jwt".into()],
                keep_policy: "translate".into(),
                note: String::new(),
                layer: "ai".into(),
                source: "s".into(),
                status: "active".into(),
            },
        ];
        assert!(pick_term(&terms, "hello world").is_none());
        let hit = pick_term(&terms, "jwt session").unwrap();
        assert_eq!(hit.zh, "令牌");
    }

    #[test]
    fn fallback_term_rejects_documents() {
        assert!(validate_fallback_term("Agent").is_ok());
        assert!(validate_fallback_term("tool use").is_ok());
        assert!(validate_fallback_term("blocked_field").is_ok());
        assert!(validate_fallback_term("huge-doge").is_ok());
        assert!(validate_fallback_term(&"word ".repeat(40)).is_err());
        assert!(validate_fallback_term("line1\nline2").is_err());
        assert!(validate_fallback_term("foo; DROP TABLE").is_err());
    }

    #[test]
    fn cap_text_truncates() {
        let s = "a".repeat(MAX_SELECTION_BYTES + 50);
        assert_eq!(cap_text(&s).len(), MAX_SELECTION_BYTES);
    }

    #[test]
    fn deepseek_on_012_timeout_is_kept_without_key() {
        let raw = "[provider]\nbase_url = \"https://api.deepseek.com\"\ntimeout_ms = 3000\n";
        let out = rewrite_legacy_cloud_migration(raw, false).expect("应打标并保留 URL");
        let cfg = parse_config_toml(&out);
        assert_eq!(cfg.provider.base_url, "https://api.deepseek.com");
        assert!(cfg.migrate.cloud_default_cleared);
    }

    #[test]
    fn default_config_has_empty_base_url() {
        let cfg = parse_config_toml(DEFAULT_CONFIG);
        assert!(cfg.provider.base_url.is_empty());
        assert_eq!(cfg.provider.timeout_ms, 3000);
        assert!(cfg.migrate.cloud_default_cleared);
        assert!(DEFAULT_CONFIG.contains("base_url = \"\""));
        assert!(DEFAULT_CONFIG.contains("密钥不要写在本文件"));
    }

    #[test]
    fn strip_api_key_from_toml() {
        let raw = "[provider]\napi_key = \"sk-secret\"\nmodel = \"x\"\n";
        let (out, key) = strip_toml_api_key(raw);
        assert_eq!(key.as_deref(), Some("sk-secret"));
        assert!(out.contains("api_key = \"\""));
        assert!(!out.contains("sk-secret"));
    }

    #[test]
    fn legacy_deepseek_default_url_is_cleared() {
        let variants = [
            "https://api.deepseek.com",
            "https://api.deepseek.com/",
            "https://api.deepseek.com/v1",
            "https://api.deepseek.com/v1/",
            "HTTPS://API.DEEPSEEK.COM",
            "HTTPS://API.DEEPSEEK.COM/V1",
            "  https://api.deepseek.com/v1/  ",
        ];
        for url in variants {
            assert!(
                is_legacy_product_default_base_url(url),
                "应识别为 0.1.1 产品默认: {url}"
            );
            assert_eq!(
                plan_legacy_cloud_url(false, url, false, 15000),
                LegacyCloudUrlAction::ClearAndMark
            );
            let raw = format!(
                "[provider]\nbase_url = \"{url}\"\nmodel = \"deepseek-v4-flash\"\ntimeout_ms = 15000\n"
            );
            let out = rewrite_legacy_cloud_migration(&raw, false)
                .unwrap_or_else(|| panic!("应清空旧默认 URL: {url}"));
            let cfg = parse_config_toml(&out);
            assert!(
                cfg.provider.base_url.is_empty(),
                "写回后 base_url 应为空, url={url}"
            );
            assert!(cfg.migrate.cloud_default_cleared);
            assert_eq!(cfg.provider.model, "deepseek-v4-flash");
            assert_eq!(cfg.provider.timeout_ms, 15000);
        }
    }

    #[test]
    fn explicit_deepseek_with_key_is_kept() {
        let raw = "[provider]\nbase_url = \"https://api.deepseek.com\"\napi_key = \"sk-test\"\nmodel = \"deepseek-v4-flash\"\n";
        let out = rewrite_legacy_cloud_migration(raw, false).expect("应打标但保留 URL");
        let cfg = parse_config_toml(&out);
        assert_eq!(cfg.provider.base_url, "https://api.deepseek.com");
        assert!(cfg.migrate.cloud_default_cleared);
        assert_eq!(
            plan_legacy_cloud_url(true, "https://api.deepseek.com", false, 3000),
            LegacyCloudUrlAction::Skip
        );
        assert!(rewrite_legacy_cloud_migration(&out, false).is_none());
    }

    #[test]
    fn flagged_deepseek_url_not_cleared_again() {
        let raw = "[provider]\nbase_url = \"https://api.deepseek.com\"\n\n[migrate]\ncloud_default_cleared = true\n";
        assert!(rewrite_legacy_cloud_migration(raw, false).is_none());
    }

    #[test]
    fn custom_base_url_is_not_migrated() {
        let custom = [
            "http://127.0.0.1:10100/v1",
            "https://api.openai.com/v1",
            "https://opencodex.example/v1",
            "https://api.deepseek.com.evil.com",
            "https://api.deepseek.com/v1/chat",
            "",
        ];
        for url in custom {
            assert!(
                !is_legacy_product_default_base_url(url),
                "自定义 URL 不得当产品默认: {url}"
            );
            assert_eq!(
                plan_legacy_cloud_url(false, url, false, 3000),
                LegacyCloudUrlAction::MarkOnly
            );
        }
    }

    #[test]
    fn key_migration_requires_roundtrip() {
        assert!(key_migration_committed(Ok(()), Some("sk-1"), "sk-1"));
        assert!(!key_migration_committed(Ok(()), Some(""), "sk-1"));
        assert!(!key_migration_committed(Ok(()), None, "sk-1"));
        assert!(!key_migration_committed(
            Err("fail".into()),
            Some("sk-1"),
            "sk-1"
        ));
    }

    #[test]
    fn schema_version_is_set() {
        let conn = db();
        assert_eq!(schema_version(&conn), 1);
    }

    #[test]
    fn import_restores_rejected_official() {
        let conn = db();
        conn.execute(
            "INSERT INTO terms (en, zh, domain, layer, source, status)
             VALUES ('widget','小部件','general','ai','seed','rejected')",
            [],
        )
        .unwrap();
        let csv = "en,zh,domain,ctx_hints,keep_policy,note,layer,source\nwidget,小部件,general,[],translate,n,ai,seed\n";
        let n = import_csv_text(&conn, csv).unwrap();
        assert_eq!(n, 1);
        let st: String = conn
            .query_row(
                "SELECT status FROM terms WHERE en='widget' AND layer='ai'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(st, "active");
        let personal = conn
            .query_row(
                "SELECT status FROM terms WHERE en='widget' AND layer='personal'",
                [],
                |r: &rusqlite::Row| r.get::<_, String>(0),
            )
            .optional()
            .unwrap();
        assert!(personal.is_none());
    }
}
