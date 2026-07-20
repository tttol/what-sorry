use clap::ValueEnum;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    Cafe,
    Shop,
    Directions,
}

impl Scenario {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Cafe => "Cafe order",
            Self::Shop => "Shop purchase",
            Self::Directions => "Asking directions",
        }
    }
    pub(crate) fn goal(self) -> &'static str {
        match self {
            Self::Cafe => "Order a drink and complete the cafe purchase.",
            Self::Shop => "Buy one item and complete the purchase.",
            Self::Directions => "Ask for and understand directions to a nearby place.",
        }
    }
    pub(crate) fn milestones(self) -> &'static [&'static str] {
        match self {
            Self::Cafe => &[
                "greeting",
                "drink order",
                "hot or iced",
                "anything else",
                "complete order",
            ],
            Self::Shop => &[
                "greeting",
                "choose item",
                "size or color",
                "payment",
                "complete purchase",
            ],
            Self::Directions => &[
                "greeting",
                "destination",
                "route instruction",
                "understanding",
                "thanks",
            ],
        }
    }
}
