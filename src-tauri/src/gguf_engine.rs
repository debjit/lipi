use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use transcribe_cpp::{Model, RunOptions, Session};

#[derive(Clone)]
pub struct GgufEngine {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    init_error: Option<String>,
    active: Option<ActiveModel>,
}

struct ActiveModel {
    // Drop the session before the model it borrows.
    session: Session,
    #[allow(dead_code)] // owns the weights the session runs
    model: Model,
    path: PathBuf,
    last_active: Instant,
    backend: String,
}

pub struct Resident {
    pub idle_seconds: u64,
    pub backend: String,
}

impl GgufEngine {
    pub fn new() -> Self {
        let init_error = match transcribe_cpp::init_backends_default() {
            Ok(()) => None,
            Err(err) => {
                let err = err.to_string();
                eprintln!("[GGUF] backend init failed: {err}");
                Some(err)
            }
        };
        Self {
            inner: Arc::new(Mutex::new(Inner {
                init_error,
                active: None,
            })),
        }
    }

    pub fn ensure_loaded(&self, model_path: &Path) -> Result<(), String> {
        let mut guard = self.lock()?;
        load_into(&mut guard, model_path)
    }

    pub fn transcribe_pcm(
        &self,
        pcm: &[f32],
        model_path: &Path,
        language: Option<&str>,
    ) -> Result<String, String> {
        let mut guard = self.lock()?;
        load_into(&mut guard, model_path)?;
        let active = guard.active.as_mut().expect("model just loaded");
        active.last_active = Instant::now();
        let mut opts = RunOptions::default();
        opts.language = language.map(str::to_string);
        let result = active
            .session
            .run(pcm, &opts)
            .map_err(|err| format!("GGUF transcription error: {err}"))?;
        Ok(result.text)
    }

    pub fn unload(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.active = None;
        }
    }

    /// `0` keeps the model loaded, matching `ModelSupervisor::check_idle_timeout`.
    pub fn unload_if_idle(&self, timeout_mins: u32) {
        if timeout_mins == 0 {
            return;
        }
        let limit = Duration::from_secs(u64::from(timeout_mins) * 60);
        if let Ok(mut guard) = self.inner.lock() {
            if guard
                .active
                .as_ref()
                .is_some_and(|active| active.last_active.elapsed() >= limit)
            {
                guard.active = None;
            }
        }
    }

    pub fn resident(&self) -> Option<Resident> {
        let guard = self.inner.lock().ok()?;
        guard.active.as_ref().map(|active| Resident {
            idle_seconds: active.last_active.elapsed().as_secs(),
            backend: active.backend.clone(),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, String> {
        self.inner.lock().map_err(|err| err.to_string())
    }
}

fn load_into(inner: &mut Inner, model_path: &Path) -> Result<(), String> {
    if let Some(err) = &inner.init_error {
        return Err(err.clone());
    }
    if inner.active.as_ref().is_some_and(|active| active.path == model_path) {
        if let Some(active) = inner.active.as_mut() {
            active.last_active = Instant::now();
        }
        return Ok(());
    }
    // Free the previous weights before mapping the next file.
    inner.active = None;
    let model = Model::load(model_path).map_err(|err| format!("Failed to load GGUF model: {err}"))?;
    let backend = model.backend();
    eprintln!("[GGUF] loaded {} on {backend}", model_path.display());
    let session = model
        .session()
        .map_err(|err| format!("Failed to create transcribe session: {err}"))?;
    inner.active = Some(ActiveModel {
        session,
        model,
        path: model_path.to_path_buf(),
        last_active: Instant::now(),
        backend,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unload_if_idle_without_a_model_keeps_the_slot_empty() {
        let engine = GgufEngine::new();
        engine.unload_if_idle(0);
        engine.unload_if_idle(1);
        engine.unload();
        assert!(engine.resident().is_none());
    }

    #[test]
    #[ignore]
    fn transcribe_local_gguf() {
        let path = std::env::var("LIPI_PARAKEET_GGUF").expect("LIPI_PARAKEET_GGUF");
        let engine = GgufEngine::new();
        let pcm = vec![0.0f32; 16_000];
        let text = engine
            .transcribe_pcm(&pcm, Path::new(&path), None)
            .expect("transcribe");
        let _ = text;
    }
}
