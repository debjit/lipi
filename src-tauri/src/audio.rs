use crate::vad::{SileroVad, SpeechSegmenter};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::fs::File;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MEETING_MAX_SAMPLES: u64 = 16_000 * 60 * 120;

enum Job {
    Batch(JoinHandle<Result<Vec<u8>, String>>),
    Live(JoinHandle<()>),
    Meeting(JoinHandle<()>),
}

pub enum MeetingAudio {
    Chunk { pcm: Vec<f32>, start_ms: u64, end_ms: u64 },
    Limit,
}

pub struct AudioRecorder {
    stop_tx: Arc<Mutex<Option<Sender<bool>>>>,
    job: Arc<Mutex<Option<Job>>>,
    live: Arc<Mutex<bool>>,
    meeting: Arc<Mutex<bool>>,
}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            stop_tx: Arc::new(Mutex::new(None)),
            job: Arc::new(Mutex::new(None)),
            live: Arc::new(Mutex::new(false)),
            meeting: Arc::new(Mutex::new(false)),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.stop_tx
            .lock()
            .map(|tx| tx.is_some())
            .unwrap_or(false)
    }

    pub fn is_live(&self) -> bool {
        self.live.lock().map(|g| *g).unwrap_or(false)
    }

    pub fn is_meeting(&self) -> bool {
        self.meeting.lock().map(|g| *g).unwrap_or(false)
    }

    pub fn start(&self) -> Result<(), String> {
        self.ensure_idle()?;
        let (stop_sender, stop_receiver) = mpsc::channel::<bool>();
        let (device, config) = input_device()?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let sample_format = config.sample_format();

        let handle = thread::spawn(move || -> Result<Vec<u8>, String> {
            let recorded = Arc::new(Mutex::new(Vec::<f32>::new()));
            let samples = Arc::clone(&recorded);
            let stream = build_stream(device, config, sample_format, channels, samples)?;
            stream.play().map_err(|e| format!("Failed to start recording: {}", e))?;
            let _ = stop_receiver.recv();
            drop(stream);
            let raw = recorded.lock().map_err(|e| e.to_string())?.clone();
            if raw.is_empty() {
                return Err("No audio samples recorded".into());
            }
            let mono = downmix(&raw, channels);
            let resampled = resample_linear(&mono, sample_rate, 16_000);
            pcm_f32_to_wav(&resampled)
        });

        self.store_job(stop_sender, Job::Batch(handle), false, false)?;
        Ok(())
    }

    pub fn start_live(&self, model_path: PathBuf) -> Result<Receiver<Vec<f32>>, String> {
        self.ensure_idle()?;
        let (stop_sender, stop_receiver) = mpsc::channel::<bool>();
        let (seg_tx, seg_rx) = mpsc::channel::<Vec<f32>>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let (device, config) = input_device()?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let sample_format = config.sample_format();

        let handle = thread::spawn(move || {
            let mut ready = Some(ready_tx);
            let report = |ready: &mut Option<Sender<Result<(), String>>>, res: Result<(), String>| {
                if let Some(tx) = ready.take() {
                    let _ = tx.send(res);
                }
            };
            let mut vad = match SileroVad::open(&model_path) {
                Ok(vad) => vad,
                Err(e) => {
                    report(&mut ready, Err(e));
                    return;
                }
            };
            let recorded = Arc::new(Mutex::new(Vec::<f32>::new()));
            let samples = Arc::clone(&recorded);
            let stream = match build_stream(device, config, sample_format, channels, samples) {
                Ok(stream) => stream,
                Err(e) => {
                    report(&mut ready, Err(e));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                report(&mut ready, Err(format!("Failed to start recording: {}", e)));
                return;
            }
            report(&mut ready, Ok(()));
            let _ = live_loop(recorded, channels, sample_rate, &mut vad, &seg_tx, stop_receiver);
            drop(stream);
        });

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.store_job(stop_sender, Job::Live(handle), true, false)?;
                Ok(seg_rx)
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                let _ = handle.join();
                Err("VAD capture thread exited before the microphone opened".into())
            }
        }
    }

    pub fn stop(&self) -> Result<Vec<u8>, String> {
        self.signal(false)?;
        match self.take_job()? {
            Job::Batch(handle) => match handle.join() {
                Ok(result) => result,
                Err(_) => Err("Audio recording thread panicked".into()),
            },
            Job::Live(handle) => {
                let _ = handle.join();
                Err("Stop was called on a live dictation session".into())
            }
            Job::Meeting(handle) => {
                let _ = handle.join();
                Err("Stop was called on a meeting recording".into())
            }
        }
    }

    /// `flush` emits the open utterance. Cancel passes false and drops it.
    pub fn end_live(&self, flush: bool) -> Result<(), String> {
        self.signal(flush)?;
        match self.take_job()? {
            Job::Live(handle) => {
                let _ = handle.join();
                Ok(())
            }
            Job::Batch(handle) => {
                let _ = handle.join();
                Err("end_live was called without a live session".into())
            }
            Job::Meeting(handle) => {
                let _ = handle.join();
                Err("end_live was called on a meeting recording".into())
            }
        }
    }

    pub fn start_meeting(&self, wav_path: PathBuf, vad_path: PathBuf) -> Result<Receiver<MeetingAudio>, String> {
        self.ensure_idle()?;
        let (stop_sender, stop_receiver) = mpsc::channel::<bool>();
        let (seg_tx, seg_rx) = mpsc::channel::<MeetingAudio>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let (device, config) = input_device()?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let sample_format = config.sample_format();

        let handle = thread::spawn(move || {
            let mut ready = Some(ready_tx);
            let report = |ready: &mut Option<Sender<Result<(), String>>>, res: Result<(), String>| {
                if let Some(tx) = ready.take() {
                    let _ = tx.send(res);
                }
            };
            let mut vad = match SileroVad::open(&vad_path) {
                Ok(vad) => vad,
                Err(e) => {
                    report(&mut ready, Err(e));
                    return;
                }
            };
            let sink = match WavSink::create(&wav_path) {
                Ok(sink) => sink,
                Err(e) => {
                    report(&mut ready, Err(e));
                    return;
                }
            };
            let recorded = Arc::new(Mutex::new(Vec::<f32>::new()));
            let samples = Arc::clone(&recorded);
            let stream = match build_stream(device, config, sample_format, channels, samples) {
                Ok(stream) => stream,
                Err(e) => {
                    report(&mut ready, Err(e));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                report(&mut ready, Err(format!("Failed to start recording: {}", e)));
                return;
            }
            report(&mut ready, Ok(()));
            let _ = meeting_loop(
                recorded,
                channels,
                sample_rate,
                &mut vad,
                sink,
                &seg_tx,
                stop_receiver,
            );
            drop(stream);
        });

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.store_job(stop_sender, Job::Meeting(handle), false, true)?;
                Ok(seg_rx)
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                let _ = handle.join();
                Err("Meeting capture thread exited before the microphone opened".into())
            }
        }
    }

    /// `flush` keeps the open utterance. Cancel passes false and drops it.
    pub fn end_meeting(&self, flush: bool) -> Result<(), String> {
        if self
            .stop_tx
            .lock()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            self.signal(flush)?;
        } else if let Ok(mut meeting) = self.meeting.lock() {
            *meeting = false;
        }
        let job = self.job.lock().map_err(|e| e.to_string())?.take();
        match job {
            Some(Job::Meeting(handle)) => {
                let _ = handle.join();
                Ok(())
            }
            Some(other) => {
                *self.job.lock().map_err(|e| e.to_string())? = Some(other);
                Err("end_meeting was called without a meeting".into())
            }
            None => Ok(()),
        }
    }

    fn ensure_idle(&self) -> Result<(), String> {
        let guard = self.stop_tx.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Err("Recording is already in progress".into());
        }
        Ok(())
    }

    fn store_job(&self, stop_sender: Sender<bool>, job: Job, live: bool, meeting: bool) -> Result<(), String> {
        *self.stop_tx.lock().map_err(|e| e.to_string())? = Some(stop_sender);
        *self.job.lock().map_err(|e| e.to_string())? = Some(job);
        *self.live.lock().map_err(|e| e.to_string())? = live;
        *self.meeting.lock().map_err(|e| e.to_string())? = meeting;
        Ok(())
    }

    fn signal(&self, flush: bool) -> Result<(), String> {
        let sender = self
            .stop_tx
            .lock()
            .map_err(|e| e.to_string())?
            .take()
            .ok_or_else(|| "Not currently recording".to_string())?;
        let _ = sender.send(flush);
        if let Ok(mut live) = self.live.lock() {
            *live = false;
        }
        if let Ok(mut meeting) = self.meeting.lock() {
            *meeting = false;
        }
        Ok(())
    }

    fn take_job(&self) -> Result<Job, String> {
        self.job
            .lock()
            .map_err(|e| e.to_string())?
            .take()
            .ok_or_else(|| "Missing recording thread handle".to_string())
    }
}

fn input_device() -> Result<(cpal::Device, cpal::SupportedStreamConfig), String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No default audio input device found".to_string())?;
    let config = device
        .default_input_config()
        .map_err(|e| format!("Failed to query default input config: {}", e))?;
    Ok((device, config))
}

fn build_stream(
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
    sample_format: SampleFormat,
    _channels: u16,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Result<Stream, String> {
    let err_fn = |err| eprintln!("Audio stream error: {}", err);
    let stream_config = config.into();
    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _| {
                if let Ok(mut lock) = samples.lock() {
                    lock.extend_from_slice(data);
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _| {
                if let Ok(mut lock) = samples.lock() {
                    for &sample in data {
                        lock.push(sample as f32 / 32768.0);
                    }
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _| {
                if let Ok(mut lock) = samples.lock() {
                    for &sample in data {
                        lock.push((sample as f32 - 32768.0) / 32768.0);
                    }
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::I32 => device.build_input_stream(
            &stream_config,
            move |data: &[i32], _| {
                if let Ok(mut lock) = samples.lock() {
                    for &sample in data {
                        lock.push(sample as f32 / 2147483648.0);
                    }
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::U8 => device.build_input_stream(
            &stream_config,
            move |data: &[u8], _| {
                if let Ok(mut lock) = samples.lock() {
                    for &sample in data {
                        lock.push((sample as f32 - 128.0) / 128.0);
                    }
                }
            },
            err_fn,
            None,
        ),
        _ => return Err(format!("Unsupported sample format: {:?}", sample_format)),
    }
    .map_err(|e| format!("Failed to build input stream: {}", e))?;
    Ok(stream)
}

fn live_loop(
    recorded: Arc<Mutex<Vec<f32>>>,
    channels: u16,
    sample_rate: u32,
    vad: &mut SileroVad,
    seg_tx: &Sender<Vec<f32>>,
    stop_receiver: Receiver<bool>,
) -> Result<(), String> {
    let mut resampler = StreamResampler::new(sample_rate);
    let mut pending = Vec::<f32>::new();
    let mut segmenter = SpeechSegmenter::new();
    let mut carry = Vec::<f32>::new();

    loop {
        let stop = match stop_receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(flush) => Some(flush),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => Some(false),
        };
        let raw = {
            let mut lock = recorded.lock().map_err(|e| e.to_string())?;
            std::mem::take(&mut *lock)
        };
        if !raw.is_empty() {
            let mono = downmix(&raw, channels);
            pending.extend(resampler.push(&mono));
        }
        while pending.len() >= 512 {
            let frame: Vec<f32> = pending.drain(..512).collect();
            let speech = vad.probability(&frame).unwrap_or(0.0) >= 0.5;
            if let Some(seg) = segmenter.push(&frame, speech) {
                let _ = seg_tx.send(seg);
            }
        }
        if let Some(flush) = stop {
            if flush {
                if !pending.is_empty() {
                    carry.append(&mut pending);
                    segmenter.push_tail(&carry);
                }
                if let Some(seg) = segmenter.flush() {
                    let _ = seg_tx.send(seg);
                }
            }
            break;
        }
    }
    Ok(())
}

fn meeting_loop(
    recorded: Arc<Mutex<Vec<f32>>>,
    channels: u16,
    sample_rate: u32,
    vad: &mut SileroVad,
    mut sink: WavSink,
    seg_tx: &Sender<MeetingAudio>,
    stop_receiver: Receiver<bool>,
) -> Result<(), String> {
    let mut resampler = StreamResampler::new(sample_rate);
    let mut pending = Vec::<f32>::new();
    let mut segmenter = SpeechSegmenter::new();
    let mut fed: u64 = 0;
    let mut written: u64 = 0;

    let send_seg = |tx: &Sender<MeetingAudio>, seg: Vec<f32>, fed: u64| {
        if seg.is_empty() {
            return;
        }
        let end_ms = fed * 1000 / 16_000;
        let start_ms = end_ms.saturating_sub(seg.len() as u64 * 1000 / 16_000);
        let _ = tx.send(MeetingAudio::Chunk { pcm: seg, start_ms, end_ms });
    };

    loop {
        let stop = match stop_receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(flush) => Some(flush),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => Some(false),
        };
        let raw = {
            let mut lock = recorded.lock().map_err(|e| e.to_string())?;
            std::mem::take(&mut *lock)
        };
        if !raw.is_empty() {
            let mono = downmix(&raw, channels);
            let samples = resampler.push(&mono);
            if !samples.is_empty() {
                sink.write_f32(&samples)?;
                written += samples.len() as u64;
                pending.extend(samples);
            }
        }
        while pending.len() >= 512 {
            let frame: Vec<f32> = pending.drain(..512).collect();
            fed += frame.len() as u64;
            let speech = vad.probability(&frame).unwrap_or(0.0) >= 0.5;
            if let Some(seg) = segmenter.push(&frame, speech) {
                send_seg(seg_tx, seg, fed);
            }
        }
        let limit = written >= MEETING_MAX_SAMPLES;
        if limit || stop.is_some() {
            let flush = limit || stop == Some(true);
            if flush && !pending.is_empty() {
                let tail = std::mem::take(&mut pending);
                fed += tail.len() as u64;
                segmenter.push_tail(&tail);
            }
            if flush {
                if let Some(seg) = segmenter.flush() {
                    send_seg(seg_tx, seg, fed);
                }
            }
            if limit {
                let _ = seg_tx.send(MeetingAudio::Limit);
            }
            break;
        }
    }
    sink.finish()?;
    Ok(())
}

struct WavSink {
    file: File,
    data_bytes: u32,
    closed: bool,
}

impl WavSink {
    fn create(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create session folder: {}", e))?;
        }
        let mut file = File::create(path).map_err(|e| format!("Failed to create recording file: {}", e))?;
        write_wav_header(&mut file, 0)?;
        Ok(Self {
            file,
            data_bytes: 0,
            closed: false,
        })
    }

    fn write_f32(&mut self, samples: &[f32]) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        let mut buf = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            let pcm = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            buf.extend_from_slice(&pcm.to_le_bytes());
        }
        self.file
            .write_all(&buf)
            .map_err(|e| format!("Failed to write audio: {}", e))?;
        self.data_bytes = self.data_bytes.saturating_add(buf.len() as u32);
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.file.flush().map_err(|e| e.to_string())?;
        self.file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        write_wav_header(&mut self.file, self.data_bytes)
    }
}

impl Drop for WavSink {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn write_wav_header(file: &mut File, data_bytes: u32) -> Result<(), String> {
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(36u32.saturating_add(data_bytes)).to_le_bytes());
    header.extend_from_slice(b"WAVE");
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&16_000u32.to_le_bytes());
    header.extend_from_slice(&32_000u32.to_le_bytes());
    header.extend_from_slice(&2u16.to_le_bytes());
    header.extend_from_slice(&16u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_bytes.to_le_bytes());
    file.write_all(&header).map_err(|e| format!("Failed to write wav header: {}", e))?;
    Ok(())
}

fn downmix(raw: &[f32], channels: u16) -> Vec<f32> {
    if channels > 1 {
        let ch = channels as usize;
        raw.chunks(ch)
            .map(|chunk| chunk.iter().copied().sum::<f32>() / (chunk.len() as f32))
            .collect()
    } else {
        raw.to_vec()
    }
}

struct StreamResampler {
    src_rate: u32,
    pending: Vec<f32>,
    src_pos: f64,
}

impl StreamResampler {
    fn new(src_rate: u32) -> Self {
        Self {
            src_rate,
            pending: Vec::new(),
            src_pos: 0.0,
        }
    }

    fn push(&mut self, mono: &[f32]) -> Vec<f32> {
        if mono.is_empty() {
            return Vec::new();
        }
        if self.src_rate == 16_000 {
            return mono.to_vec();
        }
        self.pending.extend_from_slice(mono);
        let ratio = self.src_rate as f64 / 16_000.0;
        let mut out = Vec::new();
        while self.src_pos + 1.0 < self.pending.len() as f64 {
            let idx = self.src_pos.floor() as usize;
            let frac = (self.src_pos - idx as f64) as f32;
            let s0 = self.pending[idx];
            let s1 = self.pending[(idx + 1).min(self.pending.len() - 1)];
            out.push(s0 + frac * (s1 - s0));
            self.src_pos += ratio;
        }
        let drop_n = self.src_pos.floor() as usize;
        if drop_n > 0 && drop_n < self.pending.len() {
            self.pending.drain(..drop_n);
            self.src_pos -= drop_n as f64;
        }
        out
    }
}

pub fn pcm_f32_to_wav(samples: &[f32]) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut cursor, spec)
        .map_err(|e| format!("Wav writer error: {}", e))?;
    for sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        writer
            .write_sample(pcm)
            .map_err(|e| format!("Write sample error: {}", e))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("Finalize wav error: {}", e))?;
    Ok(cursor.into_inner())
}

fn resample_linear(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if src_rate == dst_rate || input.is_empty() {
        return input.to_vec();
    }
    let ratio = src_rate as f64 / dst_rate as f64;
    let new_len = ((input.len() as f64) / ratio).floor() as usize;
    let mut output = Vec::with_capacity(new_len);
    for i in 0..new_len {
        let src_idx = (i as f64) * ratio;
        let idx_floor = src_idx.floor() as usize;
        let frac = (src_idx - (idx_floor as f64)) as f32;
        let s0 = input[idx_floor.min(input.len() - 1)];
        let s1 = input[(idx_floor + 1).min(input.len() - 1)];
        output.push(s0 + frac * (s1 - s0));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resample_linear() {
        let input = vec![0.0, 0.5, 1.0, 0.5, 0.0];
        let resampled = resample_linear(&input, 48000, 16000);
        assert_eq!(resampled.len(), 1);

        let input_long = vec![0.1f32; 48000];
        let resampled_long = resample_linear(&input_long, 48000, 16000);
        assert_eq!(resampled_long.len(), 16000);
        assert!((resampled_long[0] - 0.1).abs() < 1e-4);
    }

    #[test]
    fn wav_sink_writes_a_readable_file() {
        let path = std::env::temp_dir().join(format!("lipi_wav_sink_{}.wav", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let mut sink = WavSink::create(&path).unwrap();
            sink.write_f32(&[0.0, 0.5, -0.5, 1.0]).unwrap();
            sink.finish().unwrap();
        }
        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16_000);
        let samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(samples.len(), 4);
        assert!(samples[1] > 0);
        assert!(samples[2] < 0);
        let _ = std::fs::remove_file(&path);
    }
}
