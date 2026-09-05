//! Is every corpus document a package an application would open?
//!
//! The M1 application check — "PowerPoint opens the file without offering to
//! repair it" — cannot be run in CI, and for a while it could not be run at
//! all: three of the five corpus decks were hand-built XML that PowerPoint
//! asked to repair *before* mirsam touched them, which made the check
//! unanswerable on them either way
//! ([#9](https://github.com/aenawi/mirsam/issues/9)).
//!
//! This asserts the structural half of "would open", over the committed
//! documents rather than the generator that produced them, because the
//! generator does not run in CI and the documents are what the rest of the
//! suite reads. Each invariant below is one that a document in this corpus
//! actually violated.
//!
//! Three are the OPC container's and hold for every packaged format the corpus
//! carries — Word's `.docx` and Excel's `.xlsx` included, because the package
//! layer is the one those formats reuse rather than reimplement, and a check
//! written only against `.pptx` would leave most of what it guards untested:
//!
//! * every part is declared in `[Content_Types].xml`
//! * every relationship resolves to a part that is present
//! * every item name is percent-encoded ASCII
//!
//! The rest name PresentationML elements and so are asked of the decks alone:
//!
//! * every `p:spTree` carries the `p:grpSpPr` the schema requires after
//!   `p:nvGrpSpPr`
//! * a deck with a notes slide has the notes master it inherits from, and
//!   relates to it
//! * a theme carries all three of `clrScheme`, `fontScheme` and `fmtScheme`
//!
//! Schema validity as a whole is a stronger claim than this, and one this
//! suite deliberately does not make: it needs the published ECMA-376 XSDs,
//! which are not vendored. `scripts/validate-ooxml.py` makes it, on demand.

use mirsam_ooxml::Package;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Every corpus document of the given extensions, in name order. `*.out.*` is
/// a local repair output, not a member of the corpus — the same rule
/// `tests/golden.rs` applies.
fn corpus(extensions: &[&str]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|e| extensions.contains(&&*e.to_string_lossy()))
        })
        .filter(|path| !name_of(path).contains(".out."))
        .collect();
    found.sort();
    assert!(
        !found.is_empty(),
        "no corpus document with an extension in {extensions:?} under {:?}",
        fixtures()
    );
    found
}

/// Every corpus document, whatever format it is: what the OPC-level checks ask.
fn documents() -> Vec<PathBuf> {
    corpus(&["pptx", "docx", "xlsx"])
}

/// The presentations alone: what the PresentationML checks ask.
fn decks() -> Vec<PathBuf> {
    corpus(&["pptx"])
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// Every value of `attribute` in `xml`, in document order.
///
/// Attribute-level scanning rather than a parse: these are generated parts
/// with one quoting style, and the assertions below are about which strings
/// are present, not about structure.
fn attribute_values(xml: &str, attribute: &str) -> Vec<String> {
    let needle = format!("{attribute}=\"");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&needle) {
        rest = &rest[at + needle.len()..];
        match rest.find('"') {
            Some(end) => {
                out.push(rest[..end].to_string());
                rest = &rest[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// Percent-decode a relationship target or a ZIP item name, so the two
/// compare as part names. `ppt/media/my%20image.png` in the torture deck
/// is stored encoded on both sides.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve a relationship target against the directory its `.rels` describes.
fn resolve(rels_part: &str, target: &str) -> String {
    // "ppt/slides/_rels/slide1.xml.rels" describes "ppt/slides".
    let source_dir = rels_part
        .rsplit_once("/_rels/")
        .map(|(dir, _)| dir)
        .unwrap_or("");
    let decoded = percent_decode(target);
    let mut segments: Vec<&str> = Vec::new();
    for segment in source_dir.split('/').chain(decoded.split('/')) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

#[test]
fn every_part_of_every_corpus_document_is_declared_in_content_types() {
    for document in documents() {
        let pkg = Package::open(&document).unwrap();
        let types = pkg.read_text("[Content_Types].xml").unwrap();
        let defaults: Vec<String> = attribute_values(&types, "Extension")
            .into_iter()
            .map(|e| e.to_lowercase())
            .collect();
        let overrides = attribute_values(&types, "PartName");

        for part in pkg.part_names().unwrap() {
            if part == "[Content_Types].xml" {
                continue;
            }
            let extension = part.rsplit_once('.').map(|(_, e)| e.to_lowercase());
            let declared = overrides.contains(&format!("/{part}"))
                || extension.is_some_and(|e| defaults.contains(&e));
            assert!(
                declared,
                "{}: {part} has no content type; a consumer does not know what it is",
                name_of(&document)
            );
        }
    }
}

#[test]
fn every_relationship_of_every_corpus_document_resolves() {
    for document in documents() {
        let pkg = Package::open(&document).unwrap();
        let names: Vec<String> = pkg
            .part_names()
            .unwrap()
            .iter()
            .map(|n| percent_decode(n))
            .collect();
        for rels_part in pkg.parts_where(|n| n.ends_with(".rels")).unwrap() {
            let body = pkg.read_text(&rels_part).unwrap();
            // An external target names a URI, not a part.
            assert!(
                !body.contains("TargetMode=\"External\""),
                "{}: {rels_part} has an external target this check does not model",
                name_of(&document)
            );
            for target in attribute_values(&body, "Target") {
                let resolved = resolve(&rels_part, &target);
                assert!(
                    names.contains(&resolved),
                    "{}: {rels_part} points at {target}, which is not in the package",
                    name_of(&document)
                );
            }
        }
    }
}

#[test]
fn every_item_name_of_every_corpus_document_is_ascii() {
    // PowerPoint 2016 does not resolve a relationship to a part whose name
    // carries a non-ASCII octet — raw or percent-encoded, Arabic or Latin —
    // and offers to repair the deck. That was the whole of what made
    // `torture.pptx` prompt (#9). A corpus document must be one an application
    // opens, so no part name here may leave ASCII.
    for document in documents() {
        let pkg = Package::open(&document).unwrap();
        for name in pkg.part_names().unwrap() {
            assert!(
                name.is_ascii(),
                "{}: item {name} is not percent-encoded ASCII",
                name_of(&document)
            );
        }
    }
}

#[test]
fn every_shape_tree_carries_the_group_properties_the_schema_requires() {
    // `p:spTree` is `p:nvGrpSpPr` then `p:grpSpPr`, both required. Omitting
    // the second is what made the hand-built decks prompt for repair.
    let mut trees = 0usize;
    for deck in decks() {
        let pkg = Package::open(&deck).unwrap();
        for part in pkg
            .parts_where(|n| n.starts_with("ppt/") && n.ends_with(".xml"))
            .unwrap()
        {
            let xml = pkg.read_text(&part).unwrap();
            for tree in xml.split("<p:spTree>").skip(1) {
                let tree = tree.split("</p:spTree>").next().unwrap_or(tree);
                trees += 1;
                assert!(
                    tree.contains("<p:grpSpPr"),
                    "{}: {part} has a p:spTree with no p:grpSpPr",
                    name_of(&deck)
                );
            }
        }
    }
    assert!(
        trees > 0,
        "no shape tree was examined; the check is vacuous"
    );
}

#[test]
fn a_deck_with_speaker_notes_has_the_notes_master_they_inherit_from() {
    // The notes slide's own relationship to the master is the load-bearing
    // part: `quarterly-report.pptx`, which PowerPoint opens without a prompt,
    // has the master and the relationship but no `p:notesMasterIdLst`. So
    // this asserts what that deck demonstrates is enough, and no more.
    let mut checked = 0usize;
    for deck in decks() {
        let pkg = Package::open(&deck).unwrap();
        let names = pkg.part_names().unwrap();
        let notes = pkg
            .parts_where(|n| n.starts_with("ppt/notesSlides/") && n.ends_with(".xml"))
            .unwrap();
        if notes.is_empty() {
            continue;
        }
        checked += 1;
        assert!(
            names.iter().any(|n| n.starts_with("ppt/notesMasters/")),
            "{}: a notes slide with no notes master part",
            name_of(&deck)
        );
        for note in notes {
            let (dir, file) = note.rsplit_once('/').unwrap();
            let rels_part = format!("{dir}/_rels/{file}.rels");
            let body = pkg.read_text(&rels_part).unwrap_or_default();
            assert!(
                body.contains("/notesMaster\""),
                "{}: {note} does not relate to a notes master",
                name_of(&deck)
            );
        }
    }
    assert!(
        checked > 0,
        "no deck in the corpus carries speaker notes; the check is vacuous"
    );
}

#[test]
fn every_theme_carries_all_three_of_its_required_schemes() {
    let mut themes = 0usize;
    for deck in decks() {
        let pkg = Package::open(&deck).unwrap();
        for part in pkg
            .parts_where(|n| n.starts_with("ppt/theme/") && n.ends_with(".xml"))
            .unwrap()
        {
            let xml = pkg.read_text(&part).unwrap();
            themes += 1;
            for scheme in ["<a:clrScheme", "<a:fontScheme", "<a:fmtScheme"] {
                assert!(
                    xml.contains(scheme),
                    "{}: {part} has no {scheme}; a:themeElements requires all three",
                    name_of(&deck)
                );
            }
        }
    }
    assert!(themes > 0, "no theme was examined; the check is vacuous");
}
