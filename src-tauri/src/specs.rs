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
}
