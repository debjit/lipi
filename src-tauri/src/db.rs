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
    pub request_timeout_secs: u32,
    pub mini_record_mode: String,
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
            request_timeout_secs: 180,
            mini_record_mode: "new_note".into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct WindowState {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub maximized: bool,
    pub mini_x: Option<f64>,
    pub mini_y: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CloudflareUsagePeriod {
    pub label: String,
    pub total_neurons: f64,
    pub total_cost_usd: f64,
    pub asr_audio_secs: f64,
    pub asr_count: u32,
    pub llm_tokens: u32,
    pub llm_count: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CloudflareUsageSummary {
    pub today: CloudflareUsagePeriod,
    pub current_month: CloudflareUsagePeriod,
    pub all_time: CloudflareUsagePeriod,
    pub monthly_history: Vec<CloudflareUsagePeriod>,
}

pub const CF_USD_PER_NEURON: f64 = 0.011 / 1000.0;

pub fn cf_asr_neurons(model: &str, duration_secs: f64) -> f64 {
    let lower = model.to_lowercase();
    if lower.contains("tiny") {
        return 0.0;
    }
    let neurons_per_min = if lower.contains("whisper-large") || lower.contains("turbo") || lower.contains("large-v3") {
        46.63
    } else {
        41.14
    };
    (duration_secs.max(0.0) / 60.0) * neurons_per_min
}

pub fn cf_llm_neurons(model: &str, prompt_tokens: u32, completion_tokens: u32) -> f64 {
    let lower = model.to_lowercase();
    if lower.contains("70b") {
        (prompt_tokens as f64 * (260_000.0 / 1_000_000.0)) + (completion_tokens as f64 * (780_000.0 / 1_000_000.0))
    } else {
        // default 8B (e.g. @cf/meta/llama-3.1-8b-instruct)
        (prompt_tokens as f64 * (25_608.0 / 1_000_000.0)) + (completion_tokens as f64 * (75_147.0 / 1_000_000.0))
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
            CREATE TABLE IF NOT EXISTS cf_usage_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                service_type TEXT NOT NULL,
                model TEXT NOT NULL,
                audio_duration_secs REAL NOT NULL DEFAULT 0.0,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                neurons_consumed REAL NOT NULL DEFAULT 0.0,
                cost_usd REAL NOT NULL DEFAULT 0.0,
                day_date TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d', 'now')),
                month TEXT NOT NULL DEFAULT (strftime('%Y-%m', 'now')),
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            ",
        )?;

        // Safe migrations for pre-existing databases that lack day_date or month columns
        let _ = conn.execute(
            "ALTER TABLE cf_usage_logs ADD COLUMN day_date TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE cf_usage_logs ADD COLUMN month TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "UPDATE cf_usage_logs SET day_date = strftime('%Y-%m-%d', created_at) WHERE day_date IS NULL OR day_date = ''",
            [],
        );
        let _ = conn.execute(
            "UPDATE cf_usage_logs SET month = strftime('%Y-%m', created_at) WHERE month IS NULL OR month = ''",
            [],
        );

        // Indexes must only be created AFTER columns are guaranteed to exist
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_cf_usage_day ON cf_usage_logs(day_date);
            CREATE INDEX IF NOT EXISTS idx_cf_usage_month ON cf_usage_logs(month);
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
            request_timeout_secs: get_val("request_timeout_secs").and_then(|v| v.parse().ok()).unwrap_or(180).max(180),
            mini_record_mode: get_val("mini_record_mode").unwrap_or_else(|| "new_note".into()),
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
        set_val("request_timeout_secs", &settings.request_timeout_secs.max(180).to_string()).map_err(|e| e.to_string())?;
        set_val("mini_record_mode", &settings.mini_record_mode).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_window_state(&self) -> Option<WindowState> {
        let conn = self.conn.lock().ok()?;
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = 'window_state'").ok()?;
        let val: String = stmt.query_row([], |r| r.get(0)).ok()?;
        serde_json::from_str(&val).ok()
    }

    pub fn save_window_state(&self, state: &WindowState) -> Result<(), String> {
        let json = serde_json::to_string(state).map_err(|e| e.to_string())?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('window_state', ?1) ON CONFLICT(key) DO UPDATE SET value = ?1",
            params![json],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn log_cf_asr_usage(&self, model: &str, audio_duration_secs: f64) -> Result<(f64, f64), String> {
        let neurons = cf_asr_neurons(model, audio_duration_secs);
        let cost = neurons * CF_USD_PER_NEURON;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO cf_usage_logs (
                service_type, model, audio_duration_secs,
                prompt_tokens, completion_tokens, total_tokens,
                neurons_consumed, cost_usd, day_date, month, created_at
            ) VALUES (
                'asr', ?1, ?2, 0, 0, 0, ?3, ?4, strftime('%Y-%m-%d', 'now'), strftime('%Y-%m', 'now'), CURRENT_TIMESTAMP
            )",
            params![model, audio_duration_secs, neurons, cost],
        )
        .map_err(|e| e.to_string())?;
        Ok((neurons, cost))
    }

    pub fn log_cf_llm_usage(&self, model: &str, prompt_tokens: u32, completion_tokens: u32) -> Result<(f64, f64), String> {
        let neurons = cf_llm_neurons(model, prompt_tokens, completion_tokens);
        let cost = neurons * CF_USD_PER_NEURON;
        let total_tokens = prompt_tokens + completion_tokens;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO cf_usage_logs (
                service_type, model, audio_duration_secs,
                prompt_tokens, completion_tokens, total_tokens,
                neurons_consumed, cost_usd, day_date, month, created_at
            ) VALUES (
                'llm', ?1, 0.0, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%d', 'now'), strftime('%Y-%m', 'now'), CURRENT_TIMESTAMP
            )",
            params![model, prompt_tokens, completion_tokens, total_tokens, neurons, cost],
        )
        .map_err(|e| e.to_string())?;
        Ok((neurons, cost))
    }

    pub fn get_cf_usage_summary(&self) -> Result<CloudflareUsageSummary, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let today_str: String = conn
            .query_row("SELECT strftime('%Y-%m-%d', 'now')", [], |r| r.get(0))
            .unwrap_or_else(|_| "today".to_string());

        let today = conn
            .query_row(
                "SELECT 
                    COALESCE(SUM(neurons_consumed), 0.0),
                    COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(audio_duration_secs), 0.0),
                    COALESCE(SUM(CASE WHEN service_type = 'asr' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(total_tokens), 0),
                    COALESCE(SUM(CASE WHEN service_type = 'llm' THEN 1 ELSE 0 END), 0)
                FROM cf_usage_logs
                WHERE day_date = strftime('%Y-%m-%d', 'now')",
                [],
                |row| {
                    Ok(CloudflareUsagePeriod {
                        label: today_str,
                        total_neurons: row.get(0)?,
                        total_cost_usd: row.get(1)?,
                        asr_audio_secs: row.get(2)?,
                        asr_count: row.get::<_, i64>(3)? as u32,
                        llm_tokens: row.get::<_, i64>(4)? as u32,
                        llm_count: row.get::<_, i64>(5)? as u32,
                    })
                },
            )
            .unwrap_or_default();

        let current_month_str: String = conn
            .query_row("SELECT strftime('%Y-%m', 'now')", [], |r| r.get(0))
            .unwrap_or_else(|_| "current".to_string());

        let mut stmt = conn
            .prepare(
                "SELECT 
                    month,
                    COALESCE(SUM(neurons_consumed), 0.0),
                    COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(audio_duration_secs), 0.0),
                    COALESCE(SUM(CASE WHEN service_type = 'asr' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(total_tokens), 0),
                    COALESCE(SUM(CASE WHEN service_type = 'llm' THEN 1 ELSE 0 END), 0)
                FROM cf_usage_logs
                GROUP BY month
                ORDER BY month DESC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |row| {
                Ok(CloudflareUsagePeriod {
                    label: row.get(0)?,
                    total_neurons: row.get(1)?,
                    total_cost_usd: row.get(2)?,
                    asr_audio_secs: row.get(3)?,
                    asr_count: row.get::<_, i64>(4)? as u32,
                    llm_tokens: row.get::<_, i64>(5)? as u32,
                    llm_count: row.get::<_, i64>(6)? as u32,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut monthly_history = Vec::new();
        let mut current_month = CloudflareUsagePeriod {
            label: current_month_str.clone(),
            ..Default::default()
        };
        let mut all_time = CloudflareUsagePeriod {
            label: "all".to_string(),
            ..Default::default()
        };

        for r in rows {
            let m = r.map_err(|e| e.to_string())?;
            if m.label == current_month_str {
                current_month = m.clone();
            }
            all_time.total_neurons += m.total_neurons;
            all_time.total_cost_usd += m.total_cost_usd;
            all_time.asr_audio_secs += m.asr_audio_secs;
            all_time.asr_count += m.asr_count;
            all_time.llm_tokens += m.llm_tokens;
            all_time.llm_count += m.llm_count;
            monthly_history.push(m);
        }

        Ok(CloudflareUsageSummary {
            today,
            current_month,
            all_time,
            monthly_history,
        })
    }

    pub fn clear_cf_usage_logs(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM cf_usage_logs", [])
            .map_err(|e| e.to_string())?;
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
        assert_eq!(default_settings.mini_record_mode, "new_note");

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
            request_timeout_secs: 240,
            mini_record_mode: "append".into(),
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
        assert_eq!(loaded.request_timeout_secs, 240);
        assert_eq!(loaded.mini_record_mode, "append");
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

    #[test]
    fn test_window_state_persistence() {
        let db = Database::new(":memory:").unwrap();
        assert!(db.get_window_state().is_none());

        let state = WindowState {
            x: Some(150.0),
            y: Some(250.0),
            width: Some(1024.0),
            height: Some(768.0),
            maximized: false,
            mini_x: Some(30.0),
            mini_y: Some(60.0),
        };

        db.save_window_state(&state).unwrap();
        let loaded = db.get_window_state().unwrap();
        assert_eq!(loaded.x, Some(150.0));
        assert_eq!(loaded.y, Some(250.0));
        assert_eq!(loaded.width, Some(1024.0));
        assert_eq!(loaded.height, Some(768.0));
        assert_eq!(loaded.maximized, false);
        assert_eq!(loaded.mini_x, Some(30.0));
        assert_eq!(loaded.mini_y, Some(60.0));
    }

    #[test]
    fn test_cf_usage_tracking() {
        let db = Database::new(":memory:").unwrap();

        // 1. Initial summary should be empty and tiny model should consume 0.0 neurons
        assert_eq!(cf_asr_neurons("@cf/openai/whisper-tiny-en", 120.0), 0.0);
        let initial = db.get_cf_usage_summary().unwrap();
        assert_eq!(initial.all_time.total_neurons, 0.0);
        assert_eq!(initial.all_time.asr_count, 0);
        assert_eq!(initial.all_time.llm_count, 0);

        // 2. Log ASR usage: 60 seconds of @cf/openai/whisper (41.14 neurons)
        db.log_cf_asr_usage("@cf/openai/whisper", 60.0).unwrap();

        // 3. Log ASR usage: 30 seconds of @cf/openai/whisper-large-v3-turbo (23.315 neurons)
        db.log_cf_asr_usage("@cf/openai/whisper-large-v3-turbo", 30.0).unwrap();

        // 4. Log LLM usage: 1000 input tokens, 200 output tokens for llama-3.1-8b
        // input: 1000 * 0.025608 = 25.608; output: 200 * 0.075147 = 15.0294; total: 40.6374
        db.log_cf_llm_usage("@cf/meta/llama-3.1-8b-instruct", 1000, 200).unwrap();

        let summary = db.get_cf_usage_summary().unwrap();
        assert_eq!(summary.all_time.asr_count, 2);
        assert_eq!(summary.all_time.llm_count, 1);
        assert_eq!(summary.all_time.asr_audio_secs, 90.0);
        assert_eq!(summary.all_time.llm_tokens, 1200);
        assert_eq!(summary.today.asr_count, 2);
        assert_eq!(summary.today.llm_count, 1);

        let expected_neurons = 41.14 + (46.63 * 0.5) + (25.608 + 15.0294);
        let diff = (summary.all_time.total_neurons - expected_neurons).abs();
        assert!(diff < 0.01, "expected ~{}, got {}", expected_neurons, summary.all_time.total_neurons);
        assert!((summary.today.total_neurons - expected_neurons).abs() < 0.01);

        assert!(summary.all_time.total_cost_usd > 0.0);
        assert_eq!(summary.current_month.asr_count, 2);
        assert!(!summary.monthly_history.is_empty());

        // 5. Test clear logs
        db.clear_cf_usage_logs().unwrap();
        let cleared = db.get_cf_usage_summary().unwrap();
        assert_eq!(cleared.all_time.total_neurons, 0.0);
        assert_eq!(cleared.all_time.asr_count, 0);
        assert_eq!(cleared.all_time.llm_count, 0);
        assert!(cleared.monthly_history.is_empty());
    }

    #[test]
    fn test_cf_usage_migration_from_old_schema() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("lipi_migration_test_{}.db", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));

        // Create database with legacy schema (no day_date or month columns)
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "
                CREATE TABLE cf_usage_logs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    service_type TEXT NOT NULL,
                    model TEXT NOT NULL,
                    audio_duration_secs REAL NOT NULL DEFAULT 0.0,
                    prompt_tokens INTEGER NOT NULL DEFAULT 0,
                    completion_tokens INTEGER NOT NULL DEFAULT 0,
                    total_tokens INTEGER NOT NULL DEFAULT 0,
                    neurons_consumed REAL NOT NULL DEFAULT 0.0,
                    cost_usd REAL NOT NULL DEFAULT 0.0,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
                );
                INSERT INTO cf_usage_logs (service_type, model, audio_duration_secs, neurons_consumed, cost_usd)
                VALUES ('asr', '@cf/openai/whisper', 60.0, 41.14, 0.00045);
                ",
            ).unwrap();
        }

        // Open via Database::new which must successfully migrate without error
        let db = Database::new(&db_path).unwrap();
        let summary = db.get_cf_usage_summary().unwrap();
        assert_eq!(summary.all_time.asr_count, 1);
        assert_eq!(summary.today.asr_count, 1);
        assert!((summary.all_time.total_neurons - 41.14).abs() < 0.01);

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn test_cf_usage_migration_with_existing_month_column() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("lipi_migration_test2_{}.db", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));

        // Create database with schema that had 'month' and 'idx_cf_usage_month' but NO 'day_date'
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "
                CREATE TABLE cf_usage_logs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    service_type TEXT NOT NULL,
                    model TEXT NOT NULL,
                    audio_duration_secs REAL NOT NULL DEFAULT 0.0,
                    prompt_tokens INTEGER NOT NULL DEFAULT 0,
                    completion_tokens INTEGER NOT NULL DEFAULT 0,
                    total_tokens INTEGER NOT NULL DEFAULT 0,
                    neurons_consumed REAL NOT NULL DEFAULT 0.0,
                    cost_usd REAL NOT NULL DEFAULT 0.0,
                    month TEXT NOT NULL DEFAULT (strftime('%Y-%m', 'now')),
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
                );
                CREATE INDEX idx_cf_usage_month ON cf_usage_logs(month);
                INSERT INTO cf_usage_logs (service_type, model, audio_duration_secs, neurons_consumed, cost_usd, month)
                VALUES ('asr', '@cf/openai/whisper', 120.0, 82.28, 0.0009, strftime('%Y-%m', 'now'));
                ",
            ).unwrap();
        }

        // Open via Database::new which must successfully migrate without error
        let db = Database::new(&db_path).unwrap();
        let summary = db.get_cf_usage_summary().unwrap();
        assert_eq!(summary.all_time.asr_count, 1);
        assert_eq!(summary.today.asr_count, 1);
        assert_eq!(summary.current_month.asr_count, 1);
        assert!((summary.all_time.total_neurons - 82.28).abs() < 0.01);

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn test_user_actual_database_if_exists() {
        if let Ok(home) = std::env::var("HOME") {
            let user_db = std::path::PathBuf::from(home).join(".local/share/com.lipi.app/lipi.db");
            if user_db.exists() {
                let db = Database::new(&user_db).expect("Should successfully open and migrate user's actual database");
                let summary = db.get_cf_usage_summary().expect("Should get summary from user's actual database");
                println!("User actual DB opened & migrated successfully! Summary: {:?}", summary);
            }
        }
    }
}

