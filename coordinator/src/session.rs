use regex::Regex;
use std::sync::OnceLock;

pub(crate) fn valid_session(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn after_wake(text: &str) -> Option<String> {
    static WAKE: OnceLock<Regex> = OnceLock::new();
    WAKE.get_or_init(|| Regex::new(r"(?i)^\s*hey[\s,.:;!?-]+orion\b[\s,.:;!?-]*(.*)$").unwrap())
        .captures(text)
        .map(|capture| capture[1].trim().to_owned())
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Phase {
    Wake,
    Command,
    Transcribing,
    Responding,
}
#[derive(Debug)]
pub(crate) struct Session {
    pub id: String,
    pub phase: Phase,
}
impl Session {
    pub fn new(id: &str, phase: Phase) -> Result<Self, String> {
        if !valid_session(id) {
            return Err("Invalid Pi voice session ID".into());
        }
        Ok(Self {
            id: id.into(),
            phase,
        })
    }
    pub fn accept(&mut self, id: &str, purpose: &str, size: u64) -> Result<(), String> {
        let expected = match purpose {
            "wake_and_command" => Phase::Wake,
            "command" => Phase::Command,
            _ => return Err("Invalid utterance purpose".into()),
        };
        if self.id != id
            || self.phase != expected
            || size == 0
            || size > 18 * 32000
            || !size.is_multiple_of(2)
        {
            return Err("Stale, overlapping, or invalid Pi utterance".into());
        }
        self.phase = Phase::Transcribing;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wake_must_be_at_start_with_word_boundary() {
        assert_eq!(after_wake(" Hey, Orion! Hello "), Some("Hello".into()));
        assert_eq!(after_wake("Hey Orion"), Some(String::new()));
        for text in ["That is an onion", "I said Hey Orion", "Hey Orionized"] {
            assert_eq!(after_wake(text), None);
        }
    }
    #[test]
    fn rejects_stale_overlapping_and_oversized_audio() {
        let mut session = Session::new(&"a".repeat(32), Phase::Wake).unwrap();
        assert!(
            session
                .accept(&"b".repeat(32), "wake_and_command", 2)
                .is_err()
        );
        assert!(session.accept(&session.id.clone(), "command", 2).is_err());
        assert!(
            session
                .accept(&session.id.clone(), "wake_and_command", 18 * 32000 + 2)
                .is_err()
        );
        session
            .accept(&session.id.clone(), "wake_and_command", 2)
            .unwrap();
        assert!(
            session
                .accept(&session.id.clone(), "wake_and_command", 2)
                .is_err()
        );
    }
}
