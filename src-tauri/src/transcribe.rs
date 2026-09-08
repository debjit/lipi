use reqwest::multipart::{Form, Part};
use serde_json::Value;

pub async fn transcribe_audio(
    wav_bytes: Vec<u8>,
    base_url: &str,
    api_key: &str,
    model: &str,
    language: Option<&str>,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let trimmed_base = base_url.trim().trim_end_matches('/');
    let target_url = if trimmed_base.ends_with("/audio/transcriptions") {
        trimmed_base.to_string()
    } else {
        format!("{}/audio/transcriptions", trimmed_base)
    };

    let file_part = Part::bytes(wav_bytes)
        .file_name("recording.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("Failed to create audio part: {}", e))?;

    let mut form = Form::new()
        .part("file", file_part)
        .text("model", model.trim().to_string())
        .text("response_format", "json");

    if let Some(lang) = language {
        let trimmed_lang = lang.trim();
        if !trimmed_lang.is_empty() {
            form = form.text("language", trimmed_lang.to_string());
        }
    }

    let mut request = client.post(&target_url).multipart(form);

    let trimmed_key = api_key.trim();
    if !trimmed_key.is_empty() {
        request = request.header("Authorization", format!("Bearer {}", trimmed_key));
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Request failed to {}: {}", target_url, e))?;

    let status = response.status();
    let body_text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    if !status.is_success() {
        return Err(format!("API error (HTTP {}): {}", status.as_u16(), body_text));
    }

    // Try parsing as JSON object containing "text" field
    if let Ok(json) = serde_json::from_str::<Value>(&body_text) {
        if let Some(text) = json.get("text").and_then(|t| t.as_str()) {
            return Ok(text.trim().to_string());
        }
    }

    // Fallback if plain text returned
    Ok(body_text.trim().to_string())
}
