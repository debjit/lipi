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
    source: String,
}

struct MeetingDraft {
    title: String,
    summary: String,
    decisions: Vec<String>,
    actions: Vec<(String, Option<String>)>,
}

fn is_parakeet(settings: &crate::db::AppSettings) -> bool {
    let engine = if settings.meeting_engine.trim().is_empty() {
        "parakeet"
    } else {
        settings.meeting_engine.as_str()
    };
    engine == "parakeet" || (engine == "dictation" && settings.engine_mode == "local" && settings.local_engine == "parakeet")
}

fn model_id(settings: &crate::db::AppSettings) -> &str {
    if is_parakeet(settings) {
        "parakeet"
    } else if settings.engine_mode == "local" {
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
            out.push_str(&format!("[{}] {}\n\n", segment_stamp(seg), seg.text.trim()));
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
        text.push_str(&format!("[{}] {}", segment_stamp(seg), seg.text.trim()));
    }
    clip_transcript(&text)
}

fn segment_stamp(seg: &crate::db::MeetingSegment) -> String {
    let who = match seg.source.as_str() {
        "mic" => "Mic ",
        "call" => "Call ",
        _ => "",
    };
    format!("{who}{}", fmt_ms(seg.start_ms))
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
                MeetingAudio::Pcm { .. } => continue,
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
                    match db.insert_segment(session_id, start_ms as i64, end_ms as i64, "", text) {
                        Ok(_) => {
                            let _ = app.emit(
                                "meeting_segment",
                                SegmentEvent {
                                    session_id,
                                    start_ms,
                                    end_ms,
                                    text: text.to_string(),
                                    source: String::new(),
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
    live: Option<bool>,
    keep_audio: Option<bool>,
    call_only: Option<bool>,
    tavily: Option<bool>,
) -> Result<MeetingSession, String> {
    if state.audio.is_recording() {
        let msg = if state.audio.is_meeting() {
            "A meeting is already recording."
        } else {
            "Stop dictation before starting a meeting."
        };
        eprintln!("[Meeting] Cannot start meeting: {msg}");
        return Err(msg.into());
    }

    let settings = state.db.get_settings().unwrap_or_default();
    let custom = crate::models_folder(&settings);
    let live = live.unwrap_or(true);
    let call_only = call_only.unwrap_or(false);
    let parakeet = is_parakeet(&settings);
    eprintln!(
        "[Meeting] start_meeting (engine: {}, parakeet: {}, live: {}, call_only: {})",
        settings.meeting_engine, parakeet, live, call_only
    );
    if parakeet {
        let models_dir = crate::models::resolve_models_dir(&state.app_data_dir, custom);
        if live {
            crate::parakeet::require_meeting_models(&models_dir)?;
        } else if !crate::parakeet::tdt_ready(&models_dir) {
            return Err("Download Parakeet in Settings before starting a meeting.".into());
        }
    } else if live && !crate::models::vad_installed(&state.app_data_dir, custom) {
        crate::models::download_vad_model(&app, &state.app_data_dir, custom).await?;
    }
    state.meeting_later.store(!live, Ordering::SeqCst);
    state.meeting_keep_audio.store(keep_audio.unwrap_or(true), Ordering::SeqCst);

    let language = languages::for_asr(&settings.engine_mode, model_id(&settings), settings.language.as_deref())
        .unwrap_or("")
        .to_string();
    let style_id = style_id.unwrap_or_else(|| "notes".into());
    let style_id = if style_id.trim().is_empty() { "notes".to_string() } else { style_id.trim().to_string() };
    let custom_prefix = style_prefix.unwrap_or_default();
    let styles = crate::presets::load_meeting_styles(&state.app_data_dir);
    let prefix = crate::presets::meeting_style_text(&styles, &style_id, &custom_prefix);
    let session = state.db.create_session(
        &language,
        Utc::now().timestamp(),
        &style_id,
        &prefix,
        tavily.unwrap_or(false),
    )?;
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

    let vad_path = if parakeet || !live {
        None
    } else {
        Some(crate::models::vad_model_path(&state.app_data_dir, custom))
    };
    let session_dir = audio_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| state.app_data_dir.join("sessions").join(session.id.to_string()));
    let rx = match state.audio.start_meeting(audio_path, vad_path, live && parakeet, call_only) {
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
    let worker = if !live {
        drop(rx);
        None
    } else if parakeet {
        let app = app.clone();
        let db = state.db.clone();
        let models_dir = crate::models::resolve_models_dir(&state.app_data_dir, custom);
        let cancel = state.meeting_cancel.clone();
        let session_id = session.id;
        Some(std::thread::spawn(move || {
            crate::parakeet::run_meeting(app, db, models_dir, session_dir, cancel, session_id, call_only, rx);
        }))
    } else {
        Some(spawn_meeting_worker(
            app.clone(),
            state.supervisor.clone(),
            state.db.clone(),
            state.app_data_dir.clone(),
            state.meeting_cancel.clone(),
            session.id,
            if language.is_empty() { None } else { Some(language) },
            rx,
        ))
    };
    if let Ok(mut slot) = state.meeting_worker.lock() {
        *slot = worker;
    }

    if live && settings.engine_mode == "local" && !parakeet {
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
    eprintln!("[Meeting] stop_meeting requested for session #{}", session_id);
    let active_id = state.meeting_id.lock().ok().and_then(|guard| *guard);
    if active_id == Some(session_id) && state.audio.is_meeting() {
        state.audio.end_meeting(true)?;
        join_worker(&state.meeting_worker).await;
        if let Ok(mut id) = state.meeting_id.lock() {
            *id = None;
        }
    }
    let later = state.meeting_later.swap(false, Ordering::SeqCst);
    let keep = state.meeting_keep_audio.load(Ordering::SeqCst);
    crate::apply_tray_activity(&app, &state.tray_record_item, "transcribing", 0);
    if later {
        if let Err(err) = transcribe_saved_recording(&app, &state, session_id).await {
            let _ = state.db.set_session_status(session_id, "interrupted");
            crate::apply_tray_activity(&app, &state.tray_record_item, "idle", 0);
            return Err(err);
        }
    }
    let detail = match write_meeting_note(state.db.clone(), &state.app_data_dir, session_id).await {
        Ok(detail) => detail,
        Err(err) => {
            if let Ok(session) = state.db.get_session(session_id) {
                tidy_recording(&session, true);
            }
            crate::apply_tray_activity(&app, &state.tray_record_item, "idle", 0);
            return Err(err);
        }
    };
    if let Ok(session) = state.db.get_session(session_id) {
        tidy_recording(&session, keep);
    }
    crate::apply_tray_activity(&app, &state.tray_record_item, "idle", 0);
    let mut detail = detail;
    if !keep || !recording_file(&detail.session).is_file() {
        detail.session.audio_path.clear();
    }
    Ok(detail)
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
    let keep = state.meeting_keep_audio.load(Ordering::SeqCst);
    state.meeting_later.store(false, Ordering::SeqCst);
    state.db.set_session_status(session_id, "cancelled")?;
    if let Ok(session) = state.db.get_session(session_id) {
        tidy_recording(&session, keep);
    }
    crate::apply_tray_activity(&app, &state.tray_record_item, "idle", 0);
    Ok(())
}

#[tauri::command]
pub fn list_meetings(state: tauri::State<'_, crate::AppState>) -> Result<Vec<MeetingSession>, String> {
    state.db.list_sessions()
}

#[tauri::command]
pub fn get_meeting(state: tauri::State<'_, crate::AppState>, session_id: i64) -> Result<MeetingDetail, String> {
    let mut detail = state.db.meeting_detail(session_id)?;
    if !recording_file(&detail.session).is_file() {
        detail.session.audio_path.clear();
    }
    Ok(detail)
}

#[tauri::command]
pub async fn retranscribe_meeting(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
    session_id: i64,
) -> Result<MeetingDetail, String> {
    let active = state.meeting_id.lock().ok().and_then(|guard| *guard);
    if active == Some(session_id) && state.audio.is_meeting() {
        return Err("Stop the meeting before transcribing it again.".into());
    }
    crate::apply_tray_activity(&app, &state.tray_record_item, "transcribing", 0);
    let transcribed = transcribe_saved_recording(&app, &state, session_id).await;
    if let Err(err) = transcribed {
        crate::apply_tray_activity(&app, &state.tray_record_item, "idle", 0);
        return Err(err);
    }
    let session = state.db.get_session(session_id)?;
    if matches!(session.status.as_str(), "cancelled" | "recording" | "error") {
        state.db.set_session_status(session_id, "interrupted")?;
    }
    let detail = rewrite_if_settled(state.db.clone(), &state.app_data_dir, session_id).await;
    crate::apply_tray_activity(&app, &state.tray_record_item, "idle", 0);
    detail
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

fn recording_file(session: &MeetingSession) -> PathBuf {
    PathBuf::from(&session.audio_path)
}

fn tidy_recording(session: &MeetingSession, keep: bool) {
    let path = recording_file(session);
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_file(dir.join("mic.wav"));
        let _ = std::fs::remove_file(dir.join("call.wav"));
    }
    if !keep {
        let _ = std::fs::remove_file(&path);
    }
}

async fn transcribe_saved_recording(
    app: &AppHandle,
    state: &tauri::State<'_, crate::AppState>,
    session_id: i64,
) -> Result<(), String> {
    let session = state.db.get_session(session_id)?;
    let path = recording_file(&session);
    if !path.is_file() {
        return Err("This meeting has no saved recording.".into());
    }
    let _ = app.emit("meeting_transcribing", true);
    let pcm = crate::parakeet::read_wav_file(&path)?;
    let settings = state.db.get_settings().unwrap_or_default();
    let parakeet = is_parakeet(&settings);
    let rows = if parakeet {
        let models_dir = crate::models::resolve_models_dir(&state.app_data_dir, crate::models_folder(&settings));
        tauri::async_runtime::spawn_blocking(move || crate::parakeet::transcribe_pcm(&models_dir, &pcm))
            .await
            .map_err(|err| err.to_string())??
            .into_iter()
            .map(|(start, end, text)| (start, end, String::new(), text))
            .collect()
    } else {
        let ranges = crate::parakeet::split_ranges(pcm.len(), |at| crate::parakeet::region_quiet(&pcm, at));
        let language = if session.language.trim().is_empty() {
            None
        } else {
            Some(session.language.clone())
        };
        let mut rows = Vec::new();
        for (start, end) in ranges {
            let wav = crate::audio::pcm_f32_to_wav(&pcm[start..end])?;
            let samples = end.saturating_sub(start);
            let text = transcribe_chunk(
                &state.app_data_dir,
                &state.supervisor,
                &state.db,
                wav,
                language.as_deref(),
            )
            .await?;
            let Some(text) = crate::transcribe::accept_asr_text(&text, samples) else {
                continue;
            };
            let start_ms = (start as i64) * 1000 / 16_000;
            let end_ms = (end as i64) * 1000 / 16_000;
            rows.push((start_ms, end_ms, String::new(), text.to_string()));
        }
        rows
    };
    let _ = app.emit("meeting_transcribing", false);
    if rows.is_empty() {
        return Err("The recording produced no words. The previous transcript was kept.".into());
    }
    state.db.replace_segments(session_id, &rows)
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
        match summarize(db.as_ref(), &llm, &transcript, &session.style_prefix, session.tavily_enabled).await {
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
    db.insert_segment(session_id, start_ms, end_ms, "", text)?;
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

fn transcript_urls(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|c: char| {
            matches!(c, ',' | '.' | ')' | '(' | '"' | '\'' | ';' | ']' | '[' | '>' | '<' | '!')
        });
        if !(token.starts_with("https://") || token.starts_with("http://")) {
            continue;
        }
        if token.len() <= "https://".len() {
            continue;
        }
        if found.iter().any(|url: &String| url == token) {
            continue;
        }
        found.push(token.to_string());
        if found.len() == 3 {
            break;
        }
    }
    found
}

fn tavily_urls(enabled: bool, api_key: &str, transcript: &str) -> Vec<String> {
    if !enabled || api_key.trim().is_empty() {
        return Vec::new();
    }
    transcript_urls(transcript)
}

fn format_extract_context(body: &str) -> String {
    let Ok(json) = serde_json::from_str::<Value>(body) else {
        return String::new();
    };
    let Some(results) = json.get("results").and_then(|v| v.as_array()) else {
        return String::new();
    };
    let mut out = String::new();
    for item in results {
        let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let content = item
            .get("raw_content")
            .or_else(|| item.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if url.is_empty() && content.is_empty() {
            continue;
        }
        let snippet: String = content.chars().take(1500).collect();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(url);
        if !snippet.is_empty() {
            out.push('\n');
            out.push_str(&snippet);
        }
        if out.chars().count() >= 4000 {
            break;
        }
    }
    out.chars().take(4000).collect()
}

fn transcript_for_notes(transcript: &str, pages: &str) -> String {
    let pages = pages.trim();
    if pages.is_empty() {
        transcript.to_string()
    } else {
        format!("{transcript}\n\nWeb pages named in the meeting:\n{pages}")
    }
}

fn notes_prompt(style_prefix: &str, has_pages: bool) -> String {
    let base = crate::llm::meeting_style_prompt(style_prefix);
    if !has_pages {
        base
    } else {
        format!("{base}\n\nA block titled \"Web pages named in the meeting\" follows the transcript. Use it only to clarify sites that were actually named. Decisions and tasks still come only from what was said.")
    }
}

async fn named_page_context(
    settings: &crate::env_config::LlmSettings,
    enabled: bool,
    transcript: &str,
) -> String {
    let urls = tavily_urls(enabled, &settings.tavily_api_key, transcript);
    if urls.is_empty() {
        return String::new();
    }
    match tavily_extract(settings.tavily_api_key.trim(), &urls).await {
        Ok(body) => format_extract_context(&body),
        Err(e) => {
            eprintln!("tavily extract failed: {e}");
            String::new()
        }
    }
}

async fn tavily_extract(api_key: &str, urls: &[String]) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("Lipi/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| crate::llm::format_reqwest_error(&e))?;
    let resp = client
        .post("https://api.tavily.com/extract")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&serde_json::json!({ "urls": urls }))
        .send()
        .await
        .map_err(|e| crate::llm::format_reqwest_error(&e))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {}: {body}", status.as_u16()));
    }
    Ok(body)
}

async fn summarize(
    db: &Database,
    settings: &crate::env_config::LlmSettings,
    transcript: &str,
    style_prefix: &str,
    use_tavily: bool,
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
    let pages = named_page_context(settings, use_tavily, transcript).await;
    let user_text = transcript_for_notes(transcript, &pages);
    let prompt = notes_prompt(style_prefix, !pages.trim().is_empty());
    let (text, prompt_tokens, completion_tokens) = crate::llm::transform_text_with_prompt(
        &base_url,
        &api_key,
        &model,
        &prompt,
        &user_text,
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
    fn tavily_runs_only_when_enabled_and_a_site_is_named() {
        let said = "See https://example.com/docs and also https://example.com/docs.";
        assert!(tavily_urls(false, "tvly-test", said).is_empty());
        assert!(transcript_urls("no site here").is_empty());
        assert!(tavily_urls(true, "", said).is_empty());
        assert_eq!(tavily_urls(true, "tvly-test", said), vec!["https://example.com/docs".to_string()]);
        assert!(tavily_urls(true, "tvly-test", "talked about the launch").is_empty());
        assert_eq!(transcript_for_notes("hello", ""), "hello");
        assert!(transcript_for_notes("hello", "page").contains("Web pages named in the meeting"));
        let body = r#"{"results":[{"url":"https://example.com/docs","raw_content":"Setup guide"}],"failed_results":[]}"#;
        let pages = format_extract_context(body);
        assert!(pages.contains("https://example.com/docs"));
        assert!(pages.contains("Setup guide"));
        assert!(format_extract_context("not json").is_empty());
        assert!(format_extract_context(r#"{"failed_results":[{"url":"https://example.com","error":"404"}]}"#).is_empty());
        assert!(!notes_prompt("", false).contains("Web pages named"));
        assert!(notes_prompt("", true).contains("only to clarify sites"));
    }

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
            source: String::new(),
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
        let session = db.create_session("en", 10, "brief", "Keep the summary to two sentences.", true).unwrap();
        assert_eq!(session.style_id, "brief");
        assert_eq!(session.style_prefix, "Keep the summary to two sentences.");
        assert!(session.tavily_enabled);
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
