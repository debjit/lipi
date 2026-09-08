use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::io::Cursor;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub struct AudioRecorder {
    stop_tx: Arc<Mutex<Option<Sender<()>>>>,
    join_handle: Arc<Mutex<Option<JoinHandle<Result<Vec<u8>, String>>>>>,
}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            stop_tx: Arc::new(Mutex::new(None)),
            join_handle: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.stop_tx
            .lock()
            .map(|tx| tx.is_some())
            .unwrap_or(false)
    }

    pub fn start(&self) -> Result<(), String> {
        let mut stop_tx_guard = self.stop_tx.lock().map_err(|e| e.to_string())?;
        if stop_tx_guard.is_some() {
            return Err("Recording is already in progress".into());
        }

        let (stop_sender, stop_receiver) = channel::<()>();

        // Pre-check audio device before spawning thread so immediate errors return cleanly
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "No default audio input device found".to_string())?;

        let supported_config = device
            .default_input_config()
            .map_err(|e| format!("Failed to query default input config: {}", e))?;

        let sample_rate = supported_config.sample_rate().0;
        let channels = supported_config.channels();
        let sample_format = supported_config.sample_format();

        let handle = thread::spawn(move || -> Result<Vec<u8>, String> {
            let recorded_samples = Arc::new(Mutex::new(Vec::<f32>::new()));
            let samples_clone = Arc::clone(&recorded_samples);

            let err_fn = |err| eprintln!("Audio stream error: {}", err);

            let stream = match sample_format {
                SampleFormat::F32 => device.build_input_stream(
                    &supported_config.into(),
                    move |data: &[f32], _| {
                        if let Ok(mut lock) = samples_clone.lock() {
                            lock.extend_from_slice(data);
                        }
                    },
                    err_fn,
                    None,
                ),
                SampleFormat::I16 => device.build_input_stream(
                    &supported_config.into(),
                    move |data: &[i16], _| {
                        if let Ok(mut lock) = samples_clone.lock() {
                            for &sample in data {
                                lock.push(sample as f32 / 32768.0);
                            }
                        }
                    },
                    err_fn,
                    None,
                ),
                SampleFormat::U16 => device.build_input_stream(
                    &supported_config.into(),
                    move |data: &[u16], _| {
                        if let Ok(mut lock) = samples_clone.lock() {
                            for &sample in data {
                                lock.push((sample as f32 - 32768.0) / 32768.0);
                            }
                        }
                    },
                    err_fn,
                    None,
                ),
                _ => return Err(format!("Unsupported sample format: {:?}", sample_format)),
            }
            .map_err(|e| format!("Failed to build input stream: {}", e))?;

            stream
                .play()
                .map_err(|e| format!("Failed to start recording: {}", e))?;

            // Wait until stop signal is sent
            let _ = stop_receiver.recv();

            // Drop stream to close microphone
            drop(stream);

            let raw_samples = recorded_samples
                .lock()
                .map_err(|e| e.to_string())?
                .clone();

            if raw_samples.is_empty() {
                return Err("No audio samples recorded".into());
            }

            // Downmix to mono
            let mono_samples = if channels > 1 {
                let ch = channels as usize;
                raw_samples
                    .chunks(ch)
                    .map(|chunk| chunk.iter().copied().sum::<f32>() / (chunk.len() as f32))
                    .collect::<Vec<f32>>()
            } else {
                raw_samples
            };

            // Resample to 16,000 Hz
            let target_rate = 16_000u32;
            let resampled = resample_linear(&mono_samples, sample_rate, target_rate);

            // Convert f32 to i16 PCM
            let pcm_16k: Vec<i16> = resampled
                .into_iter()
                .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect();

            // Encode as WAV
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: target_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let mut cursor = Cursor::new(Vec::new());
            let mut writer = hound::WavWriter::new(&mut cursor, spec)
                .map_err(|e| format!("Wav writer error: {}", e))?;

            for sample in pcm_16k {
                writer
                    .write_sample(sample)
                    .map_err(|e| format!("Write sample error: {}", e))?;
            }

            writer
                .finalize()
                .map_err(|e| format!("Finalize wav error: {}", e))?;

            Ok(cursor.into_inner())
        });

        *stop_tx_guard = Some(stop_sender);
        if let Ok(mut h_guard) = self.join_handle.lock() {
            *h_guard = Some(handle);
        }

        Ok(())
    }

    pub fn stop(&self) -> Result<Vec<u8>, String> {
        let stop_sender = {
            let mut guard = self.stop_tx.lock().map_err(|e| e.to_string())?;
            guard.take().ok_or_else(|| "Not currently recording".to_string())?
        };

        // Send stop signal
        let _ = stop_sender.send(());

        // Join background thread and retrieve result
        let handle = {
            let mut guard = self.join_handle.lock().map_err(|e| e.to_string())?;
            guard.take().ok_or_else(|| "Missing recording thread handle".to_string())?
        };

        match handle.join() {
            Ok(result) => result,
            Err(_) => Err("Audio recording thread panicked".into()),
        }
    }
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
        // Ratio 3:1, len should be floor(5 / 3) = 1
        assert_eq!(resampled.len(), 1);

        let input_long = vec![0.1f32; 48000];
        let resampled_long = resample_linear(&input_long, 48000, 16000);
        assert_eq!(resampled_long.len(), 16000);
        assert!((resampled_long[0] - 0.1).abs() < 1e-4);
    }
}

