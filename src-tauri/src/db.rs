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
pub struct MeetingSession {
    pub id: i64,
    pub title: String,
    pub language: String,
    pub started_at: String,
    pub started_epoch: i64,
    pub ended_at: Option<String>,
    pub audio_path: String,
    pub status: String,
    pub summary: String,
    pub decisions: String,
    pub note_id: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MeetingSegment {
    pub id: i64,
    pub session_id: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActionItem {
    pub id: i64,
    pub session_id: i64,
    pub text: String,
    pub due_at: Option<String>,
    pub due_epoch: Option<i64>,
    pub done: bool,
    pub reminded: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AudioCapture {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub bytes: i64,
    pub error: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MeetingDetail {
    pub session: MeetingSession,
    pub segments: Vec<MeetingSegment>,
    pub actions: Vec<ActionItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSettings {
    pub api_base_url: String,
    pub api_key: String,
    pub model: String,
    pub language: Option<String>,
    pub auto_copy: bool,
    pub auto_paste: bool,
    pub always_on_top: bool,
    pub engine_mode: String,
    pub local_engine: String,
    pub local_model_size: String,
    pub models_folder: String,
    pub model_idle_timeout_mins: u32,
    pub request_timeout_secs: u32,
    pub mini_record_mode: String,
    pub shortcut_record_mode: String,
    pub provider_id: Option<String>,
    pub wizard_completed: bool,
    pub live_dictation: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            api_base_url: "https://api.openai.com/v1".into(),
            api_key: String::new(),
            model: "whisper-1".into(),
            language: None,
            auto_copy: true,
            auto_paste: false,
            always_on_top: true,
            engine_mode: "local".into(),
            local_engine: "whisper_cpu".into(),
            local_model_size: "base".into(),
            models_folder: String::new(),
            model_idle_timeout_mins: 10,
            request_timeout_secs: 180,
            mini_record_mode: "new_note".into(),
            shortcut_record_mode: "new_note".into(),
            provider_id: None,
            wizard_completed: false,
            live_dictation: false,
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
                day_date TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d', 'now', 'localtime')),
                month TEXT NOT NULL DEFAULT (strftime('%Y-%m', 'now', 'localtime')),
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
            "UPDATE cf_usage_logs SET day_date = strftime('%Y-%m-%d', created_at, 'localtime') WHERE day_date IS NULL OR day_date = ''",
            [],
        );
        let _ = conn.execute(
            "UPDATE cf_usage_logs SET month = strftime('%Y-%m', created_at, 'localtime') WHERE month IS NULL OR month = ''",
            [],
        );

        // Indexes must only be created AFTER columns are guaranteed to exist
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_cf_usage_day ON cf_usage_logs(day_date);
            CREATE INDEX IF NOT EXISTS idx_cf_usage_month ON cf_usage_logs(month);
            CREATE TABLE IF NOT EXISTS sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL DEFAULT 'Meeting',
                language TEXT NOT NULL DEFAULT '',
                started_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                started_epoch INTEGER NOT NULL DEFAULT 0,
                ended_at DATETIME,
                audio_path TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'recording',
                summary TEXT NOT NULL DEFAULT '',
                decisions TEXT NOT NULL DEFAULT '[]',
                note_id INTEGER
            );
            CREATE TABLE IF NOT EXISTS segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id INTEGER NOT NULL,
                start_ms INTEGER NOT NULL,
                end_ms INTEGER NOT NULL,
                text TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS action_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id INTEGER NOT NULL,
                text TEXT NOT NULL,
                due_at TEXT,
                due_epoch INTEGER,
                done INTEGER NOT NULL DEFAULT 0,
                reminded INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_segments_session ON segments(session_id);
            CREATE INDEX IF NOT EXISTS idx_actions_session ON action_items(session_id);
            CREATE INDEX IF NOT EXISTS idx_actions_due ON action_items(due_epoch);
            CREATE TABLE IF NOT EXISTS audio_captures (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                path TEXT NOT NULL,
                bytes INTEGER NOT NULL,
                error TEXT NOT NULL DEFAULT '',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            ",
        )?;

        // A previous run may have stopped while the mic or the summary was still open.
        let _ = conn.execute(
            "UPDATE sessions SET status = 'interrupted' WHERE status IN ('recording', 'summarizing')",
            [],
        );

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

    pub fn clear_all_notes(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM notes", [])
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
            auto_paste: get_val("auto_paste").map(|v| v == "true" || v == "1").unwrap_or(false),
            always_on_top: get_val("always_on_top").map(|v| v != "false" && v != "0").unwrap_or(true),
            engine_mode: get_val("engine_mode").unwrap_or_else(|| "local".into()),
            local_engine: get_val("local_engine").unwrap_or_else(|| "whisper_cpu".into()),
            local_model_size: get_val("local_model_size").unwrap_or_else(|| "base".into()),
            models_folder: get_val("models_folder").unwrap_or_default(),
            model_idle_timeout_mins: get_val("model_idle_timeout_mins").and_then(|v| v.parse().ok()).unwrap_or(10),
            request_timeout_secs: get_val("request_timeout_secs").and_then(|v| v.parse().ok()).unwrap_or(180).max(180),
            mini_record_mode: get_val("mini_record_mode").unwrap_or_else(|| "new_note".into()),
            shortcut_record_mode: get_val("shortcut_record_mode").unwrap_or_else(|| "new_note".into()),
            provider_id: get_val("provider_id").filter(|s| !s.is_empty()),
            wizard_completed: get_val("wizard_completed").map(|v| v == "true" || v == "1").unwrap_or(false),
            live_dictation: get_val("live_dictation").map(|v| v == "true" || v == "1").unwrap_or(false),
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
        set_val("auto_paste", if settings.auto_paste { "true" } else { "false" }).map_err(|e| e.to_string())?;
        set_val("always_on_top", if settings.always_on_top { "true" } else { "false" }).map_err(|e| e.to_string())?;
        set_val("engine_mode", &settings.engine_mode).map_err(|e| e.to_string())?;
        set_val("local_engine", &settings.local_engine).map_err(|e| e.to_string())?;
        set_val("local_model_size", &settings.local_model_size).map_err(|e| e.to_string())?;
        set_val("models_folder", &settings.models_folder).map_err(|e| e.to_string())?;
        set_val("model_idle_timeout_mins", &settings.model_idle_timeout_mins.to_string()).map_err(|e| e.to_string())?;
        set_val("request_timeout_secs", &settings.request_timeout_secs.max(180).to_string()).map_err(|e| e.to_string())?;
        set_val("mini_record_mode", &settings.mini_record_mode).map_err(|e| e.to_string())?;
        set_val("shortcut_record_mode", &settings.shortcut_record_mode).map_err(|e| e.to_string())?;
        set_val("provider_id", settings.provider_id.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
        set_val("wizard_completed", if settings.wizard_completed { "true" } else { "false" }).map_err(|e| e.to_string())?;
        set_val("live_dictation", if settings.live_dictation { "true" } else { "false" }).map_err(|e| e.to_string())?;
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
                'asr', ?1, ?2, 0, 0, 0, ?3, ?4, strftime('%Y-%m-%d', 'now', 'localtime'), strftime('%Y-%m', 'now', 'localtime'), CURRENT_TIMESTAMP
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
                'llm', ?1, 0.0, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%d', 'now', 'localtime'), strftime('%Y-%m', 'now', 'localtime'), CURRENT_TIMESTAMP
            )",
            params![model, prompt_tokens, completion_tokens, total_tokens, neurons, cost],
        )
        .map_err(|e| e.to_string())?;
        Ok((neurons, cost))
    }

    pub fn get_cf_usage_summary(&self) -> Result<CloudflareUsageSummary, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let today_str: String = conn
            .query_row("SELECT strftime('%Y-%m-%d', 'now', 'localtime')", [], |r| r.get(0))
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
                WHERE day_date = strftime('%Y-%m-%d', 'now', 'localtime')",
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
            .query_row("SELECT strftime('%Y-%m', 'now', 'localtime')", [], |r| r.get(0))
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

    pub fn create_session(&self, language: &str, started_epoch: i64) -> Result<MeetingSession, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO sessions (title, language, started_epoch, status) VALUES ('Meeting', ?1, ?2, 'recording')",
            params![language, started_epoch],
        )
        .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        drop(conn);
        self.get_session(id)
    }

    pub fn set_session_audio_path(&self, id: i64, path: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sessions SET audio_path = ?1 WHERE id = ?2",
            params![path, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_session_status_if(&self, id: i64, from: &str, to: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sessions SET status = ?1 WHERE id = ?2 AND status = ?3",
            params![to, id, from],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_session_status(&self, id: i64, status: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sessions SET status = ?1 WHERE id = ?2",
            params![status, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Moves a session into summarizing. Returns false when another caller already claimed it.
    pub fn claim_session_summary(&self, id: i64) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let changed = conn
            .execute(
                "UPDATE sessions SET status = 'summarizing' WHERE id = ?1 AND status IN ('recording', 'interrupted')",
                params![id],
            )
            .map_err(|e| e.to_string())?;
        Ok(changed == 1)
    }

    pub fn finish_session(
        &self,
        id: i64,
        title: &str,
        summary: &str,
        decisions: &str,
        note_id: Option<i64>,
        status: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sessions SET title = ?1, summary = ?2, decisions = ?3, note_id = ?4, status = ?5, ended_at = CURRENT_TIMESTAMP WHERE id = ?6",
            params![title, summary, decisions, note_id, status, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn map_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingSession> {
        Ok(MeetingSession {
            id: row.get(0)?,
            title: row.get(1)?,
            language: row.get(2)?,
            started_at: row.get(3)?,
            started_epoch: row.get(4)?,
            ended_at: row.get(5)?,
            audio_path: row.get(6)?,
            status: row.get(7)?,
            summary: row.get(8)?,
            decisions: row.get(9)?,
            note_id: row.get(10)?,
        })
    }

    const SESSION_COLS: &'static str =
        "id, title, language, started_at, started_epoch, ended_at, audio_path, status, summary, decisions, note_id";

    pub fn get_session(&self, id: i64) -> Result<MeetingSession, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "SELECT {} FROM sessions WHERE id = ?1",
            Self::SESSION_COLS
        );
        conn.query_row(&sql, params![id], Self::map_session)
            .map_err(|e| e.to_string())
    }

    pub fn list_sessions(&self) -> Result<Vec<MeetingSession>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "SELECT {} FROM sessions ORDER BY id DESC",
            Self::SESSION_COLS
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], Self::map_session)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn insert_segment(
        &self,
        session_id: i64,
        start_ms: i64,
        end_ms: i64,
        text: &str,
    ) -> Result<MeetingSegment, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO segments (session_id, start_ms, end_ms, text) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, start_ms, end_ms, text],
        )
        .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        Ok(MeetingSegment {
            id,
            session_id,
            start_ms,
            end_ms,
            text: text.to_string(),
        })
    }

    pub fn segments_for(&self, session_id: i64) -> Result<Vec<MeetingSegment>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, start_ms, end_ms, text FROM segments WHERE session_id = ?1 ORDER BY start_ms, id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(MeetingSegment {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    start_ms: row.get(2)?,
                    end_ms: row.get(3)?,
                    text: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn replace_action_items(
        &self,
        session_id: i64,
        items: &[(String, Option<String>, Option<i64>)],
    ) -> Result<Vec<ActionItem>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM action_items WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(|e| e.to_string())?;
        for (text, due_at, due_epoch) in items {
            conn.execute(
                "INSERT INTO action_items (session_id, text, due_at, due_epoch, done, reminded) VALUES (?1, ?2, ?3, ?4, 0, 0)",
                params![session_id, text, due_at, due_epoch],
            )
            .map_err(|e| e.to_string())?;
        }
        drop(conn);
        self.actions_for(session_id)
    }

    pub fn actions_for(&self, session_id: i64) -> Result<Vec<ActionItem>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, text, due_at, due_epoch, done, reminded FROM action_items WHERE session_id = ?1 ORDER BY id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![session_id], Self::map_action)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    fn map_action(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActionItem> {
        let done: i64 = row.get(5)?;
        let reminded: i64 = row.get(6)?;
        Ok(ActionItem {
            id: row.get(0)?,
            session_id: row.get(1)?,
            text: row.get(2)?,
            due_at: row.get(3)?,
            due_epoch: row.get(4)?,
            done: done != 0,
            reminded: reminded != 0,
        })
    }

    pub fn meeting_detail(&self, id: i64) -> Result<MeetingDetail, String> {
        Ok(MeetingDetail {
            session: self.get_session(id)?,
            segments: self.segments_for(id)?,
            actions: self.actions_for(id)?,
        })
    }

    pub fn set_action_done(&self, id: i64, done: bool) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let flag = if done { 1 } else { 0 };
        conn.execute(
            "UPDATE action_items SET done = ?1, reminded = CASE WHEN ?1 = 1 THEN 1 ELSE reminded END WHERE id = ?2",
            params![flag, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn due_action_items(&self, now_epoch: i64) -> Result<Vec<ActionItem>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, text, due_at, due_epoch, done, reminded FROM action_items WHERE done = 0 AND reminded = 0 AND due_epoch IS NOT NULL AND due_epoch <= ?1 ORDER BY due_epoch",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![now_epoch], Self::map_action)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn mark_action_reminded(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE action_items SET reminded = 1 WHERE id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_session(&self, id: i64) -> Result<MeetingSession, String> {
        let session = self.get_session(id)?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM segments WHERE session_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM action_items WHERE session_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(session)
    }

    pub fn insert_audio_capture(
        &self,
        name: &str,
        path: &str,
        bytes: i64,
        error: &str,
    ) -> Result<AudioCapture, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO audio_captures (name, path, bytes, error) VALUES (?1, ?2, ?3, ?4)",
            params![name, path, bytes, error],
        )
        .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        drop(conn);
        self.get_audio_capture(id)
    }

    pub fn get_audio_capture(&self, id: i64) -> Result<AudioCapture, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT id, name, path, bytes, error, created_at FROM audio_captures WHERE id = ?1",
            params![id],
            |row| {
                Ok(AudioCapture {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    bytes: row.get(3)?,
                    error: row.get(4)?,
                    created_at: row.get(5)?,
                })
            },
        )
        .map_err(|e| e.to_string())
    }

    pub fn list_audio_captures(&self) -> Result<Vec<AudioCapture>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, name, path, bytes, error, created_at FROM audio_captures ORDER BY id DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AudioCapture {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    bytes: row.get(3)?,
                    error: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn update_audio_capture_error(&self, id: i64, error: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE audio_captures SET error = ?1 WHERE id = ?2",
            params![error, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_audio_capture(&self, id: i64) -> Result<AudioCapture, String> {
        let capture = self.get_audio_capture(id)?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM audio_captures WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(capture)
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
        assert_eq!(default_settings.auto_paste, false);
        assert_eq!(default_settings.always_on_top, true);
        assert_eq!(default_settings.engine_mode, "local");
        assert_eq!(default_settings.local_engine, "whisper_cpu");
        assert_eq!(default_settings.local_model_size, "base");
        assert_eq!(default_settings.models_folder, "");
        assert_eq!(default_settings.model_idle_timeout_mins, 10);
        assert_eq!(default_settings.mini_record_mode, "new_note");
        assert_eq!(default_settings.shortcut_record_mode, "new_note");

        let new_settings = AppSettings {
            api_base_url: "https://api.groq.com/openai/v1".into(),
            api_key: "test_key".into(),
            model: "whisper-large-v3".into(),
            language: Some("en".into()),
            auto_copy: false,
            auto_paste: true,
            always_on_top: false,
            engine_mode: "local".into(),
            local_engine: "whisper_vulkan".into(),
            local_model_size: "tiny".into(),
            models_folder: "/external/models".into(),
            model_idle_timeout_mins: 5,
            request_timeout_secs: 240,
            mini_record_mode: "append".into(),
            shortcut_record_mode: "append".into(),
            provider_id: Some("GROQ_DEFAULT".into()),
            wizard_completed: true,
            live_dictation: true,
        };
        db.save_settings(&new_settings).unwrap();
        let loaded = db.get_settings().unwrap();
        assert_eq!(loaded.api_base_url, "https://api.groq.com/openai/v1");
        assert_eq!(loaded.api_key, "test_key");
        assert_eq!(loaded.model, "whisper-large-v3");
        assert_eq!(loaded.language, Some("en".into()));
        assert_eq!(loaded.auto_copy, false);
        assert_eq!(loaded.auto_paste, true);
        assert_eq!(loaded.always_on_top, false);
        assert_eq!(loaded.engine_mode, "local");
        assert_eq!(loaded.local_engine, "whisper_vulkan");
        assert_eq!(loaded.local_model_size, "tiny");
        assert_eq!(loaded.models_folder, "/external/models");
        assert_eq!(loaded.model_idle_timeout_mins, 5);
        assert_eq!(loaded.request_timeout_secs, 240);
        assert_eq!(loaded.mini_record_mode, "append");
        assert_eq!(loaded.shortcut_record_mode, "append");
        assert_eq!(loaded.provider_id, Some("GROQ_DEFAULT".into()));
        assert_eq!(loaded.wizard_completed, true);
        assert_eq!(loaded.live_dictation, true);
        assert!(!default_settings.live_dictation);
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
    fn test_meeting_sessions_segments_and_actions() {
        let db = Database::new(":memory:").unwrap();
        let session = db.create_session("bn", 1_700_000_000).unwrap();
        assert_eq!(session.status, "recording");
        assert_eq!(session.language, "bn");

        db.set_session_audio_path(session.id, "/tmp/audio.wav").unwrap();
        let seg = db.insert_segment(session.id, 0, 1500, "hello").unwrap();
        assert_eq!(seg.text, "hello");

        let actions = db
            .replace_action_items(
                session.id,
                &[("Send the notes".into(), Some("2026-10-18T09:00:00Z".into()), Some(1_700_000_100))],
            )
            .unwrap();
        assert_eq!(actions.len(), 1);
        assert!(!actions[0].done);

        let due = db.due_action_items(1_700_000_100).unwrap();
        assert_eq!(due.len(), 1);
        db.mark_action_reminded(actions[0].id).unwrap();
        assert!(db.due_action_items(1_700_000_100).unwrap().is_empty());

        assert!(db.claim_session_summary(session.id).unwrap());
        assert!(!db.claim_session_summary(session.id).unwrap());
        db.finish_session(session.id, "Standup", "Talked", "[]", Some(4), "ready")
            .unwrap();
        let detail = db.meeting_detail(session.id).unwrap();
        assert_eq!(detail.session.title, "Standup");
        assert_eq!(detail.session.note_id, Some(4));
        assert_eq!(detail.segments.len(), 1);
        assert_eq!(detail.session.status, "ready");

        let removed = db.delete_session(session.id).unwrap();
        assert_eq!(removed.audio_path, "/tmp/audio.wav");
        assert!(db.get_session(session.id).is_err());
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

