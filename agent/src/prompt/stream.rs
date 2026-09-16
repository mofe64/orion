use super::{MAX_RESPONSE_CHARACTERS, spoken_response};

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
        if plain.chars().count() > MAX_RESPONSE_CHARACTERS || plain.ends_with('…') {
            return Ok(None);
        }
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
