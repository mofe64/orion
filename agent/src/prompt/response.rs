pub(crate) fn spoken_response(text: &str) -> Result<String, String> {
    // Native search citations belong in visual output, never speech synthesis.
    let mut plain = text.to_owned();
    while let Some(start) = plain.find('') {
        if let Some(end) = plain[start..].find('') {
            plain.replace_range(start..start + end + ''.len_utf8(), "");
        } else {
            plain.truncate(start);
            break;
        }
    }
    let mut tagged = String::new();
    let mut rest = plain.as_str();
    while let Some(start) = rest.find('[') {
        tagged.push_str(&rest[..start]);
        let Some(end) = rest[start..].find(']') else {
            break;
        };
        let content = &rest[start + 1..start + end];
        if let Some(cue) = crate::ReactionCue::parse(content) {
            tagged.push_str(&format!(" [{}] ", cue.as_str()));
        } else if !(content.chars().count() <= 24
            && content
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-')))
        {
            tagged.push_str(&rest[start..=start + end]);
        }
        rest = &rest[start + end + 1..];
    }
    if !rest.contains('[') {
        tagged.push_str(rest);
    }
    let mut words = Vec::new();
    for word in tagged.split_whitespace() {
        let tag = word
            .strip_prefix('[')
            .and_then(|w| w.strip_suffix(']'))
            .and_then(crate::ReactionCue::parse);
        if tag.is_some() && words.last().copied() == Some(word) {
            continue;
        }
        words.push(word);
    }
    let response = words.join(" ");
    if words.iter().all(|word| {
        word.strip_prefix('[')
            .and_then(|w| w.strip_suffix(']'))
            .and_then(crate::ReactionCue::parse)
            .is_some()
    }) {
        return Err("Codex returned no spoken response.".into());
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_reactions_without_eating_prose() {
        for (input, expected) in [
            ("[nod] Yes.", "[agree] Yes."),
            ("Yes [Laugh] ok.", "Yes ok."),
            ("[foo] Yes.", "Yes."),
            ("See [1, 2] here.", "See [1, 2] here."),
            (
                "[ AGREE ] [nod] Yes. [shake] No.",
                "[agree] Yes. [disagree] No.",
            ),
            ("Yes. [agr", "Yes."),
        ] {
            assert_eq!(spoken_response(input).unwrap(), expected);
        }
        assert!(spoken_response("[agree]").is_err());
        assert!(spoken_response("[agree] [disagree]").is_err());
    }
    #[test]
    fn normalizes_unicode_speech_without_truncation() {
        assert_eq!(
            spoken_response("  Hello,\n Orion. ").unwrap(),
            "Hello, Orion."
        );
        assert_eq!(
            spoken_response("Found it. citesource0").unwrap(),
            "Found it."
        );
        assert!(spoken_response(" \n").is_err());
        let reply = spoken_response(&"灯 ".repeat(900)).unwrap();
        assert_eq!(reply, "灯 ".repeat(900).trim());
    }
}
