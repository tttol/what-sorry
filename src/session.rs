use crate::{
    app_server::{RescueAction, TutorResponse},
    scenario::Scenario,
};
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConversationState {
    Conversation,
    Rescue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionEvent {
    LearnerReply,
    RescueRequest,
    ShowText,
    UnderstandingSignal,
}

pub(crate) struct Session {
    scenario: Scenario,
    state: ConversationState,
    rescue_stage: u8,
    rescues: u32,
    show_text: bool,
    milestone_index: usize,
    complete: bool,
    latest_tutor_text: Option<String>,
}

impl Session {
    pub(crate) fn new(scenario: Scenario, show_text: bool) -> Self {
        Self {
            scenario,
            state: ConversationState::Conversation,
            rescue_stage: 0,
            rescues: 0,
            show_text,
            milestone_index: 0,
            complete: false,
            latest_tutor_text: None,
        }
    }
    pub(crate) fn state(&self) -> ConversationState {
        self.state
    }
    pub(crate) fn rescue_stage(&self) -> u8 {
        self.rescue_stage
    }
    pub(crate) fn rescues(&self) -> u32 {
        self.rescues
    }
    pub(crate) fn milestone(&self) -> &str {
        self.scenario
            .milestones()
            .get(self.milestone_index)
            .copied()
            .unwrap_or("complete")
    }
    pub(crate) fn is_complete(&self) -> bool {
        self.complete
    }

    pub(crate) fn classify(&self, text: &str) -> SessionEvent {
        let normalized = normalize(text);
        if matches!(
            normalized.as_str(),
            "show text" | "show me the text" | "show it"
        ) {
            SessionEvent::ShowText
        } else if is_rescue_request(&normalized) {
            SessionEvent::RescueRequest
        } else if matches!(
            normalized.as_str(),
            "got it" | "i understand" | "i got it" | "okay i understand"
        ) {
            SessionEvent::UnderstandingSignal
        } else {
            SessionEvent::LearnerReply
        }
    }

    pub(crate) fn apply(&mut self, response: &TutorResponse) -> Result<()> {
        self.latest_tutor_text = Some(response.spoken_text.clone());
        match response.rescue_action {
            RescueAction::Resume => {
                self.state = ConversationState::Conversation;
                self.rescue_stage = 0;
            }
            RescueAction::Repeat
            | RescueAction::Slower
            | RescueAction::Simplify
            | RescueAction::OfferText => {
                if self.state != ConversationState::Rescue {
                    self.rescues += 1;
                }
                self.state = ConversationState::Rescue;
                self.rescue_stage = response.rescue_action.stage();
            }
            RescueAction::None => {}
        }
        if response.show_text {
            self.show_text = true;
        }
        let next = self
            .scenario
            .milestones()
            .iter()
            .position(|item| *item == response.milestone)
            .unwrap_or(self.milestone_index);
        self.milestone_index = next;
        self.complete = response.complete;
        Ok(())
    }

    pub(crate) fn visible_text<'a>(&self, response: &'a TutorResponse) -> Option<&'a str> {
        self.show_text
            .then_some(response.learner_visible_text.as_deref())
            .flatten()
    }
    pub(crate) fn reflection(&self) -> String {
        format!(
            "You rescued the conversation {} time{}. You did great. Next time, you can also say: Sorry, could you say that again?",
            self.rescues,
            if self.rescues == 1 { "" } else { "s" }
        )
    }

    pub(crate) fn latest_tutor_text(&self) -> Option<&str> {
        self.latest_tutor_text.as_deref()
    }
}

fn normalize(text: &str) -> String {
    text.to_ascii_lowercase()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || character.is_ascii_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_rescue_request(normalized: &str) -> bool {
    matches!(
        normalized,
        "what sorry"
            | "sorry"
            | "pardon"
            | "what"
            | "could you say that again"
            | "can you say that again"
            | "say that again"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_server::TutorResponse;

    #[test]
    fn recognizes_common_rescue_phrases() {
        // GIVEN
        let session = Session::new(Scenario::Cafe, false);
        // WHEN
        let actual = session.classify("What, sorry?");
        // THEN
        assert_eq!(actual, SessionEvent::RescueRequest);
    }

    #[test]
    fn counts_a_rescue_once_until_the_conversation_resumes() -> Result<()> {
        // GIVEN
        let mut session = Session::new(Scenario::Cafe, false);
        let repeat = TutorResponse::test_response(RescueAction::Repeat, "greeting", false);
        let slower = TutorResponse::test_response(RescueAction::Slower, "greeting", false);
        // WHEN
        session.apply(&repeat)?;
        session.apply(&slower)?;
        // THEN
        assert_eq!(session.rescues(), 1);
        Ok(())
    }
}
