use crate::audio::{MeetingAudio, TrackId};
use crate::db::Database;
use crate::vad::ort_environment;
use parakeet_rs::{ExecutionConfig, ParakeetEOU, ParakeetEOUHandle, ParakeetTDT, Transcriber};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

const CHUNK: usize = 2560;
/// Live call audio older than this is skipped so a busy mic still gets the model.
const CALL_LAG: usize = 16_000 * 2;
const CALL_KEEP: usize = 16_000;
const MAX_TDT: usize = 16_000 * 180;
const TDT_OVERLAP: usize = 16_000 * 2;
const TDT_SEARCH: usize = 16_000 * 20;
const ECHO_WINDOW_MS: u64 = 20_000;

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "so", "to", "of", "in", "on", "it", "is", "was", "i",
    "you", "we", "they", "that", "this", "like", "just", "know", "yeah", "ok", "okay", "right",
    "mean", "my", "me",
];

pub fn eou_dir(models_dir: &Path) -> PathBuf {
    models_dir.join("parakeet-eou")
}

pub fn tdt_dir(models_dir: &Path) -> PathBuf {
    models_dir.join("parakeet-tdt")
}

pub fn eou_ready(models_dir: &Path) -> bool {
    let dir = eou_dir(models_dir);
    ["encoder.onnx", "decoder_joint.onnx", "tokenizer.json"]
        .iter()
        .all(|name| file_ok(&dir.join(name)))
}

pub fn tdt_ready(models_dir: &Path) -> bool {
    let dir = tdt_dir(models_dir);
    [
        "encoder-model.onnx",
        "encoder-model.onnx.data",
        "decoder_joint-model.onnx",
        "vocab.txt",
    ]
    .iter()
    .all(|name| file_ok(&dir.join(name)))
}

fn file_ok(path: &Path) -> bool {
    std::fs::metadata(path).map(|meta| meta.len() > 1000).unwrap_or(false)
}

pub fn require_meeting_models(models_dir: &Path) -> Result<(), String> {
    if eou_ready(models_dir) && tdt_ready(models_dir) {
        Ok(())
    } else {
        Err("Download Parakeet in Settings before starting a meeting.".into())
    }
}

fn execution() -> Result<ExecutionConfig, String> {
    let env = ort_environment()?.clone();
    Ok(ExecutionConfig::new().with_environment(env))
}

/// Append a streaming delta. `true` means the model closed the utterance.
pub fn push_delta(line: &mut String, chunk: &str) -> bool {
    let (delta, closed) = match chunk.strip_suffix(" [EOU]") {
        Some(head) => (head, true),
        None => (chunk, false),
    };
    // The streaming tokenizer leaves the SentencePiece word mark in the text.
    line.push_str(&delta.replace('▁', " "));
    closed
}

pub fn is_mic_echo(mic_text: &str, remote_text: &str) -> bool {
    if remote_text.trim().is_empty() {
        return false;
    }
    let mic_words = words(mic_text);
    if mic_words.len() < 4 {
        return false;
    }
    let mic_content = content_words(&mic_words);
    let remote_words = words(remote_text);
    let remote_content = content_words(&remote_words);
    if mic_content.is_empty() || remote_content.is_empty() {
        return false;
    }
    let matched = mic_content.iter().filter(|word| remote_content.contains(*word)).count();
    if matched < 2 {
        return false;
    }
    // A few wrong words still count as the call. Drop the mic line when most of it is already there.
    (matched as f64) / mic_content.len() as f64 >= 0.75
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect()
}

fn content_words(words: &[String]) -> Vec<&str> {
    words
        .iter()
        .filter(|word| !STOPWORDS.contains(&word.as_str()))
        .map(|word| word.as_str())
        .collect()
}

/// Keep the part of `next` that is not already the end of `full`.
pub fn append_new_words(full: &mut String, next: &str) {
    let next = next.trim();
    if next.is_empty() {
        return;
    }
    if full.is_empty() {
        *full = next.to_string();
        return;
    }
    let prev: Vec<&str> = full.split_whitespace().collect();
    let new: Vec<&str> = next.split_whitespace().collect();
    let max = prev.len().min(new.len());
    let mut overlap = 0;
    for count in (1..=max).rev() {
        if prev[prev.len() - count..] == new[..count] {
            overlap = count;
            break;
        }
    }
    if overlap == 1 && new[0].chars().count() < 8 {
        overlap = 0;
    }
    let rest = new[overlap..].join(" ");
    if rest.is_empty() {
        return;
    }
    full.push(' ');
    full.push_str(&rest);
}

pub fn split_ranges(sample_count: usize, is_quiet: impl Fn(usize) -> bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < sample_count {
        let limit = (start + MAX_TDT).min(sample_count);
        if limit == sample_count {
            out.push((start, sample_count));
            break;
        }
        let search_from = limit.saturating_sub(TDT_SEARCH).max(start);
        let mut cut = limit;
        let mut at = limit;
        while at > search_from {
            if is_quiet(at) {
                cut = at;
                break;
            }
            at = at.saturating_sub(1600);
        }
        if cut <= start {
            cut = limit;
        }
        out.push((start, cut));
        let next = cut.saturating_sub(TDT_OVERLAP);
        start = if next <= start { cut } else { next };
    }
    out
}

pub(crate) fn region_quiet(pcm: &[f32], at: usize) -> bool {
    let win = 4800;
    let start = at.saturating_sub(win / 2);
    let end = (start + win).min(pcm.len());
    end - start >= win / 2 && pcm[start..end].iter().all(|sample| sample.abs() < 0.02)
}

fn ms(samples: usize) -> i64 {
    (samples as i64) * 1000 / 16_000
}

pub fn transcribe_pcm(models_dir: &Path, pcm: &[f32]) -> Result<Vec<(i64, i64, String)>, String> {
    if pcm.is_empty() {
        return Ok(Vec::new());
    }
    if !tdt_ready(models_dir) {
        return Err("Download Parakeet in Settings before transcribing.".into());
    }
    let mut model = ParakeetTDT::from_pretrained(tdt_dir(models_dir), Some(execution()?))
        .map_err(|err| err.to_string())?;
    transcribe_with(&mut model, pcm)
}

fn transcribe_with(model: &mut ParakeetTDT, pcm: &[f32]) -> Result<Vec<(i64, i64, String)>, String> {
    if pcm.is_empty() {
        return Ok(Vec::new());
    }
    let ranges = split_ranges(pcm.len(), |at| region_quiet(pcm, at));
    let mut carried = String::new();
    let mut out = Vec::new();
    for (start, end) in ranges {
        let piece = model
            .transcribe_samples(pcm[start..end].to_vec(), 16_000, 1, None)
            .map_err(|err| err.to_string())?;
        let mut joined = carried.clone();
        let before = joined.len();
        append_new_words(&mut joined, &piece.text);
        let added = joined[before..].trim();
        if !added.is_empty() {
            out.push((ms(start), ms(end), added.to_string()));
        }
        carried = piece.text;
    }
    Ok(out)
}

pub fn transcribe_wav_bytes(app_data_dir: &Path, models_folder: Option<&str>, wav: &[u8]) -> Result<String, String> {
    let models_dir = crate::models::resolve_models_dir(app_data_dir, models_folder);
    let pcm = wav_bytes_to_f32(wav)?;
    let pieces = transcribe_pcm(&models_dir, &pcm)?;
    let mut text = String::new();
    for (_, _, piece) in pieces {
        append_new_words(&mut text, &piece);
    }
    Ok(text)
}

fn wav_bytes_to_f32(bytes: &[u8]) -> Result<Vec<f32>, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut reader = hound::WavReader::new(cursor).map_err(|err| format!("Could not read WAV: {err}"))?;
    decode_wav(&mut reader)
}

pub fn read_wav_file(path: &Path) -> Result<Vec<f32>, String> {
    let mut reader = hound::WavReader::open(path).map_err(|err| format!("Could not read {}: {err}", path.display()))?;
    decode_wav(&mut reader)
}

fn decode_wav<R: std::io::Read>(reader: &mut hound::WavReader<R>) -> Result<Vec<f32>, String> {
    let spec = reader.spec();
    let samples = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|sample| sample.map(|value| value as f32 / 32768.0))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?,
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?,
    };
    let channels = spec.channels.max(1) as usize;
    let mono = if channels == 1 {
        samples
    } else {
        samples
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    if spec.sample_rate == 16_000 {
        Ok(mono)
    } else {
        Ok(resample(&mono, spec.sample_rate, 16_000))
    }
}

fn resample(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if src_rate == 0 || input.is_empty() {
        return Vec::new();
    }
    let ratio = src_rate as f64 / dst_rate as f64;
    let new_len = ((input.len() as f64) / ratio).floor() as usize;
    let mut output = Vec::with_capacity(new_len);
    for index in 0..new_len {
        let src = (index as f64) * ratio;
        let left = src.floor() as usize;
        let frac = (src - left as f64) as f32;
        let a = input[left.min(input.len() - 1)];
        let b = input[(left + 1).min(input.len() - 1)];
        output.push(a + frac * (b - a));
    }
    output
}

struct Track {
    pending: Vec<f32>,
    fed: u64,
    line: String,
    start_ms: u64,
    recent: Vec<(u64, u64, String)>,
    decoder: ParakeetEOU,
}

pub fn run_meeting(
    app: AppHandle,
    db: Arc<Database>,
    models_dir: PathBuf,
    session_dir: PathBuf,
    cancel: Arc<AtomicBool>,
    session_id: i64,
    call_only: bool,
    rx: Receiver<MeetingAudio>,
) {
    if call_only {
        run_call_only(app, db, models_dir, session_dir, cancel, session_id, rx);
        return;
    }
    eprintln!("[Parakeet Meeting] Starting streaming pipeline for session #{}", session_id);
    let loaded = (|| -> Result<(ParakeetEOUHandle, Track, Track), String> {
        let handle = ParakeetEOUHandle::from_pretrained(eou_dir(&models_dir), Some(execution()?))
            .map_err(|err| err.to_string())?;
        let mic = Track::new(ParakeetEOU::from_shared(&handle));
        let call = Track::new(ParakeetEOU::from_shared(&handle));
        Ok((handle, mic, call))
    })();
    let mut session = match loaded {
        Ok(parts) => {
            eprintln!("[Parakeet Meeting] Parakeet EOU models loaded successfully for session #{}", session_id);
            Some(parts)
        }
        Err(err) => {
            eprintln!("[Parakeet Meeting] Failed to load Parakeet EOU models: {err}");
            let _ = app.emit("meeting_error", err);
            None
        }
    };
    while let Ok(event) = rx.recv() {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let Some((handle, mic, call)) = session.as_mut() else {
            continue;
        };
        queue_pcm(mic, call, &app, session_id, event);
        while let Ok(extra) = rx.try_recv() {
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            queue_pcm(mic, call, &app, session_id, extra);
        }
        // Mic audio is decoded before call audio. Each track keeps its own samples.
        loop {
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            if let Err(err) = call.shed_if_behind(handle, &app, &db, session_id) {
                let _ = app.emit("meeting_error", err);
            }
            while call.pending.len() >= CHUNK && call.fed + (CHUNK as u64) < mic.fed {
                if let Err(err) = call.decode_one(&app, &db, session_id, TrackId::Call, &[]) {
                    let _ = app.emit("meeting_error", err);
                }
                retract_mic_echo(mic, call, &app, &db, session_id);
            }
            let stepped = if mic.pending.len() >= CHUNK {
                let remote = call_context(call);
                mic.decode_one(&app, &db, session_id, TrackId::Mic, &remote)
            } else if call.pending.len() >= CHUNK {
                let stepped = call.decode_one(&app, &db, session_id, TrackId::Call, &[]);
                retract_mic_echo(mic, call, &app, &db, session_id);
                stepped
            } else {
                break;
            };
            if let Err(err) = stepped {
                let _ = app.emit("meeting_error", err);
            }
            while let Ok(extra) = rx.try_recv() {
                if cancel.load(Ordering::SeqCst) {
                    return;
                }
                queue_pcm(mic, call, &app, session_id, extra);
            }
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return;
    }
    if let Some((handle, mut mic, mut call)) = session.take() {
        let _ = call.shed_if_behind(&handle, &app, &db, session_id);
        let _ = call.push(&app, &db, session_id, TrackId::Call, &[], true, &[]);
        let remote = call_context(&call);
        let _ = mic.push(&app, &db, session_id, TrackId::Mic, &[], true, &remote);
        drop(mic);
        drop(call);
        drop(handle);
    }
    if let Err(err) = rewrite_saved(&app, &db, &models_dir, &session_dir, session_id) {
        let _ = app.emit(
            "meeting_error",
            format!("The live lines were kept. The saved transcript could not be rewritten: {err}"),
        );
    }
}

fn run_call_only(
    app: AppHandle,
    db: Arc<Database>,
    models_dir: PathBuf,
    session_dir: PathBuf,
    cancel: Arc<AtomicBool>,
    session_id: i64,
    rx: Receiver<MeetingAudio>,
) {
    let mut call = match (|| -> Result<Track, String> {
        let handle = ParakeetEOUHandle::from_pretrained(eou_dir(&models_dir), Some(execution()?))
            .map_err(|err| err.to_string())?;
        Ok(Track::new(ParakeetEOU::from_shared(&handle)))
    })() {
        Ok(call) => Some(call),
        Err(err) => {
            let _ = app.emit("meeting_error", err);
            None
        }
    };
    while let Ok(event) = rx.recv() {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let Some(call) = call.as_mut() else {
            continue;
        };
        take_call(&app, session_id, call, event);
        while let Ok(extra) = rx.try_recv() {
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            take_call(&app, session_id, call, extra);
        }
        while call.pending.len() >= CHUNK {
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            if let Err(err) = call.decode_one(&app, &db, session_id, TrackId::Call, &[]) {
                let _ = app.emit("meeting_error", err);
            }
            while let Ok(extra) = rx.try_recv() {
                if cancel.load(Ordering::SeqCst) {
                    return;
                }
                take_call(&app, session_id, call, extra);
            }
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return;
    }
    if let Some(mut call) = call.take() {
        let _ = call.push(&app, &db, session_id, TrackId::Call, &[], true, &[]);
        drop(call);
    }
    if let Err(err) = rewrite_saved(&app, &db, &models_dir, &session_dir, session_id) {
        let _ = app.emit(
            "meeting_error",
            format!("The live lines were kept. The saved transcript could not be rewritten: {err}"),
        );
    }
}

fn take_call(app: &AppHandle, session_id: i64, call: &mut Track, event: MeetingAudio) {
    match event {
        MeetingAudio::Limit => {
            let _ = app.emit("meeting_limit", session_id);
        }
        MeetingAudio::Chunk { .. } => {}
        MeetingAudio::Pcm { source, pcm } if source == TrackId::Call => call.pending.extend_from_slice(&pcm),
        MeetingAudio::Pcm { .. } => {}
    }
}

fn queue_pcm(mic: &mut Track, call: &mut Track, app: &AppHandle, session_id: i64, event: MeetingAudio) {
    match event {
        MeetingAudio::Limit => {
            let _ = app.emit("meeting_limit", session_id);
        }
        MeetingAudio::Chunk { .. } => {}
        MeetingAudio::Pcm { source, pcm } => {
            if source == TrackId::Mic {
                mic.pending.extend_from_slice(&pcm);
            } else {
                call.pending.extend_from_slice(&pcm);
            }
        }
    }
}

impl Track {
    fn new(decoder: ParakeetEOU) -> Self {
        Self {
            pending: Vec::new(),
            fed: 0,
            line: String::new(),
            start_ms: 0,
            recent: Vec::new(),
            decoder,
        }
    }

    fn push(
        &mut self,
        app: &AppHandle,
        db: &Database,
        session_id: i64,
        source: TrackId,
        pcm: &[f32],
        flush: bool,
        call_recent: &[(u64, u64, String)],
    ) -> Result<(), String> {
        self.pending.extend_from_slice(pcm);
        while self.pending.len() >= CHUNK {
            self.decode_one(app, db, session_id, source, call_recent)?;
        }
        if flush && !self.pending.is_empty() {
            let mut tail = std::mem::take(&mut self.pending);
            tail.resize(CHUNK, 0.0);
            self.pending = tail;
            self.decode_one(app, db, session_id, source, call_recent)?;
        }
        if flush {
            self.commit(app, db, session_id, source, call_recent)?;
        }
        Ok(())
    }

    fn decode_one(
        &mut self,
        app: &AppHandle,
        db: &Database,
        session_id: i64,
        source: TrackId,
        call_recent: &[(u64, u64, String)],
    ) -> Result<(), String> {
        if self.pending.len() < CHUNK {
            return Ok(());
        }
        let chunk: Vec<f32> = self.pending.drain(..CHUNK).collect();
        self.feed(app, db, session_id, source, &chunk, call_recent)
    }

    /// ponytail: the live call line drops audio older than 2s when the mic needs the model.
    /// The recording still has it; the saved transcript rewrites the call after stop.
    fn shed_if_behind(
        &mut self,
        handle: &ParakeetEOUHandle,
        app: &AppHandle,
        db: &Database,
        session_id: i64,
    ) -> Result<(), String> {
        if self.pending.len() <= CALL_LAG {
            return Ok(());
        }
        self.commit(app, db, session_id, TrackId::Call, &[])?;
        let skip = self.pending.len() - CALL_KEEP;
        self.fed += skip as u64;
        self.pending.drain(..skip);
        self.line.clear();
        self.decoder = ParakeetEOU::from_shared(handle);
        let _ = app.emit(
            "meeting_partial",
            PartialEvent {
                session_id,
                source: TrackId::Call.as_str(),
                start_ms: 0,
                text: String::new(),
            },
        );
        Ok(())
    }

    fn feed(
        &mut self,
        app: &AppHandle,
        db: &Database,
        session_id: i64,
        source: TrackId,
        chunk: &[f32],
        call_recent: &[(u64, u64, String)],
    ) -> Result<(), String> {
        self.fed += chunk.len() as u64;
        let end_ms = self.fed * 1000 / 16_000;
        let text = self.decoder.transcribe(chunk, true).map_err(|err| err.to_string())?;
        let was_empty = self.line.trim().is_empty();
        let closed = push_delta(&mut self.line, &text);
        if was_empty && !self.line.trim().is_empty() {
            self.start_ms = end_ms.saturating_sub(1_000);
        }
        if closed {
            self.commit(app, db, session_id, source, call_recent)?;
        } else if !self.line.trim().is_empty() {
            let text = self.line.trim();
            let show = if source == TrackId::Mic && is_mic_echo(text, &recent_text(call_recent, self.start_ms)) {
                String::new()
            } else {
                text.to_string()
            };
            let _ = app.emit(
                "meeting_partial",
                PartialEvent {
                    session_id,
                    source: source.as_str(),
                    start_ms: self.start_ms,
                    text: show,
                },
            );
        }
        Ok(())
    }

    fn commit(
        &mut self,
        app: &AppHandle,
        db: &Database,
        session_id: i64,
        source: TrackId,
        call_recent: &[(u64, u64, String)],
    ) -> Result<(), String> {
        let text = self.line.trim().to_string();
        self.line.clear();
        if text.is_empty() {
            return Ok(());
        }
        let end_ms = self.fed * 1000 / 16_000;
        if source == TrackId::Mic && is_mic_echo(&text, &recent_text(call_recent, self.start_ms)) {
            let _ = app.emit(
                "meeting_partial",
                PartialEvent {
                    session_id,
                    source: source.as_str(),
                    start_ms: 0,
                    text: String::new(),
                },
            );
            return Ok(());
        }
        self.recent.push((self.start_ms, end_ms, text.clone()));
        self.recent.retain(|(_, end, _)| end.saturating_add(ECHO_WINDOW_MS) >= self.start_ms);
        eprintln!("[Parakeet Meeting] [{}] Line: {}", source.as_str(), text);
        let saved = db.insert_segment(session_id, self.start_ms as i64, end_ms as i64, source.as_str(), &text)?;
        let _ = app.emit(
            "meeting_segment",
            SegmentEvent {
                session_id,
                start_ms: saved.start_ms as u64,
                end_ms: saved.end_ms as u64,
                text: saved.text,
                source: source.as_str().to_string(),
            },
        );
        let _ = app.emit(
            "meeting_partial",
            PartialEvent {
                session_id,
                source: source.as_str(),
                start_ms: 0,
                text: String::new(),
            },
        );
        Ok(())
    }
}

fn call_context(call: &Track) -> Vec<(u64, u64, String)> {
    let mut lines = call.recent.clone();
    let open = call.line.trim();
    if !open.is_empty() {
        lines.push((call.start_ms, call.fed * 1000 / 16_000, open.to_string()));
    }
    lines
}

fn retract_mic_echo(mic: &mut Track, call: &Track, app: &AppHandle, db: &Database, session_id: i64) {
    let context = call_context(call);
    let pending = std::mem::take(&mut mic.recent);
    let mut kept = Vec::new();
    for (start, end, text) in pending {
        if is_mic_echo(&text, &recent_text(&context, start)) {
            let _ = db.delete_mic_line(session_id, start as i64, &text);
            let _ = app.emit(
                "meeting_segment_drop",
                SegmentDrop {
                    session_id,
                    source: "mic",
                    start_ms: start,
                    text,
                },
            );
        } else {
            kept.push((start, end, text));
        }
    }
    mic.recent = kept;
}

fn recent_text(lines: &[(u64, u64, String)], at_ms: u64) -> String {
    let window_start = at_ms.saturating_sub(ECHO_WINDOW_MS);
    let window_end = at_ms.saturating_add(ECHO_WINDOW_MS);
    lines
        .iter()
        .filter(|(start, end, _)| *end >= window_start && *start <= window_end)
        .map(|(_, _, text)| text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(serde::Serialize, Clone)]
struct SegmentDrop {
    session_id: i64,
    source: &'static str,
    start_ms: u64,
    text: String,
}

#[derive(serde::Serialize, Clone)]
struct SegmentEvent {
    session_id: i64,
    start_ms: u64,
    end_ms: u64,
    text: String,
    source: String,
}

#[derive(serde::Serialize, Clone)]
struct PartialEvent {
    session_id: i64,
    source: &'static str,
    start_ms: u64,
    text: String,
}

fn rewrite_saved(
    app: &AppHandle,
    db: &Database,
    models_dir: &Path,
    session_dir: &Path,
    session_id: i64,
) -> Result<(), String> {
    let _ = app.emit("meeting_transcribing", true);
    let mic = read_optional(&session_dir.join("mic.wav"))?;
    let call = read_optional(&session_dir.join("call.wav"))?;
    if mic.is_empty() && call.is_empty() {
        let _ = app.emit("meeting_transcribing", false);
        return Ok(());
    }
    let mut model = ParakeetTDT::from_pretrained(tdt_dir(models_dir), Some(execution()?))
        .map_err(|err| err.to_string())?;
    let mic_lines = transcribe_with(&mut model, &mic)?;
    let call_lines = transcribe_with(&mut model, &call)?;
    let rows = drop_echo_lines(mic_lines, call_lines);
    if !rows.is_empty() {
        db.replace_segments(session_id, &rows)?;
    }
    let _ = app.emit("meeting_transcribing", false);
    Ok(())
}

fn read_optional(path: &Path) -> Result<Vec<f32>, String> {
    if path.is_file() {
        read_wav_file(path)
    } else {
        Ok(Vec::new())
    }
}

pub fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if ch == '.' || ch == '?' || ch == '!' || ch == '\n' {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                sentences.push(trimmed.to_string());
            }
            current.clear();
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_string());
    }
    sentences
}

pub fn is_sentence_echo(sentence: &str, remote_content: &[&str]) -> bool {
    let s_words = words(sentence);
    if s_words.is_empty() {
        return false;
    }
    let s_content = content_words(&s_words);
    if s_content.is_empty() {
        return false;
    }
    let matched = s_content.iter().filter(|w| remote_content.contains(*w)).count();
    let ratio = matched as f64 / s_content.len() as f64;
    if s_content.len() == 1 {
        matched == 1
    } else {
        matched >= 2 && ratio >= 0.55
    }
}

pub fn filter_mic_sentences(mic_text: &str, remote_text: &str) -> String {
    let remote_words = words(remote_text);
    let remote_content = content_words(&remote_words);
    if remote_content.is_empty() {
        return mic_text.to_string();
    }

    let sentences = split_sentences(mic_text);
    let mut kept = Vec::new();
    for s in sentences {
        if !is_sentence_echo(&s, &remote_content) {
            kept.push(s);
        }
    }
    kept.join(" ")
}

fn drop_echo_lines(mic: Vec<(i64, i64, String)>, call: Vec<(i64, i64, String)>) -> Vec<(i64, i64, String, String)> {
    let mut rows = Vec::new();
    for (start, end, text) in &call {
        rows.push((*start, *end, "call".to_string(), text.clone()));
    }
    for (start, end, text) in mic {
        let remote = call
            .iter()
            .filter(|(call_start, call_end, _)| {
                *call_end + ECHO_WINDOW_MS as i64 >= start && *call_start <= start + ECHO_WINDOW_MS as i64
            })
            .map(|(_, _, text)| text.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        if remote.trim().is_empty() {
            rows.push((start, end, "mic".to_string(), text));
            continue;
        }

        if is_mic_echo(&text, &remote) {
            continue;
        }

        let filtered = filter_mic_sentences(&text, &remote);
        if !filtered.trim().is_empty() {
            rows.push((start, end, "mic".to_string(), filtered));
        }
    }
    rows.sort_by_key(|(start, _, _, _)| *start);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_line_grows_then_commits_once() {
        let mut line = String::new();
        assert!(!push_delta(&mut line, "we should "));
        assert!(!push_delta(&mut line, "ship it"));
        assert!(push_delta(&mut line, " today [EOU]"));
        assert_eq!(line.trim(), "we should ship it today");
    }

    #[test]
    fn sentencepiece_marks_become_spaces() {
        let mut line = String::new();
        assert!(!push_delta(&mut line, "▁let▁me▁escaping"));
        assert!(push_delta(&mut line, "▁up▁with▁the▁socket [EOU]"));
        assert_eq!(line.trim(), "let me escaping up with the socket");
        let mut glued = String::from("escap");
        push_delta(&mut glued, "ing");
        assert_eq!(glued, "escaping");
    }

    #[test]
    fn echo_overlap_is_dropped_and_a_third_is_kept() {
        let call = "the quarterly budget needs a full review before friday";
        let echo = "quarterly budget needs a full review before friday";
        assert!(is_mic_echo(echo, call));
        let other = "we should hire two more engineers next month";
        assert!(!is_mic_echo(other, call));
        let over = "quarterly budget needs a full review before friday lets decide tomorrow";
        assert!(!is_mic_echo(over, call));
        let heard = "slow down or is india smartphone market changing for good because for years a new phone was more than just a gadget it was a state of symbol people wanted the latest model not always for better features but to show off the latest tech that mindset maybe changing as prices rise more people are asking is this upgrade worth the money and i guess that is the real test for phone brands making new phones that people want and prices they can afford because in the end if you're all phone still works why spend more on a new one";
        let first = "slow down or as india's smartphone market changing from good as a new phone was more than just engadgeted it was a state of symbol people wanted the latest money not always for better features but to show off the latest tech that mindset may be changed";
        let second = "as price is rise more people are asking is this upgrade worth the money";
        let third = "and i guess that is a real test for both brands making new phones that people want and prices make an afford because in the end their old phone still works why spend more on a new one";
        assert!(is_mic_echo(first, heard));
        assert!(is_mic_echo(second, heard));
        assert!(is_mic_echo(third, heard));
    }

    #[test]
    fn a_long_track_splits_under_four_minutes_on_silence() {
        let total = 16_000 * 250;
        let quiet_at = 16_000 * 170;
        let ranges = split_ranges(total, |at| (at as i64 - quiet_at as i64).abs() < 8_000);
        assert!(ranges.len() >= 2);
        assert_eq!(ranges.first().map(|range| range.0), Some(0));
        assert_eq!(ranges.last().map(|range| range.1), Some(total));
        for (start, end) in &ranges {
            assert!(end - start <= MAX_TDT, "piece {} samples", end - start);
            assert!(start < end);
        }
        let hard = split_ranges(MAX_TDT + 16_000, |_| false);
        assert!(hard.iter().all(|(start, end)| end - start <= MAX_TDT));
        assert_eq!(hard.last().map(|range| range.1), Some(MAX_TDT + 16_000));
    }

    #[test]
    fn overlap_prefix_is_not_appended_twice() {
        let mut full = "ship the build on friday".to_string();
        append_new_words(&mut full, "on friday and tell the client");
        assert_eq!(full, "ship the build on friday and tell the client");
    }

    #[test]
    fn mixed_mic_echo_and_speech_filters_only_echo() {
        let call = "But that is only one part of the problem. The biggest factor driving the price surge is artificial intelligence. And I will tell you how. Every smartphone needs memory chips. They help your phone store photos, run apps, and handle everyday tasks. But these chips are getting expensive. The reason is the AI boom. That's right, the AI boom has made chips expensive.";
        let mic = "But that is only one part of the problem. The biggest factor right price search is artificial. And I will tend to ask. Every smartphone needs memory chips. They help your phone store photos, run apps, and handle everything. Yes, so that is quite handy if you ask me. But we can control it by something else. But also it's kind of non-negotiable. But yeah, let's see how things work. Everyday tasks. But these chips are getting expensive. The reason is the AI boom. That's right, the AI boom has made chips expensive.";

        let filtered = filter_mic_sentences(mic, call);
        // The user speech sentences should be kept
        assert!(filtered.contains("quite handy if you ask me"));
        assert!(filtered.contains("control it by something else"));
        assert!(filtered.contains("kind of non-negotiable"));
        // The echoed call sentences should NOT be in the filtered output
        assert!(!filtered.contains("price surge"));
        assert!(!filtered.contains("price search is artificial"));
        assert!(!filtered.contains("Every smartphone needs memory chips"));
        assert!(!filtered.contains("AI boom has made chips expensive"));
    }

    #[test]
    fn pure_mic_echo_is_completely_dropped() {
        let call = "The quarterly budget needs a full review before friday.";
        let mic = "Quarterly budget needs a full review before friday.";
        let filtered = filter_mic_sentences(mic, call);
        assert_eq!(filtered.trim(), "");
    }

    #[test]
    fn pure_user_speech_is_completely_kept() {
        let call = "The quarterly budget needs a full review before friday.";
        let mic = "I think we should schedule the marketing launch for next month.";
        let filtered = filter_mic_sentences(mic, call);
        assert_eq!(filtered.trim(), mic.trim());
    }

    #[test]
    fn drop_echo_lines_preserves_distinct_speech() {
        let call_lines = vec![
            (0, 45000, "The biggest factor driving the price surge is artificial intelligence. Every smartphone needs memory chips.".to_string()),
        ];
        let mic_lines = vec![
            (0, 45000, "The biggest factor right price search is artificial. Yes, so that is quite handy if you ask me. Every smartphone needs memory chips.".to_string()),
        ];
        let rows = drop_echo_lines(mic_lines, call_lines);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].2, "call");
        assert_eq!(rows[1].2, "mic");
        assert!(rows[1].3.contains("quite handy if you ask me"));
        assert!(!rows[1].3.contains("Every smartphone needs memory chips"));
    }
}
