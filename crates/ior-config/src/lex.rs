//! Character-level scanning for the TOML subset: values, strings with
//! escapes, integers, booleans and single-line arrays. Shared by the
//! statement parser in [`crate::parser`]; all errors carry 1-based
//! line/column positions.

use crate::error::ParseError;
use crate::value::Value;

/// One source line, split into characters, with its 1-based number.
pub(crate) struct Line<'a> {
    /// 1-based line number.
    pub(crate) no: usize,
    /// The line's characters (columns are 1-based indexes into this).
    pub(crate) chars: &'a [char],
}

/// Skips spaces and tabs.
pub(crate) fn skip_ws(line: &Line, from: usize) -> usize {
    let mut i = from;
    while i < line.chars.len() && (line.chars[i] == ' ' || line.chars[i] == '\t') {
        i += 1;
    }
    i
}

/// True when `i` points past the end of the line.
pub(crate) fn at_end(line: &Line, i: usize) -> bool {
    i >= line.chars.len()
}

/// Parses a value; `depth` is the array-nesting level this value lives at
/// (1 = statement level). Arrays may nest one level, so a third opening
/// bracket errors.
pub(crate) fn parse_value(
    line: &Line,
    start: usize,
    depth: usize,
) -> Result<(Value, usize), ParseError> {
    let i = skip_ws(line, start);
    if at_end(line, i) {
        return Err(ParseError::new(line.no, i + 1, "expected a value"));
    }
    let c = line.chars[i];
    if c == '"' {
        let (text, next) = parse_basic_string(line, i)?;
        Ok((Value::Str(text), next))
    } else if c == '[' {
        if depth >= 3 {
            return Err(ParseError::new(
                line.no,
                i + 1,
                "arrays may nest at most one level",
            ));
        }
        parse_array(line, i, depth)
    } else if c == 't' || c == 'f' {
        let (word, next) = parse_word(line, i);
        match word.as_str() {
            "true" => Ok((Value::Bool(true), next)),
            "false" => Ok((Value::Bool(false), next)),
            _ => Err(unsupported_value(line, i, &word)),
        }
    } else if c.is_ascii_digit() || c == '+' || c == '-' {
        parse_integer(line, i)
    } else {
        Err(ParseError::new(
            line.no,
            i + 1,
            format!("unexpected character '{c}' (expected string, integer, boolean or array)"),
        ))
    }
}

/// A word token: classifies `true`/`false`/integers and gives precise
/// messages for floats, dates and other unsupported values.
fn parse_word(line: &Line, start: usize) -> (String, usize) {
    let mut i = start;
    while i < line.chars.len() {
        let c = line.chars[i];
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '.' | '-' | ':') {
            i += 1;
        } else {
            break;
        }
    }
    (line.chars[start..i].iter().collect(), i)
}

fn parse_integer(line: &Line, start: usize) -> Result<(Value, usize), ParseError> {
    let (word, next) = parse_word(line, start);
    let digits = word.strip_prefix(['+', '-']).unwrap_or(&word);
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        return match word.parse::<i64>() {
            Ok(n) => Ok((Value::Int(n), next)),
            Err(_) => Err(ParseError::new(
                line.no,
                start + 1,
                format!("integer '{word}' is out of 64-bit range"),
            )),
        };
    }
    if word.contains('.') {
        return Err(ParseError::new(
            line.no,
            start + 1,
            format!("float '{word}' is not supported (only decimal integers)"),
        ));
    }
    if word.contains(':') {
        return Err(ParseError::new(
            line.no,
            start + 1,
            format!("date/time value '{word}' is not supported"),
        ));
    }
    // A '-' after the first character is date-like (`2024-01-01`).
    if word.get(1..).is_some_and(|rest| rest.contains('-')) {
        return Err(ParseError::new(
            line.no,
            start + 1,
            format!("date/time value '{word}' is not supported"),
        ));
    }
    Err(unsupported_value(line, start, &word))
}

fn unsupported_value(line: &Line, start: usize, word: &str) -> ParseError {
    ParseError::new(
        line.no,
        start + 1,
        format!("unsupported value '{word}' (expected string, integer, boolean or array)"),
    )
}

/// Parses a single-line array; `depth` is this array's own nesting level.
fn parse_array(line: &Line, start: usize, depth: usize) -> Result<(Value, usize), ParseError> {
    let mut items = Vec::new();
    let mut i = skip_ws(line, start + 1);
    loop {
        if at_end(line, i) || line.chars[i] == '#' {
            return Err(ParseError::new(
                line.no,
                i + 1,
                "unterminated array (must open and close on the same line)",
            ));
        }
        if line.chars[i] == ']' {
            return Ok((Value::Array(items), i + 1));
        }
        let (item, next) = parse_value(line, i, depth + 1)?;
        items.push(item);
        i = skip_ws(line, next);
        if at_end(line, i) {
            return Err(ParseError::new(
                line.no,
                i + 1,
                "unterminated array (must open and close on the same line)",
            ));
        }
        match line.chars[i] {
            ',' => i = skip_ws(line, i + 1),
            ']' => return Ok((Value::Array(items), i + 1)),
            c => {
                return Err(ParseError::new(
                    line.no,
                    i + 1,
                    format!("unexpected character '{c}' in array (expected ',' or ']')"),
                ))
            }
        }
    }
}

/// Parses a basic string with `\" \\ \n \t \u{XXXX}` escapes; `start`
/// points at the opening quote. Multi-line strings are not supported.
pub(crate) fn parse_basic_string(line: &Line, start: usize) -> Result<(String, usize), ParseError> {
    let mut out = String::new();
    let mut i = start + 1;
    loop {
        if at_end(line, i) {
            return Err(ParseError::new(
                line.no,
                i + 1,
                "unterminated string (multi-line strings are not supported)",
            ));
        }
        match line.chars[i] {
            '"' => return Ok((out, i + 1)),
            '\\' => {
                let (c, next) = parse_escape(line, i)?;
                out.push(c);
                i = next;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
}

/// Parses one escape sequence starting at the backslash.
fn parse_escape(line: &Line, backslash: usize) -> Result<(char, usize), ParseError> {
    let esc = backslash + 1;
    if at_end(line, esc) {
        return Err(ParseError::new(
            line.no,
            esc + 1,
            "unterminated escape sequence",
        ));
    }
    let c = line.chars[esc];
    let simple = match c {
        '"' => Some('"'),
        '\\' => Some('\\'),
        'n' => Some('\n'),
        't' => Some('\t'),
        _ => None,
    };
    if let Some(ch) = simple {
        return Ok((ch, esc + 1));
    }
    if c != 'u' {
        return Err(ParseError::new(
            line.no,
            esc + 1,
            format!("unsupported escape '\\{c}' (supported: \\\" \\\\ \\n \\t \\u{{XXXX}})"),
        ));
    }
    let mut i = esc + 1;
    if at_end(line, i) || line.chars[i] != '{' {
        return Err(ParseError::new(
            line.no,
            i + 1,
            "expected '{' after \\u escape",
        ));
    }
    i += 1;
    let (mut code, mut digits) = (0u32, 0usize);
    while !at_end(line, i) && line.chars[i] != '}' && digits < 6 {
        let h = line.chars[i];
        match h.to_digit(16) {
            Some(d) => {
                code = code * 16 + d;
                digits += 1;
                i += 1;
            }
            None => {
                return Err(ParseError::new(
                    line.no,
                    i + 1,
                    format!("invalid hex digit '{h}' in \\u escape"),
                ))
            }
        }
    }
    if digits == 0 {
        return Err(ParseError::new(
            line.no,
            i + 1,
            "\\u escape needs 1-6 hex digits",
        ));
    }
    if at_end(line, i) || line.chars[i] != '}' {
        return Err(ParseError::new(
            line.no,
            i + 1,
            "unterminated \\u escape (expected '}')",
        ));
    }
    match char::from_u32(code) {
        Some(ch) => Ok((ch, i + 1)),
        None => Err(ParseError::new(
            line.no,
            esc + 1,
            format!("\\u value {code:#x} is not a valid unicode scalar"),
        )),
    }
}
