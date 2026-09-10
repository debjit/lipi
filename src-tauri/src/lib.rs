mod audio;
mod db;
mod engine;
mod env_config;
mod llm;
mod models;
mod presets;
mod transcribe;

use audio::AudioRecorder;
use db::{AppSettings, Database, Note};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::Manager;

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
}

#[tauri::command]
fn start_recording(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.audio.start()
}

#[tauri::command]
fn is_recording(state: tauri::State<'_, AppState>) -> bool {
    state.audio.is_recording()
}

#[tauri::command]
async fn stop_recording_and_transcribe(
    state: tauri::State<'_, AppState>,
) -> Result<OperationResult, String> {
    let wav_bytes = state.audio.stop()?;
    let settings = state.db.get_settings()?;

    if settings.engine_mode == "local" {
        let custom_dir = if settings.models_folder.trim().is_empty() {
            None
        } else {
            Some(settings.models_folder.as_str())
        };
        let text = state
            .supervisor
            .transcribe(
                wav_bytes,
                &settings.local_engine,
                &settings.local_model_size,
                settings.language.as_deref(),
                &state.app_data_dir,
                custom_dir,
            )
            .await?;
        return Ok(OperationResult {
            text,
            cf_neurons: None,
            cf_cost: None,
        });
    }

    if settings.api_base_url.trim().is_empty() {
        return Err("API endpoint URL is missing. Set it in Settings.".into());
    }

    let llm_cfg = env_config::load_llm_settings(&state.app_data_dir);
    let timeout_secs = llm_cfg.request_timeout_secs.max(settings.request_timeout_secs).max(180) as u64;

    let wav_len = wav_bytes.len();
    let duration_secs = if wav_len > 44 {
        (wav_len - 44) as f64 / 32000.0
    } else {
        0.0
    };

    let result = transcribe::transcribe_audio(
        wav_bytes,
        &settings.api_base_url,
        &settings.api_key,
        &settings.model,
        settings.language.as_deref(),
        timeout_secs,
    )
    .await?;

    let mut cf_neurons = None;
    let mut cf_cost = None;

    if settings.api_base_url.contains("api.cloudflare.com") {
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
    } else {
        models::ensure_whisper_cli_binary(&app_data_dir)
            .await
            .map(|p| format!("Whisper runner ready at {}", p.to_string_lossy()))
    }
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
    }
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
    window.close().map_err(|e| e.to_string())
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
    let settings = env_config::load_llm_settings(&state.app_data_dir);
    let active_id = provider_id.unwrap_or_else(|| settings.active_provider_id.clone());

    let provider = settings
        .providers
        .iter()
        .find(|p| p.id == active_id)
        .ok_or_else(|| format!("Provider '{}' not found in configuration", active_id))?;

    let base_url = provider.resolved_base_url();
    let model_to_use = model.unwrap_or_else(|| settings.model.clone());

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
        &provider.api_key,
        &model_to_use,
        &system_prompt,
        &text,
        timeout_secs,
    )
    .await?;

    let mut cf_neurons = None;
    let mut cf_cost = None;

    if base_url.contains("cloudflare.com") || provider.provider_type == "cloudflare" {
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


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
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

            app.manage(AppState {
                db: db_arc,
                audio: Arc::new(AudioRecorder::new()),
                supervisor: supervisor_arc,
                app_data_dir,
                is_mini: std::sync::atomic::AtomicBool::new(false),
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(state) = window.try_state::<AppState>() {
                    save_current_window_state(
                        &state.db,
                        window,
                        state.is_mini.load(std::sync::atomic::Ordering::SeqCst),
                    );
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            start_recording,
            is_recording,
            stop_recording_and_transcribe,
            get_notes,
            save_note,
            update_note,
            delete_note,
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
            clear_cf_usage_logs
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
