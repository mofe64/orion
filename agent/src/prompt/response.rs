pub const MAX_RESPONSE_CHARACTERS: usize = 800;

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
    let response = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if response.is_empty() {
        return Err("Codex returned no spoken response.".into());
    }
    if response.chars().count() <= MAX_RESPONSE_CHARACTERS {
        return Ok(response);
    }
    // Count Unicode characters, never slice a UTF-8 code point in half.
    let truncated: String = response.chars().take(MAX_RESPONSE_CHARACTERS).collect();
    let prefix = truncated
        .rsplit_once(' ')
        .map_or(truncated.as_str(), |(head, _)| head);
    Ok(format!("{}…", prefix.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_bounds_unicode_speech() {
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
        assert!(reply.chars().count() <= MAX_RESPONSE_CHARACTERS + 1);
        assert!(reply.ends_with('…'));
    }
}
