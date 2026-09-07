//! Bytes to objects: the syntax half of ISO 32000-1 §7.2 and §7.3.
//!
//! A borrowing scanner over the whole file, so an object can be read at an
//! offset the cross-reference table names without copying anything before it.
//!
//! ## A stream's length is checked, not believed
//!
//! `/Length` may be an indirect reference — `<< /Length 12 0 R >>` — which is
//! a value the lexer cannot resolve, because resolving it means consulting the
//! cross-reference table that is itself being parsed out of streams. Worse,
//! plenty of real writers get the direct form *wrong* by a byte or two.
//!
//! So the length is treated as a hint and verified: if `/Length` is a direct
//! integer and the bytes it names are followed by `endstream`, it is taken. In
//! every other case — indirect, absent, or simply wrong — the data runs to the
//! next `endstream`. One rule covers all three, and no caller has to hand the
//! lexer a resolver it cannot have yet.

use mirsam_core::error::{Error, Result};

use crate::object::{Dictionary, Name, Object, ObjectId, Stream, is_regular, is_whitespace};

/// How deep a nesting of arrays and dictionaries is read before the file is
/// called malformed. Well past anything a document contains, and short of the
/// depth at which a hand-written adversarial file would exhaust the stack.
const MAX_DEPTH: usize = 128;

pub struct Lexer<'a> {
    data: &'a [u8],
    pos: usize,
}

/// A parse failure, always carrying the byte offset it happened at: an object
/// layer that reported "malformed dictionary" without saying where would be
/// unusable against a document nobody can open in an editor.
fn malformed(at: usize, what: impl std::fmt::Display) -> Error {
    Error::Format(format!("at byte {at}: {what}"))
}

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// A lexer positioned at `pos`, which is what a cross-reference entry
    /// hands over.
    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.data.len());
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.data.get(self.pos + ahead).copied()
    }

    /// White space and comments, which §7.2.4 says are equivalent to a single
    /// space anywhere a token boundary is allowed.
    pub fn skip_space(&mut self) {
        loop {
            match self.peek() {
                Some(b) if is_whitespace(b) => self.pos += 1,
                Some(b'%') => {
                    while let Some(b) = self.peek() {
                        if b == b'\r' || b == b'\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                _ => return,
            }
        }
    }

    /// Consume `word` if it is the next token, and answer whether it was.
    ///
    /// The token has to *end* where the keyword does: without that check,
    /// `endstream` would match a lexer looking for `end`, and `obj` would
    /// match inside `objstm`.
    pub fn keyword(&mut self, word: &str) -> bool {
        self.skip_space();
        let end = self.pos + word.len();
        if self.data.get(self.pos..end) != Some(word.as_bytes()) {
            return false;
        }
        if self.data.get(end).copied().is_some_and(is_regular) {
            return false;
        }
        self.pos = end;
        true
    }

    /// The run of regular characters at the cursor: a keyword, a number, or
    /// whatever a malformed file has there instead.
    fn token(&mut self) -> &'a [u8] {
        self.skip_space();
        let start = self.pos;
        while self.peek().is_some_and(is_regular) {
            self.pos += 1;
        }
        &self.data[start..self.pos]
    }

    /// One object, following a reference no further than recognising it.
    pub fn object(&mut self) -> Result<Object> {
        self.object_at_depth(0)
    }

    fn object_at_depth(&mut self, depth: usize) -> Result<Object> {
        if depth > MAX_DEPTH {
            return Err(malformed(self.pos, "objects nested past any real document"));
        }
        self.skip_space();
        let at = self.pos;
        match self.peek() {
            None => Err(malformed(at, "expected an object, found end of file")),
            Some(b'/') => Ok(Object::Name(self.name()?)),
            Some(b'(') => Ok(Object::String(self.literal_string()?)),
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_space();
                    match self.peek() {
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(Object::Array(items));
                        }
                        None => return Err(malformed(at, "array with no closing `]`")),
                        _ => items.push(self.object_at_depth(depth + 1)?),
                    }
                }
            }
            Some(b'<') if self.peek_at(1) == Some(b'<') => self.dictionary_or_stream(depth),
            Some(b'<') => Ok(Object::String(self.hex_string()?)),
            Some(b) if b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.') => self.numeric(),
            Some(stray @ (b']' | b'>' | b')' | b'}')) => {
                Err(malformed(at, format!("stray `{}`", stray as char)))
            }
            Some(_) => match self.token() {
                b"true" => Ok(Object::Boolean(true)),
                b"false" => Ok(Object::Boolean(false)),
                b"null" => Ok(Object::Null),
                [] => Err(malformed(at, "expected an object, found a delimiter")),
                other => Err(malformed(
                    at,
                    format!("unknown keyword {:?}", String::from_utf8_lossy(other)),
                )),
            },
        }
    }

    /// A number, or the `n g R` reference two of them may begin.
    ///
    /// Reference recognition is lookahead with a rewind rather than a
    /// post-pass over a token list: `1 0 R` and `1 0` differ only in what
    /// follows, and every context that admits one admits the other.
    fn numeric(&mut self) -> Result<Object> {
        let at = self.pos;
        let first = number(self.token(), at)?;

        if let Object::Integer(number) = first
            && let Ok(number) = u32::try_from(number)
        {
            let rewind = self.pos;
            let generation = self.token();
            if !generation.is_empty()
                && generation.iter().all(u8::is_ascii_digit)
                && let Ok(generation) = String::from_utf8_lossy(generation).parse::<u16>()
                && self.keyword("R")
            {
                return Ok(Object::Reference(ObjectId::new(number, generation)));
            }
            self.pos = rewind;
        }
        Ok(first)
    }

    fn name(&mut self) -> Result<Name> {
        let at = self.pos;
        self.pos += 1; // the `/`
        let mut bytes = Vec::new();
        while let Some(b) = self.peek() {
            if !is_regular(b) {
                break;
            }
            self.pos += 1;
            if b != b'#' {
                bytes.push(b);
                continue;
            }
            // `#xx`. A `#` not followed by two hex digits is kept literally:
            // the alternative is failing a whole document over one byte a
            // writer forgot to escape, and every reader in the world opens it.
            match (self.peek(), self.peek_at(1)) {
                (Some(hi), Some(lo)) if hi.is_ascii_hexdigit() && lo.is_ascii_hexdigit() => {
                    bytes.push(hex(hi) * 16 + hex(lo));
                    self.pos += 2;
                }
                _ => bytes.push(b'#'),
            }
        }
        if bytes.is_empty() && self.pos == at + 1 {
            // `/` alone is the empty name, which is legal.
        }
        Ok(Name::new(bytes))
    }

    /// `( ... )`, with §7.3.4.2's escapes and balanced inner parentheses.
    fn literal_string(&mut self) -> Result<Vec<u8>> {
        let at = self.pos;
        self.pos += 1; // the `(`
        let mut bytes = Vec::new();
        let mut depth = 1usize;
        while let Some(b) = self.peek() {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    bytes.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(bytes);
                    }
                    bytes.push(b);
                }
                b'\\' => {
                    let Some(escape) = self.peek() else { break };
                    self.pos += 1;
                    match escape {
                        b'n' => bytes.push(b'\n'),
                        b'r' => bytes.push(b'\r'),
                        b't' => bytes.push(b'\t'),
                        b'b' => bytes.push(0x08),
                        b'f' => bytes.push(0x0c),
                        b'(' | b')' | b'\\' => bytes.push(escape),
                        // A backslash before an end-of-line joins the lines and
                        // contributes nothing.
                        b'\n' => {}
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'0'..=b'7' => {
                            let mut value = u32::from(escape - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            // §7.3.4.2: high-order overflow is ignored.
                            bytes.push((value & 0xff) as u8);
                        }
                        // "\q" is q. The backslash goes, the byte stays.
                        other => bytes.push(other),
                    }
                }
                _ => bytes.push(b),
            }
        }
        Err(malformed(at, "string with no closing `)`"))
    }

    /// `<...>`, hexadecimal. White space between digits is ignored and an odd
    /// final digit is padded with a zero, both per §7.3.4.3.
    fn hex_string(&mut self) -> Result<Vec<u8>> {
        let at = self.pos;
        self.pos += 1; // the `<`
        let mut bytes = Vec::new();
        let mut half: Option<u8> = None;
        while let Some(b) = self.peek() {
            self.pos += 1;
            match b {
                b'>' => {
                    if let Some(hi) = half {
                        bytes.push(hi * 16);
                    }
                    return Ok(bytes);
                }
                b if b.is_ascii_hexdigit() => match half.take() {
                    Some(hi) => bytes.push(hi * 16 + hex(b)),
                    None => half = Some(hex(b)),
                },
                b if is_whitespace(b) => {}
                other => {
                    return Err(malformed(
                        self.pos - 1,
                        format!("`{}` in a hexadecimal string", other as char),
                    ));
                }
            }
        }
        Err(malformed(at, "hexadecimal string with no closing `>`"))
    }

    fn dictionary_or_stream(&mut self, depth: usize) -> Result<Object> {
        let at = self.pos;
        self.pos += 2; // the `<<`
        let mut dict = Dictionary::new();
        loop {
            self.skip_space();
            match self.peek() {
                Some(b'>') if self.peek_at(1) == Some(b'>') => {
                    self.pos += 2;
                    break;
                }
                Some(b'/') => {
                    let key = self.name()?;
                    let value = self.object_at_depth(depth + 1)?;
                    dict.insert(key, value);
                }
                None => return Err(malformed(at, "dictionary with no closing `>>`")),
                Some(other) => {
                    return Err(malformed(
                        self.pos,
                        format!("expected a key, found `{}`", other as char),
                    ));
                }
            }
        }

        let rewind = self.pos;
        if !self.keyword("stream") {
            self.pos = rewind;
            return Ok(Object::Dictionary(dict));
        }
        // §7.3.8.1: CRLF or LF after the keyword, never CR alone. A file that
        // uses CR alone is still read: dropping it costs nothing, and treating
        // it as data would shift every byte of the stream.
        match (self.peek(), self.peek_at(1)) {
            (Some(b'\r'), Some(b'\n')) => self.pos += 2,
            (Some(b'\n'), _) | (Some(b'\r'), _) => self.pos += 1,
            _ => {}
        }
        let data = self.stream_data(&dict)?;
        Ok(Object::Stream(Stream { dict, data }))
    }

    /// The bytes between `stream` and `endstream`, leaving the cursor past the
    /// closing keyword. See the module note on why `/Length` is verified.
    fn stream_data(&mut self, dict: &Dictionary) -> Result<Vec<u8>> {
        let start = self.pos;

        if let Some(length) = dict.get("Length").and_then(Object::as_i64)
            && let Ok(length) = usize::try_from(length)
            && let Some(end) = start.checked_add(length)
            && end <= self.data.len()
        {
            let mut probe = Lexer::at(self.data, end);
            if probe.keyword("endstream") {
                self.pos = probe.pos;
                return Ok(self.data[start..end].to_vec());
            }
        }

        let Some(found) = find(&self.data[start..], b"endstream") else {
            return Err(malformed(start, "stream with no `endstream`"));
        };
        let mut end = start + found;
        self.pos = end + b"endstream".len();
        // The end-of-line before `endstream` belongs to the syntax, not to the
        // data, and only that one: a stream may legitimately end in a newline.
        if end > start && self.data[end - 1] == b'\n' {
            end -= 1;
        }
        if end > start && self.data[end - 1] == b'\r' {
            end -= 1;
        }
        Ok(self.data[start..end].to_vec())
    }

    /// `n g obj <object> endobj`, at the cursor.
    ///
    /// `endobj` is not required: a writer that omits it produces a file every
    /// reader opens, and the object is complete without it.
    pub fn indirect(&mut self) -> Result<(ObjectId, Object)> {
        self.skip_space();
        let at = self.pos;
        let number = self.token();
        let generation = self.token();
        let header = (
            String::from_utf8_lossy(number).parse::<u32>(),
            String::from_utf8_lossy(generation).parse::<u16>(),
        );
        let (Ok(number), Ok(generation)) = header else {
            return Err(malformed(at, "expected `<n> <g> obj`"));
        };
        if !self.keyword("obj") {
            return Err(malformed(at, "expected `obj` after the object number"));
        }
        let object = self.object()?;
        let rewind = self.pos;
        if !self.keyword("endobj") {
            self.pos = rewind;
        }
        Ok((ObjectId::new(number, generation), object))
    }
}

fn hex(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        _ => b - b'A' + 10,
    }
}

/// A number token, which §7.3.3 spells more loosely than it looks: `4.`, `-.2`
/// and `34.5` are all legal, and writers emit `--5` and `6.-2` often enough
/// that every reader tolerates them.
fn number(token: &[u8], at: usize) -> Result<Object> {
    if token.is_empty() {
        return Err(malformed(at, "expected a number"));
    }
    let text = String::from_utf8_lossy(token);
    if !token.contains(&b'.')
        && let Ok(value) = text.parse::<i64>()
    {
        return Ok(Object::Integer(value));
    }
    if let Ok(value) = text.parse::<f64>() {
        return Ok(Object::Real(value));
    }
    // Salvage the leading well-formed number, which is what the malformed
    // spellings above amount to. `--5` is 0 with a stray sign, not a failure
    // to read the file.
    let mut cleaned = String::new();
    for (i, c) in text.chars().enumerate() {
        match c {
            '+' | '-' if i == 0 => cleaned.push(c),
            '.' if !cleaned.contains('.') => cleaned.push(c),
            '0'..='9' => cleaned.push(c),
            _ => break,
        }
    }
    match cleaned.trim_start_matches(['+', '-']) {
        "" | "." => Ok(Object::Integer(0)),
        _ => cleaned
            .parse::<f64>()
            .map(Object::Real)
            .map_err(|_| malformed(at, format!("unreadable number {text:?}"))),
    }
}

/// The first offset of `needle` in `haystack`.
///
/// A plain scan: the needles here are short keywords and the streams they are
/// looked for in are the exception rather than the rule, so a search structure
/// would cost more to build than it saves.
pub(crate) fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The last offset of `needle` in `haystack`.
pub(crate) fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}
