//! The cross-reference chain: newest wins, and a malformed chain terminates.

use mirsam_pdf::xref::{Slot, read, recover};

/// A document with two sections, the newer one redefining object 3.
fn incremental(previous_marks_free: bool) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let first_object = out.len();
    out.extend_from_slice(b"3 0 obj\n(old)\nendobj\n");
    let first_table = out.len();
    out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n0000000000 65535 f \n");
    out.extend_from_slice(b"0000000000 65535 f \n");
    out.extend_from_slice(format!("{first_object:010} 00000 n \n").as_bytes());
    out.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{first_table}\n%%EOF\n").as_bytes(),
    );

    let second_object = out.len();
    out.extend_from_slice(b"3 0 obj\n(new)\nendobj\n");
    let second_table = out.len();
    let entry = if previous_marks_free {
        "0000000000 65535 f \n".to_string()
    } else {
        format!("{second_object:010} 00000 n \n")
    };
    out.extend_from_slice(format!("xref\n3 1\n{entry}").as_bytes());
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size 4 /Root 1 0 R /Prev {first_table} >>\nstartxref\n{second_table}\n%%EOF\n"
        )
        .as_bytes(),
    );
    out
}

#[test]
fn the_newer_section_wins() {
    let data = incremental(false);
    let xref = read(&data, 0).expect("read");
    let Some(Slot::InFile { offset, .. }) = xref.slots.get(&3) else {
        panic!("object 3 is not in the table")
    };
    assert_eq!(
        &data[*offset as usize..*offset as usize + 13],
        b"3 0 obj\n(new)"
    );
}

/// A free entry in a newer section is what *hides* an object an older one
/// still names. Reading it as "no entry here, look further back" would
/// resurrect a deleted object.
#[test]
fn a_free_entry_in_a_newer_section_hides_the_older_one() {
    let data = incremental(true);
    let xref = read(&data, 0).expect("read");
    assert_eq!(xref.slots.get(&3), Some(&Slot::Free));
}

/// A `/Prev` pointing at the section that carried it. Two writers producing
/// the same offset is a loop, not a longer history.
#[test]
fn a_prev_loop_terminates() {
    let mut out = b"%PDF-1.4\n".to_vec();
    let object = out.len();
    out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog >>\nendobj\n");
    let table = out.len();
    out.extend_from_slice(b"xref\n0 2\n0000000000 65535 f \n");
    out.extend_from_slice(format!("{object:010} 00000 n \n").as_bytes());
    out.extend_from_slice(
        format!("trailer\n<< /Size 2 /Root 1 0 R /Prev {table} >>\nstartxref\n{table}\n%%EOF\n")
            .as_bytes(),
    );
    let xref = read(&out, 0).expect("read");
    assert_eq!(xref.slots.len(), 2);
}

/// A file with no `startxref` at all is not readable through the chain, and
/// says so rather than answering with an empty table.
#[test]
fn no_startxref_is_an_error() {
    let error = read(b"%PDF-1.4\n1 0 obj\nnull\nendobj\n", 0)
        .unwrap_err()
        .to_string();
    assert!(error.contains("no `startxref`"), "{error}");
}

/// The scan takes the *last* definition of an object number, which is the same
/// rule the chain walk enforces from the other end: an update appends.
#[test]
fn recovery_takes_the_last_definition() {
    let data = incremental(false);
    let xref = recover(&data);
    let Some(Slot::InFile { offset, .. }) = xref.slots.get(&3) else {
        panic!("object 3 was not found")
    };
    assert_eq!(
        &data[*offset as usize..*offset as usize + 13],
        b"3 0 obj\n(new)"
    );
    // And the newest trailer, so a /Root moved by an update is the one used.
    assert!(xref.trailer.contains_key("Root"));
}

/// `objstm` is not `obj`. A scan that matched inside a longer token would
/// invent objects out of the middle of dictionaries.
#[test]
fn recovery_does_not_match_inside_a_longer_token() {
    let data = b"%PDF-1.4\n1 0 objstm\n2 0 obj\nnull\nendobj\n";
    let xref = recover(data);
    assert_eq!(xref.slots.keys().copied().collect::<Vec<_>>(), [2]);
}

/// The cross-reference streams in the fixture corpus, read through the same
/// entry point: a compressed object comes back as one, and the type-2 entry
/// names the stream that holds it.
#[test]
fn a_cross_reference_stream_names_object_streams() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdfs/compressed.pdf");
    let data = std::fs::read(path).expect("fixture");
    let xref = read(&data, 0).expect("read");
    assert_eq!(
        xref.slots.get(&1),
        Some(&Slot::InStream {
            stream: 8,
            index: 0
        })
    );
    assert!(matches!(xref.slots.get(&4), Some(Slot::InFile { .. })));
    assert!(xref.trailer.contains_key("Root"));
}
