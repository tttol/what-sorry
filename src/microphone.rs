use crate::audio::{SAMPLE_RATE, UtteranceJob, UtteranceKind, VadSegmenter, VoiceActivityDetector};
use anyhow::{Context, Result, bail};
use cpal::{
    Device, SampleFormat, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const RAW_AUDIO_QUEUE_CAPACITY: usize = 256;
const UTTERANCE_QUEUE_CAPACITY: usize = 8;

pub(crate) struct MicrophoneCapture {
    stream: Option<Stream>,
    jobs: Receiver<UtteranceJob>,
    worker: Option<JoinHandle<()>>,
    paused: Arc<Mutex<bool>>,
}

impl MicrophoneCapture {
    pub(crate) fn start<D: VoiceActivityDetector + 'static>(detector: D) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No microphone input device is available")?;
        let supported = device
            .default_input_config()
            .context("Could not read microphone configuration")?;
        let config = supported.config();
        let (sender, receiver) = mpsc::sync_channel(RAW_AUDIO_QUEUE_CAPACITY);
        let (job_sender, jobs) = mpsc::sync_channel(UTTERANCE_QUEUE_CAPACITY);
        let paused = Arc::new(Mutex::new(false));
        let worker = spawn_vad_worker(receiver, job_sender, detector);
        let stream = create_stream(
            &device,
            &config,
            supported.sample_format(),
            sender,
            Arc::clone(&paused),
        )?;
        stream.play().context(
            "Could not start microphone capture. Grant microphone permission and retry.",
        )?;
        Ok(Self {
            stream: Some(stream),
            jobs,
            worker: Some(worker),
            paused,
        })
    }

    pub(crate) fn receive_final(&self, timeout: Duration) -> Result<Option<UtteranceJob>> {
        loop {
            match self.jobs.recv_timeout(timeout) {
                Ok(job) if job.kind == UtteranceKind::Final => return Ok(Some(job)),
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => return Ok(None),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("Microphone processing stopped unexpectedly")
                }
            }
        }
    }

    pub(crate) fn pause(&mut self) -> Result<()> {
        *self
            .paused
            .lock()
            .map_err(|_| anyhow::anyhow!("Microphone pause state is unavailable"))? = true;
        self.stream
            .as_ref()
            .context("Microphone stream is not available")?
            .pause()
            .context("Could not pause microphone")
    }

    pub(crate) fn resume(&mut self) -> Result<()> {
        *self
            .paused
            .lock()
            .map_err(|_| anyhow::anyhow!("Microphone pause state is unavailable"))? = false;
        self.stream
            .as_ref()
            .context("Microphone stream is not available")?
            .play()
            .context("Could not resume microphone")
    }

    pub(crate) fn stop(&mut self) -> Result<()> {
        self.stream.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("Microphone worker panicked"))?;
        }
        Ok(())
    }
}

impl Drop for MicrophoneCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn spawn_vad_worker<D: VoiceActivityDetector + 'static>(
    receiver: Receiver<Vec<f32>>,
    sender: SyncSender<UtteranceJob>,
    detector: D,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut segmenter = VadSegmenter::new(detector);
        for samples in receiver {
            match segmenter.push(&samples) {
                Ok(jobs) => jobs.into_iter().for_each(|job| {
                    let _ = sender.try_send(job);
                }),
                Err(_) => break,
            }
        }
    })
}

fn create_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    sender: SyncSender<Vec<f32>>,
    paused: Arc<Mutex<bool>>,
) -> Result<Stream> {
    let rate = config.sample_rate.0;
    let channels = usize::from(config.channels);
    let error = |cause| eprintln!("microphone error: {cause}");
    match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _| send_mono(data, channels, rate, &sender, &paused),
            error,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _| {
                send_mono(
                    &data
                        .iter()
                        .map(|sample| f32::from(*sample) / f32::from(i16::MAX))
                        .collect::<Vec<_>>(),
                    channels,
                    rate,
                    &sender,
                    &paused,
                )
            },
            error,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            config,
            move |data: &[u16], _| {
                send_mono(
                    &data
                        .iter()
                        .map(|sample| (f32::from(*sample) / f32::from(u16::MAX)) * 2.0 - 1.0)
                        .collect::<Vec<_>>(),
                    channels,
                    rate,
                    &sender,
                    &paused,
                )
            },
            error,
            None,
        ),
        _ => bail!("Unsupported microphone sample format: {sample_format:?}"),
    }
    .context("Could not create microphone stream")
}

fn send_mono(
    data: &[f32],
    channels: usize,
    input_rate: u32,
    sender: &SyncSender<Vec<f32>>,
    paused: &Mutex<bool>,
) {
    let is_paused = paused.lock().map(|value| *value).unwrap_or(true);
    if is_paused {
        return;
    }
    let mono = data
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect::<Vec<_>>();
    let samples = resample_to_16khz(&mono, input_rate);
    if !samples.is_empty() {
        let _ = sender.try_send(samples);
    }
}

fn resample_to_16khz(input: &[f32], input_rate: u32) -> Vec<f32> {
    if input_rate as usize == SAMPLE_RATE {
        return input.to_vec();
    }
    let output_len = input.len().saturating_mul(SAMPLE_RATE) / input_rate as usize;
    (0..output_len)
        .map(|index| {
            let source = index as f64 * input_rate as f64 / SAMPLE_RATE as f64;
            let lower = source.floor() as usize;
            let upper = (lower + 1).min(input.len().saturating_sub(1));
            let fraction = (source - lower as f64) as f32;
            input[lower] * (1.0 - fraction) + input[upper] * fraction
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::resample_to_16khz;
    #[test]
    fn downsamples_to_sixteen_kilohertz() {
        // GIVEN
        let input = vec![0.5; 48_000];
        // WHEN
        let actual = resample_to_16khz(&input, 48_000);
        // THEN
        assert_eq!(actual.len(), 16_000);
    }
}
