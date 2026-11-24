//! Playnite's built-in value converters, as pure string functions. Themes can't ship code, so
//! every `{Binding …, Converter={StaticResource X}}` names one of Playnite's own converters
//! (`source/Playnite/Converters`, MIT); this is the set the theme corpus uses, plus the
//! arithmetic part of the `MathConverter` expressions themes size things with.
//!
//! CONTRACT:
//!   /// Run converter `name` ("BooleanToVisibilityConverter", "ListToString", "pcon:Negate" …)
//!   /// on `input` with an optional ConverterParameter. None: not a converter Lodestar knows,
//!   /// or an input it can't convert (the element keeps its unconverted value / default).
//!   pub fn apply(name: &str, input: &str, param: Option<&str>) -> Option<String>
//!   /// MathConverter: evaluate `expr` with x / y / z / a … and [0] [1] … bound to `values`.
//!   pub fn math(expr: &str, values: &[&str]) -> Option<String>
//!
//! How values are written (the binder's raw strings):
//! - null / missing = "" (empty);
//! - booleans "True" / "False" (any case; "1" / "0" too); visibilities "Visible" / "Collapsed"
//!   / "Hidden";
//! - lists = items joined by '\n' (a value without '\n' is one item, as WPF's `ToString()`);
//! - durations = seconds; date-times = Unix seconds or an ISO date ("2025-03-12[T…]");
//! - sizes = bytes. Numbers come out in WPF's invariant form ("0.5", "-310", "12").
//!
//! MathConverter here: + − × ÷ %, parentheses, unary minus / `!`, comparisons
//! (< > <= >= == !=), `&&` / `||`, `c ? a : b`, `Max` `Min` `Abs` `Round` `Floor` `Ceiling`
//! `Sqrt` `Pow`, `true` / `false` / `null`, backtick strings (`` `Visible` ``) and a top-level
//! comma list ("0, 0, [0], [1]" → a Thickness "0,0,8,4"). Every value is one binding's
//! input; an expression that reads a value it wasn't given gives None.

use super::loc;

const VISIBLE: &str = "Visible";
const COLLAPSED: &str = "Collapsed";
const HIDDEN: &str = "Hidden";

/// Run Playnite converter `name` (see the module docs).
pub fn apply(name: &str, input: &str, param: Option<&str>) -> Option<String> {
    let key = normalise(name);
    let s = |v: &str| Some(v.to_string());
    let vis = |b: bool| s(if b { VISIBLE } else { COLLAPSED });
    let raw_param = param;
    let param = param.map(str::trim);
    match key.as_str() {
        // Booleans → visibility.
        "booleantovisibility" | "multibooleantovisibility" => match input.trim() {
            "" => vis(false),
            v if v.contains('\n') => vis(v.lines().all(|l| boolean(l) == Some(true))),
            v => boolean(v).map(vis)?,
        },
        "invertedbooleantovisibility" => vis(boolean(input) != Some(true)),
        "invertablebooleantovisibility" => {
            let b = boolean(input)?;
            vis(if param.is_some_and(|p| p.eq_ignore_ascii_case("inverted")) { !b } else { b })
        }
        "booleantohidden" => s(if boolean(input) == Some(true) { VISIBLE } else { HIDDEN }),
        "invertedbooleantohidden" => s(if boolean(input) == Some(true) { HIDDEN } else { VISIBLE }),
        "booltoyesno" => s(if boolean(input)? { "Yes" } else { "No" }),
        // Booleans.
        "invertedboolen" | "invertedboolean" | "invertedbool" => boolean(input).map(|b| bool_text(!b)),
        "negate" => match boolean(input) {
            Some(b) => Some(bool_text(!b)),
            None => Some(bool_text(false)),
        },
        "opacitybool" => {
            let b = boolean(input)?;
            let flip = param.and_then(boolean).unwrap_or(false);
            s(if b != flip { "1" } else { "0.5" })
        }
        // Null / empty tests.
        "nulltovisibility" => vis(!input.trim().is_empty()),
        "invertednulltovisibility" => vis(input.trim().is_empty()),
        "nulltobool" => Some(bool_text(!input.trim().is_empty())),
        "invertednulltobool" => Some(bool_text(input.trim().is_empty())),
        "stringnulloremptytovisibility" => vis(!input.trim().is_empty()),
        "invertedstringnulloremptytovisibility" => vis(input.trim().is_empty()),
        "stringnullorempytobool" | "stringnulloremptytobool" => Some(bool_text(input.trim().is_empty())),
        "icollectionnullorEmptytovisibility" | "icollectionnulloremptytovisibility" | "listsizetovisibility" => {
            vis(!items(input).is_empty())
        }
        "listsizetobool" => Some(bool_text(!items(input).is_empty())),
        // Equality.
        "objectequalitytobool" => Some(bool_text(equal(input, param))),
        "invertedobjectequalitytobool" => Some(bool_text(!equal(input, param))),
        "objectequalitytovisibility" => vis(equal(input, param)),
        "invertedobjectequalitytovisibility" => vis(!equal(input, param)),
        "inttovisibility" => {
            let (v, p) = (int(input)?, int(param?)?);
            vis(v == p)
        }
        "invertedinttovisibility" => match (int(input), param.and_then(int)) {
            (Some(v), Some(p)) => vis(v != p),
            _ => vis(true),
        },
        // Text.
        "objecttostring" | "imagestringtoimage" | "imagestringtoimagecached" | "nullablestring" => s(input),
        "stringtouppercase" => Some(input.to_uppercase()),
        "listtostring" => {
            let it = items(input);
            Some(it.join(raw_param.map(|p| p.trim_matches('\'')).unwrap_or(",")))
        }
        "nicelisttostring" => Some(items(input).join(", ")),
        // Numbers / dates.
        "playtimetostring" => playtime(input, param.and_then(boolean).unwrap_or(false)),
        "datetimetolastplayed" => match input.trim() {
            "" => text("LOCNever"),
            v => date_time(v).map(|t| crate::model::format_last_played(Some(t))),
        },
        "nullabledatetostring" => match input.trim() {
            "" => s(""),
            v => date_time(v).map(date_text),
        },
        "releasedatetostring" => match input.trim() {
            "" => s(""),
            v => Some(release_date(v)),
        },
        "nullableulongbytessizetostring" => match input.trim() {
            "" => s(""),
            v => v.parse::<u64>().ok().map(bytes_text),
        },
        "ulongtostring" | "nullableinttostring" | "nullableulongtostring" => s(input.trim()),
        "math" => math(param?, &[input]),
        _ => None,
    }
}

/// "{StaticResource BooleanToVisibilityConverter}", "pcon:BooleanToVisibilityConverter",
/// "BooleanToVisibility" → "booleantovisibility".
fn normalise(name: &str) -> String {
    let mut n = name.trim().trim_matches(|c| c == '{' || c == '}').trim();
    for p in ["StaticResource", "DynamicResource"] {
        if let Some(r) = n.strip_prefix(p) {
            n = r.trim();
        }
    }
    let n = n.rsplit(':').next().unwrap_or(n);
    let lower = n.to_ascii_lowercase();
    let lower = lower.strip_suffix("cached").map(|s| format!("{s}cached")).unwrap_or(lower);
    let lower = lower.replace("convertercached", "cached");
    lower.strip_suffix("converter").map(str::to_string).unwrap_or(lower)
}

fn text(key: &str) -> Option<String> {
    loc::standard(key).map(str::to_string)
}

fn bool_text(b: bool) -> String {
    if b { "True".into() } else { "False".into() }
}

/// "True" / "False" / "1" / "0" / "Visible" / "Collapsed" (as WPF's bool parse plus the
/// visibility values themes put in Tags).
pub(crate) fn boolean(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "visible" | "yes" => Some(true),
        "false" | "0" | "collapsed" | "hidden" | "no" => Some(false),
        _ => None,
    }
}

fn int(v: &str) -> Option<i64> {
    let t = v.trim();
    t.parse::<i64>().ok().or_else(|| t.parse::<f64>().ok().filter(|f| f.fract() == 0.0).map(|f| f as i64))
}

fn items(v: &str) -> Vec<&str> {
    if v.trim().is_empty() {
        return vec![];
    }
    v.split('\n').map(str::trim).filter(|s| !s.is_empty()).collect()
}

fn equal(v: &str, p: Option<&str>) -> bool {
    let Some(p) = p else { return false };
    if v.trim().is_empty() {
        return false;
    }
    let (a, b) = (v.trim(), p.trim().trim_matches('\''));
    a == b
        || match (a.parse::<f64>(), b.parse::<f64>()) {
            (Ok(x), Ok(y)) => x == y,
            _ => {
                matches!((boolean(a), boolean(b)), (Some(x), Some(y)) if x == y && !a.chars().all(|c| c.is_ascii_digit()))
            }
        }
}

/// Playnite's PlayTimeToStringConverter, in English: "Not played", "45 seconds",
/// "12 minutes", "3h 20m", with `days`: "2d 3h 20m" from 24 h on.
fn playtime(input: &str, days: bool) -> Option<String> {
    let t = input.trim();
    let secs: u64 = if t.is_empty() { 0 } else { t.parse::<f64>().ok().filter(|v| *v >= 0.0)? as u64 };
    let fmt = |key: &str, args: &[u64]| -> Option<String> {
        let mut s = loc::standard(key)?.to_string();
        for (i, a) in args.iter().enumerate() {
            s = s.replace(&format!("{{{i}}}"), &a.to_string());
        }
        Some(s)
    };
    if secs == 0 {
        return fmt("LOCPlayedNone", &[]);
    }
    if secs < 60 {
        return fmt("LOCPlayedSeconds", &[secs]);
    }
    let minutes = secs / 60;
    if minutes < 60 {
        return fmt("LOCPlayedMinutes", &[minutes]);
    }
    let hours = minutes / 60;
    if days && hours >= 24 {
        return fmt("LOCPlayedDays", &[hours / 24, hours % 24, minutes % 60]);
    }
    fmt("LOCPlayedHours", &[hours, minutes - hours * 60])
}

/// Unix seconds from "1731000000" or an ISO date / date-time ("2025-03-12", "2025-03-12T10:00:00").
fn date_time(v: &str) -> Option<i64> {
    let t = v.trim();
    if let Ok(n) = t.parse::<i64>() {
        return Some(n);
    }
    let (y, m, d) = ymd(t)?;
    Some(days_from_civil(y, m.max(1), d.max(1)) * 86_400)
}

/// (year, month, day) from "2025", "2025-03", "2025-03-12" or "2025-03-12T…" (0 = not given).
fn ymd(t: &str) -> Option<(i64, u32, u32)> {
    let date = t.split(['T', ' ']).next()?;
    let mut it = date.split(['-', '/']);
    let y: i64 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next().map(|m| m.trim().parse().ok()).unwrap_or(Some(0))?;
    let d: u32 = it.next().map(|d| d.trim().parse().ok()).unwrap_or(Some(0))?;
    (m <= 12 && d <= 31 && (1..=9999).contains(&y)).then_some((y, m, d))
}

/// Days since 1970-01-01 (Howard Hinnant's civil-from-days inverse).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "12 Mar 2025" (Lodestar's date style).
fn date_text(unix: i64) -> String {
    let (y, m, d) = civil_from_days(unix.div_euclid(86_400));
    format!("{d} {} {y}", MONTHS[(m as usize).clamp(1, 12) - 1])
}

/// A release date as far as it is known: "12 Mar 2025", "Mar 2025", "2025".
fn release_date(v: &str) -> String {
    if let Ok(n) = v.trim().parse::<i64>()
        && !(1..=9999).contains(&n)
    {
        return date_text(n);
    }
    match ymd(v) {
        Some((y, 0, _)) => y.to_string(),
        Some((y, m, 0)) => format!("{} {y}", MONTHS[m as usize - 1]),
        Some((y, m, d)) => format!("{d} {} {y}", MONTHS[m as usize - 1]),
        None => v.trim().to_string(),
    }
}

/// Playnite's NullableUlongBytesSizeToStringConverter: "512 B", "1.50 KB", "4.375 GB".
fn bytes_text(bytes: u64) -> String {
    let (suffix, readable) = if bytes >= 0x1000_0000_0000_0000 {
        ("EB", (bytes >> 50) as f64)
    } else if bytes >= 0x4_0000_0000_0000 {
        ("PB", (bytes >> 40) as f64)
    } else if bytes >= 0x100_0000_0000 {
        ("TB", (bytes >> 30) as f64)
    } else if bytes >= 0x4000_0000 {
        ("GB", (bytes >> 20) as f64)
    } else if bytes >= 0x10_0000 {
        ("MB", (bytes >> 10) as f64)
    } else if bytes >= 0x400 {
        ("KB", bytes as f64)
    } else {
        return format!("{bytes} B");
    };
    // .NET "0.00#": two decimals at least, three at most.
    let mut s = format!("{:.3}", readable / 1024.0);
    if s.ends_with('0') {
        s.pop();
    }
    format!("{s} {suffix}")
}

// ---- MathConverter --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum V {
    Num(f64),
    Bool(bool),
    Str(String),
    Null,
}

impl V {
    fn from_input(s: &str) -> V {
        let t = s.trim();
        if t.is_empty() {
            return V::Null;
        }
        if let Ok(n) = t.parse::<f64>() {
            return V::Num(n);
        }
        match t.to_ascii_lowercase().as_str() {
            "true" => V::Bool(true),
            "false" => V::Bool(false),
            _ => V::Str(t.to_string()),
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            V::Num(n) => Some(*n),
            V::Bool(b) => Some(*b as u8 as f64),
            V::Str(s) => s.trim().parse().ok(),
            V::Null => None,
        }
    }

    fn truth(&self) -> Option<bool> {
        match self {
            V::Bool(b) => Some(*b),
            V::Num(n) => Some(*n != 0.0),
            V::Str(s) => boolean(s),
            V::Null => Some(false),
        }
    }

    fn text(&self) -> String {
        match self {
            V::Num(n) => num_text(*n),
            V::Bool(b) => bool_text(*b),
            V::Str(s) => s.clone(),
            V::Null => String::new(),
        }
    }
}

/// WPF's invariant number text: "12", "0.5", "-310", "0.333333333333333".
fn num_text(n: f64) -> String {
    if !n.is_finite() {
        return if n.is_nan() {
            "NaN".into()
        } else if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{:.12}", n);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Index(usize),
    Str(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Question,
    Colon,
}

fn lex(expr: &str) -> Option<Vec<Tok>> {
    let s = expr.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = vec![];
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let st = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                i += 1;
            }
            out.push(Tok::Num(c[st..i].iter().collect::<String>().parse().ok()?));
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(c[st..i].iter().collect()));
            continue;
        }
        match ch {
            '[' => {
                let st = i + 1;
                let end = c[st..].iter().position(|x| *x == ']')? + st;
                out.push(Tok::Index(c[st..end].iter().collect::<String>().trim().parse().ok()?));
                i = end + 1;
                continue;
            }
            '`' | '"' | '\'' => {
                let st = i + 1;
                let end = c[st..].iter().position(|x| *x == ch)? + st;
                out.push(Tok::Str(c[st..end].iter().collect()));
                i = end + 1;
                continue;
            }
            '(' => out.push(Tok::LParen),
            ')' => out.push(Tok::RParen),
            ',' => out.push(Tok::Comma),
            '?' => out.push(Tok::Question),
            ':' => out.push(Tok::Colon),
            _ => {
                let two: String = c[i..(i + 2).min(c.len())].iter().collect();
                let op = ["<=", ">=", "==", "!=", "&&", "||"].into_iter().find(|o| *o == two);
                if let Some(o) = op {
                    out.push(Tok::Op(o));
                    i += 2;
                    continue;
                }
                let one = ["+", "-", "*", "/", "%", "<", ">", "!", "="].into_iter().find(|o| o.starts_with(ch))?;
                out.push(Tok::Op(if one == "=" { "==" } else { one }));
            }
        }
        i += 1;
    }
    Some(out)
}

struct P<'a> {
    t: &'a [Tok],
    i: usize,
    vals: &'a [V],
    /// Nesting of brackets / unary operators so far: theme text can nest them thousands deep,
    /// which would overflow the import thread's stack (and abort the whole program).
    depth: u32,
}

/// Deeper expressions than this give None (real themes nest a handful).
const MAX_DEPTH: u32 = 64;

impl P<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }

    fn eat(&mut self, want: &Tok) -> bool {
        if self.peek() == Some(want) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn op(&mut self, ops: &[&str]) -> Option<&'static str> {
        if let Some(Tok::Op(o)) = self.peek()
            && ops.contains(o)
        {
            let o = *o;
            self.i += 1;
            return Some(o);
        }
        None
    }

    fn ternary(&mut self) -> Option<V> {
        self.depth += 1;
        let v = if self.depth > MAX_DEPTH { None } else { self.ternary_inner() };
        self.depth -= 1;
        v
    }

    fn ternary_inner(&mut self) -> Option<V> {
        let c = self.or()?;
        if self.eat(&Tok::Question) {
            let a = self.ternary()?;
            if !self.eat(&Tok::Colon) {
                return None;
            }
            let b = self.ternary()?;
            return Some(if c.truth()? { a } else { b });
        }
        Some(c)
    }

    fn or(&mut self) -> Option<V> {
        let mut l = self.and()?;
        while self.op(&["||"]).is_some() {
            let r = self.and()?;
            l = V::Bool(l.truth()? || r.truth()?);
        }
        Some(l)
    }

    fn and(&mut self) -> Option<V> {
        let mut l = self.cmp()?;
        while self.op(&["&&"]).is_some() {
            let r = self.cmp()?;
            l = V::Bool(l.truth()? && r.truth()?);
        }
        Some(l)
    }

    fn cmp(&mut self) -> Option<V> {
        let l = self.add()?;
        let Some(o) = self.op(&["<", ">", "<=", ">=", "==", "!="]) else {
            return Some(l);
        };
        let r = self.add()?;
        let b = match o {
            "==" | "!=" => {
                let eq = match (l.num(), r.num()) {
                    (Some(a), Some(b)) if !matches!((&l, &r), (V::Str(_), V::Str(_))) => a == b,
                    _ => match (&l, &r) {
                        (V::Bool(a), b) | (b, V::Bool(a)) => b.truth() == Some(*a),
                        _ => l.text().eq_ignore_ascii_case(&r.text()),
                    },
                };
                if o == "==" { eq } else { !eq }
            }
            _ => {
                let (a, b) = (l.num()?, r.num()?);
                match o {
                    "<" => a < b,
                    ">" => a > b,
                    "<=" => a <= b,
                    _ => a >= b,
                }
            }
        };
        Some(V::Bool(b))
    }

    fn add(&mut self) -> Option<V> {
        let mut l = self.mul()?;
        while let Some(o) = self.op(&["+", "-"]) {
            let r = self.mul()?;
            l = match (o, &l, &r) {
                ("+", V::Str(a), b) => V::Str(format!("{a}{}", b.text())),
                ("+", a, V::Str(b)) if r.num().is_none() => V::Str(format!("{}{b}", a.text())),
                ("+", _, _) => V::Num(l.num()? + r.num()?),
                _ => V::Num(l.num()? - r.num()?),
            };
        }
        Some(l)
    }

    fn mul(&mut self) -> Option<V> {
        let mut l = self.unary()?;
        while let Some(o) = self.op(&["*", "/", "%"]) {
            let r = self.unary()?;
            let (a, b) = (l.num()?, r.num()?);
            l = V::Num(match o {
                "*" => a * b,
                "/" => a / b,
                _ => a % b,
            });
        }
        Some(l)
    }

    fn unary(&mut self) -> Option<V> {
        self.depth += 1;
        let v = if self.depth > MAX_DEPTH { None } else { self.unary_inner() };
        self.depth -= 1;
        v
    }

    fn unary_inner(&mut self) -> Option<V> {
        if self.op(&["-"]).is_some() {
            return Some(V::Num(-self.unary()?.num()?));
        }
        if self.op(&["+"]).is_some() {
            return Some(V::Num(self.unary()?.num()?));
        }
        if self.op(&["!"]).is_some() {
            return Some(V::Bool(!self.unary()?.truth()?));
        }
        self.atom()
    }

    fn atom(&mut self) -> Option<V> {
        let t = self.peek()?.clone();
        self.i += 1;
        match t {
            Tok::Num(n) => Some(V::Num(n)),
            Tok::Str(s) => Some(V::Str(s)),
            Tok::Index(i) => self.vals.get(i).cloned(),
            Tok::LParen => {
                let v = self.ternary()?;
                self.eat(&Tok::RParen).then_some(v)
            }
            Tok::Ident(id) => {
                if self.eat(&Tok::LParen) {
                    let mut args = vec![];
                    if !self.eat(&Tok::RParen) {
                        loop {
                            args.push(self.ternary()?);
                            if self.eat(&Tok::RParen) {
                                break;
                            }
                            if !self.eat(&Tok::Comma) {
                                return None;
                            }
                        }
                    }
                    return call(&id, &args);
                }
                match id.to_ascii_lowercase().as_str() {
                    "true" => Some(V::Bool(true)),
                    "false" => Some(V::Bool(false)),
                    "null" => Some(V::Null),
                    // x y z then a b c … name the values in order (MathConverter's convention).
                    v if v.len() == 1 => {
                        let ch = v.as_bytes()[0];
                        let idx = match ch {
                            b'x'..=b'z' => (ch - b'x') as usize,
                            b'a'..=b'w' => 3 + (ch - b'a') as usize,
                            _ => return None,
                        };
                        self.vals.get(idx).cloned()
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

fn call(f: &str, a: &[V]) -> Option<V> {
    let n = |i: usize| a.get(i).and_then(V::num);
    let v = match f.to_ascii_lowercase().as_str() {
        "max" => a.iter().map(V::num).collect::<Option<Vec<_>>>()?.into_iter().reduce(f64::max)?,
        "min" => a.iter().map(V::num).collect::<Option<Vec<_>>>()?.into_iter().reduce(f64::min)?,
        "abs" => n(0)?.abs(),
        "round" => match n(1) {
            Some(d) => {
                let p = 10f64.powi(d as i32);
                round_even(n(0)? * p) / p
            }
            None => round_even(n(0)?),
        },
        "floor" => n(0)?.floor(),
        "ceiling" | "ceil" => n(0)?.ceil(),
        "sqrt" => n(0)?.sqrt(),
        "pow" => n(0)?.powf(n(1)?),
        _ => return None,
    };
    Some(V::Num(v))
}

/// .NET Math.Round: halves go to the even neighbour.
fn round_even(x: f64) -> f64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && r % 2.0 != 0.0 { r - x.signum() } else { r }
}

/// MathConverter (see the module docs).
pub fn math(expr: &str, values: &[&str]) -> Option<String> {
    let toks = lex(expr)?;
    let vals: Vec<V> = values.iter().map(|v| V::from_input(v)).collect();
    let mut p = P { t: &toks, i: 0, vals: &vals, depth: 0 };
    let mut parts = vec![p.ternary()?];
    while p.eat(&Tok::Comma) {
        parts.push(p.ternary()?);
    }
    if p.i != toks.len() {
        return None;
    }
    let texts: Vec<String> = parts.iter().map(V::text).collect();
    Some(texts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deeply_nested_math_gives_up_instead_of_overflowing_the_stack() {
        // On a small stack: thousands of brackets or signs must not abort the program.
        let t = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let n = 5000;
                let brackets = format!("{}x{}", "(".repeat(n), ")".repeat(n));
                let signs = format!("{}x", "-".repeat(n));
                let nots = format!("{}x", "!".repeat(n));
                let calls = format!("{}x{}", "Max(".repeat(n), ")".repeat(n));
                [brackets, signs, nots, calls].map(|e| apply("MathConverter", "1", Some(&e)))
            })
            .unwrap();
        assert_eq!(t.join().unwrap(), [None, None, None, None]);
        // What themes really write still works.
        assert_eq!(apply("MathConverter", "4", Some("((((x * 2)))) - -1")).as_deref(), Some("9"));
        assert_eq!(math("!!!(x > 1) ? 1 : 2", &["4"]).as_deref(), Some("2"));
    }

    #[test]
    fn converters_table() {
        let none: Option<&str> = None;
        let rows: &[(&str, &str, Option<&str>, Option<&str>)] = &[
            ("BooleanToVisibilityConverter", "True", none, Some("Visible")),
            ("{StaticResource BooleanToVisibilityConverter}", "False", none, Some("Collapsed")),
            ("BooleanToVisibility", "", none, Some("Collapsed")),
            ("MultiBooleanToVisibilityConverter", "True\nTrue", none, Some("Visible")),
            ("MultiBooleanToVisibilityConverter", "True\nFalse", none, Some("Collapsed")),
            ("InvertedBooleanToVisibilityConverter", "True", none, Some("Collapsed")),
            ("InvertedBooleanToVisibilityConverter", "False", none, Some("Visible")),
            ("InvertedBooleanToVisibilityConverter", "", none, Some("Visible")),
            ("InvertableBooleanToVisibilityConverter", "True", Some("Inverted"), Some("Collapsed")),
            ("InvertableBooleanToVisibilityConverter", "True", Some("Normal"), Some("Visible")),
            ("BooleanToHiddenConverter", "False", none, Some("Hidden")),
            ("BoolToYesNoConverter", "True", none, Some("Yes")),
            ("InvertedBoolenConverter", "True", none, Some("False")),
            ("NegateConverter", "False", none, Some("True")),
            ("NegateConverter", "x", none, Some("False")),
            ("OpacityBoolConverter", "True", none, Some("1")),
            ("OpacityBoolConverter", "False", none, Some("0.5")),
            ("OpacityBoolConverter", "True", Some("True"), Some("0.5")),
            ("NullToVisibilityConverter", "", none, Some("Collapsed")),
            ("NullToVisibilityConverter", "cover.png", none, Some("Visible")),
            ("StringNullOrEmptyToVisibilityConverter", "  ", none, Some("Collapsed")),
            ("StringNullOrEmptyToVisibilityConverter", "Hi", none, Some("Visible")),
            ("StringNullOrEmptyToBoolConverter", "", none, Some("True")),
            ("ICollectionNullOrEmptyToVisibilityConverter", "", none, Some("Collapsed")),
            ("ICollectionNullOrEmptyToVisibilityConverter", "RPG\nAction", none, Some("Visible")),
            ("ObjectEqualityToVisibilityConverter", "Steam", Some("Steam"), Some("Visible")),
            ("ObjectEqualityToVisibilityConverter", "GOG", Some("Steam"), Some("Collapsed")),
            ("InvertedObjectEqualityToVisibilityConverter", "GOG", Some("Steam"), Some("Visible")),
            ("ObjectEqualityToBoolConverter", "3", Some("3.0"), Some("True")),
            ("IntToVisibilityConverter", "2", Some("2"), Some("Visible")),
            ("IntToVisibilityConverter", "1", Some("2"), Some("Collapsed")),
            ("InvertedIntToVisibilityConverter", "1", Some("2"), Some("Visible")),
            ("ObjectToStringConverter", "Played", none, Some("Played")),
            ("ImageStringToImageConverterCached", "a.png", none, Some("a.png")),
            ("StringToUpperCaseConverter", "Time played", none, Some("TIME PLAYED")),
            ("ListToStringConverter", "RPG\nAction", none, Some("RPG,Action")),
            ("ListToStringConverter", "RPG\nAction", Some(", "), Some("RPG, Action")),
            ("ListToStringConverter", "RPG", Some(" • "), Some("RPG")),
            ("ListToStringConverter", "", Some(", "), Some("")),
            ("NiceListToStringConverter", "A\nB", none, Some("A, B")),
            ("PlayTimeToStringConverter", "0", none, Some("Not played")),
            ("PlayTimeToStringConverter", "", none, Some("Not played")),
            ("PlayTimeToStringConverter", "45", none, Some("45 seconds")),
            ("PlayTimeToStringConverter", "720", none, Some("12 minutes")),
            ("PlayTimeToStringConverter", "12000", none, Some("3h 20m")),
            ("PlayTimeToStringConverter", "184800", none, Some("51h 20m")),
            ("PlayTimeToStringConverter", "184800", Some("True"), Some("2d 3h 20m")),
            ("PlayTimeToStringConverter", "-5", none, None),
            ("DateTimeToLastPlayedConverter", "", none, Some("Never")),
            ("NullableDateToStringConverter", "2025-03-12", none, Some("12 Mar 2025")),
            ("NullableDateToStringConverter", "1741737600", none, Some("12 Mar 2025")),
            ("NullableDateToStringConverter", "", none, Some("")),
            ("ReleaseDateToStringConverter", "2025-03-12", none, Some("12 Mar 2025")),
            ("ReleaseDateToStringConverter", "2025-03", none, Some("Mar 2025")),
            ("ReleaseDateToStringConverter", "2025", none, Some("2025")),
            ("NullableUlongBytesSizeToStringConverter", "512", none, Some("512 B")),
            ("NullableUlongBytesSizeToStringConverter", "1536", none, Some("1.50 KB")),
            ("NullableUlongBytesSizeToStringConverter", "4697620480", none, Some("4.375 GB")),
            ("NullableUlongBytesSizeToStringConverter", "1073741824", none, Some("1.00 GB")),
            ("NullableUlongBytesSizeToStringConverter", "", none, Some("")),
            ("MathConverter", "3", Some("x * -310"), Some("-930")),
            ("SomethingElseConverter", "x", none, None),
        ];
        for (name, input, param, want) in rows {
            assert_eq!(apply(name, input, *param).as_deref(), *want, "{name}({input:?}, {param:?})");
        }
    }

    #[test]
    fn last_played_dates_read_like_lodestar() {
        let t = crate::model::now_unix();
        assert_eq!(apply("DateTimeToLastPlayedConverter", &t.to_string(), None).as_deref(), Some("Today"));
    }

    #[test]
    fn math_table() {
        let rows: &[(&str, &[&str], Option<&str>)] = &[
            ("x * 100 / 320", &["640"], Some("200")),
            ("x * 96 / 320", &["320"], Some("96")),
            ("x < 0 ? 0 : x * -310", &["-1"], Some("0")),
            ("x &lt; 0 ? 0 : x * -310", &["2"], Some("-620")),
            ("x > 1", &["0"], Some("False")),
            ("x &gt;= 560", &["600"], Some("True")),
            ("x &lt; 120 ? `Visible` : `Collapsed`", &["80"], Some("Visible")),
            ("x < y + 20 + 400", &["500", "100"], Some("True")),
            ("y == true ? Max(0, (x - 24) / 2) : x", &["424", "True"], Some("200")),
            ("y == true ? Max(0, (x - 24) / 2) : x", &["424", "False"], Some("424")),
            ("z == true ? x : Max(0, x - y - 32)", &["800", "300", "False"], Some("468")),
            ("0, 0, [0], [1]", &["8", "4"], Some("0,0,8,4")),
            ("[1] &lt; 0 ? 1 : 1 - 0.05 * Min(Abs([0] - [1]), 4)", &["5", "3"], Some("0.9")),
            ("[1] < 0 ? 1 : 1 - 0.05 * Min(Abs([0] - [1]), 4)", &["5", "-1"], Some("1")),
            ("700 * [2] / ([1] * ([0] + 0.5))", &["1.5", "2", "4"], Some("700")),
            ("-x", &["2.5"], Some("-2.5")),
            ("x / 3", &["1"], Some("0.333333333333")),
            ("Round(x)", &["2.5"], Some("2")),
            ("Round(x, 1)", &["2.25"], Some("2.2")),
            ("x % 7 + Floor(3.7) - Ceiling(0.2)", &["9"], Some("4")),
            ("!(x > 2) || y", &["5", "false"], Some("False")),
            ("x + `px`", &["5"], Some("5px")),
            // Missing values, broken expressions: nothing.
            ("x + y", &["1"], None),
            ("x +", &["1"], None),
            ("Frobnicate(x)", &["1"], None),
            ("(x", &["1"], None),
        ];
        for (expr, vals, want) in rows {
            assert_eq!(math(expr, vals).as_deref(), *want, "{expr} with {vals:?}");
        }
    }

    #[test]
    fn dates_round_trip() {
        for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2025, 3, 12), (2026, 11, 23), (1999, 12, 31)] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn names_normalise() {
        assert_eq!(normalise("BooleanToVisibilityConverter"), "booleantovisibility");
        assert_eq!(normalise("{StaticResource ImageStringToImageConverterCached}"), "imagestringtoimagecached");
        assert_eq!(normalise("pcon:NegateConverter"), "negate");
        assert_eq!(normalise("MathConverter"), "math");
    }
}
