//! Resolving a typeface name to the bytes that answer for it.
//!
//! The directory searched is the shaping fixtures'. Three files, three
//! families, and one difference each — which makes it a machine for asking
//! whether the right bytes came back rather than merely some bytes: the font
//! that shapes and the font that does not are indistinguishable except by
//! shaping through them.
//!
//! Nothing here indexes the machine it runs on. A suite that did would assert
//! whatever the developer happened to have installed.

use std::path::PathBuf;

use mirsam_core::coverage::coverage;
use mirsam_core::ports::FontSource;
use mirsam_core::shape::shape;
use mirsam_fonts::SystemFonts;

fn fixtures() -> SystemFonts {
    SystemFonts::in_dirs([PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../mirsam-core/tests/fonts"
    ))])
}

#[test]
fn every_font_in_the_directory_is_indexed_by_the_name_it_gives_itself() {
    assert_eq!(
        fixtures().families().collect::<Vec<_>>(),
        ["mirsam joining", "mirsam nonjoining", "mirsam partial"]
    );
}

#[test]
fn the_name_resolves_to_the_bytes_of_that_font_and_no_other() {
    // The two fonts are the same file but for a GSUB, so the only way to
    // prove the right one came back is to shape through it. `مرحبا` requires
    // five joins.
    let source = fixtures();

    let shaping = source.load("Mirsam Joining").unwrap().unwrap();
    assert_eq!(shaping.family, "Mirsam Joining");
    assert!(shaping.path.ends_with("joining.ttf"));
    assert_eq!(shape(&shaping.font().unwrap(), "مرحبا").joins_produced(), 5);

    let flat = source.load("Mirsam Nonjoining").unwrap().unwrap();
    assert_eq!(flat.family, "Mirsam Nonjoining");
    assert!(flat.path.ends_with("nonjoining.ttf"));
    assert_eq!(shape(&flat.font().unwrap(), "مرحبا").joins_produced(), 0);
}

#[test]
fn a_family_spread_over_several_files_resolves_to_its_regular_face() {
    // `bold.ttf` states `Mirsam Joining` too, sorts before `joining.ttf`, and
    // has no GSUB. A source picking by filename answers with it and shapes
    // nothing — which is what taking the first `Arial` on macOS does.
    let file = fixtures().load("Mirsam Joining").unwrap().unwrap();
    assert!(
        file.path.ends_with("joining.ttf"),
        "resolved to {} instead of the regular face",
        file.path
    );
    assert_eq!(shape(&file.font().unwrap(), "مرحبا").joins_produced(), 5);
}

#[test]
fn a_document_may_capitalise_a_family_however_it_likes() {
    let source = fixtures();
    for asked in [
        "Mirsam Partial",
        "mirsam partial",
        "MIRSAM PARTIAL",
        " Mirsam Partial ",
    ] {
        let file = source.load(asked).unwrap().expect(asked);
        // The family reported is the one the file states, not the one asked
        // for: a report has to name the font, not the request.
        assert_eq!(file.family, "Mirsam Partial", "asked for {asked:?}");
    }
}

#[test]
fn a_narrower_cut_of_a_family_is_a_different_font() {
    // `Mirsam Joining Condensed` is not `Mirsam Joining`. A source that
    // trimmed its way to a match would answer with a font the document never
    // named.
    assert_eq!(
        fixtures().load("Mirsam Joining Condensed").unwrap(),
        None,
        "matching must not fall back to a prefix"
    );
}

#[test]
fn a_font_this_machine_does_not_have_is_an_answer_not_an_error() {
    // The reportable state: the tool can no longer say what the reader will
    // see. It is not a failure to read anything.
    assert_eq!(fixtures().load("Traditional Arabic").unwrap(), None);
}

#[test]
fn the_resolved_font_is_the_one_coverage_judges() {
    // End to end, and the whole point of the port: a document names a
    // typeface, the machine answers with bytes, and the domain says what that
    // font will and will not draw. The fixtures map U+0621..U+064A, so
    // Persian peh is genuinely absent from the file that answered.
    let source = fixtures();
    let file = source.load("Mirsam Joining").unwrap().unwrap();
    let report = coverage(&file.font().unwrap(), "مرحبا پ");

    assert_eq!(report.checked, 6);
    assert_eq!(
        report
            .missing
            .iter()
            .map(|m| m.to_string())
            .collect::<Vec<_>>(),
        ["U+067E ARABIC LETTER PEH"]
    );
}
