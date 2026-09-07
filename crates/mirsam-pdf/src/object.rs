//! PDF's object types: what everything in the file is made of.
//!
//! ISO 32000-1 §7.3 gives eight basic types and two composites. This module is
//! the model alone — [`crate::lexer`] builds it out of bytes, [`crate::document`]
//! resolves the references between the pieces.
//!
//! ## A name is bytes, not a string
//!
//! §7.3.5 defines a name as a sequence of bytes with `#xx` escapes, and says
//! nothing about their encoding. A well-behaved writer emits ASCII, but a name
//! is also the one place a PDF stores a *key*, so decoding one lossily would
//! silently merge two dictionary entries that the file kept apart. [`Name`]
//! therefore holds the decoded bytes and compares on them; [`Name::as_str`]
//! offers the text form to whoever wants it and answers `None` rather than
//! guessing.

use std::fmt;

/// A PDF name: `/Type`, `/FlateDecode`, `/Root`.
///
/// Holds the *decoded* bytes, so `/A#42` and `/AB` are one name, which is what
/// §7.3.5 requires and what a dictionary lookup depends on.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Name(Vec<u8>);

impl Name {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The name as text, or `None` when it is not UTF-8.
    ///
    /// Never lossy: a caller comparing names compares bytes, and one printing
    /// a name gets [`fmt::Display`], which escapes rather than replaces.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// Whether this is the name `other` spells.
    pub fn is(&self, other: &str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl From<&str> for Name {
    fn from(s: &str) -> Self {
        Self(s.as_bytes().to_vec())
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.is(other)
    }
}

/// Written the way the file writes it, with the escapes a PDF requires — so a
/// name in a diagnostic can be searched for in the document it came from.
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("/")?;
        for &b in &self.0 {
            match b {
                b'!'..=b'~' if !is_delimiter(b) && b != b'#' => {
                    f.write_str(std::str::from_utf8(&[b]).unwrap_or("?"))?
                }
                _ => write!(f, "#{b:02X}")?,
            }
        }
        Ok(())
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

/// The characters §7.2.2 reserves as delimiters. Everything else outside
/// whitespace is a regular character, and may appear in a name unescaped.
pub(crate) fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// The six bytes §7.2.2 calls white space. The null is one of them, which is
/// why a byte-level scanner cannot use `u8::is_ascii_whitespace` here.
pub(crate) fn is_whitespace(b: u8) -> bool {
    matches!(b, 0 | b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}

/// Neither white space nor a delimiter: the bytes a name, a number or a
/// keyword is made of.
pub(crate) fn is_regular(b: u8) -> bool {
    !is_whitespace(b) && !is_delimiter(b)
}

/// The address of an indirect object: `12 0 R`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct ObjectId {
    pub number: u32,
    pub generation: u16,
}

impl ObjectId {
    pub fn new(number: u32, generation: u16) -> Self {
        Self { number, generation }
    }
}

/// Printed as the file writes a reference to it, so an `unread` entry names an
/// object in the words a person would use to find it in the file.
impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} R", self.number, self.generation)
    }
}

/// A dictionary: an ordered list of name/value pairs.
///
/// A `Vec` rather than a map, for two reasons. A PDF dictionary is small —
/// tens of entries at the outside — so a linear scan beats hashing, and file
/// order is preserved, which keeps a dictionary printed back in a diagnostic
/// looking like the one in the document.
///
/// **A repeated key keeps the last value.** §7.3.7 leaves the case undefined.
/// Last wins because the only way a duplicate arises in practice is a writer
/// appending an entry it meant to supersede an earlier one.
#[derive(Clone, PartialEq, Default)]
pub struct Dictionary(Vec<(Name, Object)>);

impl Dictionary {
    pub fn new() -> Self {
        Self::default()
    }

    /// A dictionary with no entries, in a `const` context — what a document
    /// hands back for a dictionary it does not have, so no caller has to
    /// distinguish "no `/Resources`" from "an empty one".
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    /// The value stored under `key`, unresolved: a `/Length` that the file
    /// wrote as `12 0 R` comes back as [`Object::Reference`]. Use
    /// [`crate::Pdf::get`] to follow it.
    pub fn get(&self, key: &str) -> Option<&Object> {
        self.0
            .iter()
            .find(|(name, _)| name.is(key))
            .map(|(_, value)| value)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn insert(&mut self, name: Name, value: Object) {
        match self.0.iter_mut().find(|(existing, _)| *existing == name) {
            Some(slot) => slot.1 = value,
            None => self.0.push((name, value)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Name, &Object)> {
        self.0.iter().map(|(name, value)| (name, value))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<(Name, Object)> for Dictionary {
    fn from_iter<I: IntoIterator<Item = (Name, Object)>>(iter: I) -> Self {
        let mut dict = Self::new();
        for (name, value) in iter {
            dict.insert(name, value);
        }
        dict
    }
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<<")?;
        for (name, value) in &self.0 {
            write!(f, " {name} {value:?}")?;
        }
        f.write_str(" >>")
    }
}

/// A stream: a dictionary and the bytes after its `stream` keyword.
///
/// **`data` is what the file holds, still filtered.** Decoding is
/// [`crate::Pdf::decoded`]'s, and it is deliberately not done here: a stream
/// whose filter this crate does not implement has to be *named* rather than
/// failed, and only the document knows how to record that.
#[derive(Clone, PartialEq)]
pub struct Stream {
    pub dict: Dictionary,
    pub data: Vec<u8>,
}

impl fmt::Debug for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} stream({} bytes)", self.dict, self.data.len())
    }
}

/// Any PDF object.
#[derive(Clone, PartialEq)]
pub enum Object {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    /// A literal or hexadecimal string, decoded to the bytes it denotes.
    /// Still bytes: which of PDFDocEncoding, UTF-16BE or a font's own codes
    /// they are depends on where the string was found.
    String(Vec<u8>),
    Name(Name),
    Array(Vec<Object>),
    Dictionary(Dictionary),
    Stream(Stream),
    Reference(ObjectId),
}

impl Object {
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// An integer, or a real that names one exactly. A PDF writer is free to
    /// spell a count `3.0`, and a reader that refused it would reject the file
    /// over its punctuation.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(n) => Some(*n),
            Self::Real(r) if r.fract() == 0.0 && r.is_finite() => Some(*r as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Integer(n) => Some(*n as f64),
            Self::Real(r) => Some(*r),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            Self::String(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&Name> {
        match self {
            Self::Name(name) => Some(name),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The dictionary of a dictionary *or* of a stream.
    ///
    /// A stream is a dictionary with bytes attached, and every caller that
    /// wants `/Type` or `/Filter` wants the same answer from both.
    pub fn as_dict(&self) -> Option<&Dictionary> {
        match self {
            Self::Dictionary(dict) => Some(dict),
            Self::Stream(stream) => Some(&stream.dict),
            _ => None,
        }
    }

    pub fn as_stream(&self) -> Option<&Stream> {
        match self {
            Self::Stream(stream) => Some(stream),
            _ => None,
        }
    }

    pub fn as_reference(&self) -> Option<ObjectId> {
        match self {
            Self::Reference(id) => Some(*id),
            _ => None,
        }
    }

    /// Whether this object's `/Type` is `name`.
    ///
    /// Answers `false` for a dictionary with no `/Type` at all: several are
    /// optional in PDF, so absence is not disagreement, and a caller that
    /// wants to infer a type from an object's shape has to say so.
    pub fn is_type(&self, name: &str) -> bool {
        self.as_dict()
            .and_then(|dict| dict.get("Type"))
            .and_then(Object::as_name)
            .is_some_and(|found| found.is(name))
    }
}

impl fmt::Debug for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Boolean(b) => write!(f, "{b}"),
            Self::Integer(n) => write!(f, "{n}"),
            Self::Real(r) => write!(f, "{r}"),
            Self::String(bytes) => write!(f, "({} bytes)", bytes.len()),
            Self::Name(name) => write!(f, "{name}"),
            Self::Array(items) => {
                f.write_str("[")?;
                for item in items {
                    write!(f, " {item:?}")?;
                }
                f.write_str(" ]")
            }
            Self::Dictionary(dict) => write!(f, "{dict:?}"),
            Self::Stream(stream) => write!(f, "{stream:?}"),
            Self::Reference(id) => write!(f, "{id}"),
        }
    }
}
