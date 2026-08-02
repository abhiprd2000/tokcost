//! Output rendering: a minimal hand-written JSON serializer, raw ANSI
//! escape helpers for the live ticker, and human-friendly number/USD
//! formatting. No `serde`, no `colored`/`ansi_term` — this tool only ever
//! *writes* JSON (never parses it), so a small write-only value type is
//! enough.

// ---------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    Array(Vec<Json>),
    /// Insertion-ordered key/value pairs (unlike a `HashMap`, so field
    /// order in the output matches the order fields were added).
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Int(n) => out.push_str(&n.to_string()),
            Json::Num(n) => out.push_str(&n.to_string()),
            Json::Str(s) => write_json_string(s, out),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<&str> for Json {
    fn from(s: &str) -> Self {
        Json::Str(s.to_string())
    }
}
impl From<String> for Json {
    fn from(s: String) -> Self {
        Json::Str(s)
    }
}
impl From<bool> for Json {
    fn from(b: bool) -> Self {
        Json::Bool(b)
    }
}
impl From<i64> for Json {
    fn from(n: i64) -> Self {
        Json::Int(n)
    }
}
impl From<usize> for Json {
    fn from(n: usize) -> Self {
        Json::Int(n as i64)
    }
}
impl From<f64> for Json {
    fn from(n: f64) -> Self {
        Json::Num(n)
    }
}

/// Ergonomic builder for `Json::Object` so call sites read as a flat list
/// of fields rather than a nested `Vec<(String, Json)>` literal.
#[derive(Default)]
pub struct JsonObject(Vec<(String, Json)>);

impl JsonObject {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn field(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.0.push((key.to_string(), value.into()));
        self
    }

    pub fn build(self) -> Json {
        Json::Object(self.0)
    }
}

// ---------------------------------------------------------------------
// ANSI (for the live ticker)
// ---------------------------------------------------------------------

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const CYAN: &str = "\x1b[36m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";

/// Move to the start of the current line and clear it, so the next write
/// overwrites the previous ticker frame in place.
pub const CLEAR_LINE: &str = "\r\x1b[2K";

pub fn colorize(text: &str, color: &str) -> String {
    format!("{color}{text}{RESET}")
}

// ---------------------------------------------------------------------
// Human-friendly formatting
// ---------------------------------------------------------------------

/// Format an integer with `,` thousands separators: `1234567` -> `"1,234,567"`.
pub fn format_int(n: u64) -> String {
    let digits = n.to_string();
    let bytes = digits.as_bytes();
    let first_group_len = match bytes.len() % 3 {
        0 => 3,
        rem => rem,
    };

    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    out.push_str(&digits[..first_group_len]);
    let mut i = first_group_len;
    while i < bytes.len() {
        out.push(',');
        out.push_str(&digits[i..i + 3]);
        i += 3;
    }
    out
}

/// Round a USD amount to 6 decimal places (enough precision for sub-cent
/// per-token costs) so machine-readable output like JSON doesn't leak raw
/// `f64` multiplication noise (e.g. `0.010100000000000001`).
pub fn round_money(usd: f64) -> f64 {
    (usd * 1_000_000.0).round() / 1_000_000.0
}

/// Format a USD amount, using extra decimal places for sub-cent values so
/// small per-file costs don't all round down to `$0.00`.
pub fn format_usd(usd: f64) -> String {
    if usd.abs() < 0.01 {
        format!("${usd:.4}")
    } else {
        format!("${usd:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_scalars_serialize() {
        assert_eq!(Json::Null.to_json_string(), "null");
        assert_eq!(Json::Bool(true).to_json_string(), "true");
        assert_eq!(Json::Bool(false).to_json_string(), "false");
        assert_eq!(Json::Int(-42).to_json_string(), "-42");
        assert_eq!(Json::Num(3.5).to_json_string(), "3.5");
    }

    #[test]
    fn json_strings_escape_special_characters() {
        let s = Json::Str("line\n\"quoted\"\t\\end".to_string()).to_json_string();
        assert_eq!(s, "\"line\\n\\\"quoted\\\"\\t\\\\end\"");
    }

    #[test]
    fn json_strings_escape_control_characters() {
        let s = Json::Str("\u{1}bell".to_string()).to_json_string();
        assert_eq!(s, "\"\\u0001bell\"");
    }

    #[test]
    fn json_array_and_object_serialize_in_order() {
        let arr = Json::Array(vec![Json::Int(1), Json::Int(2), Json::Int(3)]);
        assert_eq!(arr.to_json_string(), "[1,2,3]");

        let obj = JsonObject::new()
            .field("model", "gpt-4o")
            .field("tokens", 42usize)
            .field("exact", true)
            .build();
        assert_eq!(
            obj.to_json_string(),
            r#"{"model":"gpt-4o","tokens":42,"exact":true}"#
        );
    }

    #[test]
    fn json_nested_object_in_array() {
        let value = Json::Array(vec![
            JsonObject::new().field("n", 1usize).build(),
            JsonObject::new().field("n", 2usize).build(),
        ]);
        assert_eq!(value.to_json_string(), r#"[{"n":1},{"n":2}]"#);
    }

    #[test]
    fn format_int_groups_by_thousands() {
        assert_eq!(format_int(0), "0");
        assert_eq!(format_int(7), "7");
        assert_eq!(format_int(999), "999");
        assert_eq!(format_int(1000), "1,000");
        assert_eq!(format_int(1234567), "1,234,567");
        assert_eq!(format_int(100), "100");
    }

    #[test]
    fn round_money_strips_float_multiplication_noise() {
        // 4040.0 / 1_000_000.0 * 2.50 produces 0.010100000000000001 in raw
        // f64 arithmetic; rounding to 6 decimal places cleans that up for
        // JSON output without losing sub-cent precision.
        let noisy = 4040.0 / 1_000_000.0 * 2.50;
        assert_eq!(round_money(noisy), 0.0101);
        assert_eq!(round_money(1.23456789), 1.234568);
    }

    #[test]
    fn format_usd_uses_more_precision_below_a_cent() {
        assert_eq!(format_usd(0.0), "$0.0000");
        assert_eq!(format_usd(4.2), "$4.20");
        assert_eq!(format_usd(0.0031), "$0.0031");
        assert_eq!(format_usd(0.01), "$0.01");
    }

    #[test]
    fn colorize_wraps_in_ansi_and_reset() {
        assert_eq!(colorize("hi", GREEN), "\x1b[32mhi\x1b[0m");
    }
}
