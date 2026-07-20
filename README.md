# What, Sorry?

> Ask again without fear. Keep the conversation going.

What, Sorry? is a voice-first English conversation CLI for beginners. If a learner says “What, sorry?”, the current conversation pauses safely, the tutor helps them understand, and the learner returns to the same conversation instead of dropping out.

The initial scenarios are cafe ordering, shopping, and asking directions. The cafe order is the recommended demo.

## What happens in a rescue

1. The learner asks again: “What, sorry?"(or press `w` key)
2. The tutor repeats the previous phrase and reassures them.
3. Repeated requests progress to slower speech, simpler English, then an offer to reveal text.
4. “Got it”, “I understand”, or a relevant answer resumes the saved conversation.
5. The session ends with the number of `Conversation Rescues`, celebrating persistence rather than correctness.

## Requirements

- macOS 26 or later with microphone permission for the terminal application.
- Apple Silicon or another macOS environment supported by `whisper-rs` Metal.
- Rust 1.96+.
- Either the `codex` CLI or `ncodex` installed, working, logged in, and authorized to use `gpt-5.6-terra`.
- macOS `/usr/bin/say`; this is the only tutor-output channel by default.

The app uses the local `whisper-rs` + Metal / Silero VAD flow based on [taptext](https://github.com/tttol/taptext): 16 kHz mono audio, voice-activity segmentation, and local English transcription. Unlike taptext, it takes microphone input rather than system audio.

## Run

```bash
cargo run -- --scenario cafe
```

On first launch, confirm the model download. Models are stored in `~/Library/Caches/what-sorry/models` and their SHA-256 hashes are verified before use.

Other scenarios:

```bash
cargo run -- --scenario shop
cargo run -- --scenario directions
```

Optional modes:

```bash
cargo run -- --scenario cafe --show-text
```

`--show-text` displays tutor text throughout the session. By default it remains hidden; during a rescue, say “show text” to reveal the phrase. The terminal prints only recognized learner speech as `You: ...`; tutor responses are voice-only by default.

While the tutor prepares a response, the CLI shows an `AI is thinking...` indicator. Press `w` to request the same rescue behavior as “What, sorry?”. Press `s` at any time between turns to show the latest phrase spoken by the AI; press `s` again to hide it. Press `q`/Ctrl-C to quit.

## Permissions and troubleshooting

- **No microphone input:** Allow the host terminal app in System Settings → Privacy & Security → Microphone, then restart it.
- **Model download fails:** Check network access, then rerun from an interactive terminal so the download prompt can be answered.
- **Codex App Server fails to start:** The app tries `codex`, the official `~/.local/bin/codex` installation, `ncodex`, then the ChatGPT-bundled executable. The error prints each failed attempt and its cause.
- **Tutor response is unavailable:** The session reports the failure instead of silently advancing. Retry after resolving the Codex CLI issue.

## Architecture

- `microphone` streams microphone audio into a bounded queue and pauses it while the tutor speaks.
- `audio`, `vad`, `recognition`, and `model` implement the local taptext-style VAD + Whisper recognition pipeline.
- `app_server` launches `codex app-server` over stdio JSONL, keeps one Codex thread per learning session, validates streamed structured tutor responses, and never exposes normal transcript text by default.
- `session` owns rescue state, text visibility, milestone tracking, and the rescue total. `scenario` owns each scenario’s fixed goal and valid milestones.

## Verification

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

For a manual demo, start `cafe`, say an order, ask “What, sorry?” at least once, respond “Hot, please” after the repeated prompt, and complete the order. Confirm that the reflection reports at least one `Conversation Rescue`.
