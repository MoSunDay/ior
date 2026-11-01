//! The configuration value tree produced by the parser: plain data enums
//! and an insertion-ordered table, so serialization stays deterministic.

/// A parsed TOML-subset value. Tables keep insertion order; there is no
/// float/date/inline-table variant by design (see crate docs).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Basic (double-quoted) string with escapes already resolved.
    Str(String),
    /// Decimal 64-bit integer.
    Int(i64),
    /// `true` / `false`.
    Bool(bool),
    /// Single-line array (may nest one level).
    Array(Vec<Value>),
    /// A table (`[section]` / `[a.b]`).
    Table(Table),
}

/// String-keyed map that remembers insertion order (deterministic output).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    entries: Vec<(String, Value)>,
}

impl Table {
    /// An empty table.
    pub fn new() -> Self {
        Table {
            entries: Vec::new(),
        }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Looks a key up.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// Looks a key up, requiring a table value (convenience for schema
    /// extraction and tests).
    pub fn get_table(&self, key: &str) -> Option<&Table> {
        match self.get(key) {
            Some(Value::Table(t)) => Some(t),
            _ => None,
        }
    }

    /// Mutable table lookup (convenience for tests and patch building).
    pub fn get_table_mut(&mut self, key: &str) -> Option<&mut Table> {
        match self.get_mut(key) {
            Some(Value::Table(t)) => Some(t),
            _ => None,
        }
    }

    /// Builds a table from `(key, value)` pairs in order (later duplicates
    /// of a key replace earlier ones, keeping the first position).
    pub fn from_pairs(pairs: impl IntoIterator<Item = (impl Into<String>, Value)>) -> Self {
        let mut table = Table::new();
        for (key, value) in pairs {
            table.insert(key.into(), value);
        }
        table
    }

    /// Mutable lookup (the parser/merge build tables up front).
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries
            .iter_mut()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// Inserts a key, replacing an existing value in place (order kept).
    pub fn insert(&mut self, key: String, value: Value) {
        match self.entries.iter_mut().find(|(name, _)| *name == key) {
            Some(slot) => slot.1 = value,
            None => self.entries.push((key, value)),
        }
    }

    /// Iterates entries in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }
}

/// Short type name used in error messages ("string", "integer", ...).
pub fn kind(value: &Value) -> &'static str {
    match value {
        Value::Str(_) => "string",
        Value::Int(_) => "integer",
        Value::Bool(_) => "boolean",
        Value::Array(_) => "array",
        Value::Table(_) => "table",
    }
}

/// Structural equality ignoring key order: `parse(serialize(t))` moves
/// sub-tables after scalars, so roundtrip comparisons use this.
pub fn equivalent(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| equivalent(p, q))
        }
        (Value::Table(x), Value::Table(y)) => tables_equivalent(x, y),
        _ => false,
    }
}

/// Order-insensitive table equality (same keys, equivalent values).
pub fn tables_equivalent(a: &Table, b: &Table) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(key, value)| b.get(key).is_some_and(|other| equivalent(value, other)))
}

/// Escapes `text` for a basic TOML string (`\" \\ \n \t`, other control
/// characters as `\u{XX}`).
pub fn escape_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_replaces_in_place_keeping_order() {
        let mut t = Table::new();
        t.insert("a".into(), Value::Int(1));
        t.insert("b".into(), Value::Int(2));
        t.insert("a".into(), Value::Int(9));
        let keys: Vec<&str> = t.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["a", "b"]);
        assert_eq!(t.get("a"), Some(&Value::Int(9)));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn equivalent_ignores_key_order_but_not_content() {
        let mut a = Table::new();
        a.insert("x".into(), Value::Int(1));
        a.insert("y".into(), Value::Int(2));
        let mut b = Table::new();
        b.insert("y".into(), Value::Int(2));
        b.insert("x".into(), Value::Int(1));
        assert!(tables_equivalent(&a, &b));
        b.insert("x".into(), Value::Int(3));
        assert!(!tables_equivalent(&a, &b));
        let mut c = Table::new();
        c.insert("z".into(), Value::Int(1));
        assert!(!tables_equivalent(&a, &c));
        assert!(!equivalent(&Value::Int(1), &Value::Str("1".into())));
    }

    #[test]
    fn escape_string_covers_all_supported_escapes() {
        assert_eq!(escape_string("a\"b\\c\nd\te"), "a\\\"b\\\\c\\nd\\te");
        assert_eq!(escape_string("\u{1}"), "\\u{1}");
        assert_eq!(escape_string("中文，。"), "中文，。");
    }
}
