//! Statement-level parsing: comments, `[section]`/`[a.b]` headers,
//! `key = value` lines and duplicate-key/redefinition detection. Value
//! scanning lives in [`crate::lex`]. Errors always carry 1-based
//! line/column; nothing panics.

use crate::error::ParseError;
use crate::lex::{at_end, parse_basic_string, parse_value, skip_ws, Line};
use crate::value::{kind, Table, Value};

/// Builds a [`ParseError`] positioned on this line (column 1-based).
fn at(line: &Line, col: usize, message: impl Into<String>) -> ParseError {
    ParseError::new(line.no, col, message)
}

/// Parses `input` into an insertion-ordered [`Table`].
pub fn parse(input: &str) -> Result<Table, ParseError> {
    let mut root = Table::new();
    // Header paths already opened (explicitly, or implicitly by `[a.b]`).
    let mut defined: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for (idx, raw) in input.lines().enumerate() {
        let owned: Vec<char> = raw.chars().collect();
        let line = Line {
            no: idx + 1,
            chars: &owned,
        };
        let i = skip_ws(&line, 0);
        if at_end(&line, i) || line.chars[i] == '#' {
            continue;
        }
        if line.chars[i] == '[' {
            if line.chars.get(i + 1) == Some(&'[') {
                return Err(at(
                    &line,
                    i + 1,
                    "array-of-tables ([[name]]) headers are not supported",
                ));
            }
            let (path, next) = parse_header(&line, i)?;
            expect_line_end(&line, next)?;
            open_table(&mut root, &path, &mut defined, line.no, i + 1)?;
            current = path;
        } else {
            let (keys, next) = parse_key_path(&line, i)?;
            let key_col = i + 1;
            let mut j = skip_ws(&line, next);
            if at_end(&line, j) || line.chars[j] != '=' {
                return Err(at(
                    &line,
                    j + 1,
                    format!("expected '=' after key '{}'", keys.join(".")),
                ));
            }
            j = skip_ws(&line, j + 1);
            if at_end(&line, j) {
                return Err(at(&line, j + 1, "expected a value after '='"));
            }
            let (value, after) = parse_value(&line, j, 1)?;
            expect_line_end(&line, after)?;
            insert_entry(&mut root, &current, &keys, value, line.no, key_col)?;
        }
    }
    Ok(root)
}

/// True when `key` can be written unquoted: ASCII alphanumerics, `_` or
/// `-`, and not digits-only (numeric keys are rejected).
pub fn is_bare_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(is_key_char) && !key.chars().all(|c| c.is_ascii_digit())
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Accepts only end-of-line or a trailing `# comment` after a value.
fn expect_line_end(line: &Line, from: usize) -> Result<(), ParseError> {
    let i = skip_ws(line, from);
    if at_end(line, i) || line.chars[i] == '#' {
        Ok(())
    } else {
        let tail: String = line.chars[i..].iter().collect();
        Err(at(
            line,
            i + 1,
            format!("unexpected trailing content: '{tail}'"),
        ))
    }
}

/// Parses `[a.b]` into its key path; `start` points at `[`.
fn parse_header(line: &Line, start: usize) -> Result<(Vec<String>, usize), ParseError> {
    let mut path = Vec::new();
    let mut i = skip_ws(line, start + 1);
    loop {
        let (segment, next) = parse_key_segment(line, i)?;
        path.push(segment);
        i = skip_ws(line, next);
        if at_end(line, i) {
            return Err(at(line, i + 1, "unterminated table header (missing ']')"));
        }
        match line.chars[i] {
            '.' => i = skip_ws(line, i + 1),
            ']' => return Ok((path, i + 1)),
            c => {
                return Err(at(
                    line,
                    i + 1,
                    format!("unexpected character '{c}' in table header"),
                ))
            }
        }
    }
}

/// Parses a (possibly dotted) key before `=`.
fn parse_key_path(line: &Line, start: usize) -> Result<(Vec<String>, usize), ParseError> {
    let mut keys = Vec::new();
    let mut i = start;
    loop {
        let (segment, next) = parse_key_segment(line, i)?;
        keys.push(segment);
        i = skip_ws(line, next);
        if !at_end(line, i) && line.chars[i] == '.' {
            i = skip_ws(line, i + 1);
            if at_end(line, i) {
                return Err(ParseError::new(line.no, i + 1, "expected a key after '.'"));
            }
        } else {
            return Ok((keys, i));
        }
    }
}

/// One key segment: a bare key or a quoted string. Numeric keys error.
fn parse_key_segment(line: &Line, start: usize) -> Result<(String, usize), ParseError> {
    if at_end(line, start) {
        return Err(ParseError::new(line.no, start + 1, "expected a key"));
    }
    let (segment, next) = if line.chars[start] == '"' {
        parse_basic_string(line, start)?
    } else {
        let mut end = start;
        while end < line.chars.len() && is_key_char(line.chars[end]) {
            end += 1;
        }
        if end == start {
            let c = line.chars[start];
            return Err(at(
                line,
                start + 1,
                format!("unexpected character '{c}' in key (bare keys are A-Z a-z 0-9 _ -; quote other keys)"),
            ));
        }
        (line.chars[start..end].iter().collect(), end)
    };
    if segment.chars().all(|c| c.is_ascii_digit()) {
        return Err(at(
            line,
            start + 1,
            format!("numeric key '{segment}' is not supported; use a named key"),
        ));
    }
    Ok((segment, next))
}

/// Opens (creating intermediates) the table at `path`, rejecting
/// redefinitions and attempts to extend a non-table value.
fn open_table(
    root: &mut Table,
    path: &[String],
    defined: &mut Vec<Vec<String>>,
    line_no: usize,
    col: usize,
) -> Result<(), ParseError> {
    if defined.iter().any(|p| p.as_slice() == path) {
        return Err(ParseError::new(
            line_no,
            col,
            format!(
                "table header [{}] is defined more than once",
                path.join(".")
            ),
        ));
    }
    let mut node: &mut Table = root;
    for segment in path {
        node = child_table_mut(node, segment, line_no, col, path.join("."))?;
    }
    for len in 1..=path.len() {
        let prefix = path[..len].to_vec();
        if !defined.contains(&prefix) {
            defined.push(prefix);
        }
    }
    Ok(())
}

/// Inserts `keys` (relative to the current `section`) into `root`.
fn insert_entry(
    root: &mut Table,
    section: &[String],
    keys: &[String],
    value: Value,
    line_no: usize,
    key_col: usize,
) -> Result<(), ParseError> {
    let mut full: Vec<String> = section.to_vec();
    full.extend(keys.iter().cloned());
    let dotted = full.join(".");
    let last = full.pop().unwrap_or_default();
    let mut node: &mut Table = root;
    for segment in &full {
        node = child_table_mut(node, segment, line_no, key_col, dotted.clone())?;
    }
    if node.get(&last).is_some() {
        return Err(ParseError::new(
            line_no,
            key_col,
            format!("duplicate key '{dotted}'"),
        ));
    }
    node.insert(last, value);
    Ok(())
}

/// Returns the (created if missing) child table; errors when the existing
/// child holds a non-table value.
fn child_table_mut<'t>(
    parent: &'t mut Table,
    key: &str,
    line_no: usize,
    col: usize,
    context: String,
) -> Result<&'t mut Table, ParseError> {
    if parent.get(key).is_none() {
        parent.insert(key.to_string(), Value::Table(Table::new()));
    }
    match parent.get_mut(key) {
        Some(Value::Table(t)) => Ok(t),
        Some(v) => Err(ParseError::new(
            line_no,
            col,
            format!(
                "cannot extend '{key}' under [{context}]: it already holds a {} value",
                kind(v)
            ),
        )),
        None => Err(ParseError::new(
            line_no,
            col,
            format!("cannot open '{key}' under [{context}]"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_of(input: &str) -> ParseError {
        parse(input).expect_err("input must not parse")
    }

    #[test]
    fn parses_sections_scalars_and_comments() {
        let source = "# top comment\n[engine]\nchinese_punct = true   # trailing comment\npage_size = 5\nshift_action = \"commit\"\n";
        let table = parse(source).expect("valid");
        let engine = table.get_table("engine").expect("engine section");
        assert_eq!(engine.get("chinese_punct"), Some(&Value::Bool(true)));
        assert_eq!(engine.get("page_size"), Some(&Value::Int(5)));
        assert_eq!(
            engine.get("shift_action"),
            Some(&Value::Str("commit".into()))
        );
        assert_eq!(engine.len(), 3);
    }

    #[test]
    fn parses_dotted_headers_keys_and_negative_ints() {
        let table = parse("[engine.fuzzy]\nz_zh = true\n\n[switches]\nascii_mode = false\n")
            .expect("valid");
        let fuzzy = table
            .get_table("engine")
            .and_then(|e| e.get_table("fuzzy"))
            .expect("nested table");
        assert_eq!(fuzzy.get("z_zh"), Some(&Value::Bool(true)));

        let table = parse("[a]\nb.c = -7\n").expect("dotted key");
        let b = table
            .get_table("a")
            .and_then(|t| t.get_table("b"))
            .expect("implicit table from dotted key");
        assert_eq!(b.get("c"), Some(&Value::Int(-7)));
    }

    #[test]
    fn parses_arrays_with_one_nesting_level() {
        let table = parse("[t]\nxs = [\"a\", \"b\", ]\nnested = [[1, 2], [3]]\nempty = []\n")
            .expect("valid");
        let t = table.get_table("t").unwrap();
        assert_eq!(
            t.get("xs"),
            Some(&Value::Array(vec![
                Value::Str("a".into()),
                Value::Str("b".into())
            ]))
        );
        assert_eq!(
            t.get("nested"),
            Some(&Value::Array(vec![
                Value::Array(vec![Value::Int(1), Value::Int(2)]),
                Value::Array(vec![Value::Int(3)])
            ]))
        );
        assert_eq!(t.get("empty"), Some(&Value::Array(vec![])));
    }

    #[test]
    fn parses_string_escapes_and_quoted_keys() {
        let table = parse("[t]\ns = \"a\\\"b\\\\c\\nd\\te\\u{4e2d}\\u{1F600}\"\n").expect("valid");
        assert_eq!(
            table.get_table("t").unwrap().get("s"),
            Some(&Value::Str("a\"b\\c\nd\te中\u{1F600}".into()))
        );

        let quoted = parse("[punct]\n\",\" = \"，\"\n").expect("quoted key");
        assert_eq!(
            quoted.get_table("punct").unwrap().get(","),
            Some(&Value::Str("，".into()))
        );
    }

    #[test]
    fn error_line_numbers_are_one_based() {
        let e = err_of("[ok]\nflag = tru\n");
        assert_eq!((e.line, e.col), (2, 8));
        let e = err_of("[ok]\n\n\n[ok]\n");
        assert_eq!(e.line, 4);
        let e = err_of("k = \"abc\n");
        assert_eq!((e.line, e.col), (1, 9));
    }

    #[test]
    fn error_duplicate_and_numeric_keys() {
        let e = err_of("[engine]\npage_size = 5\npage_size = 6\n");
        assert_eq!(e.line, 3);
        assert!(e.message.contains("duplicate key 'engine.page_size'"));
        let e = err_of("[t]\n1 = \"one\"\n");
        assert!(e.message.contains("numeric key '1'"));
        let e = err_of("[t]\n\"123\" = \"one\"\n");
        assert!(e.message.contains("numeric key '123'"));
    }

    #[test]
    fn error_unsupported_values_and_syntax() {
        let e = err_of("[t]\nx = 1.5\n");
        assert!(e.message.contains("float '1.5' is not supported"));
        let e = err_of("[t]\nx = 2024-01-01\n");
        assert!(e.message.contains("date/time"));
        let e = err_of("[t]\nx = [[ [1] ]]\n");
        assert!(e.message.contains("arrays may nest at most one level"));
        let e = err_of("[t]\nx = [1, 2\n");
        assert!(e.message.contains("unterminated array"));
        let e = err_of("[t]\nx = \"a\\q\"\n");
        assert!(e.message.contains("unsupported escape '\\q'"));
        let e = err_of("[t]\nx = \"\\u{zz}\"\n");
        assert!(e.message.contains("invalid hex digit"));
        let e = err_of("[t]\nx = \"\\u{}\"\n");
        assert!(e.message.contains("1-6 hex digits"));
        let e = err_of("[t]\nx = \"\\u{110000}\"\n");
        assert!(e.message.contains("not a valid unicode scalar"));
        let e = err_of("[[array]]\n");
        assert!(e.message.contains("array-of-tables"));
        let e = err_of("[t]\nx 5\n");
        assert!(e.message.contains("expected '=' after key 'x'"));
        let e = err_of("[t]\nx = 1 junk\n");
        assert!(e.message.contains("unexpected trailing content"));
        let e = err_of("[t\n");
        assert!(e.message.contains("unterminated table header"));
    }

    #[test]
    fn error_table_redefinition_and_extension() {
        let e = err_of("[a]\nx = 1\n[a]\ny = 2\n");
        assert_eq!(e.line, 3);
        assert!(e.message.contains("[a] is defined more than once"));
        let e = err_of("[a.b]\n[a]\n");
        assert!(e.message.contains("[a] is defined more than once"));
        let e = err_of("[a]\nx = 1\n[a.x]\ny = 2\n");
        assert!(e.message.contains("cannot extend 'x'"));
    }

    #[test]
    fn empty_input_and_blank_lines_give_empty_table() {
        assert!(parse("").unwrap().is_empty());
        assert!(parse("\n\n# only a comment\n").unwrap().is_empty());
    }
}
