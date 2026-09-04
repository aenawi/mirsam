//! Does the relationship graph agree with the decks the rest of the suite reads?
//!
//! `rels.rs` has unit tests over synthetic packages, which prove the walk is
//! the walk that was designed. They cannot prove it is the walk PowerPoint
//! writes — that claim needs real decks, and the corpus is five of them,
//! including two produced by PowerPoint itself.
//!
//! Two of the assertions here are the ones the design rests on:
//!
//! * every slide reaches a layout, a master and a theme, and each of those is
//!   a part the package can actually read — the M2 chain, end to end;
//! * the role the graph infers from incoming relationships matches the role
//!   the part's directory implies, on every part of every deck. The graph does
//!   not read directory names, so this is an independent check of the
//!   inference rather than a restatement of it. It is also the check that
//!   would fail first if the inference were ever quietly replaced by a name
//!   match.

use mirsam_ooxml::rels::{PACKAGE_ROOT, RelationshipGraph, Role, Target, rels_part_for};
use mirsam_ooxml::{Package, PptxDocument};
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

/// Every corpus deck, in name order. `*.out.*` is a local repair output, not
/// a member of the corpus — the same rule `tests/golden.rs` applies.
fn decks() -> Vec<PathBuf> {
    let mut decks: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "pptx"))
        .filter(|path| !name_of(path).contains(".out."))
        .collect();
    decks.sort();
    assert!(!decks.is_empty(), "no corpus decks under {:?}", fixtures());
    decks
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// The role a part's directory implies, for the parts PowerPoint places by
/// convention. Used only to cross-check the graph's own inference.
fn role_by_convention(part: &str) -> Option<Role> {
    let by_prefix = [
        ("ppt/slides/", Role::Slide),
        ("ppt/slideLayouts/", Role::SlideLayout),
        ("ppt/slideMasters/", Role::SlideMaster),
        ("ppt/notesSlides/", Role::NotesSlide),
        ("ppt/notesMasters/", Role::NotesMaster),
        ("ppt/handoutMasters/", Role::HandoutMaster),
        ("ppt/theme/", Role::Theme),
    ];
    if !part.ends_with(".xml") || part.contains("/_rels/") {
        return None;
    }
    if part == "ppt/presentation.xml" {
        return Some(Role::OfficeDocument);
    }
    by_prefix
        .iter()
        .find(|(prefix, _)| part.starts_with(prefix))
        .map(|(_, role)| *role)
}

#[test]
fn every_deck_reaches_its_presentation_from_the_package_root() {
    for deck in decks() {
        let pkg = Package::open(&deck).unwrap();
        let graph = RelationshipGraph::read(&pkg).unwrap();
        let presentation = graph
            .office_document()
            .unwrap_or_else(|| panic!("{}: no officeDocument relationship", name_of(&deck)));
        assert!(
            pkg.read_text(presentation).is_ok(),
            "{}: the presentation part {presentation} cannot be read",
            name_of(&deck)
        );
        assert_eq!(
            graph.of(PACKAGE_ROOT).map(|r| r.source()),
            Some(PACKAGE_ROOT),
            "{}: _rels/.rels is not filed under the package root",
            name_of(&deck)
        );
    }
}

#[test]
fn every_slide_of_every_deck_inherits_layout_then_master_then_theme() {
    let mut slides = 0usize;
    for deck in decks() {
        let pkg = Package::open(&deck).unwrap();
        let graph = RelationshipGraph::read(&pkg).unwrap();

        for slide in graph.parts_with_role(Role::Slide) {
            slides += 1;
            let chain = graph.inheritance_chain(slide);
            let roles: Vec<Role> = chain.iter().map(|part| graph.role_of(part)).collect();
            assert_eq!(
                roles,
                [
                    Role::Slide,
                    Role::SlideLayout,
                    Role::SlideMaster,
                    Role::Theme
                ],
                "{}: {slide} inherits from {chain:?}",
                name_of(&deck)
            );
            for part in &chain {
                assert!(
                    pkg.read_text(part).is_ok(),
                    "{}: {slide} inherits from {part}, which cannot be read",
                    name_of(&deck)
                );
            }
            // The named accessors and the chain are one walk, not two.
            assert_eq!(graph.layout_of(slide).as_deref(), Some(chain[1].as_str()));
            assert_eq!(graph.master_of(slide).as_deref(), Some(chain[2].as_str()));
            assert_eq!(graph.theme_of(slide).as_deref(), Some(chain[3].as_str()));
        }
    }
    assert!(slides > 0, "no slide was examined; the check is vacuous");
}

#[test]
fn a_notes_slide_inherits_from_the_notes_master_and_its_theme() {
    let mut notes = 0usize;
    for deck in decks() {
        let pkg = Package::open(&deck).unwrap();
        let graph = RelationshipGraph::read(&pkg).unwrap();
        for note in graph.parts_with_role(Role::NotesSlide) {
            notes += 1;
            let master = graph
                .master_of(note)
                .unwrap_or_else(|| panic!("{}: {note} has no notes master", name_of(&deck)));
            assert_eq!(graph.role_of(&master), Role::NotesMaster);
            assert!(
                graph.theme_of(note).is_some(),
                "{}: {note} reaches no theme through {master}",
                name_of(&deck)
            );
        }
    }
    assert!(
        notes > 0,
        "no deck in the corpus carries speaker notes; the check is vacuous"
    );
}

#[test]
fn the_role_read_from_the_graph_is_the_role_the_layout_convention_implies() {
    let mut checked = 0usize;
    for deck in decks() {
        let pkg = Package::open(&deck).unwrap();
        let graph = RelationshipGraph::read(&pkg).unwrap();
        for part in pkg.part_names().unwrap() {
            let Some(expected) = role_by_convention(&part) else {
                continue;
            };
            checked += 1;
            assert_eq!(
                graph.role_of(&part),
                expected,
                "{}: {part} is reached as {:?}, but sits where a {expected:?} sits",
                name_of(&deck),
                graph.role_of(&part)
            );
        }
    }
    assert!(checked > 0, "no part was examined; the check is vacuous");
}

#[test]
fn every_resolved_target_names_a_part_the_package_can_read() {
    // The graph resolves to stored item names, so a target it hands back must
    // be readable without any further decoding. `torture.pptx` carries
    // `ppt/media/my%20image.png` on both sides of the relationship, which is
    // exactly the case a resolver that decoded names would break.
    let mut encoded = 0usize;
    for deck in decks() {
        let pkg = Package::open(&deck).unwrap();
        let graph = RelationshipGraph::read(&pkg).unwrap();
        for part in pkg.part_names().unwrap() {
            let Some(rels) = graph.of(&part) else {
                continue;
            };
            for item in rels.iter() {
                let Target::Part(target) = item.target() else {
                    continue;
                };
                if target.contains('%') {
                    encoded += 1;
                }
                assert!(
                    pkg.read_bytes(target).is_ok(),
                    "{}: {part} relates to {target}, which the package cannot read",
                    name_of(&deck)
                );
            }
        }
    }
    assert!(
        encoded > 0,
        "no percent-encoded target in the corpus; the encoding case is untested"
    );
}

#[test]
fn a_part_declaring_relationships_is_filed_under_its_own_name() {
    for deck in decks() {
        let pkg = Package::open(&deck).unwrap();
        let graph = RelationshipGraph::read(&pkg).unwrap();
        for rels_part in pkg.parts_where(|n| n.ends_with(".rels")).unwrap() {
            let source = graph
                .of(&source_of(&rels_part))
                .unwrap_or_else(|| panic!("{}: {rels_part} was not read", name_of(&deck)));
            assert_eq!(rels_part_for(source.source()), rels_part);
        }
    }
}

#[test]
fn an_open_document_hands_back_the_same_graph_the_package_does() {
    // The adapter is where 2.2 will reach for the chain, so the accessor is
    // part of the contract rather than a convenience.
    for deck in decks() {
        let doc = PptxDocument::open(&deck).unwrap();
        let graph = doc.relationships().unwrap();
        let direct = RelationshipGraph::read(doc.package()).unwrap();
        for slide in graph.parts_with_role(Role::Slide) {
            assert_eq!(
                graph.inheritance_chain(slide),
                direct.inheritance_chain(slide),
                "{}: the adapter's graph disagrees with the package's on {slide}",
                name_of(&deck)
            );
        }
    }
}

/// The part a `.rels` item describes, worked out here rather than borrowed
/// from the module under test, so the naming assertion is not circular.
fn source_of(rels_part: &str) -> String {
    match rels_part {
        "_rels/.rels" => PACKAGE_ROOT.to_string(),
        other => other.replace("/_rels/", "/").replace(".rels", ""),
    }
}
