use crate::{
    scenario::Scenario,
    session::{Session, SessionEvent},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    env,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RescueAction {
    None,
    Repeat,
    Slower,
    Simplify,
    OfferText,
    Resume,
}
impl RescueAction {
    pub(crate) fn stage(self) -> u8 {
        match self {
            Self::Repeat => 1,
            Self::Slower => 2,
            Self::Simplify => 3,
            Self::OfferText => 4,
            Self::None | Self::Resume => 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TutorResponse {
    pub(crate) spoken_text: String,
    pub(crate) learner_visible_text: Option<String>,
    pub(crate) rescue_action: RescueAction,
    pub(crate) milestone: String,
    pub(crate) show_text: bool,
    pub(crate) complete: bool,
}

impl TutorResponse {
    #[cfg(test)]
    pub(crate) fn test_response(
        rescue_action: RescueAction,
        milestone: &str,
        complete: bool,
    ) -> Self {
        Self {
            spoken_text: "You are doing great.".into(),
            learner_visible_text: None,
            rescue_action,
            milestone: milestone.into(),
            show_text: false,
            complete,
        }
    }
}

pub(crate) struct CodexAppServer {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    thread_id: String,
    request_id: u64,
    scenario: Scenario,
}

impl CodexAppServer {
    pub(crate) fn start(scenario: Scenario) -> Result<Self> {
        let commands = app_server_commands();
        let mut failures = Vec::new();
        for command in commands {
            match Self::start_with_command(&command, scenario) {
                Ok(server) => return Ok(server),
                Err(cause) => failures.push(format!("{command}: {cause:#}")),
            }
        }
        bail!(
            "Could not start Codex App Server. Tried:\n{}",
            failures.join("\n")
        )
    }

    fn start_with_command(command: &str, scenario: Scenario) -> Result<Self> {
        let mut child = Command::new(command)
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Could not start `{command} app-server`"))?;
        let input = child
            .stdin
            .take()
            .context("Codex App Server has no stdin")?;
        let output = BufReader::new(
            child
                .stdout
                .take()
                .context("Codex App Server has no stdout")?,
        );
        let mut server = Self {
            child,
            input,
            output,
            thread_id: String::new(),
            request_id: 0,
            scenario,
        };
        server.request("initialize", json!({ "clientInfo": { "name": "what_sorry", "title": "What, Sorry?", "version": env!("CARGO_PKG_VERSION") } }))?;
        server.notify("initialized", json!({}))?;
        let result = server.request("thread/start", json!({ "model": "gpt-5.6-terra", "approvalPolicy": "never", "sandbox": "read-only", "serviceName": "what-sorry" }))?;
        server.thread_id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .context("Codex App Server did not return a thread id")?
            .to_owned();
        Ok(server)
    }

    pub(crate) fn opening(&mut self, session: &Session) -> Result<TutorResponse> {
        self.turn(
            session,
            "Start the scenario with a short friendly greeting and the first question.",
            SessionEvent::LearnerReply,
        )
    }
    pub(crate) fn respond(
        &mut self,
        session: &Session,
        learner_text: &str,
        event: SessionEvent,
    ) -> Result<TutorResponse> {
        self.turn(session, learner_text, event)
    }

    fn turn(
        &mut self,
        session: &Session,
        learner_text: &str,
        event: SessionEvent,
    ) -> Result<TutorResponse> {
        let prompt = tutor_prompt(self.scenario, session, learner_text, event);
        let result = self.request(
            "turn/start",
            json!({
                "threadId": self.thread_id,
                "input": [{ "type": "text", "text": prompt }],
                "approvalPolicy": "never",
                "sandboxPolicy": { "type": "readOnly" },
                "outputSchema": tutor_response_schema()
            }),
        )?;
        let turn_id = result
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .context("Codex App Server did not return a turn id")?
            .to_owned();
        let response_text = self.read_turn(&turn_id)?;
        parse_tutor_response(&response_text)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.request_id += 1;
        let id = self.request_id;
        self.send(json!({ "method": method, "id": id, "params": params }))?;
        loop {
            let message = self.read_message()?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    bail!("Codex App Server error: {error}");
                }
                return message
                    .get("result")
                    .cloned()
                    .context("Codex App Server returned no result");
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(json!({ "method": method, "params": params }))
    }
    fn send(&mut self, message: Value) -> Result<()> {
        writeln!(self.input, "{}", serde_json::to_string(&message)?)
            .context("Could not write to Codex App Server")?;
        self.input
            .flush()
            .context("Could not flush Codex App Server input")
    }

    fn read_turn(&mut self, turn_id: &str) -> Result<String> {
        let mut text = String::new();
        loop {
            let message = self.read_message()?;
            let method = message.get("method").and_then(Value::as_str);
            if method == Some("item/agentMessage/delta")
                && let Some(delta) = message.pointer("/params/delta").and_then(Value::as_str)
            {
                text.push_str(delta);
            }
            if method == Some("item/completed")
                && let Some(item_text) =
                    message.pointer("/params/item/text").and_then(Value::as_str)
            {
                text = item_text.to_owned();
            }
            if method == Some("turn/completed")
                && message.pointer("/params/turn/id").and_then(Value::as_str) == Some(turn_id)
            {
                let status = message
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str)
                    .unwrap_or("failed");
                if status != "completed" {
                    bail!("Codex tutor turn did not complete: {status}");
                }
                return (!text.is_empty())
                    .then_some(text)
                    .context("Codex tutor turn contained no response text");
            }
        }
    }

    fn read_message(&mut self) -> Result<Value> {
        let mut line = String::new();
        if self
            .output
            .read_line(&mut line)
            .context("Could not read Codex App Server output")?
            == 0
        {
            bail!("Codex App Server stopped unexpectedly");
        }
        serde_json::from_str(line.trim()).context("Codex App Server returned invalid JSON")
    }
}

fn app_server_commands() -> Vec<String> {
    let mut commands = vec!["codex".to_owned()];
    if let Some(home) = env::var_os("HOME") {
        commands.push(
            std::path::PathBuf::from(home)
                .join(".local/bin/codex")
                .display()
                .to_string(),
        );
    }
    commands.push("ncodex".to_owned());
    commands.push("/Applications/ChatGPT.app/Contents/Resources/codex".to_owned());
    commands
}

impl Drop for CodexAppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tutor_prompt(
    scenario: Scenario,
    session: &Session,
    learner_text: &str,
    event: SessionEvent,
) -> String {
    let event_json = serde_json::to_string(&event).unwrap_or_else(|_| "\"learner_reply\"".into());
    format!(
        "You are the voice-only tutor in What, Sorry?, an English app for beginners. Never use tools, run commands, or edit files. Do not correct a learner for asking again. Return exactly one JSON object, with no Markdown and no prose before or after it. Schema: {{\"spoken_text\":string,\"learner_visible_text\":string|null,\"rescue_action\":\"none|repeat|slower|simplify|offer_text|resume\",\"milestone\":string,\"show_text\":boolean,\"complete\":boolean}}.\nScenario: {}. Goal: {}. Valid milestones: {}.\nCurrent milestone: {}. State: {:?}. Rescue stage: {}. Rescue count: {}.\nLearner event: {}. Learner speech: {:?}.\nIf the event is rescue_request, repeat the last tutor phrase at stage 1, make it slower at stage 2, simplify at stage 3, and at stage 4 offer text without showing it. If the event is show_text, set show_text true and include the exact phrase in learner_visible_text. If understanding_signal or a relevant answer arrives during rescue, use rescue_action resume and continue from the saved milestone. Keep spoken_text short, kind, and in English. Advance only one valid milestone when the learner intent satisfies it.",
        scenario.title(),
        scenario.goal(),
        scenario.milestones().join(", "),
        session.milestone(),
        session.state(),
        session.rescue_stage(),
        session.rescues(),
        event_json,
        learner_text
    )
}

fn tutor_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "spoken_text": { "type": "string" },
            "learner_visible_text": { "type": ["string", "null"] },
            "rescue_action": { "type": "string", "enum": ["none", "repeat", "slower", "simplify", "offer_text", "resume"] },
            "milestone": { "type": "string" },
            "show_text": { "type": "boolean" },
            "complete": { "type": "boolean" }
        },
        "required": ["spoken_text", "learner_visible_text", "rescue_action", "milestone", "show_text", "complete"],
        "additionalProperties": false
    })
}

fn parse_tutor_response(text: &str) -> Result<TutorResponse> {
    let candidate = text
        .trim()
        .strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(text.trim());
    let response: TutorResponse = serde_json::from_str(candidate)
        .context("Tutor response did not match the required JSON schema")?;
    if response.spoken_text.trim().is_empty() {
        bail!("Tutor response has empty spoken_text");
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::{RescueAction, app_server_commands, parse_tutor_response};
    use anyhow::Result;
    #[test]
    fn parses_a_strict_tutor_response() -> Result<()> {
        // GIVEN
        let input = r#"{"spoken_text":"Would you like it hot or iced?","learner_visible_text":null,"rescue_action":"repeat","milestone":"hot or iced","show_text":false,"complete":false}"#;
        // WHEN
        let actual = parse_tutor_response(input)?;
        // THEN
        assert_eq!(actual.rescue_action, RescueAction::Repeat);
        Ok(())
    }

    #[test]
    fn rejects_non_json_tutor_response() {
        // GIVEN
        let input = "Sure, I can help.";
        // WHEN
        let actual = parse_tutor_response(input);
        // THEN
        assert!(actual.is_err());
    }

    #[test]
    fn includes_the_official_local_installation_as_a_fallback() {
        // GIVEN
        let expected_suffix = ".local/bin/codex";

        // WHEN
        let actual = app_server_commands();

        // THEN
        assert!(
            actual
                .iter()
                .any(|command| command.ends_with(expected_suffix))
        );
    }
}
