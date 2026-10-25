use crate::{
    config::App,
    error::{Error, Result},
    voice,
};
use axum::{Extension, Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceThresholds {
    pub strong: f32,
    pub margin: f32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub recording_context: String,
    pub voice_thresholds: VoiceThresholds,
}
impl Default for Settings {
    fn default() -> Self {
        let strong = std::env::var("VAULT_VOICE_STRONG")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
            .unwrap_or(0.82);
        Self {
            recording_context: String::new(),
            voice_thresholds: VoiceThresholds {
                strong,
                margin: 0.10,
            },
        }
    }
}
pub async fn load(app: &App) -> anyhow::Result<Settings> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT value_json FROM app_settings WHERE key='processing'")
            .fetch_optional(&app.db)
            .await?;
    Ok(if let Some(stored) = stored {
        serde_json::from_str(&stored)?
    } else {
        Settings::default()
    })
}
fn view(app: &App, settings: &Settings) -> Value {
    let c = &app.config;
    let configured = |ready: bool, name: String| if ready { name } else { "Not configured".into() };
    json!({
        "recording_context": settings.recording_context,
        "voice_thresholds": settings.voice_thresholds,
        "models": {
            "transcription": format!("AssemblyAI {}", c.transcription_models.join(", ")),
            "text": configured(!c.groq_key.is_empty(), format!("Groq {}", c.text_model)),
            "embedding": configured(!c.openai_key.is_empty(), format!("OpenAI {}", c.embedding_model)),
            "voice": configured(c.voice_model.is_some(), format!("Local {}", voice::MODEL)),
        },
        "voice_configured": c.voice_model.is_some(),
        "recorder_timezone": c.recorder_timezone,
    })
}
pub async fn get(
    State(app): State<App>,
    Extension(access): Extension<crate::auth::Access>,
) -> Result<Json<Value>> {
    access.admin()?;
    Ok(Json(view(&app, &load(&app).await?)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    recording_context: Option<String>,
    voice_thresholds: Option<VoiceThresholds>,
}
pub async fn update(
    State(app): State<App>,
    Extension(access): Extension<crate::auth::Access>,
    Json(update): Json<Update>,
) -> Result<Json<Value>> {
    access.admin()?;
    let _lock = app.mutations.lock().await;
    let mut settings = load(&app).await?;
    if let Some(context) = update.recording_context {
        if context.chars().count() > 8000 || context.contains('\0') {
            return Err(Error::bad(
                "Recording context must be at most 8000 characters without null bytes",
            ));
        }
        settings.recording_context = context;
    }
    if let Some(thresholds) = update.voice_thresholds {
        if !thresholds.strong.is_finite()
            || !thresholds.margin.is_finite()
            || !(0.0..=1.0).contains(&thresholds.strong)
            || !(0.0..=1.0).contains(&thresholds.margin)
        {
            return Err(Error::bad("Voice thresholds must be between 0 and 1"));
        }
        settings.voice_thresholds = thresholds;
    }
    sqlx::query("INSERT INTO app_settings(key,value_json)VALUES('processing',?) ON CONFLICT(key)DO UPDATE SET value_json=excluded.value_json").bind(serde_json::to_string(&settings).unwrap()).execute(&app.db).await?;
    Ok(Json(view(&app, &settings)))
}
