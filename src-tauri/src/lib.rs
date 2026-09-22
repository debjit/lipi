mod audio;
mod db;
mod vad;
mod engine;
mod env_config;
mod llm;
mod models;
mod paste;
mod presets;
mod specs;
mod transcribe;

use audio::AudioRecorder;
use db::{AppSettings, Database, Note};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use tauri::{
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

const TRAY_IDLE_ICON: &[u8] = include_bytes!("../icons/tray_idle_32.png");
const TRAY_RECORDING_ICON: &[u8] = include_bytes!("../icons/tray_recording_32.png");

fn format_mmss(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn escape_menu_label(label: &str) -> String {
    label.replace('&', "&&").replace('\n', " ")
}

fn activity_is_busy(phase: &str) -> bool {
    matches!(phase, "recording" | "transcribing" | "transforming")
}

fn activity_tooltip(phase: &str, elapsed_secs: u64) -> String {
    let clock = format_mmss(elapsed_secs);
    match phase {
        "recording" => format!("Lipi · Recording {clock}"),
        "transcribing" => format!("Lipi · Transcribing {clock}"),
        "transforming" => format!("Lipi · Transforming {clock}"),
        _ => "Lipi - Voice to Notes".to_string(),
    }
}

fn activity_menu_label(phase: &str, elapsed_secs: u64) -> String {
    let clock = format_mmss(elapsed_secs);
    match phase {
        "recording" => format!("Recording {clock}"),
        "transcribing" => format!("Transcribing {clock}"),
        "transforming" => format!("Transforming {clock}"),
        _ => "Start / Stop Recording".to_string(),
    }
}

fn update_tray_icon(app: &tauri::AppHandle, busy: bool) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let bytes = if busy { TRAY_RECORDING_ICON } else { TRAY_IDLE_ICON };
        if let Ok(img) = tauri::image::Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(img));
        }
    }
}

fn apply_tray_activity(app: &tauri::AppHandle, record_item: &MenuItem<tauri::Wry>, phase: &str, elapsed_secs: u64) {
    update_tray_icon(app, activity_is_busy(phase));
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_tooltip(Some(activity_tooltip(phase, elapsed_secs)));
    }
    let _ = record_item.set_text(activity_menu_label(phase, elapsed_secs));
}

fn build_preset_submenu(
    app: &tauri::AppHandle,
    app_data_dir: &std::path::Path,
) -> Result<Submenu<tauri::Wry>, String> {
    let settings = env_config::load_llm_settings(app_data_dir);
    let active = settings.voice_preset;
    let presets = presets::load_presets(app_data_dir);
    let has_custom = presets.iter().any(|p| p.id == "custom");

    let mut items: Vec<CheckMenuItem<tauri::Wry>> = Vec::new();
    for preset in &presets {
        let item = CheckMenuItem::with_id(
            app,
            format!("preset:{}", preset.id),
            escape_menu_label(&preset.label),
            true,
            preset.id == active,
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        items.push(item);
    }
    let custom_item = if has_custom {
        None
    } else {
        Some(
            CheckMenuItem::with_id(
                app,
                "preset:custom",
                "Custom Instructions",
                true,
                active == "custom",
                None::<&str>,
            )
            .map_err(|e| e.to_string())?,
        )
    };

    let mut refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|item| item as _).collect();
    if let Some(custom) = custom_item.as_ref() {
        refs.push(custom);
    }

    Submenu::with_items(app, "Preset", true, &refs).map_err(|e| e.to_string())
}

fn build_tray_menu(
    app: &tauri::AppHandle,
    app_data_dir: &std::path::Path,
    record_item: &MenuItem<tauri::Wry>,
) -> Result<Menu<tauri::Wry>, String> {
    let show_item = MenuItem::with_id(app, "show", "Show Lipi", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let mini_item = MenuItem::with_id(app, "toggle_mini", "Toggle Mini Mode", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let preset_menu = build_preset_submenu(app, app_data_dir)?;
    let pref_item = MenuItem::with_id(app, "preferences", "Preferences", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let sep = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Lipi", true, None::<&str>)
        .map_err(|e| e.to_string())?;

    Menu::with_items(
        app,
        &[
            &show_item,
            &mini_item,
            record_item,
            &preset_menu,
            &pref_item,
            &sep,
            &quit_item,
        ],
    )
    .map_err(|e| e.to_string())
}

fn refresh_tray_menu(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let menu = build_tray_menu(app, &state.app_data_dir, &state.tray_record_item)?;
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn select_tray_preset(app: &tauri::AppHandle, preset_id: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let app_data_dir = state.app_data_dir.clone();
    drop(state);

    let mut settings = env_config::load_llm_settings(&app_data_dir);
    if settings.voice_preset == preset_id {
        return;
    }
    settings.presets = presets::load_presets(&app_data_dir);
    settings.voice_preset = preset_id.to_string();
    if let Err(e) = env_config::save_llm_settings(&app_data_dir, &settings) {
        eprintln!("Failed to save tray preset: {e}");
        return;
    }
    let _ = app.emit("tray_preset_changed", preset_id.to_string());
    if let Some(state) = app.try_state::<AppState>() {
        if let Err(e) = refresh_tray_menu(app, &state) {
            eprintln!("Failed to refresh tray presets: {e}");
        }
    }
}

#[tauri::command]
fn set_tray_activity(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    phase: String,
    elapsed_secs: u64,
) -> Result<(), String> {
    apply_tray_activity(&app, &state.tray_record_item, &phase, elapsed_secs);
    Ok(())
}

#[tauri::command]
fn refresh_tray_presets(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    refresh_tray_menu(&app, &state)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OperationResult {
    pub text: String,
    pub cf_neurons: Option<f64>,
    pub cf_cost: Option<f64>,
}

pub struct AppState {
    db: Arc<Database>,
    audio: Arc<AudioRecorder>,
    supervisor: Arc<engine::ModelSupervisor>,
    app_data_dir: std::path::PathBuf,
    is_mini: std::sync::atomic::AtomicBool,
    paste_target: Arc<std::sync::Mutex<Option<paste::PasteTarget>>>,
    global_shortcut_registered: std::sync::atomic::AtomicBool,
    tray_record_item: MenuItem<tauri::Wry>,
    live_worker: Arc<std::sync::Mutex<Option<JoinHandle<Result<String, String>>>>>,
    live_cancel: Arc<AtomicBool>,
}

#[tauri::command]
fn start_recording(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let settings = state.db.get_settings().unwrap_or_default();
    let is_mini = state.is_mini.load(std::sync::atomic::Ordering::SeqCst);
    if settings.auto_paste {
        let target = paste::capture_paste_target(is_mini);
        if let Ok(mut guard) = state.paste_target.lock() {
            *guard = if target.has_target() { Some(target) } else { None };
        }
    } else if let Ok(mut guard) = state.paste_target.lock() {
        *guard = None;
    }

    let live = settings.engine_mode == "local" && settings.live_dictation;
    if live {
        let custom = models_folder(&settings);
        if !models::vad_installed(&state.app_data_dir, custom) {
            return Err("Live dictation needs the VAD model. Turn it on in Settings to download it.".into());
        }
        let path = models::vad_model_path(&state.app_data_dir, custom);
        let rx = state.audio.start_live(path)?;
        state.live_cancel.store(false, Ordering::SeqCst);
        let worker = spawn_live_worker(
            app.clone(),
            state.supervisor.clone(),
            state.db.clone(),
            state.app_data_dir.clone(),
            state.live_cancel.clone(),
            rx,
        );
        if let Ok(mut slot) = state.live_worker.lock() {
            *slot = Some(worker);
        }
        let supervisor = state.supervisor.clone();
        let engine = settings.local_engine.clone();
        let size = settings.local_model_size.clone();
        let dir = state.app_data_dir.clone();
        let folder = settings.models_folder.clone();
        tauri::async_runtime::spawn(async move {
            let custom = if folder.trim().is_empty() { None } else { Some(folder) };
            let _ = supervisor.ensure_loaded(&engine, &size, &dir, custom.as_deref()).await;
        });
    } else {
        state.audio.start()?;
    }
    apply_tray_activity(&app, &state.tray_record_item, "recording", 0);
    Ok(())
}

fn models_folder(settings: &AppSettings) -> Option<&str> {
    if settings.models_folder.trim().is_empty() {
        None
    } else {
        Some(settings.models_folder.as_str())
    }
}

fn spawn_live_worker(
    app: tauri::AppHandle,
    supervisor: Arc<engine::ModelSupervisor>,
    db: Arc<Database>,
    app_data_dir: std::path::PathBuf,
    cancel: Arc<AtomicBool>,
    rx: std::sync::mpsc::Receiver<Vec<f32>>,
) -> JoinHandle<Result<String, String>> {
    std::thread::spawn(move || {
        let mut full = String::new();
        let mut seq = 0u32;
        let mut last_error = None;
        while let Ok(pcm) = rx.recv() {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let wav = match audio::pcm_f32_to_wav(&pcm) {
                Ok(wav) => wav,
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            };
            let settings = db.get_settings().unwrap_or_default();
            let prompt = prompt_tail(&full);
            let text = tauri::async_runtime::block_on(supervisor.transcribe(
                wav,
                &settings.local_engine,
                &settings.local_model_size,
                settings.language.as_deref(),
                &app_data_dir,
                models_folder(&settings),
                prompt.as_deref(),
            ));
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            match text {
                Ok(text) => {
                    let text = text.trim();
                    if text.is_empty() {
                        continue;
                    }
                    if !full.is_empty() {
                        full.push(' ');
                    }
                    full.push_str(text);
                    seq += 1;
                    let _ = app.emit("transcript_chunk", serde_json::json!({ "seq": seq, "text": text }));
                }
                Err(e) => {
                    last_error = Some(e.clone());
                    let _ = app.emit("transcript_error", e);
                }
            }
        }
        if full.is_empty() {
            if let Some(e) = last_error {
                return Err(e);
            }
        }
        Ok(full)
    })
}

fn prompt_tail(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tail: String = trimmed.chars().rev().take(200).collect::<String>().chars().rev().collect();
    Some(tail)
}

fn take_live_worker(state: &AppState) -> Option<JoinHandle<Result<String, String>>> {
    state.live_worker.lock().ok().and_then(|mut slot| slot.take())
}

#[tauri::command]
fn cancel_recording(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.live_cancel.store(true, Ordering::SeqCst);
    if state.audio.is_live() {
        state.audio.end_live(false)?;
        if let Some(handle) = take_live_worker(&state) {
            let _ = handle.join();
        }
    } else if state.audio.is_recording() {
        let _ = state.audio.stop()?;
    }
    apply_tray_activity(&app, &state.tray_record_item, "idle", 0);
    Ok(())
}

#[tauri::command]
async fn paste_into_previous_app(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<bool, String> {
    let target = if let Ok(guard) = state.paste_target.lock() {
        guard.clone()
    } else {
        None
    };

    let has_target = target.as_ref().map(|t| t.has_target()).unwrap_or(false);
    if !has_target {
        // No external target: user was inside Lipi window, keep in Lipi editor!
        return Ok(false);
    }

    if !state.is_mini.load(std::sync::atomic::Ordering::SeqCst) {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.minimize();
        }
    }

    paste::paste_into_previous_app(target, &text)?;
    Ok(true)
}

#[tauri::command]
fn is_global_shortcut_registered(state: tauri::State<'_, AppState>) -> bool {
    state
        .global_shortcut_registered
        .load(std::sync::atomic::Ordering::SeqCst)
}

#[tauri::command]
fn is_recording(state: tauri::State<'_, AppState>) -> bool {
    state.audio.is_recording()
}

#[tauri::command]
async fn stop_recording_and_transcribe(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<OperationResult, String> {
    apply_tray_activity(&app, &state.tray_record_item, "transcribing", 0);
    if state.audio.is_live() {
        state.audio.end_live(true)?;
        let handle = take_live_worker(&state);
        let text = if let Some(handle) = handle {
            tauri::async_runtime::spawn_blocking(move || match handle.join() {
                Ok(result) => result,
                Err(_) => Err("Live dictation thread panicked".into()),
            })
            .await
            .map_err(|e| e.to_string())?
        } else {
            Ok(String::new())
        }?;
        return Ok(OperationResult {
            text,
            cf_neurons: None,
            cf_cost: None,
        });
    }
    let wav_bytes = state.audio.stop()?;
    let settings = state.db.get_settings()?;

    if settings.engine_mode == "local" {
        let custom_dir = models_folder(&settings);
        let text = state
            .supervisor
            .transcribe(
                wav_bytes,
                &settings.local_engine,
                &settings.local_model_size,
                settings.language.as_deref(),
                &state.app_data_dir,
                custom_dir,
                None,
            )
            .await?;
        return Ok(OperationResult {
            text,
            cf_neurons: None,
            cf_cost: None,
        });
    }

    let llm_cfg = env_config::load_llm_settings(&state.app_data_dir);
    let (api_base_url, api_key) = if let Some(pid) = &settings.provider_id {
        if let Some(prov) = llm_cfg.providers.iter().find(|p| &p.id == pid) {
            (prov.resolved_base_url(), prov.api_key.clone())
        } else {
            (settings.api_base_url.clone(), settings.api_key.clone())
        }
    } else {
        (settings.api_base_url.clone(), settings.api_key.clone())
    };

    if api_base_url.trim().is_empty() {
        return Err("API endpoint URL is missing. Set it in Settings.".into());
    }

    let timeout_secs = llm_cfg.request_timeout_secs.max(settings.request_timeout_secs).max(180) as u64;

    let wav_len = wav_bytes.len();
    let duration_secs = if wav_len > 44 {
        (wav_len - 44) as f64 / 32000.0
    } else {
        0.0
    };

    let result = transcribe::transcribe_audio(
        wav_bytes,
        &api_base_url,
        &api_key,
        &settings.model,
        settings.language.as_deref(),
        timeout_secs,
    )
    .await?;

    let mut cf_neurons = None;
    let mut cf_cost = None;

    if api_base_url.contains("api.cloudflare.com") {
        if let Ok((neurons, cost)) = state.db.log_cf_asr_usage(&settings.model, duration_secs) {
            eprintln!(
                "[Cloudflare ASR] Audio: {:.2}s | Model: {} | Consumed: {:.2} Neurons (~${:.5})",
                duration_secs, settings.model, neurons, cost
            );
            cf_neurons = Some(neurons);
            cf_cost = Some(cost);
        }
    }

    Ok(OperationResult {
        text: result,
        cf_neurons,
        cf_cost,
    })
}

#[tauri::command]
fn get_notes(state: tauri::State<'_, AppState>) -> Result<Vec<Note>, String> {
    state.db.get_notes()
}

#[tauri::command]
fn save_note(
    state: tauri::State<'_, AppState>,
    title: String,
    content: String,
) -> Result<Note, String> {
    state.db.save_note(&title, &content)
}

#[tauri::command]
fn update_note(
    state: tauri::State<'_, AppState>,
    id: i64,
    title: String,
    content: String,
) -> Result<(), String> {
    state.db.update_note(id, &title, &content)
}

#[tauri::command]
fn delete_note(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    state.db.delete_note(id)
}

#[tauri::command]
fn clear_all_notes(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.db.clear_all_notes()
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> Result<AppSettings, String> {
    state.db.get_settings()
}

#[tauri::command]
fn save_settings(
    state: tauri::State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    state.db.save_settings(&settings)
}

fn save_current_window_state(db: &Database, window: &tauri::Window, is_mini: bool) {
    let mut current = db.get_window_state().unwrap_or_default();
    let scale_factor = window.scale_factor().unwrap_or(1.0);

    if is_mini {
        if let Ok(pos) = window.outer_position() {
            let logical_pos = pos.to_logical::<f64>(scale_factor);
            current.mini_x = Some(logical_pos.x);
            current.mini_y = Some(logical_pos.y);
            let _ = db.save_window_state(&current);
        }
    } else {
        let is_max = window.is_maximized().unwrap_or(false);
        current.maximized = is_max;
        if !is_max {
            if let Ok(pos) = window.outer_position() {
                let logical_pos = pos.to_logical::<f64>(scale_factor);
                current.x = Some(logical_pos.x);
                current.y = Some(logical_pos.y);
            }
            if let Ok(size) = window.inner_size() {
                let logical_size = size.to_logical::<f64>(scale_factor);
                current.width = Some(logical_size.width);
                current.height = Some(logical_size.height);
            }
        }
        let _ = db.save_window_state(&current);
    }
}

#[tauri::command]
fn set_mini_mode(state: tauri::State<'_, AppState>, window: tauri::Window, mini: bool) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if mini {
        save_current_window_state(&state.db, &window, false);
        state.is_mini.store(true, Ordering::SeqCst);

        let _ = window.set_resizable(false);
        let _ = window.set_decorations(false);
        let _ = window.set_min_size(Some(tauri::LogicalSize::new(120.0, 40.0)));
        let _ = window.set_max_size(Some(tauri::LogicalSize::new(155.0, 50.0)));
        window.set_size(tauri::LogicalSize::new(135.0, 44.0)).map_err(|e| e.to_string())?;
        window.set_always_on_top(true).map_err(|e| e.to_string())?;

        let saved = state.db.get_window_state().unwrap_or_default();
        if let (Some(mx), Some(my)) = (saved.mini_x, saved.mini_y) {
            let _ = window.set_position(tauri::LogicalPosition::new(mx, my));
        } else if let Ok(Some(monitor)) = window.current_monitor() {
            let scale_factor = monitor.scale_factor();
            let screen_size = monitor.size().to_logical::<f64>(scale_factor);
            let screen_pos = monitor.position().to_logical::<f64>(scale_factor);
            let margin_x = 30.0;
            let margin_y = 60.0;
            let widget_h = 44.0;
            let target_x = screen_pos.x + margin_x;
            let target_y = (screen_pos.y + screen_size.height - widget_h - margin_y).max(screen_pos.y);
            let _ = window.set_position(tauri::LogicalPosition::new(target_x, target_y));
        }

        // Re-assert always_on_top after compositor finishes surface reconstruction
        let w = window.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = w.set_always_on_top(true);
            std::thread::sleep(std::time::Duration::from_millis(200));
            let _ = w.set_always_on_top(true);
        });
    } else {
        save_current_window_state(&state.db, &window, true);
        state.is_mini.store(false, Ordering::SeqCst);

        let _ = window.set_max_size::<tauri::LogicalSize<f64>>(None);
        let _ = window.set_min_size(Some(tauri::LogicalSize::new(280.0, 64.0)));
        let _ = window.set_decorations(true);
        let _ = window.set_resizable(true);

        let saved = state.db.get_window_state().unwrap_or_default();
        let target_w = saved.width.unwrap_or(960.0).max(280.0);
        let target_h = saved.height.unwrap_or(700.0).max(64.0);
        window.set_size(tauri::LogicalSize::new(target_w, target_h)).map_err(|e| e.to_string())?;

        if let (Some(x), Some(y)) = (saved.x, saved.y) {
            let _ = window.set_position(tauri::LogicalPosition::new(x, y));
        } else {
            let _ = window.center();
        }

        if saved.maximized {
            let _ = window.maximize();
        }

        let settings = state.db.get_settings().unwrap_or_default();
        let top = settings.always_on_top;
        window.set_always_on_top(top).map_err(|e| e.to_string())?;

        let w = window.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = w.set_always_on_top(top);
        });
    }
    Ok(())
}

#[tauri::command]
fn set_always_on_top(window: tauri::Window, always_on_top: bool) -> Result<(), String> {
    window.set_always_on_top(always_on_top).map_err(|e| e.to_string())?;
    let w = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let _ = w.set_always_on_top(always_on_top);
    });
    Ok(())
}

#[tauri::command]
fn write_to_clipboard(text: String) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_model_memory_status(state: tauri::State<'_, AppState>) -> Result<engine::ModelMemoryStatus, String> {
    let settings = state.db.get_settings().unwrap_or_default();
    Ok(state.supervisor.get_status(settings.model_idle_timeout_mins))
}

#[tauri::command]
fn unload_model_from_memory(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.supervisor.unload();
    Ok(())
}

#[tauri::command]
async fn preload_model_to_memory(
    state: tauri::State<'_, AppState>,
    engine: String,
    model_size: String,
    custom_models_dir: Option<String>,
) -> Result<u16, String> {
    let settings = state.db.get_settings().unwrap_or_default();
    let folder = custom_models_dir.as_deref().or_else(|| {
        if settings.models_folder.trim().is_empty() {
            None
        } else {
            Some(settings.models_folder.as_str())
        }
    });
    state
        .supervisor
        .ensure_loaded(&engine, &model_size, &state.app_data_dir, folder)
        .await
}

#[tauri::command]
fn get_model_status(
    state: tauri::State<'_, AppState>,
    engine: String,
    model_size: String,
    custom_models_dir: Option<String>,
) -> Result<models::ModelStatus, String> {
    let settings = state.db.get_settings().unwrap_or_default();
    let folder = custom_models_dir.as_deref().or_else(|| {
        if settings.models_folder.trim().is_empty() {
            None
        } else {
            Some(settings.models_folder.as_str())
        }
    });
    models::check_model_status(&state.app_data_dir, folder, &engine, &model_size)
}

#[tauri::command]
fn vad_installed(state: tauri::State<'_, AppState>) -> bool {
    let settings = state.db.get_settings().unwrap_or_default();
    models::vad_installed(&state.app_data_dir, models_folder(&settings))
}

#[tauri::command]
async fn download_vad_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let settings = state.db.get_settings().unwrap_or_default();
    models::download_vad_model(&app, &state.app_data_dir, models_folder(&settings)).await
}

#[tauri::command]
async fn download_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    engine: String,
    model_size: String,
    custom_models_dir: Option<String>,
) -> Result<String, String> {
    let settings = state.db.get_settings().unwrap_or_default();
    let folder = custom_models_dir.as_deref().or_else(|| {
        if settings.models_folder.trim().is_empty() {
            None
        } else {
            Some(settings.models_folder.as_str())
        }
    });
    models::download_model_weights(&app, &state.app_data_dir, folder, &engine, &model_size).await
}

#[tauri::command]
fn delete_model(
    state: tauri::State<'_, AppState>,
    engine: String,
    model_size: String,
    custom_models_dir: Option<String>,
) -> Result<(), String> {
    let settings = state.db.get_settings().unwrap_or_default();
    let folder = custom_models_dir.as_deref().or_else(|| {
        if settings.models_folder.trim().is_empty() {
            None
        } else {
            Some(settings.models_folder.as_str())
        }
    });
    models::delete_model(&state.app_data_dir, folder, &engine, &model_size)
}

#[tauri::command]
async fn prepare_engine(
    state: tauri::State<'_, AppState>,
    engine: String,
) -> Result<String, String> {
    let app_data_dir = state.app_data_dir.clone();
    if engine == "faster_whisper" {
        tauri::async_runtime::spawn_blocking(move || {
            models::install_faster_whisper_deps(&app_data_dir)
        })
        .await
        .map_err(|e| e.to_string())?
    } else if engine == "whisper_vulkan" {
        models::ensure_whisper_vulkan_backend(&app_data_dir)
            .await
            .map(|p| format!("Vulkan GPU backend ready at {}", p.to_string_lossy()))
    } else {
        models::ensure_whisper_cli_binary(&app_data_dir)
            .await
            .map(|p| format!("Whisper runner ready at {}", p.to_string_lossy()))
    }
}

#[tauri::command]
fn remove_whisper_binary(state: tauri::State<'_, AppState>) -> Result<String, String> {
    state.supervisor.unload();
    models::delete_whisper_binary(&state.app_data_dir)?;
    Ok("Whisper runner binary removed".into())
}

#[tauri::command]
async fn pick_directory() -> Result<Option<String>, String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("zenity")
            .arg("--file-selection")
            .arg("--directory")
            .arg("--title=Select Models Storage Directory")
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(Some(path));
                }
            }
        }

        if let Ok(output) = std::process::Command::new("kdialog")
            .arg("--title")
            .arg("Select Models Storage Directory")
            .arg("--getexistingdirectory")
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(Some(path));
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let folder = tauri::async_runtime::spawn_blocking(|| {
            rfd::FileDialog::new()
                .set_title("Select Models Storage Directory")
                .pick_folder()
        })
        .await
        .map_err(|e| e.to_string())?;
        return Ok(folder.map(|p| p.to_string_lossy().into_owned()));
    }

    #[cfg(not(target_os = "windows"))]
    Ok(None)
}

#[tauri::command]
async fn install_faster_whisper(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let app_data_dir = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || models::install_faster_whisper_deps(&app_data_dir))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn start_drag(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

#[tauri::command]
fn minimize_window(window: tauri::Window) -> Result<(), String> {
    window.minimize().map_err(|e| e.to_string())
}

#[tauri::command]
fn close_window(state: tauri::State<'_, AppState>, window: tauri::Window) -> Result<(), String> {
    save_current_window_state(
        &state.db,
        &window,
        state.is_mini.load(std::sync::atomic::Ordering::SeqCst),
    );
    window.hide().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_llm_config(state: tauri::State<'_, AppState>) -> Result<env_config::LlmSettings, String> {
    let mut settings = env_config::load_llm_settings(&state.app_data_dir);
    // Load presets dynamically from presets/*.md directory
    settings.presets = presets::load_presets(&state.app_data_dir);
    Ok(settings)
}

#[tauri::command]
fn save_llm_config(
    state: tauri::State<'_, AppState>,
    config: env_config::LlmSettings,
) -> Result<(), String> {
    env_config::save_llm_settings(&state.app_data_dir, &config)?;
    // Synchronize presets with markdown files
    for p in &config.presets {
        let _ = presets::save_preset_file(&state.app_data_dir, p);
    }
    Ok(())
}

#[tauri::command]
fn open_presets_folder(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let dir = presets::presets_dir(&state.app_data_dir);
    let _ = presets::ensure_presets_dir(&state.app_data_dir);
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn delete_preset_markdown(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    presets::delete_preset_file(&state.app_data_dir, &id)
}

#[tauri::command]
fn restore_presets_defaults(state: tauri::State<'_, AppState>) -> Result<Vec<presets::PromptPreset>, String> {
    presets::restore_default_presets(&state.app_data_dir)
}

#[tauri::command]
async fn fetch_llm_models(
    state: tauri::State<'_, AppState>,
    provider_id: String,
) -> Result<Vec<String>, String> {
    let settings = env_config::load_llm_settings(&state.app_data_dir);
    let provider = settings
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    let base_url = provider.resolved_base_url();
    llm::fetch_models(&base_url, &provider.api_key).await
}

#[tauri::command]
async fn transform_with_llm(
    state: tauri::State<'_, AppState>,
    text: String,
    provider_id: Option<String>,
    model: Option<String>,
    preset: String,
    custom_prompt: Option<String>,
) -> Result<OperationResult, String> {
    let mut settings = env_config::load_llm_settings(&state.app_data_dir);
    let active_id = provider_id.unwrap_or_else(|| settings.active_provider_id.clone());

    let (base_url, api_key, provider_type, provider_model) = {
        let provider = settings
            .providers
            .iter()
            .find(|p| p.id == active_id)
            .ok_or_else(|| format!("Provider '{}' not found in configuration", active_id))?;
        (
            provider.resolved_base_url(),
            provider.api_key.clone(),
            provider.provider_type.clone(),
            provider.model.clone(),
        )
    };

    let model_to_use = model
        .filter(|m| !m.trim().is_empty())
        .or_else(|| if !provider_model.trim().is_empty() { Some(provider_model) } else { None })
        .unwrap_or_else(|| settings.model.clone());

    let system_prompt = if preset == "custom" {
        let p = custom_prompt.unwrap_or_else(|| settings.custom_prompt.clone());
        if p.trim().is_empty() {
            "You are an editor. Improve the following text. Do not add commentary. Output ONLY the improved text.".to_string()
        } else {
            p.trim().to_string()
        }
    } else {
        let all_presets = presets::load_presets(&state.app_data_dir);
        if let Some(found) = all_presets.iter().find(|pr| pr.id == preset) {
            found.prompt.clone()
        } else if let Some(found) = settings.presets.iter().find(|pr| pr.id == preset) {
            found.prompt.clone()
        } else {
            llm::resolve_system_prompt(&preset, &custom_prompt.unwrap_or_default())
        }
    };

    let timeout_secs = settings.request_timeout_secs.max(180) as u64;

    let (result_text, prompt_tokens, completion_tokens) = llm::transform_text_with_prompt(
        &base_url,
        &api_key,
        &model_to_use,
        &system_prompt,
        &text,
        timeout_secs,
    )
    .await?;

    // Record used model in provider's model & recent_models
    if !model_to_use.trim().is_empty() {
        let trimmed_m = model_to_use.trim().to_string();
        if let Some(p) = settings.providers.iter_mut().find(|p| p.id == active_id) {
            p.model = trimmed_m.clone();
            if !p.recent_models.contains(&trimmed_m) {
                p.recent_models.insert(0, trimmed_m);
                p.recent_models.truncate(6);
            }
            settings.model = model_to_use.clone();
            let _ = env_config::save_llm_settings(&state.app_data_dir, &settings);
        }
    }

    let mut cf_neurons = None;
    let mut cf_cost = None;

    if base_url.contains("cloudflare.com") || provider_type == "cloudflare" {
        if let Ok((neurons, cost)) = state.db.log_cf_llm_usage(&model_to_use, prompt_tokens, completion_tokens) {
            eprintln!(
                "[Cloudflare LLM] Model: {} | In: {}, Out: {} tokens | Consumed: {:.2} Neurons (~${:.5})",
                model_to_use, prompt_tokens, completion_tokens, neurons, cost
            );
            cf_neurons = Some(neurons);
            cf_cost = Some(cost);
        }
    }

    Ok(OperationResult {
        text: result_text,
        cf_neurons,
        cf_cost,
    })
}

#[tauri::command]
fn get_cf_usage_summary(state: tauri::State<'_, AppState>) -> Result<db::CloudflareUsageSummary, String> {
    state.db.get_cf_usage_summary()
}

#[tauri::command]
fn clear_cf_usage_logs(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.db.clear_cf_usage_logs()
}

#[tauri::command]
fn get_system_specs() -> specs::SystemSpecs {
    specs::detect_system_specs()
}

#[tauri::command]
fn check_system_prerequisites(state: tauri::State<'_, AppState>) -> specs::PrerequisiteStatus {
    specs::check_system_prerequisites(&state.app_data_dir)
}

#[tauri::command]
async fn test_and_fetch_models(base_url: String, api_key: String) -> Result<Vec<String>, String> {
    llm::fetch_models(&base_url, &api_key).await
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));

            if let Err(e) = std::fs::create_dir_all(&app_data_dir) {
                eprintln!("Warning: could not create app_data_dir: {}", e);
            }

            // Ensure presets directory and .guide.md exist
            let _ = presets::ensure_presets_dir(&app_data_dir);

            let old_db_path = app_data_dir.join("transcribe.db");
            let db_path = app_data_dir.join("lipi.db");
            if old_db_path.exists() && !db_path.exists() {
                let _ = std::fs::rename(&old_db_path, &db_path);
            }
            if !db_path.exists() {
                if let Ok(home) = std::env::var("HOME") {
                    let snap_db = std::path::PathBuf::from(home)
                        .join("snap/code/261/.local/share/com.lipi.app/lipi.db");
                    if snap_db.exists() {
                        let _ = std::fs::copy(&snap_db, &db_path);
                    }
                }
            }
            let db = Database::new(db_path).expect("Failed to initialize SQLite database");
            let db_arc = Arc::new(db);
            let supervisor_arc = Arc::new(engine::ModelSupervisor::new());

            let watchdog_db = db_arc.clone();
            let watchdog_sup = supervisor_arc.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(10));
                    let timeout = watchdog_db
                        .get_settings()
                        .map(|s| s.model_idle_timeout_mins)
                        .unwrap_or(10);
                    watchdog_sup.check_idle_timeout(timeout);
                }
            });

            let saved_window_state = db_arc.get_window_state();
            if let Some(w) = app.get_webview_window("main") {
                if let Some(ref saved) = saved_window_state {
                    let target_w = saved.width.unwrap_or(960.0).max(280.0);
                    let target_h = saved.height.unwrap_or(700.0).max(64.0);
                    let _ = w.set_size(tauri::LogicalSize::new(target_w, target_h));
                    if let (Some(x), Some(y)) = (saved.x, saved.y) {
                        let _ = w.set_position(tauri::LogicalPosition::new(x, y));
                    }
                    if saved.maximized {
                        let _ = w.maximize();
                    }
                }
            }

            // Setup System Tray
            let record_item = MenuItem::with_id(app, "toggle_record", "Start / Stop Recording", true, None::<&str>)?;
            let tray_menu = build_tray_menu(app.handle(), &app_data_dir, &record_item)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

            let tray_icon = tauri::image::Image::from_bytes(TRAY_IDLE_ICON)
                .unwrap_or_else(|_| app.default_window_icon().unwrap().clone());

            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(tray_icon)
                .tooltip("Lipi - Voice to Notes")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.unminimize();
                                let _ = w.set_focus();
                            }
                        }
                        "toggle_mini" => {
                            let _ = app.emit("tray_toggle_mini", ());
                        }
                        "toggle_record" => {
                            let _ = app.emit("tray_toggle_recording", ());
                        }
                        "preferences" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.unminimize();
                                let _ = w.set_focus();
                            }
                            let _ = app.emit("open_preferences", ());
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        other => {
                            if let Some(preset_id) = other.strip_prefix("preset:") {
                                select_tray_preset(app, preset_id);
                            }
                        }
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.unminimize();
                                let _ = w.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            let app_handle_for_shortcut = app.handle().clone();
            let mut global_shortcut_registered = false;
            use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
            let shortcut = Shortcut::new(Some(Modifiers::ALT), Code::KeyR);
            match app.global_shortcut().on_shortcut(shortcut, move |_app, _sc, event| {
                if event.state() == ShortcutState::Pressed {
                    let _ = app_handle_for_shortcut.emit("tray_toggle_recording", ());
                }
            }) {
                Ok(_) => {
                    global_shortcut_registered = true;
                }
                Err(e) => {
                    eprintln!("[GlobalShortcut] Warning: Could not register global Alt+R: {}", e);
                }
            }

            app.manage(AppState {
                db: db_arc,
                audio: Arc::new(AudioRecorder::new()),
                supervisor: supervisor_arc,
                app_data_dir,
                is_mini: std::sync::atomic::AtomicBool::new(false),
                paste_target: Arc::new(std::sync::Mutex::new(None)),
                global_shortcut_registered: std::sync::atomic::AtomicBool::new(global_shortcut_registered),
                tray_record_item: record_item,
                live_worker: Arc::new(std::sync::Mutex::new(None)),
                live_cancel: Arc::new(AtomicBool::new(false)),
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Some(state) = window.try_state::<AppState>() {
                    save_current_window_state(
                        &state.db,
                        window,
                        state.is_mini.load(std::sync::atomic::Ordering::SeqCst),
                    );
                }
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_tray_activity,
            refresh_tray_presets,
            start_recording,
            cancel_recording,
            vad_installed,
            download_vad_model,
            is_recording,
            stop_recording_and_transcribe,
            paste_into_previous_app,
            is_global_shortcut_registered,
            get_notes,
            save_note,
            update_note,
            delete_note,
            clear_all_notes,
            get_settings,
            save_settings,
            get_model_status,
            get_model_memory_status,
            unload_model_from_memory,
            preload_model_to_memory,
            download_model,
            delete_model,
            install_faster_whisper,
            prepare_engine,
            remove_whisper_binary,
            pick_directory,
            write_to_clipboard,
            set_mini_mode,
            set_always_on_top,
            start_drag,
            minimize_window,
            close_window,
            get_llm_config,
            save_llm_config,
            fetch_llm_models,
            transform_with_llm,
            open_presets_folder,
            delete_preset_markdown,
            restore_presets_defaults,
            get_cf_usage_summary,
            clear_cf_usage_logs,
            get_system_specs,
            check_system_prerequisites,
            test_and_fetch_models
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
