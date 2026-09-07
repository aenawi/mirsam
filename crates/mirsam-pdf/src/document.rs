//! The document: the object graph, resolved, and the page tree over it.
//!
//! This is the package layer of a format that is not a package. `Package` in
//! `mirsam-ooxml` answers "give me the part named `ppt/slides/slide1.xml`";
//! [`Pdf`] answers "give me object `12 0 R`, wherever the file put it" — in the
//! body, inside a compressed object stream, or in a position only a scan
//! could find.
//!
//! ## Everything is loaded at open, and the file bytes are then dropped
//!
//! An object stream has to be decompressed before the objects inside it exist
//! at all, and a page's `/Resources` may be inherited from a node three levels
//! up the tree. Lazy loading would mean either interior mutability threaded
//! through every accessor or a resolver argument on every call, and it would
//! buy nothing: this tool reads the whole document.
//!
//! Streams are kept *filtered*, so what is held is about the size of the file,
//! and the file's own bytes are released once the graph is built.
//!
//! ## What could not be read is answered for
//!
//! A page whose content is a JPEG, an object at an offset that is not there, an
//! object stream behind a filter this crate does not implement — each is
//! recorded and named by [`Pdf::unread`]. ADR 0009 is the reason: a source the
//! adapter could not read is part of the report, and a document that produced
//! no text must never be indistinguishable from one with no defects.

use mirsam_core::error::{Error, Result};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use crate::filter::{self, Decoded};
use crate::lexer::{Lexer, find};
use crate::object::{Dictionary, Name, Object, ObjectId, Stream};
use crate::xref::{self, Slot, Xref};
use crate::{encrypt, page};

/// How far into the file `%PDF-` is looked for. §7.5.2 puts it at byte zero;
/// a kilobyte of tolerance covers the shell scripts and mail gateways that put
/// something in front of it, and stops well short of finding the string inside
/// an embedded document.
const HEADER_WINDOW: usize = 1024;

/// How many references are followed before a chain is called circular.
/// `12 0 R` pointing at `13 0 R` pointing back is a file, not a document.
const MAX_INDIRECTION: usize = 32;

/// Something the file did not state usably and the loader had to work out.
///
/// Not a defect and not a finding: a fact about how the document was read,
/// which a caller may want to say out loud and which the tests assert on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reconstruction {
    /// The cross-reference chain was unusable, so the body was scanned for
    /// `<n> <g> obj`.
    CrossReferenceTable,
    /// `/Root /Pages` did not yield a walkable tree, so the pages were taken
    /// from the objects that call themselves `/Type /Page`.
    PageTree,
}

impl fmt::Display for Reconstruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CrossReferenceTable => f.write_str("the cross-reference table"),
            Self::PageTree => f.write_str("the page tree"),
        }
    }
}

/// Something in the document this crate did not read, and why.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Unread {
    /// The object it was, where there is one to name.
    pub object: Option<ObjectId>,
    pub reason: String,
}

/// Named as the document names it, so a reader can find the thing being talked
/// about: `12 0 R — /DCTDecode, an image codec this tool does not decode`.
impl fmt::Display for Unread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.object {
            Some(id) => write!(f, "{id} — {}", self.reason),
            None => f.write_str(&self.reason),
        }
    }
}

/// One entry of the object graph.
struct Entry {
    generation: u16,
    object: Object,
}

/// A PDF, read.
pub struct Pdf {
    name: String,
    version: String,
    objects: BTreeMap<u32, Entry>,
    trailer: Dictionary,
    catalog: ObjectId,
    pages: Vec<page::Page>,
    reconstructed: Vec<Reconstruction>,
    /// Grown while reading, because a stream's filter is only discovered when
    /// somebody asks for its bytes. A `RefCell` rather than `&mut self` on
    /// every accessor: recording what was not read is bookkeeping, and making
    /// it change the shape of the read API would put it in every caller.
    unread: RefCell<Vec<Unread>>,
}

/// A summary rather than the graph: printing every object of a document is
/// not a thing a test failure or a `dbg!` wants, and the pieces are all
/// reachable through the accessors.
impl fmt::Debug for Pdf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pdf")
            .field("name", &self.name)
            .field("version", &self.version)
            .field("objects", &self.objects.len())
            .field("pages", &self.pages.len())
            .field("reconstructed", &self.reconstructed)
            .field("unread", &self.unread.borrow().len())
            .finish()
    }
}

impl Pdf {
    /// Open and read a PDF.
    pub fn open(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(Error::NotFound);
        }
        let data = std::fs::read(path)?;
        Self::from_bytes(&data, &path.display().to_string())
    }

    /// Read a PDF already in memory. `name` is used only in messages.
    pub fn from_bytes(data: &[u8], name: &str) -> Result<Self> {
        let (shift, version) = header(data)?;

        let mut reconstructed = Vec::new();
        let xref = match xref::read(data, shift) {
            Ok(xref) if !xref.slots.is_empty() => xref,
            _ => {
                reconstructed.push(Reconstruction::CrossReferenceTable);
                xref::recover(data)
            }
        };

        refuse_encryption(data, &xref, shift)?;

        let mut unread = Vec::new();
        let mut objects = load(data, &xref, shift, &mut unread);

        // An offset that pointed at nothing is worth one scan of the body
        // before it is reported: a file with a stale table is a file every
        // viewer opens, and the objects are all still there.
        let missing = xref
            .slots
            .iter()
            .filter(|(number, slot)| {
                matches!(slot, Slot::InFile { .. }) && !objects.contains_key(number)
            })
            .count();
        if missing > 0 {
            let rebuilt = xref::recover(data);
            let mut recovered = load(data, &rebuilt, shift, &mut Vec::new());
            let mut gained = false;
            for (number, entry) in std::mem::take(&mut recovered) {
                if let std::collections::btree_map::Entry::Vacant(slot) = objects.entry(number) {
                    slot.insert(entry);
                    gained = true;
                }
            }
            if gained {
                reconstructed.push(Reconstruction::CrossReferenceTable);
                unread.retain(|entry| {
                    entry
                        .object
                        .is_none_or(|id| !objects.contains_key(&id.number))
                });
            }
        }

        if objects.is_empty() {
            return Err(Error::Format(format!(
                "{name}: no readable objects; the file states a cross-reference table and has \
                 nothing where it points"
            )));
        }

        let mut pdf = Self {
            name: name.to_string(),
            version,
            objects,
            trailer: xref.trailer.clone(),
            catalog: ObjectId::default(),
            pages: Vec::new(),
            reconstructed,
            unread: RefCell::new(unread),
        };
        pdf.expand_object_streams(&xref);
        pdf.catalog = pdf.find_catalog()?;
        let (pages, rebuilt) = page::tree(&pdf);
        pdf.pages = pages;
        if rebuilt {
            pdf.reconstructed.push(Reconstruction::PageTree);
        }
        pdf.reconstructed.sort_unstable();
        pdf.reconstructed.dedup();
        Ok(pdf)
    }

    /// The path or label this document was opened under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The version in the `%PDF-` header: `1.7`, `2.0`.
    ///
    /// The header alone. A catalog may override it with `/Version`, and
    /// [`Pdf::catalog`] is where a caller asks for that — reporting one number
    /// for two different claims would hide a disagreement between them.
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn trailer(&self) -> &Dictionary {
        &self.trailer
    }

    /// The document catalog, `/Root`.
    pub fn catalog(&self) -> &Dictionary {
        self.object(self.catalog)
            .and_then(Object::as_dict)
            .unwrap_or(&EMPTY)
    }

    /// The pages, in the order the page tree gives them.
    pub fn pages(&self) -> &[page::Page] {
        &self.pages
    }

    /// How many objects were loaded.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// What the loader had to work out because the file did not state it
    /// usably. Empty for a well-formed document.
    pub fn reconstructed(&self) -> &[Reconstruction] {
        &self.reconstructed
    }

    /// What this crate did not read, in object order, each named as the
    /// document names it.
    ///
    /// This is what a `DocumentReader` over PDF answers `unread_sources` with.
    pub fn unread(&self) -> Vec<String> {
        let mut entries = self.unread.borrow().clone();
        entries.sort();
        entries.dedup();
        entries.iter().map(ToString::to_string).collect()
    }

    /// Record something that was not read. Public because the text layer above
    /// this one has its own reasons — a font with no `ToUnicode` map is a
    /// source it could not read, and the list has to be one list.
    pub fn note_unread(&self, object: Option<ObjectId>, reason: impl Into<String>) {
        self.unread.borrow_mut().push(Unread {
            object,
            reason: reason.into(),
        });
    }

    /// The object at `id`, or `Null` if the document has none.
    ///
    /// **Matched on the object number alone.** A reference carries a
    /// generation, and matching on it would be more faithful to §7.3.10 — but
    /// a generation that disagrees with the table is something writers produce
    /// and every reader ignores, and refusing to follow such a reference would
    /// lose an object the file plainly holds.
    pub fn object(&self, id: ObjectId) -> Option<&Object> {
        self.objects.get(&id.number).map(|entry| &entry.object)
    }

    /// The generation the file stored `number` under.
    pub fn generation(&self, number: u32) -> Option<u16> {
        self.objects.get(&number).map(|entry| entry.generation)
    }

    /// Every object, in object-number order — which is the order the file
    /// wrote them, and the only order available to a document whose page tree
    /// has to be reconstructed.
    pub fn objects(&self) -> impl Iterator<Item = (ObjectId, &Object)> {
        self.objects
            .iter()
            .map(|(number, entry)| (ObjectId::new(*number, entry.generation), &entry.object))
    }

    /// Follow references until something else is reached.
    pub fn resolve<'a>(&'a self, object: &'a Object) -> &'a Object {
        let mut current = object;
        for _ in 0..MAX_INDIRECTION {
            let Object::Reference(id) = current else {
                return current;
            };
            match self.object(*id) {
                Some(next) => current = next,
                None => return &Object::Null,
            }
        }
        // A cycle. `Null` is the honest answer: §7.3.9 already says an
        // undefined reference is null, and a document whose objects point at
        // each other has no value there to give.
        &Object::Null
    }

    /// A dictionary entry, resolved. Absent and explicitly null are one
    /// answer, as §7.3.7 says they are.
    pub fn get<'a>(&'a self, dict: &'a Dictionary, key: &str) -> Option<&'a Object> {
        match dict.get(key).map(|value| self.resolve(value)) {
            Some(Object::Null) | None => None,
            Some(value) => Some(value),
        }
    }

    /// The bytes of a stream, decoded.
    ///
    /// `Ok(None)` means the stream is behind a filter this crate does not
    /// decode — an image codec, all but always — and the fact has been
    /// recorded in [`Pdf::unread`]. It is not an error: a scanned page is a
    /// page whose text is not in the file, which is a true thing to report and
    /// not a failure to read one.
    pub fn decoded(&self, id: ObjectId) -> Result<Option<Vec<u8>>> {
        let Some(stream) = self.object(id).and_then(Object::as_stream) else {
            return Ok(None);
        };
        self.decode_stream(Some(id), stream)
    }

    fn decode_stream(&self, id: Option<ObjectId>, stream: &Stream) -> Result<Option<Vec<u8>>> {
        let filters = self.filters(&stream.dict);
        let parms = self.decode_parms(&stream.dict, filters.len());
        match filter::decode(&stream.data, &filters, &parms)? {
            Decoded::Bytes(bytes) => Ok(Some(bytes)),
            Decoded::Stopped { filter } => {
                self.note_unread(id, format!("{filter}, a filter this tool does not decode"));
                Ok(None)
            }
        }
    }

    /// `/Filter`, which is a name or an array of them, either of which may be
    /// written indirectly.
    fn filters(&self, dict: &Dictionary) -> Vec<Name> {
        let Some(filter) = self.get(dict, "Filter").or_else(|| self.get(dict, "F")) else {
            return Vec::new();
        };
        match filter {
            Object::Name(name) => vec![name.clone()],
            Object::Array(items) => items
                .iter()
                .filter_map(|item| self.resolve(item).as_name())
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `/DecodeParms`, padded to the length of the filter chain so index `i`
    /// belongs to filter `i` whatever shape the file wrote it in.
    fn decode_parms(&self, dict: &Dictionary, filters: usize) -> Vec<Dictionary> {
        let mut parms = match self
            .get(dict, "DecodeParms")
            .or_else(|| self.get(dict, "DP"))
        {
            Some(Object::Dictionary(one)) => vec![one.clone()],
            Some(Object::Array(items)) => items
                .iter()
                .map(|item| self.resolve(item).as_dict().cloned().unwrap_or_default())
                .collect(),
            _ => Vec::new(),
        };
        parms.resize(filters, Dictionary::new());
        parms
    }

    /// Decompress every object stream the table pointed into, and add the
    /// objects it holds.
    ///
    /// Objects already loaded from the body win: a compressed object is only
    /// reachable through the table that named it, so a body definition of the
    /// same number came from a section that superseded it.
    fn expand_object_streams(&mut self, xref: &Xref) {
        for (container, members) in xref.in_streams() {
            let id = ObjectId::new(container, self.generation(container).unwrap_or(0));
            let Some(stream) = self.object(id).and_then(Object::as_stream) else {
                self.note_unread(Some(id), "an object stream the file does not contain");
                continue;
            };
            if !stream
                .dict
                .get("Type")
                .is_none_or(|t| t.as_name().is_some_and(|name| name.is("ObjStm")))
            {
                self.note_unread(Some(id), "referenced as an object stream, and is not one");
                continue;
            }
            let count = self
                .get(&stream.dict, "N")
                .and_then(Object::as_i64)
                .unwrap_or(0);
            let first = self
                .get(&stream.dict, "First")
                .and_then(Object::as_i64)
                .unwrap_or(0);
            let bytes = match self.decode_stream(Some(id), stream) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => continue,
                Err(e) => {
                    self.note_unread(Some(id), format!("object stream: {e}"));
                    continue;
                }
            };

            let Ok(first) = usize::try_from(first) else {
                self.note_unread(Some(id), "object stream: /First is not an offset");
                continue;
            };
            // The header is `N` pairs of `<object number> <offset>`, each
            // offset relative to /First.
            let mut header = Lexer::new(&bytes);
            let mut offsets = Vec::new();
            for _ in 0..count.max(0) {
                let (Ok(Some(number)), Ok(Some(offset))) = (
                    header.object().map(|o| o.as_i64()),
                    header.object().map(|o| o.as_i64()),
                ) else {
                    break;
                };
                match (u32::try_from(number), usize::try_from(offset)) {
                    (Ok(number), Ok(offset)) => offsets.push((number, offset)),
                    _ => break,
                }
            }

            let wanted: BTreeSet<u32> = members.iter().map(|(number, _)| *number).collect();
            for (number, offset) in offsets {
                if !wanted.contains(&number) || self.objects.contains_key(&number) {
                    continue;
                }
                let Some(at) = first.checked_add(offset).filter(|at| *at < bytes.len()) else {
                    self.note_unread(
                        Some(ObjectId::new(number, 0)),
                        format!("object stream {container}: its offset is past the end"),
                    );
                    continue;
                };
                match Lexer::at(&bytes, at).object() {
                    // §7.5.7: every object in an object stream has generation 0.
                    Ok(object) => {
                        self.objects.insert(
                            number,
                            Entry {
                                generation: 0,
                                object,
                            },
                        );
                    }
                    Err(e) => self.note_unread(
                        Some(ObjectId::new(number, 0)),
                        format!("in object stream {container}: {e}"),
                    ),
                }
            }
        }
    }

    /// `/Root`, or the object that calls itself the catalog when the trailer
    /// does not say.
    fn find_catalog(&self) -> Result<ObjectId> {
        if let Some(id) = self.trailer.get("Root").and_then(Object::as_reference)
            && self.object(id).is_some_and(|root| root.is_type("Catalog"))
        {
            return Ok(id);
        }
        let found = self
            .objects
            .iter()
            .find(|(_, entry)| entry.object.is_type("Catalog"))
            .map(|(number, entry)| ObjectId::new(*number, entry.generation));
        found.ok_or_else(|| {
            Error::Format(format!(
                "{}: no document catalog; the trailer names none and no object calls itself one",
                self.name
            ))
        })
    }
}

static EMPTY: Dictionary = Dictionary::empty();

/// `%PDF-<version>`, and where in the file it starts.
fn header(data: &[u8]) -> Result<(usize, String)> {
    let window = &data[..data.len().min(HEADER_WINDOW)];
    let Some(at) = find(window, b"%PDF-") else {
        return Err(Error::Format(
            "not a PDF: no `%PDF-` header in the first kilobyte".into(),
        ));
    };
    let version: String = data[at + 5..]
        .iter()
        .take(8)
        .take_while(|b| b.is_ascii_digit() || **b == b'.')
        .map(|b| *b as char)
        .collect();
    if version.is_empty() {
        return Err(Error::Format(
            "not a PDF: the `%PDF-` header states no version".into(),
        ));
    }
    Ok((at, version))
}

/// Refuse an encrypted document, naming the encryption.
///
/// The `/Encrypt` dictionary is read on its own, before the body: §7.6.1 keeps
/// it in the clear precisely so a reader can find out what it is up against
/// without being able to decrypt anything.
fn refuse_encryption(data: &[u8], xref: &Xref, shift: usize) -> Result<()> {
    let Some(entry) = xref.trailer.get("Encrypt") else {
        return Ok(());
    };
    let dict = match entry {
        Object::Dictionary(dict) => Some(dict.clone()),
        Object::Reference(id) => match xref.slots.get(&id.number) {
            Some(Slot::InFile { offset, .. }) => usize::try_from(*offset)
                .ok()
                .and_then(|at| at.checked_add(shift))
                .filter(|at| *at < data.len())
                .and_then(|at| Lexer::at(data, at).indirect().ok())
                .and_then(|(_, object)| object.as_dict().cloned()),
            _ => None,
        },
        _ => None,
    };

    let what = dict
        .as_ref()
        .map(encrypt::describe)
        .unwrap_or_else(|| "an encryption the file does not describe".into());
    Err(Error::Format(format!(
        "encrypted with {what}; mirsam does not decrypt, so this document is unread rather than \
         clean. Rebuild it without protection, or remove the protection in the application that \
         wrote it"
    )))
}

/// Parse every object the table places in the body.
fn load(data: &[u8], xref: &Xref, shift: usize, unread: &mut Vec<Unread>) -> BTreeMap<u32, Entry> {
    let mut objects = BTreeMap::new();
    for (&number, slot) in &xref.slots {
        let Slot::InFile { offset, generation } = *slot else {
            continue;
        };
        let Ok(offset) = usize::try_from(offset) else {
            continue;
        };
        let id = ObjectId::new(number, generation);

        // The stored offset, then the same offset past a prologue. A file with
        // bytes before its header may have been written either way round.
        let mut parsed = None;
        for at in [Some(offset), offset.checked_add(shift)]
            .into_iter()
            .flatten()
        {
            if at >= data.len() {
                continue;
            }
            match Lexer::at(data, at).indirect() {
                Ok((found, object)) if found.number == number => {
                    parsed = Some((found.generation, object));
                    break;
                }
                _ => continue,
            }
        }
        match parsed {
            Some((generation, object)) => {
                objects.insert(number, Entry { generation, object });
            }
            None => unread.push(Unread {
                object: Some(id),
                reason: format!(
                    "the cross-reference table puts it at byte {offset}, and it is not there"
                ),
            }),
        }
    }
    objects
}
