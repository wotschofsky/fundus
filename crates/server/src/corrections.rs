//! Recording-local speaker splitting: discover two voices, infer the rejected group from a
//! passage (or ask which voice to keep), then apply a reversible correction against its snapshot.
use crate::{
    auth::Access,
    config::{App, id, now},
    error::{Error, Result},
    pipeline,
    voice,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Turn {
    id: String,
    speaker: String,
    start_ms: i64,
    end_ms: i64,
    text: String,
    confidence: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Speaker {
    label: String,
    person_id: Option<String>,
    name: Option<String>,
    state: String,
    evidence: String,
}
#[derive(Debug, Deserialize)]
pub struct Request {
    pub speaker: String,
    pub not_segment_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct VoiceGroup {
    id: usize,
    example: Turn,
    passages: Vec<Turn>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    id: String,
    source_label: String,
    source_name: String,
    groups: [VoiceGroup; 2],
    unchanged: usize,
    #[serde(default)]
    suggested_keep: Option<usize>,
}
#[derive(Deserialize)]
pub struct Apply {
    pub keep_group: usize,
    pub target_label: Option<String>,
    pub person_id: Option<String>,
    pub new_name: Option<String>,
}

async fn turns(db: &mut sqlx::SqliteConnection, entry: &str) -> Result<Vec<Turn>> {
    Ok(sqlx::query_as!(Turn, "SELECT id AS \"id!\",speaker,start_ms,end_ms,text,confidence FROM segments WHERE entry_id=? ORDER BY position", entry).fetch_all(db).await?)
}
async fn speakers(db: &mut sqlx::SqliteConnection, entry: &str) -> Result<Vec<Speaker>> {
    Ok(sqlx::query_as!(Speaker, "SELECT s.label,s.person_id,p.name AS \"name?\",s.state,s.evidence FROM speakers s LEFT JOIN people p ON p.id=s.person_id WHERE s.entry_id=? ORDER BY s.label", entry).fetch_all(db).await?)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PassageLock {
    segment_id: String,
    speaker: String,
    created_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Sample {
    id: String,
    person_id: String,
    speaker: String,
    model: String,
    vector_json: String,
    provenance: String,
    created_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Probe {
    speaker: String,
    model: String,
    vector_json: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Notes {
    title: String,
    title_manual: i64,
    summary: Option<String>,
    /// Missing in snapshots taken before a summary could be edited by hand.
    #[serde(default)]
    summary_manual: i64,
    warnings: String,
    revision: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Before {
    rows: Vec<Turn>,
    speakers: Vec<Speaker>,
    locks: Vec<PassageLock>,
    samples: Vec<Sample>,
    probes: Vec<Probe>,
    notes: Notes,
}

async fn capture(db: &mut sqlx::SqliteConnection, entry: &str) -> Result<Before> {
    let rows = turns(db, entry).await?;
    let speakers = speakers(db, entry).await?;
    let locks = sqlx::query_as!(PassageLock, "SELECT segment_id AS \"segment_id!\",speaker,created_at FROM speaker_corrections WHERE entry_id=? ORDER BY segment_id", entry).fetch_all(&mut *db).await?;
    let samples = sqlx::query_as!(Sample, "SELECT id AS \"id!\",person_id,speaker,model,vector_json,provenance,created_at FROM voice_samples WHERE entry_id=? ORDER BY id", entry).fetch_all(&mut *db).await?;
    let probes = sqlx::query_as!(Probe, "SELECT speaker,model,vector_json FROM voice_probes WHERE entry_id=? ORDER BY speaker,model", entry).fetch_all(&mut *db).await?;
    let notes = sqlx::query_as!(Notes, "SELECT title,title_manual AS \"title_manual!: i64\",summary,summary_manual AS \"summary_manual!: i64\",warnings,notes_revision AS revision FROM entries WHERE id=?", entry).fetch_one(db).await?;
    Ok(Before {
        rows,
        speakers,
        locks,
        samples,
        probes,
        notes,
    })
}
fn fingerprint(state: &Before) -> Result<String> {
    // Generated notes can refresh without invalidating Undo; a title or summary edited by the
    // owner cannot. The summary is part of the hash only once it has been edited, so snapshots
    // from before that flag existed still match.
    let title = (state.notes.title_manual != 0).then_some(&state.notes.title);
    let base = (
        &state.rows,
        &state.speakers,
        &state.locks,
        &state.samples,
        &state.probes,
        state.notes.title_manual,
        title,
    );
    let encoded = if state.notes.summary_manual == 0 {
        serde_json::to_string(&base)
    } else {
        serde_json::to_string(&(base, state.notes.summary_manual, &state.notes.summary))
    }
    .map_err(anyhow::Error::from)?;
    Ok(crate::auth::hash(&encoded))
}
async fn undo_available(
    db: &mut sqlx::SqliteConnection,
    entry: &str,
    expected: &str,
    revision: i64,
) -> Result<bool> {
    let row = sqlx::query!(
        "SELECT notes_revision,stage,requested_work,trashed_at FROM entries WHERE id=?",
        entry
    )
    .fetch_optional(&mut *db)
    .await?
    .ok_or_else(Error::not_found)?;
    if row.trashed_at.is_some()
        || row.notes_revision != revision
        || (row.stage != "complete" && row.requested_work.as_deref() != Some("summary"))
    {
        return Ok(false);
    }
    Ok(fingerprint(&capture(db, entry).await?)? == expected)
}

pub async fn undo_status(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(entry): Path<String>,
) -> Result<Json<serde_json::Value>> {
    access.admin()?;
    let mut tx = app.db.begin().await?;
    let saved = sqlx::query!(
        "SELECT after_snapshot,revision FROM speaker_correction_history WHERE entry_id=?",
        entry
    )
    .fetch_optional(&mut *tx)
    .await?;
    let available = if let Some(saved) = saved {
        undo_available(&mut tx, &entry, &saved.after_snapshot, saved.revision).await?
    } else {
        false
    };
    tx.commit().await?;
    Ok(Json(serde_json::json!({"available":available})))
}

pub async fn undo(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(entry): Path<String>,
) -> Result<Json<serde_json::Value>> {
    access.admin()?;
    let _lock = app.mutations.lock().await;
    let mut tx = app.db.begin().await?;
    let saved = sqlx::query!("SELECT before_json,after_snapshot,revision,changed FROM speaker_correction_history WHERE entry_id=?", entry).fetch_optional(&mut *tx).await?.ok_or_else(|| Error::conflict("There is no speaker correction to undo"))?;
    if !undo_available(&mut tx, &entry, &saved.after_snapshot, saved.revision).await? {
        return Err(Error::conflict(
            "This recording was edited after the correction. Undo would overwrite those edits",
        ));
    }
    let before: Before = serde_json::from_str(&saved.before_json).map_err(anyhow::Error::from)?;
    for row in &before.rows {
        sqlx::query!(
            "UPDATE segments SET speaker=? WHERE entry_id=? AND id=?",
            row.speaker,
            entry,
            row.id
        )
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query!("DELETE FROM speaker_corrections WHERE entry_id=?", entry)
        .execute(&mut *tx)
        .await?;
    sqlx::query!("DELETE FROM voice_samples WHERE entry_id=?", entry)
        .execute(&mut *tx)
        .await?;
    sqlx::query!("DELETE FROM voice_probes WHERE entry_id=?", entry)
        .execute(&mut *tx)
        .await?;
    sqlx::query!("DELETE FROM speakers WHERE entry_id=?", entry)
        .execute(&mut *tx)
        .await?;
    for speaker in &before.speakers {
        sqlx::query!(
            "INSERT INTO speakers(entry_id,label,person_id,state,evidence) VALUES(?,?,?,?,?)",
            entry,
            speaker.label,
            speaker.person_id,
            speaker.state,
            speaker.evidence
        )
        .execute(&mut *tx)
        .await?;
    }
    for row in &before.locks {
        sqlx::query!("INSERT INTO speaker_corrections(segment_id,entry_id,speaker,created_at) VALUES(?,?,?,?)", row.segment_id, entry, row.speaker, row.created_at).execute(&mut *tx).await?;
    }
    for sample in &before.samples {
        sqlx::query!("INSERT INTO voice_samples(id,person_id,entry_id,speaker,model,vector_json,provenance,created_at) SELECT ?,?,?,?,?,?,?,? WHERE EXISTS(SELECT 1 FROM people WHERE id=?)", sample.id, sample.person_id, entry, sample.speaker, sample.model, sample.vector_json, sample.provenance, sample.created_at, sample.person_id).execute(&mut *tx).await?;
    }
    for probe in &before.probes {
        sqlx::query!(
            "INSERT INTO voice_probes(entry_id,speaker,model,vector_json) VALUES(?,?,?,?)",
            entry,
            probe.speaker,
            probe.model,
            probe.vector_json
        )
        .execute(&mut *tx)
        .await?;
    }
    let timestamp = now();
    // Incrementing the generation also prevents a summary already in flight from overwriting Undo.
    sqlx::query!("UPDATE entries SET title=?,title_manual=?,summary=?,summary_manual=?,warnings=?,notes_revision=notes_revision+1,stage='complete',active_stage=NULL,requested_work=NULL,pause_requested=0,failure=NULL,updated_at=? WHERE id=?", before.notes.title, before.notes.title_manual, before.notes.summary, before.notes.summary_manual, before.notes.warnings, timestamp, entry).execute(&mut *tx).await?;
    sqlx::query!(
        "UPDATE passage_fts SET title=? WHERE entry_id=?",
        before.notes.title,
        entry
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM speaker_correction_history WHERE entry_id=?",
        entry
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM speaker_correction_reviews WHERE entry_id=?",
        entry
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Err(error) = pipeline::refresh_summary_passage(&app, &entry).await {
        tracing::warn!(%entry, %error, "could not index the summary");
    }
    Ok(Json(serde_json::json!({"restored":saved.changed})))
}

async fn snapshot(
    db: &mut sqlx::SqliteConnection,
    entry: &str,
) -> Result<(Vec<Turn>, Vec<Speaker>, BTreeSet<String>, String)> {
    let stage = sqlx::query_scalar!(
        "SELECT stage FROM entries WHERE id=? AND trashed_at IS NULL",
        entry
    )
    .fetch_optional(&mut *db)
    .await?
    .ok_or_else(Error::not_found)?;
    if stage != "complete" {
        return Err(Error::conflict(
            "Wait until this recording finishes processing",
        ));
    }
    let before = capture(db, entry).await?;
    let hash = fingerprint(&before)?;
    let locked = before.locks.iter().map(|r| r.segment_id.clone()).collect();
    Ok((before.rows, before.speakers, locked, hash))
}

fn clean(row: &Turn, rows: &[Turn]) -> bool {
    row.end_ms - row.start_ms >= 3000
        && row.confidence.is_none_or(|c| c >= 0.6)
        && !rows.iter().any(|other| {
            other.id != row.id && other.start_ms < row.end_ms && other.end_ms > row.start_ms
        })
}

/// Deterministic spherical two-means. Isolated outliers cannot become seeds when there is enough
/// speech to require support. Low similarity and low-margin passages never enter the proposal.
fn split_voices(rows: &[Turn], vectors: &BTreeMap<String, Vec<f32>>) -> Option<[VoiceGroup; 2]> {
    let eligible: Vec<_> = rows
        .iter()
        .filter(|r| {
            vectors
                .get(&r.id)
                .is_some_and(|v| voice::cosine(v, v) > 0.99)
        })
        .collect();
    if eligible.len() < 2 {
        return None;
    }
    let support = if eligible.len() >= 4 { 2 } else { 1 };
    let samples: Vec<_> = (0..eligible.len().min(64))
        .map(|k| eligible[k * eligible.len() / eligible.len().min(64)])
        .filter(|r| {
            eligible
                .iter()
                .filter(|other| {
                    voice::cosine(&vectors[&r.id], &vectors[&other.id]) >= voice::SAME_VOICE
                })
                .count()
                >= support
        })
        .collect();
    let mut best = (voice::SAME_VOICE, None);
    for (i, a) in samples.iter().enumerate() {
        for b in samples.iter().skip(i + 1) {
            let score = voice::cosine(&vectors[&a.id], &vectors[&b.id]);
            if score < best.0 {
                best = (score, Some((*a, *b)));
            }
        }
    }
    let (a, b) = best.1?;
    let mut centers = [
        voice::normalized(vectors[&a.id].clone()),
        voice::normalized(vectors[&b.id].clone()),
    ];
    let mut previous = vec![];
    let mut members = [vec![], vec![]];
    for _ in 0..20 {
        members = [vec![], vec![]];
        let mut assignment = vec![];
        for row in &eligible {
            let scores = [
                voice::cosine(&vectors[&row.id], &centers[0]),
                voice::cosine(&vectors[&row.id], &centers[1]),
            ];
            let group = usize::from(scores[1] > scores[0]);
            if scores[group] >= voice::SAME_VOICE && scores[group] - scores[1 - group] >= 0.12 {
                members[group].push((*row).clone());
                assignment.push(Some(group));
            } else {
                assignment.push(None);
            }
        }
        if members.iter().any(|g| g.len() < support) {
            return None;
        }
        if assignment == previous {
            break;
        }
        previous = assignment;
        for group in 0..2 {
            let mut sum = vec![0.; centers[group].len()];
            for row in &members[group] {
                for (s, x) in sum.iter_mut().zip(&vectors[&row.id]) {
                    *s += x;
                }
            }
            centers[group] = voice::normalized(sum);
        }
    }
    if voice::cosine(&centers[0], &centers[1]) >= voice::SAME_VOICE {
        return None;
    }
    // Never include a last-iteration assignment that no longer matches its updated center.
    for group in 0..2 {
        members[group].retain(|r| {
            let own = voice::cosine(&vectors[&r.id], &centers[group]);
            own >= voice::SAME_VOICE
                && own - voice::cosine(&vectors[&r.id], &centers[1 - group]) >= 0.12
        });
        if members[group].len() < support {
            return None;
        }
    }
    let mut groups: Vec<_> = members
        .into_iter()
        .enumerate()
        .map(|(i, passages)| {
            let mut example = passages
                .iter()
                .max_by(|a, b| {
                    voice::cosine(&vectors[&a.id], &centers[i])
                        .total_cmp(&voice::cosine(&vectors[&b.id], &centers[i]))
                        .then_with(|| b.start_ms.cmp(&a.start_ms))
                })
                .unwrap()
                .clone();
            example.end_ms = example.end_ms.min(example.start_ms + 10_000);
            VoiceGroup {
                id: i,
                example,
                passages,
            }
        })
        .collect();
    groups.sort_by_key(|g| g.passages[0].start_ms);
    for (i, group) in groups.iter_mut().enumerate() {
        group.id = i;
    }
    groups.try_into().ok()
}

fn retained_voice(groups: &[VoiceGroup; 2], not_segment: Option<&str>) -> Option<usize> {
    let id = not_segment?;
    groups
        .iter()
        .find(|g| g.passages.iter().any(|r| r.id == id))
        .map(|g| 1 - g.id)
}

pub async fn preview(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path(entry): Path<String>,
    Json(request): Json<Request>,
) -> Result<Json<Proposal>> {
    access.admin()?;
    let (all_rows, speakers, _, hash) = {
        let mut tx = app.db.begin().await?;
        let value = snapshot(&mut tx, &entry).await?;
        tx.commit().await?;
        value
    };
    if let Some(segment) = &request.not_segment_id
        && !all_rows
            .iter()
            .any(|r| &r.id == segment && r.speaker == request.speaker)
    {
        return Err(Error::bad(
            "The selected passage is no longer assigned to this speaker",
        ));
    }
    let index = speakers
        .iter()
        .position(|s| s.label == request.speaker)
        .ok_or_else(Error::not_found)?;
    let source = &speakers[index];
    let rows: Vec<_> = all_rows
        .iter()
        .filter(|r| r.speaker == request.speaker)
        .cloned()
        .collect();
    let eligible: Vec<_> = rows
        .iter()
        .filter(|r| clean(r, &all_rows))
        .map(|r| (r.id.clone(), r.start_ms, r.end_ms))
        .collect();
    if eligible.len() < 2 {
        return Err(Error::bad(
            "There is not enough clear speech to separate this speaker. You can rename the whole group in the sidebar instead",
        ));
    }
    let model = app.config.voice_model.as_ref().filter(|p| p.is_file()).ok_or_else(|| Error::bad("Voice splitting is unavailable. Configure the server's speaker model and ONNX Runtime first"))?;
    let audio = app.entry_dir(&entry).join("playback.m4a");
    if !audio.is_file() {
        return Err(Error::bad("This recording's audio is unavailable"));
    }
    let vectors = voice::measure_passages(&app, model, &audio, &eligible).await?;
    let groups = split_voices(&rows, &vectors).ok_or_else(|| {
        Error::bad("Couldn't clearly distinguish two voices in this group. Nothing has changed")
    })?;
    let suggested_keep = retained_voice(&groups, request.not_segment_id.as_deref());
    let unchanged = rows.len() - groups.iter().map(|g| g.passages.len()).sum::<usize>();
    let proposal = Proposal {
        id: id(),
        source_label: request.speaker,
        source_name: source
            .name
            .clone()
            .unwrap_or_else(|| format!("Speaker {}", index + 1)),
        groups,
        unchanged,
        suggested_keep,
    };
    let mut tx = app.db.begin().await?;
    let (_, _, _, current) = snapshot(&mut tx, &entry).await?;
    if current != hash {
        return Err(Error::conflict(
            "The transcript changed during analysis. Try splitting the speaker again",
        ));
    }
    let payload = serde_json::to_string(&proposal).map_err(anyhow::Error::from)?;
    let expires = chrono::Utc::now().timestamp() + 3600;
    // A cancelled or slower analysis must not invalidate a newer open review.
    sqlx::query!("DELETE FROM speaker_correction_reviews WHERE expires_at < unixepoch()")
        .execute(&mut *tx)
        .await?;
    sqlx::query!("INSERT INTO speaker_correction_reviews(id,entry_id,snapshot,proposal,expires_at) VALUES(?,?,?,?,?)", proposal.id, entry, hash, payload, expires).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(proposal))
}

pub async fn apply(
    State(app): State<App>,
    Extension(access): Extension<Access>,
    Path((entry, review)): Path<(String, String)>,
    Json(request): Json<Apply>,
) -> Result<Json<serde_json::Value>> {
    access.admin()?;
    if request.keep_group > 1 {
        return Err(Error::bad("Choose which voice keeps the current name"));
    }
    let _lock = app.mutations.lock().await;
    let mut tx = app.db.begin().await?;
    if let Some(saved) = sqlx::query!("SELECT id AS \"id!\",after_snapshot,revision,changed,summary_refresh FROM speaker_correction_history WHERE entry_id=? AND id=?", entry, review).fetch_optional(&mut *tx).await?
        && undo_available(&mut tx, &entry, &saved.after_snapshot, saved.revision).await?
    {
        return Ok(Json(serde_json::json!({"changed":saved.changed,"undo_id":saved.id,"summary_refresh":saved.summary_refresh != 0})));
    }
    let saved = sqlx::query!("SELECT snapshot,proposal FROM speaker_correction_reviews WHERE id=? AND entry_id=? AND expires_at>=unixepoch()", review, entry).fetch_optional(&mut *tx).await?.ok_or_else(|| Error::conflict("This review expired. Split the speaker again"))?;
    let (_, speakers, _, current) = snapshot(&mut tx, &entry).await?;
    if current != saved.snapshot {
        return Err(Error::conflict(
            "The transcript or its speakers changed. Split the speaker again",
        ));
    }
    let proposal: Proposal = serde_json::from_str(&saved.proposal)
        .map_err(|_| Error::conflict("This review is out of date. Split the speaker again"))?;
    let source = speakers
        .iter()
        .find(|s| s.label == proposal.source_label)
        .ok_or_else(Error::not_found)?;
    let before = capture(&mut tx, &entry).await?;
    let new_name = request.new_name.map(|n| n.trim().to_string());
    if new_name
        .as_ref()
        .is_some_and(|n| n.is_empty() || n.chars().count() > 120)
    {
        return Err(Error::bad("Use a name between 1 and 120 characters"));
    }
    if usize::from(request.target_label.is_some())
        + usize::from(request.person_id.is_some())
        + usize::from(new_name.is_some())
        > 1
    {
        return Err(Error::bad("Choose one name for the other voice"));
    }
    let (target_label, mut person, create) = if let Some(label) = request.target_label {
        let target = speakers
            .iter()
            .find(|s| s.label == label)
            .ok_or_else(Error::not_found)?;
        if label == source.label {
            return Err(Error::bad("The two voices must be kept separate"));
        }
        (label, target.person_id.clone(), false)
    } else {
        (format!("corrected-{}", id()), request.person_id, true)
    };
    if let Some(person_id) = &person {
        if source.person_id.as_ref() == Some(person_id) {
            return Err(Error::bad("Choose a different person for the other voice"));
        }
        if sqlx::query_scalar!("SELECT COUNT(*) FROM people WHERE id=?", person_id)
            .fetch_one(&mut *tx)
            .await?
            == 0
        {
            return Err(Error::conflict(
                "The other person was removed. Choose another name",
            ));
        }
    }
    if let Some(name) = new_name {
        let person_id = id();
        sqlx::query!("INSERT INTO people(id,name) VALUES(?,?)", person_id, name)
            .execute(&mut *tx)
            .await?;
        person = Some(person_id);
    }
    if create {
        let state = if person.is_some() {
            "manual"
        } else {
            "manual_unknown"
        };
        sqlx::query!(
            "INSERT INTO speakers(entry_id,label,person_id,state) VALUES(?,?,?,?)",
            entry,
            target_label,
            person,
            state
        )
        .execute(&mut *tx)
        .await?;
    }
    let timestamp = now();
    let moving = &proposal.groups[1 - request.keep_group];
    for row in &moving.passages {
        sqlx::query!(
            "UPDATE segments SET speaker=? WHERE entry_id=? AND id=?",
            target_label,
            entry,
            row.id
        )
        .execute(&mut *tx)
        .await?;
    }
    // The owner accepts both groups together. Later automated analysis cannot erase this split.
    for group in &proposal.groups {
        let label = if group.id == request.keep_group {
            &proposal.source_label
        } else {
            &target_label
        };
        for row in &group.passages {
            sqlx::query!("INSERT INTO speaker_corrections(segment_id,entry_id,speaker,created_at) VALUES(?,?,?,?) ON CONFLICT(segment_id) DO UPDATE SET speaker=excluded.speaker,created_at=excluded.created_at", row.id, entry, label, timestamp).execute(&mut *tx).await?;
        }
    }
    for label in [&proposal.source_label, &target_label] {
        sqlx::query!(
            "DELETE FROM voice_probes WHERE entry_id=? AND speaker=?",
            entry,
            label
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "DELETE FROM voice_samples WHERE entry_id=? AND speaker=?",
            entry,
            label
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("UPDATE speakers SET evidence='[]',state=CASE WHEN person_id IS NULL THEN 'manual_unknown' ELSE 'manual' END WHERE entry_id=? AND label=?", entry, label).execute(&mut *tx).await?;
    }
    let revision = before.notes.revision + 1;
    // A hand-edited summary stays. A generated one is cleared, and rewritten when a model is configured.
    let clear_summary = before.notes.summary_manual == 0;
    let refresh_summary = clear_summary && !app.config.openai_key.is_empty();
    sqlx::query!("UPDATE entries SET summary=CASE WHEN ? THEN NULL ELSE summary END,notes_revision=?,stage=CASE WHEN ? THEN 'queued' ELSE 'complete' END,requested_work=CASE WHEN ? THEN 'summary' ELSE NULL END,active_stage=NULL,pause_requested=0,failure=NULL,updated_at=? WHERE id=?", clear_summary, revision, refresh_summary, refresh_summary, timestamp, entry).execute(&mut *tx).await?;
    let after = fingerprint(&capture(&mut tx, &entry).await?)?;
    let before_json = serde_json::to_string(&before).map_err(anyhow::Error::from)?;
    let changed = moving.passages.len() as i64;
    sqlx::query!("INSERT INTO speaker_correction_history(id,entry_id,before_json,after_snapshot,revision,changed,summary_refresh,created_at) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(entry_id) DO UPDATE SET id=excluded.id,before_json=excluded.before_json,after_snapshot=excluded.after_snapshot,revision=excluded.revision,changed=excluded.changed,summary_refresh=excluded.summary_refresh,created_at=excluded.created_at", proposal.id, entry, before_json, after, revision, changed, refresh_summary, timestamp).execute(&mut *tx).await?;
    sqlx::query!(
        "DELETE FROM speaker_correction_reviews WHERE entry_id=?",
        entry
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Err(error) = pipeline::refresh_summary_passage(&app, &entry).await {
        tracing::warn!(%entry, %error, "could not index the summary");
    }
    if refresh_summary {
        app.wake.notify_one();
    }
    Ok(Json(
        serde_json::json!({"changed":changed,"undo_id":proposal.id,"summary_refresh":refresh_summary}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    fn row(id: &str, label: &str, start: i64) -> Turn {
        Turn {
            id: id.into(),
            speaker: label.into(),
            start_ms: start,
            end_ms: start + 5000,
            text: id.into(),
            confidence: Some(0.95),
        }
    }
    fn synthetic() -> (Vec<Turn>, BTreeMap<String, Vec<f32>>) {
        let rows = (0..4)
            .map(|i| row(&format!("r{i}"), "A", i * 6000))
            .collect();
        let vectors = (0..4)
            .map(|i| {
                (
                    format!("r{i}"),
                    if i % 2 == 0 {
                        vec![1., 0., 0.]
                    } else {
                        vec![0., 1., 0.]
                    },
                )
            })
            .collect();
        (rows, vectors)
    }
    fn owner() -> Extension<Access> {
        Extension(Access {
            scope: "admin".into(),
        })
    }
    #[test]
    fn the_rejected_passage_identifies_the_voice_to_separate_with_an_uncertain_fallback() {
        let (rows, vectors) = synthetic();
        let groups = split_voices(&rows, &vectors).unwrap();
        assert_eq!(retained_voice(&groups, Some("r0")), Some(1));
        assert_eq!(retained_voice(&groups, Some("r1")), Some(0));
        assert_eq!(retained_voice(&groups, Some("uncertain")), None);
        assert_eq!(retained_voice(&groups, None), None);
    }
    #[test]
    fn separates_mixed_voices_without_either_reference_and_keeps_order_stable() {
        let (rows, vectors) = synthetic();
        let groups = split_voices(&rows, &vectors).unwrap();
        assert_eq!(
            groups[0]
                .passages
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["r0", "r2"]
        );
        assert_eq!(
            groups[1]
                .passages
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["r1", "r3"]
        );
        assert_eq!((groups[0].id, groups[1].id), (0, 1));
        assert_eq!(groups[0].example.id, "r0");
        assert_eq!(groups[1].example.id, "r1");
    }
    #[test]
    fn voice_examples_are_short_without_trimming_the_reviewed_passages() {
        let (mut rows, vectors) = synthetic();
        rows[0].end_ms = 60_000;
        let groups = split_voices(&rows, &vectors).unwrap();
        assert_eq!(groups[0].example.end_ms, 10_000);
        assert_eq!(groups[0].passages[0].end_ms, 60_000);
    }
    #[test]
    fn never_forces_a_single_voice_to_split_or_uses_isolated_noise_as_a_voice() {
        let (mut rows, mut vectors) = synthetic();
        for v in vectors.values_mut() {
            *v = vec![1., 0., 0.];
        }
        assert!(split_voices(&rows, &vectors).is_none());
        rows.push(row("noise", "A", 30000));
        vectors.insert("noise".into(), vec![0., 0., 1.]);
        assert!(split_voices(&rows, &vectors).is_none());
        assert!(split_voices(&rows[..1], &vectors).is_none());
    }
    #[test]
    fn uncertain_missing_and_third_voice_passages_are_left_out() {
        let (mut rows, mut vectors) = synthetic();
        for (id, vector) in [
            ("ambiguous", vec![0.707, 0.707, 0.]),
            ("noise", vec![0., 0., 1.]),
            ("invalid", vec![0., 0., 0.]),
        ] {
            rows.push(row(id, "A", rows.len() as i64 * 6000));
            vectors.insert(id.into(), vector);
        }
        rows.push(row("missing", "A", 60000));
        let groups = split_voices(&rows, &vectors).unwrap();
        assert_eq!(groups.iter().map(|g| g.passages.len()).sum::<usize>(), 4);
        assert_eq!(groups[0].passages[0].id, "r0");
        let mut overlap = rows.clone();
        overlap.push(row("overlap", "B", 2000));
        assert!(!clean(&overlap[0], &overlap));
    }

    async fn fixture() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::from_env().unwrap();
        config.data = dir.path().into();
        config.voice_model = None;
        config.openai_key.clear();
        let app = App::new(config).await.unwrap();
        for entry in ["e", "other"] {
            sqlx::query("INSERT INTO entries(id,fingerprint,title,duration_ms,timezone,stage,summary,created_at,updated_at) VALUES(?,?,?,60000,'UTC','complete','Old attribution','now','now')").bind(entry).bind(entry).bind(entry).execute(&app.db).await.unwrap();
        }
        sqlx::query(
            "INSERT INTO people(id,name) VALUES('p','Original person'),('q','Other person')",
        )
        .execute(&app.db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO speakers(entry_id,label,person_id,state) VALUES('e','A','p','manual'),('e','B','q','manual'),('other','A','p','manual')").execute(&app.db).await.unwrap();
        let (mut rows, vectors) = synthetic();
        let groups = split_voices(&rows, &vectors).unwrap();
        rows.push(row("uncertain", "A", 30000));
        for (position, r) in rows.iter().enumerate() {
            sqlx::query("INSERT INTO segments(id,entry_id,position,speaker,start_ms,end_ms,text) VALUES(?,'e',?,'A',?,?,?)").bind(&r.id).bind(position as i64).bind(r.start_ms).bind(r.end_ms).bind(&r.text).execute(&app.db).await.unwrap();
        }
        for entry in ["e", "other"] {
            sqlx::query("INSERT INTO voice_probes(entry_id,speaker,model,vector_json) VALUES(?,'A','test','[1,0]')").bind(entry).execute(&app.db).await.unwrap();
            sqlx::query("INSERT INTO voice_samples(id,person_id,entry_id,speaker,model,vector_json,provenance,created_at) VALUES(?,'p',?,'A','test','[1,0]','manual','now')").bind(entry).bind(entry).execute(&app.db).await.unwrap();
        }
        let proposal = Proposal {
            id: "review".into(),
            source_label: "A".into(),
            source_name: "Original person".into(),
            groups,
            unchanged: 1,
            suggested_keep: None,
        };
        let mut tx = app.db.begin().await.unwrap();
        let (_, _, _, hash) = snapshot(&mut tx, "e").await.unwrap();
        sqlx::query("INSERT INTO speaker_correction_reviews(id,entry_id,snapshot,proposal,expires_at) VALUES('review','e',?,?,unixepoch()+3600)").bind(hash).bind(serde_json::to_string(&proposal).unwrap()).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        (dir, app)
    }
    fn choice(keep_group: usize) -> Apply {
        Apply {
            keep_group,
            target_label: None,
            person_id: None,
            new_name: None,
        }
    }
    async fn accept(app: &App, request: Apply) -> Result<Json<serde_json::Value>> {
        apply(
            State(app.clone()),
            owner(),
            Path(("e".into(), "review".into())),
            Json(request),
        )
        .await
    }
    async fn labels(app: &App) -> Vec<String> {
        sqlx::query_scalar("SELECT speaker FROM segments WHERE entry_id='e' ORDER BY position")
            .fetch_all(&app.db)
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn a_draft_is_inert_and_both_orientations_apply_the_whole_group_atomically() {
        for keep in 0..2 {
            let (_dir, app) = fixture().await;
            assert_eq!(labels(&app).await, ["A", "A", "A", "A", "A"]);
            let result = accept(&app, choice(keep)).await.unwrap();
            assert_eq!(result.0["changed"], 2);
            let after = labels(&app).await;
            for (i, label) in after[..4].iter().enumerate() {
                assert_eq!(label == "A", i % 2 == keep);
            }
            assert_eq!(after[4], "A");
            let locked: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM speaker_corrections WHERE entry_id='e'")
                    .fetch_one(&app.db)
                    .await
                    .unwrap();
            assert_eq!(locked, 4);
            let samples: Vec<String> = sqlx::query_scalar("SELECT entry_id FROM voice_samples")
                .fetch_all(&app.db)
                .await
                .unwrap();
            assert_eq!(samples, ["other"]);
            let probes: Vec<String> = sqlx::query_scalar("SELECT entry_id FROM voice_probes")
                .fetch_all(&app.db)
                .await
                .unwrap();
            assert_eq!(probes, ["other"]);
            let summaries: Vec<Option<String>> =
                sqlx::query_scalar("SELECT summary FROM entries ORDER BY id")
                    .fetch_all(&app.db)
                    .await
                    .unwrap();
            assert_eq!(summaries, [None, Some("Old attribution".into())]);
            assert_eq!(accept(&app, choice(keep)).await.unwrap().0["changed"], 2);
        }
    }
    #[tokio::test]
    async fn optional_names_support_existing_speakers_people_and_new_people() {
        for kind in ["speaker", "person", "new"] {
            let (_dir, app) = fixture().await;
            let mut request = choice(0);
            match kind {
                "speaker" => request.target_label = Some("B".into()),
                "person" => request.person_id = Some("q".into()),
                _ => request.new_name = Some("New person".into()),
            }
            assert!(accept(&app, request).await.is_ok());
            let person: String = sqlx::query_scalar("SELECT p.name FROM segments s JOIN speakers sp ON sp.entry_id=s.entry_id AND sp.label=s.speaker JOIN people p ON p.id=sp.person_id WHERE s.id='r1'").fetch_one(&app.db).await.unwrap();
            assert_eq!(
                person,
                if kind == "new" {
                    "New person"
                } else {
                    "Other person"
                }
            );
        }
    }
    #[tokio::test]
    async fn invalid_choices_leave_the_recording_and_names_untouched() {
        let (_dir, app) = fixture().await;
        let mut requests = vec![choice(2), choice(usize::MAX)];
        let mut same = choice(0);
        same.target_label = Some("A".into());
        requests.push(same);
        let mut same = choice(0);
        same.person_id = Some("p".into());
        requests.push(same);
        let mut blank = choice(0);
        blank.new_name = Some("  ".into());
        requests.push(blank);
        let mut both = choice(0);
        both.person_id = Some("q".into());
        both.new_name = Some("New".into());
        requests.push(both);
        for request in requests {
            assert_eq!(
                accept(&app, request).await.unwrap_err().0,
                StatusCode::BAD_REQUEST
            );
            assert_eq!(labels(&app).await, ["A", "A", "A", "A", "A"]);
            let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM people")
                .fetch_one(&app.db)
                .await
                .unwrap();
            assert_eq!(n, 2);
        }
    }
    #[tokio::test]
    async fn stale_expired_and_processing_reviews_cannot_be_applied() {
        for mutation in [
            "UPDATE segments SET text='Changed' WHERE id='r1'",
            "UPDATE speakers SET person_id=NULL WHERE entry_id='e' AND label='A'",
            "UPDATE speaker_correction_reviews SET expires_at=0",
            "UPDATE entries SET stage='recognizing' WHERE id='e'",
            "UPDATE entries SET trashed_at='now' WHERE id='e'",
        ] {
            let (_dir, app) = fixture().await;
            sqlx::query(mutation).execute(&app.db).await.unwrap();
            let status = accept(&app, choice(0)).await.unwrap_err().0;
            assert!(status == StatusCode::CONFLICT || status == StatusCode::NOT_FOUND);
            assert_eq!(labels(&app).await, ["A", "A", "A", "A", "A"]);
        }
    }
    #[tokio::test]
    async fn voice_splits_require_owner_access_and_analysis_failures_leave_no_changes() {
        let (_dir, app) = fixture().await;
        for scope in ["read", "upload"] {
            let access = || {
                Extension(Access {
                    scope: scope.into(),
                })
            };
            assert_eq!(
                preview(
                    State(app.clone()),
                    access(),
                    Path("e".into()),
                    Json(Request {
                        speaker: "A".into(),
                        not_segment_id: None,
                    })
                )
                .await
                .unwrap_err()
                .0,
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                apply(
                    State(app.clone()),
                    access(),
                    Path(("e".into(), "review".into())),
                    Json(choice(0))
                )
                .await
                .unwrap_err()
                .0,
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            preview(
                State(app.clone()),
                owner(),
                Path("e".into()),
                Json(Request {
                    speaker: "A".into(),
                    not_segment_id: None,
                })
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(labels(&app).await, ["A", "A", "A", "A", "A"]);
    }
    async fn restore(app: &App) -> Result<Json<serde_json::Value>> {
        undo(State(app.clone()), owner(), Path("e".into())).await
    }
    async fn available(app: &App) -> bool {
        undo_status(State(app.clone()), owner(), Path("e".into()))
            .await
            .unwrap()
            .0["available"]
            == true
    }
    #[tokio::test]
    async fn undo_restores_the_previous_recording_state_and_survives_reload() {
        let (_dir, app) = fixture().await;
        let mut db = app.db.acquire().await.unwrap();
        let original = capture(&mut db, "e").await.unwrap();
        let original_hash = fingerprint(&original).unwrap();
        drop(db);
        assert!(accept(&app, choice(0)).await.is_ok());
        assert!(available(&app).await);
        let reopened = App::new((*app.config).clone()).await.unwrap();
        assert!(available(&reopened).await);
        assert_eq!(restore(&reopened).await.unwrap().0["restored"], 2);
        let mut db = reopened.db.acquire().await.unwrap();
        let restored = capture(&mut db, "e").await.unwrap();
        assert_eq!(fingerprint(&restored).unwrap(), original_hash);
        assert_eq!(restored.notes.summary, original.notes.summary);
        assert_eq!(restored.notes.title, original.notes.title);
        assert!(restored.notes.revision > original.notes.revision);
        drop(db);
        assert!(!available(&app).await);
        assert_eq!(restore(&app).await.unwrap_err().0, StatusCode::CONFLICT);
    }
    #[tokio::test]
    async fn undo_refuses_to_overwrite_subsequent_edits() {
        for edit in [
            "UPDATE segments SET text='Later edit' WHERE id='r0'",
            "UPDATE speakers SET person_id='q' WHERE entry_id='e' AND label='A'",
            "UPDATE entries SET title='Manual title',title_manual=1 WHERE id='e'",
            "UPDATE entries SET notes_revision=notes_revision+1 WHERE id='e'",
        ] {
            let (_dir, app) = fixture().await;
            let _ = accept(&app, choice(0)).await.unwrap();
            sqlx::query(edit).execute(&app.db).await.unwrap();
            let before = labels(&app).await;
            assert!(!available(&app).await);
            assert_eq!(restore(&app).await.unwrap_err().0, StatusCode::CONFLICT);
            assert_eq!(labels(&app).await, before);
        }
    }
    #[tokio::test]
    async fn undo_requires_owner_access() {
        let (_dir, app) = fixture().await;
        let _ = accept(&app, choice(0)).await.unwrap();
        for scope in ["read", "upload"] {
            let access = || {
                Extension(Access {
                    scope: scope.into(),
                })
            };
            assert_eq!(
                undo(State(app.clone()), access(), Path("e".into()))
                    .await
                    .unwrap_err()
                    .0,
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                undo_status(State(app.clone()), access(), Path("e".into()))
                    .await
                    .unwrap_err()
                    .0,
                StatusCode::FORBIDDEN
            );
        }
        assert!(available(&app).await);
    }
    async fn notes_mock(
        app: &mut App,
    ) -> (
        std::sync::Arc<tokio::sync::Notify>,
        std::sync::Arc<tokio::sync::Notify>,
        tokio::task::JoinHandle<()>,
        std::sync::Arc<std::sync::Mutex<serde_json::Value>>,
    ) {
        use std::sync::{Arc, Mutex};
        use tokio::sync::Notify;
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let body = Arc::new(Mutex::new(serde_json::Value::Null));
        let (e, r, captured) = (entered.clone(), release.clone(), body.clone());
        let router = axum::Router::new().route("/chat/completions", axum::routing::post(move |Json(input): Json<serde_json::Value>| {
            let (e, r, captured) = (e.clone(), r.clone(), captured.clone());
            async move {
                *captured.lock().unwrap() = input;
                e.notify_one(); r.notified().await;
                let notes = serde_json::json!({"language":"English","title":"Updated title","summary":"Fresh summary after correction","names":[]});
                Json(serde_json::json!({"choices":[{"message":{"content":notes.to_string()}}]}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = Arc::make_mut(&mut app.config);
        config.openai_key = "synthetic".into();
        config.openai_url = format!("http://{}", listener.local_addr().unwrap());
        tokio::fs::create_dir_all(app.entry_dir("e")).await.unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (entered, release, server, body)
    }
    #[tokio::test]
    async fn applying_a_correction_automatically_refreshes_notes_and_undo_remains_available() {
        let (_dir, mut app) = fixture().await;
        let (entered, release, server, body) = notes_mock(&mut app).await;
        let receipt = accept(&app, choice(0)).await.unwrap().0;
        assert_eq!(receipt["summary_refresh"], true);
        let stop = tokio_util::sync::CancellationToken::new();
        let worker = tokio::spawn(crate::pipeline::worker(app.clone(), stop.clone()));
        tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        assert!(body.lock().unwrap().to_string().contains("corrected-"));
        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let state: (String, Option<String>) =
                    sqlx::query_as("SELECT stage,summary FROM entries WHERE id='e'")
                        .fetch_one(&app.db)
                        .await
                        .unwrap();
                if state
                    == (
                        "complete".into(),
                        Some("Fresh summary after correction".into()),
                    )
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        stop.cancel();
        worker.await.unwrap();
        assert!(available(&app).await);
        let _ = restore(&app).await.unwrap();
        let notes: (String, Option<String>) =
            sqlx::query_as("SELECT title,summary FROM entries WHERE id='e'")
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert_eq!(notes, ("e".into(), Some("Old attribution".into())));
        server.abort();
    }
    #[tokio::test]
    async fn undo_during_an_inflight_summary_cannot_be_overwritten_by_its_late_response() {
        let (_dir, mut app) = fixture().await;
        let (entered, release, server, _) = notes_mock(&mut app).await;
        let _ = accept(&app, choice(0)).await.unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        let worker = tokio::spawn(crate::pipeline::worker(app.clone(), stop.clone()));
        tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        assert!(available(&app).await);
        let _ = restore(&app).await.unwrap();
        release.notify_one();
        stop.cancel();
        worker.await.unwrap();
        assert_eq!(labels(&app).await, ["A", "A", "A", "A", "A"]);
        let state: (String, String, Option<String>) =
            sqlx::query_as("SELECT stage,title,summary FROM entries WHERE id='e'")
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert_eq!(
            state,
            (
                "complete".into(),
                "e".into(),
                Some("Old attribution".into())
            )
        );
        server.abort();
    }
}
