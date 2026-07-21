mod app_server;
mod audio;
mod keyboard;
mod microphone;
mod model;
mod recognition;
mod scenario;
mod session;
mod speech;
mod vad;

use std::{
    fmt::Display,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context, Result};
use clap::Parser;

use crate::{
    app_server::CodexAppServer,
    keyboard::{KeyCommand, KeyboardListener},
    microphone::MicrophoneCapture,
    model::ensure_models,
    recognition::WhisperTranscriber,
    scenario::Scenario,
    session::Session,
    speech::Speaker,
    vad::SileroVad,
};

#[derive(Debug, Parser)]
#[command(
    name = "what-sorry",
    about = "Ask again without fear. Keep the conversation going."
)]
pub struct Cli {
    #[arg(short, long, value_enum, default_value_t = Scenario::Cafe)]
    scenario: Scenario,
    #[arg(long)]
    show_text: bool,
}

pub fn run(cli: Cli) -> Result<()> {
    whisper_rs::install_logging_hooks();
    let paths = ensure_models()?;
    let mut transcriber = WhisperTranscriber::load(&paths.recognition)?;
    let detector = SileroVad::load(&paths.vad)?;
    let mut capture = MicrophoneCapture::start(detector)?;
    let mut server = CodexAppServer::start(cli.scenario)?;
    let speaker = Speaker::new();
    let mut session = Session::new(cli.scenario, cli.show_text);
    let keyboard = KeyboardListener::start()?;
    let mut latest_phrase_display = LatestPhraseDisplay::default();
    let running = Arc::new(AtomicBool::new(true));
    let signal_running = Arc::clone(&running);
    ctrlc::set_handler(move || signal_running.store(false, Ordering::Release))
        .context("Could not configure Ctrl-C handling")?;

    print_line(format!("What, Sorry? — {}", cli.scenario.title()))?;
    print_line("Speak after each prompt. Say 'What, sorry?' whenever you need help.")?;
    print_shortcuts()?;
    let opening = request_tutor(&mut latest_phrase_display, || server.opening(&session))?;
    speak_response(&speaker, &mut capture, &mut session, opening)?;
    while running.load(Ordering::Acquire) && !session.is_complete() {
        match keyboard.try_next() {
            Some(KeyCommand::ShowLatestTutorText) => {
                latest_phrase_display.toggle(session.latest_tutor_text())?
            }
            Some(KeyCommand::RepeatCurrentPhrase) => {
                let response = request_tutor(&mut latest_phrase_display, || {
                    server.respond(
                        &session,
                        "What, sorry?",
                        session::SessionEvent::RescueRequest,
                    )
                })?;
                speak_response(&speaker, &mut capture, &mut session, response)?;
            }
            Some(KeyCommand::Quit) => break,
            None => {}
        }
        let job = capture.receive_final(Duration::from_millis(250))?;
        let Some(job) = job else { continue };
        let Some(learner_text) = transcriber.transcribe(&job.samples)? else {
            continue;
        };
        latest_phrase_display.clear()?;
        print_line(format!("You: {learner_text}"))?;
        let event = session.classify(&learner_text);
        let response = request_tutor(&mut latest_phrase_display, || {
            server.respond(&session, &learner_text, event)
        })?;
        speak_response(&speaker, &mut capture, &mut session, response)?;
    }
    if session.is_complete() {
        print_line(format!("Conversation Rescues: {}", session.rescues()))?;
        speaker.speak(&session.reflection())?;
    }
    capture.stop()?;
    Ok(())
}

fn print_line(text: impl Display) -> Result<()> {
    let mut output = std::io::stdout().lock();
    write!(output, "\r{text}\r\n").context("Could not write terminal output")?;
    output.flush().context("Could not flush terminal output")
}

fn print_shortcuts() -> Result<()> {
    print_line("Keyboard shortcuts")?;
    print_line("  [w] Ask again / repeat the current AI phrase")?;
    print_line("  [s] Show or hide the latest AI text")?;
    print_line("  [q] Quit  •  [Ctrl-C] Quit")
}

#[derive(Default)]
struct LatestPhraseDisplay {
    visible: bool,
}

impl LatestPhraseDisplay {
    fn toggle(&mut self, text: Option<&str>) -> Result<()> {
        if self.visible {
            return self.clear();
        }
        let Some(text) = text else {
            print_line("AI has not spoken yet.")?;
            return Ok(());
        };
        print_line(format!("AI: {text}"))?;
        self.visible = true;
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        if self.visible {
            print!("\x1b[1A\r\x1b[2K");
            std::io::stdout()
                .flush()
                .context("Could not hide the latest AI phrase")?;
            self.visible = false;
        }
        Ok(())
    }
}

fn request_tutor<T>(
    latest_phrase_display: &mut LatestPhraseDisplay,
    request: impl FnOnce() -> Result<T>,
) -> Result<T> {
    latest_phrase_display.clear()?;
    let spinner = LoadingSpinner::start();
    let result = request();
    spinner.stop()?;
    print!("\r\x1b[2K");
    std::io::stdout()
        .flush()
        .context("Could not clear AI thinking indicator")?;
    result
}

const SPINNER_FRAMES: [&str; 4] = ["⠋", "⠙", "⠹", "⠸"];

struct LoadingSpinner {
    is_running: Arc<AtomicBool>,
    worker: JoinHandle<()>,
}

impl LoadingSpinner {
    fn start() -> Self {
        let is_running = Arc::new(AtomicBool::new(true));
        let worker_running = Arc::clone(&is_running);
        let worker = thread::spawn(move || {
            let mut frame_index = 0;
            while worker_running.load(Ordering::Acquire) {
                print!("\r{} AI is thinking...", SPINNER_FRAMES[frame_index]);
                let _ = std::io::stdout().flush();
                frame_index = (frame_index + 1) % SPINNER_FRAMES.len();
                thread::sleep(Duration::from_millis(120));
            }
        });
        Self { is_running, worker }
    }

    fn stop(self) -> Result<()> {
        self.is_running.store(false, Ordering::Release);
        self.worker
            .join()
            .map_err(|_| anyhow::anyhow!("AI thinking indicator stopped unexpectedly"))
    }
}

fn speak_response(
    speaker: &Speaker,
    capture: &mut MicrophoneCapture,
    session: &mut Session,
    response: app_server::TutorResponse,
) -> Result<()> {
    session.apply(&response)?;
    if let Some(text) = session.visible_text(&response) {
        print_line(text)?;
    }
    capture.pause()?;
    speaker.speak(&response.spoken_text)?;
    capture.resume()?;
    Ok(())
}
