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
    let phrase = WAKE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:heyorion|(?:hey|hay|he|hei|hi|a|oi)[\s\p{P}]+(?:orion|orien|o['’]ryan|oryan|o-rion|arion))\b").unwrap()
    }).find(text)?;
    // Count words before the match, rather than anchoring at the clip boundary.
    // Both words of a spaced phrase must fit within the first eight words.
    let words = text[..phrase.end()].split_whitespace().count();
    (words <= 8).then(|| {
        text[phrase.end()..]
            .trim_start_matches(|c: char| c.is_whitespace() || ",.:;!?-".contains(c))
            .trim()
            .to_owned()
    })
}

pub(crate) fn useful_followup(text: &str) -> bool {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
        >= 2
        && !after_wake(text).is_some_and(|command| command.is_empty())
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
    pub continuation: bool,
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
            continuation: false,
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
            || size > 18 * 32000
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
    fn wake_is_found_in_the_first_eight_words_with_word_boundaries() {
        assert_eq!(after_wake(" Hey, Orion! Hello "), Some("Hello".into()));
        assert_eq!(after_wake("Hey Orion"), Some(String::new()));
        for text in [
            "That is an onion",
            "Hey Orionized",
            "one two three four five six seven Hey Orion",
        ] {
            assert_eq!(after_wake(text), None);
        }
    }
    #[test]
    fn wake_accepts_asr_spellings_of_hey_but_not_other_greetings() {
        for text in [
            "He Orion",
            "Oi Orion.",
            "Hay, Orion",
            "Hei Orion",
            "Hi Orion",
        ] {
            assert_eq!(after_wake(text), Some(String::new()), "{text}");
        }
        for text in [
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
    fn wake_phrase_examples_and_followup_filter() {
        for (text, command) in [
            ("Hey Orion, what time is it?", "what time is it?"),
            ("OK so, hey Orion, lights on", "lights on"),
            ("So. Hey Orion.", ""),
            ("HeyOrion stop", "stop"),
            ("Hay Orien what's up", "what's up"),
            ("a O'Ryan, stop", "stop"),
            ("hey o-rion stop", "stop"),
        ] {
            assert_eq!(after_wake(text).as_deref(), Some(command), "{text}");
        }
        for text in ["Orion is a constellation", "Hey Ryan", "hello there"] {
            assert_eq!(after_wake(text), None);
        }
        for text in ["", "So.", "Orion.", "Hey Orion", "Hey Orion.", "HeyOrion"] {
            assert!(!useful_followup(text), "{text}");
        }
        assert!(useful_followup("lights on"));
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
