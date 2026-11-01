//! Minimal JSON string codec for the optional transcript-polish call:
//! escape a Rust string into a JSON literal, and pull one string field
//! back out of a chat-completions response without a full parser. The
//! scanner tracks string boundaries, so a key name appearing inside some
//! other string (or as a substring like `reasoning_content`) never
//! matches.

/// Renders `text` as the contents of a JSON string literal (no quotes).
pub fn escape_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Extracts the string value of the first top-level-ish `"key"` whose
/// name matches `key` exactly. `None` when absent or not a string.
pub fn extract_string_field(text: &str, key: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let needle = format!("\"{key}\"");
    let needle = needle.as_bytes();
    let mut idx = 0;
    let mut in_string = false;
    while idx < bytes.len() {
        let byte = bytes[idx];
        if in_string {
            if byte == b'\\' {
                idx += 2; // skip escaped char
                continue;
            }
            if byte == b'"' {
                in_string = false;
            }
            idx += 1;
            continue;
        }
        if byte == b'"' {
            // A quote that opens a value string: first test whether it is
            // actually the needle (a key), then fall through to consuming
            // the string so its contents can never trigger matches.
            if bytes[idx..].starts_with(needle) && opens_object_entry(bytes, idx) {
                return parse_string_at(bytes, skip_colon(bytes, idx + needle.len()));
            }
            in_string = true;
        }
        idx += 1;
    }
    None
}

/// The needle must open an object entry, i.e. the previous non-whitespace
/// byte is `{`, `,` or `[` — rejects accidental substrings of longer
/// names (`reasoning_content` for a `content` lookup).
fn opens_object_entry(bytes: &[u8], needle_start: usize) -> bool {
    let mut idx = needle_start;
    while idx > 0 {
        idx -= 1;
        match bytes[idx] {
            b' ' | b'\t' | b'\n' | b'\r' => continue,
            b'{' | b',' | b'[' => return true,
            _ => return false,
        }
    }
    false
}

/// After the key literal: skip whitespace, expect `:`, skip whitespace.
fn skip_colon(bytes: &[u8], mut idx: usize) -> usize {
    while idx < bytes.len() && matches!(bytes[idx], b' ' | b'\t' | b'\n' | b'\r') {
        idx += 1;
    }
    if idx < bytes.len() && bytes[idx] == b':' {
        idx += 1;
    }
    while idx < bytes.len() && matches!(bytes[idx], b' ' | b'\t' | b'\n' | b'\r') {
        idx += 1;
    }
    idx
}

/// Parses a JSON string literal starting at `idx` (must be a `"`).
fn parse_string_at(bytes: &[u8], idx: usize) -> Option<String> {
    if idx >= bytes.len() || bytes[idx] != b'"' {
        return None;
    }
    let mut out = String::new();
    let mut i = idx + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some(out),
            b'\\' => {
                i += 1;
                match *bytes.get(i)? {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hex = bytes.get(i + 1..i + 5)?;
                        let code = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                        i += 4;
                        if (0xD800..0xDC00).contains(&code) {
                            // surrogate pair: \uXXXX\uXXXX
                            if bytes.get(i + 1)? != &b'\\' || bytes.get(i + 2)? != &b'u' {
                                return None;
                            }
                            let hex2 = bytes.get(i + 3..i + 7)?;
                            let low =
                                u32::from_str_radix(std::str::from_utf8(hex2).ok()?, 16).ok()?;
                            if !(0xDC00..0xE000).contains(&low) {
                                return None;
                            }
                            let combined = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                            out.push(char::from_u32(combined)?);
                            i += 6;
                        } else {
                            out.push(char::from_u32(code)?);
                        }
                    }
                    _ => return None,
                }
                i += 1;
            }
            _ => {
                // Copy raw UTF-8; from_utf8_lossy handles any width.
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i] & 0xC0) == 0x80 {
                    i += 1;
                }
                out.push_str(&String::from_utf8_lossy(&bytes[start..i]));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quotes_backslashes_and_control_chars() {
        assert_eq!(escape_string("plain"), "plain");
        assert_eq!(escape_string("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(escape_string("line\nbreak\ttab"), "line\\nbreak\\ttab");
        assert_eq!(escape_string("\u{1}"), "\\u0001");
        assert_eq!(escape_string("汉字"), "汉字");
    }

    #[test]
    fn extracts_content_from_a_chat_completions_body() {
        let body = r#"{"id":"x","choices":[{"index":0,"message":{"role":"assistant","content":"你好，世界！"},"finish_reason":"stop"}],"usage":{"total_tokens":9}}"#;
        assert_eq!(
            extract_string_field(body, "content").unwrap(),
            "你好，世界！"
        );
        assert_eq!(extract_string_field(body, "role").unwrap(), "assistant");
        assert_eq!(extract_string_field(body, "finish_reason").unwrap(), "stop");
    }

    #[test]
    fn substring_keys_and_string_contents_do_not_match() {
        // "reasoning_content" must not satisfy a lookup of "content",
        // and a key mentioned inside a string value is ignored.
        let body = r#"{"choices":[{"message":{"reasoning_content":"\"content\":\"X\"","content":"real"}}]}"#;
        assert_eq!(extract_string_field(body, "content").unwrap(), "real");
    }

    #[test]
    fn decodes_escapes_and_surrogate_pairs() {
        let body = r#"{"content":"a\"b\\c\nA"}"#;
        assert_eq!(extract_string_field(body, "content").unwrap(), "a\"b\\c\nA");
        let emoji = r#"{"content":"\ud83c\udfb9"}"#; // 🎹 U+1F3B9
        assert_eq!(extract_string_field(emoji, "content").unwrap(), "🎹");
    }

    #[test]
    fn missing_or_non_string_fields_return_none() {
        assert_eq!(extract_string_field(r#"{"a":1}"#, "content"), None);
        assert_eq!(extract_string_field(r#"{"content":42}"#, "content"), None);
        assert_eq!(
            extract_string_field(r#"{"content":"unterminated"#, "content"),
            None
        );
    }
}
