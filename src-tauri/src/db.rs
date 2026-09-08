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

        let new_settings = AppSettings {
            api_base_url: "https://api.groq.com/openai/v1".into(),
            api_key: "test_key".into(),
            model: "whisper-large-v3".into(),
            language: Some("en".into()),
            auto_copy: false,
            always_on_top: false,
        };
        db.save_settings(&new_settings).unwrap();
        let loaded = db.get_settings().unwrap();
        assert_eq!(loaded.api_base_url, "https://api.groq.com/openai/v1");
        assert_eq!(loaded.api_key, "test_key");
        assert_eq!(loaded.model, "whisper-large-v3");
        assert_eq!(loaded.language, Some("en".into()));
        assert_eq!(loaded.auto_copy, false);
        assert_eq!(loaded.always_on_top, false);
    }
}

