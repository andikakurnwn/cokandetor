//! Tolerant reader for the *incomplete* JSON that streams in while a model is
//! still generating tool arguments. Used only for the live preview: the real
//! tool input is always parsed strictly once the block is complete.

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub key: String,
    pub value: String,
    pub complete: bool,
}

/// Top-level string fields of a (possibly truncated) JSON object, in order.
/// Non-string values are skipped. Stops at the first field that is cut off,
/// which is returned with `complete == false`.
pub fn string_fields(src: &str) -> Vec<Field> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = skip_ws(b, 0);
    if i >= b.len() || b[i] != b'{' {
        return out;
    }
    i += 1;
    loop {
        i = skip_ws(b, i);
        if i < b.len() && b[i] == b',' {
            i = skip_ws(b, i + 1);
        }
        if i >= b.len() || b[i] != b'"' {
            return out;
        }
        let (key, complete, next) = parse_string(src, i + 1);
        if !complete {
            return out;
        }
        i = skip_ws(b, next);
        if i >= b.len() || b[i] != b':' {
            return out;
        }
        i = skip_ws(b, i + 1);
        if i >= b.len() {
            return out;
        }
        if b[i] == b'"' {
            let (value, complete, next) = parse_string(src, i + 1);
            out.push(Field {
                key,
                value,
                complete,
            });
            if !complete {
                return out;
            }
            i = next;
        } else {
            match skip_value(b, i) {
                Some(next) => i = next,
                None => return out,
            }
        }
    }
}

pub fn get<'a>(fields: &'a [Field], key: &str) -> Option<&'a Field> {
    fields.iter().find(|f| f.key == key)
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Decodes a JSON string starting just after its opening quote.
/// Returns (decoded, complete, index after the closing quote).
fn parse_string(src: &str, start: usize) -> (String, bool, usize) {
    let b = src.as_bytes();
    let mut out = String::new();
    let mut i = start;
    let mut run = i;
    while i < b.len() {
        match b[i] {
            b'"' => {
                out.push_str(&src[run..i]);
                return (out, true, i + 1);
            }
            b'\\' => {
                out.push_str(&src[run..i]);
                let Some(&c) = b.get(i + 1) else {
                    return (out, false, i);
                };
                match c {
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'u' => {
                        let Some(cp) = src.get(i + 2..i + 6).and_then(hex) else {
                            return (out, false, i);
                        };
                        if (0xD800..0xDC00).contains(&cp) {
                            let low = (src.get(i + 6..i + 8) == Some("\\u"))
                                .then(|| src.get(i + 8..i + 12).and_then(hex))
                                .flatten();
                            match low {
                                Some(lo) if (0xDC00..0xE000).contains(&lo) => {
                                    let c = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                                    i += 12;
                                }
                                None if src.len() < i + 12 => return (out, false, i),
                                _ => {
                                    out.push('\u{FFFD}');
                                    i += 6;
                                }
                            }
                        } else {
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                            i += 6;
                        }
                        run = i;
                        continue;
                    }
                    other => out.push(other as char),
                }
                i += 2;
                run = i;
            }
            _ => i += 1,
        }
    }
    out.push_str(&src[run..i]);
    (out, false, i)
}

fn hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s, 16).ok()
}

/// Skips a non-string value; returns the index of the following `,` or `}`.
fn skip_value(b: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'{' | b'[' => depth += 1,
                b'}' | b']' => {
                    if depth == 0 {
                        return Some(i);
                    }
                    depth -= 1;
                }
                b',' if depth == 0 => return Some(i),
                _ => {}
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_content() {
        let f = string_fields(r#"{"path": "src/main.rs", "content": "fn main() {\n    println!(\"hi"#);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].value, "src/main.rs");
        assert!(f[0].complete);
        assert_eq!(f[1].value, "fn main() {\n    println!(\"hi");
        assert!(!f[1].complete);
    }

    #[test]
    fn skips_other_values_and_cut_escapes() {
        let f = string_fields(r#"{"replace_all": true, "n": {"a": [1, "x"]}, "old_text": "a\"#);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].key, "old_text");
        assert_eq!(f[0].value, "a");
        assert!(!f[0].complete);
    }

    #[test]
    fn unicode_escapes() {
        let f = string_fields(r#"{"s": "é😀!"}"#);
        assert_eq!(f[0].value, "é😀!");
        assert!(f[0].complete);
    }
}
