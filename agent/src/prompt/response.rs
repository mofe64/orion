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
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

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
