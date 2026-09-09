use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSettings {
    pub api_base_url: String,
    pub api_key: String,
    pub model: String,
    pub language: Option<String>,
    pub auto_copy: bool,
    pub always_on_top: bool,
    pub engine_mode: String,
    pub local_engine: String,
    pub local_model_size: String,
    pub models_folder: String,
    pub model_idle_timeout_mins: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            api_base_url: "https://api.openai.com/v1".into(),
            api_key: String::new(),
            model: "whisper-1".into(),
            language: None,
            auto_copy: true,
            always_on_top: true,
            engine_mode: "cloud".into(),
            local_engine: "whisper_cpu".into(),
            local_model_size: "base".into(),
            models_folder: String::new(),
            model_idle_timeout_mins: 10,
        }
    }
}

pub struct Database {
    conn: std::sync::Mutex<Connection>,
}

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    pub fn get_notes(&self) -> Result<Vec<Note>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, title, content, created_at, updated_at FROM notes ORDER BY updated_at DESC")
            .map_err(|e| e.to_string())?;

        let note_iter = stmt
            .query_map([], |row| {
                Ok(Note {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut notes = Vec::new();
        for note in note_iter {
            notes.push(note.map_err(|e| e.to_string())?);
        }
        Ok(notes)
    }

    pub fn save_note(&self, title: &str, content: &str) -> Result<Note, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO notes (title, content, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
            params![title, content],
        )
        .map_err(|e| e.to_string())?;

        let id = conn.last_insert_rowid();
        let mut stmt = conn
            .prepare("SELECT id, title, content, created_at, updated_at FROM notes WHERE id = ?1")
            .map_err(|e| e.to_string())?;

        stmt.query_row(params![id], |row| {
            Ok(Note {
                id: row.get(0)?,
                title: row.get(1)?,
                content: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())
    }

    pub fn update_note(&self, id: i64, title: &str, content: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE notes SET title = ?1, content = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?3",
            params![title, content, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_note(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM notes WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_settings(&self) -> Result<AppSettings, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let get_val = |key: &str| -> Option<String> {
            let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1").ok()?;
            stmt.query_row(params![key], |r| r.get(0)).ok()
        };

        Ok(AppSettings {
            api_base_url: get_val("api_base_url").unwrap_or_else(|| "https://api.openai.com/v1".into()),
            api_key: get_val("api_key").unwrap_or_default(),
            model: get_val("model").unwrap_or_else(|| "whisper-1".into()),
            language: get_val("language").filter(|s| !s.is_empty()),
            auto_copy: get_val("auto_copy").map(|v| v != "false" && v != "0").unwrap_or(true),
            always_on_top: get_val("always_on_top").map(|v| v != "false" && v != "0").unwrap_or(true),
            engine_mode: get_val("engine_mode").unwrap_or_else(|| "cloud".into()),
            local_engine: get_val("local_engine").unwrap_or_else(|| "whisper_cpu".into()),
            local_model_size: get_val("local_model_size").unwrap_or_else(|| "base".into()),
            models_folder: get_val("models_folder").unwrap_or_default(),
            model_idle_timeout_mins: get_val("model_idle_timeout_mins").and_then(|v| v.parse().ok()).unwrap_or(10),
        })
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let set_val = |key: &str, val: &str| -> Result<(), rusqlite::Error> {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![key, val],
            )?;
            Ok(())
        };

        set_val("api_base_url", &settings.api_base_url).map_err(|e| e.to_string())?;
        set_val("api_key", &settings.api_key).map_err(|e| e.to_string())?;
        set_val("model", &settings.model).map_err(|e| e.to_string())?;
        set_val("language", settings.language.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
        set_val("auto_copy", if settings.auto_copy { "true" } else { "false" }).map_err(|e| e.to_string())?;
        set_val("always_on_top", if settings.always_on_top { "true" } else { "false" }).map_err(|e| e.to_string())?;
        set_val("engine_mode", &settings.engine_mode).map_err(|e| e.to_string())?;
        set_val("local_engine", &settings.local_engine).map_err(|e| e.to_string())?;
        set_val("local_model_size", &settings.local_model_size).map_err(|e| e.to_string())?;
        set_val("models_folder", &settings.models_folder).map_err(|e| e.to_string())?;
        set_val("model_idle_timeout_mins", &settings.model_idle_timeout_mins.to_string()).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_crud_and_settings() {
        let db = Database::new(":memory:").expect("Failed to create in-memory db");

        let note = db.save_note("Test Title", "Test Content").unwrap();
        assert_eq!(note.title, "Test Title");
        assert_eq!(note.content, "Test Content");

        let notes = db.get_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, note.id);

        db.update_note(note.id, "Updated Title", "Updated Content").unwrap();
        let updated = db.get_notes().unwrap();
        assert_eq!(updated[0].title, "Updated Title");
        assert_eq!(updated[0].content, "Updated Content");

        db.delete_note(note.id).unwrap();
        let empty_notes = db.get_notes().unwrap();
        assert_eq!(empty_notes.len(), 0);

        let default_settings = db.get_settings().unwrap();
        assert_eq!(default_settings.api_base_url, "https://api.openai.com/v1");
        assert_eq!(default_settings.auto_copy, true);
        assert_eq!(default_settings.always_on_top, true);
        assert_eq!(default_settings.engine_mode, "cloud");
        assert_eq!(default_settings.local_engine, "whisper_cpu");
        assert_eq!(default_settings.local_model_size, "base");
        assert_eq!(default_settings.models_folder, "");
        assert_eq!(default_settings.model_idle_timeout_mins, 10);

        let new_settings = AppSettings {
            api_base_url: "https://api.groq.com/openai/v1".into(),
            api_key: "test_key".into(),
            model: "whisper-large-v3".into(),
            language: Some("en".into()),
            auto_copy: false,
            always_on_top: false,
            engine_mode: "local".into(),
            local_engine: "whisper_vulkan".into(),
            local_model_size: "tiny".into(),
            models_folder: "/external/models".into(),
            model_idle_timeout_mins: 5,
        };
        db.save_settings(&new_settings).unwrap();
        let loaded = db.get_settings().unwrap();
        assert_eq!(loaded.api_base_url, "https://api.groq.com/openai/v1");
        assert_eq!(loaded.api_key, "test_key");
        assert_eq!(loaded.model, "whisper-large-v3");
        assert_eq!(loaded.language, Some("en".into()));
        assert_eq!(loaded.auto_copy, false);
        assert_eq!(loaded.always_on_top, false);
        assert_eq!(loaded.engine_mode, "local");
        assert_eq!(loaded.local_engine, "whisper_vulkan");
        assert_eq!(loaded.local_model_size, "tiny");
        assert_eq!(loaded.models_folder, "/external/models");
        assert_eq!(loaded.model_idle_timeout_mins, 5);
    }

    #[test]
    fn test_db_file_persistence() {
        let temp_file = std::env::temp_dir().join(format!("lipi_test_persist_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&temp_file);

        {
            let db = Database::new(&temp_file).unwrap();
            let mut s = db.get_settings().unwrap();
            s.engine_mode = "local".into();
            s.local_engine = "faster_whisper".into();
            s.local_model_size = "small".into();
            s.models_folder = "/media/drive/models".into();
            s.model_idle_timeout_mins = 30;
            s.api_key = "saved_secret".into();
            db.save_settings(&s).unwrap();
        }

        // Reopen database from same file path
        {
            let db = Database::new(&temp_file).unwrap();
            let loaded = db.get_settings().unwrap();
            assert_eq!(loaded.engine_mode, "local");
            assert_eq!(loaded.local_engine, "faster_whisper");
            assert_eq!(loaded.local_model_size, "small");
            assert_eq!(loaded.models_folder, "/media/drive/models");
            assert_eq!(loaded.model_idle_timeout_mins, 30);
            assert_eq!(loaded.api_key, "saved_secret");
        }

        let _ = std::fs::remove_file(&temp_file);
    }
}

