//! A minimal JSON writer.
//!
//! napkin has exactly one dependency (libc) so that it builds on whatever
//! strange machine a contributor happens to own. That rules out serde, so
//! this is the whole serializer: enough to emit a result file, no more.

use std::fmt::Write as _;

pub enum J {
    Null,
    Bool(bool),
    /// Integers are emitted verbatim; floats go through `num` below.
    Raw(String),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(&'static str, J)>),
}

impl J {
    pub fn s(v: impl Into<String>) -> J {
        J::Str(v.into())
    }

    pub fn u(v: u64) -> J {
        J::Raw(v.to_string())
    }

    /// Floats are rounded to 3 decimals. JSON has no NaN or Infinity, and a
    /// probe that produced one is a bug we would rather surface as null than
    /// smuggle into the dataset as a number.
    pub fn f(v: f64) -> J {
        if v.is_finite() {
            J::Raw(format!("{:.3}", v))
        } else {
            J::Null
        }
    }

    pub fn write(&self, out: &mut String, indent: usize) {
        let pad = "  ".repeat(indent);
        let pad_in = "  ".repeat(indent + 1);
        match self {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Raw(r) => out.push_str(r),
            J::Str(s) => escape(s, out),
            J::Arr(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                for (n, item) in items.iter().enumerate() {
                    out.push_str(&pad_in);
                    item.write(out, indent + 1);
                    if n + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                let _ = write!(out, "{}]", pad);
            }
            J::Obj(fields) => {
                if fields.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                for (n, (k, v)) in fields.iter().enumerate() {
                    out.push_str(&pad_in);
                    escape(k, out);
                    out.push_str(": ");
                    v.write(out, indent + 1);
                    if n + 1 < fields.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                let _ = write!(out, "{}}}", pad);
            }
        }
    }
}

impl std::fmt::Display for J {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        f.write_str(&out)
    }
}

fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
