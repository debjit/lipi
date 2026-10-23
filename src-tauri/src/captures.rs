use crate::db::{AudioCapture, Database};
use std::fs;
use std::path::{Path, PathBuf};

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub fn audio_ext(name: &str) -> String {
    match Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "mp3".into(),
        "m4a" | "mp4" => "m4a".into(),
        "ogg" => "ogg".into(),
        "webm" => "webm".into(),
        "flac" => "flac".into(),
        "mpeg" | "mpga" => "mpeg".into(),
        _ => "wav".into(),
    }
}

pub fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "mp3" | "mpeg" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "webm" => "audio/webm",
        "flac" => "audio/flac",
        _ => "audio/wav",
    }
}

pub fn is_wav_name(name: &str) -> bool {
    audio_ext(name) == "wav"
}

/// Copy audio into the app data folder and remember why transcription failed.
pub fn keep_failed(
    db: &Database,
    app_data_dir: &Path,
    name: &str,
    bytes: &[u8],
    error: &str,
) -> String {
    if bytes.is_empty() {
        return error.to_string();
    }
    let size = format_bytes(bytes.len() as u64);
    match store(db, app_data_dir, name, bytes, error, None, None) {
        Ok(_) => format!(
            "{error}\n\nFile size: {size}. The audio was saved. Switch provider and retry it from Saved audio."
        ),
        Err(save_err) => format!("{error}\n\nThe audio could not be saved ({size}): {save_err}"),
    }
}

/// Keep a failed meeting chunk so it can be retried into that meeting.
pub fn keep_meeting_chunk(
    db: &Database,
    app_data_dir: &Path,
    session_id: i64,
    start_ms: i64,
    bytes: &[u8],
    error: &str,
) -> String {
    if bytes.is_empty() {
        return error.to_string();
    }
    let size = format_bytes(bytes.len() as u64);
    let name = format!("meeting-{session_id}-{start_ms}.wav");
    match store(db, app_data_dir, &name, bytes, error, Some(session_id), Some(start_ms)) {
        Ok(_) => format!(
            "{error}\n\nFile size: {size}. The audio was saved. Switch provider and retry it on this meeting."
        ),
        Err(save_err) => format!("{error}\n\nThe audio could not be saved ({size}): {save_err}"),
    }
}

pub fn store(
    db: &Database,
    app_data_dir: &Path,
    name: &str,
    bytes: &[u8],
    error: &str,
    session_id: Option<i64>,
    start_ms: Option<i64>,
) -> Result<AudioCapture, String> {
    let dir = app_data_dir.join("captures");
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create captures folder: {e}"))?;
    let ext = audio_ext(name);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let stored_name = format!("capture-{stamp}.{ext}");
    let path = dir.join(&stored_name);
    fs::write(&path, bytes).map_err(|e| format!("Failed to write audio: {e}"))?;
    let display = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&stored_name);
    match db.insert_audio_capture(
        display,
        &path.to_string_lossy(),
        bytes.len() as i64,
        error,
        session_id,
        start_ms,
    ) {
        Ok(capture) => Ok(capture),
        Err(e) => {
            let _ = fs::remove_file(&path);
            Err(e)
        }
    }
}

pub fn read_capture(capture: &AudioCapture) -> Result<(Vec<u8>, String), String> {
    let bytes = fs::read(&capture.path).map_err(|e| format!("Saved audio is missing: {e}"))?;
    let ext = audio_ext(&capture.path);
    Ok((bytes, mime_for_ext(&ext).to_string()))
}

pub fn remove_file(capture: &AudioCapture) {
    let path = PathBuf::from(&capture.path);
    if path.starts_with_component_captures() {
        let _ = fs::remove_file(&path);
    } else if capture.path.contains("captures") {
        let _ = fs::remove_file(&path);
    }
}

trait CapturePath {
    fn starts_with_component_captures(&self) -> bool;
}

impl CapturePath for PathBuf {
    fn starts_with_component_captures(&self) -> bool {
        self.components().any(|c| c.as_os_str() == "captures")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_and_extension() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(2048), "2 KB");
        assert_eq!(format_bytes(26_214_400), "25.0 MB");
        assert_eq!(audio_ext("Voice.MP3"), "mp3");
        assert_eq!(mime_for_ext("mp3"), "audio/mpeg");
        assert!(is_wav_name("recording.wav"));
        assert!(!is_wav_name("note.m4a"));
    }

    #[test]
    fn keep_failed_writes_a_row_and_file() {
        let dir = std::env::temp_dir().join(format!("lipi_captures_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = Database::new(dir.join("t.db")).unwrap();
        let msg = keep_failed(
            &db,
            &dir,
            "talk.mp3",
            b"not-really-audio",
            "API error: Maximum content size limit (26214400) exceeded",
        );
        assert!(msg.contains("Maximum content size limit"));
        assert!(msg.contains("saved"));
        let list = db.list_audio_captures().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "talk.mp3");
        assert!(Path::new(&list[0].path).is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}
