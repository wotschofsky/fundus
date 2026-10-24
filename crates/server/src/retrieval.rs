use crate::{
    config::App,
    error::{Error, Result},
    provider,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{QueryBuilder, Row, Sqlite};
use std::collections::HashMap;
#[derive(Clone, Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
pub struct Filters {
    pub from: Option<String>,
    pub to: Option<String>,
    pub labels: Option<String>,
    pub person_id: Option<String>,
    pub stage: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<u32>,
    pub trash: Option<bool>,
}
fn filter<'a>(q: &mut QueryBuilder<'a, Sqlite>, f: &Filters) -> Result<()> {
    q.push(if f.trash.unwrap_or(false) {
        " AND e.trashed_at IS NOT NULL"
    } else {
        " AND e.trashed_at IS NULL"
    });
    for (field, value) in [("e.recorded_at >= ", &f.from), ("e.recorded_at < ", &f.to)] {
        if let Some(s) = value {
            let day = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|_| Error::bad("Date filters must use YYYY-MM-DD (UTC)"))?;
            let day = if field.contains('<') {
                day.succ_opt()
                    .ok_or_else(|| Error::bad("Invalid end date"))?
            } else {
                day
            };
            q.push(" AND ")
                .push(field)
                .push_bind(format!("{day}T00:00:00.000Z"));
        }
    }
    if let Some(labels) = &f.labels {
        for label in labels.split(',').filter(|s| !s.trim().is_empty()).take(20) {
            q.push(
                " AND EXISTS(SELECT 1 FROM entry_labels el WHERE el.entry_id=e.id AND el.label=",
            )
            .push_bind(label.trim().to_string())
            .push(")");
        }
    }
    if let Some(person) = f.person_id.as_ref().filter(|s| !s.trim().is_empty()) {
        q.push(" AND EXISTS(SELECT 1 FROM speakers sp WHERE sp.entry_id=e.id AND sp.person_id=")
            .push_bind(person.clone())
            .push(")");
    }
    if let Some(stage) = f.stage.as_ref().filter(|s| !s.trim().is_empty()) {
        q.push(" AND e.stage=").push_bind(stage.clone());
    }
    Ok(())
}
fn entry(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({"id":row.get::<String,_>("id"),"title":row.get::<String,_>("title"),"recorded_at":row.get::<Option<String>,_>("recorded_at"),"duration_ms":row.get::<i64,_>("duration_ms"),"trim_start_frame":row.get::<Option<i64>,_>("trim_start_frame"),"trim_end_frame":row.get::<Option<i64>,_>("trim_end_frame"),"trashed_at":row.get::<Option<String>,_>("trashed_at"),"timezone":row.get::<String,_>("timezone"),"stage":row.get::<String,_>("stage"),"active_stage":row.get::<Option<String>,_>("active_stage"),"pause_requested":row.get::<bool,_>("pause_requested") && row.get::<String,_>("stage") != "paused","failure":row.get::<Option<String>,_>("failure"),"warnings":serde_json::from_str::<Value>(&row.get::<String,_>("warnings")).unwrap_or(json!([])),"summary":row.get::<Option<String>,_>("summary"),"created_at":row.get::<String,_>("created_at"),"labels":serde_json::from_str::<Value>(&row.get::<String,_>("labels")).unwrap_or(json!([])),"audio_available":row.get::<Option<String>,_>("audio_path").is_some()})
}
const SELECT: &str = "SELECT e.*,(SELECT json_group_array(label) FROM entry_labels WHERE entry_id=e.id) AS labels FROM entries e WHERE 1=1";
pub async fn list(app: &App, f: Filters) -> Result<Value> {
    let limit = f.limit.unwrap_or(30).clamp(1, 100);
    let offset = f.cursor.unwrap_or(0);
    let mut q = QueryBuilder::new(SELECT);
    filter(&mut q, &f)?;
    q.push(" ORDER BY COALESCE(e.recorded_at,e.created_at) DESC,e.id LIMIT ")
        .push_bind(limit as i64 + 1)
        .push(" OFFSET ")
        .push_bind(offset as i64);
    let rows = q.build().fetch_all(&app.db).await?;
    let more = rows.len() > limit as usize;
    Ok(
        json!({"entries":rows.iter().take(limit as usize).map(entry).collect::<Vec<_>>(),"next_cursor":if more{Some(offset+limit)}else{None},"date_filter_timezone":"UTC"}),
    )
}
pub async fn get(app: &App, id: &str) -> Result<Value> {
    get_entry(app, id, false).await
}
pub async fn get_including_trash(app: &App, id: &str) -> Result<Value> {
    get_entry(app, id, true).await
}
async fn get_entry(app: &App, id: &str, include_trash: bool) -> Result<Value> {
    let clause = if include_trash {
        ""
    } else {
        " AND e.trashed_at IS NULL"
    };
    let row = sqlx::query(&format!("{SELECT} AND e.id=?{clause}"))
        .bind(id)
        .fetch_optional(&app.db)
        .await?
        .ok_or_else(Error::not_found)?;
    let mut v = entry(&row);
    let speakers=sqlx::query("SELECT s.label,s.person_id,s.state,s.evidence,p.name FROM speakers s LEFT JOIN people p ON p.id=s.person_id WHERE s.entry_id=? ORDER BY s.label").bind(id).fetch_all(&app.db).await?;
    v["speakers"]=json!(speakers.iter().map(|r|json!({"label":r.get::<String,_>("label"),"person_id":r.get::<Option<String>,_>("person_id"),"name":r.get::<Option<String>,_>("name"),"state":r.get::<String,_>("state"),"evidence":serde_json::from_str::<Value>(&r.get::<String,_>("evidence")).unwrap_or(json!([]))})).collect::<Vec<_>>());
    let sources=sqlx::query("SELECT es.offset_ms,es.recorded_at,u.original_name,u.format_json FROM entry_sources es JOIN uploads u ON u.id=es.upload_id WHERE es.entry_id=? ORDER BY es.position").bind(id).fetch_all(&app.db).await?;
    v["sources"]=json!(sources.iter().map(|r|json!({"offset_ms":r.get::<i64,_>("offset_ms"),"recorded_at":r.get::<Option<String>,_>("recorded_at"),"filename":r.get::<String,_>("original_name"),"format":serde_json::from_str::<Value>(&r.get::<String,_>("format_json")).unwrap_or(Value::Null)})).collect::<Vec<_>>());
    v["url"] = json!(format!("{}entries/{id}", app.config.public_url));
    Ok(v)
}
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
pub struct Read {
    pub entry_id: String,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub cursor: Option<u32>,
    pub limit: Option<u32>,
}
pub async fn transcript(app: &App, r: Read) -> Result<Value> {
    let known: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE id=? AND trashed_at IS NULL")
            .bind(&r.entry_id)
            .fetch_one(&app.db)
            .await?;
    if known == 0 {
        return Err(Error::not_found());
    }
    let limit = r.limit.unwrap_or(100).clamp(1, 300);
    let cursor = r.cursor.unwrap_or(0);
    let from = r.from_ms.unwrap_or(0);
    let to = r.to_ms.unwrap_or(i64::MAX);
    if from < 0 || to < from {
        return Err(Error::bad("Invalid audio time range"));
    }
    let rows=sqlx::query("SELECT s.*,sp.person_id,p.name FROM segments s LEFT JOIN speakers sp ON sp.entry_id=s.entry_id AND sp.label=s.speaker LEFT JOIN people p ON p.id=sp.person_id WHERE s.entry_id=? AND s.end_ms>? AND s.start_ms<? AND s.position>=? ORDER BY s.position LIMIT ?").bind(&r.entry_id).bind(from).bind(to).bind(cursor as i64).bind(limit as i64+1).fetch_all(&app.db).await?;
    let more = rows.len() > limit as usize;
    let next = if more {
        rows.get(limit as usize)
            .map(|r| r.get::<i64, _>("position"))
    } else {
        None
    };
    Ok(
        json!({"entry_id":r.entry_id,"segments":rows.iter().take(limit as usize).map(|s|json!({"id":s.get::<String,_>("id"),"position":s.get::<i64,_>("position"),"speaker":s.get::<String,_>("speaker"),"speaker_name":s.get::<Option<String>,_>("name"),"person_id":s.get::<Option<String>,_>("person_id"),"start_ms":s.get::<i64,_>("start_ms"),"end_ms":s.get::<i64,_>("end_ms"),"text":s.get::<String,_>("text")})).collect::<Vec<_>>(),"truncated":more,"next_cursor":next}),
    )
}
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
pub struct Search {
    pub query: String,
    #[serde(flatten)]
    pub filters: Filters,
}
fn lexical(query: &str) -> String {
    let tokens: Vec<_> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .take(32)
        .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
        .collect();
    tokens.join(" OR ")
}
pub async fn search(app: &App, s: Search) -> Result<Value> {
    if s.query.trim().is_empty() || s.query.len() > 4096 {
        return Err(Error::bad("Search requires 1–4096 characters"));
    }
    let mut q = QueryBuilder::new(
        "SELECT p.*,e.title,e.recorded_at FROM passages p JOIN entries e ON e.id=p.entry_id WHERE 1=1",
    );
    filter(&mut q, &s.filters)?;
    let rows = q.build().fetch_all(&app.db).await?;
    let mut ranks: HashMap<String, f32> = HashMap::new();
    let expression = lexical(&s.query);
    if !expression.is_empty() {
        let mut q = QueryBuilder::new(
            "SELECT passage_fts.id FROM passage_fts JOIN entries e ON e.id=passage_fts.entry_id WHERE passage_fts MATCH ",
        );
        q.push_bind(expression);
        filter(&mut q, &s.filters)?;
        q.push(" ORDER BY bm25(passage_fts) LIMIT 100");
        let lexical = q.build().fetch_all(&app.db).await?;
        for (i, r) in lexical.iter().enumerate() {
            *ranks.entry(r.get("id")).or_default() += 1. / (61. + i as f32);
        }
    }
    let mut semantic = false;
    if let Ok(v) = provider::embeddings(app, std::slice::from_ref(&s.query)).await {
        let probe = &v[0];
        let mut scores = vec![];
        for row in &rows {
            let encoded: Option<String> = row.get("embedding");
            if let Some(encoded) = encoded
                && let Ok(vector) = serde_json::from_str::<Vec<f32>>(&encoded)
                && vector.len() == probe.len()
            {
                scores.push((
                    row.get::<String, _>("id"),
                    crate::voice::cosine(probe, &vector),
                ));
            }
        }
        scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (i, (id, score)) in scores.into_iter().take(100).enumerate() {
            if score > 0.2 {
                *ranks.entry(id).or_default() += 1. / (61. + i as f32);
            }
        }
        semantic = true;
    }
    let mut matched: Vec<_> = rows
        .iter()
        .filter_map(|r| {
            ranks
                .get(&r.get::<String, _>("id"))
                .map(|score| (r, *score))
        })
        .collect();
    matched.sort_by(|(a, x), (b, y)| {
        y.total_cmp(x)
            .then(a.get::<String, _>("id").cmp(&b.get::<String, _>("id")))
    });
    let pending: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE stage NOT IN ('complete','paused') AND trashed_at IS NULL")
        .fetch_one(&app.db)
        .await?;
    let missing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM passages p JOIN entries e ON e.id=p.entry_id WHERE p.embedding IS NULL AND e.trashed_at IS NULL")
        .fetch_one(&app.db)
        .await?;
    Ok(
        json!({"results":matched.into_iter().take(s.filters.limit.unwrap_or(12).clamp(1,50) as usize).map(|(r,score)|{let eid:String=r.get("entry_id");let start:i64=r.get("start_ms");json!({"id":r.get::<String,_>("id"),"entry_id":eid,"title":r.get::<String,_>("title"),"recorded_at":r.get::<Option<String>,_>("recorded_at"),"start_ms":start,"end_ms":r.get::<i64,_>("end_ms"),"excerpt":r.get::<String,_>("text"),"score":score,"url":format!("{}entries/{eid}?t={}",app.config.public_url,start as f64/1000.)})}).collect::<Vec<_>>(),"mode":if semantic{"hybrid"}else{"keyword"},"coverage":{"pending_entries":pending,"passages_without_embeddings":missing},"partial_results":pending>0||missing>0||!semantic}),
    )
}
pub async fn labels(app: &App) -> Result<Value> {
    let rows=sqlx::query("SELECT l.name,COUNT(e.id) AS count FROM labels l LEFT JOIN entry_labels el ON el.label=l.name LEFT JOIN entries e ON e.id=el.entry_id AND e.trashed_at IS NULL GROUP BY l.name ORDER BY l.name").fetch_all(&app.db).await?;
    Ok(
        json!({"labels":rows.iter().map(|r|json!({"name":r.get::<String,_>("name"),"count":r.get::<i64,_>("count")})).collect::<Vec<_>>()}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_query_escapes_fts_syntax() {
        assert_eq!(
            lexical("he said: \"hello\" AND (world)*"),
            "\"he\" OR \"said\" OR \"hello\" OR \"AND\" OR \"world\""
        );
    }
}
