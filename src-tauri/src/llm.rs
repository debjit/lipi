use serde_json::Value;

pub fn resolve_system_prompt(preset: &str, custom_prompt: &str) -> String {
    match preset {
        "professional" => {
            "You are an executive communications editor. Rewrite the transcribed speech text into clear, polished, professional business language while preserving all original facts and substance. Do not add commentary, pleasantries, or introductory remarks. Output ONLY the rewritten text.".to_string()
        }
        "casual" => {
            "Rewrite the transcribed speech text into a friendly, relaxed, conversational tone. Smooth out awkward spoken hesitations while keeping the speaker's personality. Do not add commentary or pleasantries. Output ONLY the rewritten text.".to_string()
        }
        "concise" => {
            "Condense the transcribed speech text into a punchy, high-impact summary. Eliminate filler words, redundancies, and rambling. Do not add commentary. Output ONLY the concise text.".to_string()
        }
        "bullets" => {
            "Extract the core points, key details, and action items from the transcribed speech text into a structured Markdown bullet list. Do not add commentary. Output ONLY the bullet points.".to_string()
        }
        "brainstorm" => {
            "Analyze the transcribed speech to uncover what the speaker is truly thinking about and exploring. Identify the central premise, implicit questions, creative angles, key takeaways, and potential next steps or open threads. Organize the thoughts into a clear, structured insight summary with constructive ideas. Do not add conversational fluff.".to_string()
        }
        "custom" => {
            if custom_prompt.trim().is_empty() {
                "You are an editor. Improve the following text. Do not add commentary. Output ONLY the improved text.".to_string()
            } else {
                custom_prompt.trim().to_string()
            }
        }
        _ => {
            // Default: grammar_fix
            "You are an expert copyeditor. Fix all grammatical mistakes, spelling errors, punctuation mistakes, and typos in the transcribed speech text. Strictly preserve the original tone, vocabulary, and meaning. Do not add any conversational remarks, explanations, or quotes. Output ONLY the corrected text.".to_string()
        }
    }
}

pub fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if (trimmed.contains(":11434") || trimmed.to_lowercase().contains("ollama"))
        && !trimmed.ends_with("/v1")
        && !trimmed.ends_with("/chat/completions")
        && !trimmed.ends_with("/models")
    {
        format!("{}/v1", trimmed)
    } else if trimmed.contains("cloudflare.com") && trimmed.ends_with("/ai") {
        format!("{}/v1", trimmed)
    } else {
        trimmed.to_string()
    }
}

pub fn get_curated_fallback_models(base_url: &str) -> Vec<String> {
    if base_url.contains("cloudflare.com") {
        vec![
            "@cf/meta/llama-3.3-70b-instruct-fp8-fast".into(),
            "@cf/meta/llama-3.1-8b-instruct-fp8".into(),
            "@cf/meta/llama-3.1-8b-instruct".into(),
            "@cf/meta/llama-3.2-3b-instruct".into(),
            "@cf/meta/llama-3.2-1b-instruct".into(),
            "@cf/deepseek-ai/deepseek-r1-distill-qwen-32b".into(),
            "@cf/deepseek-ai/deepseek-v4-flash-0731".into(),
            "@cf/qwen/qwen2.5-coder-32b-instruct".into(),
            "@cf/qwen/qwen2.5-7b-instruct".into(),
            "@cf/openai/gpt-oss-120b".into(),
            "@cf/openai/gpt-oss-20b".into(),
            "@cf/mistralai/mistral-small-3.1-24b-instruct".into(),
        ]
    } else if base_url.contains("groq.com") {
        vec![
            "llama-3.3-70b-versatile".into(),
            "llama-3.1-8b-instant".into(),
            "mixtral-8x7b-32768".into(),
        ]
    } else if base_url.contains("openai.com") {
        vec![
            "gpt-4o-mini".into(),
            "gpt-4o".into(),
            "gpt-3.5-turbo".into(),
        ]
    } else if base_url.contains(":11434") || base_url.to_lowercase().contains("ollama") {
        vec![
            "llama3.2".into(),
            "llama3.2:1b".into(),
            "qwen2.5:3b".into(),
            "qwen2.5:7b".into(),
            "phi3:mini".into(),
            "mistral".into(),
        ]
    } else {
        vec![
            "llama3.2".into(),
            "llama3.1".into(),
            "mistral".into(),
            "qwen2.5".into(),
        ]
    }
}

pub fn resolve_models_url(base_url: &str) -> String {
    let normalized = normalize_base_url(base_url);
    let trimmed = normalized.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }

    if trimmed.contains("cloudflare.com") {
        if let Some(pos) = trimmed.find("/accounts/") {
            let after = &trimmed[pos + "/accounts/".len()..];
            let acc = after.split('/').next().unwrap_or("");
            if !acc.is_empty() && acc != "<account_id>" && acc != "{account_id}" {
                return format!(
                    "https://api.cloudflare.com/client/v4/accounts/{}/ai/models/search?per_page=100",
                    acc
                );
            }
        }
    }

    if trimmed.ends_with("/models") {
        trimmed.to_string()
    } else {
        format!("{}/models", trimmed)
    }
}

pub fn format_reqwest_error(e: &reqwest::Error) -> String {
    let mut msg = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        let s_str = s.to_string();
        if !msg.contains(&s_str) {
            msg.push_str(&format!(": {}", s_str));
        }
        source = std::error::Error::source(s);
    }
    msg
}

pub async fn fetch_models(base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let url = resolve_models_url(base_url);
    if url.is_empty() {
        return Err("Endpoint base URL is empty".into());
    }

    let client = reqwest::Client::builder()
        .user_agent(concat!("Lipi/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format_reqwest_error(&e))?;

    let mut req = client.get(&url);
    let trimmed_key = api_key.trim();
    if !trimmed_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", trimmed_key));
    }

    let resp = req.send().await;
    match resp {
        Ok(res) if res.status().is_success() => {
            let body_text = res.text().await.unwrap_or_default();
            let mut model_names = Vec::new();

            if let Ok(json) = serde_json::from_str::<Value>(&body_text) {
                // 1. OpenAI standard: {"data": [{"id": "model-name"}, ...]}
                if let Some(data_arr) = json.get("data").and_then(|d| d.as_array()) {
                    for item in data_arr {
                        if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                            model_names.push(id.to_string());
                        }
                    }
                }

                // 2. Cloudflare / alternative format: {"result": [{"name": "model-name"}, ...]}
                if model_names.is_empty() {
                    if let Some(res_arr) = json.get("result").and_then(|r| r.as_array()) {
                        for item in res_arr {
                            if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                                model_names.push(name.to_string());
                            } else if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                                model_names.push(id.to_string());
                            }
                        }
                    }
                }

                // 3. Ollama native tags format: {"models": [{"name": "model-name"}, ...]}
                if model_names.is_empty() {
                    if let Some(models_arr) = json.get("models").and_then(|m| m.as_array()) {
                        for item in models_arr {
                            if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                                model_names.push(name.to_string());
                            } else if let Some(m) = item.get("model").and_then(|m| m.as_str()) {
                                model_names.push(m.to_string());
                            }
                        }
                    }
                }
            }

            if !model_names.is_empty() {
                model_names.sort();
                model_names.dedup();
                return Ok(model_names);
            }

            // Fallback to curated if returned empty list
            Ok(get_curated_fallback_models(base_url))
        }
        _ => {
            // Secondary attempt for Ollama if /models failed: try native /api/tags
            if (trimmed.contains(":11434") || trimmed.to_lowercase().contains("ollama"))
                && !url.contains("/api/tags")
            {
                let root = trimmed.trim_end_matches("/v1");
                let tags_url = format!("{}/api/tags", root);
                if let Ok(res) = client.get(&tags_url).send().await {
                    if res.status().is_success() {
                        if let Ok(body_text) = res.text().await {
                            if let Ok(json) = serde_json::from_str::<Value>(&body_text) {
                                let mut names = Vec::new();
                                if let Some(arr) = json.get("models").and_then(|m| m.as_array()) {
                                    for item in arr {
                                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                                            names.push(name.to_string());
                                        }
                                    }
                                }
                                if !names.is_empty() {
                                    names.sort();
                                    return Ok(names);
                                }
                            }
                        }
                    }
                }
            }

            // If fetching /models endpoint is not supported by this server or auth failed,
            // provide curated list rather than hard erroring
            Ok(get_curated_fallback_models(base_url))
        }
    }
}

pub async fn transform_text_with_prompt(
    base_url: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_text: &str,
    timeout_secs: u64,
) -> Result<(String, u32, u32), String> {
    let trimmed_text = user_text.trim();
    if trimmed_text.is_empty() {
        return Err("No text provided to transform".into());
    }

    let normalized_base = normalize_base_url(base_url);
    let trimmed_base = normalized_base.trim().trim_end_matches('/');
    if trimmed_base.is_empty() {
        return Err("LLM endpoint URL is missing. Check your settings.".into());
    }

    let endpoint = if trimmed_base.ends_with("/chat/completions") {
        trimmed_base.to_string()
    } else {
        format!("{}/chat/completions", trimmed_base)
    };

    let model_to_use = if model.trim().is_empty() {
        if trimmed_base.contains("cloudflare.com") {
            "@cf/meta/llama-3.1-8b-instruct"
        } else if trimmed_base.contains("groq.com") {
            "llama-3.3-70b-versatile"
        } else if trimmed_base.contains(":11434") || trimmed_base.to_lowercase().contains("ollama") {
            "llama3.2"
        } else {
            "gpt-4o-mini"
        }
    } else {
        model.trim()
    };

    let client = reqwest::Client::builder()
        .user_agent(concat!("Lipi/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(timeout_secs.max(180)))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", format_reqwest_error(&e)))?;

    let payload = serde_json::json!({
        "model": model_to_use,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": trimmed_text }
        ],
        "temperature": 0.2,
        "stream": false
    });

    let mut req = client.post(&endpoint).json(&payload);
    let trimmed_key = api_key.trim();
    if !trimmed_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", trimmed_key));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("LLM request failed to {}: {}", endpoint, format_reqwest_error(&e)))?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|e| format!("Failed reading response: {}", e))?;

    if !status.is_success() {
        let err_msg = if let Ok(json) = serde_json::from_str::<Value>(&body_text) {
            if let Some(msg) = json
                .get("error")
                .and_then(|e| e.get("message").or(Some(e)))
                .and_then(|m| m.as_str())
            {
                msg.to_string()
            } else if let Some(msg) = json
                .get("errors")
                .and_then(|e| e.as_array())
                .and_then(|a| a.first())
                .and_then(|o| o.get("message"))
                .and_then(|m| m.as_str())
            {
                msg.to_string()
            } else {
                body_text.clone()
            }
        } else {
            body_text.clone()
        };

        return Err(format!("LLM Error (HTTP {}): {}", status.as_u16(), err_msg));
    }

    let mut prompt_tokens = 0u32;
    let mut completion_tokens = 0u32;
    let mut response_content: Option<String> = None;

    if let Ok(json) = serde_json::from_str::<Value>(&body_text) {
        if let Some(usage) = json.get("usage") {
            prompt_tokens = usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            completion_tokens = usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        }

        // Standard OpenAI choices[0].message.content
        if let Some(content) = json
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("message"))
            .and_then(|msg| msg.get("content"))
            .and_then(|c| c.as_str())
        {
            response_content = Some(clean_llm_response(content));
        } else if let Some(resp_text) = json
            .get("result")
            .and_then(|r| r.get("response"))
            .and_then(|t| t.as_str())
        {
            // Direct Cloudflare Workers AI fallback format: {"result": {"response": "..."}}
            response_content = Some(clean_llm_response(resp_text));
        }
    }

    let final_text = response_content.unwrap_or_else(|| clean_llm_response(&body_text));

    if prompt_tokens == 0 {
        prompt_tokens = (system_prompt.len() + trimmed_text.len()).div_ceil(4) as u32;
    }
    if completion_tokens == 0 {
        completion_tokens = final_text.len().div_ceil(4) as u32;
    }

    Ok((final_text, prompt_tokens, completion_tokens))
}

fn clean_llm_response(raw: &str) -> String {
    let trimmed = raw.trim();
    // Strip wrapping markdown code blocks if the model erroneously added them
    if trimmed.starts_with("```") && trimmed.ends_with("```") {
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() >= 2 {
            let inner = lines[1..lines.len() - 1].join("\n");
            return inner.trim().to_string();
        }
    }
    trimmed.to_string()
}

#[allow(dead_code)]
pub async fn transform_text(
    base_url: &str,
    api_key: &str,
    model: &str,
    preset: &str,
    custom_prompt: &str,
    user_text: &str,
) -> Result<String, String> {
    let system_prompt = resolve_system_prompt(preset, custom_prompt);
    transform_text_with_prompt(base_url, api_key, model, &system_prompt, user_text, 180)
        .await
        .map(|(t, _, _)| t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompts() {
        assert!(resolve_system_prompt("grammar_fix", "").contains("copyeditor"));
        assert!(resolve_system_prompt("professional", "").contains("business"));
        assert!(resolve_system_prompt("casual", "").contains("conversational"));
        assert!(resolve_system_prompt("concise", "").contains("Condense"));
        assert!(resolve_system_prompt("bullets", "").contains("bullet"));
        assert_eq!(
            resolve_system_prompt("custom", "Make it a poem"),
            "Make it a poem"
        );
    }

    #[test]
    fn test_clean_response() {
        assert_eq!(clean_llm_response("Hello world"), "Hello world");
        assert_eq!(
            clean_llm_response("```markdown\nHello world\n```"),
            "Hello world"
        );
    }

    #[test]
    fn test_fallback_models() {
        let cf = get_curated_fallback_models("https://api.cloudflare.com/client/v4/accounts/1/ai/v1");
        assert!(cf.iter().any(|m| m.contains("llama-3.1")));

        let groq = get_curated_fallback_models("https://api.groq.com/openai/v1");
        assert!(groq.iter().any(|m| m.contains("llama-3.3")));

        let ollama = get_curated_fallback_models("http://localhost:11434/v1");
        assert!(ollama.iter().any(|m| m == "llama3.2"));
        assert!(ollama.iter().any(|m| m == "llama3.2:1b"));
    }

    #[test]
    fn test_normalize_base_url() {
        assert_eq!(
            normalize_base_url("http://localhost:11434"),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            normalize_base_url("http://localhost:11434/v1"),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            normalize_base_url("http://127.0.0.1:11434/"),
            "http://127.0.0.1:11434/v1"
        );
        assert_eq!(
            normalize_base_url("https://api.groq.com/openai/v1"),
            "https://api.groq.com/openai/v1"
        );
        assert_eq!(
            normalize_base_url("https://api.cloudflare.com/client/v4/accounts/c3a0/ai"),
            "https://api.cloudflare.com/client/v4/accounts/c3a0/ai/v1"
        );
        assert_eq!(
            normalize_base_url("https://api.cloudflare.com/client/v4/accounts/c3a0/ai/v1"),
            "https://api.cloudflare.com/client/v4/accounts/c3a0/ai/v1"
        );
    }

    #[test]
    fn test_resolve_models_url() {
        assert_eq!(
            resolve_models_url("https://api.cloudflare.com/client/v4/accounts/my_acc/ai/v1"),
            "https://api.cloudflare.com/client/v4/accounts/my_acc/ai/models/search?per_page=100"
        );
        assert_eq!(
            resolve_models_url("https://api.groq.com/openai/v1"),
            "https://api.groq.com/openai/v1/models"
        );
        assert_eq!(
            resolve_models_url("https://integrate.api.nvidia.com/v1"),
            "https://integrate.api.nvidia.com/v1/models"
        );
    }

    #[test]
    fn test_live_cloudflare_fetch_models() {
        if let Ok(token) = std::env::var("CF_TEST_TOKEN") {
            tauri::async_runtime::block_on(async {
                let base_url = "https://api.cloudflare.com/client/v4/accounts/b67a31aae92890aa15406bbf58d8a8cc/ai/v1";
                let models = fetch_models(base_url, &token).await.expect("Failed to fetch models");
                assert!(models.len() > 20, "Expected > 20 models, got {}", models.len());
                assert!(models.iter().any(|m| m.contains("llama")));
            });
        }
    }
}
