use super::spoken_response;

/// Release complete sentences only; never pass partial citation markup to TTS.
#[derive(Default)]
pub(crate) struct Sentences {
    raw: String,
    pub emitted: String,
}
impl Sentences {
    pub fn push(&mut self, delta: &str) -> Result<Option<String>, String> {
        if self.raw.len() + delta.len() > 64 * 1024 {
            return Err("Oversized streamed answer".into());
        }
        self.raw.push_str(delta);
        let chars: Vec<_> = self.raw.char_indices().collect();
        let mut citation = false;
        let mut boundary = None;
        for (index, &(offset, ch)) in chars.iter().enumerate() {
            if ch == '' {
                citation = true;
            }
            if ch == '' {
                citation = false;
            }
            if !citation
                && matches!(ch, '.' | '!' | '?')
                && chars
                    .get(index + 1)
                    .is_some_and(|(_, next)| next.is_whitespace())
            {
                boundary = Some(offset + ch.len_utf8());
            }
        }
        let Some(boundary) = boundary else {
            return Ok(None);
        };
        let plain = spoken_response(&self.raw[..boundary])?;
        let next = plain
            .strip_prefix(&self.emitted)
            .ok_or("Streamed answer changed an emitted sentence")?
            .trim();
        if next.is_empty() {
            return Ok(None);
        }
        let next = next.to_owned();
        self.emitted = plain;
        Ok(Some(next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn holds_split_reaction_tags_and_preserves_final_prefix() {
        let mut stream = Sentences::default();
        assert_eq!(stream.push("Yes. [agr").unwrap().as_deref(), Some("Yes."));
        assert_eq!(
            stream.push("ee] Sure. ").unwrap().as_deref(),
            Some("[agree] Sure.")
        );
        assert!(
            spoken_response("Yes. [agree] Sure. [disagree] No.")
                .unwrap()
                .strip_prefix(&stream.emitted)
                .is_some()
        );
    }
    #[test]
    fn streams_beyond_800_characters_and_keeps_the_runaway_guard() {
        let mut stream = Sentences::default();
        let prefix = format!("{}.", "a".repeat(799));
        assert_eq!(
            stream.push(&format!("{prefix} ")).unwrap(),
            Some(prefix.clone())
        );
        assert_eq!(
            stream.push("More words. ").unwrap().as_deref(),
            Some("More words.")
        );
        let final_text = format!("{prefix} More words. An ending.");
        assert!(
            spoken_response(&final_text)
                .unwrap()
                .starts_with(&stream.emitted)
        );
        assert!(stream.push(&"x".repeat(64 * 1024)).is_err());
    }

    #[test]
    fn waits_for_a_sentence_and_never_reads_partial_citations() {
        let mut stream = Sentences::default();
        assert_eq!(stream.push("A warm light").unwrap(), None);
        assert_eq!(stream.push(" is ready.").unwrap(), None);
        assert_eq!(
            stream.push(" citeid").unwrap().as_deref(),
            Some("A warm light is ready.")
        );
        assert_eq!(stream.push(". not speech. ").unwrap(), None);
        assert_eq!(
            stream.push(" Take a breath. ").unwrap().as_deref(),
            Some("Take a breath.")
        );
        assert_eq!(stream.emitted, "A warm light is ready. Take a breath.");
    }
}
