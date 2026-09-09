mod audio;
mod db;
mod engine;
mod models;
mod transcribe;

use audio::AudioRecorder;
use db::{AppSettings, Database, Note};
use std::sync::Arc;
use tauri::Manager;

pub struct AppState {
    db: Arc<Database>,
    audio: Arc<AudioRecorder>,
    supervisor: Arc<engine::ModelSupervisor>,
    app_data_dir: std::path::PathBuf,
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
) -> Result<String, String> {
    let wav_bytes = state.audio.stop()?;
    let settings = state.db.get_settings()?;

    if settings.engine_mode == "local" {
        let custom_dir = if settings.models_folder.trim().is_empty() {
            None
        } else {
            Some(settings.models_folder.as_str())
        };
        return state
            .supervisor
            .transcribe(
                wav_bytes,
                &settings.local_engine,
                &settings.local_model_size,
                settings.language.as_deref(),
                &state.app_data_dir,
                custom_dir,
            )
            .await;
    }

    if settings.api_base_url.trim().is_empty() {
        return Err("API endpoint URL is missing. Set it in Settings.".into());
    }

    transcribe::transcribe_audio(
        wav_bytes,
        &settings.api_base_url,
        &settings.api_key,
        &settings.model,
        settings.language.as_deref(),
    )
    .await
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

#[tauri::command]
fn set_mini_mode(state: tauri::State<'_, AppState>, window: tauri::Window, mini: bool) -> Result<(), String> {
    if mini {
        let _ = window.set_resizable(false);
        let _ = window.set_decorations(false);
        let _ = window.set_min_size(Some(tauri::LogicalSize::new(120.0, 40.0)));
        let _ = window.set_max_size(Some(tauri::LogicalSize::new(155.0, 50.0)));
        window.set_size(tauri::LogicalSize::new(135.0, 44.0)).map_err(|e| e.to_string())?;
        window.set_always_on_top(true).map_err(|e| e.to_string())?;

        if let Ok(Some(monitor)) = window.current_monitor() {
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
        let _ = window.set_max_size::<tauri::LogicalSize<f64>>(None);
        let _ = window.set_min_size(Some(tauri::LogicalSize::new(280.0, 64.0)));
        let _ = window.set_decorations(true);
        let _ = window.set_resizable(true);
        window.set_size(tauri::LogicalSize::new(960.0, 700.0)).map_err(|e| e.to_string())?;
        let _ = window.center();
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
fn close_window(window: tauri::Window) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
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

            app.manage(AppState {
                db: db_arc,
                audio: Arc::new(AudioRecorder::new()),
                supervisor: supervisor_arc,
                app_data_dir,
            });

            Ok(())
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
            close_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
