mod audio;
mod db;
mod transcribe;

use audio::AudioRecorder;
use db::{AppSettings, Database, Note};
use std::sync::Arc;
use tauri::Manager;

pub struct AppState {
    db: Arc<Database>,
    audio: Arc<AudioRecorder>,
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

    if settings.api_key.trim().is_empty() && settings.api_base_url.contains("openai.com") {
        return Err("OpenAI API key is missing. Set it in Settings.".into());
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
        let settings = state.db.get_settings().unwrap_or_else(|_| AppSettings {
            api_base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            language: None,
            auto_copy: true,
            always_on_top: true,
        });
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
            let db = Database::new(db_path).expect("Failed to initialize SQLite database");

            app.manage(AppState {
                db: Arc::new(db),
                audio: Arc::new(AudioRecorder::new()),
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
            set_mini_mode,
            set_always_on_top,
            start_drag,
            minimize_window,
            close_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
