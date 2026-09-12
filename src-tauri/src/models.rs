use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::Emitter;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelStatus {
    pub engine: String,
    pub model_size: String,
    pub installed: bool,
    pub file_path: String,
    pub file_size_bytes: u64,
    pub binary_available: bool,
    pub binary_path: String,
    pub models_dir: String,
}

pub fn resolve_models_dir(app_data_dir: &Path, custom_dir: Option<&str>) -> PathBuf {
    if let Some(custom) = custom_dir {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            if let Some(stripped) = trimmed.strip_prefix("~/") {
                if let Ok(home) = std::env::var("HOME") {
                    return PathBuf::from(home).join(stripped);
                }
            }
            return PathBuf::from(trimmed);
        }
    }
    app_data_dir.join("models")
}

#[allow(dead_code)]
pub fn get_models_dir(app_data_dir: &Path) -> PathBuf {
    resolve_models_dir(app_data_dir, None)
}

pub fn get_bin_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("bin")
}

pub fn get_model_filename(engine: &str, model_size: &str) -> String {
    let size = match model_size.to_lowercase().as_str() {
        "tiny" => "tiny",
        "small" => "small",
        _ => "base",
    };
    if engine == "faster_whisper" {
        format!("faster-whisper-{}", size)
    } else {
        format!("ggml-{}.bin", size)
    }
}

pub fn get_ggml_download_url(model_size: &str) -> Result<&'static str, String> {
    match model_size.to_lowercase().as_str() {
        "tiny" => Ok("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin"),
        "base" => Ok("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"),
        "small" => Ok("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"),
        _ => Err(format!("Unsupported model size: {}", model_size)),
    }
}

pub fn find_whisper_binary(app_data_dir: &Path) -> Option<PathBuf> {
    let bin_dir = get_bin_dir(app_data_dir);
    let candidates = [
        bin_dir.join("whisper-cli"),
        bin_dir.join("whisper-cli.exe"),
        bin_dir.join("main"),
        bin_dir.join("main.exe"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    // Check system PATH
    for name in &["whisper-cli", "whisper"] {
        if let Ok(path) = which::which(name) {
            return Some(path);
        }
    }

    None
}

pub fn find_whisper_server_binary(app_data_dir: &Path) -> Option<PathBuf> {
    let bin_dir = get_bin_dir(app_data_dir);
    let candidates = [
        bin_dir.join("whisper-server"),
        bin_dir.join("whisper-server.exe"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    if let Ok(path) = which::which("whisper-server") {
        return Some(path);
    }

    None
}

// Fallback search using std::process::Command "which" or "where"
mod which {
    use std::path::PathBuf;
    use std::process::Command;

    pub fn which(name: &str) -> Result<PathBuf, ()> {
        let cmd = if cfg!(windows) { "where" } else { "which" };
        let output = Command::new(cmd).arg(name).output().map_err(|_| ())?;
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout);
            let first_line = path_str.lines().next().unwrap_or("").trim();
            if !first_line.is_empty() {
                let p = PathBuf::from(first_line);
                if p.exists() {
                    return Ok(p);
                }
            }
        }
        Err(())
    }
}

pub fn which_command(name: &str) -> Option<PathBuf> {
    which::which(name).ok()
}

pub fn find_faster_whisper_python(app_data_dir: &Path) -> Option<PathBuf> {
    let venv_py = if cfg!(windows) {
        app_data_dir.join("venv").join("Scripts").join("python.exe")
    } else {
        app_data_dir.join("venv").join("bin").join("python3")
    };

    if venv_py.exists() {
        let check = std::process::Command::new(&venv_py)
            .arg("-c")
            .arg("import faster_whisper")
            .output();
        if let Ok(out) = check {
            if out.status.success() {
                return Some(venv_py);
            }
        }
    }

    // Check system python3
    let sys_check = std::process::Command::new("python3")
        .arg("-c")
        .arg("import faster_whisper")
        .output();
    if let Ok(out) = sys_check {
        if out.status.success() {
            return Some(PathBuf::from("python3"));
        }
    }

    None
}

pub fn install_faster_whisper_deps(app_data_dir: &Path) -> Result<String, String> {
    let venv_dir = app_data_dir.join("venv");
    if !venv_dir.exists() {
        let venv_status = std::process::Command::new("python3")
            .arg("-m")
            .arg("venv")
            .arg(&venv_dir)
            .status()
            .map_err(|e| format!("Failed to create virtual environment: {}", e))?;
        if !venv_status.success() {
            return Err("Failed to create virtual environment with 'python3 -m venv'".into());
        }
    }

    let pip_exe = if cfg!(windows) {
        venv_dir.join("Scripts").join("pip.exe")
    } else {
        venv_dir.join("bin").join("pip")
    };

    let pip_output = std::process::Command::new(pip_exe)
        .arg("install")
        .arg("faster-whisper")
        .output()
        .map_err(|e| format!("Failed to run pip install: {}", e))?;

    if !pip_output.status.success() {
        let err = String::from_utf8_lossy(&pip_output.stderr);
        return Err(format!("pip install faster-whisper failed: {}", err.trim()));
    }

    Ok("faster-whisper installed successfully in application environment".into())
}

pub fn check_model_status(
    app_data_dir: &Path,
    custom_dir: Option<&str>,
    engine: &str,
    model_size: &str,
) -> Result<ModelStatus, String> {
    let models_dir = resolve_models_dir(app_data_dir, custom_dir);
    let filename = get_model_filename(engine, model_size);
    let target_path = models_dir.join(&filename);

    let (installed, size_bytes) = if engine == "faster_whisper" {
        let model_bin = target_path.join("model.bin");
        if model_bin.is_file() {
            let metadata = fs::metadata(&model_bin).map_err(|e| e.to_string())?;
            (metadata.len() > 1000, metadata.len())
        } else {
            (false, 0)
        }
    } else if target_path.is_file() {
        let metadata = fs::metadata(&target_path).map_err(|e| e.to_string())?;
        (metadata.len() > 1000, metadata.len())
    } else {
        (false, 0)
    };

    let (binary_available, binary_path) = if engine == "faster_whisper" {
        match find_faster_whisper_python(app_data_dir) {
            Some(p) => (true, p.to_string_lossy().to_string()),
            None => (false, "faster-whisper python module not found".into()),
        }
    } else {
        let b = find_whisper_binary(app_data_dir);
        let avail = b.is_some();
        let path = b.map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        (avail, path)
    };

    Ok(ModelStatus {
        engine: engine.to_string(),
        model_size: model_size.to_string(),
        installed,
        file_path: target_path.to_string_lossy().to_string(),
        file_size_bytes: size_bytes,
        binary_available,
        binary_path,
        models_dir: models_dir.to_string_lossy().to_string(),
    })
}

pub async fn ensure_whisper_cli_binary(app_data_dir: &Path) -> Result<PathBuf, String> {
    if let Some(path) = find_whisper_binary(app_data_dir) {
        return Ok(path);
    }

    let bin_dir = get_bin_dir(app_data_dir);
    fs::create_dir_all(&bin_dir).map_err(|e| format!("Failed to create bin dir: {}", e))?;

    // Download official whisper-bin-ubuntu-x64 release on Linux x86_64
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let tar_url = "https://github.com/ggml-org/whisper.cpp/releases/download/b4938/whisper-bin-ubuntu-x64.tar.gz";
        let client = reqwest::Client::new();
        let resp = client
            .get(tar_url)
            .header("User-Agent", "lipi")
            .send()
            .await
            .map_err(|e| format!("Failed to download whisper binary archive: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Download failed with status: {}", resp.status()));
        }

        let archive_bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read archive bytes: {}", e))?;

        let tar_gz_path = bin_dir.join("whisper-bin.tar.gz");
        fs::write(&tar_gz_path, &archive_bytes)
            .map_err(|e| format!("Failed to write archive to disk: {}", e))?;

        let status = std::process::Command::new("tar")
            .arg("-xzf")
            .arg(&tar_gz_path)
            .arg("--strip-components=1")
            .arg("-C")
            .arg(&bin_dir)
            .status()
            .map_err(|e| format!("Failed to extract archive: {}", e))?;

        let _ = fs::remove_file(&tar_gz_path);

        if !status.success() {
            return Err("Failed to unpack whisper binary archive".into());
        }

        let binary_path = bin_dir.join("whisper-cli");
        if binary_path.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&binary_path, fs::Permissions::from_mode(0o755));
            }
            return Ok(binary_path);
        }
    }

    // Download official whisper-bin-x64 release on Windows x86_64
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        let zip_url = "https://github.com/ggml-org/whisper.cpp/releases/download/b4938/whisper-bin-x64.zip";
        let client = reqwest::Client::new();
        let resp = client
            .get(zip_url)
            .header("User-Agent", "lipi")
            .send()
            .await
            .map_err(|e| format!("Failed to download whisper binary archive: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Download failed with status: {}", resp.status()));
        }

        let archive_bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read archive bytes: {}", e))?;

        let zip_path = bin_dir.join("whisper-bin.zip");
        fs::write(&zip_path, &archive_bytes)
            .map_err(|e| format!("Failed to write archive to disk: {}", e))?;

        // Windows 10+ includes tar.exe which extracts .zip files natively
        let mut extracted = false;
        if let Ok(status) = std::process::Command::new("tar")
            .arg("-xf")
            .arg(&zip_path)
            .arg("-C")
            .arg(&bin_dir)
            .status()
        {
            if status.success() {
                extracted = true;
            }
        }

        if !extracted {
            let ps_script = format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                zip_path.to_string_lossy(),
                bin_dir.to_string_lossy()
            );
            let _ = std::process::Command::new("powershell")
                .arg("-NoProfile")
                .arg("-Command")
                .arg(&ps_script)
                .status();
        }

        let _ = fs::remove_file(&zip_path);

        if let Some(p) = find_whisper_binary(app_data_dir) {
            return Ok(p);
        }
    }

    Err("whisper-cli binary not found. Please install whisper-cli or ensure it is in PATH.".into())
}

pub async fn ensure_whisper_server_binary(app_data_dir: &Path) -> Result<PathBuf, String> {
    if let Some(path) = find_whisper_server_binary(app_data_dir) {
        return Ok(path);
    }
    let _ = ensure_whisper_cli_binary(app_data_dir).await?;
    find_whisper_server_binary(app_data_dir)
        .ok_or_else(|| "whisper-server binary not found in bin directory".into())
}

async fn download_faster_whisper_weights(
    app_handle: &tauri::AppHandle,
    models_dir: &Path,
    model_size: &str,
) -> Result<String, String> {
    let size = match model_size.to_lowercase().as_str() {
        "tiny" => "tiny",
        "small" => "small",
        _ => "base",
    };
    let model_folder = models_dir.join(format!("faster-whisper-{}", size));
    fs::create_dir_all(&model_folder).map_err(|e| format!("Failed to create model folder: {}", e))?;

    let client = reqwest::Client::new();

    // 1. Download smaller metadata files
    let meta_files = ["config.json", "tokenizer.json", "vocabulary.txt"];
    for meta in &meta_files {
        let file_path = model_folder.join(meta);
        if !file_path.exists() {
            let url = format!(
                "https://huggingface.co/Systran/faster-whisper-{}/resolve/main/{}",
                size, meta
            );
            let resp = client
                .get(&url)
                .header("User-Agent", "lipi")
                .send()
                .await
                .map_err(|e| format!("Failed to download {}: {}", meta, e))?;

            if !resp.status().is_success() {
                return Err(format!("Download {} failed with status: {}", meta, resp.status()));
            }

            let bytes = resp
                .bytes()
                .await
                .map_err(|e| format!("Failed to read {}: {}", meta, e))?;
            fs::write(&file_path, &bytes)
                .map_err(|e| format!("Failed to save {}: {}", meta, e))?;
        }
    }

    // 2. Stream model.bin with progress
    let model_bin_url = format!(
        "https://huggingface.co/Systran/faster-whisper-{}/resolve/main/model.bin",
        size
    );
    let target_bin = model_folder.join("model.bin");
    let temp_bin = model_folder.join("model.bin.part");

    let mut response = client
        .get(&model_bin_url)
        .header("User-Agent", "lipi")
        .send()
        .await
        .map_err(|e| format!("Download request for model.bin failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("model.bin download failed with status: {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut file = File::create(&temp_bin)
        .map_err(|e| format!("Failed to create temporary file: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_emit_percent: i64 = -1;
    let start_time = std::time::Instant::now();

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Failed reading chunk: {}", e))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("Failed writing chunk: {}", e))?;
        downloaded += chunk.len() as u64;

        let percent = if total_size > 0 {
            ((downloaded as f64 / total_size as f64) * 100.0) as i64
        } else {
            0
        };

        if percent != last_emit_percent {
            last_emit_percent = percent;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed_bps = if elapsed > 0.4 { (downloaded as f64) / elapsed } else { 0.0 };
            let eta_secs = if speed_bps > 1000.0 && total_size > downloaded {
                ((total_size - downloaded) as f64) / speed_bps
            } else {
                0.0
            };

            let _ = app_handle.emit(
                "model-download-progress",
                serde_json::json!({
                    "engine": "faster_whisper",
                    "model_size": model_size,
                    "downloaded": downloaded,
                    "total": total_size,
                    "percentage": percent,
                    "speed_bps": speed_bps,
                    "eta_secs": eta_secs
                }),
            );
        }
    }

    file.flush()
        .map_err(|e| format!("Failed to flush file: {}", e))?;
    drop(file);

    fs::rename(&temp_bin, &target_bin)
        .map_err(|e| format!("Failed to finalize model.bin: {}", e))?;

    let _ = app_handle.emit(
        "model-download-progress",
        serde_json::json!({
            "engine": "faster_whisper",
            "model_size": model_size,
            "downloaded": downloaded,
            "total": total_size,
            "percentage": 100,
            "speed_bps": 0.0,
            "eta_secs": 0.0
        }),
    );

    Ok(model_folder.to_string_lossy().to_string())
}

pub async fn download_model_weights(
    app_handle: &tauri::AppHandle,
    app_data_dir: &Path,
    custom_dir: Option<&str>,
    engine: &str,
    model_size: &str,
) -> Result<String, String> {
    let models_dir = resolve_models_dir(app_data_dir, custom_dir);
    fs::create_dir_all(&models_dir).map_err(|e| format!("Failed to create models dir: {}", e))?;

    if engine == "faster_whisper" {
        return download_faster_whisper_weights(app_handle, &models_dir, model_size).await;
    }

    // For whisper_cpu & whisper_vulkan: download GGML model file with progress
    let url = get_ggml_download_url(model_size)?;
    let filename = get_model_filename(engine, model_size);
    let target_path = models_dir.join(&filename);
    let temp_path = models_dir.join(format!("{}.part", filename));

    let client = reqwest::Client::new();
    let mut response = client
        .get(url)
        .header("User-Agent", "lipi")
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut file = File::create(&temp_path)
        .map_err(|e| format!("Failed to create temporary file: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_emit_percent: i64 = -1;
    let start_time = std::time::Instant::now();

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Failed reading chunk: {}", e))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("Failed writing chunk: {}", e))?;
        downloaded += chunk.len() as u64;

        let percent = if total_size > 0 {
            ((downloaded as f64 / total_size as f64) * 100.0) as i64
        } else {
            0
        };

        if percent != last_emit_percent {
            last_emit_percent = percent;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed_bps = if elapsed > 0.4 { (downloaded as f64) / elapsed } else { 0.0 };
            let eta_secs = if speed_bps > 1000.0 && total_size > downloaded {
                ((total_size - downloaded) as f64) / speed_bps
            } else {
                0.0
            };

            let _ = app_handle.emit(
                "model-download-progress",
                serde_json::json!({
                    "engine": engine,
                    "model_size": model_size,
                    "downloaded": downloaded,
                    "total": total_size,
                    "percentage": percent,
                    "speed_bps": speed_bps,
                    "eta_secs": eta_secs
                }),
            );
        }
    }

    file.flush()
        .map_err(|e| format!("Failed to flush file: {}", e))?;
    drop(file);

    fs::rename(&temp_path, &target_path)
        .map_err(|e| format!("Failed to finalize model file: {}", e))?;

    // Also verify binary existence for whisper.cpp
    let _ = ensure_whisper_cli_binary(app_data_dir).await;

    let _ = app_handle.emit(
        "model-download-progress",
        serde_json::json!({
            "engine": engine,
            "model_size": model_size,
            "downloaded": downloaded,
            "total": total_size,
            "percentage": 100,
            "speed_bps": 0.0,
            "eta_secs": 0.0
        }),
    );

    Ok(target_path.to_string_lossy().to_string())
}

pub fn delete_model(
    app_data_dir: &Path,
    custom_dir: Option<&str>,
    engine: &str,
    model_size: &str,
) -> Result<(), String> {
    let models_dir = resolve_models_dir(app_data_dir, custom_dir);
    let filename = get_model_filename(engine, model_size);
    let target_path = models_dir.join(&filename);

    if target_path.is_file() {
        fs::remove_file(&target_path).map_err(|e| e.to_string())?;
    } else if target_path.is_dir() {
        fs::remove_dir_all(&target_path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_filename_and_urls() {
        assert_eq!(get_model_filename("whisper_cpu", "tiny"), "ggml-tiny.bin");
        assert_eq!(get_model_filename("whisper_cpu", "base"), "ggml-base.bin");
        assert_eq!(get_model_filename("whisper_cpu", "small"), "ggml-small.bin");
        assert_eq!(get_model_filename("whisper_vulkan", "base"), "ggml-base.bin");
        assert_eq!(get_model_filename("faster_whisper", "base"), "faster-whisper-base");

        assert!(get_ggml_download_url("tiny").unwrap().contains("ggml-tiny.bin"));
        assert!(get_ggml_download_url("base").unwrap().contains("ggml-base.bin"));
        assert!(get_ggml_download_url("small").unwrap().contains("ggml-small.bin"));
        assert!(get_ggml_download_url("unknown").is_err());
    }

    #[test]
    fn test_resolve_models_dir() {
        let app_data = PathBuf::from("/home/user/.local/share/lipi");
        assert_eq!(resolve_models_dir(&app_data, None), app_data.join("models"));
        assert_eq!(resolve_models_dir(&app_data, Some("")), app_data.join("models"));
        assert_eq!(resolve_models_dir(&app_data, Some("   ")), app_data.join("models"));

        assert_eq!(
            resolve_models_dir(&app_data, Some("/media/external/models")),
            PathBuf::from("/media/external/models")
        );
    }

    #[test]
    fn test_check_model_status_uninstalled() {
        let temp_dir = std::env::temp_dir().join(format!("lipi_test_{}", std::process::id()));
        let status = check_model_status(&temp_dir, None, "whisper_cpu", "base").unwrap();
        assert!(!status.installed);
        assert_eq!(status.model_size, "base");
        assert_eq!(status.engine, "whisper_cpu");
        assert_eq!(status.models_dir, temp_dir.join("models").to_string_lossy());
    }

    #[test]
    fn test_faster_whisper_status_and_delete() {
        let temp_dir = std::env::temp_dir().join(format!("lipi_fw_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);

        let initial_status = check_model_status(&temp_dir, None, "faster_whisper", "tiny").unwrap();
        assert!(!initial_status.installed);

        let fw_folder = get_models_dir(&temp_dir).join("faster-whisper-tiny");
        fs::create_dir_all(&fw_folder).unwrap();
        fs::write(fw_folder.join("model.bin"), vec![0u8; 2048]).unwrap();

        let installed_status = check_model_status(&temp_dir, None, "faster_whisper", "tiny").unwrap();
        assert!(installed_status.installed);
        assert_eq!(installed_status.file_size_bytes, 2048);

        delete_model(&temp_dir, None, "faster_whisper", "tiny").unwrap();
        let deleted_status = check_model_status(&temp_dir, None, "faster_whisper", "tiny").unwrap();
        assert!(!deleted_status.installed);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
