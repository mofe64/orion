fn is_url_start(text: &str) -> bool {
    let lower = text.get(..8).unwrap_or(text).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.")
}

/// Web addresses are never worth hearing. Markdown links keep their words;
/// bare URLs disappear along with any parentheses left empty around them.
fn without_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        // [words](https://...) -> words
        if let Some(after) = rest.strip_prefix('[')
            && let Some(close) = after.find("](")
            && !after[..close].contains(['[', ']'])
            && is_url_start(&after[close + 2..])
            && let Some(end) = after[close + 2..].find(')')
        {
            out.push_str(&after[..close]);
            rest = &after[close + 2 + end + 1..];
            continue;
        }
        let at_word_start = out
            .chars()
            .last()
            .is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '<' | '"' | '\''));
        if at_word_start && is_url_start(rest) {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            // Keep sentence punctuation that followed the address.
            let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '>', '"']);
            rest = &rest[url.len()..];
            if (out.ends_with('(') || out.ends_with('<'))
                && let Some(close) = rest.strip_prefix(')').or_else(|| rest.strip_prefix('>'))
            {
                out.pop();
                rest = close;
            }
            // A label that only introduced the address goes with it.
            let trimmed = out.trim_end();
            if let Some(label) = ["Sources:", "Source:", "Links:", "Link:"]
                .iter()
                .find(|label| {
                    trimmed
                        .to_ascii_lowercase()
                        .ends_with(&label.to_ascii_lowercase())
                })
            {
                out.truncate(trimmed.len() - label.len());
                if out.trim_end().is_empty() || out.trim_end().ends_with(['.', '!', '?']) {
                    rest = rest.trim_start_matches(['.', ',', ';', ':']);
                }
            }
            while out.ends_with(' ') && rest.starts_with(['.', ',', ';', ':', '!', '?']) {
                out.pop();
            }
            continue;
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

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
    let plain = without_links(&plain);
    let mut tagged = String::new();
    let mut rest = plain.as_str();
    while let Some(start) = rest.find('[') {
        tagged.push_str(&rest[..start]);
        let Some(end) = rest[start..].find(']') else {
            tagged.push_str(&rest[start + 1..]);
            rest = "";
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
            ("Yes [Laugh] ok.", "Yes [laugh] ok."),
            ("[Happy] Great.", "[happy] Great."),
            ("[oops] I misheard.", "[sorry] I misheard."),
            ("[apologise] I cannot do that.", "[sorry] I cannot do that."),
            ("[bashful] Thank you.", "[shy] Thank you."),
            ("[flattered] That is kind.", "[shy] That is kind."),
            ("[foo] Yes.", "Yes."),
            ("See [1, 2] here.", "See [1, 2] here."),
            (
                "[ AGREE ] [nod] Yes. [shake] No.",
                "[agree] Yes. [disagree] No.",
            ),
            ("Yes. [agr", "Yes. agr"),
            ("Press the [ key to continue", "Press the key to continue"),
        ] {
            assert_eq!(spoken_response(input).unwrap(), expected);
        }
        assert!(spoken_response("[agree]").is_err());
        assert!(spoken_response("[laugh]").is_err());
        assert_eq!(
            spoken_response("[laugh] Nice joke.").unwrap(),
            "[laugh] Nice joke."
        );
        assert!(spoken_response("[agree] [disagree]").is_err());
    }
    #[test]
    fn never_speaks_web_addresses() {
        for (input, expected) in [
            (
                "Read [BBC News](https://www.bbc.co.uk/news/x) today.",
                "Read BBC News today.",
            ),
            ("It rose 3% (https://example.com/a?b=1).", "It rose 3%."),
            (
                "Done. Source: https://example.com/page. Next.",
                "Done. Next.",
            ),
            ("See www.example.com for more.", "See for more."),
            ("Visit HTTPS://Example.com, then rest.", "Visit, then rest."),
            ("A [note] and (a remark) stay.", "A and (a remark) stay."),
            ("The ratio is 3.5 today.", "The ratio is 3.5 today."),
        ] {
            assert_eq!(spoken_response(input).unwrap(), expected, "{input}");
        }
        assert!(spoken_response("Sources: https://a.com https://b.com").is_err());
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
