//! Who each diarized speaker is, from the owner's decisions, voice similarity and the speakers' own
//! introductions; ported from mic-ingest's IdentityResolver. A wrong name is much worse than none,
//! so anything short of clear evidence stays unidentified.
use crate::{
    config::{App, id, now},
    provider::NameFinding,
    settings::VoiceThresholds,
    voice::{self, Analysis},
};
use anyhow::Result;
use serde_json::json;
use std::collections::{BTreeMap, HashSet};

/// A validated self-introduction and the known person of that name.
#[derive(Clone, Copy)]
pub struct Intro<'a> {
    pub name: &'a str,
    /// The one known person of that name; None if there is none, or several.
    pub person: Option<&'a str>,
    pub ambiguous: bool,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Unidentified,
    Person(String),
    /// Introduced themselves with a name nobody has yet: a new person.
    NewPerson(String),
}

#[derive(Debug, PartialEq)]
pub struct Resolution {
    pub outcome: Outcome,
    pub voice_applied: bool,
    pub intro_applied: bool,
    /// Why this voice may join the person's profile, if it may.
    pub enrollment: Option<&'static str>,
}

/// Fewer excerpts than this are enough to compare a voice, not to add a voice match to a profile.
const ENROLL_MIN_EXCERPTS: usize = 4;

/// Decides one undecided speaker. `candidates` are known people closest first; `has_profile` says
/// whether a person already has a voice profile.
pub fn resolve(
    candidates: &[(String, f32)],
    excerpts: usize,
    intro: Option<Intro>,
    has_profile: impl Fn(&str) -> bool,
    t: &VoiceThresholds,
) -> Resolution {
    let result = |outcome, voice_applied, intro_applied, enrollment| Resolution {
        outcome,
        voice_applied,
        intro_applied,
        enrollment,
    };
    let best = candidates.first();
    let lead = match (best, candidates.get(1)) {
        (Some(best), Some(second)) => best.1 - second.1,
        _ => f32::INFINITY,
    };
    let strong = best.is_some_and(|b| b.1 >= t.strong && lead >= t.margin);
    let borderline = best.is_some_and(|b| b.1 >= t.borderline);
    let agrees = intro
        .and_then(|i| i.person)
        .is_some_and(|p| best.is_some_and(|b| b.0 == p));
    if let Some((person, score)) = best
        && strong
    {
        if intro.is_some() && !agrees {
            // The voice says one person, the speaker says they are someone else. Trust neither.
            return result(Outcome::Unidentified, false, false, None);
        }
        if agrees {
            return result(
                Outcome::Person(person.clone()),
                true,
                true,
                Some("self_introduction"),
            );
        }
        let enroll = *score >= t.enroll && excerpts >= ENROLL_MIN_EXCERPTS;
        return result(
            Outcome::Person(person.clone()),
            true,
            false,
            enroll.then_some("voice_match"),
        );
    }
    // A borderline voice counts only with the speaker's own introduction as the same person.
    if let Some((person, _)) = best
        && borderline
        && agrees
    {
        return result(
            Outcome::Person(person.clone()),
            true,
            true,
            Some("self_introduction"),
        );
    }
    match intro {
        Some(Intro {
            person: Some(person),
            ..
        }) => {
            // A known person whose stored voice did not match: accept the name, keep the audio
            // out of their existing profile.
            let enroll = !has_profile(person);
            result(
                Outcome::Person(person.into()),
                false,
                true,
                enroll.then_some("self_introduction"),
            )
        }
        Some(Intro {
            ambiguous: true, ..
        }) => result(Outcome::Unidentified, false, false, None),
        Some(Intro { name, .. }) => result(
            Outcome::NewPerson(name.into()),
            false,
            true,
            Some("self_introduction"),
        ),
        // Closest voice, a mention, an address: not enough.
        None => result(Outcome::Unidentified, false, false, None),
    }
}

/// Self-introductions that hold: the quote was spoken by that speaker and contains the name, and no
/// other finding names that speaker differently or gives the name to another speaker.
async fn introductions(
    app: &App,
    entry: &str,
    names: &[NameFinding],
) -> Result<BTreeMap<String, (String, String)>> {
    let mut valid = vec![];
    for finding in names {
        let name = finding.name.trim();
        // The model sometimes repeats the transcript's brackets: "[A]" for speaker A.
        let speaker = finding.speaker.trim().trim_matches(['[', ']']);
        if name.is_empty() || name.len() > 120 || finding.quote.is_empty() {
            continue;
        }
        let spoken: Vec<String> =
            sqlx::query_scalar("SELECT text FROM segments WHERE entry_id=? AND speaker=?")
                .bind(entry)
                .bind(speaker)
                .fetch_all(&app.db)
                .await?;
        if spoken.iter().any(|text| text.contains(&finding.quote))
            && finding.quote.to_lowercase().contains(&name.to_lowercase())
        {
            valid.push((speaker.to_string(), name, finding.quote.as_str()));
        }
    }
    let mut found = BTreeMap::new();
    let mut seen = HashSet::new();
    for (speaker, name, quote) in &valid {
        let contradicted = valid.iter().any(|(other, other_name, _)| {
            (other == speaker) != (other_name.to_lowercase() == name.to_lowercase())
        });
        if !contradicted && seen.insert(speaker.clone()) {
            found.insert(speaker.clone(), (name.to_string(), quote.to_string()));
        }
    }
    Ok(found)
}

/// Decides every speaker the owner has not decided, and writes the outcome, its evidence and any
/// voice enrollment. `names` use the transcript's current labels.
pub async fn apply(
    app: &App,
    entry: &str,
    analysis: &Analysis,
    names: &[NameFinding],
) -> Result<()> {
    let thresholds = crate::settings::load(app).await?.voice_thresholds;
    let intros = introductions(app, entry, names).await?;
    let people: Vec<(String, String)> = sqlx::query_as("SELECT id,name FROM people")
        .fetch_all(&app.db)
        .await?;
    let profiles: HashSet<String> = sqlx::query_scalar(
        "SELECT DISTINCT person_id FROM voice_samples WHERE model=? AND entry_id!=?",
    )
    .bind(voice::MODEL)
    .bind(entry)
    .fetch_all(&app.db)
    .await?
    .into_iter()
    .collect();
    let labels: Vec<String> = sqlx::query_scalar(
        "SELECT label FROM speakers WHERE entry_id=? AND state NOT IN ('manual','manual_unknown')",
    )
    .bind(entry)
    .fetch_all(&app.db)
    .await?;
    let key = |name: &str| name.trim().to_lowercase();
    let mut tx = app.db.begin().await?;
    for label in labels {
        let voice = analysis.labels.get(&label);
        let candidates = voice.map_or(&[][..], |v| &v.candidates[..]);
        let intro = intros.get(&label).map(|(name, _)| {
            let named: Vec<_> = people.iter().filter(|(_, n)| key(n) == key(name)).collect();
            Intro {
                name,
                person: (named.len() == 1).then(|| named[0].0.as_str()),
                ambiguous: named.len() > 1,
            }
        });
        let decision = resolve(
            candidates,
            voice.map_or(0, |v| v.excerpts),
            intro,
            |person| profiles.contains(person),
            &thresholds,
        );
        let mut evidence = vec![];
        if let Some((person, score)) = candidates.first()
            && *score >= thresholds.borderline
        {
            evidence.push(json!({"source":"voice","person_id":person,"score":(score * 1000.).round() / 1000.,"model":voice::MODEL,"applied":decision.voice_applied}));
        }
        if let Some((name, quote)) = intros.get(&label) {
            evidence.push(json!({"source":"transcript","name":name,"quote":quote,"applied":decision.intro_applied}));
        }
        let person = match decision.outcome {
            Outcome::Unidentified => None,
            Outcome::Person(person) => Some(person),
            Outcome::NewPerson(name) => {
                let person = id();
                sqlx::query("INSERT INTO people(id,name)VALUES(?,?)")
                    .bind(&person)
                    .bind(name.trim())
                    .execute(&mut *tx)
                    .await?;
                tracing::info!(%entry, %label, "created a person from a self-introduction");
                Some(person)
            }
        };
        sqlx::query(
            "UPDATE speakers SET person_id=?,state=?,evidence=? WHERE entry_id=? AND label=?",
        )
        .bind(&person)
        .bind(if person.is_some() {
            "automatic"
        } else {
            "unknown"
        })
        .bind(serde_json::to_string(&evidence)?)
        .bind(entry)
        .bind(&label)
        .execute(&mut *tx)
        .await?;
        // Automatic samples follow the current decision; the owner's own are kept by `voice::assign`.
        sqlx::query("DELETE FROM voice_samples WHERE entry_id=? AND speaker=?")
            .bind(entry)
            .bind(&label)
            .execute(&mut *tx)
            .await?;
        if let (Some(person), Some(provenance), Some(voice)) = (&person, decision.enrollment, voice)
        {
            sqlx::query("INSERT INTO voice_samples(id,person_id,entry_id,speaker,model,vector_json,provenance,created_at)VALUES(?,?,?,?,?,?,?,?)").bind(id()).bind(person).bind(entry).bind(&label).bind(voice::MODEL).bind(serde_json::to_string(&voice.vector)?).bind(provenance).bind(now()).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM voice_samples WHERE person_id=? AND model=? AND id NOT IN(SELECT id FROM voice_samples WHERE person_id=? AND model=? ORDER BY created_at DESC LIMIT 24)").bind(person).bind(voice::MODEL).bind(person).bind(voice::MODEL).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: VoiceThresholds = VoiceThresholds {
        strong: 0.78,
        margin: 0.10,
        borderline: 0.50,
        enroll: 0.82,
    };
    fn c(list: &[(&str, f32)]) -> Vec<(String, f32)> {
        list.iter().map(|(p, s)| (p.to_string(), *s)).collect()
    }
    fn intro(name: &str, person: Option<&'static str>) -> Option<Intro<'static>> {
        Some(Intro {
            name: Box::leak(name.to_string().into_boxed_str()),
            person,
            ambiguous: false,
        })
    }
    #[test]
    fn a_strong_clear_voice_names_the_speaker_and_enrolls_only_when_very_close() {
        let r = resolve(&c(&[("lena", 0.80), ("max", 0.40)]), 8, None, |_| true, &T);
        assert_eq!(r.outcome, Outcome::Person("lena".into()));
        assert_eq!(r.enrollment, None);
        let r = resolve(&c(&[("lena", 0.86)]), 8, None, |_| true, &T);
        assert_eq!(r.enrollment, Some("voice_match"));
        // Too little speech to grow a profile from.
        let r = resolve(&c(&[("lena", 0.86)]), 2, None, |_| true, &T);
        assert_eq!(r.enrollment, None);
    }
    #[test]
    fn two_close_people_or_a_borderline_voice_alone_name_nobody() {
        let r = resolve(&c(&[("lena", 0.85), ("lea", 0.80)]), 8, None, |_| true, &T);
        assert_eq!(r.outcome, Outcome::Unidentified);
        let r = resolve(&c(&[("lena", 0.65)]), 8, None, |_| true, &T);
        assert_eq!(r.outcome, Outcome::Unidentified);
    }
    #[test]
    fn an_introduction_confirms_a_borderline_voice_and_contradicts_a_strong_one() {
        let r = resolve(
            &c(&[("lena", 0.65)]),
            8,
            intro("Lena", Some("lena")),
            |_| true,
            &T,
        );
        assert_eq!(r.outcome, Outcome::Person("lena".into()));
        assert!(r.voice_applied && r.intro_applied);
        let r = resolve(
            &c(&[("lena", 0.90)]),
            8,
            intro("Max", Some("max")),
            |_| true,
            &T,
        );
        assert_eq!(r.outcome, Outcome::Unidentified);
    }
    #[test]
    fn an_introduction_alone_names_a_known_or_new_person() {
        let r = resolve(&[], 0, intro("Max", Some("max")), |_| true, &T);
        assert_eq!(r.outcome, Outcome::Person("max".into()));
        // Their existing profile did not match this voice: it is not added.
        assert_eq!(r.enrollment, None);
        let r = resolve(&[], 4, intro("Max", Some("max")), |_| false, &T);
        assert_eq!(r.enrollment, Some("self_introduction"));
        let r = resolve(&[], 4, intro("Jo", None), |_| false, &T);
        assert_eq!(r.outcome, Outcome::NewPerson("Jo".into()));
        let ambiguous = Some(Intro {
            name: "Sam",
            person: None,
            ambiguous: true,
        });
        assert_eq!(
            resolve(&[], 4, ambiguous, |_| false, &T).outcome,
            Outcome::Unidentified
        );
    }
}
