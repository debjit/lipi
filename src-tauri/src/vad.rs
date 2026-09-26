use ndarray::{Array2, Array3};
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;

const FRAME: usize = 512;
const CONTEXT: usize = 64;
const SPEECH_START: usize = 1440;
const SILENCE_END: usize = 9600;
const MIN_KEEP: usize = 4000;
const MAX_LEN: usize = 16_000 * 20;
const OVERLAP: usize = 3200;
const PREROLL_MAX: usize = 4800;

pub struct SileroVad {
    session: Session,
    state: Array3<f32>,
    context: Vec<f32>,
}

impl SileroVad {
    pub fn open(path: &Path) -> Result<Self, String> {
        let _ = ort::init().commit();
        let session = Session::builder()
            .map_err(|e| e.to_string())?
            .with_intra_threads(1)
            .map_err(|e| e.to_string())?
            .commit_from_file(path)
            .map_err(|e| format!("Failed to load VAD model: {}", e))?;
        Ok(Self {
            session,
            state: Array3::<f32>::zeros((2, 1, 128)),
            context: vec![0.0; CONTEXT],
        })
    }

    pub fn probability(&mut self, frame: &[f32]) -> Result<f32, String> {
        if frame.len() != FRAME {
            return Err(format!("VAD frame must be {FRAME} samples"));
        }
        let mut input = Vec::with_capacity(CONTEXT + FRAME);
        input.extend_from_slice(&self.context);
        input.extend_from_slice(frame);
        self.context.copy_from_slice(&input[input.len() - CONTEXT..]);

        let input_arr = Array2::from_shape_vec((1, CONTEXT + FRAME), input)
            .map_err(|e| e.to_string())?;
        let input_tensor = Tensor::from_array(input_arr).map_err(|e| e.to_string())?;
        let state_tensor = Tensor::from_array(self.state.clone()).map_err(|e| e.to_string())?;
        let sr_tensor = Tensor::from_array(ndarray::arr0(16000i64)).map_err(|e| e.to_string())?;

        let outputs = self
            .session
            .run(ort::inputs![
                "input" => input_tensor,
                "state" => state_tensor,
                "sr" => sr_tensor,
            ])
            .map_err(|e| format!("VAD inference failed: {}", e))?;

        let (_, probs) = outputs["output"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let prob = probs.first().copied().unwrap_or(0.0);

        let (_, next_state) = outputs["stateN"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        if next_state.len() == self.state.len() {
            self.state
                .iter_mut()
                .zip(next_state.iter())
                .for_each(|(dst, src)| *dst = *src);
        }
        Ok(prob)
    }
}

pub struct SpeechSegmenter {
    in_speech: bool,
    speech_samples: usize,
    silence_samples: usize,
    current: Vec<f32>,
    preroll: Vec<f32>,
}

impl SpeechSegmenter {
    pub fn new() -> Self {
        Self {
            in_speech: false,
            speech_samples: 0,
            silence_samples: 0,
            current: Vec::new(),
            preroll: Vec::new(),
        }
    }

    fn push_preroll(&mut self, frame: &[f32]) {
        self.preroll.extend_from_slice(frame);
        if self.preroll.len() > PREROLL_MAX {
            let drop_n = self.preroll.len() - PREROLL_MAX;
            self.preroll.drain(..drop_n);
        }
    }

    pub fn push(&mut self, frame: &[f32], speech: bool) -> Option<Vec<f32>> {
        if !self.in_speech {
            self.push_preroll(frame);
            if speech {
                self.speech_samples += frame.len();
                if self.speech_samples >= SPEECH_START {
                    self.in_speech = true;
                    self.current = self.preroll.clone();
                    self.silence_samples = 0;
                }
            } else {
                self.speech_samples = 0;
            }
            return None;
        }

        self.current.extend_from_slice(frame);
        if speech {
            self.silence_samples = 0;
        } else {
            self.silence_samples += frame.len();
        }
        if self.current.len() >= MAX_LEN {
            return self.cut_max();
        }
        if self.silence_samples >= SILENCE_END {
            return self.finish();
        }
        None
    }

    pub fn push_tail(&mut self, samples: &[f32]) {
        if self.in_speech {
            self.current.extend_from_slice(samples);
        }
    }

    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if self.in_speech {
            self.finish()
        } else {
            None
        }
    }

    fn finish(&mut self) -> Option<Vec<f32>> {
        let seg = std::mem::take(&mut self.current);
        self.in_speech = false;
        self.speech_samples = 0;
        self.silence_samples = 0;
        self.preroll.clear();
        if seg.len() < MIN_KEEP {
            None
        } else {
            Some(seg)
        }
    }

    fn cut_max(&mut self) -> Option<Vec<f32>> {
        if self.current.len() <= OVERLAP {
            return None;
        }
        let overlap = self.current.split_off(self.current.len() - OVERLAP);
        let seg = std::mem::replace(&mut self.current, overlap);
        self.silence_samples = 0;
        if seg.len() < MIN_KEEP {
            None
        } else {
            Some(seg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segmenter_emits_one_span_for_speech_between_silence() {
        let mut seg = SpeechSegmenter::new();
        let frame = vec![0.2f32; FRAME];
        for _ in 0..8 {
            assert!(seg.push(&frame, false).is_none());
        }
        let mut got = None;
        for i in 0..40 {
            if let Some(audio) = seg.push(&frame, i < 16) {
                assert!(got.is_none(), "expected a single segment");
                got = Some(audio);
            }
        }
        let audio = got.expect("silence after speech should close a segment");
        assert!(audio.len() >= MIN_KEEP);
        assert!(audio.len() < MAX_LEN);
    }

    #[test]
    fn segmenter_flush_keeps_open_speech() {
        let mut seg = SpeechSegmenter::new();
        let frame = vec![0.2f32; FRAME];
        for _ in 0..12 {
            assert!(seg.push(&frame, true).is_none());
        }
        let audio = seg.flush().expect("open speech should flush");
        assert!(audio.len() >= MIN_KEEP);
    }

    #[test]
    fn silero_scores_silence_when_model_present() {
        let path = std::env::var("LIPI_VAD_MODEL").ok().map(std::path::PathBuf::from);
        let Some(path) = path.filter(|p| p.is_file()) else {
            return;
        };
        let mut vad = SileroVad::open(&path).expect("load vad");
        let silence = vec![0.0f32; FRAME];
        let prob = vad.probability(&silence).expect("prob");
        assert!(prob < 0.8, "silence probability {prob}");
    }
}
