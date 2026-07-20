use anyhow::{Context, Result};
use std::path::Path;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

pub(crate) struct WhisperTranscriber {
    state: WhisperState,
    threads: i32,
}

impl WhisperTranscriber {
    pub(crate) fn load(model_path: &Path) -> Result<Self> {
        let context =
            WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
                .with_context(|| {
                    format!("Could not load Whisper model: {}", model_path.display())
                })?;
        let state = context
            .create_state()
            .context("Could not create Whisper state")?;
        let available = std::thread::available_parallelism().map_or(4, usize::from);
        let threads = i32::try_from(available.saturating_sub(2).max(1)).unwrap_or(i32::MAX);
        Ok(Self { state, threads })
    }

    pub(crate) fn transcribe(&mut self, samples: &[f32]) -> Result<Option<String>> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.threads);
        params.set_language(Some("en"));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_token_timestamps(false);
        self.state
            .full(params, samples)
            .context("Whisper transcription failed")?;
        let text = self
            .state
            .as_iter()
            .filter_map(|segment| segment.to_str().ok())
            .collect::<String>();
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        Ok((!normalized.is_empty() && !is_non_speech_token(&normalized)).then_some(normalized))
    }
}

fn is_non_speech_token(text: &str) -> bool {
    (text.starts_with('[') && text.ends_with(']')) || (text.starts_with('(') && text.ends_with(')'))
}

#[cfg(test)]
mod tests {
    use super::is_non_speech_token;

    #[test]
    fn recognizes_whisper_non_speech_tokens() {
        // GIVEN
        let input = ["[BLANK_AUDIO]", "(speaking in foreign language)"];

        // WHEN
        let actual = input.map(is_non_speech_token);

        // THEN
        assert_eq!(actual, [true, true]);
    }
}
