//! Compile smart-list predicates before paging. Counts are separate requests.
use super::*;

pub(super) const KIND: &str = "CASE WHEN json_extract(i.summary,'$.kind')='files' AND json_extract(i.summary,'$.file_count')=1 AND (lower(json_extract(i.summary,'$.preview')) GLOB '*.png' OR lower(json_extract(i.summary,'$.preview')) GLOB '*.jpg' OR lower(json_extract(i.summary,'$.preview')) GLOB '*.jpeg' OR lower(json_extract(i.summary,'$.preview')) GLOB '*.heic' OR lower(json_extract(i.summary,'$.preview')) GLOB '*.heif' OR lower(json_extract(i.summary,'$.preview')) GLOB '*.tif' OR lower(json_extract(i.summary,'$.preview')) GLOB '*.tiff' OR lower(json_extract(i.summary,'$.preview')) GLOB '*.webp' OR lower(json_extract(i.summary,'$.preview')) GLOB '*.gif') THEN 'image' ELSE json_extract(i.summary,'$.kind') END";

pub(super) fn rule(index: usize, r: &Rule, q: &Value, params: &mut Vec<SqlValue>) -> String {
    match r.field.as_str() {
        "app" | "device" | "kind" => {
            let field = match r.field.as_str() {
                "app" => "json_extract(i.summary,'$.source_app_id')",
                "device" => "json_extract(i.summary,'$.source_device_id')",
                _ => KIND,
            };
            params.push(SqlValue::Text(r.value.clone()));
            let op = if r.op == "is_not" { "<>" } else { "=" };
            // Missing source metadata does not match a negative rule either.
            format!("({field} {op} ?)")
        }
        "text" => {
            let needle = r.value.to_lowercase();
            let candidate = if r.op != "not_contains" {
                super::fts_query(&needle)
            } else {
                None
            };
            if let Some(candidate) = &candidate {
                params.push(SqlValue::Text(candidate.clone()));
            }
            params.extend((0..3).map(|_| SqlValue::Text(needle.clone())));
            let found = "(instr(i.search_text,?)>0 OR instr(unicode_lower(COALESCE(i.title,'')),?)>0 OR instr(unicode_lower(COALESCE(i.ocr_text,'')),?)>0)";
            if r.op == "not_contains" {
                format!("NOT {found}")
            } else if candidate.is_some() {
                format!(
                    "(i.id IN (SELECT rowid FROM cl_items_fts WHERE cl_items_fts MATCH ?) AND {found})"
                )
            } else {
                found.into()
            }
        }
        "date" => {
            let window = q["rule_windows"].get(index.to_string());
            let explicit = window.and_then(|v| v["start"].as_u64().zip(v["end"].as_u64()));
            let start = q["day_start_ms"].as_u64().unwrap_or(0);
            let end = q["day_end_ms"].as_u64().unwrap_or(0);
            let bounds = explicit.or_else(|| match r.op.as_str() {
                "today" => Some((start, end)),
                "yesterday" => Some((q["yesterday_start_ms"].as_u64().unwrap_or(start), start)),
                "last_days" => Some((
                    start.saturating_sub(
                        r.value
                            .parse::<u64>()
                            .unwrap_or(1)
                            .saturating_sub(1)
                            .saturating_mul(86400000),
                    ),
                    end,
                )),
                "between" => Some((
                    r.value.parse().unwrap_or(u64::MAX),
                    r.value2.parse().unwrap_or(0),
                )),
                _ => None,
            });
            let Some((start, end)) = bounds else {
                return "0".into();
            };
            params.extend([
                SqlValue::Integer(start.min(i64::MAX as u64) as i64),
                SqlValue::Integer(end.min(i64::MAX as u64) as i64),
            ]);
            "(json_extract(i.summary,'$.copied_at_ms')>=? AND json_extract(i.summary,'$.copied_at_ms')<?)".into()
        }
        _ => "0".into(),
    }
}

pub(super) fn configure(db: &Connection) -> Result<()> {
    use rusqlite::functions::FunctionFlags;
    db.create_scalar_function(
        "unicode_lower",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| Ok(ctx.get::<String>(0)?.to_lowercase()),
    )
    .map_err(err)
}
