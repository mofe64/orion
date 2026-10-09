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
    // Qwen sometimes spells a spoken "Hey" as "Hay", "He", "Hei" or "Oi"; accept
    // those spellings of the wake phrase, never other greetings.
    WAKE.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:hey|hay|he|hei|oi)[\s,.:;!?-]+orion\b[\s,.:;!?-]*(.*)$").unwrap()
    })
    .captures(text)
    .map(|capture| capture[1].trim().to_owned())
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Phase {
    Wake,
    VerifyingWake,
    Command,
    Transcribing,
    Responding,
}
#[derive(Debug)]
pub(crate) struct Session {
    pub id: String,
    pub phase: Phase,
    pub wake_verified: bool,
    prefix_attempted: bool,
}
impl Session {
    pub fn new(id: &str, phase: Phase) -> Result<Self, String> {
        if !valid_session(id) {
            return Err("Invalid Pi voice session ID".into());
        }
        Ok(Self {
            id: id.into(),
            phase,
            wake_verified: false,
            prefix_attempted: false,
        })
    }
    /// Record the listener's acoustic verdict. It replaces the ASR prefix pass,
    /// so no prefix can follow. Rejection retires the session in the pipeline;
    /// Qwen checks complete utterances only for accepted candidates.
    pub fn acoustic_verdict(&mut self, id: &str, accepted: bool) -> Result<(), String> {
        if self.id != id || self.phase != Phase::Wake || self.prefix_attempted {
            return Err("Unexpected acoustic wake verdict".into());
        }
        self.prefix_attempted = true;
        self.wake_verified = accepted;
        Ok(())
    }
    pub fn accept(&mut self, id: &str, purpose: &str, size: u64) -> Result<(), String> {
        let expected = match purpose {
            "wake_prefix" if !self.prefix_attempted && size <= 4 * 32000 => Phase::Wake,
            "wake_and_command" => Phase::Wake,
            "command" => Phase::Command,
            _ => return Err("Invalid utterance purpose".into()),
        };
        if self.id != id
            || self.phase != expected
            || size == 0
            || size > 33 * 32000
            || !size.is_multiple_of(2)
        {
            return Err("Stale, overlapping, or invalid Pi utterance".into());
        }
        if purpose == "wake_prefix" {
            self.prefix_attempted = true;
            self.phase = Phase::VerifyingWake;
        } else {
            self.phase = Phase::Transcribing;
        }
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
    fn wake_accepts_asr_spellings_of_hey_but_not_other_greetings() {
        for text in ["He Orion", "Oi Orion.", "Hay, Orion", "Hei Orion"] {
            assert_eq!(after_wake(text), Some(String::new()), "{text}");
        }
        for text in [
            "Hi Orion",
            "Hello Orion",
            "Okay Orion",
            "Here, Orion",
            "Hey Ryan",
            "The Orion",
        ] {
            assert_eq!(after_wake(text), None, "{text}");
        }
    }
    #[test]
    fn acoustic_verdict_replaces_prefix_once() {
        let id = "a".repeat(32);
        let mut session = Session::new(&id, Phase::Wake).unwrap();
        session.acoustic_verdict(&id, true).unwrap();
        assert!(session.wake_verified);
        assert!(session.acoustic_verdict(&id, false).is_err());
        assert!(session.accept(&id, "wake_prefix", 64000).is_err());
        session.accept(&id, "wake_and_command", 64000).unwrap();
        let mut late = Session::new(&id, Phase::Wake).unwrap();
        late.accept(&id, "wake_prefix", 64000).unwrap();
        assert!(late.acoustic_verdict(&id, true).is_err());
    }
    #[test]
    fn prefix_cannot_overlap_full_audio_or_be_repeated() {
        let id = "a".repeat(32);
        let mut session = Session::new(&id, Phase::Wake).unwrap();
        assert!(session.accept(&id, "wake_prefix", 4 * 32000 + 2).is_err());
        session.accept(&id, "wake_prefix", 64000).unwrap();
        assert_eq!(session.phase, Phase::VerifyingWake);
        assert!(session.accept(&id, "wake_and_command", 64000).is_err());
        session.phase = Phase::Wake;
        assert!(session.accept(&id, "wake_prefix", 64000).is_err());
        session.accept(&id, "wake_and_command", 64000).unwrap();
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
                .accept(&session.id.clone(), "wake_and_command", 33 * 32000 + 2)
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
