use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn sanitize_id(id: &str) -> String {
    let s: String = id
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect();
    let trimmed = s.trim_matches('_');
    if trimmed.is_empty() {
        "PROVIDER_DEFAULT".to_string()
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlmProvider {
    pub id: String,
    pub name: String,
    pub provider_type: String, // "cloudflare" | "groq" | "openai" | "ollama" | "custom"
    pub api_key: String,
    pub account_id: String,
    pub base_url: String,
}

impl LlmProvider {
    pub fn resolved_base_url(&self) -> String {
        let trimmed = self.base_url.trim();
        if !trimmed.is_empty() {
            return trimmed.trim_end_matches('/').to_string();
        }
        match self.provider_type.to_lowercase().as_str() {
            "cloudflare" => {
                let acc = self.account_id.trim();
                format!("https://api.cloudflare.com/client/v4/accounts/{}/ai/v1", acc)
            }
            "groq" => "https://api.groq.com/openai/v1".to_string(),
            "openai" => "https://api.openai.com/v1".to_string(),
            "ollama" => "http://localhost:11434/v1".to_string(),
            _ => "http://localhost:8000/v1".to_string(),
        }
    }
}

pub use crate::presets::default_presets as default_prompt_presets;
pub use crate::presets::PromptPreset;

pub fn escape_env_val(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace('"', "\\\"")
}

pub fn unescape_env_val(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\\"", "\"").replace("\\\\", "\\")
}

fn default_request_timeout_secs() -> u32 {
    180
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlmSettings {
    pub enabled: bool,
    pub active_provider_id: String,
    pub model: String,
    pub voice_preset: String,
    pub custom_prompt: String,
    pub auto_mode: bool,
    pub providers: Vec<LlmProvider>,
    #[serde(default = "default_prompt_presets")]
    pub presets: Vec<PromptPreset>,
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u32,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            active_provider_id: "CLOUDFLARE_DEFAULT".into(),
            model: "@cf/meta/llama-3.1-8b-instruct".into(),
            voice_preset: "grammar_fix".into(),
            custom_prompt: String::new(),
            auto_mode: false,
            providers: vec![
                LlmProvider {
                    id: "CLOUDFLARE_DEFAULT".into(),
                    name: "Cloudflare Workers AI".into(),
                    provider_type: "cloudflare".into(),
                    api_key: String::new(),
                    account_id: String::new(),
                    base_url: String::new(),
                },
                LlmProvider {
                    id: "GROQ_DEFAULT".into(),
                    name: "Groq Cloud".into(),
                    provider_type: "groq".into(),
                    api_key: String::new(),
                    account_id: String::new(),
                    base_url: "https://api.groq.com/openai/v1".into(),
                },
                LlmProvider {
                    id: "OLLAMA_DEFAULT".into(),
                    name: "Ollama (Local)".into(),
                    provider_type: "ollama".into(),
                    api_key: String::new(),
                    account_id: String::new(),
                    base_url: "http://localhost:11434/v1".into(),
                },
            ],
            presets: default_prompt_presets(),
            request_timeout_secs: 180,
        }
    }
}

pub fn parse_env_file(path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim().to_string();
                let mut val = v.trim().to_string();
                if (val.starts_with('"') && val.ends_with('"'))
                    || (val.starts_with('\'') && val.ends_with('\''))
                {
                    if val.len() >= 2 {
                        val = val[1..val.len() - 1].to_string();
                    }
                }
                let unescaped = unescape_env_val(&val);
                map.insert(key, unescaped);
            }
        }
    }
    map
}

pub fn load_llm_settings(app_data_dir: &Path) -> LlmSettings {
    let env_path = app_data_dir.join(".env");
    let map = parse_env_file(&env_path);

    let mut settings = LlmSettings::default();

    if let Some(val) = map.get("LLM_ENABLED") {
        settings.enabled = val == "true" || val == "1";
    }
    if let Some(val) = map.get("LLM_ACTIVE_PROVIDER_ID") {
        if !val.trim().is_empty() {
            settings.active_provider_id = sanitize_id(val);
        }
    }
    if let Some(val) = map.get("LLM_MODEL") {
        if !val.trim().is_empty() {
            settings.model = val.clone();
        }
    }
    if let Some(val) = map.get("LLM_VOICE_PRESET") {
        if !val.trim().is_empty() {
            settings.voice_preset = val.clone();
        }
    }
    if let Some(val) = map.get("LLM_CUSTOM_PROMPT") {
        settings.custom_prompt = val.clone();
    }
    if let Some(val) = map.get("LLM_AUTO_MODE") {
        settings.auto_mode = val == "true" || val == "1";
    }
    if let Some(val) = map.get("LLM_REQUEST_TIMEOUT_SECS") {
        if let Ok(secs) = val.parse::<u32>() {
            settings.request_timeout_secs = secs.max(180);
        }
    }

    // Load individual provider blocks via LLM_PROVIDER_IDS
    if let Some(ids_str) = map.get("LLM_PROVIDER_IDS") {
        let mut loaded_providers = Vec::new();
        for raw_id in ids_str.split(',') {
            let pid = sanitize_id(raw_id);
            if pid.is_empty() {
                continue;
            }

            let name_key = format!("PROVIDER_{}_NAME", pid);
            let type_key = format!("PROVIDER_{}_TYPE", pid);
            let acc_key = format!("PROVIDER_{}_ACCOUNT_ID", pid);
            let key_key = format!("PROVIDER_{}_API_KEY", pid);
            let url_key = format!("PROVIDER_{}_BASE_URL", pid);

            let provider_type = map.get(&type_key).cloned().unwrap_or_else(|| {
                if pid.contains("CLOUDFLARE") {
                    "cloudflare".into()
                } else if pid.contains("GROQ") {
                    "groq".into()
                } else if pid.contains("OPENAI") {
                    "openai".into()
                } else if pid.contains("OLLAMA") {
                    "ollama".into()
                } else {
                    "custom".into()
                }
            });

            let name = map.get(&name_key).cloned().unwrap_or_else(|| {
                if provider_type == "cloudflare" {
                    "Cloudflare Workers AI".into()
                } else if provider_type == "groq" {
                    "Groq Cloud".into()
                } else if provider_type == "openai" {
                    "OpenAI".into()
                } else if provider_type == "ollama" {
                    "Ollama (Local)".into()
                } else {
                    format!("Provider {}", pid)
                }
            });

            let account_id = map.get(&acc_key).cloned().unwrap_or_default();
            let api_key = map.get(&key_key).cloned().unwrap_or_default();
            let base_url = map.get(&url_key).cloned().unwrap_or_default();

            loaded_providers.push(LlmProvider {
                id: pid,
                name,
                provider_type,
                api_key,
                account_id,
                base_url,
            });
        }

        if !loaded_providers.is_empty() {
            settings.providers = loaded_providers;
        }
    } else {
        // Fallback: Check if legacy LLM_PROVIDERS_JSON exists
        if let Some(json_str) = map.get("LLM_PROVIDERS_JSON") {
            if let Ok(providers) = serde_json::from_str::<Vec<LlmProvider>>(json_str) {
                if !providers.is_empty() {
                    settings.providers = providers
                        .into_iter()
                        .map(|mut p| {
                            p.id = sanitize_id(&p.id);
                            p
                        })
                        .collect();
                }
            }
        }
    }

    // Ensure active_provider_id exists in providers list
    if !settings.providers.iter().any(|p| p.id == settings.active_provider_id) {
        if let Some(first) = settings.providers.first() {
            settings.active_provider_id = first.id.clone();
        }
    }

    // Load individual preset blocks via LLM_PRESET_IDS
    if let Some(ids_str) = map.get("LLM_PRESET_IDS") {
        let mut loaded_presets = Vec::new();
        for raw_id in ids_str.split(',') {
            let pid = sanitize_id(raw_id);
            if pid.is_empty() {
                continue;
            }

            let id_key = format!("PRESET_{}_ID", pid);
            let label_key = format!("PRESET_{}_LABEL", pid);
            let prompt_key = format!("PRESET_{}_PROMPT", pid);

            let id = map.get(&id_key).cloned().unwrap_or_else(|| raw_id.trim().to_string());
            let label = map.get(&label_key).cloned().unwrap_or_else(|| id.clone());
            let prompt = map.get(&prompt_key).cloned().unwrap_or_default();

            loaded_presets.push(PromptPreset { id, label, prompt });
        }
        if !loaded_presets.is_empty() {
            settings.presets = loaded_presets;
        }
    }

    settings
}

pub fn save_llm_settings(app_data_dir: &Path, settings: &LlmSettings) -> Result<(), String> {
    let env_path = app_data_dir.join(".env");

    let active_id = sanitize_id(&settings.active_provider_id);
    let provider_ids: Vec<String> = settings
        .providers
        .iter()
        .map(|p| sanitize_id(&p.id))
        .collect();

    let preset_ids: Vec<String> = settings
        .presets
        .iter()
        .map(|p| sanitize_id(&p.id))
        .collect();

    let mut out = String::new();
    out.push_str("# Lipi LLM Configuration (.env)\n\n");

    out.push_str("# Global LLM Settings\n");
    out.push_str(&format!("LLM_ENABLED={}\n", settings.enabled));
    out.push_str(&format!("LLM_ACTIVE_PROVIDER_ID={}\n", active_id));
    out.push_str(&format!("LLM_MODEL=\"{}\"\n", escape_env_val(&settings.model)));
    out.push_str(&format!("LLM_VOICE_PRESET=\"{}\"\n", escape_env_val(&settings.voice_preset)));
    out.push_str(&format!("LLM_CUSTOM_PROMPT=\"{}\"\n", escape_env_val(&settings.custom_prompt)));
    out.push_str(&format!("LLM_AUTO_MODE={}\n", settings.auto_mode));
    out.push_str(&format!("LLM_REQUEST_TIMEOUT_SECS={}\n", settings.request_timeout_secs.max(180)));
    out.push_str(&format!("LLM_PROVIDER_IDS={}\n", provider_ids.join(",")));
    out.push_str(&format!("LLM_PRESET_IDS={}\n\n", preset_ids.join(",")));

    // Write structured provider blocks with start and end comment demarcations
    for p in &settings.providers {
        let pid = sanitize_id(&p.id);
        out.push_str(&format!("# --- START PROVIDER: {} ({}) ---\n", pid, p.name));
        out.push_str(&format!("PROVIDER_{}_ID={}\n", pid, pid));
        out.push_str(&format!("PROVIDER_{}_NAME=\"{}\"\n", pid, escape_env_val(&p.name)));
        out.push_str(&format!("PROVIDER_{}_TYPE={}\n", pid, p.provider_type));
        out.push_str(&format!("PROVIDER_{}_ACCOUNT_ID=\"{}\"\n", pid, escape_env_val(&p.account_id)));
        out.push_str(&format!("PROVIDER_{}_API_KEY=\"{}\"\n", pid, escape_env_val(&p.api_key)));
        out.push_str(&format!("PROVIDER_{}_BASE_URL=\"{}\"\n", pid, escape_env_val(&p.base_url)));
        out.push_str(&format!("# --- END PROVIDER: {} ---\n\n", pid));
    }

    // Write structured preset blocks with start and end comment demarcations
    for p in &settings.presets {
        let pid = sanitize_id(&p.id);
        out.push_str(&format!("# --- START PRESET: {} ({}) ---\n", pid, p.label));
        out.push_str(&format!("PRESET_{}_ID={}\n", pid, p.id));
        out.push_str(&format!("PRESET_{}_LABEL=\"{}\"\n", pid, escape_env_val(&p.label)));
        out.push_str(&format!("PRESET_{}_PROMPT=\"{}\"\n", pid, escape_env_val(&p.prompt)));
        out.push_str(&format!("# --- END PRESET: {} ---\n\n", pid));
    }

    fs::write(&env_path, out).map_err(|e| format!("Failed writing .env file: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_id() {
        assert_eq!(sanitize_id("cloudflare-default"), "CLOUDFLARE_DEFAULT");
        assert_eq!(sanitize_id("custom 123"), "CUSTOM_123");
        assert_eq!(sanitize_id("groq.cloud!"), "GROQ_CLOUD");
        assert_eq!(sanitize_id(""), "PROVIDER_DEFAULT");
    }

    #[test]
    fn test_save_and_load_llm_settings_blocks() {
        let temp_dir = std::env::temp_dir().join(format!("lipi_env_block_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let mut settings = LlmSettings::default();
        settings.enabled = true;
        settings.active_provider_id = "CUSTOM_OLLAMA".into();
        settings.model = "llama3.2".into();
        settings.auto_mode = true;
        settings.voice_preset = "professional".into();
        settings.providers = vec![
            LlmProvider {
                id: "CLOUDFLARE_DEFAULT".into(),
                name: "Cloudflare Workers AI".into(),
                provider_type: "cloudflare".into(),
                api_key: "tok_cf".into(),
                account_id: "acc_cf".into(),
                base_url: String::new(),
            },
            LlmProvider {
                id: "CUSTOM_OLLAMA".into(),
                name: "My Local Ollama".into(),
                provider_type: "custom".into(),
                api_key: String::new(),
                account_id: String::new(),
                base_url: "http://localhost:11434/v1".into(),
            },
        ];
        settings.request_timeout_secs = 240;

        save_llm_settings(&temp_dir, &settings).unwrap();

        // Check file content has human-readable blocks
        let env_content = fs::read_to_string(temp_dir.join(".env")).unwrap();
        assert!(env_content.contains("LLM_REQUEST_TIMEOUT_SECS=240"));
        assert!(env_content.contains("# --- START PROVIDER: CUSTOM_OLLAMA (My Local Ollama) ---"));
        assert!(env_content.contains("PROVIDER_CUSTOM_OLLAMA_BASE_URL=\"http://localhost:11434/v1\""));
        assert!(env_content.contains("# --- END PROVIDER: CUSTOM_OLLAMA ---"));

        // Load back and verify
        let loaded = load_llm_settings(&temp_dir);
        assert!(loaded.enabled);
        assert_eq!(loaded.request_timeout_secs, 240);
        assert_eq!(loaded.active_provider_id, "CUSTOM_OLLAMA");
        assert_eq!(loaded.model, "llama3.2");
        assert_eq!(loaded.providers.len(), 2);
        assert_eq!(loaded.providers[1].name, "My Local Ollama");
        assert_eq!(loaded.providers[1].base_url, "http://localhost:11434/v1");

        // Now test removing a provider
        let mut settings_removed = loaded.clone();
        settings_removed.providers.retain(|p| p.id != "CUSTOM_OLLAMA");
        settings_removed.active_provider_id = "CLOUDFLARE_DEFAULT".into();

        save_llm_settings(&temp_dir, &settings_removed).unwrap();
        let env_content_removed = fs::read_to_string(temp_dir.join(".env")).unwrap();
        assert!(!env_content_removed.contains("CUSTOM_OLLAMA"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_save_and_load_llm_presets() {
        let temp_dir = std::env::temp_dir().join(format!("lipi_preset_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let mut settings = LlmSettings::default();
        settings.presets.push(PromptPreset {
            id: "bengali_formal".into(),
            label: "🇧🇩 Bengali Formal".into(),
            prompt: "Translate and rewrite in formal Bengali.\nUse polite phrasing.".into(),
        });

        save_llm_settings(&temp_dir, &settings).unwrap();

        let env_content = fs::read_to_string(temp_dir.join(".env")).unwrap();
        assert!(env_content.contains("# --- START PRESET: BENGALI_FORMAL (🇧🇩 Bengali Formal) ---"));
        assert!(env_content.contains("PRESET_BENGALI_FORMAL_ID=bengali_formal"));
        assert!(env_content.contains("PRESET_BENGALI_FORMAL_PROMPT=\"Translate and rewrite in formal Bengali.\\nUse polite phrasing.\""));
        assert!(env_content.contains("# --- END PRESET: BENGALI_FORMAL ---"));

        let loaded = load_llm_settings(&temp_dir);
        let found = loaded.presets.iter().find(|p| p.id == "bengali_formal").expect("bengali_formal preset missing");
        assert_eq!(found.label, "🇧🇩 Bengali Formal");
        assert_eq!(found.prompt, "Translate and rewrite in formal Bengali.\nUse polite phrasing.");

        // Test removing preset
        let mut settings_removed = loaded.clone();
        settings_removed.presets.retain(|p| p.id != "bengali_formal");
        save_llm_settings(&temp_dir, &settings_removed).unwrap();

        let env_removed = fs::read_to_string(temp_dir.join(".env")).unwrap();
        assert!(!env_removed.contains("BENGALI_FORMAL"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
