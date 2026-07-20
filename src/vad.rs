use crate::audio::{SAMPLE_RATE, VoiceActivityDetector};
use anyhow::{Context, Result};
use std::path::Path;
use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

pub(crate) struct SileroVad {
    context: WhisperVadContext,
    params: WhisperVadParams,
}

impl SileroVad {
    pub(crate) fn load(model_path: &Path) -> Result<Self> {
        let path = model_path
            .to_str()
            .context("VAD model path is not valid UTF-8")?;
        let mut context_params = WhisperVadContextParams::default();
        context_params.set_n_threads(1);
        context_params.set_use_gpu(false);
        let context = WhisperVadContext::new(path, context_params)
            .with_context(|| format!("Could not load VAD model: {path}"))?;
        let mut params = WhisperVadParams::default();
        params.set_threshold(0.5);
        params.set_min_speech_duration(100);
        params.set_min_silence_duration(100);
        params.set_max_speech_duration(15.0);
        params.set_speech_pad(0);
        params.set_samples_overlap(0.0);
        Ok(Self { context, params })
    }
}

impl VoiceActivityDetector for SileroVad {
    fn has_recent_speech(&mut self, samples: &[f32]) -> Result<bool> {
        let segments = self
            .context
            .segments_from_samples(self.params, samples)
            .context("Voice activity detection failed")?;
        let threshold = (samples.len() as f32 * 100.0 / SAMPLE_RATE as f32 - 20.0).max(0.0);
        Ok(segments.into_iter().any(|segment| segment.end >= threshold))
    }
}
