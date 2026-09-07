//! Stream filters: §7.4, and the line between a filter that was not applied
//! and one this tool will never apply.

use mirsam_pdf::filter::{Decoded, decode};
use mirsam_pdf::object::{Dictionary, Name, Object};

fn bytes(data: &[u8], filters: &[&str], parms: &[Dictionary]) -> Vec<u8> {
    let filters: Vec<Name> = filters.iter().copied().map(Name::from).collect();
    match decode(data, &filters, parms).expect("decode") {
        Decoded::Bytes(bytes) => bytes,
        Decoded::Stopped { filter } => panic!("stopped at {filter}"),
    }
}

fn stopped(data: &[u8], filters: &[&str]) -> Name {
    let filters: Vec<Name> = filters.iter().copied().map(Name::from).collect();
    match decode(data, &filters, &[]).expect("decode") {
        Decoded::Bytes(_) => panic!("expected to stop"),
        Decoded::Stopped { filter } => filter,
    }
}

fn parms(entries: &[(&str, i64)]) -> Dictionary {
    entries
        .iter()
        .map(|(key, value)| (Name::from(*key), Object::Integer(*value)))
        .collect()
}

fn deflate(data: &[u8]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn no_filter_is_the_bytes_themselves() {
    assert_eq!(bytes(b"plain", &[], &[]), b"plain");
}

#[test]
fn flate() {
    let source = b"the annual report, at some length, so it actually compresses";
    assert_eq!(bytes(&deflate(source), &["FlateDecode"], &[]), source);
    // Abbreviated names are §7.4's own, for inline images.
    assert_eq!(bytes(&deflate(source), &["Fl"], &[]), source);
}

/// A truncated stream keeps what it had. Discarding it would turn a document
/// with a damaged tail into a document with no text, which is the failure this
/// project treats as worse than a miss.
#[test]
fn a_truncated_flate_stream_keeps_what_decoded() {
    let source = b"the annual report, at some length, so it actually compresses";
    let mut compressed = deflate(source);
    compressed.truncate(compressed.len() - 4);
    let out = bytes(&compressed, &["FlateDecode"], &[]);
    assert!(!out.is_empty(), "a truncated stream decoded to nothing");
    assert!(source.starts_with(&out[..]));
}

#[test]
fn ascii_hex() {
    assert_eq!(bytes(b"48656C6C6F>", &["ASCIIHexDecode"], &[]), b"Hello");
    // White space between digits is ignored, and a missing final digit is a
    // zero.
    assert_eq!(bytes(b"48 65\n6C 6C 6F 9>", &["AHx"], &[]), b"Hello\x90");
}

#[test]
fn ascii85() {
    assert_eq!(
        bytes(b"87cURD]i,\"Ebo80~>", &["ASCII85Decode"], &[]),
        b"Hello World!"
    );
    // `z` is four zero bytes, and only at a group boundary.
    assert_eq!(bytes(b"z~>", &["A85"], &[]), [0, 0, 0, 0]);
    // The `<~` some writers open with.
    assert_eq!(bytes(b"<~87cURDZ~>", &["ASCII85Decode"], &[]), b"Hello");
}

#[test]
fn run_length() {
    // A length byte under 128 introduces that many literal bytes plus one;
    // over 128 repeats the next byte 257 - length times; 128 ends the data.
    let encoded = [4u8, b'H', b'e', b'l', b'l', b'o', 253, b'!', 128, b'?'];
    assert_eq!(bytes(&encoded, &["RunLengthDecode"], &[]), b"Hello!!!!");
}

#[test]
fn lzw() {
    // §7.4.4.2's own example: the codes for -----A---B, packed nine bits wide.
    let encoded = [0x80u8, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
    assert_eq!(bytes(&encoded, &["LZWDecode"], &[]), b"-----A---B");
}

/// The PNG predictors, which a cross-reference stream always carries and
/// without which the table decodes to noise that looks like data.
#[test]
fn png_predictors() {
    // Three rows of four bytes, each prefixed with its filter type: none, sub
    // (each byte is a delta from its left neighbour) and up (from the row
    // above).
    let encoded = [
        0u8, 1, 2, 3, 4, // 1 2 3 4
        1, 1, 1, 1, 1, // 1 2 3 4 again, as deltas
        2, 1, 1, 1, 1, // 2 3 4 5
    ];
    let out = mirsam_pdf::filter::decode(
        &deflate(&encoded),
        &[Name::from("FlateDecode")],
        &[parms(&[("Predictor", 12), ("Columns", 4)])],
    )
    .expect("decode");
    let Decoded::Bytes(out) = out else {
        panic!("stopped")
    };
    assert_eq!(out, [1, 2, 3, 4, 1, 2, 3, 4, 2, 3, 4, 5]);
}

#[test]
fn the_tiff_predictor() {
    let encoded = deflate(&[1u8, 1, 1, 1, 5, 1, 1, 1]);
    let out = mirsam_pdf::filter::decode(
        &encoded,
        &[Name::from("FlateDecode")],
        &[parms(&[
            ("Predictor", 2),
            ("Columns", 4),
            ("BitsPerComponent", 8),
        ])],
    )
    .expect("decode");
    let Decoded::Bytes(out) = out else {
        panic!("stopped")
    };
    assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn a_chain_unwinds_in_order() {
    let source = b"the annual report";
    let mut ascii85 = Vec::new();
    for chunk in deflate(source).chunks(4) {
        let mut value = 0u32;
        for i in 0..4 {
            value = value * 256 + u32::from(chunk.get(i).copied().unwrap_or(0));
        }
        let mut digits = [0u8; 5];
        for slot in digits.iter_mut().rev() {
            *slot = 33 + (value % 85) as u8;
            value /= 85;
        }
        ascii85.extend_from_slice(&digits[..chunk.len() + 1]);
    }
    ascii85.extend_from_slice(b"~>");
    assert_eq!(
        bytes(&ascii85, &["ASCII85Decode", "FlateDecode"], &[]),
        source
    );
}

/// An image codec is named rather than decoded. The bytes behind one are not
/// text this crate failed to read; they are text that is not in the file, and
/// ADR 0009 says the difference has to reach the report.
#[test]
fn an_image_codec_stops_the_chain_by_name() {
    for codec in ["DCTDecode", "JPXDecode", "CCITTFaxDecode", "JBIG2Decode"] {
        assert!(stopped(b"\xff\xd8\xff", &[codec]).is(codec));
    }
}

/// So does a filter this crate has simply never met, rather than an error that
/// would lose the rest of the document.
#[test]
fn an_unknown_filter_stops_the_chain_too() {
    assert!(stopped(b"x", &["SomeVendorDecode"]).is("SomeVendorDecode"));
}

/// `/Crypt` reaching this module means an encrypted document was not refused
/// at open, which is a bug in this crate rather than a fact about the file —
/// so it stops, and the name says which.
#[test]
fn crypt_stops_the_chain() {
    assert!(stopped(b"x", &["Crypt"]).is("Crypt"));
}
