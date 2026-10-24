use crate::audio::Format;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::LazyLock};
static DJI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^([a-z]+[0-9]+)_MIC([0-9]+)_([0-9]{8})_([0-9]{6})((?:_[a-z0-9]+)*)\.wav$")
        .unwrap()
});
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub filename: String,
    pub hash: String,
    pub format: Format,
    pub recorded_at: Option<DateTime<Utc>>,
    pub stream: String,
}
pub fn parse_dji(name: &str, zone: chrono_tz::Tz) -> Option<(DateTime<Utc>, String)> {
    let c = DJI.captures(name)?;
    let local =
        NaiveDateTime::parse_from_str(&format!("{}{}", &c[3], &c[4]), "%Y%m%d%H%M%S").ok()?;
    // Ambiguous DST timestamps must not be silently guessed.
    let start = zone
        .from_local_datetime(&local)
        .single()?
        .with_timezone(&Utc);
    Some((
        start,
        format!("{}|{}", c[1].to_uppercase(), c[5].to_lowercase()),
    ))
}
pub fn group(sources: Vec<Source>) -> Vec<Vec<Source>> {
    let mut streams: BTreeMap<String, Vec<Source>> = BTreeMap::new();
    for s in sources {
        streams
            .entry(if s.recorded_at.is_some() {
                s.stream.clone()
            } else {
                s.id.clone()
            })
            .or_default()
            .push(s);
    }
    let mut groups: Vec<Vec<Source>> = vec![];
    for mut stream in streams.into_values() {
        stream.sort_by(|a, b| {
            a.recorded_at
                .cmp(&b.recorded_at)
                .then(a.filename.cmp(&b.filename))
        });
        let mut current: Vec<Source> = vec![];
        for source in stream {
            let join = current.last().is_some_and(|prev| {
                prev.format.sample_rate == source.format.sample_rate
                    && prev.format.channels == source.format.channels
                    && prev.format.bit_depth == source.format.bit_depth
                    && prev.format.is_float == source.format.is_float
                    && prev.format.duration_ms() >= 1_785_000
                    && match (prev.recorded_at, source.recorded_at) {
                        (Some(a), Some(b)) => {
                            ((b - a).num_milliseconds() - prev.format.duration_ms()).abs() <= 2000
                        }
                        _ => false,
                    }
            });
            if !join && !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
            current.push(source);
        }
        if !current.is_empty() {
            groups.push(current);
        }
    }
    groups.sort_by_key(|g| g[0].recorded_at);
    groups
}
#[cfg(test)]
mod tests {
    use super::*;
    fn source(name: &str, ms: i64) -> Source {
        let (t, stream) = parse_dji(name, chrono_tz::UTC).unwrap();
        Source {
            id: name.into(),
            filename: name.into(),
            hash: name.into(),
            format: Format {
                sample_rate: 48000,
                channels: 1,
                bit_depth: 24,
                is_float: false,
                frames: (ms * 48) as u64,
            },
            recorded_at: Some(t),
            stream,
        }
    }
    #[test]
    fn rollover_and_stop_restart() {
        let a = source("TX01_MIC001_20261001_235000_orig.wav", 1800150);
        let b = source("TX01_MIC002_20261002_002000_orig.wav", 60000);
        let c = source("TX01_MIC003_20261002_002100_orig.wav", 60000);
        let groups = group(vec![c, b, a]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 2);
    }
    #[test]
    fn variants_and_formats_stay_separate() {
        let a = source("TX01_MIC001_20261001_120000_orig.wav", 1800000);
        let b = source("TX01_MIC002_20261001_123000_edit.wav", 1800000);
        assert_eq!(group(vec![a, b]).len(), 2);
    }
    #[test]
    fn invalid_dates_and_ambiguous_times_are_not_guessed() {
        assert!(parse_dji("TX01_MIC001_20260230_120000.wav", chrono_tz::UTC).is_none());
        assert!(parse_dji("TX01_MIC001_20261025_023000.wav", chrono_tz::Europe::Berlin).is_none());
    }
}

/// Native clients send their visible session boundaries, including manual edits.
/// Source IDs are opaque import-local identifiers, never filesystem paths.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPlan {
    pub source_ids: Vec<String>,
    pub trim_start_frame: Option<u64>,
    pub trim_end_frame: Option<u64>,
}

pub fn validate_source_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

pub fn validate_plan(plans: &[SessionPlan]) -> anyhow::Result<()> {
    use anyhow::ensure;
    use std::collections::HashSet;
    ensure!(
        !plans.is_empty() && plans.len() <= 1000,
        "Select between 1 and 1000 sessions"
    );
    let mut seen = HashSet::new();
    for plan in plans {
        ensure!(
            !plan.source_ids.is_empty(),
            "A session must contain at least one source"
        );
        for id in &plan.source_ids {
            ensure!(
                validate_source_id(id),
                "Source IDs must be nonempty and at most 256 characters"
            );
            ensure!(
                seen.insert(id),
                "A source can appear only once in an import"
            );
        }
        ensure!(
            seen.len() <= 10000,
            "An import can contain at most 10000 sources"
        );
        if let Some(end) = plan.trim_end_frame {
            ensure!(
                end > plan.trim_start_frame.unwrap_or(0),
                "Trim end must follow trim start"
            );
        }
    }
    Ok(())
}

pub struct PlannedSession {
    pub sources: Vec<Source>,
    pub start_frame: u64,
    pub end_frame: u64,
    pub total_frames: u64,
}
impl PlannedSession {
    pub fn is_trimmed(&self) -> bool {
        self.start_frame != 0 || self.end_frame != self.total_frames
    }
    pub fn duration_ms(&self) -> i64 {
        let rate = u64::from(self.sources[0].format.sample_rate);
        ((self.end_frame - self.start_frame) * 1000).div_ceil(rate) as i64
    }
    pub fn recorded_at(&self) -> Option<DateTime<Utc>> {
        let mut offset = 0;
        for source in &self.sources {
            if self.start_frame < offset + source.format.frames {
                let millis = ((self.start_frame - offset) as f64
                    / f64::from(source.format.sample_rate)
                    * 1000.)
                    .round() as i64;
                return source
                    .recorded_at
                    .map(|date| date + chrono::Duration::milliseconds(millis));
            }
            offset += source.format.frames;
        }
        None
    }
    pub fn fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut material = self
            .sources
            .iter()
            .map(|source| source.hash.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // Preserve existing whole-session deduplication across old and native clients.
        if self.is_trimmed() {
            material.push_str(&format!(
                "\nvault-trim-v1:{}:{}",
                self.start_frame, self.end_frame
            ));
        }
        hex::encode(Sha256::digest(material))
    }
}

pub fn planned_sessions(
    sources: Vec<(String, Source)>,
    plans: &[SessionPlan],
) -> anyhow::Result<Vec<PlannedSession>> {
    use anyhow::{Context, ensure};
    use std::collections::HashMap;
    validate_plan(plans)?;
    let mut available: HashMap<String, Source> = sources.into_iter().collect();
    let mut result = Vec::new();
    for plan in plans {
        let mut members = Vec::new();
        for id in &plan.source_ids {
            members.push(
                available
                    .remove(id)
                    .context("A planned source is missing or failed verification")?,
            );
        }
        let first = &members[0];
        let mut total = 0u64;
        for source in &members {
            ensure!(
                source.format.sample_rate == first.format.sample_rate
                    && source.format.channels == first.format.channels
                    && source.format.bit_depth == first.format.bit_depth
                    && source.format.is_float == first.format.is_float,
                "A session's source audio formats must match"
            );
            if source.recorded_at.is_some() && first.recorded_at.is_some() {
                ensure!(
                    source.stream == first.stream,
                    "A session cannot merge different microphones or renditions"
                );
            }
            total = total
                .checked_add(source.format.frames)
                .context("Session is too long")?;
        }
        let start = plan.trim_start_frame.unwrap_or(0);
        let end = plan.trim_end_frame.unwrap_or(total);
        ensure!(
            start < end && end <= total && end <= i64::MAX as u64,
            "Trim must be inside the session's verified audio frames"
        );
        result.push(PlannedSession {
            sources: members,
            start_frame: start,
            end_frame: end,
            total_frames: total,
        });
    }
    ensure!(
        available.is_empty(),
        "Every uploaded source must belong to a planned session"
    );
    Ok(result)
}
