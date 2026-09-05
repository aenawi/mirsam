//! Reading a font file's family names without reading the font.
//!
//! Indexing a machine means asking several hundred files what they are called.
//! Parsing each one whole would mean reading half a gigabyte on macOS to learn
//! a few hundred short strings, so this reads three small ranges instead: the
//! table directory at the top of the file, the record naming the `name` table,
//! and the `name` table itself. A typeface's name lives in a table of a few
//! kilobytes however large the outlines behind it are.
//!
//! What is *not* hand-rolled is the naming table's own format, which has
//! platforms, encodings and languages and is exactly the kind of thing a
//! second implementation gets subtly wrong. `ttf_parser::name::Table` parses
//! the bytes once they are in hand. The only format knowledge here is the
//! table directory — a tag, a length and an offset, repeated — and the `ttcf`
//! header that puts several of those in one file.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use ttf_parser::PlatformId;
use ttf_parser::name::{Name, Table, name_id};

/// A name table larger than this is a corrupt file, not a font.
///
/// The length comes from the file, so it decides an allocation. Real naming
/// tables run to a few kilobytes; the largest fonts shipped with an operating
/// system do not approach this.
const MAX_NAME_TABLE: u32 = 1 << 20;

/// One face inside a font file, and the family names it answers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    /// Which face within the file: always 0 unless the file is a collection.
    pub index: u32,
    /// Every distinct family name the face states, in the order the table
    /// gives them. More than one is ordinary — a font names its family in
    /// each language it is localised into, and a document may name any of
    /// them.
    pub families: Vec<String>,
    /// The style within the family: `Regular`, `Bold Italic`, `Light`.
    ///
    /// Needed because a family is spread over several files and they all
    /// state the same family name. A document that says `Arial` means
    /// `Arial.ttf`, and a source that took whichever of the eleven files
    /// sorted first would answer with `Arial Bold Italic.ttf` — a different
    /// font, with, on macOS, a different idea of whether it has any Arabic.
    pub subfamily: Option<String>,
}

impl Face {
    /// Whether this is the upright, unemphasised face of its family: the one
    /// a document naming the family alone is asking for.
    pub fn is_regular(&self) -> bool {
        self.subfamily
            .as_deref()
            .is_some_and(|style| style.eq_ignore_ascii_case("regular"))
    }
}

/// Every face in the font file at `path`, and what each calls itself.
///
/// An empty result means the file parsed but named nothing, which is a font
/// no document can ask for by name. An error means the file could not be
/// read; a caller indexing a directory should skip it, because a machine with
/// one truncated font is not a machine with no fonts.
pub fn faces(path: &Path) -> io::Result<Vec<Face>> {
    let mut file = File::open(path)?;
    let mut tag = [0u8; 4];
    file.read_exact(&mut tag)?;

    let offsets = if &tag == b"ttcf" {
        collection_offsets(&mut file)?
    } else {
        vec![0]
    };

    let mut faces = Vec::new();
    for (index, offset) in offsets.into_iter().enumerate() {
        if let Some(table) = name_table(&mut file, offset)? {
            faces.push(Face {
                index: index as u32,
                families: families(&table),
                subfamily: subfamily(&table),
            });
        }
    }
    Ok(faces)
}

/// Where each face's table directory begins, from a `ttcf` header.
fn collection_offsets(file: &mut File) -> io::Result<Vec<u64>> {
    file.seek(SeekFrom::Start(8))?; // past the tag and the version
    let count = read_u32(file)?;
    (0..count).map(|_| read_u32(file).map(u64::from)).collect()
}

/// The naming table of the face whose table directory starts at `offset`.
///
/// `Ok(None)` when the face has no `name` table at all, which is legal and
/// leaves the file unaddressable by name.
fn name_table(file: &mut File, offset: u64) -> io::Result<Option<Vec<u8>>> {
    file.seek(SeekFrom::Start(offset + 4))?; // past sfntVersion
    let tables = u32::from(read_u16(file)?);
    file.seek(SeekFrom::Start(offset + 12))?; // past the binary-search hints

    let mut directory = vec![0u8; 16 * tables as usize];
    file.read_exact(&mut directory)?;

    let Some(record) = directory.chunks_exact(16).find(|r| &r[..4] == b"name") else {
        return Ok(None);
    };
    let table_offset = u32::from_be_bytes([record[8], record[9], record[10], record[11]]);
    let length = u32::from_be_bytes([record[12], record[13], record[14], record[15]]);
    if length == 0 || length > MAX_NAME_TABLE {
        return Ok(None);
    }

    file.seek(SeekFrom::Start(u64::from(table_offset)))?;
    let mut table = vec![0u8; length as usize];
    file.read_exact(&mut table)?;

    Ok(Some(table))
}

/// The distinct family names a naming table states, in table order.
///
/// Both `FAMILY` and `TYPOGRAPHIC_FAMILY` are read. They differ for a family
/// with more than four styles — `Helvetica Neue Condensed Black` is the
/// former and `Helvetica Neue` the latter — and a document may name either.
/// So are localised names: a deck authored in Arabic may well say `جيزة`
/// where an English one says `Geeza Pro`, and both are the same file.
fn families(table: &[u8]) -> Vec<String> {
    let Some(parsed) = Table::parse(table) else {
        return Vec::new();
    };

    let mut families: Vec<String> = Vec::new();
    for name in parsed.names {
        if name.name_id != name_id::FAMILY && name.name_id != name_id::TYPOGRAPHIC_FAMILY {
            continue;
        }
        let Some(value) = text(&name) else {
            continue;
        };
        if !families.contains(&value) {
            families.push(value);
        }
    }
    families
}

/// The style within the family, preferring the typographic name.
///
/// `Regular` for the upright face, `Bold Italic` and its kin for the others.
fn subfamily(table: &[u8]) -> Option<String> {
    let parsed = Table::parse(table)?;
    let of = |wanted: u16| {
        parsed
            .names
            .into_iter()
            .find(|name| name.name_id == wanted)
            .and_then(|name| text(&name))
    };
    of(name_id::TYPOGRAPHIC_SUBFAMILY).or_else(|| of(name_id::SUBFAMILY))
}

/// One name record as text, or `None` if this cannot read it exactly.
///
/// `Name::to_string` handles the Unicode encodings and refuses the legacy
/// ones. Refusing outright would lose most of what Apple ships: macOS fonts
/// state their English names on the Macintosh platform in Mac Roman and keep
/// the Unicode records for the localisations, so `Helvetica.ttc` names itself
/// nothing at all through the Unicode records alone.
///
/// Mac Roman agrees with ASCII below 0x80 and diverges above it. So an
/// all-ASCII record is decoded exactly, and anything else is left unread
/// rather than guessed at — a wrong name is worse than a missing one, because
/// it resolves.
fn text(name: &Name) -> Option<String> {
    let value = match name.to_string() {
        Some(unicode) => unicode,
        None if name.platform_id == PlatformId::Macintosh
            && name.encoding_id == MAC_ROMAN
            && name.name.is_ascii() =>
        {
            String::from_utf8(name.name.to_vec()).ok()?
        }
        None => return None,
    };
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// Encoding 0 of the Macintosh platform.
const MAC_ROMAN: u16 = 0;

fn read_u16(file: &mut File) -> io::Result<u16> {
    let mut buf = [0u8; 2];
    file.read_exact(&mut buf)?;
    Ok(u16::from_be_bytes(buf))
}

fn read_u32(file: &mut File) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(platform_id: PlatformId, encoding_id: u16, name: &[u8]) -> Name<'_> {
        Name {
            platform_id,
            encoding_id,
            language_id: 0,
            name_id: name_id::FAMILY,
            name,
        }
    }

    #[test]
    fn a_windows_record_is_read_as_utf16() {
        let utf16: Vec<u8> = "Geeza Pro"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        let name = record(PlatformId::Windows, 1, &utf16);
        assert_eq!(text(&name).as_deref(), Some("Geeza Pro"));
    }

    #[test]
    fn an_ascii_mac_roman_record_is_read_exactly() {
        // How macOS states an English family name. Refusing it would leave
        // `Helvetica.ttc` naming itself nothing at all, because its Unicode
        // records carry only the localisations.
        let name = record(PlatformId::Macintosh, MAC_ROMAN, b"Helvetica");
        assert_eq!(text(&name).as_deref(), Some("Helvetica"));
    }

    #[test]
    fn a_mac_roman_record_that_is_not_ascii_is_left_unread() {
        // Above 0x80 Mac Roman is its own codepage: 0xA5 is a bullet, not the
        // Latin-1 yen. A wrong name is worse than a missing one, because it
        // resolves.
        let name = record(PlatformId::Macintosh, MAC_ROMAN, b"Caf\xa9");
        assert_eq!(text(&name), None);
    }

    #[test]
    fn only_the_upright_face_is_the_regular_one() {
        let face = |style: &str| Face {
            index: 0,
            families: vec!["Arial".to_string()],
            subfamily: Some(style.to_string()),
        };
        assert!(face("Regular").is_regular());
        assert!(face("regular").is_regular());
        assert!(!face("Bold Italic").is_regular());
        assert!(
            !Face {
                index: 0,
                families: vec!["Arial".to_string()],
                subfamily: None,
            }
            .is_regular()
        );
    }
}
