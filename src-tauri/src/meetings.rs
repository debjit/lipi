use crate::audio::MeetingAudio;
use crate::db::{ActionItem, Database, MeetingDetail, MeetingSession};
use crate::engine::ModelSupervisor;
use crate::languages;
use chrono::{DateTime, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

#[derive(Serialize, Clone)]
struct SegmentEvent {
    session_id: i64,
    start_ms: u64,
    end_ms: u64,
    text: String,
}

struct MeetingDraft {
    title: String,
    summary: String,
    decisions: Vec<String>,
    actions: Vec<(String, Option<String>)>,
}

fn model_id(settings: &crate::db::AppSettings) -> &str {
    if settings.engine_mode == "local" {
        settings.local_model_size.as_str()
    } else {
        settings.model.as_str()
    }
}

pub fn parse_due(raw: &str) -> Option<(String, i64)> {
    let s = raw.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("null") || s.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        let utc = dt.with_timezone(&Utc);
        return Some((utc.to_rfc3339(), utc.timestamp()));
    }
    let normalized = s.replace(' ', "T");
    if let Ok(dt) = NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M:%S") {
        return local_due(dt);
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M") {
        return local_due(dt);
    }
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = date.and_hms_opt(9, 0, 0)?;
        return local_due(dt);
    }
    None
}

fn local_due(dt: NaiveDateTime) -> Option<(String, i64)> {
    let local = match Local.from_local_datetime(&dt) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(dt, _) => dt,
        LocalResult::None => return None,
    };
    Some((local.to_rfc3339(), local.timestamp()))
}

fn parse_meeting_notes(raw: &str) -> MeetingDraft {
    let cleaned = strip_fences(raw);
    let json = json_object(&cleaned);
    let Some(value) = json.and_then(|text| serde_json::from_str::<Value>(&text).ok()) else {
        return MeetingDraft {
            title: String::new(),
            summary: cleaned,
            decisions: Vec::new(),
            actions: Vec::new(),
        };
    };
    let title = value.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let summary = value.get("summary").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let decisions = string_list(value.get("decisions"));
    let mut actions = Vec::new();
    if let Some(items) = value.get("action_items").and_then(|v| v.as_array()) {
        for item in items {
            if let Some(text) = item.as_str() {
                let text = text.trim();
                if !text.is_empty() {
                    actions.push((text.to_string(), None));
                }
                continue;
            }
            let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if text.is_empty() {
                continue;
            }
            let due = item
                .get("due")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            actions.push((text, due));
        }
    }
    MeetingDraft {
        title,
        summary,
        decisions,
        actions,
    }
}

fn stored_decisions(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect(),
        Some(Value::String(text)) => {
            let text = text.trim();
            if text.is_empty() {
                Vec::new()
            } else {
                vec![text.to_string()]
            }
        }
        _ => Vec::new(),
    }
}

fn strip_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("```") && trimmed.ends_with("```") {
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() >= 2 {
            return lines[1..lines.len() - 1].join("\n").trim().to_string();
        }
    }
    trimmed.to_string()
}

fn json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(text[start..=end].to_string())
}

fn fmt_ms(ms: i64) -> String {
    let total = (ms.max(0) / 1000) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn clip_transcript(text: &str) -> String {
    const MAX: usize = 16_000;
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= MAX {
        return text.to_string();
    }
    let head: String = chars.iter().take(8_000).collect();
    let tail: String = chars.iter().skip(chars.len() - 8_000).collect();
    format!("{head}\n\n[...]\n\n{tail}")
}

pub fn render_meeting_note(
    summary: &str,
    decisions: &[String],
    actions: &[ActionItem],
    segments: &[crate::db::MeetingSegment],
) -> String {
    let mut out = String::new();
    if !summary.trim().is_empty() {
        out.push_str("## Summary\n\n");
        out.push_str(summary.trim());
        out.push_str("\n\n");
    }
    if !decisions.is_empty() {
        out.push_str("## Decisions\n\n");
        for decision in decisions {
            out.push_str("- ");
            out.push_str(decision.trim());
            out.push('\n');
        }
        out.push('\n');
    }
    if !actions.is_empty() {
        out.push_str("## Action items\n\n");
        for action in actions {
            let mark = if action.done { "x" } else { " " };
            out.push_str(&format!("- [{mark}] {}", action.text.trim()));
            if let Some(due) = action.due_at.as_deref().filter(|d| !d.is_empty()) {
                out.push_str(&format!(" (due {due})"));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str("## Transcript\n\n");
    if segments.is_empty() {
        out.push_str("No speech was captured.\n");
    } else {
        for seg in segments {
            out.push_str(&format!("[{}] {}\n\n", fmt_ms(seg.start_ms), seg.text.trim()));
        }
    }
    out.trim().to_string()
}

fn transcript_for_model(segments: &[crate::db::MeetingSegment]) -> String {
    let mut text = String::new();
    for seg in segments {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("[{}] {}", fmt_ms(seg.start_ms), seg.text.trim()));
    }
    clip_transcript(&text)
}

fn default_title() -> String {
    Local::now().format("Meeting %Y-%m-%d %H:%M").to_string()
}

async fn transcribe_chunk(
    app_data_dir: &Path,
    supervisor: &ModelSupervisor,
    db: &Database,
    wav: Vec<u8>,
    language: Option<&str>,
) -> Result<String, String> {
    let settings = db.get_settings().unwrap_or_default();
    if settings.engine_mode == "local" {
        return supervisor
            .transcribe(
                wav,
                &settings.local_engine,
                &settings.local_model_size,
                language,
                app_data_dir,
                crate::models_folder(&settings),
                None,
            )
            .await;
    }

    let llm_cfg = crate::env_config::load_llm_settings(app_data_dir);
    let (api_base_url, api_key) = crate::env_config::resolve_asr_endpoint(&settings, &llm_cfg);
    if api_base_url.trim().is_empty() {
        return Err("API endpoint URL is missing. Set it in Settings.".into());
    }
    let timeout_secs = llm_cfg.request_timeout_secs.max(settings.request_timeout_secs).max(180) as u64;
    let wav_len = wav.len();
    let duration_secs = if wav_len > 44 { (wav_len - 44) as f64 / 32000.0 } else { 0.0 };
    let result = crate::transcribe::transcribe_audio(
        wav,
        &api_base_url,
        &api_key,
        &settings.model,
        language,
        timeout_secs,
        "chunk.wav",
        "audio/wav",
    )
    .await?;
    if api_base_url.contains("api.cloudflare.com") {
        let _ = db.log_cf_asr_usage(&settings.model, duration_secs);
    }
    Ok(result)
}

fn spawn_meeting_worker(
    app: AppHandle,
    supervisor: Arc<ModelSupervisor>,
    db: Arc<Database>,
    app_data_dir: PathBuf,
    cancel: Arc<AtomicBool>,
    session_id: i64,
    language: Option<String>,
    rx: std::sync::mpsc::Receiver<MeetingAudio>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut full = String::new();
        let lang = language.filter(|c| !c.trim().is_empty());
        while let Ok(event) = rx.recv() {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let (pcm, start_ms, end_ms) = match event {
                MeetingAudio::Chunk { pcm, start_ms, end_ms } => (pcm, start_ms, end_ms),
                MeetingAudio::Limit => {
                    let _ = app.emit("meeting_limit", session_id);
                    continue;
                }
            };
            let wav = match crate::audio::pcm_f32_to_wav(&pcm) {
                Ok(wav) => wav,
                Err(e) => {
                    let _ = app.emit("meeting_error", e);
                    continue;
                }
            };
            let _ = app.emit("meeting_transcribing", true);
            let samples = pcm.len();
            let wav_for_retry = wav.clone();
            let text = tauri::async_runtime::block_on(transcribe_chunk(
                &app_data_dir,
                &supervisor,
                &db,
                wav,
                lang.as_deref(),
            ));
            let _ = app.emit("meeting_transcribing", false);
            match text {
                Ok(text) => {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    let Some(text) = crate::transcribe::accept_asr_text(&text, samples) else {
                        continue;
                    };
                    if !full.is_empty() && full.trim_end().ends_with(text) && text.chars().count() < 40 {
                        continue;
                    }
                    if !full.is_empty() {
                        full.push(' ');
                    }
                    full.push_str(text);
                    match db.insert_segment(session_id, start_ms as i64, end_ms as i64, text) {
                        Ok(_) => {
                            let _ = app.emit(
                                "meeting_segment",
                                SegmentEvent {
                                    session_id,
                                    start_ms,
                                    end_ms,
                                    text: text.to_string(),
                                },
                            );
                        }
                        Err(e) => {
                            let _ = app.emit("meeting_error", e);
                        }
                    }
                }
                Err(e) => {
                    let notice = crate::captures::keep_meeting_chunk(
                        &db,
                        &app_data_dir,
                        session_id,
                        start_ms as i64,
                        &wav_for_retry,
                        &e,
                    );
                    let _ = app.emit("meeting_error", notice);
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                }
            }
        }
    })
}

async fn join_worker(slot: &std::sync::Mutex<Option<JoinHandle<()>>>) {
    let handle = slot.lock().ok().and_then(|mut guard| guard.take());
    if let Some(handle) = handle {
        let _ = tauri::async_runtime::spawn_blocking(move || handle.join()).await;
    }
}

#[tauri::command]
pub async fn start_meeting(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
    style_id: Option<String>,
    style_prefix: Option<String>,
) -> Result<MeetingSession, String> {
    if state.audio.is_recording() {
        return Err(if state.audio.is_meeting() {
            "A meeting is already recording.".into()
        } else {
            "Stop dictation before starting a meeting.".into()
        });
    }

    let settings = state.db.get_settings().unwrap_or_default();
    let custom = crate::models_folder(&settings);
    if !crate::models::vad_installed(&state.app_data_dir, custom) {
        crate::models::download_vad_model(&app, &state.app_data_dir, custom).await?;
    }

    let language = languages::for_asr(&settings.engine_mode, model_id(&settings), settings.language.as_deref())
        .unwrap_or("")
        .to_string();
    let style_id = style_id.unwrap_or_else(|| "notes".into());
    let style_id = if style_id.trim().is_empty() { "notes".to_string() } else { style_id.trim().to_string() };
    let custom_prefix = style_prefix.unwrap_or_default();
    let styles = crate::presets::load_meeting_styles(&state.app_data_dir);
    let prefix = crate::presets::meeting_style_text(&styles, &style_id, &custom_prefix);
    let session = state.db.create_session(&language, Utc::now().timestamp(), &style_id, &prefix)?;
    let audio_path = state
        .app_data_dir
        .join("sessions")
        .join(session.id.to_string())
        .join("audio.wav");
    let path_str = audio_path.to_string_lossy().to_string();
    if let Err(e) = state.db.set_session_audio_path(session.id, &path_str) {
        let _ = state.db.set_session_status(session.id, "error");
        return Err(e);
    }

    let vad_path = crate::models::vad_model_path(&state.app_data_dir, custom);
    let rx = match state.audio.start_meeting(audio_path, vad_path) {
        Ok((rx, sources)) => {
            let _ = app.emit("meeting_capture", &sources);
            rx
        }
        Err(e) => {
            let _ = state.db.set_session_status(session.id, "error");
            return Err(e);
        }
    };

    state.meeting_cancel.store(false, Ordering::SeqCst);
    if let Ok(mut id) = state.meeting_id.lock() {
        *id = Some(session.id);
    }
    let worker = spawn_meeting_worker(
        app.clone(),
        state.supervisor.clone(),
        state.db.clone(),
        state.app_data_dir.clone(),
        state.meeting_cancel.clone(),
        session.id,
        if language.is_empty() { None } else { Some(language) },
        rx,
    );
    if let Ok(mut slot) = state.meeting_worker.lock() {
        *slot = Some(worker);
    }

    if settings.engine_mode == "local" {
        let supervisor = state.supervisor.clone();
        let engine = settings.local_engine.clone();
        let size = settings.local_model_size.clone();
        let dir = state.app_data_dir.clone();
        let folder = settings.models_folder.clone();
        tauri::async_runtime::spawn(async move {
            let custom = if folder.trim().is_empty() { None } else { Some(folder) };
            let _ = supervisor.ensure_loaded(&engine, &size, &dir, custom.as_deref()).await;
        });
    }

    crate::apply_tray_activity(&app, &state.tray_record_item, "recording", 0);
    state.db.get_session(session.id)
}

#[tauri::command]
pub async fn stop_meeting(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
    session_id: i64,
) -> Result<MeetingDetail, String> {
    let active_id = state.meeting_id.lock().ok().and_then(|guard| *guard);
    if active_id == Some(session_id) && state.audio.is_meeting() {
        state.audio.end_meeting(true)?;
        join_worker(&state.meeting_worker).await;
        if let Ok(mut id) = state.meeting_id.lock() {
            *id = None;
        }
    }
    crate::apply_tray_activity(&app, &state.tray_record_item, "transcribing", 0);
    let detail = write_meeting_note(state.db.clone(), &state.app_data_dir, session_id).await;
    crate::apply_tray_activity(&app, &state.tray_record_item, "idle", 0);
    detail
}

#[tauri::command]
pub async fn cancel_meeting(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
    session_id: i64,
) -> Result<(), String> {
    let active_id = state.meeting_id.lock().ok().and_then(|guard| *guard);
    if active_id == Some(session_id) && state.audio.is_meeting() {
        state.meeting_cancel.store(true, Ordering::SeqCst);
        state.audio.end_meeting(false)?;
        join_worker(&state.meeting_worker).await;
        if let Ok(mut id) = state.meeting_id.lock() {
            *id = None;
        }
    }
    state.db.set_session_status(session_id, "cancelled")?;
    crate::apply_tray_activity(&app, &state.tray_record_item, "idle", 0);
    Ok(())
}

#[tauri::command]
pub fn list_meetings(state: tauri::State<'_, crate::AppState>) -> Result<Vec<MeetingSession>, String> {
    state.db.list_sessions()
}

#[tauri::command]
pub fn get_meeting(state: tauri::State<'_, crate::AppState>, session_id: i64) -> Result<MeetingDetail, String> {
    state.db.meeting_detail(session_id)
}

#[tauri::command]
pub fn delete_meeting(state: tauri::State<'_, crate::AppState>, session_id: i64) -> Result<(), String> {
    if let Ok(captures) = state.db.delete_session_captures(session_id) {
        for capture in captures {
            crate::captures::remove_file(&capture);
        }
    }
    let session = state.db.delete_session(session_id)?;
    remove_session_audio(&session);
    Ok(())
}

#[tauri::command]
pub fn list_session_captures(
    state: tauri::State<'_, crate::AppState>,
    session_id: i64,
) -> Result<Vec<crate::db::AudioCapture>, String> {
    state.db.list_session_captures(session_id)
}

#[tauri::command]
pub fn set_action_item_done(
    state: tauri::State<'_, crate::AppState>,
    id: i64,
    done: bool,
) -> Result<(), String> {
    state.db.set_action_done(id, done)
}

#[tauri::command]
pub fn list_languages(engine_mode: String, model_id: String) -> Vec<languages::LanguageOption> {
    languages::languages_for(&engine_mode, &model_id)
}

#[tauri::command]
pub fn list_meeting_styles(state: tauri::State<'_, crate::AppState>) -> Vec<crate::presets::PromptPreset> {
    crate::presets::load_meeting_styles(&state.app_data_dir)
}

fn remove_session_audio(session: &MeetingSession) {
    let path = PathBuf::from(&session.audio_path);
    if session.audio_path.is_empty() {
        return;
    }
    let _ = std::fs::remove_file(&path);
    if let Some(dir) = path.parent() {
        if dir.file_name().and_then(|name| name.to_str()) == Some(&session.id.to_string()) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

async fn wait_until_settled(db: &Database, session_id: i64) -> Result<MeetingDetail, String> {
    for _ in 0..120 {
        let session = db.get_session(session_id)?;
        if session.status != "summarizing" {
            return db.meeting_detail(session_id);
        }
        let _ = tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(500));
        })
        .await;
    }
    db.meeting_detail(session_id)
}

async fn write_meeting_note(
    db: Arc<Database>,
    app_data_dir: &Path,
    session_id: i64,
) -> Result<MeetingDetail, String> {
    let existing = db.get_session(session_id)?;
    if existing.status == "summarizing" {
        return wait_until_settled(&db, session_id).await;
    }
    if matches!(existing.status.as_str(), "ready" | "cancelled") {
        return db.meeting_detail(session_id);
    }
    if !db.claim_session_summary(session_id)? {
        return wait_until_settled(&db, session_id).await;
    }

    let outcome = finish_claimed_session(db.clone(), app_data_dir, session_id).await;
    if outcome.is_err() {
        let _ = db.set_session_status_if(session_id, "summarizing", "interrupted");
    }
    outcome
}

async fn finish_claimed_session(
    db: Arc<Database>,
    app_data_dir: &Path,
    session_id: i64,
) -> Result<MeetingDetail, String> {
    let session = db.get_session(session_id)?;
    let segments = db.segments_for(session_id)?;
    let llm = crate::env_config::load_llm_settings(app_data_dir);
    let rewriting = session.note_id.is_some();
    let mut title = String::new();
    let mut summary = String::new();
    let mut decisions = Vec::new();
    let mut action_drafts: Vec<(String, Option<String>)> = Vec::new();
    let mut replace_actions = false;

    if segments.is_empty() {
        title = default_title();
        summary.clear();
    } else if llm.enabled {
        let transcript = transcript_for_model(&segments);
        match summarize(db.as_ref(), &llm, &transcript, &session.style_prefix).await {
            Ok(draft) => {
                title = draft.title;
                summary = draft.summary;
                decisions = draft.decisions;
                action_drafts = draft.actions;
                replace_actions = true;
            }
            Err(_) if rewriting => {
                title = session.title.clone();
                summary = session.summary.clone();
                decisions = stored_decisions(&session.decisions);
            }
            Err(e) => {
                summary = format!("The summary could not be written: {e}");
            }
        }
    } else if rewriting {
        title = session.title.clone();
        summary = session.summary.clone();
        decisions = stored_decisions(&session.decisions);
    }

    if title.trim().is_empty() {
        title = default_title();
    }

    let actions = if replace_actions {
        let mut stored_actions = Vec::new();
        for (text, due_raw) in &action_drafts {
            let parsed = due_raw.as_deref().and_then(parse_due);
            stored_actions.push((
                text.clone(),
                parsed.as_ref().map(|(label, _)| label.clone()).or(due_raw.clone()),
                parsed.map(|(_, epoch)| epoch),
            ));
        }
        db.replace_action_items(session_id, &stored_actions)?
    } else if rewriting {
        db.actions_for(session_id)?
    } else {
        db.replace_action_items(session_id, &[])?
    };
    persist_meeting_note(
        db.as_ref(),
        session_id,
        &title,
        &summary,
        &decisions,
        &actions,
        &segments,
        session.note_id.is_none(),
    )?;
    db.meeting_detail(session_id)
}

fn persist_meeting_note(
    db: &Database,
    session_id: i64,
    title: &str,
    summary: &str,
    decisions: &[String],
    actions: &[ActionItem],
    segments: &[crate::db::MeetingSegment],
    first_write: bool,
) -> Result<i64, String> {
    let body = render_meeting_note(summary, decisions, actions, segments);
    let existing = db.get_session(session_id)?;
    let note_id = if let Some(id) = existing.note_id {
        db.update_note(id, title, &body)?;
        id
    } else {
        db.save_note(title, &body)?.id
    };
    let decisions_json = serde_json::to_string(decisions).unwrap_or_else(|_| "[]".into());
    if first_write {
        db.finish_session(session_id, title, summary, &decisions_json, Some(note_id), "ready")?;
    } else {
        db.update_session_summary(session_id, title, summary, &decisions_json, Some(note_id))?;
    }
    Ok(note_id)
}

pub fn attach_recovered_text(
    db: &Database,
    session_id: i64,
    start_ms: i64,
    end_ms: i64,
    text: &str,
) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("The provider returned an empty transcript.".into());
    }
    db.get_session(session_id)?;
    db.insert_segment(session_id, start_ms, end_ms, text)?;
    Ok(())
}

pub fn chunk_end_ms(start_ms: i64, wav: &[u8]) -> i64 {
    let duration = if wav.len() > 44 {
        ((wav.len() - 44) / 32) as i64
    } else {
        0
    };
    start_ms.saturating_add(duration)
}

pub async fn rewrite_if_settled(
    db: Arc<Database>,
    app_data_dir: &Path,
    session_id: i64,
) -> Result<MeetingDetail, String> {
    let session = db.get_session(session_id)?;
    if matches!(session.status.as_str(), "recording" | "cancelled" | "error") {
        return db.meeting_detail(session_id);
    }
    if session.status == "summarizing" {
        return wait_until_settled(&db, session_id).await;
    }
    if !db.claim_session_rewrite(session_id)? {
        return wait_until_settled(&db, session_id).await;
    }
    let outcome = finish_claimed_session(db.clone(), app_data_dir, session_id).await;
    if outcome.is_err() {
        let _ = db.set_session_status_if(session_id, "summarizing", "interrupted");
    }
    outcome
}

async fn summarize(
    db: &Database,
    settings: &crate::env_config::LlmSettings,
    transcript: &str,
    style_prefix: &str,
) -> Result<MeetingDraft, String> {
    let active_id = settings.active_provider_id.clone();
    let provider = settings
        .providers
        .iter()
        .find(|p| p.id == active_id)
        .ok_or_else(|| format!("Provider '{active_id}' not found in configuration"))?;
    let base_url = provider.resolved_base_url();
    let api_key = provider.api_key.clone();
    let provider_type = provider.provider_type.clone();
    let model = if !provider.model.trim().is_empty() {
        provider.model.clone()
    } else {
        settings.model.clone()
    };
    let timeout_secs = settings.request_timeout_secs.max(180) as u64;
    let (text, prompt_tokens, completion_tokens) = crate::llm::transform_text_with_prompt(
        &base_url,
        &api_key,
        &model,
        &crate::llm::meeting_style_prompt(style_prefix),
        transcript,
        timeout_secs,
    )
    .await?;
    if base_url.contains("cloudflare.com") || provider_type == "cloudflare" {
        let _ = db.log_cf_llm_usage(&model, prompt_tokens, completion_tokens);
    }
    Ok(parse_meeting_notes(&text))
}

pub fn notify_due(app: &AppHandle, db: &Database) {
    let now = Utc::now().timestamp();
    let Ok(items) = db.due_action_items(now) else {
        return;
    };
    for item in items {
        let body = match item.due_at.as_deref().filter(|due| !due.is_empty()) {
            Some(due) => format!("{}\nDue {due}", item.text),
            None => item.text.clone(),
        };
        let shown = app.notification().builder().title("Lipi reminder").body(body).show();
        if shown.is_ok() {
            let _ = db.mark_action_reminded(item.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MeetingSegment;

    #[test]
    fn parses_meeting_json_and_due_dates() {
        let raw = "```json\n{\"title\":\"Standup\",\"summary\":\"Shipped the note.\",\"decisions\":[\"Ship it\"],\"action_items\":[{\"text\":\"Email Sam\",\"due\":\"2026-10-18T09:00:00Z\"},\"File the receipt\"]}\n```";
        let draft = parse_meeting_notes(raw);
        assert_eq!(draft.title, "Standup");
        assert_eq!(draft.decisions, vec!["Ship it".to_string()]);
        assert_eq!(draft.actions.len(), 2);
        assert_eq!(draft.actions[0].0, "Email Sam");
        let due = parse_due(draft.actions[0].1.as_deref().unwrap()).unwrap();
        assert!(due.0.contains("2026-10-18"));
        assert!(parse_due("tomorrow").is_none());
        assert!(parse_due("").is_none());
        let day = parse_due("2026-10-18").unwrap();
        assert!(day.0.contains("2026-10-18"));
    }

    #[test]
    fn plain_text_becomes_the_summary() {
        let draft = parse_meeting_notes("not json");
        assert_eq!(draft.summary, "not json");
        assert!(draft.actions.is_empty());
    }

    #[test]
    fn note_contains_summary_actions_and_transcript() {
        let actions = vec![ActionItem {
            id: 1,
            session_id: 1,
            text: "Send notes".into(),
            due_at: Some("2026-10-18T09:00:00Z".into()),
            due_epoch: Some(1),
            done: false,
            reminded: false,
        }];
        let segments = vec![MeetingSegment {
            id: 1,
            session_id: 1,
            start_ms: 1000,
            end_ms: 2000,
            text: "Hello team".into(),
        }];
        let note = render_meeting_note("We met.", &["Ship it".into()], &actions, &segments);
        assert!(note.contains("## Summary"));
        assert!(note.contains("We met."));
        assert!(note.contains("- Ship it"));
        assert!(note.contains("- [ ] Send notes"));
        assert!(note.contains("[00:01] Hello team"));
    }

    #[test]
    fn recovered_text_joins_the_meeting_and_rewrite_keeps_the_note() {
        let db = Database::new(":memory:").unwrap();
        let session = db.create_session("en", 10, "brief", "Keep the summary to two sentences.").unwrap();
        assert_eq!(session.style_id, "brief");
        assert_eq!(session.style_prefix, "Keep the summary to two sentences.");
        attach_recovered_text(&db, session.id, 0, 1200, "hello team").unwrap();
        let segments = db.segments_for(session.id).unwrap();
        assert_eq!(segments[0].text, "hello team");
        let note_id = persist_meeting_note(
            &db,
            session.id,
            "Standup",
            "Hello",
            &[],
            &[],
            &segments,
            true,
        )
        .unwrap();
        attach_recovered_text(&db, session.id, 1200, 2400, "ship the note").unwrap();
        let segments = db.segments_for(session.id).unwrap();
        assert_eq!(segments.len(), 2);
        let again = persist_meeting_note(
            &db,
            session.id,
            "Standup",
            "Hello and ship",
            &[],
            &[],
            &segments,
            false,
        )
        .unwrap();
        assert_eq!(note_id, again);
        let notes = db.get_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].content.contains("ship the note"));
        let saved = db.get_session(session.id).unwrap();
        assert_eq!(saved.status, "ready");
        assert_eq!(saved.note_id, Some(note_id));
        assert!(saved.ended_at.is_some());
    }
}
