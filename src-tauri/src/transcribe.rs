use reqwest::multipart::{Form, Part};
use serde_json::Value;

pub fn resolve_target_url(base_url: &str, model: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');

    // Cloudflare Workers AI Whisper routing
    if trimmed.contains("api.cloudflare.com") {
        if trimmed.contains("/ai/run/") {
            return trimmed.to_string();
        }
        let model_name = if model.trim().is_empty() {
            "@cf/openai/whisper"
        } else {
            model.trim()
        };
        let root = trimmed
            .trim_end_matches("/audio/transcriptions")
            .trim_end_matches("/transcriptions")
            .trim_end_matches("/v1")
            .trim_end_matches('/');
        return format!("{}/run/{}", root, model_name);
    }

    if trimmed.ends_with("/audio/transcriptions")
        || trimmed.ends_with("/transcriptions")
        || trimmed.ends_with("/transcribe")
        || trimmed.ends_with("/inference")
    {
        trimmed.to_string()
    } else {
        format!("{}/audio/transcriptions", trimmed)
    }
}

pub fn base64_encode(bytes: &[u8]) -> String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        out.push(CHARSET[(b0 >> 2) as usize] as char);
        out.push(CHARSET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARSET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARSET[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn requires_cloudflare_json(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower.contains("large-v3") || lower.contains("turbo") || lower.contains("whisper-large")
}

pub async fn transcribe_audio(
    wav_bytes: Vec<u8>,
    base_url: &str,
    api_key: &str,
    model: &str,
    language: Option<&str>,
) -> Result<String, String> {
    let trimmed_base = base_url.trim();
    if trimmed_base.is_empty() {
        return Err("API endpoint URL is missing. Set it in Settings.".into());
    }

    if trimmed_base.contains("<account_id>") || trimmed_base.contains("{account_id}") {
        return Err("Cloudflare URL contains '<account_id>'. Please replace it with your real Cloudflare Account ID in Settings.".into());
    }

    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let target_url = resolve_target_url(trimmed_base, model);
    let is_cloudflare = target_url.contains("api.cloudflare.com");
    let trimmed_key = api_key.trim();

    let (status, body_text) = if is_cloudflare {
        let cloudflare_needs_json = requires_cloudflare_json(model);

        let build_cf_request = |use_json: bool| {
            let mut req = client.post(&target_url);
            if !trimmed_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", trimmed_key));
            }
            if use_json {
                let mut payload = serde_json::json!({
                    "audio": base64_encode(&wav_bytes),
                    "task": "transcribe"
                });
                if let Some(lang) = language {
                    let trimmed_lang = lang.trim();
                    if !trimmed_lang.is_empty() && trimmed_lang != "auto" {
                        payload["language"] = serde_json::json!(trimmed_lang);
                    }
                }
                req.json(&payload)
            } else {
                req.header("Content-Type", "application/octet-stream")
                    .body(wav_bytes.clone())
            }
        };

        let resp = build_cf_request(cloudflare_needs_json)
            .send()
            .await
            .map_err(|e| format!("Request failed to {}: {}", target_url, e))?;

        let mut st = resp.status();
        let mut txt = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        // If octet-stream was rejected by Cloudflare with "Invalid input", transparently retry with base64 JSON
        if !st.is_success() && !cloudflare_needs_json && txt.contains("Invalid input") {
            eprintln!("[Cloudflare] Octet-stream returned 'Invalid input'. Retrying with base64 JSON payload...");
            if let Ok(retry_resp) = build_cf_request(true).send().await {
                st = retry_resp.status();
                if let Ok(retry_txt) = retry_resp.text().await {
                    txt = retry_txt;
                }
            }
        }
        (st, txt)
    } else {
        // OpenAI-compatible multipart form
        let file_part = Part::bytes(wav_bytes)
            .file_name("recording.wav")
            .mime_str("audio/wav")
            .map_err(|e| format!("Failed to create audio part: {}", e))?;

        let mut form = Form::new()
            .part("file", file_part)
            .text("response_format", "json");

        let trimmed_model = model.trim();
        if !trimmed_model.is_empty() {
            form = form.text("model", trimmed_model.to_string());
        }

        if let Some(lang) = language {
            let trimmed_lang = lang.trim();
            if !trimmed_lang.is_empty() && trimmed_lang != "auto" {
                form = form.text("language", trimmed_lang.to_string());
            }
        }

        let mut req = client.post(&target_url).multipart(form);
        if !trimmed_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", trimmed_key));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Request failed to {}: {}", target_url, e))?;

        let st = resp.status();
        let txt = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;
        (st, txt)
    };

    if !status.is_success() {
        let err_summary = if let Ok(json) = serde_json::from_str::<Value>(&body_text) {
            if let Some(msg) = json
                .get("errors")
                .and_then(|e| e.as_array())
                .and_then(|arr| arr.first())
                .and_then(|obj| obj.get("message"))
                .and_then(|m| m.as_str())
            {
                format!("Cloudflare API error: {}", msg)
            } else if let Some(msg) = json
                .get("error")
                .and_then(|e| e.get("message").or(Some(e)))
                .and_then(|m| m.as_str())
            {
                format!("API error: {}", msg)
            } else {
                format!("API error (HTTP {}): {}", status.as_u16(), body_text)
            }
        } else {
            format!("API error (HTTP {}): {}", status.as_u16(), body_text)
        };

        eprintln!(
            "[Transcribe Failed]\n  Summary: {}\n  URL: {}\n  Model: {}\n  Raw: {}",
            err_summary, target_url, model, body_text
        );

        return Err(format!(
            "{}\n[URL: {} | Model: {} | HTTP {}]",
            err_summary,
            target_url,
            if model.trim().is_empty() { "(default)" } else { model.trim() },
            status.as_u16()
        ));
    }

    // Try parsing as JSON object containing "text", "result.text", "transcription", etc.
    if let Ok(json) = serde_json::from_str::<Value>(&body_text) {
        // 1. Direct text field (OpenAI, Groq, Ollama, vLLM)
        if let Some(text) = json.get("text").and_then(|t| t.as_str()) {
            return Ok(text.trim().to_string());
        }
        // 2. Cloudflare Workers AI format: {"result": {"text": "..."}} or {"result": {"transcription_info": {"text": "..."}}}
        if let Some(result_obj) = json.get("result") {
            if let Some(text) = result_obj.get("text").and_then(|t| t.as_str()) {
                return Ok(text.trim().to_string());
            }
            if let Some(text) = result_obj.get("transcription").and_then(|t| t.as_str()) {
                return Ok(text.trim().to_string());
            }
            if let Some(text) = result_obj
                .get("transcription_info")
                .and_then(|i| i.get("text"))
                .and_then(|t| t.as_str())
            {
                return Ok(text.trim().to_string());
            }
            if let Some(text) = result_obj.get("vtt").and_then(|t| t.as_str()) {
                return Ok(text.trim().to_string());
            }
            if let Some(text) = result_obj.as_str() {
                return Ok(text.trim().to_string());
            }
        }
        // 3. Alternative JSON keys
        if let Some(text) = json.get("transcription").and_then(|t| t.as_str()) {
            return Ok(text.trim().to_string());
        }
        if let Some(text) = json.get("output").and_then(|t| t.as_str()) {
            return Ok(text.trim().to_string());
        }
    }

    // Fallback if plain text returned
    Ok(body_text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_url_resolution() {
        assert_eq!(
            resolve_target_url("https://api.openai.com/v1", "whisper-1"),
            "https://api.openai.com/v1/audio/transcriptions"
        );
        assert_eq!(
            resolve_target_url("http://localhost:8000/v1", "whisper-1"),
            "http://localhost:8000/v1/audio/transcriptions"
        );
        assert_eq!(
            resolve_target_url("http://localhost:8080/inference", ""),
            "http://localhost:8080/inference"
        );
    }

    #[test]
    fn test_cloudflare_url_resolution() {
        // User inputs account endpoint with /ai/v1 and @cf/openai/whisper
        assert_eq!(
            resolve_target_url(
                "https://api.cloudflare.com/client/v4/accounts/c3a0b1/ai/v1",
                "@cf/openai/whisper"
            ),
            "https://api.cloudflare.com/client/v4/accounts/c3a0b1/ai/run/@cf/openai/whisper"
        );

        // User inputs account endpoint with /ai/v1 and empty model (defaults to @cf/openai/whisper)
        assert_eq!(
            resolve_target_url(
                "https://api.cloudflare.com/client/v4/accounts/c3a0b1/ai/v1",
                ""
            ),
            "https://api.cloudflare.com/client/v4/accounts/c3a0b1/ai/run/@cf/openai/whisper"
        );

        // User inputs full run path directly
        assert_eq!(
            resolve_target_url(
                "https://api.cloudflare.com/client/v4/accounts/c3a0b1/ai/run/@cf/openai/whisper",
                ""
            ),
            "https://api.cloudflare.com/client/v4/accounts/c3a0b1/ai/run/@cf/openai/whisper"
        );

        // User inputs path ending in /audio/transcriptions
        assert_eq!(
            resolve_target_url(
                "https://api.cloudflare.com/client/v4/accounts/c3a0b1/ai/v1/audio/transcriptions",
                "@cf/openai/whisper"
            ),
            "https://api.cloudflare.com/client/v4/accounts/c3a0b1/ai/run/@cf/openai/whisper"
        );
    }

    #[test]
    fn test_cloudflare_json_parsing() {
        let sample_cf_response = r#"{
            "result": {
                "text": "Hello world from Cloudflare Workers AI"
            },
            "success": true,
            "errors": [],
            "messages": []
        }"#;

        let json: Value = serde_json::from_str(sample_cf_response).unwrap();
        let text = json
            .get("result")
            .and_then(|r| r.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert_eq!(text, "Hello world from Cloudflare Workers AI");
    }

    #[test]
    fn test_cloudflare_turbo_parsing() {
        let sample_turbo_response = r#"{
            "result": {
                "transcription_info": {
                    "text": "Hello world from Cloudflare Large Turbo"
                },
                "word_count": 6
            },
            "success": true
        }"#;

        let json: Value = serde_json::from_str(sample_turbo_response).unwrap();
        let text = json
            .get("result")
            .and_then(|r| r.get("transcription_info"))
            .and_then(|i| i.get("text"))
            .and_then(|t| t.as_str())
            .unwrap();
        assert_eq!(text, "Hello world from Cloudflare Large Turbo");
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn test_requires_cloudflare_json() {
        assert!(requires_cloudflare_json("@cf/openai/whisper-large-v3-turbo"));
        assert!(requires_cloudflare_json("whisper-large-v3-turbo"));
        assert!(requires_cloudflare_json("@cf/openai/whisper-large"));
        assert!(!requires_cloudflare_json("@cf/openai/whisper"));
        assert!(!requires_cloudflare_json("@cf/openai/whisper-tiny-en"));
    }
}
