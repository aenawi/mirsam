//! The OPC relationship graph: which part a part inherits from.
//!
//! PowerPoint does not store a paragraph's formatting in one place. A property
//! the paragraph leaves unset is taken from its placeholder on the slide
//! layout, from the layout's slide master, and finally from the master's
//! theme. Before any of that can be resolved, the chain itself has to be
//! *found*, and OOXML does not write it into the parts: it writes it into the
//! package's relationship items, one `_rels/<part>.rels` beside each part that
//! has outgoing edges.
//!
//! This module reads those items and answers one question — given a part,
//! which parts does it inherit from, in order. Reading the properties out of
//! them is 2.2; this is the graph they walk.
//!
//! ## A part's role is read from the graph, not from its name
//!
//! Every deck in the wild stores slides under `ppt/slides/`, but the OPC
//! specification does not require it, and a linter that decides what a part
//! *is* by matching its directory is a linter that silently reports nothing on
//! the first package that names things differently.
//!
//! So a part's [`Role`] comes from the relationship type that points *at* it:
//! the part `ppt/presentation.xml` reaches with a `slide` relationship is a
//! slide, whatever it is called. That also settles an ambiguity a downward
//! walk cannot: a slide master relates to its layouts *and* its theme, so
//! "follow the first relationship upward" would walk from a master back down
//! into a layout. Knowing the role first makes each hop exact — a master's one
//! step up is its `theme`, and nothing else is considered.
//!
//! ## Targets resolve to stored item names
//!
//! A relationship target is a URI reference relative to the part that carries
//! it, so `../slideLayouts/slideLayout1.xml` from `ppt/slides/slide1.xml.rels`
//! is `ppt/slideLayouts/slideLayout1.xml`. It may also be percent-encoded —
//! the torture deck's image is `../media/my%20image.png` — and so may the ZIP
//! item name it points at. A resolved target is therefore kept in the form the
//! package actually stores, because the only useful thing to do with it is
//! hand it back to [`Package::read_text`], which matches item names literally.
//! The decoded form is tried only when the encoded one is not in the package.

use mirsam_core::error::{Error, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::collections::{BTreeMap, BTreeSet};

use crate::package::Package;

/// The namespace every relationship type named in this module lives under.
///
/// Matched in full rather than by its last segment: `hdphoto`, `chartTrackingRefBased`
/// and other Microsoft extensions live under a different namespace with the
/// same shape, and a suffix match would one day accept one of them as a
/// standard type.
const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

/// The key under which the package's own `_rels/.rels` is filed: it describes
/// the package root, which is not a part and so has no part name.
pub const PACKAGE_ROOT: &str = "";

/// What a relationship points at.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    /// A part of this package, named as the package stores it.
    Part(String),
    /// A URI outside the package. Never resolved, never read.
    External(String),
}

/// One `<Relationship>` entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Relationship {
    id: String,
    relationship_type: String,
    target: Target,
}

impl Relationship {
    /// The `r:id` a part uses to refer to this relationship.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The relationship type, in full.
    pub fn relationship_type(&self) -> &str {
        &self.relationship_type
    }

    /// The type's last segment, for a type in the standard namespace:
    /// `slideLayout`, `theme`, `image`. `None` for any other namespace.
    pub fn kind(&self) -> Option<&str> {
        self.relationship_type.strip_prefix(OFFICE)
    }

    /// Whether this is a standard relationship of the given kind.
    pub fn is(&self, kind: &str) -> bool {
        self.kind() == Some(kind)
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The part this points at, or `None` when the target is external.
    pub fn part(&self) -> Option<&str> {
        match &self.target {
            Target::Part(name) => Some(name),
            Target::External(_) => None,
        }
    }
}

/// Every relationship one part declares, in the order its `.rels` lists them.
#[derive(Debug, Clone, Default)]
pub struct PartRelationships {
    source: String,
    items: Vec<Relationship>,
}

impl PartRelationships {
    /// Parse one `.rels` item.
    ///
    /// `rels_part` is the item's own name — `ppt/slides/_rels/slide1.xml.rels`
    /// — because a target resolves against the directory of the part being
    /// described, which is that name with the `_rels` segment removed.
    ///
    /// A malformed entry is skipped rather than failing the parse: a
    /// relationship with no target is one edge this graph does not have, and
    /// refusing to read the rest of a deck over it would turn a defect the
    /// tool could report into a document it cannot open.
    pub fn parse(rels_part: &str, xml: &str) -> Result<Self> {
        let dir = source_dir(rels_part);
        let mut reader = Reader::from_str(xml);
        let mut items = Vec::new();

        loop {
            match reader.read_event() {
                Err(e) => return Err(Error::Format(format!("{rels_part}: {e}"))),
                Ok(Event::Eof) => break,
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    if e.local_name().as_ref() != "Relationship" {
                        continue;
                    }
                    let (Some(id), Some(relationship_type), Some(raw)) = (
                        attribute(&e, "Id"),
                        attribute(&e, "Type"),
                        attribute(&e, "Target"),
                    ) else {
                        continue;
                    };
                    let external = attribute(&e, "TargetMode").as_deref() == Some("External");
                    let target = if external {
                        Target::External(raw)
                    } else {
                        Target::Part(resolve(&dir, &raw))
                    };
                    items.push(Relationship {
                        id,
                        relationship_type,
                        target,
                    });
                }
                Ok(_) => {}
            }
        }

        Ok(Self {
            source: source_part(rels_part),
            items,
        })
    }

    /// The part these relationships belong to. [`PACKAGE_ROOT`] for `_rels/.rels`.
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn iter(&self) -> impl Iterator<Item = &Relationship> {
        self.items.iter()
    }

    /// The relationship a part refers to by `r:id`.
    pub fn by_id(&self, id: &str) -> Option<&Relationship> {
        self.items.iter().find(|r| r.id == id)
    }

    /// Every standard relationship of one kind, in document order.
    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Relationship> + 'a {
        self.items.iter().filter(move |r| r.is(kind))
    }

    /// The first part reached by a standard relationship of one kind.
    ///
    /// First rather than only: a slide master lists eleven `slideLayout`
    /// relationships. For the kinds this graph walks upward — one layout per
    /// slide, one master per layout, one theme per master — there is exactly
    /// one, and taking the first is taking the one.
    pub fn first_part_of_kind(&self, kind: &str) -> Option<&str> {
        self.items
            .iter()
            .filter(|r| r.is(kind))
            .find_map(Relationship::part)
    }

    /// Re-point targets at the names the package actually stores.
    ///
    /// A target and the item it names are normally encoded the same way, and
    /// then this changes nothing. When they are not, the decoded form is the
    /// one that can be read, so it wins. A target matching neither is left as
    /// resolved: it is a dangling relationship, and saying so with the name it
    /// asked for is more useful than silently rewriting it.
    fn realign(&mut self, known: &BTreeSet<&str>) {
        for item in &mut self.items {
            if let Target::Part(name) = &item.target
                && !known.contains(name.as_str())
            {
                let decoded = percent_decode(name);
                if known.contains(decoded.as_str()) {
                    item.target = Target::Part(decoded);
                }
            }
        }
    }
}

/// What a part is, as the graph reaches it.
///
/// Ordered so that a part reached by two different relationship types — which
/// no well-formed package produces — resolves to the same role on every run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Role {
    /// The package's main story: a presentation, or a Word document. Named for
    /// the relationship that reaches it rather than for either format, because
    /// one relationship type reaches both and a `Presentation` role on
    /// `word/document.xml` would be the graph misreporting what it read.
    OfficeDocument,
    Slide,
    SlideLayout,
    SlideMaster,
    NotesSlide,
    NotesMaster,
    HandoutMaster,
    Theme,
    /// Reached by no relationship, or by one whose type this graph does not
    /// place in the inheritance chain — an image, a chart, an embedding.
    #[default]
    Other,
}

impl Role {
    /// The role a part takes from the relationship type pointing at it.
    fn from_kind(kind: &str) -> Self {
        match kind {
            "officeDocument" => Self::OfficeDocument,
            "slide" => Self::Slide,
            "slideLayout" => Self::SlideLayout,
            "slideMaster" => Self::SlideMaster,
            "notesSlide" => Self::NotesSlide,
            "notesMaster" => Self::NotesMaster,
            "handoutMaster" => Self::HandoutMaster,
            "theme" => Self::Theme,
            _ => Self::Other,
        }
    }

    /// The relationship kind that leads one step up the inheritance chain.
    ///
    /// `None` ends the walk. A theme is the top; the main document part is
    /// beside the chain rather than above it — it *owns* the masters, and a
    /// property is never inherited from it by this route. Word's own chain is
    /// not walked here at all: a paragraph style is named inside
    /// `word/styles.xml` rather than by a relationship, so it is
    /// [`crate::style`]'s to resolve and not the graph's.
    pub fn inherits_from(self) -> Option<&'static str> {
        match self {
            Self::Slide => Some("slideLayout"),
            Self::SlideLayout | Self::HandoutMaster => Some("slideMaster"),
            Self::NotesSlide => Some("notesMaster"),
            Self::SlideMaster | Self::NotesMaster => Some("theme"),
            Self::OfficeDocument | Self::Theme | Self::Other => None,
        }
    }
}

/// Every relationship in a package, and the role each part plays.
#[derive(Debug, Clone, Default)]
pub struct RelationshipGraph {
    by_source: BTreeMap<String, PartRelationships>,
    roles: BTreeMap<String, Role>,
}

impl RelationshipGraph {
    /// Read every `.rels` item in a package.
    pub fn read(package: &Package) -> Result<Self> {
        let names = package.part_names()?;
        let known: BTreeSet<&str> = names.iter().map(String::as_str).collect();

        let mut by_source = BTreeMap::new();
        for rels_part in names.iter().filter(|n| n.ends_with(".rels")) {
            let xml = package.read_text(rels_part)?;
            let mut rels = PartRelationships::parse(rels_part, &xml)?;
            rels.realign(&known);
            by_source.insert(rels.source.clone(), rels);
        }

        Ok(Self {
            roles: roles_from(&by_source),
            by_source,
        })
    }

    /// Build a graph from already-parsed items. The package-reading path and
    /// the test path go through the same role inference this way.
    pub fn from_parts(parts: impl IntoIterator<Item = PartRelationships>) -> Self {
        let by_source: BTreeMap<String, PartRelationships> = parts
            .into_iter()
            .map(|rels| (rels.source.clone(), rels))
            .collect();
        Self {
            roles: roles_from(&by_source),
            by_source,
        }
    }

    /// The relationships one part declares, if it declares any.
    pub fn of(&self, part: &str) -> Option<&PartRelationships> {
        self.by_source.get(part)
    }

    /// Resolve one `r:id` against the part that used it.
    pub fn target_of(&self, part: &str, id: &str) -> Option<&str> {
        self.of(part)?.by_id(id)?.part()
    }

    /// The first part `part` reaches by a standard relationship of one kind.
    pub fn first_part_of_kind(&self, part: &str, kind: &str) -> Option<&str> {
        self.of(part)?.first_part_of_kind(kind)
    }

    /// What the graph reaches this part as.
    pub fn role_of(&self, part: &str) -> Role {
        self.roles.get(part).copied().unwrap_or_default()
    }

    /// Every part with one role, in name order.
    pub fn parts_with_role(&self, role: Role) -> Vec<&str> {
        self.roles
            .iter()
            .filter(|(_, r)| **r == role)
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// The main document part, reached from the package root: a presentation
    /// in a `.pptx`, `word/document.xml` in a `.docx`.
    pub fn office_document(&self) -> Option<&str> {
        self.first_part_of_kind(PACKAGE_ROOT, "officeDocument")
    }

    /// The parts `part` inherits from, nearest first, excluding `part` itself.
    ///
    /// A slide gives its layout, that layout's master and that master's theme.
    /// A part with no role in the chain gives nothing — an image inherits no
    /// paragraph properties, and inventing a chain for it would be a guess.
    ///
    /// A package whose relationships form a cycle terminates the walk at the
    /// repeat rather than looping: a malformed deck is a deck to report on,
    /// not one to hang on.
    pub fn ancestors(&self, part: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut seen = BTreeSet::new();
        seen.insert(part.to_string());
        let mut here = part.to_string();

        while let Some(kind) = self.role_of(&here).inherits_from() {
            let Some(next) = self
                .first_part_of_kind(&here, kind)
                .map(str::to_string)
                .filter(|next| seen.insert(next.clone()))
            else {
                break;
            };
            chain.push(next.clone());
            here = next;
        }
        chain
    }

    /// `part` followed by everything it inherits from.
    pub fn inheritance_chain(&self, part: &str) -> Vec<String> {
        let mut chain = vec![part.to_string()];
        chain.extend(self.ancestors(part));
        chain
    }

    /// The slide layout in this part's chain.
    pub fn layout_of(&self, part: &str) -> Option<String> {
        self.ancestor_with_role(part, Role::SlideLayout)
    }

    /// The master in this part's chain — a slide master, or the notes master
    /// behind a notes slide.
    pub fn master_of(&self, part: &str) -> Option<String> {
        self.ancestor_with_role(part, Role::SlideMaster)
            .or_else(|| self.ancestor_with_role(part, Role::NotesMaster))
    }

    /// The theme at the top of this part's chain.
    pub fn theme_of(&self, part: &str) -> Option<String> {
        self.ancestor_with_role(part, Role::Theme)
    }

    fn ancestor_with_role(&self, part: &str, role: Role) -> Option<String> {
        self.ancestors(part)
            .into_iter()
            .find(|name| self.role_of(name) == role)
    }
}

/// Infer each part's role from the relationship types pointing at it.
///
/// The lowest-ordered role wins when a part is reached more than one way, so
/// the result does not depend on iteration order.
fn roles_from(by_source: &BTreeMap<String, PartRelationships>) -> BTreeMap<String, Role> {
    let mut roles: BTreeMap<String, Role> = BTreeMap::new();
    for rels in by_source.values() {
        for item in rels.iter() {
            let (Some(kind), Some(part)) = (item.kind(), item.part()) else {
                continue;
            };
            let role = Role::from_kind(kind);
            if role == Role::Other {
                continue;
            }
            roles
                .entry(part.to_string())
                .and_modify(|existing| *existing = (*existing).min(role))
                .or_insert(role);
        }
    }
    roles
}

/// Read an attribute's normalised value, the same way the part scanners do.
fn attribute(tag: &BytesStart, name: &str) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

/// The `.rels` item that describes a part.
///
/// `ppt/slides/slide1.xml` is described by
/// `ppt/slides/_rels/slide1.xml.rels`, and the package root by `_rels/.rels`.
pub fn rels_part_for(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// The part a `.rels` item describes: the inverse of [`rels_part_for`].
fn source_part(rels_part: &str) -> String {
    let file = rels_part
        .rsplit_once('/')
        .map_or(rels_part, |(_, file)| file)
        .strip_suffix(".rels")
        .unwrap_or_default();
    match source_dir(rels_part) {
        dir if dir.is_empty() => file.to_string(),
        dir => format!("{dir}/{file}"),
    }
}

/// The directory a `.rels` item's targets resolve against: the directory of
/// the part it describes, which is its own directory with `_rels` removed.
fn source_dir(rels_part: &str) -> String {
    let dir = rels_part.rsplit_once('/').map_or("", |(dir, _)| dir);
    dir.strip_suffix("_rels")
        .map_or(dir, |head| head.trim_end_matches('/'))
        .to_string()
}

/// Resolve a relationship target against the directory it was declared in.
///
/// A target beginning with `/` is package-absolute; everything else is
/// relative, with `.` and `..` collapsed. The result carries no leading
/// slash, because that is how a ZIP item is named.
fn resolve(source_dir: &str, target: &str) -> String {
    let (base, rest) = match target.strip_prefix('/') {
        Some(absolute) => ("", absolute),
        None => (source_dir, target),
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in base.split('/').chain(rest.split('/')) {
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

/// Percent-decode a target, for the package that encodes its `.rels` and its
/// item names differently. Invalid escapes are left as written.
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

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"";

    fn rels(entries: &str) -> String {
        format!("<?xml version=\"1.0\"?><Relationships {NS}>{entries}</Relationships>")
    }

    fn entry(id: &str, kind: &str, target: &str) -> String {
        format!("<Relationship Id=\"{id}\" Type=\"{OFFICE}{kind}\" Target=\"{target}\"/>")
    }

    /// A three-part deck: one slide, its layout, that layout's master, and the
    /// master's theme, plus the package root pointing at a presentation.
    fn deck() -> RelationshipGraph {
        let parts = [
            (
                "_rels/.rels",
                rels(&entry("rId1", "officeDocument", "ppt/presentation.xml")),
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                rels(&format!(
                    "{}{}",
                    entry("rId1", "slideMaster", "slideMasters/slideMaster1.xml"),
                    entry("rId2", "slide", "slides/slide1.xml"),
                )),
            ),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                rels(&entry(
                    "rId1",
                    "slideLayout",
                    "../slideLayouts/slideLayout1.xml",
                )),
            ),
            (
                "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
                rels(&entry(
                    "rId1",
                    "slideMaster",
                    "../slideMasters/slideMaster1.xml",
                )),
            ),
            (
                "ppt/slideMasters/_rels/slideMaster1.xml.rels",
                rels(&format!(
                    "{}{}",
                    entry("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"),
                    entry("rId2", "theme", "../theme/theme1.xml"),
                )),
            ),
        ];
        RelationshipGraph::from_parts(
            parts
                .iter()
                .map(|(name, xml)| PartRelationships::parse(name, xml).unwrap()),
        )
    }

    #[test]
    fn a_rels_item_names_the_part_it_describes() {
        assert_eq!(
            source_part("ppt/slides/_rels/slide1.xml.rels"),
            "ppt/slides/slide1.xml"
        );
        assert_eq!(
            source_part("ppt/_rels/presentation.xml.rels"),
            "ppt/presentation.xml"
        );
        assert_eq!(source_part("_rels/.rels"), PACKAGE_ROOT);
    }

    #[test]
    fn naming_a_rels_item_is_the_inverse_of_reading_one() {
        for part in [
            "ppt/slides/slide1.xml",
            "ppt/presentation.xml",
            "[Content_Types].xml",
        ] {
            assert_eq!(source_part(&rels_part_for(part)), part);
        }
    }

    #[test]
    fn a_target_resolves_against_the_part_that_declared_it() {
        assert_eq!(
            resolve("ppt/slides", "../slideLayouts/slideLayout1.xml"),
            "ppt/slideLayouts/slideLayout1.xml"
        );
        assert_eq!(resolve("ppt", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(resolve("", "ppt/presentation.xml"), "ppt/presentation.xml");
        // Package-absolute, and a `.` segment that means nothing.
        assert_eq!(
            resolve("ppt/slides", "/ppt/theme/theme1.xml"),
            "ppt/theme/theme1.xml"
        );
        assert_eq!(
            resolve("ppt/slides", "./slide2.xml"),
            "ppt/slides/slide2.xml"
        );
    }

    #[test]
    fn an_external_target_is_never_resolved_to_a_part() {
        let xml = rels(&format!(
            "<Relationship Id=\"rId1\" Type=\"{OFFICE}hyperlink\" \
             Target=\"https://example.com/a\" TargetMode=\"External\"/>"
        ));
        let parsed = PartRelationships::parse("ppt/slides/_rels/slide1.xml.rels", &xml).unwrap();
        let link = parsed.by_id("rId1").unwrap();
        assert_eq!(link.part(), None);
        assert_eq!(
            link.target(),
            &Target::External("https://example.com/a".into())
        );
    }

    #[test]
    fn a_type_outside_the_standard_namespace_has_no_kind() {
        let xml = rels(
            "<Relationship Id=\"rId1\" \
             Type=\"http://schemas.microsoft.com/office/2007/relationships/slideLayout\" \
             Target=\"../slideLayouts/slideLayout1.xml\"/>",
        );
        let parsed = PartRelationships::parse("ppt/slides/_rels/slide1.xml.rels", &xml).unwrap();
        let item = parsed.by_id("rId1").unwrap();
        assert_eq!(item.kind(), None);
        assert!(!item.is("slideLayout"));
        assert_eq!(parsed.first_part_of_kind("slideLayout"), None);
    }

    #[test]
    fn a_parts_role_comes_from_the_relationship_that_reaches_it() {
        let graph = deck();
        assert_eq!(graph.role_of("ppt/presentation.xml"), Role::OfficeDocument);
        assert_eq!(graph.role_of("ppt/slides/slide1.xml"), Role::Slide);
        assert_eq!(
            graph.role_of("ppt/slideLayouts/slideLayout1.xml"),
            Role::SlideLayout
        );
        assert_eq!(
            graph.role_of("ppt/slideMasters/slideMaster1.xml"),
            Role::SlideMaster
        );
        assert_eq!(graph.role_of("ppt/theme/theme1.xml"), Role::Theme);
        // Nothing points at it, so the graph claims nothing about it.
        assert_eq!(graph.role_of("ppt/media/image1.png"), Role::Other);
    }

    #[test]
    fn a_slide_inherits_from_its_layout_then_master_then_theme() {
        let graph = deck();
        assert_eq!(
            graph.inheritance_chain("ppt/slides/slide1.xml"),
            [
                "ppt/slides/slide1.xml",
                "ppt/slideLayouts/slideLayout1.xml",
                "ppt/slideMasters/slideMaster1.xml",
                "ppt/theme/theme1.xml",
            ]
        );
        assert_eq!(
            graph.layout_of("ppt/slides/slide1.xml").as_deref(),
            Some("ppt/slideLayouts/slideLayout1.xml")
        );
        assert_eq!(
            graph.master_of("ppt/slides/slide1.xml").as_deref(),
            Some("ppt/slideMasters/slideMaster1.xml")
        );
        assert_eq!(
            graph.theme_of("ppt/slides/slide1.xml").as_deref(),
            Some("ppt/theme/theme1.xml")
        );
    }

    #[test]
    fn a_master_walks_up_to_its_theme_and_never_down_to_a_layout() {
        // The master lists its layout *before* its theme, so a walk that
        // followed the first relationship rather than the role would descend.
        let graph = deck();
        assert_eq!(
            graph.ancestors("ppt/slideMasters/slideMaster1.xml"),
            ["ppt/theme/theme1.xml"]
        );
    }

    #[test]
    fn the_presentation_is_reached_from_the_package_root() {
        assert_eq!(deck().office_document(), Some("ppt/presentation.xml"));
    }

    #[test]
    fn a_part_outside_the_chain_inherits_nothing() {
        let graph = deck();
        assert!(graph.ancestors("ppt/media/image1.png").is_empty());
        assert_eq!(
            graph.inheritance_chain("ppt/media/image1.png"),
            ["ppt/media/image1.png"]
        );
    }

    #[test]
    fn a_cycle_in_the_relationships_terminates_the_walk() {
        // Two layouts naming each other as master: malformed, and the kind of
        // thing this tool exists to survive rather than hang on.
        let parts = [
            (
                "_rels/.rels",
                rels(&entry("rId1", "officeDocument", "ppt/presentation.xml")),
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                rels(&entry("rId1", "slideLayout", "a.xml")),
            ),
            (
                "ppt/_rels/a.xml.rels",
                rels(&entry("rId1", "slideMaster", "b.xml")),
            ),
            (
                "ppt/_rels/b.xml.rels",
                rels(&entry("rId1", "theme", "a.xml")),
            ),
        ];
        let graph = RelationshipGraph::from_parts(
            parts
                .iter()
                .map(|(name, xml)| PartRelationships::parse(name, xml).unwrap()),
        );
        // `b.xml` names `a.xml` as its theme; the walk stops at the repeat
        // rather than emitting it a second time.
        assert_eq!(graph.ancestors("ppt/a.xml"), ["ppt/b.xml"]);
    }

    #[test]
    fn a_relationship_missing_an_attribute_is_skipped_not_fatal() {
        let xml = rels(&format!(
            "<Relationship Id=\"rId1\" Type=\"{OFFICE}slideLayout\"/>{}",
            entry("rId2", "theme", "../theme/theme1.xml")
        ));
        let parsed = PartRelationships::parse("ppt/slides/_rels/slide1.xml.rels", &xml).unwrap();
        assert_eq!(parsed.iter().count(), 1);
        assert_eq!(
            parsed.first_part_of_kind("theme"),
            Some("ppt/theme/theme1.xml")
        );
    }

    #[test]
    fn a_target_keeps_the_encoding_the_package_stores() {
        let xml = rels(&entry("rId1", "image", "../media/my%20image.png"));
        let mut parsed =
            PartRelationships::parse("ppt/slides/_rels/slide1.xml.rels", &xml).unwrap();

        // The package stores the encoded name, as the torture deck does.
        parsed.realign(&BTreeSet::from(["ppt/media/my%20image.png"]));
        assert_eq!(
            parsed.first_part_of_kind("image"),
            Some("ppt/media/my%20image.png")
        );

        // A package that stores the decoded name gets the decoded name.
        let mut parsed =
            PartRelationships::parse("ppt/slides/_rels/slide1.xml.rels", &xml).unwrap();
        parsed.realign(&BTreeSet::from(["ppt/media/my image.png"]));
        assert_eq!(
            parsed.first_part_of_kind("image"),
            Some("ppt/media/my image.png")
        );
    }

    #[test]
    fn a_dangling_target_keeps_the_name_it_asked_for() {
        let xml = rels(&entry("rId1", "image", "../media/gone.png"));
        let mut parsed =
            PartRelationships::parse("ppt/slides/_rels/slide1.xml.rels", &xml).unwrap();
        parsed.realign(&BTreeSet::from(["ppt/media/other.png"]));
        assert_eq!(
            parsed.first_part_of_kind("image"),
            Some("ppt/media/gone.png")
        );
    }
}
