//! Valve KeyValues parsers: text (VDF/ACF) and binary (shortcuts.vdf, appinfo.vdf).
//!
//! CONTRACT:
//!   pub enum Vdf { Str(String), Obj(Vec<(String, Vdf)>) }
//!   pub fn parse(text: &str) -> anyhow::Result<Vdf>
//!   pub fn parse_binary / parse_binary_with_strings  binary KeyValues; numbers become decimal `Vdf::Str`s
//!   impl Vdf { get(&self, key) -> Option<&Vdf> (case-insensitive), str(&self)->Option<&str>,
//!              path(&self, &[&str]) -> Option<&Vdf>, entries(&self) -> &[(String, Vdf)], int, uint }
//!
//! Both parsers are total: malformed or truncated input returns `Err`, never panics, and nesting
//! depth is capped so hostile files cannot overflow the stack.

use anyhow::{Result, anyhow, bail};

#[derive(Clone, Debug, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

/// Deeper nesting than this is treated as garbage (real Steam files nest < 15 levels).
const MAX_DEPTH: usize = 256;

impl Vdf {
    /// Case-insensitive child lookup (first match).
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(v) => v.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }
    /// Every child with this key (case-insensitive); duplicate keys are kept by the parser.
    #[cfg(test)]
    pub fn get_all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Vdf> + 'a {
        self.entries().iter().filter(move |(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }
    pub fn path(&self, keys: &[&str]) -> Option<&Vdf> {
        keys.iter().try_fold(self, |v, k| v.get(k))
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }
    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(v) => v,
            Vdf::Str(_) => &[],
        }
    }
    /// String value parsed as a signed integer (whitespace trimmed).
    pub fn int(&self) -> Option<i64> {
        self.str()?.trim().parse().ok()
    }
    /// String value parsed as an unsigned integer.
    pub fn uint(&self) -> Option<u64> {
        self.str()?.trim().parse().ok()
    }
    /// Child `key` as a string.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key)?.str()
    }
}

// ---------------------------------------------------------------------------------------------
// Text KeyValues
// ---------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Tok {
    Str(String),
    Open,
    Close,
    Eof,
}

struct Lexer<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
    line: usize,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        let s = s.strip_prefix('\u{feff}').unwrap_or(s);
        Lexer { s, b: s.as_bytes(), i: 0, line: 1 }
    }

    fn skip_ws_and_comments(&mut self) {
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'\n' => {
                    self.line += 1;
                    self.i += 1;
                }
                b' ' | b'\t' | b'\r' | 0x0b | 0x0c => self.i += 1,
                b'/' if self.b.get(self.i + 1) == Some(&b'/') => {
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
    }

    /// Next token. `[$CONDITIONAL]` tags are consumed and skipped.
    fn token(&mut self) -> Result<Tok> {
        loop {
            self.skip_ws_and_comments();
            let Some(&c) = self.b.get(self.i) else {
                return Ok(Tok::Eof);
            };
            match c {
                b'{' => {
                    self.i += 1;
                    return Ok(Tok::Open);
                }
                b'}' => {
                    self.i += 1;
                    return Ok(Tok::Close);
                }
                b'[' => {
                    // Conditional like [$WIN32] / [!$X360&&$PS3]: skip to the closing bracket.
                    match self.b[self.i..].iter().position(|&x| x == b']' || x == b'\n') {
                        Some(p) if self.b[self.i + p] == b']' => self.i += p + 1,
                        _ => bail!("line {}: unterminated conditional", self.line),
                    }
                }
                b'"' => return self.quoted().map(Tok::Str),
                _ => {
                    let start = self.i;
                    while self.i < self.b.len() {
                        match self.b[self.i] {
                            b' ' | b'\t' | b'\r' | b'\n' | b'{' | b'}' | b'"' => break,
                            _ => self.i += 1,
                        }
                    }
                    // Only ASCII delimiters end a token, so the slice is on char boundaries.
                    return Ok(Tok::Str(self.s[start..self.i].to_string()));
                }
            }
        }
    }

    fn quoted(&mut self) -> Result<String> {
        let start_line = self.line;
        self.i += 1; // opening quote
        let mut out = String::new();
        let mut run = self.i; // start of the current unescaped run
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'"' => {
                    out.push_str(&self.s[run..self.i]);
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    out.push_str(&self.s[run..self.i]);
                    let esc = self.b.get(self.i + 1).copied();
                    match esc {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'n') => out.push('\n'),
                        Some(b't') => out.push('\t'),
                        Some(b'r') => out.push('\r'),
                        // Unknown escape: keep the backslash, let the next char be read normally.
                        _ => {
                            out.push('\\');
                            self.i += 1;
                            run = self.i;
                            continue;
                        }
                    }
                    self.i += 2;
                    run = self.i;
                }
                b'\n' => {
                    self.line += 1;
                    self.i += 1;
                }
                _ => self.i += 1,
            }
        }
        Err(anyhow!("line {start_line}: unterminated string"))
    }
}

/// Parse text KeyValues. The result is an `Obj` holding the top-level pairs
/// (usually a single one, e.g. `"AppState" { … }`).
pub fn parse(text: &str) -> Result<Vdf> {
    let mut lx = Lexer::new(text);
    // Explicit stack instead of recursion: (key of the object being built, its entries).
    let mut stack: Vec<(String, Vec<(String, Vdf)>)> = vec![(String::new(), Vec::new())];
    loop {
        match lx.token()? {
            Tok::Eof => {
                if stack.len() > 1 {
                    bail!("unexpected end of file ({} unclosed objects)", stack.len() - 1);
                }
                let (_, root) = stack.pop().unwrap_or_default();
                return Ok(Vdf::Obj(root));
            }
            Tok::Close => {
                if stack.len() == 1 {
                    bail!("line {}: unmatched '}}'", lx.line);
                }
                if let Some((key, entries)) = stack.pop()
                    && let Some(parent) = stack.last_mut()
                {
                    parent.1.push((key, Vdf::Obj(entries)));
                }
            }
            Tok::Open => bail!("line {}: '{{' without a key", lx.line),
            Tok::Str(key) => match lx.token()? {
                Tok::Str(val) => {
                    if let Some(top) = stack.last_mut() {
                        top.1.push((key, Vdf::Str(val)));
                    }
                }
                Tok::Open => {
                    if stack.len() > MAX_DEPTH {
                        bail!("nesting too deep");
                    }
                    stack.push((key, Vec::new()));
                }
                Tok::Close => bail!("line {}: key {key:?} without a value", lx.line),
                Tok::Eof => bail!("unexpected end of file after key {key:?}"),
            },
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Binary KeyValues
// ---------------------------------------------------------------------------------------------

const T_OBJECT: u8 = 0x00;
const T_STRING: u8 = 0x01;
const T_INT32: u8 = 0x02;
const T_FLOAT: u8 = 0x03;
const T_PTR: u8 = 0x04;
const T_WSTRING: u8 = 0x05;
const T_COLOR: u8 = 0x06;
const T_UINT64: u8 = 0x07;
const T_END: u8 = 0x08;
const T_INT64: u8 = 0x0A;
const T_END_ALT: u8 = 0x0B;

struct BinReader<'a, 's> {
    d: &'a [u8],
    i: usize,
    strings: Option<&'s [String]>,
}

impl BinReader<'_, '_> {
    fn u8(&mut self) -> Result<u8> {
        let v = *self.d.get(self.i).ok_or_else(|| anyhow!("truncated binary vdf"))?;
        self.i += 1;
        Ok(v)
    }
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let s = self.d.get(self.i..self.i + N).ok_or_else(|| anyhow!("truncated binary vdf"))?;
        self.i += N;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }
    fn cstr(&mut self) -> Result<String> {
        let rest = self.d.get(self.i..).unwrap_or(&[]);
        let n = rest.iter().position(|&c| c == 0).ok_or_else(|| anyhow!("unterminated string in binary vdf"))?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.i += n + 1;
        Ok(s)
    }
    /// UTF-16LE string terminated by a 0x0000 unit.
    fn wstr(&mut self) -> Result<String> {
        let mut units = Vec::new();
        loop {
            let u = u16::from_le_bytes(self.bytes::<2>()?);
            if u == 0 {
                break;
            }
            units.push(u);
        }
        Ok(String::from_utf16_lossy(&units))
    }
    fn key(&mut self) -> Result<String> {
        match self.strings {
            Some(table) => {
                let idx = u32::from_le_bytes(self.bytes::<4>()?) as usize;
                table.get(idx).cloned().ok_or_else(|| anyhow!("binary vdf key index {idx} out of range"))
            }
            None => self.cstr(),
        }
    }

    /// Read nodes until an end marker (or EOF when `top` is set).
    fn object(&mut self, depth: usize, top: bool) -> Result<Vec<(String, Vdf)>> {
        if depth > MAX_DEPTH {
            bail!("binary vdf nesting too deep");
        }
        let mut out = Vec::new();
        loop {
            if top && self.i >= self.d.len() {
                return Ok(out);
            }
            let t = self.u8()?;
            if t == T_END || t == T_END_ALT {
                return Ok(out);
            }
            let key = self.key()?;
            let val = match t {
                T_OBJECT => Vdf::Obj(self.object(depth + 1, false)?),
                T_STRING => Vdf::Str(self.cstr()?),
                T_WSTRING => Vdf::Str(self.wstr()?),
                T_INT32 => Vdf::Str(i32::from_le_bytes(self.bytes::<4>()?).to_string()),
                T_PTR | T_COLOR => Vdf::Str(u32::from_le_bytes(self.bytes::<4>()?).to_string()),
                T_FLOAT => Vdf::Str(f32::from_le_bytes(self.bytes::<4>()?).to_string()),
                T_UINT64 => Vdf::Str(u64::from_le_bytes(self.bytes::<8>()?).to_string()),
                T_INT64 => Vdf::Str(i64::from_le_bytes(self.bytes::<8>()?).to_string()),
                other => bail!("unknown binary vdf type 0x{other:02x} at byte {}", self.i),
            };
            out.push((key, val));
        }
    }
}

/// Parse binary KeyValues with inline C-string keys (shortcuts.vdf, appinfo ≤ v28).
/// Numbers become decimal strings (int32 stays signed: use `Vdf::int` and cast).
/// Returns an `Obj` of the top-level nodes; parsing stops at the first unmatched end marker.
pub fn parse_binary(data: &[u8]) -> Result<Vdf> {
    let mut r = BinReader { d: data, i: 0, strings: None };
    Ok(Vdf::Obj(r.object(0, true)?))
}

/// Parse binary KeyValues whose keys are little-endian u32 indexes into `strings`
/// (appinfo.vdf v29).
pub fn parse_binary_with_strings(data: &[u8], strings: &[String]) -> Result<Vdf> {
    let mut r = BinReader { d: data, i: 0, strings: Some(strings) };
    Ok(Vdf::Obj(r.object(0, true)?))
}

/// Builder for binary KeyValues, used by tests (and handy for fixtures).
#[cfg(test)]
pub mod write {
    pub fn obj(out: &mut Vec<u8>, key: &str) {
        out.push(0x00);
        out.extend_from_slice(key.as_bytes());
        out.push(0);
    }
    pub fn str(out: &mut Vec<u8>, key: &str, val: &str) {
        out.push(0x01);
        out.extend_from_slice(key.as_bytes());
        out.push(0);
        out.extend_from_slice(val.as_bytes());
        out.push(0);
    }
    pub fn int(out: &mut Vec<u8>, key: &str, val: i32) {
        out.push(0x02);
        out.extend_from_slice(key.as_bytes());
        out.push(0);
        out.extend_from_slice(&val.to_le_bytes());
    }
    pub fn end(out: &mut Vec<u8>) {
        out.push(0x08);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_basic_acf() {
        let t = r#"
"AppState"
{
	"appid"		"292030"
	"name"		"The Witcher 3: Wild Hunt"
	"StateFlags"		"4"
	"UserConfig"
	{
		"language"		"english"
	}
}
"#;
        let v = parse(t).unwrap();
        let app = v.get("appstate").unwrap();
        assert_eq!(app.get_str("APPID"), Some("292030"));
        assert_eq!(app.get("stateflags").and_then(Vdf::int), Some(4));
        assert_eq!(app.path(&["UserConfig", "language"]).and_then(Vdf::str), Some("english"));
        assert_eq!(app.get("missing"), None);
    }

    #[test]
    fn text_escapes_comments_conditionals_unquoted() {
        let t = "// header comment\n\
                 root { // trailing\n\
                 \"a\\\"b\" \"x\\\\y\\nz\\tw\" [$WIN32]\n\
                 unq val\n\
                 \"path\" \"C:\\Program Files\"\n\
                 dup 1 dup 2\n\
                 \"empty\" \"\"\n\
                 \"ünï\" \"cødé\"\n\
                 }";
        let v = parse(t).unwrap();
        let r = v.get("root").unwrap();
        assert_eq!(r.get_str("a\"b"), Some("x\\y\nz\tw"));
        assert_eq!(r.get_str("unq"), Some("val"));
        assert_eq!(r.get_str("path"), Some("C:\\Program Files"));
        let dups: Vec<_> = r.get_all("dup").filter_map(Vdf::str).collect();
        assert_eq!(dups, vec!["1", "2"]);
        assert_eq!(r.get_str("empty"), Some(""));
        assert_eq!(r.get_str("ünï"), Some("cødé"));
    }

    #[test]
    fn text_bom_and_multiple_roots() {
        let v = parse("\u{feff}\"a\" \"1\"\n\"b\" { }").unwrap();
        assert_eq!(v.entries().len(), 2);
        assert_eq!(v.get("b"), Some(&Vdf::Obj(vec![])));
    }

    #[test]
    fn text_garbage_is_err_not_panic() {
        for bad in [
            "\"a\" {",
            "\"a\" { \"b\" \"c\"",
            "}",
            "{",
            "\"a\"",
            "\"a\" \"unterminated",
            "\"a\" { \"b\" }",
            "\"a\" [$X",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should fail");
        }
        // Truncations of a valid file never panic.
        let ok = "\"x\" { \"y\" { \"z\" \"\\\"q\\\"\" } // c\n }";
        for n in 0..ok.len() {
            if ok.is_char_boundary(n) {
                let _ = parse(&ok[..n]);
            }
        }
        // Deep nesting is rejected, not a stack overflow.
        let deep = "a {".repeat(100_000);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn binary_roundtrip() {
        let mut b = Vec::new();
        write::obj(&mut b, "shortcuts");
        write::obj(&mut b, "0");
        write::int(&mut b, "appid", -1_234_567);
        write::str(&mut b, "AppName", "My Game");
        b.push(T_UINT64);
        b.extend_from_slice(b"big\0");
        b.extend_from_slice(&u64::MAX.to_le_bytes());
        b.push(T_FLOAT);
        b.extend_from_slice(b"f\0");
        b.extend_from_slice(&1.5f32.to_le_bytes());
        b.push(T_INT64);
        b.extend_from_slice(b"neg\0");
        b.extend_from_slice(&(-5i64).to_le_bytes());
        write::obj(&mut b, "tags");
        write::str(&mut b, "0", "fav");
        write::end(&mut b);
        write::end(&mut b);
        write::end(&mut b);
        write::end(&mut b);
        let v = parse_binary(&b).unwrap();
        let e = v.path(&["shortcuts", "0"]).unwrap();
        assert_eq!(e.get("appid").and_then(Vdf::int), Some(-1_234_567));
        assert_eq!(e.get_str("appname"), Some("My Game"));
        assert_eq!(e.get("big").and_then(Vdf::uint), Some(u64::MAX));
        assert_eq!(e.get_str("f"), Some("1.5"));
        assert_eq!(e.get("neg").and_then(Vdf::int), Some(-5));
        assert_eq!(e.path(&["tags", "0"]).and_then(Vdf::str), Some("fav"));
    }

    #[test]
    fn binary_string_table_keys() {
        let strings: Vec<String> = ["appinfo", "common", "name", "type"].map(String::from).into();
        let mut b = vec![T_OBJECT];
        b.extend_from_slice(&0u32.to_le_bytes());
        b.push(T_OBJECT);
        b.extend_from_slice(&1u32.to_le_bytes());
        b.push(T_STRING);
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(b"Game X\0");
        b.push(T_STRING);
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(b"Game\0");
        b.extend_from_slice(&[T_END, T_END, T_END]);
        let v = parse_binary_with_strings(&b, &strings).unwrap();
        assert_eq!(v.path(&["appinfo", "common", "name"]).and_then(Vdf::str), Some("Game X"));
        // Bad index -> Err
        let mut bad = vec![T_STRING];
        bad.extend_from_slice(&99u32.to_le_bytes());
        bad.extend_from_slice(b"x\0");
        assert!(parse_binary_with_strings(&bad, &strings).is_err());
    }

    #[test]
    fn binary_garbage_never_panics() {
        let mut b = Vec::new();
        write::obj(&mut b, "s");
        write::int(&mut b, "i", 7);
        write::str(&mut b, "k", "v");
        write::end(&mut b);
        for n in 0..b.len() {
            let _ = parse_binary(&b[..n]);
        }
        assert!(parse_binary(&[0x42, b'a', 0]).is_err());
        // Pseudo-random garbage
        let mut x: u32 = 12345;
        let junk: Vec<u8> = (0..4096)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (x >> 16) as u8 % 12
            })
            .collect();
        let _ = parse_binary(&junk);
        let _ = parse_binary_with_strings(&junk, &["a".into()]);
        let deep = [0u8, b'a', 0].repeat(10_000);
        assert!(parse_binary(&deep).is_err());
    }
}
