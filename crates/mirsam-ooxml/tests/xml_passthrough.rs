//! Does the token stream round-trip byte-identically?
//!
//! `roundtrip.rs` proves an untouched *part* survives a package rewrite. This
//! proves an untouched *token* survives a part rewrite. Without it, "the diff
//! contains exactly the intended change and nothing else" is not a claim that
//! can be tested.

use mirsam_ooxml::package::Package;
use mirsam_ooxml::rewrite::passthrough;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn passthrough_is_byte_identical_on_every_part_of_the_torture_deck() {
    let pkg = Package::open(fixture("torture.pptx")).unwrap();
    let parts = pkg
        .parts_where(|n| n.ends_with(".xml") || n.ends_with(".rels"))
        .unwrap();
    assert!(parts.len() >= 15, "expected the full deck, got {parts:?}");

    for part in parts {
        let before = pkg.read_text(&part).unwrap();
        let after = passthrough(&part, &before).unwrap();
        assert!(
            before == after,
            "{part}: token round-trip changed the bytes at offset {:?}\n  before: {:?}\n  after:  {:?}",
            before
                .as_bytes()
                .iter()
                .zip(after.as_bytes())
                .position(|(a, b)| a != b),
            before.get(..200),
            after.get(..200),
        );
    }
}
