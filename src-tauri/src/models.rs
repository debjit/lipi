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

pub(crate) fn apply_no_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("lipi")
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))
}

fn remove_part(path: &Path) {
    let _ = fs::remove_file(path);
}

fn is_windows_store_stub(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains(r"\WindowsApps\") || s.contains("/WindowsApps/")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn venv_python_path(app_data_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        app_data_dir.join("venv").join("Scripts").join("python.exe")
    } else {
        let py3 = app_data_dir.join("venv").join("bin").join("python3");
        if py3.exists() {
            py3
        } else {
            app_data_dir.join("venv").join("bin").join("python")
        }
    }
}

fn python_can_import(python: &Path, module: &str) -> bool {
    if is_windows_store_stub(python) {
        return false;
    }
    let mut cmd = std::process::Command::new(python);
    apply_no_window(&mut cmd);
    cmd.arg("-c").arg(format!("import {}", module));
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn resolve_models_dir(app_data_dir: &Path, custom_dir: Option<&str>) -> PathBuf {
    if let Some(custom) = custom_dir {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            let home_relative = trimmed
                .strip_prefix("~/")
                .or_else(|| trimmed.strip_prefix("~\\"));
            if let Some(stripped) = home_relative {
                if let Some(home) = home_dir() {
                    return home.join(stripped);
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

fn vulkan_backend_candidates(bin_dir: &Path) -> [PathBuf; 3] {
    [
        bin_dir.join("ggml-vulkan.dll"),
        bin_dir.join("libggml-vulkan.so"),
        bin_dir.join("ggml-vulkan.so"),
    ]
}

pub fn vulkan_backend_path(app_data_dir: &Path) -> Option<PathBuf> {
    let bin_dir = get_bin_dir(app_data_dir);
    vulkan_backend_candidates(&bin_dir)
        .into_iter()
        .find(|p| p.exists())
}

fn find_file_named(root: &Path, names: &[&str]) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_named(&path, names) {
                return Some(found);
            }
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if names.iter().any(|want| name.eq_ignore_ascii_case(want)) {
                return Some(path);
            }
        }
    }
    None
}

pub fn get_model_filename(engine: &str, model_size: &str) -> String {
    if engine == "faster_whisper" {
        let size = match model_size.to_lowercase().as_str() {
            "tiny" => "tiny",
            "small" => "small",
            "medium" => "medium",
            "turbo" | "large-v3-turbo" | "turbo_q8" => "large-v3-turbo",
            _ => "base",
        };
        format!("faster-whisper-{}", size)
    } else {
        let size = match model_size.to_lowercase().as_str() {
            "tiny" => "tiny",
            "small" => "small",
            "medium" => "medium",
            "turbo" | "large-v3-turbo" | "turbo_q8" => "large-v3-turbo-q8_0",
            _ => "base",
        };
        format!("ggml-{}.bin", size)
    }
}

pub fn get_ggml_download_url(model_size: &str) -> Result<&'static str, String> {
    match model_size.to_lowercase().as_str() {
        "tiny" => Ok("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin"),
        "base" => Ok("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"),
        "small" => Ok("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"),
        "medium" => Ok("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin"),
        "turbo" | "large-v3-turbo" | "turbo_q8" => Ok("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin"),
        _ => Err(format!("Unsupported model size: {}", model_size)),
    }
}

fn flatten_release_dir_if_exists(bin_dir: &Path) {
    let release_dir = bin_dir.join("Release");
    if release_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&release_dir) {
            for entry in entries.flatten() {
                let dest = bin_dir.join(entry.file_name());
                if dest.exists() {
                    let _ = fs::remove_file(&dest);
                }
                if let Err(_) = fs::rename(entry.path(), &dest) {
                    if let Ok(_) = fs::copy(entry.path(), &dest) {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
        let _ = fs::remove_dir_all(&release_dir);
    }
}

pub fn find_whisper_binary(app_data_dir: &Path) -> Option<PathBuf> {
    let bin_dir = get_bin_dir(app_data_dir);
    flatten_release_dir_if_exists(&bin_dir);

    let candidates = [
        bin_dir.join("whisper-cli"),
        bin_dir.join("whisper-cli.exe"),
        bin_dir.join("main"),
        bin_dir.join("main.exe"),
        bin_dir.join("Release").join("whisper-cli.exe"),
        bin_dir.join("Release").join("main.exe"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    // Check system PATH
    for name in &["whisper-cli", "whisper-cli.exe"] {
        if let Ok(path) = which::which(name) {
            return Some(path);
        }
    }

    None
}

pub fn find_whisper_server_binary(app_data_dir: &Path) -> Option<PathBuf> {
    let bin_dir = get_bin_dir(app_data_dir);
    flatten_release_dir_if_exists(&bin_dir);

    let candidates = [
        bin_dir.join("whisper-server"),
        bin_dir.join("whisper-server.exe"),
        bin_dir.join("Release").join("whisper-server.exe"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    for name in &["whisper-server", "whisper-server.exe"] {
        if let Ok(path) = which::which(name) {
            return Some(path);
        }
    }

    None
}

// Fallback search using std::process::Command "which" or "where"
mod which {
    use std::path::PathBuf;
    use std::process::Command;

    pub fn which(name: &str) -> Result<PathBuf, ()> {
        let finder = if cfg!(windows) { "where.exe" } else { "which" };
        let mut cmd = Command::new(finder);
        super::apply_no_window(&mut cmd);
        let output = cmd.arg(name).output().map_err(|_| ())?;
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let p = PathBuf::from(trimmed);
                if p.exists() && !super::is_windows_store_stub(&p) {
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
    // Prefer an existing system install of faster-whisper, then the app venv.
    let candidates: &[&str] = if cfg!(windows) {
        &["py", "python", "python3"]
    } else {
        &["python3", "python"]
    };

    for candidate in candidates {
        if let Some(path) = which_command(candidate) {
            if python_can_import(&path, "faster_whisper") {
                return Some(path);
            }
        }
    }

    let venv_py = venv_python_path(app_data_dir);
    if venv_py.exists() && python_can_import(&venv_py, "faster_whisper") {
        return Some(venv_py);
    }

    None
}

pub fn is_python_functional(cmd: &Path) -> bool {
    python_can_import(cmd, "sys")
}

pub fn find_working_python_cmd() -> Option<PathBuf> {
    let candidates: &[&'static str] = if cfg!(windows) {
        &["py", "python", "python3"]
    } else {
        &["python3", "python"]
    };

    for &cmd in candidates {
        if let Some(path) = which_command(cmd) {
            if is_python_functional(&path) {
                return Some(path);
            }
        }
    }
    None
}

pub fn install_faster_whisper_deps(app_data_dir: &Path) -> Result<String, String> {
    let py_path = find_working_python_cmd().ok_or_else(|| {
        if cfg!(windows) {
            "Python 3 is not found on your system PATH. Please install Python from https://python.org or run 'winget install Python.Python.3.11' (check 'Add Python to PATH').".to_string()
        } else {
            "Python 3 is not found on your system. Please install python3 (e.g. 'sudo apt install python3-pip').".to_string()
        }
    })?;

    let venv_dir = app_data_dir.join("venv");
    let mut venv_py = venv_python_path(app_data_dir);
    if !venv_py.exists() {
        if venv_dir.exists() {
            let _ = fs::remove_dir_all(&venv_dir);
        }
        let mut venv_cmd = std::process::Command::new(&py_path);
        apply_no_window(&mut venv_cmd);
        venv_cmd.arg("-m").arg("venv").arg(&venv_dir);
        let venv_status = venv_cmd
            .status()
            .map_err(|e| format!("Failed to create virtual environment: {}", e))?;
        if !venv_status.success() {
            return Err(format!(
                "Failed to create virtual environment with '{} -m venv'",
                py_path.to_string_lossy()
            ));
        }
        venv_py = venv_python_path(app_data_dir);
    }

    if !venv_py.exists() {
        return Err("Virtual environment was created but Python was not found inside it.".into());
    }

    let mut pip_cmd = std::process::Command::new(&venv_py);
    apply_no_window(&mut pip_cmd);
    pip_cmd
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("faster-whisper");

    let pip_output = pip_cmd
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
            None => {
                if let Some(sys_py) = find_working_python_cmd() {
                    (
                        false,
                        format!("Python detected at {}", sys_py.to_string_lossy()),
                    )
                } else {
                    (false, "Python 3 not found in PATH".into())
                }
            }
        }
    } else {
        let b = find_whisper_binary(app_data_dir);
        if engine == "whisper_vulkan" {
            match (b, vulkan_backend_path(app_data_dir)) {
                (Some(cli), Some(vk)) => (
                    true,
                    format!("{} + {}", cli.to_string_lossy(), vk.file_name().unwrap_or_default().to_string_lossy()),
                ),
                (Some(_), None) => (
                    false,
                    "CPU whisper.cpp runner found. Download the Vulkan GPU backend to use this engine.".into(),
                ),
                (None, _) => (false, "whisper-cli / whisper-server runner binary not downloaded yet".into()),
            }
        } else {
            let avail = b.is_some();
            let path = b.map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            (avail, path)
        }
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
        let client = download_client()?;
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
        let client = download_client()?;
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

        let mut tar_cmd = std::process::Command::new("tar");
        apply_no_window(&mut tar_cmd);
        let mut extracted = false;
        if let Ok(status) = tar_cmd
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
            let mut ps_cmd = std::process::Command::new("powershell");
            apply_no_window(&mut ps_cmd);
            let _ = ps_cmd
                .arg("-NoProfile")
                .arg("-Command")
                .arg(&ps_script)
                .status();
        }

        let _ = fs::remove_file(&zip_path);

        // Flatten Release directory so executables and DLLs reside directly in bin_dir
        flatten_release_dir_if_exists(&bin_dir);

        if let Some(p) = find_whisper_binary(app_data_dir) {
            return Ok(p);
        }
    }

    Err("whisper-cli binary not found. Please install whisper-cli or ensure it is in PATH.".into())
}

pub async fn ensure_whisper_vulkan_backend(app_data_dir: &Path) -> Result<PathBuf, String> {
    let _ = ensure_whisper_cli_binary(app_data_dir).await?;
    if let Some(existing) = vulkan_backend_path(app_data_dir) {
        return Ok(existing);
    }

    let bin_dir = get_bin_dir(app_data_dir);
    fs::create_dir_all(&bin_dir).map_err(|e| format!("Failed to create bin dir: {}", e))?;

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    let archive_url = "https://github.com/ggml-org/llama.cpp/releases/download/b10992/llama-b10992-bin-win-vulkan-x64.zip";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let archive_url = "https://github.com/ggml-org/llama.cpp/releases/download/b10992/llama-b10992-bin-ubuntu-vulkan-x64.tar.gz";
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64")
    )))]
    {
        return Err(
            "Vulkan GPU backend is not bundled for this OS/arch. Install a whisper.cpp build that includes ggml-vulkan."
                .into(),
        );
    }

    #[cfg(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64")
    ))]
    {
        let client = download_client()?;
        let resp = client
            .get(archive_url)
            .header("User-Agent", "lipi")
            .send()
            .await
            .map_err(|e| format!("Failed to download Vulkan GPU backend: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "Vulkan GPU backend download failed with status: {}",
                resp.status()
            ));
        }

        let archive_bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed to read Vulkan archive bytes: {}", e))?;

        let extract_dir = std::env::temp_dir().join("lipi_vulkan_extract");
        let _ = fs::remove_dir_all(&extract_dir);
        fs::create_dir_all(&extract_dir)
            .map_err(|e| format!("Failed to create Vulkan extract dir: {}", e))?;

        #[cfg(windows)]
        let archive_path = extract_dir.join("llama-vulkan.zip");
        #[cfg(not(windows))]
        let archive_path = extract_dir.join("llama-vulkan.tar.gz");

        fs::write(&archive_path, &archive_bytes)
            .map_err(|e| format!("Failed to write Vulkan archive: {}", e))?;

        let mut tar_cmd = std::process::Command::new("tar");
        apply_no_window(&mut tar_cmd);
        #[cfg(windows)]
        {
            tar_cmd.arg("-xf").arg(&archive_path).arg("-C").arg(&extract_dir);
        }
        #[cfg(not(windows))]
        {
            tar_cmd
                .arg("-xzf")
                .arg(&archive_path)
                .arg("-C")
                .arg(&extract_dir);
        }
        let status = tar_cmd
            .status()
            .map_err(|e| format!("Failed to extract Vulkan archive: {}", e))?;
        if !status.success() {
            return Err("Failed to unpack Vulkan GPU backend archive".into());
        }

        let found = find_file_named(
            &extract_dir,
            &["ggml-vulkan.dll", "libggml-vulkan.so", "ggml-vulkan.so"],
        )
        .ok_or_else(|| "Vulkan backend library was not found inside the downloaded archive".to_string())?;

        let dest = bin_dir.join(found.file_name().unwrap());
        fs::copy(&found, &dest).map_err(|e| format!("Failed to install Vulkan backend: {}", e))?;
        let _ = fs::remove_dir_all(&extract_dir);

        if dest.exists() {
            return Ok(dest);
        }
    }

    Err("Vulkan GPU backend could not be installed. The official whisper.cpp Windows zip is CPU-only.".into())
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
        "medium" => "medium",
        "turbo" | "large-v3-turbo" | "turbo_q8" => "large-v3-turbo",
        _ => "base",
    };
    let model_folder = models_dir.join(format!("faster-whisper-{}", size));
    fs::create_dir_all(&model_folder).map_err(|e| format!("Failed to create model folder: {}", e))?;

    let client = download_client()?;

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
        .map_err(|e| {
            remove_part(&temp_bin);
            format!("Failed reading chunk: {}", e)
        })?
    {
        file.write_all(&chunk).map_err(|e| {
            remove_part(&temp_bin);
            format!("Failed writing chunk: {}", e)
        })?;
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

    if target_bin.exists() {
        let _ = fs::remove_file(&target_bin);
    }
    if let Err(_) = fs::rename(&temp_bin, &target_bin) {
        fs::copy(&temp_bin, &target_bin).map_err(|e| format!("Failed to finalize model.bin: {}", e))?;
        let _ = fs::remove_file(&temp_bin);
    }

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

    let client = download_client()?;
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
        .map_err(|e| {
            remove_part(&temp_path);
            format!("Failed reading chunk: {}", e)
        })?
    {
        file.write_all(&chunk).map_err(|e| {
            remove_part(&temp_path);
            format!("Failed writing chunk: {}", e)
        })?;
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

    if target_path.exists() {
        let _ = fs::remove_file(&target_path);
    }
    if let Err(_) = fs::rename(&temp_path, &target_path) {
        fs::copy(&temp_path, &target_path).map_err(|e| format!("Failed to finalize model file: {}", e))?;
        let _ = fs::remove_file(&temp_path);
    }

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

pub fn vad_model_path(app_data_dir: &Path, custom_dir: Option<&str>) -> PathBuf {
    resolve_models_dir(app_data_dir, custom_dir).join("silero_vad.onnx")
}

pub fn vad_installed(app_data_dir: &Path, custom_dir: Option<&str>) -> bool {
    fs::metadata(vad_model_path(app_data_dir, custom_dir))
        .map(|m| m.len() > 1000)
        .unwrap_or(false)
}

pub async fn download_vad_model(
    app_handle: &tauri::AppHandle,
    app_data_dir: &Path,
    custom_dir: Option<&str>,
) -> Result<String, String> {
    let models_dir = resolve_models_dir(app_data_dir, custom_dir);
    fs::create_dir_all(&models_dir).map_err(|e| format!("Failed to create models dir: {}", e))?;
    let target_path = models_dir.join("silero_vad.onnx");
    if vad_installed(app_data_dir, custom_dir) {
        return Ok(target_path.to_string_lossy().to_string());
    }

    let url = "https://github.com/snakers4/silero-vad/raw/v5.1.2/src/silero_vad/data/silero_vad.onnx";
    let temp_path = models_dir.join("silero_vad.onnx.part");
    let client = download_client()?;
    let mut response = client
        .get(url)
        .header("User-Agent", "lipi")
        .send()
        .await
        .map_err(|e| format!("VAD download failed: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("VAD download failed with status: {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut file = File::create(&temp_path).map_err(|e| format!("Failed to create VAD file: {}", e))?;
    let mut downloaded: u64 = 0;
    let mut last_emit_percent: i64 = -1;
    let start_time = std::time::Instant::now();
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        remove_part(&temp_path);
        format!("Failed reading VAD chunk: {}", e)
    })? {
        file.write_all(&chunk).map_err(|e| {
            remove_part(&temp_path);
            format!("Failed writing VAD chunk: {}", e)
        })?;
        downloaded += chunk.len() as u64;
        let percent = if total_size > 0 { (downloaded as f64 / total_size as f64 * 100.0) as i64 } else { 0 };
        if percent != last_emit_percent {
            last_emit_percent = percent;
            let elapsed = start_time.elapsed().as_secs_f64().max(0.001);
            let speed = downloaded as f64 / elapsed;
            let eta = if total_size > downloaded { (total_size - downloaded) as f64 / speed } else { 0.0 };
            let _ = app_handle.emit(
                "model-download-progress",
                serde_json::json!({
                    "engine": "vad",
                    "model_size": "silero",
                    "downloaded": downloaded,
                    "total": total_size,
                    "percentage": percent,
                    "speed_bps": speed,
                    "eta_secs": eta
                }),
            );
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    if target_path.exists() {
        let _ = fs::remove_file(&target_path);
    }
    fs::rename(&temp_path, &target_path).map_err(|e| format!("Failed to save VAD model: {}", e))?;
    Ok(target_path.to_string_lossy().to_string())
}

pub fn delete_whisper_binary(app_data_dir: &Path) -> Result<(), String> {
    let bin_dir = get_bin_dir(app_data_dir);
    if bin_dir.exists() {
        fs::remove_dir_all(&bin_dir)
            .map_err(|e| format!("Failed to remove whisper binary directory: {}", e))?;
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
        assert_eq!(get_model_filename("whisper_cpu", "medium"), "ggml-medium.bin");
        assert_eq!(get_model_filename("whisper_cpu", "turbo_q8"), "ggml-large-v3-turbo-q8_0.bin");
        assert_eq!(get_model_filename("whisper_vulkan", "base"), "ggml-base.bin");
        assert_eq!(get_model_filename("faster_whisper", "base"), "faster-whisper-base");
        assert_eq!(get_model_filename("faster_whisper", "turbo_q8"), "faster-whisper-large-v3-turbo");

        assert!(get_ggml_download_url("tiny").unwrap().contains("ggml-tiny.bin"));
        assert!(get_ggml_download_url("base").unwrap().contains("ggml-base.bin"));
        assert!(get_ggml_download_url("small").unwrap().contains("ggml-small.bin"));
        assert!(get_ggml_download_url("medium").unwrap().contains("ggml-medium.bin"));
        assert!(get_ggml_download_url("turbo_q8").unwrap().contains("ggml-large-v3-turbo-q8_0.bin"));
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

        if let Some(home) = home_dir() {
            assert_eq!(
                resolve_models_dir(&app_data, Some("~/custom-models")),
                home.join("custom-models")
            );
        }
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
