//! The fixture corpus, one file per generator family.
//!
//! `scripts/make-pdf-fixture.py` writes these byte by byte; `make pdfs`
//! regenerates them. Each asserts the *whole* of what the object layer owes a
//! caller for that family — the pages come back, the content decodes, the
//! Arabic survives, and nothing is silently missing — because a test that only
//! checked the file opened would pass on a reader that returned an empty
//! document.

use mirsam_pdf::{Pdf, Reconstruction};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/pdfs")
        .join(name)
}

fn open(name: &str) -> Pdf {
    Pdf::open(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// How the fixtures write the Arabic: one byte per character, as a hexadecimal
/// string. Fourteen codes for the fourteen characters of `\u{627}\u{644}\u{62a}\u{642}\u{631}\u{64a}\u{631}
/// \u{627}\u{644}\u{633}\u{646}\u{648}\u{64a}`, which is the shape a generator that subset its font
/// produces and the shape PLAN \u{a7}6.3 hunts a reversal in.
const ARABIC_RUN: &[u8] = b"<4142434445464748494A4B4C4D4E>";

fn content(pdf: &Pdf) -> Vec<u8> {
    pdf.pages()[0].content(pdf)
}

#[test]
fn classic_table_uncompressed() {
    let pdf = open("classic.pdf");
    assert_eq!(pdf.version(), "1.4");
    assert_eq!(pdf.pages().len(), 1);
    assert!(pdf.reconstructed().is_empty(), "nothing needed rebuilding");
    assert!(pdf.unread().is_empty(), "{:?}", pdf.unread());

    let content = content(&pdf);
    assert!(content.starts_with(b"BT\n"));
    assert!(
        content.contains_str(ARABIC_RUN),
        "the Arabic run is not in the content stream"
    );
}

#[test]
fn cross_reference_stream_and_object_stream() {
    let pdf = open("compressed.pdf");
    assert_eq!(pdf.version(), "1.7");
    assert!(pdf.reconstructed().is_empty(), "{:?}", pdf.reconstructed());
    assert!(pdf.unread().is_empty(), "{:?}", pdf.unread());

    // The catalog, the page tree node, the page and the font all came out of
    // the object stream: a reader with no inflate finds none of them.
    assert_eq!(pdf.pages().len(), 1);
    assert!(pdf.catalog().contains_key("Pages"));
    let content = content(&pdf);
    assert!(
        content.starts_with(b"BT\n"),
        "{:?}",
        &content[..8.min(content.len())]
    );
    assert!(content.contains_str(ARABIC_RUN));
}

#[test]
fn hybrid_file_prefers_the_stream() {
    let pdf = open("hybrid.pdf");
    assert_eq!(pdf.pages().len(), 1);
    // Object 6 is the ToUnicode CMap, and only the /XRefStm says where it is.
    // A reader that stopped at the classic table has five objects and a font
    // that says nothing about what it draws.
    assert!(
        pdf.object(mirsam_pdf::ObjectId::new(6, 0)).is_some(),
        "the object only the /XRefStm names was not found"
    );
    assert!(pdf.unread().is_empty(), "{:?}", pdf.unread());
}

#[test]
fn an_incremental_update_supersedes_what_it_replaces() {
    let base = open("classic.pdf");
    let updated = open("incremental.pdf");

    let before = content(&base);
    let after = content(&updated);
    assert_ne!(before, after, "the update changed nothing");
    assert!(
        after.len() > before.len(),
        "the newer content stream is the longer one, and this is not it"
    );
    assert!(updated.reconstructed().is_empty());
}

#[test]
fn a_stale_startxref_is_recovered_from() {
    let pdf = open("damaged.pdf");
    assert_eq!(
        pdf.reconstructed(),
        [Reconstruction::CrossReferenceTable],
        "the rebuild has to be said out loud"
    );
    assert_eq!(pdf.pages().len(), 1);
    assert_eq!(content(&pdf), content(&open("classic.pdf")));
}

#[test]
fn an_encrypted_document_is_refused_by_name() {
    let error = Pdf::open(&fixture("encrypted.pdf"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("AES-256"), "{error}");
    assert!(error.contains("/Standard security handler"), "{error}");
    assert!(error.contains("V5 R6"), "{error}");
    assert!(
        error.contains("unread rather than clean"),
        "the refusal has to say what it is not claiming: {error}"
    );
}

#[test]
fn the_ascii_filters_a_distiller_writes() {
    let pdf = open("ascii-filters.pdf");
    assert!(pdf.unread().is_empty(), "{:?}", pdf.unread());

    // /Contents is an array of three streams behind three different filters,
    // joined at their boundaries.
    let content = content(&pdf);
    assert!(
        content.starts_with(b"BT\n"),
        "ASCII85 then Flate did not unwind"
    );
    assert!(content.contains_str(b"% run length\n"), "RunLengthDecode");
    assert!(content.contains_str(b"% lzw\n"), "LZWDecode");

    // The ToUnicode CMap is behind ASCIIHexDecode.
    let cmap = pdf
        .decoded(mirsam_pdf::ObjectId::new(6, 0))
        .unwrap()
        .unwrap();
    assert!(cmap.contains_str(b"beginbfchar"));
}

#[test]
fn a_scanned_page_is_named_rather_than_called_clean() {
    let pdf = open("image-only.pdf");
    assert_eq!(pdf.pages().len(), 1);

    // The page's own content stream reads perfectly well; it draws an image.
    let content = content(&pdf);
    assert!(content.contains_str(b"/Im1 Do"));

    // Asking for the image is where the honesty is: the bytes are not text
    // this crate failed to read, they are text that is not in the file.
    let image = pdf.decoded(mirsam_pdf::ObjectId::new(5, 0)).unwrap();
    assert!(image.is_none(), "an image codec must not come back decoded");
    let unread = pdf.unread();
    assert_eq!(unread.len(), 1, "{unread:?}");
    assert!(unread[0].starts_with("5 0 R — /DCTDecode"), "{unread:?}");
}

#[test]
fn every_fixture_states_its_own_version_and_one_page() {
    for name in [
        "classic.pdf",
        "compressed.pdf",
        "hybrid.pdf",
        "incremental.pdf",
        "damaged.pdf",
        "ascii-filters.pdf",
        "image-only.pdf",
    ] {
        let pdf = open(name);
        assert!(pdf.version().starts_with("1."), "{name}: {}", pdf.version());
        assert_eq!(pdf.pages().len(), 1, "{name}");
        // MediaBox is stated on the /Pages node in every fixture, so a page
        // that has one proves the inheritable attributes came down the tree.
        assert!(
            pdf.pages()[0].dict.contains_key("MediaBox"),
            "{name}: the page did not inherit /MediaBox"
        );
    }
}

/// `slice::contains` is for one element; this is for a subslice.
trait ContainsStr {
    fn contains_str(&self, needle: &[u8]) -> bool;
}

impl ContainsStr for Vec<u8> {
    fn contains_str(&self, needle: &[u8]) -> bool {
        self.windows(needle.len()).any(|w| w == needle)
    }
}
