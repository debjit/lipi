use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemSpecs {
    pub total_ram_gb: f64,
    pub cpu_cores: usize,
    pub os: String,
    pub recommended_mode: String,       // "local" | "cloud"
    pub recommended_whisper: String,    // "tiny" | "base" | "small"
    pub summary_text: String,
}

pub fn get_total_ram_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("sysctl").arg("-n").arg("hw.memsize").output() {
            if output.status.success() {
                if let Ok(val) = String::from_utf8_lossy(&output.stdout).trim().parse::<u64>() {
                    return val;
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("powershell")
            .arg("-NoProfile")
            .arg("-Command")
            .arg("(Get-CimInstance Win32_OperatingSystem).TotalVisibleMemorySize")
            .output()
        {
            if output.status.success() {
                if let Ok(kb) = String::from_utf8_lossy(&output.stdout).trim().parse::<u64>() {
                    return kb * 1024;
                }
            }
        }
    }

    // Conservative 8 GB default fallback
    8 * 1024 * 1024 * 1024
}

pub fn detect_system_specs() -> SystemSpecs {
    let ram_bytes = get_total_ram_bytes();
    let total_ram_gb = ((ram_bytes as f64) / (1024.0 * 1024.0 * 1024.0) * 10.0).round() / 10.0;
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let os = std::env::consts::OS.to_string();

    let (recommended_mode, recommended_whisper, summary_text) = if total_ram_gb >= 15.0 {
        (
            "local".to_string(),
            "base".to_string(),
            format!(
                "High performance detected ({:.1} GB RAM, {} CPU cores). Local Whisper and local LLMs (via Ollama) will run smoothly offline.",
                total_ram_gb, cpu_cores
            ),
        )
    } else if total_ram_gb >= 7.5 {
        (
            "local".to_string(),
            "base".to_string(),
            format!(
                "Balanced system ({:.1} GB RAM, {} CPU cores). Recommended: Local Whisper for offline voice + fast free Cloud LLM or small local Ollama model.",
                total_ram_gb, cpu_cores
            ),
        )
    } else {
        (
            "cloud".to_string(),
            "tiny".to_string(),
            format!(
                "Lightweight memory ({:.1} GB RAM). Cloud / Hybrid mode recommended for instant response with zero hardware strain.",
                total_ram_gb
            ),
        )
    };

    SystemSpecs {
        total_ram_gb,
        cpu_cores,
        os,
        recommended_mode,
        recommended_whisper,
        summary_text,
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PrerequisiteStatus {
    pub os: String,
    pub has_audio_device: bool,
    pub audio_device_name: Option<String>,
    pub whisper_binary_found: bool,
    pub whisper_binary_path: Option<String>,
    pub python_found: bool,
    pub python_path: Option<String>,
    pub venv_python_found: bool,
    pub ollama_binary_found: bool,
}

pub fn check_system_prerequisites(app_data_dir: &std::path::Path) -> PrerequisiteStatus {
    use cpal::traits::{DeviceTrait, HostTrait};
    let os = std::env::consts::OS.to_string();

    let host = cpal::default_host();
    let (has_audio_device, audio_device_name) = match host.default_input_device() {
        Some(dev) => (true, dev.name().ok()),
        None => (false, None),
    };

    let whisper_bin = crate::models::find_whisper_binary(app_data_dir);
    let whisper_binary_found = whisper_bin.is_some();
    let whisper_binary_path = whisper_bin.map(|p| p.to_string_lossy().to_string());

    let py = crate::models::which_command("python3")
        .or_else(|| crate::models::which_command("python"));
    let python_found = py.is_some();
    let python_path = py.map(|p| p.to_string_lossy().to_string());

    let venv_py = crate::models::find_faster_whisper_python(app_data_dir);
    let venv_python_found = venv_py.is_some();

    let ollama_bin = crate::models::which_command("ollama");
    let ollama_binary_found = ollama_bin.is_some();

    PrerequisiteStatus {
        os,
        has_audio_device,
        audio_device_name,
        whisper_binary_found,
        whisper_binary_path,
        python_found,
        python_path,
        venv_python_found,
        ollama_binary_found,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_system_specs() {
        let specs = detect_system_specs();
        assert!(specs.total_ram_gb > 0.0);
        assert!(specs.cpu_cores > 0);
        assert!(!specs.os.is_empty());
        assert!(!specs.recommended_mode.is_empty());
        assert!(!specs.summary_text.is_empty());
    }

    #[test]
    fn test_check_system_prerequisites() {
        let temp_dir = std::env::temp_dir().join(format!("lipi_prereq_test_{}", std::process::id()));
        let prereqs = check_system_prerequisites(&temp_dir);
        assert!(!prereqs.os.is_empty());
    }
}

