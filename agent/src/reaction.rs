/// Reactions enabled in spoken replies. Transport uses canonical names only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReactionCue {
    Agree,
    Disagree,
}
impl ReactionCue {
    pub fn enabled() -> &'static [Self] {
        &[Self::Agree, Self::Disagree]
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agree => "agree",
            Self::Disagree => "disagree",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        let cue = match value.trim().to_ascii_lowercase().as_str() {
            "agree" | "nod" => Self::Agree,
            "disagree" | "shake" => Self::Disagree,
            _ => return None,
        };
        Self::enabled().contains(&cue).then_some(cue)
    }
    pub fn sound(self) -> Option<&'static str> {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_one_vocabulary() {
        assert_eq!(
            ReactionCue::enabled()
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>(),
            ["agree", "disagree"]
        );
        assert_eq!(ReactionCue::parse(" SHAKE "), Some(ReactionCue::Disagree));
        assert!(ReactionCue::parse("laugh").is_none());
        assert!(ReactionCue::enabled().iter().all(|c| c.sound().is_none()));
    }
}
