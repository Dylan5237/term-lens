//! 术语表：提取、归一、裁决、种子、导入导出、层隔离。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: i64 = 1;
pub const MAX_EXTRACT: usize = 8;
pub const SEED_CLASSIC: &str = include_str!("../../data/seed_terms.csv");
pub const SEED_AI: &str = include_str!("../../data/ai_terms_latest.csv");

const STOPWORDS: &[&str] = &[
    "the", "is", "of", "a", "an", "to", "and", "or", "in", "on", "for", "with", "at", "by", "from",
    "as", "be", "are", "was", "were", "it", "this", "that", "not", "but", "if", "we", "you",
    "they", "i", "he", "she", "its", "our", "your", "can", "may", "do", "does", "did", "so",
    "than", "then",
];

const LAYER_RANK: &[&str] = &["ms", "ai", "personal"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Term {
    pub en: String,
    pub zh: String,
    pub domain: String,
    pub ctx_hints: Vec<String>,
    pub keep_policy: String,
    pub note: String,
    pub layer: String,
    pub source: String,
    pub status: String,
}

pub fn layer_rank(layer: &str) -> i32 {
    LAYER_RANK.iter().position(|l| *l == layer).unwrap_or(0) as i32
}

/// LLM 域的 token 锁定「词元」；security 域保留令牌，不在此锁定。
pub fn locked_official_zh(en: &str, domain: &str) -> Option<&'static str> {
    if en.eq_ignore_ascii_case("token") && domain == "llm" {
        Some("词元")
    } else {
        None
    }
}

pub fn apply_schema(conn: &Connection) -> rusqlite::Result<()> {
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
         );
         CREATE TABLE IF NOT EXISTS meta (
           key TEXT PRIMARY KEY,
           value TEXT NOT NULL
         );",
    )?;
    migrate(conn)
}

pub fn schema_version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT value FROM meta WHERE key='schema_version'",
        [],
        |r| {
            let s: String = r.get(0)?;
            Ok(s.parse::<i64>().unwrap_or(0))
        },
    )
    .unwrap_or(0)
}

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let v = schema_version(conn);
    if v < 1 {
        // 旧版 reject 误伤官方层：解卡，但不碰 personal
        conn.execute(
            "UPDATE terms SET status='active', updated_at=datetime('now','localtime') \
             WHERE layer != 'personal' AND status='rejected'",
            [],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES ('schema_version','1')",
            [],
        )?;
    }
    Ok(())
}

pub fn init_db(db_path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(db_path)?;
    apply_schema(&conn)?;
    Ok(conn)
}

fn row_to_term(row: &rusqlite::Row) -> rusqlite::Result<Term> {
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

pub fn lookup_local(conn: &Connection, en: &str) -> rusqlite::Result<Vec<Term>> {
    let key = en.to_lowercase();
    let mut stmt = conn.prepare(&format!(
        "{SELECT_COLS} WHERE en = ?1 AND status IN ('active','pending') \
         ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END, hit_count DESC"
    ))?;
    let rows = stmt.query_map([&key], row_to_term)?;
    let mut out = Vec::new();
    for row in rows {
        match row {
            Ok(t) => out.push(t),
            Err(_) => continue,
        }
    }
    Ok(out)
}

/// best==0 且多候选时不猜，返回 None（调用方仍持有 candidates）。
pub fn pick_term(terms: &[Term], context: &str) -> Option<Term> {
    if terms.is_empty() {
        return None;
    }
    let mut indexed: Vec<(i32, usize)> = terms
        .iter()
        .enumerate()
        .map(|(i, t)| (layer_rank(&t.layer), i))
        .collect();
    indexed.sort_by_key(|(rank, _)| -rank);
    if terms.len() == 1 {
        return Some(terms[indexed[0].1].clone());
    }
    let ctx = context.to_lowercase();
    let scored: Vec<(i32, usize)> = terms
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let score = if t.domain == "general" {
                0
            } else {
                t.ctx_hints
                    .iter()
                    .filter(|h| ctx.contains(&h.to_lowercase()))
                    .count() as i32
            };
            (score, i)
        })
        .collect();
    let best = scored.iter().map(|(s, _)| *s).max().unwrap_or(0);
    if best == 0 {
        return None;
    }
    let idx = scored
        .iter()
        .filter(|(s, _)| *s == best)
        .max_by_key(|(_, i)| layer_rank(&terms[*i].layer))
        .map(|(_, i)| *i)?;
    Some(terms[idx].clone())
}

pub fn lookup_terms(
    conn: &Connection,
    en: &str,
    ctx: &str,
) -> rusqlite::Result<(Option<Term>, Vec<Term>)> {
    let mut found = lookup_local(conn, en.trim())?;
    if found.is_empty() {
        let norm = normalize(en);
        if norm != en.to_lowercase() {
            found = lookup_local(conn, &norm)?;
        }
    }
    let hit = pick_term(&found, ctx);
    Ok((hit, found))
}

pub fn log_query(
    conn: &Connection,
    en: &str,
    layer_hit: &str,
    latency_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO query_log(en, layer_hit, latency_ms) VALUES (?1,?2,?3)",
        rusqlite::params![en.to_lowercase(), layer_hit, latency_ms],
    )?;
    Ok(())
}

pub fn bump_hit_count(conn: &Connection, en: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE terms SET hit_count = hit_count + 1 WHERE en = ?1",
        rusqlite::params![en.to_lowercase()],
    )?;
    Ok(())
}

fn hints_json(t: &Term) -> String {
    serde_json::to_string(&t.ctx_hints).unwrap_or_else(|_| "[]".into())
}

pub fn upsert(conn: &Connection, t: &Term) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO terms (en, zh, domain, ctx_hints, keep_policy, note, layer, source, status)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(en, domain, layer) DO UPDATE SET
           zh = excluded.zh, note = excluded.note, keep_policy = excluded.keep_policy,
           status = excluded.status, source = excluded.source,
           updated_at = datetime('now','localtime')",
        rusqlite::params![
            t.en.to_lowercase(),
            t.zh,
            t.domain,
            hints_json(t),
            t.keep_policy,
            t.note,
            t.layer,
            t.source,
            t.status
        ],
    )?;
    Ok(())
}

pub fn insert_official(conn: &Connection, t: &Term) -> rusqlite::Result<()> {
    if t.layer == "personal" {
        return Ok(());
    }
    let mut t = t.clone();
    t.en = t.en.to_lowercase();
    if let Some(zh) = locked_official_zh(&t.en, &t.domain) {
        t.zh = zh.to_string();
        conn.execute(
            "INSERT INTO terms (en, zh, domain, ctx_hints, keep_policy, note, layer, source, status)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(en, domain, layer) DO UPDATE SET
               zh = excluded.zh,
               note = excluded.note,
               keep_policy = excluded.keep_policy,
               ctx_hints = excluded.ctx_hints,
               source = excluded.source,
               status = CASE WHEN terms.status='rejected' THEN 'active' ELSE terms.status END,
               updated_at = datetime('now','localtime')",
            rusqlite::params![
                t.en,
                t.zh,
                t.domain,
                hints_json(&t),
                t.keep_policy,
                t.note,
                t.layer,
                t.source,
                t.status
            ],
        )?;
    } else {
        conn.execute(
            "INSERT OR IGNORE INTO terms (en, zh, domain, ctx_hints, keep_policy, note, layer, source, status)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                t.en,
                t.zh,
                t.domain,
                hints_json(&t),
                t.keep_policy,
                t.note,
                t.layer,
                t.source,
                t.status
            ],
        )?;
    }
    Ok(())
}

pub fn apply_official_seeds(conn: &Connection) -> rusqlite::Result<usize> {
    let mut n = 0usize;
    for line in SEED_CLASSIC.lines().skip(1).chain(SEED_AI.lines().skip(1)) {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(t) = parse_seed_line(line) {
            insert_official(conn, &t)?;
            n += 1;
        }
    }
    Ok(n)
}

pub fn parse_seed_line(line: &str) -> Option<Term> {
    let parts = parse_csv_line(line);
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

pub fn parse_csv_line(line: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    parts.push(cur);
    parts
}

pub fn csv_unique_keys(classic: &str, ai: &str) -> Result<Vec<(String, String, String)>, String> {
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    let mut dups = Vec::new();
    for (src, body) in [("seed_terms.csv", classic), ("ai_terms_latest.csv", ai)] {
        for (i, line) in body.lines().enumerate().skip(1) {
            if line.trim().is_empty() {
                continue;
            }
            let Some(t) = parse_seed_line(line) else {
                continue;
            };
            let key = (t.en.to_lowercase(), t.domain.clone(), t.layer.clone());
            if !seen.insert(key.clone()) {
                dups.push(format!(
                    "{src}:{} duplicate ({}, {}, {})",
                    i + 1,
                    key.0,
                    key.1,
                    key.2
                ));
            }
        }
    }
    if !dups.is_empty() {
        return Err(dups.join("; "));
    }
    Ok(seen.into_iter().collect())
}

pub fn is_stopword(w: &str) -> bool {
    STOPWORDS.contains(&w.to_ascii_lowercase().as_str())
}

fn is_connector(c: char) -> bool {
    matches!(c, ' ' | '_' | '-')
}

/// 按 ASCII 字母数字切词（CJK 紧贴视为边界）。返回 (词, 后接连接符)。
fn ascii_tokens(text: &str) -> Vec<(String, Option<char>)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < chars.len() && chars[i].is_ascii_alphanumeric() {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        let sep = chars.get(i).copied().filter(|c| is_connector(*c));
        out.push((word, sep));
    }
    out
}

fn push_unique(out: &mut Vec<String>, seen: &mut HashSet<String>, w: &str) -> bool {
    let lw = w.to_lowercase();
    if lw.len() < 2 || is_stopword(&lw) || seen.contains(&lw) {
        return out.len() >= MAX_EXTRACT;
    }
    seen.insert(lw);
    out.push(w.to_string());
    out.len() >= MAX_EXTRACT
}

/// ASCII 词边界，CJK 紧贴可抽出；满 8 个唯一非停用词早停。
pub fn extract_terms(text: &str) -> Vec<String> {
    let tokens = ascii_tokens(text);
    let mut seen = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for i in 0..tokens.len() {
        if push_unique(&mut out, &mut seen, &tokens[i].0) {
            break;
        }
        if let (Some(sep), Some((next, _))) = (tokens[i].1, tokens.get(i + 1)) {
            if is_connector(sep) && !is_stopword(&tokens[i].0) && !is_stopword(next) {
                let bigram = format!("{}{}{}", tokens[i].0, sep, next);
                if push_unique(&mut out, &mut seen, &bigram) {
                    break;
                }
            }
        }
    }
    out
}

/// 只剥一层复数 s；`class` 因以 ss 结尾保持不变，禁止 trim_end_matches 全剥。
pub fn normalize(en: &str) -> String {
    let s = en.trim().to_lowercase();
    if s.len() > 4 && s.ends_with('s') && !s.ends_with("ss") {
        if let Some(stem) = s.strip_suffix('s') {
            if !stem.is_empty() {
                return stem.to_string();
            }
        }
    }
    s
}

pub fn adopt_term(
    conn: &Connection,
    en: &str,
    zh: &str,
    domain: &str,
    note: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO terms (en, zh, domain, keep_policy, note, layer, source, status)
         VALUES (?1,?2,?3,'translate',?4,'personal','user-adopted','active')
         ON CONFLICT(en, domain, layer) DO UPDATE SET
           zh = excluded.zh, note = excluded.note,
           status = 'active', source = 'user-adopted',
           updated_at = datetime('now','localtime')",
        rusqlite::params![en.to_lowercase(), zh, domain, note],
    )
}

pub fn fix_term(conn: &Connection, en: &str, zh: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO terms (en, zh, domain, keep_policy, note, layer, source, status)
         VALUES (?1,?2,'general','translate','用户手工裁决','personal','user-fixed','active')
         ON CONFLICT(en, domain, layer) DO UPDATE SET
           zh = excluded.zh, note = excluded.note, status='active',
           source='user-fixed', updated_at = datetime('now','localtime')",
        rusqlite::params![en.to_lowercase(), zh],
    )
}

/// 只伤 personal 层；种子层 status 不变。
pub fn reject_term(conn: &Connection, en: &str) -> rusqlite::Result<()> {
    let key = en.to_lowercase();
    conn.execute(
        "UPDATE terms SET status='rejected', updated_at=datetime('now','localtime') \
         WHERE en=?1 AND layer='personal' AND status IN ('active','pending')",
        rusqlite::params![key],
    )?;
    conn.execute(
        "INSERT INTO terms (en, zh, domain, layer, source, status)
         VALUES (?1,'(已否决)','general','personal','user-rejected','rejected')
         ON CONFLICT(en, domain, layer) DO UPDATE SET status='rejected',
           zh=excluded.zh, source='user-rejected', updated_at=datetime('now','localtime')",
        rusqlite::params![key],
    )?;
    Ok(())
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[derive(Debug, Serialize)]
pub struct ExportOutcome {
    pub path: PathBuf,
    pub written: usize,
}

/// ctx_hints 按 Option 读；单行错误不结束循环；条数以实际写入为准。
pub fn export_db(conn: &Connection, out_path: &Path) -> Result<ExportOutcome, String> {
    let file = std::fs::File::create(out_path).map_err(|e| format!("无法创建导出文件: {e}"))?;
    let mut w = BufWriter::new(file);
    writeln!(w, "en,zh,domain,ctx_hints,keep_policy,note,layer,source")
        .map_err(|e| format!("写表头失败: {e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT en, zh, domain, ctx_hints, keep_policy, note, layer, source \
             FROM terms WHERE status='active' ORDER BY en",
        )
        .map_err(|e| format!("查询失败: {e}"))?;
    let mut rows = stmt
        .query_map([], |r| {
            let hints: Option<String> = r.get(3)?;
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                hints.unwrap_or_default(),
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })
        .map_err(|e| format!("映射失败: {e}"))?;
    let mut written = 0usize;
    loop {
        match rows.next() {
            Some(Ok(r)) => {
                if writeln!(
                    w,
                    "{},{},{},{},{},{},{},{}",
                    csv_escape(&r.0),
                    csv_escape(&r.1),
                    csv_escape(&r.2),
                    csv_escape(&r.3),
                    csv_escape(&r.4),
                    csv_escape(&r.5),
                    csv_escape(&r.6),
                    csv_escape(&r.7)
                )
                .is_ok()
                {
                    written += 1;
                }
            }
            Some(Err(_)) => continue,
            None => break,
        }
    }
    w.flush().map_err(|e| format!("flush 失败: {e}"))?;
    Ok(ExportOutcome {
        path: out_path.to_path_buf(),
        written,
    })
}

pub fn import_csv(conn: &Connection, path: &Path) -> Result<usize, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("读 CSV 失败: {e}"))?;
    import_csv_text(conn, &content).map_err(|e| e.to_string())
}

pub fn import_csv_text(conn: &Connection, content: &str) -> rusqlite::Result<usize> {
    let mut n = 0usize;
    for line in content.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let Some(t) = parse_seed_line(line) else {
            continue;
        };
        if t.layer == "personal" {
            continue;
        }
        let en = t.en.to_lowercase();
        let existing: Option<(String, String)> = conn
            .query_row(
                "SELECT status, layer FROM terms WHERE en=?1 AND domain=?2 AND layer=?3",
                rusqlite::params![en, t.domain, t.layer],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match existing {
            None => {
                insert_official(conn, &t)?;
                n += 1;
            }
            Some((status, layer)) if status == "rejected" && layer != "personal" => {
                conn.execute(
                    "UPDATE terms SET status='active', updated_at=datetime('now','localtime') \
                     WHERE en=?1 AND domain=?2 AND layer=?3 AND layer!='personal'",
                    rusqlite::params![en, t.domain, t.layer],
                )?;
                n += 1;
            }
            Some(_) => {}
        }
    }
    Ok(n)
}

/// 真正 95 分位：升序后下标 n*19/20。
pub fn p95_offset(n: usize) -> usize {
    if n == 0 {
        0
    } else {
        n * 19 / 20
    }
}

pub fn p95_value(mut xs: Vec<i64>) -> i64 {
    if xs.is_empty() {
        return 0;
    }
    xs.sort_unstable();
    let idx = p95_offset(xs.len()).min(xs.len() - 1);
    xs[idx]
}

pub fn stats_db(conn: &Connection) -> serde_json::Value {
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM query_log", [], |r| r.get(0))
        .unwrap_or(0);
    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM query_log WHERE layer_hit NOT IN ('miss','')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let terms: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM terms WHERE status='active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let p95: i64 = conn
        .query_row(
            "SELECT latency_ms FROM query_log WHERE layer_hit!='miss' \
             ORDER BY latency_ms ASC LIMIT 1 \
             OFFSET (SELECT (COUNT(*) * 19) / 20 FROM query_log WHERE layer_hit!='miss')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    serde_json::json!({
        "queries_total": total,
        "local_hit_rate": if total > 0 { (hits as f64 / total as f64 * 10000.0).round() / 10000.0 } else { 0.0 },
        "active_terms": terms,
        "local_p95_ms": p95,
    })
}

pub fn memory_db() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    apply_schema(&conn)?;
    Ok(conn)
}
