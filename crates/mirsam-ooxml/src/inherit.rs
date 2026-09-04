//! Property chain resolution: what a paragraph inherits, and from where.
//!
//! PowerPoint stores a paragraph's formatting in up to four places, and the
//! paragraph itself is only the first of them. A property it leaves unset is
//! taken from the list style on its own shape, then from the matching
//! placeholder on the slide layout, then from the same placeholder on the
//! slide master, and finally from one of the master's three named text
//! styles. [`crate::rels`] found the parts; this module reads the properties
//! out of them and fills in what the paragraph did not say.
//!
//! ## What a placeholder resolves against
//!
//! A shape says which placeholder it is with `p:ph`, whose `@type` defaults to
//! `body` exactly as `ST_PlaceholderType` does — a layout's bare
//! `<p:ph idx="1"/>` is a body placeholder, not an untyped one. A slide's
//! placeholder is answered by the layout's with the same `@idx`, where an
//! absent `@idx` is index zero; the type has to agree as well, because a title
//! and a body placeholder both leave `@idx` off. `title` and `ctrTitle` are
//! one placeholder under two names and match each other.
//!
//! The last word is the master's [`TextStyle`]: `p:titleStyle` for a title
//! placeholder, `p:bodyStyle` for a body one, and `p:otherStyle` for
//! everything else — including every shape that is not a placeholder at all,
//! which is what a text box, a table cell and a chart's fallback drawing are.
//! A notes master states one `p:notesStyle` for everything it lays out, and a
//! placeholder there resolves against it whatever its type.
//!
//! ## What is resolved, and what is not
//!
//! Direction and alignment. [ADR 0007] decides what to conclude from an
//! inherited value by asking whether it agrees with the text, and states that
//! test for those two properties only; there is no decided answer yet for an
//! inherited language tag, so resolving one would be inventing the semantics
//! rather than implementing them. The font slots are worse than undecided:
//! a real master writes `<a:cs typeface="+mn-cs"/>`, a *reference* into the
//! theme's `a:fontScheme` which cannot be resolved without reading it. Both
//! are 2.3.
//!
//! List level 1 only, for the same reason: `a:pPr/@lvl` selecting between
//! `lvl1pPr` and `lvl9pPr` is 2.3 by name. Level 1 is the level a paragraph
//! that states no `@lvl` uses, which is every paragraph in the corpus.
//!
//! [ADR 0007]: https://github.com/aenawi/mirsam/blob/main/docs/adr/0007-an-inherited-default-is-not-a-choice.md

use mirsam_core::error::{Error, Result};
use mirsam_core::text::{Alignment, Direction, Origin, Properties, Resolved};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::collections::BTreeMap;

use crate::package::Package;
use crate::pptx::{is_true, parse_alignment};
use crate::rels::{RelationshipGraph, Role};

/// Read an attribute's normalised value, the same way the part scanners do.
fn attribute(tag: &BytesStart, name: &str) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

/// The directional properties one style source states at a list level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Level {
    pub direction: Option<Direction>,
    pub alignment: Option<Alignment>,
}

impl Level {
    /// Read `rtl` and `algn` off an `a:lvl1pPr`.
    fn read(tag: &BytesStart) -> Self {
        Self {
            direction: attribute(tag, "rtl").map(|v| {
                if is_true(&v) {
                    Direction::Rtl
                } else {
                    Direction::Ltr
                }
            }),
            alignment: attribute(tag, "algn").and_then(|v| parse_alignment(&v)),
        }
    }

    fn is_empty(self) -> bool {
        self.direction.is_none() && self.alignment.is_none()
    }
}

/// Which of a master's named text styles governs a shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextStyle {
    /// `p:notesStyle`, first because a notes master has only this one and it
    /// governs every placeholder on a notes slide whatever its type.
    Notes,
    Title,
    Body,
    Other,
}

impl TextStyle {
    /// The element carrying this style in a master.
    fn element(self) -> &'static str {
        match self {
            Self::Notes => "p:notesStyle",
            Self::Title => "p:titleStyle",
            Self::Body => "p:bodyStyle",
            Self::Other => "p:otherStyle",
        }
    }

    /// The style named by an element, if it names one.
    fn from_element(name: &str) -> Option<Self> {
        [Self::Notes, Self::Title, Self::Body, Self::Other]
            .into_iter()
            .find(|style| style.element() == name)
    }

    /// How a finding cites it: `bodyStyle/lvl1pPr`.
    fn property(self) -> String {
        format!("{}/lvl1pPr", &self.element()["p:".len()..])
    }
}

/// A placeholder's identity, as a shape states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    /// `p:ph/@type`, defaulted to `body` as the schema defaults it.
    kind: String,
    /// `p:ph/@idx`. Absent means index zero.
    idx: Option<u32>,
}

impl Placeholder {
    /// Read a `p:ph` element.
    pub fn read(tag: &BytesStart) -> Self {
        Self {
            kind: attribute(tag, "type").unwrap_or_else(|| "body".to_string()),
            idx: attribute(tag, "idx").and_then(|v| v.parse().ok()),
        }
    }

    /// Whether a placeholder on a slide is answered by one on a layout or a
    /// master.
    fn matches(&self, other: &Self) -> bool {
        // An absent index is index zero, so a title placeholder on the slide
        // and one on the layout meet there rather than nowhere.
        if self.idx.unwrap_or(0) != other.idx.unwrap_or(0) {
            return false;
        }
        // Index zero is shared by every untyped placeholder, so the type has
        // to agree as well. Compared through `text_style` rather than
        // literally, because `title` and `ctrTitle` are the same placeholder
        // and a layout may name either.
        self.text_style() == other.text_style()
    }

    /// The master text style this placeholder resolves against.
    pub fn text_style(&self) -> TextStyle {
        match self.kind.as_str() {
            "title" | "ctrTitle" => TextStyle::Title,
            "body" | "subTitle" | "obj" => TextStyle::Body,
            _ => TextStyle::Other,
        }
    }

    /// How a finding cites it: `ph[type=body,idx=1]`.
    fn property(&self) -> String {
        match self.idx {
            Some(idx) => format!("ph[type={},idx={idx}]/lstStyle/lvl1pPr", self.kind),
            None => format!("ph[type={}]/lstStyle/lvl1pPr", self.kind),
        }
    }
}

/// Every style one part supplies to the parts below it.
#[derive(Debug, Clone, Default)]
pub struct PartStyles {
    /// Each placeholder shape's own `a:lstStyle/a:lvl1pPr`, in document order.
    placeholders: Vec<(Placeholder, Level)>,
    /// The master's named text styles.
    text_styles: BTreeMap<TextStyle, Level>,
}

impl PartStyles {
    /// Read the style sources out of one part.
    ///
    /// Everything else in the part is ignored, including its text: this pass
    /// answers "what does this part give the parts below it", and a slide's
    /// own paragraphs are the scanner's business, not this one's.
    pub fn parse(part: &str, xml: &str) -> Result<Self> {
        let mut reader = Reader::from_str(xml);
        let mut styles = Self::default();
        // The placeholder the shape being read declares, and the level its own
        // list style states. Both are discarded when the shape closes.
        let mut shape: Option<Placeholder> = None;
        let mut shape_level = Level::default();
        let mut in_list_style = false;
        let mut named: Option<TextStyle> = None;

        loop {
            let event = match reader.read_event() {
                Err(e) => return Err(Error::Format(format!("{part}: {e}"))),
                Ok(event) => event,
            };
            match event {
                Event::Eof => break,

                Event::Start(ref e) | Event::Empty(ref e) => {
                    // A region is only entered on a `Start`: `<a:lstStyle/>`
                    // has no `End` to leave it by, and states nothing anyway.
                    let opening = matches!(event, Event::Start(_));
                    match e.name().as_ref() {
                        "p:ph" => shape = Some(Placeholder::read(e)),
                        "a:lstStyle" if opening => in_list_style = true,
                        // The same element under two parents: one of the
                        // master's named styles, and a shape's own list style.
                        // Which parent it is under decides where it goes.
                        "a:lvl1pPr" => {
                            if let Some(style) = named {
                                styles.text_styles.insert(style, Level::read(e));
                            } else if in_list_style {
                                shape_level = Level::read(e);
                            }
                        }
                        name => {
                            if opening && let Some(style) = TextStyle::from_element(name) {
                                named = Some(style);
                            }
                        }
                    }
                }

                Event::End(ref e) => match e.name().as_ref() {
                    "a:lstStyle" => in_list_style = false,
                    // A placeholder shape's list style is what it gives the
                    // shapes below it. A shape that is not a placeholder, or
                    // one whose list style says nothing, gives nothing.
                    //
                    // All four shape elements, not `p:sp` alone: a picture or
                    // a graphic frame can be a placeholder too, and a
                    // placeholder left standing after one closed would be
                    // attributed to the next shape that declares none.
                    "p:sp" | "p:graphicFrame" | "p:pic" | "p:cxnSp" => {
                        if let Some(placeholder) = shape.take()
                            && !shape_level.is_empty()
                        {
                            styles.placeholders.push((placeholder, shape_level));
                        }
                        shape_level = Level::default();
                    }
                    name => {
                        if TextStyle::from_element(name).is_some() {
                            named = None;
                        }
                    }
                },

                _ => {}
            }
        }
        Ok(styles)
    }

    /// The list style this part's matching placeholder shape states.
    fn placeholder(&self, want: &Placeholder) -> Option<(&Placeholder, Level)> {
        self.placeholders
            .iter()
            .find(|(here, _)| want.matches(here))
            .map(|(here, level)| (here, *level))
    }

    /// The named text style governing `want` here, and which one it turned
    /// out to be.
    ///
    /// A notes master has one style for everything it lays out, so it answers
    /// with `p:notesStyle` whatever was asked for. No slide master carries
    /// one, so the preference costs nothing there.
    fn named(&self, want: TextStyle) -> Option<(TextStyle, Level)> {
        self.text_styles
            .get_key_value(&TextStyle::Notes)
            .or_else(|| self.text_styles.get_key_value(&want))
            .map(|(style, level)| (*style, *level))
    }

    fn is_empty(&self) -> bool {
        self.placeholders.is_empty() && self.text_styles.is_empty()
    }
}

/// Every style source in one package, and the chain each part resolves along.
#[derive(Debug, Clone, Default)]
pub struct StyleIndex {
    styles: BTreeMap<String, PartStyles>,
    /// Each part's chain, nearest first and including the part itself, with
    /// the parts that supply no styles already dropped.
    chains: BTreeMap<String, Vec<String>>,
}

impl StyleIndex {
    /// Read every style source in a package and the chain each part walks.
    pub fn read(package: &Package) -> Result<Self> {
        let graph = RelationshipGraph::read(package)?;
        Self::from_graph(package, &graph)
    }

    /// The same, for a caller that already holds the graph.
    pub fn from_graph(package: &Package, graph: &RelationshipGraph) -> Result<Self> {
        // Only the parts that lay text out. A part's role comes from the
        // relationship reaching it, never from its directory (2.1), so this
        // is exact on a package that stores its parts anywhere.
        let carries_text = |role: Role| {
            matches!(
                role,
                Role::Slide
                    | Role::SlideLayout
                    | Role::SlideMaster
                    | Role::NotesSlide
                    | Role::NotesMaster
                    | Role::HandoutMaster
            )
        };
        let parts = package.parts_where(|name| carries_text(graph.role_of(name)))?;

        let mut index = Self::default();
        for part in &parts {
            let styles = PartStyles::parse(part, &package.read_text(part)?)?;
            if !styles.is_empty() {
                index.styles.insert(part.clone(), styles);
            }
        }
        for part in &parts {
            index.link(part, graph.inheritance_chain(part));
        }
        Ok(index)
    }

    /// Build an index from already-parsed parts, so the package-reading path
    /// and the test path resolve through the same code.
    pub fn from_parts(
        styles: impl IntoIterator<Item = (String, PartStyles)>,
        chains: impl IntoIterator<Item = (String, Vec<String>)>,
    ) -> Self {
        let mut index = Self {
            styles: styles.into_iter().collect(),
            chains: BTreeMap::new(),
        };
        for (part, chain) in chains {
            index.link(&part, chain);
        }
        index
    }

    /// Record a part's chain, keeping only the hops that supply styles.
    fn link(&mut self, part: &str, chain: Vec<String>) {
        let sources: Vec<String> = chain
            .into_iter()
            .filter(|name| self.styles.contains_key(name))
            .collect();
        if !sources.is_empty() {
            self.chains.insert(part.to_string(), sources);
        }
    }

    /// Fill in every property the paragraph left unset, from the nearest
    /// source above it that states one.
    ///
    /// `placeholder` is the one its shape declares, or `None` for a shape that
    /// is not a placeholder — a text box, a table, a chart's fallback drawing
    /// — which takes the master's `otherStyle` and nothing else.
    ///
    /// Nearest first: the part's own placeholder list style, the layout's, the
    /// master's, and last the master's named text style. A property already
    /// resolved is left alone, so a value the paragraph wrote, or one it takes
    /// from the text body enclosing it, still wins over the chain.
    pub fn resolve(&self, part: &str, placeholder: Option<&Placeholder>, props: &mut Properties) {
        let Some(chain) = self.chains.get(part) else {
            return;
        };
        let want = placeholder.map_or(TextStyle::Other, Placeholder::text_style);

        for name in chain {
            let Some(styles) = self.styles.get(name) else {
                continue;
            };
            if let Some(ph) = placeholder
                && let Some((matched, level)) = styles.placeholder(ph)
            {
                take(props, level, name, &matched.property());
            }
            // A master's named styles sit below its own placeholder shapes and
            // below everything a layout said, and are consulted on the same
            // hop rather than after the walk: only one part in a chain carries
            // them, so the two orderings differ nowhere.
            if let Some((style, level)) = styles.named(want) {
                take(props, level, name, &style.property());
            }
        }
    }
}

/// Take from one style source whatever the paragraph has not resolved yet,
/// recording which part and property supplied it.
fn take(props: &mut Properties, level: Level, part: &str, property: &str) {
    if props.direction.is_unset()
        && let Some(direction) = level.direction
    {
        props.direction =
            Resolved::Inherited(direction, Origin::new(part, format!("{property}@rtl")));
    }
    if props.alignment.is_unset()
        && let Some(alignment) = level.alignment
    {
        props.alignment =
            Resolved::Inherited(alignment, Origin::new(part, format!("{property}@algn")));
    }
}
