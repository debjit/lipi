use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct PromptPreset {
    pub id: String,
    pub label: String,
    pub prompt: String,
}

pub fn presets_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("presets")
}

pub const GUIDE_CONTENT: &str = r#"# Lipi Presets Guide (.guide.md)

This directory contains voice & tone transformation presets for Lipi.
Each preset is stored as an open-format Markdown (`.md`) file.

> **Note:** Any file starting with a dot `.` (such as `.guide.md`) is guidance documentation and will **not** be listed as a preset in Lipi.

## Preset Format (YAML Frontmatter + Markdown Body)

Each preset file uses standard frontmatter between `---` markers, followed by the system instruction prompt.

```markdown
---
id: 1
label: ✍️ Fix Grammar & Typos
description: Fix typos, spelling, and punctuation while preserving the speaker's original voice
---
You are an expert copyeditor. Fix all grammatical mistakes, spelling errors, punctuation mistakes, and typos in the transcribed speech text. Strictly preserve the original tone, vocabulary, and meaning. Do not add any conversational remarks, explanations, or quotes. Output ONLY the corrected text.
```

### Supported Frontmatter Fields:
- `id`: Unique identifier (e.g. `grammar_fix`, `1`, `professional`). If omitted, the filename without `.md` is used.
- `label` or `name`: Human-readable display label shown in Lipi's preset selector.
- `description`: Optional summary of the preset purpose.

### Body (Instruction Prompt):
The text after the second `---` is the exact system instruction prompt sent to the OpenAI-compatible LLM when transforming your audio transcript.

## Creating Custom Presets
You can create a new `.md` file directly in this folder:
1. Create e.g. `executive.md`, `1.md`, or `bengali.md`.
2. Add the `---` frontmatter header and your prompt instructions.
3. Save the file. Lipi will automatically detect and list it in the workspace and settings.
"#;

pub fn default_presets() -> Vec<PromptPreset> {
    vec![
        PromptPreset {
            id: "grammar_fix".into(),
            label: "✍️ Fix Grammar & Typos".into(),
            prompt: "You are an expert copyeditor. Fix all grammatical mistakes, spelling errors, punctuation mistakes, and typos in the transcribed speech text. Strictly preserve the original tone, vocabulary, and meaning. Do not add any conversational remarks, explanations, or quotes. Output ONLY the corrected text.".into(),
        },
        PromptPreset {
            id: "professional".into(),
            label: "💼 Professional Tone".into(),
            prompt: "You are an executive communications editor. Rewrite the transcribed speech text into clear, polished, professional business language while preserving all original facts and substance. Do not add commentary, pleasantries, or introductory remarks. Output ONLY the rewritten text.".into(),
        },
        PromptPreset {
            id: "casual".into(),
            label: "☕ Casual & Friendly".into(),
            prompt: "Rewrite the transcribed speech text into a friendly, relaxed, conversational tone. Smooth out awkward spoken hesitations while keeping the speaker's personality. Do not add commentary or pleasantries. Output ONLY the rewritten text.".into(),
        },
        PromptPreset {
            id: "concise".into(),
            label: "⚡ Concise Summary".into(),
            prompt: "Condense the transcribed speech text into a punchy, high-impact summary. Eliminate filler words, redundancies, and rambling. Do not add commentary. Output ONLY the concise text.".into(),
        },
        PromptPreset {
            id: "bullets".into(),
            label: "📌 Bullet Points".into(),
            prompt: "Extract the core points, key details, and action items from the transcribed speech text into a structured Markdown bullet list. Do not add commentary. Output ONLY the bullet points.".into(),
        },
    ]
}

pub fn serialize_preset(preset: &PromptPreset) -> String {
    format!(
        "---\nid: {}\nlabel: {}\n---\n\n{}\n",
        preset.id.trim(),
        preset.label.trim(),
        preset.prompt.trim()
    )
}

pub fn parse_preset_file(filename: &str, content: &str) -> PromptPreset {
    let mut id = filename.trim_end_matches(".md").to_string();
    let mut label = id.replace('_', " ");
    let mut prompt = content.trim().to_string();

    let trimmed = content.trim_start();
    if trimmed.starts_with("---") {
        let after_start = &trimmed[3..];
        if let Some(end_pos) = after_start.find("\n---") {
            let frontmatter = &after_start[..end_pos];
            let body = after_start[end_pos + 4..].trim();
            prompt = body.to_string();

            for line in frontmatter.lines() {
                let l = line.trim();
                if let Some((k, v)) = l.split_once(':') {
                    let key = k.trim().to_lowercase();
                    let mut val = v.trim().to_string();
                    if (val.starts_with('"') && val.ends_with('"'))
                        || (val.starts_with('\'') && val.ends_with('\''))
                    {
                        if val.len() >= 2 {
                            val = val[1..val.len() - 1].to_string();
                        }
                    }
                    match key.as_str() {
                        "id" => {
                            if !val.is_empty() {
                                id = val;
                            }
                        }
                        "label" | "name" | "title" => {
                            if !val.is_empty() {
                                label = val;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    } else if let Some(first_line) = content.lines().next() {
        if first_line.starts_with("# ") {
            label = first_line.trim_start_matches("# ").trim().to_string();
            prompt = content[first_line.len()..].trim().to_string();
        }
    }

    PromptPreset { id, label, prompt }
}

pub fn ensure_presets_dir(app_data_dir: &Path) -> Result<(), String> {
    let dir = presets_dir(app_data_dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // Always ensure .guide.md exists
    let guide_path = dir.join(".guide.md");
    if !guide_path.exists() {
        let _ = fs::write(&guide_path, GUIDE_CONTENT);
    }

    // Seed defaults if no .md preset files exist
    let mut has_presets = false;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with('.') && name.ends_with(".md") {
                has_presets = true;
                break;
            }
        }
    }

    if !has_presets {
        for p in default_presets() {
            let file_path = dir.join(format!("{}.md", p.id));
            let _ = fs::write(&file_path, serialize_preset(&p));
        }
    }

    Ok(())
}

pub fn load_presets(app_data_dir: &Path) -> Vec<PromptPreset> {
    let _ = ensure_presets_dir(app_data_dir);
    let dir = presets_dir(app_data_dir);

    let mut presets = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().to_string();
            // Skip hidden files or guide files (anything starting with '.')
            if filename.starts_with('.') || !filename.ends_with(".md") {
                continue;
            }

            if let Ok(content) = fs::read_to_string(&path) {
                presets.push(parse_preset_file(&filename, &content));
            }
        }
    }

    if presets.is_empty() {
        return default_presets();
    }

    // Sort: default IDs first in order, then custom alphabetically
    let order = ["grammar_fix", "professional", "casual", "concise", "bullets"];
    presets.sort_by(|a, b| {
        let pos_a = order.iter().position(|&x| x == a.id);
        let pos_b = order.iter().position(|&x| x == b.id);
        match (pos_a, pos_b) {
            (Some(i), Some(j)) => i.cmp(&j),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.label.cmp(&b.label),
        }
    });

    presets
}

pub fn save_preset_file(app_data_dir: &Path, preset: &PromptPreset) -> Result<(), String> {
    let _ = ensure_presets_dir(app_data_dir);
    let dir = presets_dir(app_data_dir);
    let safe_id = preset.id.trim().replace(['/', '\\', ' '], "_");
    let file_path = dir.join(format!("{}.md", safe_id));
    fs::write(&file_path, serialize_preset(preset)).map_err(|e| e.to_string())
}

pub fn delete_preset_file(app_data_dir: &Path, id: &str) -> Result<(), String> {
    let dir = presets_dir(app_data_dir);
    let safe_id = id.trim().replace(['/', '\\', ' '], "_");
    let file_path = dir.join(format!("{}.md", safe_id));
    if file_path.exists() {
        fs::remove_file(&file_path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn restore_default_presets(app_data_dir: &Path) -> Result<Vec<PromptPreset>, String> {
    let _ = ensure_presets_dir(app_data_dir);
    let dir = presets_dir(app_data_dir);

    // Remove existing non-hidden .md files
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with('.') && name.ends_with(".md") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    // Re-seed defaults
    for p in default_presets() {
        let file_path = dir.join(format!("{}.md", p.id));
        let _ = fs::write(&file_path, serialize_preset(&p));
    }

    Ok(load_presets(app_data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let content = r#"---
id: test_1
label: Test Label
---

This is the system prompt.
Line two of prompt."#;
        let p = parse_preset_file("test_1.md", content);
        assert_eq!(p.id, "test_1");
        assert_eq!(p.label, "Test Label");
        assert_eq!(p.prompt, "This is the system prompt.\nLine two of prompt.");
    }

    #[test]
    fn test_skip_guide_file() {
        let temp_dir = std::env::temp_dir().join(format!("lipi_presets_guide_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let presets = load_presets(&temp_dir);
        // Guide file should exist on disk
        let guide_path = presets_dir(&temp_dir).join(".guide.md");
        assert!(guide_path.exists());

        // Presets should NOT include .guide.md
        assert!(!presets.iter().any(|p| p.id.contains("guide")));

        // Should include default presets
        assert!(presets.iter().any(|p| p.id == "grammar_fix"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_custom_file_and_crud() {
        let temp_dir = std::env::temp_dir().join(format!("lipi_presets_crud_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        // Add 1.md with frontmatter as requested by user
        let p1 = PromptPreset {
            id: "1".into(),
            label: "Custom One".into(),
            prompt: "Transform speech to technical report.".into(),
        };
        save_preset_file(&temp_dir, &p1).unwrap();

        let presets = load_presets(&temp_dir);
        let found = presets.iter().find(|p| p.id == "1").expect("1.md was not loaded");
        assert_eq!(found.label, "Custom One");
        assert_eq!(found.prompt, "Transform speech to technical report.");

        // Delete 1.md
        delete_preset_file(&temp_dir, "1").unwrap();
        let presets_after = load_presets(&temp_dir);
        assert!(!presets_after.iter().any(|p| p.id == "1"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
