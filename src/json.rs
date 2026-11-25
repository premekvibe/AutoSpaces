//! Tiny JSON reader/writer helpers (integers only), shared by variants.json and state.json.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(i64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_uint(&self) -> Option<u64> {
        match *self {
            Json::Num(n) if (0..4_000_000_000).contains(&n) => Some(n as u64),
            _ => None,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("JSON: {what} at byte {}", self.i))
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            self.err(&format!("expected '{}'", c as char))
        }
    }
    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            self.err("unexpected token")
        }
    }
    fn value(&mut self, depth: u32) -> Result<Json, String> {
        if depth > 32 {
            return self.err("nesting too deep");
        }
        self.ws();
        match self.s.get(self.i) {
            None => self.err("unexpected end"),
            Some(b'{') => {
                self.i += 1;
                let mut m = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    m.push((k, self.value(depth + 1)?));
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        _ => return self.err("expected ',' or '}'"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(a));
                        }
                        _ => return self.err("expected ',' or ']'"),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(_) => {
                let start = self.i;
                // Integers only: nothing in the schema needs fractions.
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i])
                    .ok()
                    .and_then(|t| t.parse::<i64>().ok())
                    .map(Json::Num)
                    .map_or_else(|| self.err("bad number"), Ok)
            }
        }
    }
    /// Four hex digits of a `\u` escape (advances past them).
    fn hex4(&mut self) -> Option<u32> {
        let h = self.s.get(self.i..self.i + 4)?;
        let code = std::str::from_utf8(h).ok().filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))?;
        self.i += 4;
        u32::from_str_radix(code, 16).ok()
    }
    fn string(&mut self) -> Result<String, String> {
        if self.s.get(self.i) != Some(&b'"') {
            return self.err("expected string");
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else {
                return self.err("unterminated string");
            };
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| "JSON: invalid UTF-8".into()),
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else {
                        return self.err("bad escape");
                    };
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let Some(code) = self.hex4() else {
                                return self.err("bad \\u escape");
                            };
                            // A UTF-16 surrogate pair (emoji in a GitHub release text) is one character;
                            // a lone surrogate becomes U+FFFD instead of failing the whole document.
                            let code = if (0xD800..0xDC00).contains(&code) && self.s.get(self.i..self.i + 2) == Some(b"\\u") {
                                let save = self.i;
                                self.i += 2;
                                match self.hex4() {
                                    Some(lo) if (0xDC00..0xE000).contains(&lo) => 0x10000 + ((code - 0xD800) << 10) + (lo - 0xDC00),
                                    _ => {
                                        self.i = save;
                                        0xFFFD
                                    }
                                }
                            } else {
                                code
                            };
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        _ => return self.err("bad escape"),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                _ => out.push(c),
            }
        }
    }
}

pub fn parse_json(text: &str) -> Result<Json, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return p.err("trailing data");
    }
    Ok(v)
}


/// JSON string literal with escaping.
pub fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_roundtrip() {
        let t = "a\"b\\c\n\u{1}é";
        assert_eq!(parse_json(&quote(t)), Ok(Json::Str(t.into())));
    }

    #[test]
    fn json_basics() {
        let j = parse_json(r#" {"a":[1,-25,true,false,null],"b":"x\"\\\u0041\n"} "#).unwrap();
        assert_eq!(
            j.get("a"),
            Some(&Json::Arr(vec![Json::Num(1), Json::Num(-25), Json::Bool(true), Json::Bool(false), Json::Null]))
        );
        assert_eq!(j.get("b"), Some(&Json::Str("x\"\\A\n".into())));
        assert!(parse_json("{\"a\":}").is_err());
        assert!(parse_json("[1,]").is_err());
        assert!(parse_json("[1.5]").is_err());
        assert!(parse_json("\"abc").is_err());
        assert_eq!(parse_json("\u{feff}[]"), Ok(Json::Arr(vec![])));
    }

    #[test]
    fn surrogate_pairs_and_lone_surrogates() {
        assert_eq!(parse_json(r#""a😀b""#), Ok(Json::Str("a\u{1F600}b".into())));
        assert_eq!(parse_json(r#""\ud83dx""#), Ok(Json::Str("\u{FFFD}x".into())));
        assert_eq!(parse_json(r#""\ud83dA""#), Ok(Json::Str("\u{FFFD}A".into())));
        assert_eq!(parse_json(r#""\ude00""#), Ok(Json::Str("\u{FFFD}".into())));
        assert!(parse_json(r#""\u12""#).is_err());
        assert!(parse_json(r#""\u12g4""#).is_err());
    }
}
