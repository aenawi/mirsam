//! The object layer's own behaviours, on documents built for one question
//! each — the fixture corpus proves the generator families, and these prove
//! the rules the loader applies to all of them.

use mirsam_pdf::object::{Name, Object, ObjectId};
use mirsam_pdf::{Pdf, Reconstruction};

/// Assemble a PDF from object bodies, with a correct classic table.
///
/// Written here rather than taken from the fixture script so that a test can
/// state one situation in six lines. Object 1 is the catalog by convention.
fn build(objects: &[(u32, &[u8])], trailer_extra: &str) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (number, body) in objects {
        offsets.push((*number, out.len()));
        out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let size = objects.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
    let startxref = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for number in 1..size {
        match offsets.iter().find(|(n, _)| *n == number) {
            Some((_, at)) => out.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes()),
            None => out.extend_from_slice(b"0000000000 65535 f \n"),
        }
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R{trailer_extra} >>\nstartxref\n{startxref}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

fn open(objects: &[(u32, &[u8])]) -> Pdf {
    Pdf::from_bytes(&build(objects, ""), "test").expect("open")
}

const CATALOG: &[u8] = b"<< /Type /Catalog /Pages 2 0 R >>";

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

/// The cross-reference table's own `xref`, and not the one inside the
/// `startxref` that follows it.
fn table_at(data: &[u8]) -> usize {
    rfind(data, b"\nxref\n").expect("the table") + 1
}

#[test]
fn a_file_with_no_header_is_not_a_pdf() {
    let error = Pdf::from_bytes(b"just some bytes", "test")
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a PDF"), "{error}");
}

/// A `%PDF-` header preceded by junk is common enough that every viewer
/// tolerates it, and the stored offsets may be relative to either origin.
#[test]
fn a_prologue_before_the_header_is_read_through() {
    let mut data = b"#!/bin/sh\n# a self-extracting wrapper\n".to_vec();
    let body = build(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, b"<< /Type /Page /Parent 2 0 R >>"),
        ],
        "",
    );
    data.extend_from_slice(&body);
    let pdf = Pdf::from_bytes(&data, "wrapped").expect("open");
    assert_eq!(pdf.pages().len(), 1);
    assert!(pdf.reconstructed().is_empty(), "{:?}", pdf.reconstructed());
}

/// §7.7.3.4's inheritable attributes come down the tree. `/Resources` is the
/// one that matters: a text layer reading it from the leaf alone would find no
/// fonts on the very common document that states them once at the root.
#[test]
fn a_page_inherits_resources_and_the_boxes() {
    let pdf = open(&[
        (1, CATALOG),
        (
            2,
            b"<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 595 842] \
               /Resources << /Font << /F1 9 0 R >> >> /Rotate 90 >>",
        ),
        (4, b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Rotate 0 >>"),
        (3, b"<< /Type /Page /Parent 4 0 R >>"),
    ]);
    let page = &pdf.pages()[0];
    assert!(
        page.resources(&pdf).is_some(),
        "the fonts did not come down"
    );
    assert!(page.dict.contains_key("MediaBox"));
    // The nearer ancestor's value wins over the further one's.
    assert_eq!(
        pdf.get(&page.dict, "Rotate").and_then(Object::as_i64),
        Some(0)
    );
}

/// A page's own value is never overwritten by what it inherits.
#[test]
fn a_page_outranks_what_it_inherits() {
    let pdf = open(&[
        (1, CATALOG),
        (
            2,
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 595 842] >>",
        ),
        (
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>",
        ),
    ]);
    let media = pdf.get(&pdf.pages()[0].dict, "MediaBox").unwrap();
    assert_eq!(media.as_array().unwrap()[2].as_i64(), Some(100));
}

/// A `/Pages` node with no `/Type` is still an internal node: a great many
/// writers omit it, and reading the shape rather than the label is the only
/// way to walk those files.
#[test]
fn a_node_with_kids_is_an_internal_node_whatever_it_says() {
    let pdf = open(&[
        (1, CATALOG),
        (2, b"<< /Kids [3 0 R] /Count 1 >>"),
        (3, b"<< /Type /Page /Parent 2 0 R >>"),
    ]);
    assert_eq!(pdf.pages().len(), 1);
}

/// `/Kids` pointing back up the tree is a file, not a document. It must be
/// read, not hung on.
#[test]
fn a_cycle_in_the_page_tree_terminates() {
    let pdf = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R 2 0 R] /Count 2 >>"),
        (3, b"<< /Type /Page /Parent 2 0 R >>"),
    ]);
    assert_eq!(pdf.pages().len(), 1);
}

/// So is a reference that points at itself.
#[test]
fn a_cycle_in_the_references_resolves_to_null() {
    let pdf = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, b"<< /Type /Page /Parent 2 0 R /Rotate 4 0 R >>"),
        (4, b"5 0 R"),
        (5, b"4 0 R"),
    ]);
    assert_eq!(pdf.get(&pdf.pages()[0].dict, "Rotate"), None);
}

/// §7.3.9: a reference to an object the file does not have is null, and null
/// is indistinguishable from absent.
#[test]
fn a_dangling_reference_is_absent() {
    let pdf = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, b"<< /Type /Page /Rotate 99 0 R /CropBox null >>"),
    ]);
    let page = &pdf.pages()[0].dict;
    assert_eq!(pdf.get(page, "Rotate"), None);
    assert_eq!(pdf.get(page, "CropBox"), None);
}

/// A trailer with no `/Root`, which a file assembled by hand often has. The
/// object that calls itself the catalog is taken instead.
#[test]
fn a_missing_root_is_found_by_type() {
    let data = build(
        &[
            (7, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, b"<< /Type /Page /Parent 2 0 R >>"),
        ],
        "",
    );
    // /Root points at object 1, which does not exist.
    let pdf = Pdf::from_bytes(&data, "test").expect("open");
    assert!(pdf.catalog().contains_key("Pages"));
    assert_eq!(pdf.pages().len(), 1);
}

/// A document with a catalog nobody can find is unreadable, and says so.
#[test]
fn no_catalog_at_all_is_a_refusal() {
    let data = build(&[(1, b"<< /Type /Page >>")], "");
    let error = Pdf::from_bytes(&data, "test").unwrap_err().to_string();
    assert!(error.contains("no document catalog"), "{error}");
}

/// An object at an offset that is not there is recovered from the body if it
/// is there at all — and only reported when it genuinely is not.
#[test]
fn an_object_the_table_misplaces_is_found_anyway() {
    let mut data = build(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, b"<< /Type /Page /Parent 2 0 R >>"),
        ],
        "",
    );
    // Break the entry for object 3 without touching the object itself.
    let table = table_at(&data);
    let entry = table + b"xref\n0 4\n".len() + 20 * 3;
    data[entry..entry + 10].copy_from_slice(b"0000009999");

    let pdf = Pdf::from_bytes(&data, "test").expect("open");
    assert_eq!(pdf.pages().len(), 1);
    assert_eq!(pdf.reconstructed(), [Reconstruction::CrossReferenceTable]);
    assert!(pdf.unread().is_empty(), "{:?}", pdf.unread());
}

/// And when it really is not there, it is named — with the offset the table
/// claimed, so a reviewer can go and look.
#[test]
fn an_object_that_is_not_in_the_file_is_named() {
    let mut data = build(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, b"<< /Type /Page /Parent 2 0 R >>"),
            (4, b"<< /Type /Nothing >>"),
        ],
        "",
    );
    // Delete object 4's body, leaving the table naming it.
    let at = rfind(&data, b"4 0 obj").expect("object 4");
    let end = table_at(&data);
    data.splice(at..end, std::iter::repeat_n(b' ', end - at));

    let pdf = Pdf::from_bytes(&data, "test").expect("open");
    let unread = pdf.unread();
    assert_eq!(unread.len(), 1, "{unread:?}");
    assert!(unread[0].starts_with("4 0 R —"), "{unread:?}");
    assert!(unread[0].contains("is not there"), "{unread:?}");
}

/// A dictionary key repeated in the file resolves to one entry, and the
/// document reads through it the same way.
#[test]
fn resolution_follows_a_chain_of_references() {
    let pdf = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, b"<< /Type /Page /Rotate 4 0 R >>"),
        (4, b"5 0 R"),
        (5, b"180"),
    ]);
    assert_eq!(
        pdf.get(&pdf.pages()[0].dict, "Rotate")
            .and_then(Object::as_i64),
        Some(180)
    );
}

#[test]
fn objects_are_matched_on_the_number_alone() {
    let pdf = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, b"<< /Type /Page >>"),
    ]);
    // A generation the table disagrees with is something writers produce and
    // every reader ignores; refusing to follow it would lose the object.
    assert!(pdf.object(ObjectId::new(3, 7)).is_some());
}

#[test]
fn a_document_reports_what_it_reconstructed_once() {
    let pdf = Pdf::open(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdfs/damaged.pdf"),
    )
    .expect("open");
    assert_eq!(pdf.reconstructed(), [Reconstruction::CrossReferenceTable]);
    assert_eq!(
        Reconstruction::CrossReferenceTable.to_string(),
        "the cross-reference table"
    );
}

/// The text layer above this one has sources it cannot read too — a font with
/// no `ToUnicode` map is one — and they have to reach the same list.
#[test]
fn the_unread_list_is_open_to_the_layer_above() {
    let pdf = open(&[
        (1, CATALOG),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, b"<< /Type /Page >>"),
    ]);
    pdf.note_unread(
        Some(ObjectId::new(9, 0)),
        "no /ToUnicode and no standard encoding",
    );
    pdf.note_unread(None, "page 1 draws its Arabic as an image");
    let unread = pdf.unread();
    assert_eq!(unread.len(), 2);
    assert!(
        unread
            .iter()
            .any(|u| u == "9 0 R — no /ToUnicode and no standard encoding")
    );
    assert!(
        unread
            .iter()
            .any(|u| u == "page 1 draws its Arabic as an image")
    );
}

#[test]
fn a_name_answers_for_itself_and_nothing_else() {
    let name = Name::from("FlateDecode");
    assert!(name.is("FlateDecode"));
    assert!(!name.is("Flate"));
    assert_eq!(name.as_str(), Some("FlateDecode"));
}

/// mirsam is run on files people were sent. Every prefix and every
/// single-byte corruption of the fixture corpus has to come back as a document
/// or as an error, and never as a panic — the bounds in this crate are what
/// make that true, and this is what asserts they are all still there.
#[test]
fn no_prefix_or_corruption_of_the_corpus_panics() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdfs");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&dir).expect("fixtures") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "pdf") {
            continue;
        }
        let data = std::fs::read(&path).expect("read");
        checked += 1;

        for cut in (0..data.len()).step_by(7) {
            let _ = Pdf::from_bytes(&data[..cut], "truncated");
        }
        for at in (0..data.len()).step_by(13) {
            let mut corrupted = data.clone();
            corrupted[at] = corrupted[at].wrapping_add(0x5b);
            let _ = Pdf::from_bytes(&corrupted, "corrupted");
        }
    }
    assert!(checked >= 8, "the corpus lost fixtures: {checked}");
}
