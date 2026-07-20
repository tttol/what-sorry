use std::collections::VecDeque;

use anyhow::Result;

pub(crate) const SAMPLE_RATE: usize = 16_000;
const ANALYSIS_STEP_SAMPLES: usize = SAMPLE_RATE / 10;
const ANALYSIS_WINDOW_SAMPLES: usize = SAMPLE_RATE;
const PRE_ROLL_SAMPLES: usize = SAMPLE_RATE * 3 / 10;
const SPEECH_CONFIRMATION_STEPS: usize = 2;
const SILENCE_CONFIRMATION_STEPS: usize = 7;
const PARTIAL_INTERVAL_SAMPLES: usize = SAMPLE_RATE;

pub(crate) trait VoiceActivityDetector: Send {
    fn has_recent_speech(&mut self, samples: &[f32]) -> Result<bool>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UtteranceKind {
    Partial,
    Final,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UtteranceJob {
    pub(crate) kind: UtteranceKind,
    pub(crate) samples: Vec<f32>,
}

enum VadState {
    Idle {
        pre_roll: VecDeque<f32>,
        speech_steps: usize,
    },
    Speaking {
        samples: Vec<f32>,
        silence_steps: usize,
        next_partial_sample_count: usize,
    },
}

pub(crate) struct VadSegmenter<D> {
    detector: D,
    state: VadState,
    pending: Vec<f32>,
    analysis: VecDeque<f32>,
}

impl<D: VoiceActivityDetector> VadSegmenter<D> {
    pub(crate) fn new(detector: D) -> Self {
        Self {
            detector,
            state: VadState::Idle {
                pre_roll: VecDeque::with_capacity(PRE_ROLL_SAMPLES),
                speech_steps: 0,
            },
            pending: Vec::with_capacity(ANALYSIS_STEP_SAMPLES * 2),
            analysis: VecDeque::with_capacity(ANALYSIS_WINDOW_SAMPLES),
        }
    }

    pub(crate) fn push(&mut self, samples: &[f32]) -> Result<Vec<UtteranceJob>> {
        self.pending.extend_from_slice(samples);
        let count = self.pending.len() / ANALYSIS_STEP_SAMPLES * ANALYSIS_STEP_SAMPLES;
        let steps = self.pending[..count]
            .chunks_exact(ANALYSIS_STEP_SAMPLES)
            .map(<[f32]>::to_vec)
            .collect::<Vec<_>>();
        self.pending.drain(..count);
        steps
            .into_iter()
            .map(|step| self.process_step(&step))
            .collect::<Result<Vec<_>>>()
            .map(|jobs| jobs.into_iter().flatten().collect())
    }

    fn process_step(&mut self, step: &[f32]) -> Result<Vec<UtteranceJob>> {
        self.analysis.extend(step.iter().copied());
        if self.analysis.len() > ANALYSIS_WINDOW_SAMPLES {
            self.analysis
                .drain(..self.analysis.len() - ANALYSIS_WINDOW_SAMPLES);
        }
        let analysis = self.analysis.iter().copied().collect::<Vec<_>>();
        let has_speech = self.detector.has_recent_speech(&analysis)?;
        self.transition(step, has_speech)
    }

    fn transition(&mut self, step: &[f32], has_speech: bool) -> Result<Vec<UtteranceJob>> {
        let state = std::mem::replace(
            &mut self.state,
            VadState::Idle {
                pre_roll: VecDeque::with_capacity(PRE_ROLL_SAMPLES),
                speech_steps: 0,
            },
        );
        let (next_state, jobs) = match state {
            VadState::Idle {
                mut pre_roll,
                speech_steps,
            } => {
                retain_tail(&mut pre_roll, step, PRE_ROLL_SAMPLES);
                let next_speech_steps = if has_speech { speech_steps + 1 } else { 0 };
                if next_speech_steps < SPEECH_CONFIRMATION_STEPS {
                    (
                        VadState::Idle {
                            pre_roll,
                            speech_steps: next_speech_steps,
                        },
                        Vec::new(),
                    )
                } else {
                    (
                        VadState::Speaking {
                            samples: pre_roll.into_iter().collect(),
                            silence_steps: 0,
                            next_partial_sample_count: PARTIAL_INTERVAL_SAMPLES,
                        },
                        Vec::new(),
                    )
                }
            }
            VadState::Speaking {
                mut samples,
                silence_steps,
                mut next_partial_sample_count,
            } => {
                samples.extend_from_slice(step);
                let next_silence = if has_speech { 0 } else { silence_steps + 1 };
                if next_silence >= SILENCE_CONFIRMATION_STEPS {
                    samples.truncate(samples.len().saturating_sub(
                        (SILENCE_CONFIRMATION_STEPS * ANALYSIS_STEP_SAMPLES) - PRE_ROLL_SAMPLES,
                    ));
                    (
                        VadState::Idle {
                            pre_roll: VecDeque::new(),
                            speech_steps: 0,
                        },
                        vec![UtteranceJob {
                            kind: UtteranceKind::Final,
                            samples,
                        }],
                    )
                } else {
                    let jobs = if samples.len() >= next_partial_sample_count {
                        next_partial_sample_count += PARTIAL_INTERVAL_SAMPLES;
                        vec![UtteranceJob {
                            kind: UtteranceKind::Partial,
                            samples: samples.clone(),
                        }]
                    } else {
                        Vec::new()
                    };
                    (
                        VadState::Speaking {
                            samples,
                            silence_steps: next_silence,
                            next_partial_sample_count,
                        },
                        jobs,
                    )
                }
            }
        };
        self.state = next_state;
        Ok(jobs)
    }
}

fn retain_tail(buffer: &mut VecDeque<f32>, samples: &[f32], capacity: usize) {
    buffer.extend(samples.iter().copied());
    if buffer.len() > capacity {
        buffer.drain(..buffer.len() - capacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::collections::VecDeque;

    struct FakeVad {
        decisions: VecDeque<bool>,
    }
    impl VoiceActivityDetector for FakeVad {
        fn has_recent_speech(&mut self, _: &[f32]) -> Result<bool> {
            Ok(self.decisions.pop_front().unwrap_or(false))
        }
    }

    #[test]
    fn finalizes_after_seven_hundred_milliseconds_of_silence() -> Result<()> {
        // GIVEN
        let detector = FakeVad {
            decisions: [true, true]
                .into_iter()
                .chain(std::iter::repeat_n(false, 7))
                .collect(),
        };
        let mut segmenter = VadSegmenter::new(detector);
        let samples = vec![0.5; ANALYSIS_STEP_SAMPLES * 9];

        // WHEN
        let actual = segmenter.push(&samples)?;

        // THEN
        assert_eq!(
            actual.last().map(|job| job.kind),
            Some(UtteranceKind::Final)
        );
        Ok(())
    }
}
