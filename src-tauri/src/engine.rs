use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use crate::models::{
    self, ensure_whisper_cli_binary, ensure_whisper_server_binary, get_model_filename,
    resolve_models_dir,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMemoryStatus {
    pub is_loaded: bool,
    pub engine: Option<String>,
    pub model_size: Option<String>,
    pub loaded_at_timestamp: Option<u64>,
    pub last_active_timestamp: Option<u64>,
    pub idle_seconds: u64,
    pub idle_timeout_mins: u32,
    pub estimated_ram_mb: u64,
    pub port: Option<u16>,
}

fn current_epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn estimate_ram_mb(model_size: &str) -> u64 {
    match model_size.to_lowercase().as_str() {
        "tiny" => 150,
        "base" => 250,
        "small" => 500,
        "medium" => 1500,
        "large" => 3000,
        _ => 250,
    }
}

struct RunningDaemon {
    engine: String,
    model_size: String,
    port: u16,
    child: std::process::Child,
    #[allow(dead_code)]
    loaded_at: std::time::Instant,
    last_active: std::time::Instant,
    loaded_at_timestamp: u64,
    last_active_timestamp: u64,
    estimated_ram_mb: u64,
}

impl Drop for RunningDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct ModelSupervisor {
    daemon: Mutex<Option<RunningDaemon>>,
}

impl ModelSupervisor {
    pub fn new() -> Self {
        Self {
            daemon: Mutex::new(None),
        }
    }

    pub fn get_status(&self, idle_timeout_mins: u32) -> ModelMemoryStatus {
        let mut lock = match self.daemon.lock() {
            Ok(l) => l,
            Err(poisoned) => poisoned.into_inner(),
        };

        if let Some(ref mut d) = *lock {
            if let Ok(Some(_)) = d.child.try_wait() {
                *lock = None;
                return ModelMemoryStatus {
                    is_loaded: false,
                    engine: None,
                    model_size: None,
                    loaded_at_timestamp: None,
                    last_active_timestamp: None,
                    idle_seconds: 0,
                    idle_timeout_mins,
                    estimated_ram_mb: 0,
                    port: None,
                };
            }

            let idle_secs = d.last_active.elapsed().as_secs();
            ModelMemoryStatus {
                is_loaded: true,
                engine: Some(d.engine.clone()),
                model_size: Some(d.model_size.clone()),
                loaded_at_timestamp: Some(d.loaded_at_timestamp),
                last_active_timestamp: Some(d.last_active_timestamp),
                idle_seconds: idle_secs,
                idle_timeout_mins,
                estimated_ram_mb: d.estimated_ram_mb,
                port: Some(d.port),
            }
        } else {
            ModelMemoryStatus {
                is_loaded: false,
                engine: None,
                model_size: None,
                loaded_at_timestamp: None,
                last_active_timestamp: None,
                idle_seconds: 0,
                idle_timeout_mins,
                estimated_ram_mb: 0,
                port: None,
            }
        }
    }

    pub fn unload(&self) {
        let mut lock = match self.daemon.lock() {
            Ok(l) => l,
            Err(poisoned) => poisoned.into_inner(),
        };
        drop(lock.take());
    }

    pub fn check_idle_timeout(&self, timeout_mins: u32) -> bool {
        if timeout_mins == 0 {
            return false;
        }

        let mut lock = match self.daemon.lock() {
            Ok(l) => l,
            Err(poisoned) => poisoned.into_inner(),
        };

        if let Some(ref d) = *lock {
            let idle_secs = d.last_active.elapsed().as_secs();
            let limit_secs = (timeout_mins as u64) * 60;
            if idle_secs >= limit_secs {
                drop(lock.take());
                return true;
            }
        }
        false
    }

    pub async fn ensure_loaded(
        &self,
        engine: &str,
        model_size: &str,
        app_data_dir: &Path,
        custom_models_dir: Option<&str>,
    ) -> Result<u16, String> {
        {
            let mut lock = match self.daemon.lock() {
                Ok(l) => l,
                Err(poisoned) => poisoned.into_inner(),
            };

            if let Some(ref mut d) = *lock {
                if d.engine == engine && d.model_size == model_size {
                    if let Ok(None) = d.child.try_wait() {
                        d.last_active = std::time::Instant::now();
                        d.last_active_timestamp = current_epoch_seconds();
                        return Ok(d.port);
                    }
                }
                drop(lock.take());
            }
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to find free TCP port: {}", e))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Failed to get socket address: {}", e))?
            .port();
        drop(listener);

        let models_dir = resolve_models_dir(app_data_dir, custom_models_dir);
        let child = if engine == "faster_whisper" {
            let python_exe = models::find_faster_whisper_python(app_data_dir).ok_or_else(|| {
                "faster-whisper is not installed. Setup Python in Settings.".to_string()
            })?;

            let size = match model_size.to_lowercase().as_str() {
                "tiny" => "tiny",
                "small" => "small",
                _ => "base",
            };
            let model_folder = models_dir.join(format!("faster-whisper-{}", size));
            let target_arg = if model_folder.join("model.bin").exists() {
                model_folder.to_string_lossy().to_string()
            } else {
                size.to_string()
            };

            let py_server_code = r#"
import sys, io, json
from http.server import HTTPServer, BaseHTTPRequestHandler
from faster_whisper import WhisperModel

port = int(sys.argv[1])
model_target = sys.argv[2]
model = WhisperModel(model_target, device="cpu", compute_type="int8")

class H(BaseHTTPRequestHandler):
    def do_POST(self):
        try:
            length = int(self.headers.get('Content-Length', 0))
            audio = self.rfile.read(length)
            lang = self.headers.get('X-Language', None)
            kw = {}
            if lang and lang != 'auto':
                kw['language'] = lang
            segments, _ = model.transcribe(io.BytesIO(audio), **kw)
            text = "".join(s.text for s in segments).strip()
            res = json.dumps({"text": text}).encode('utf-8')
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(res)))
            self.end_headers()
            self.wfile.write(res)
        except Exception as e:
            res = json.dumps({"error": str(e)}).encode('utf-8')
            self.send_response(500)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(res)))
            self.end_headers()
            self.wfile.write(res)

    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Length', '2')
        self.end_headers()
        self.wfile.write(b"OK")

    def log_message(self, *a):
        pass

server = HTTPServer(('127.0.0.1', port), H)
server.serve_forever()
"#;

            Command::new(python_exe)
                .arg("-c")
                .arg(py_server_code)
                .arg(port.to_string())
                .arg(target_arg)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("Failed to spawn faster-whisper daemon: {}", e))?
        } else {
            let model_file = models_dir.join(get_model_filename("whisper_cpu", model_size));
            if !model_file.exists() {
                return Err(format!(
                    "Model file '{}' not found in {}. Download it first.",
                    model_size,
                    models_dir.to_string_lossy()
                ));
            }

            let server_binary = ensure_whisper_server_binary(app_data_dir).await?;
            let mut cmd = Command::new(&server_binary);

            if let Some(parent) = server_binary.parent() {
                let current_ld = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
                let new_ld = if current_ld.is_empty() {
                    parent.to_string_lossy().to_string()
                } else {
                    format!("{}:{}", parent.to_string_lossy(), current_ld)
                };
                cmd.env("LD_LIBRARY_PATH", new_ld);
            }

            let num_threads = std::thread::available_parallelism()
                .map(|n| n.get().min(8).to_string())
                .unwrap_or_else(|_| "4".to_string());

            cmd.arg("--host")
                .arg("127.0.0.1")
                .arg("--port")
                .arg(port.to_string())
                .arg("-m")
                .arg(&model_file)
                .arg("--inference-path")
                .arg("/inference")
                .arg("-nt")
                .arg("-t")
                .arg(num_threads);

            if engine == "whisper_vulkan" {
                cmd.arg("-ng").arg("99");
            } else {
                cmd.arg("-ng").arg("0");
            }

            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("Failed to spawn whisper-server: {}", e))?
        };

        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let mut ready = false;
        let mut child = child;

        for _ in 0..75 {
            std::thread::sleep(std::time::Duration::from_millis(200));

            if let Ok(Some(exit)) = child.try_wait() {
                return Err(format!("Daemon exited prematurely with status: {}", exit));
            }

            if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(150)).is_ok() {
                ready = true;
                break;
            }
        }

        if !ready {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Daemon failed to respond within 15 seconds".into());
        }

        let now_instant = std::time::Instant::now();
        let epoch = current_epoch_seconds();
        let ram = estimate_ram_mb(model_size);

        let daemon = RunningDaemon {
            engine: engine.to_string(),
            model_size: model_size.to_string(),
            port,
            child,
            loaded_at: now_instant,
            last_active: now_instant,
            loaded_at_timestamp: epoch,
            last_active_timestamp: epoch,
            estimated_ram_mb: ram,
        };

        let mut lock = match self.daemon.lock() {
            Ok(l) => l,
            Err(poisoned) => poisoned.into_inner(),
        };
        *lock = Some(daemon);

        Ok(port)
    }

    pub async fn transcribe(
        &self,
        wav_bytes: Vec<u8>,
        engine: &str,
        model_size: &str,
        language: Option<&str>,
        app_data_dir: &Path,
        custom_models_dir: Option<&str>,
    ) -> Result<String, String> {
        if wav_bytes.is_empty() {
            return Err("Audio buffer is empty".into());
        }

        match self.ensure_loaded(engine, model_size, app_data_dir, custom_models_dir).await {
            Ok(port) => {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(180))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new());

                let res = if engine == "faster_whisper" {
                    let mut req = client
                        .post(format!("http://127.0.0.1:{}/transcribe", port))
                        .header("Content-Type", "audio/wav")
                        .body(wav_bytes.clone());

                    if let Some(lang) = language {
                        let trimmed = lang.trim();
                        if !trimmed.is_empty() && trimmed != "auto" {
                            req = req.header("X-Language", trimmed);
                        }
                    }

                    req.send().await
                } else {
                    let mut form = reqwest::multipart::Form::new()
                        .part(
                            "file",
                            reqwest::multipart::Part::bytes(wav_bytes.clone())
                                .file_name("audio.wav")
                                .mime_str("audio/wav")
                                .map_err(|e| e.to_string())?,
                        )
                        .text("response_format", "json");

                    if let Some(lang) = language {
                        let trimmed = lang.trim();
                        if !trimmed.is_empty() && trimmed != "auto" {
                            form = form.text("language", trimmed.to_string());
                        }
                    }

                    client
                        .post(format!("http://127.0.0.1:{}/inference", port))
                        .multipart(form)
                        .send()
                        .await
                };

                match res {
                    Ok(resp) if resp.status().is_success() => {
                        let text_or_json = resp.text().await.unwrap_or_default();
                        let parsed_text = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text_or_json) {
                            v.get("text")
                                .and_then(|t| t.as_str())
                                .unwrap_or(&text_or_json)
                                .to_string()
                        } else {
                            text_or_json
                        };

                        if let Ok(mut lock) = self.daemon.lock() {
                            if let Some(ref mut d) = *lock {
                                d.last_active = std::time::Instant::now();
                                d.last_active_timestamp = current_epoch_seconds();
                            }
                        }

                        return Ok(parsed_text.trim().to_string());
                    }
                    Ok(resp) => {
                        eprintln!("Daemon returned HTTP {}, falling back to one-shot", resp.status());
                        self.unload();
                    }
                    Err(e) => {
                        eprintln!("Daemon request error: {}, falling back to one-shot", e);
                        self.unload();
                    }
                }
            }
            Err(e) => {
                eprintln!("Daemon startup error: {}, falling back to one-shot", e);
            }
        }

        transcribe_local(
            wav_bytes,
            engine,
            model_size,
            language,
            app_data_dir,
            custom_models_dir,
        )
        .await
    }
}

pub async fn transcribe_local(
    wav_bytes: Vec<u8>,
    engine: &str,
    model_size: &str,
    language: Option<&str>,
    app_data_dir: &Path,
    custom_models_dir: Option<&str>,
) -> Result<String, String> {
    if wav_bytes.is_empty() {
        return Err("Audio buffer is empty".into());
    }

    let temp_dir = std::env::temp_dir();
    let temp_wav = temp_dir.join(format!("lipi_input_{}.wav", std::process::id()));
    fs::write(&temp_wav, &wav_bytes)
        .map_err(|e| format!("Failed to write temporary audio file: {}", e))?;

    let result = match engine {
        "faster_whisper" => transcribe_faster_whisper(&temp_wav, model_size, language, app_data_dir, custom_models_dir),
        "whisper_vulkan" => transcribe_whisper_cpp(&temp_wav, model_size, language, app_data_dir, custom_models_dir, true).await,
        _ => transcribe_whisper_cpp(&temp_wav, model_size, language, app_data_dir, custom_models_dir, false).await,
    };

    let _ = fs::remove_file(&temp_wav);
    result
}

async fn transcribe_whisper_cpp(
    wav_path: &Path,
    model_size: &str,
    language: Option<&str>,
    app_data_dir: &Path,
    custom_models_dir: Option<&str>,
    vulkan: bool,
) -> Result<String, String> {
    let models_dir = resolve_models_dir(app_data_dir, custom_models_dir);
    let model_file = models_dir.join(get_model_filename("whisper_cpu", model_size));

    if !model_file.exists() {
        return Err(format!(
            "Model '{}' is not installed in {}. Please download it in Settings.",
            model_size,
            models_dir.to_string_lossy()
        ));
    }

    let binary_path = ensure_whisper_cli_binary(app_data_dir).await?;

    let mut cmd = Command::new(&binary_path);
    if let Some(parent) = binary_path.parent() {
        let current_ld = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        let new_ld = if current_ld.is_empty() {
            parent.to_string_lossy().to_string()
        } else {
            format!("{}:{}", parent.to_string_lossy(), current_ld)
        };
        cmd.env("LD_LIBRARY_PATH", new_ld);
    }

    cmd.arg("-m")
        .arg(&model_file)
        .arg("-f")
        .arg(wav_path)
        .arg("-nt")
        .arg("--no-prints");

    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get().min(8).to_string())
        .unwrap_or_else(|_| "4".to_string());
    cmd.arg("-t").arg(num_threads);

    if vulkan {
        // -ng 99 enables GPU layer offloading for ggml Vulkan backend
        cmd.arg("-ng").arg("99");
    } else {
        cmd.arg("-ng").arg("0");
    }

    if let Some(lang) = language {
        let trimmed = lang.trim();
        if !trimmed.is_empty() && trimmed != "auto" {
            cmd.arg("-l").arg(trimmed);
        }
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute whisper-cli: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("whisper-cli execution error: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().to_string())
}

fn transcribe_faster_whisper(
    wav_path: &Path,
    model_size: &str,
    language: Option<&str>,
    app_data_dir: &Path,
    custom_models_dir: Option<&str>,
) -> Result<String, String> {
    let python_exe = crate::models::find_faster_whisper_python(app_data_dir).ok_or_else(|| {
        "faster-whisper is not installed. Please click '⚡ Setup Python Environment' in Settings or run 'pip install faster-whisper'.".to_string()
    })?;

    let models_dir = resolve_models_dir(app_data_dir, custom_models_dir);
    let size = match model_size.to_lowercase().as_str() {
        "tiny" => "tiny",
        "small" => "small",
        _ => "base",
    };
    let model_folder = models_dir.join(format!("faster-whisper-{}", size));

    let py_script = r#"
import sys
import os

try:
    from faster_whisper import WhisperModel
except ImportError:
    sys.stderr.write("faster-whisper is not installed in the active environment.\n")
    sys.exit(1)

wav_path = sys.argv[1]
model_target = sys.argv[2]
lang = sys.argv[3] if len(sys.argv) > 3 and sys.argv[3] else None

try:
    # If model_target is a directory containing model.bin, loads completely offline
    model = WhisperModel(model_target, device="cpu", compute_type="int8")
    kwargs = {}
    if lang and lang != "auto":
        kwargs["language"] = lang
    segments, _ = model.transcribe(wav_path, **kwargs)
    text = "".join(seg.text for seg in segments).strip()
    sys.stdout.write(text)
except Exception as e:
    sys.stderr.write(f"Inference error: {e}\n")
    sys.exit(1)
"#;

    let target_arg = if model_folder.join("model.bin").exists() {
        model_folder.to_string_lossy().to_string()
    } else {
        size.to_string()
    };

    let mut cmd = Command::new(python_exe);
    cmd.arg("-c")
        .arg(py_script)
        .arg(wav_path)
        .arg(target_arg);

    if let Some(lang) = language {
        let trimmed = lang.trim();
        if !trimmed.is_empty() {
            cmd.arg(trimmed);
        }
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to spawn python: {}. Ensure python is installed.", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ram_estimation() {
        assert_eq!(estimate_ram_mb("tiny"), 150);
        assert_eq!(estimate_ram_mb("base"), 250);
        assert_eq!(estimate_ram_mb("small"), 500);
        assert_eq!(estimate_ram_mb("medium"), 1500);
        assert_eq!(estimate_ram_mb("large"), 3000);
    }

    #[test]
    fn test_supervisor_initial_state_and_unload() {
        let sup = ModelSupervisor::new();
        let status = sup.get_status(10);
        assert!(!status.is_loaded);
        assert_eq!(status.idle_timeout_mins, 10);
        assert_eq!(status.estimated_ram_mb, 0);
        assert!(status.engine.is_none());
        assert!(status.port.is_none());

        sup.unload();
        let status2 = sup.get_status(10);
        assert!(!status2.is_loaded);

        assert!(!sup.check_idle_timeout(10));
    }
}
