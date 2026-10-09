/// Reactions enabled in spoken replies. Transport uses canonical names only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReactionCue {
    Agree,
    Disagree,
    Happy,
    Curious,
    Thinking,
    Surprised,
    Sympathy,
    Unsure,
    Laugh,
}
impl ReactionCue {
    pub fn enabled() -> &'static [Self] {
        &[
            Self::Agree,
            Self::Disagree,
            Self::Happy,
            Self::Curious,
            Self::Thinking,
            Self::Surprised,
            Self::Sympathy,
            Self::Unsure,
            Self::Laugh,
        ]
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agree => "agree",
            Self::Disagree => "disagree",
            Self::Happy => "happy",
            Self::Curious => "curious",
            Self::Thinking => "thinking",
            Self::Surprised => "surprised",
            Self::Sympathy => "sympathy",
            Self::Unsure => "unsure",
            Self::Laugh => "laugh",
        }
    }
    pub fn when(self) -> &'static str {
        match self {
            Self::Agree => "agrees, confirms or answers yes",
            Self::Disagree => "corrects, disagrees or answers no",
            Self::Happy => "shares good news, thanks the user or shares their delight",
            Self::Curious => "asks the user a question or finds something intriguing",
            Self::Thinking => "weighs options, estimates or works something out",
            Self::Surprised => {
                "reacts to something unexpected, including a surprising fact the user shares (prefer this over agree)"
            }
            Self::Sympathy => "responds to sadness, frustration or bad news",
            Self::Unsure => "is uncertain, such as maybe, it depends or I'm not sure",
            Self::Laugh => "responds to a joke or makes a light one",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        let cue = match value.trim().to_ascii_lowercase().as_str() {
            "agree" | "nod" => Self::Agree,
            "disagree" | "shake" => Self::Disagree,
            "happy" => Self::Happy,
            "curious" => Self::Curious,
            "thinking" => Self::Thinking,
            "surprised" => Self::Surprised,
            "sympathy" => Self::Sympathy,
            "unsure" => Self::Unsure,
            "laugh" => Self::Laugh,
            _ => return None,
        };
        Self::enabled().contains(&cue).then_some(cue)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enabled_vocabulary_is_canonical() {
        assert_eq!(
            ReactionCue::enabled()
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>(),
            [
                "agree",
                "disagree",
                "happy",
                "curious",
                "thinking",
                "surprised",
                "sympathy",
                "unsure",
                "laugh"
            ]
        );
        for cue in ReactionCue::enabled() {
            assert_eq!(ReactionCue::parse(cue.as_str()), Some(*cue));
        }
        assert_eq!(ReactionCue::parse(" SHAKE "), Some(ReactionCue::Disagree));
        assert_eq!(ReactionCue::parse("nod"), Some(ReactionCue::Agree));
        assert!(ReactionCue::parse("joy").is_none());
    }
}
