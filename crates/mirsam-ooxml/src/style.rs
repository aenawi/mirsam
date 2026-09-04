//! Word's style chain: what a paragraph inherits, and from where.
//!
//! WordprocessingML stores a paragraph's formatting in up to four places, and
//! the paragraph itself is only the last of them. `word/styles.xml` holds the
//! rest: the `w:docDefaults` every paragraph in the document starts from, and
//! the named `w:style` definitions a paragraph reaches with `w:pPr/w:pStyle`
//! and a run with `w:rPr/w:rStyle`, each of which may itself be `w:basedOn`
//! another. This module reads those sources and fills in what the paragraph
//! did not say — the same job [`crate::inherit`] does for PowerPoint, over a
//! vocabulary that shares not one element name with it.
//!
//! ## The order, nearest first
//!
//! 1. Direct formatting — the paragraph's own `w:pPr` and its runs' `w:rPr`.
//!    Read by [`crate::docx`], and already resolved before this module runs.
//! 2. The character style a run names with `w:rStyle`, then everything it is
//!    `w:basedOn`. A character style carries run properties only, so this hop
//!    can supply a font and never a direction.
//! 3. The paragraph style named by `w:pStyle`, then everything it is
//!    `w:basedOn`.
//! 4. **Or**, for a paragraph naming no style at all, the document's default
//!    paragraph style — `w:style[@w:type="paragraph"][@w:default="1"]` — and
//!    its own `w:basedOn` chain. A paragraph that names a style does not also
//!    take the default one ([ECMA-376] Part 1 §17.7.2): the style it named is
//!    the whole answer, and its `w:basedOn` chain is where it looks next.
//! 5. `w:docDefaults`, which is what the document states for everything.
//!
//! **`w:link` is not a hop.** A linked style is one paragraph style and one
//! character style that Word presents as a single entry in its UI, and it
//! writes the run properties into *both* halves. Following the link would
//! resolve a value that is already stated where the walk is looking, and on a
//! document where the halves disagree it would prefer the half Word does not
//! apply.
//!
//! Table styles are the one source deliberately absent, because tables are
//! PLAN §3.4 and a cell has no unit of its own here yet.
//!
//! ## What is resolved, and what is not
//!
//! Direction, alignment, the complex-script font slot, and whether the
//! paragraph has a real list. The same set [`crate::inherit`] resolves, for
//! the same reasons, plus the list.
//!
//! [ADR 0007] decides what to conclude from an inherited value by asking
//! whether it agrees with the text, and states that test for direction and
//! alignment. It says nothing about an inherited language tag, so `w:lang` is
//! left unresolved rather than given semantics this project has not decided.
//!
//! The `cs` slot needs no agreement test: `complex-font-missing` asks whether
//! *any* complex-script font is named, so resolving the slot can only make the
//! tool quieter. The Latin slot is deliberately not resolved for the mirror
//! image of that reason — inheriting a template's `w:asciiTheme` would
//! manufacture that rule's precondition on every paragraph in every document.
//! A list is the same shape of argument: `literal-bullet` is silent on a
//! paragraph that already has one, so a style that supplies a list can only
//! ever remove a finding.
//!
//! ## Theme fonts
//!
//! `w:rFonts/@w:cstheme` names a slot of the theme's `a:fontScheme` rather
//! than a typeface — `minorBidi` is the theme's minor complex-script font, the
//! same slot DrawingML writes as `+mn-cs`. The theme part is DrawingML in both
//! formats, so [`FontScheme`] reads Word's theme unchanged; only the spelling
//! of the reference differs, and [`theme_reference`] is where it is read.
//!
//! When a `w:rFonts` states both `@w:cs` and `@w:cstheme`, the theme wins and
//! the plain name is the fallback for a document whose theme does not answer.
//! Word writes both, the second being the resolved value cached for consumers
//! that do not implement themes.
//!
//! [ADR 0007]: https://github.com/aenawi/mirsam/blob/main/docs/adr/0007-an-inherited-default-is-not-a-choice.md
//! [ECMA-376]: https://ecma-international.org/publications-and-standards/standards/ecma-376/

use mirsam_core::error::{Error, Result};
use mirsam_core::text::{Alignment, Bullet, Direction, Origin, Properties, Resolved};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::collections::{BTreeMap, BTreeSet};

use crate::docx::parse_alignment;
use crate::inherit::{FontScheme, ThemeFont, ThemeScript};
use crate::package::Package;
use crate::rels::RelationshipGraph;
use crate::token::is_true;

/// How far a `w:basedOn` chain is followed before it is called a cycle.
///
/// A well-formed stylesheet nests a handful of styles deep. The bound exists
/// only so a document whose `w:basedOn` edges form a loop is a document to
/// report on rather than one to hang on; [`StyleSheet::chain`] also refuses to
/// visit a style twice, which catches every loop this could not.
const MAX_DEPTH: usize = 64;

/// Read an attribute's normalised value, the same way the part scanners do.
fn attribute(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

/// The same, discarding an attribute that is present but empty.
fn non_empty_attribute(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    attribute(tag, name).filter(|v| !v.is_empty())
}

/// Whether an `ST_OnOff` *element* is on: an absent `w:val` means true.
fn on_off_element(tag: &BytesStart<'_>) -> bool {
    attribute(tag, "w:val").is_none_or(|v| is_true(&v))
}

/// The theme slot an `ST_Theme` value names.
///
/// Word spells the reference as a word where DrawingML spells it `+mn-cs`, but
/// both name one slot of one theme's `a:fontScheme`. `Bidi` is the
/// complex-script slot, which is the only one this module goes on to resolve;
/// the others are read so that a value naming a slot is never mistaken for a
/// typeface called `minorHAnsi`.
pub fn theme_reference(value: &str) -> Option<(ThemeFont, ThemeScript)> {
    let (font, script) = match value {
        "majorAscii" | "majorHAnsi" => (ThemeFont::Major, ThemeScript::Latin),
        "majorEastAsia" => (ThemeFont::Major, ThemeScript::EastAsian),
        "majorBidi" => (ThemeFont::Major, ThemeScript::Complex),
        "minorAscii" | "minorHAnsi" => (ThemeFont::Minor, ThemeScript::Latin),
        "minorEastAsia" => (ThemeFont::Minor, ThemeScript::EastAsian),
        "minorBidi" => (ThemeFont::Minor, ThemeScript::Complex),
        _ => return None,
    };
    Some((font, script))
}

/// A complex-script font slot as one source states it.
///
/// Both halves are kept rather than one resolved winner, because which one
/// answers is not known until a theme is in hand: `@w:cstheme` wins where the
/// theme names a typeface for the slot, and `@w:cs` is what a document without
/// a theme part — or with one whose slot is empty — actually renders with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontRef {
    /// `@w:cstheme`, if it names a slot.
    theme: Option<(ThemeFont, ThemeScript)>,
    /// `@w:cs`, if it names a typeface.
    named: Option<String>,
}

impl FontRef {
    /// Read the complex-script slot off a `w:rFonts`.
    fn read(tag: &BytesStart<'_>) -> Self {
        Self {
            theme: non_empty_attribute(tag, "w:cstheme")
                .as_deref()
                .and_then(theme_reference),
            named: non_empty_attribute(tag, "w:cs"),
        }
    }

    fn is_empty(&self) -> bool {
        self.theme.is_none() && self.named.is_none()
    }

    /// Fill in whichever half this reference has not been given yet.
    ///
    /// First writer wins per half, matching how [`crate::docx`] reads a
    /// paragraph's own runs: `w:pPr/w:rPr` describes the paragraph and the
    /// first run is what it goes on to say.
    fn merge(&mut self, other: Self) {
        if self.theme.is_none() {
            self.theme = other.theme;
        }
        if self.named.is_none() {
            self.named = other.named;
        }
    }
}

/// The properties one style source states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Formatting {
    /// `w:pPr/w:bidi`.
    direction: Option<Direction>,
    /// `w:pPr/w:jc`.
    alignment: Option<Alignment>,
    /// `w:rPr/w:rFonts`, complex-script slot.
    complex_font: FontRef,
    /// `w:pPr/w:numPr`, and whether it names a real list or removes one.
    bullet: Option<Bullet>,
}

impl Formatting {
    fn is_empty(&self) -> bool {
        self.direction.is_none()
            && self.alignment.is_none()
            && self.complex_font.is_empty()
            && self.bullet.is_none()
    }
}

/// One `w:style` definition.
#[derive(Debug, Clone, Default)]
struct Style {
    /// `w:basedOn/@w:val`: the style this one continues from.
    based_on: Option<String>,
    formatting: Formatting,
}

/// A `w:style` element being read.
#[derive(Debug, Clone, Default)]
struct StyleBuilder {
    id: Option<String>,
    /// `@w:type`. `paragraph` is the schema's default, and only a paragraph
    /// style can be the document's default one.
    kind: String,
    default: bool,
    style: Style,
}

/// Which source the element currently being read belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The `w:style` currently open.
    Style,
    /// `w:docDefaults`.
    Defaults,
}

/// Whether `stack` ends with exactly `tail`.
fn ends_with(stack: &[String], tail: &[&str]) -> bool {
    stack.len() >= tail.len()
        && stack[stack.len() - tail.len()..]
            .iter()
            .zip(tail)
            .all(|(here, want)| here == want)
}

/// The source a paragraph property at this point belongs to.
///
/// Matched on the whole path rather than on the parent alone, because
/// `w:tblPr` carries its own `w:jc` and `w:tblStylePr` its own `w:pPr`, and
/// both sit inside a `w:style`. Reading a table's alignment as a paragraph's
/// would put a value in a report that no paragraph has.
fn paragraph_source(stack: &[String]) -> Option<Source> {
    if ends_with(stack, &["w:style", "w:pPr"]) {
        Some(Source::Style)
    } else if ends_with(stack, &["w:docDefaults", "w:pPrDefault", "w:pPr"]) {
        Some(Source::Defaults)
    } else {
        None
    }
}

/// The source a run property at this point belongs to.
fn run_source(stack: &[String]) -> Option<Source> {
    if ends_with(stack, &["w:style", "w:rPr"]) {
        Some(Source::Style)
    } else if ends_with(stack, &["w:docDefaults", "w:rPrDefault", "w:rPr"]) {
        Some(Source::Defaults)
    } else {
        None
    }
}

/// The `w:numId` a `w:numPr` names, as a bullet state.
///
/// `w:numId w:val="0"` is how a source removes numbering rather than supplies
/// it, which is [`Bullet::Suppressed`] and not a list at all. A `w:numPr`
/// naming no `w:numId` states a level within whatever list is already in
/// force, so it is left to the source that named the list.
fn bullet_of(value: &str) -> Bullet {
    if value.trim() == "0" {
        Bullet::Suppressed
    } else {
        Bullet::Native
    }
}

/// The style sources of one Word document, and the theme its font references
/// resolve against.
#[derive(Debug, Clone, Default)]
pub struct StyleSheet {
    /// The part every origin recorded here names: `word/styles.xml`.
    part: String,
    defaults: Formatting,
    /// Every `w:style` with a `@w:styleId`, by that id.
    styles: BTreeMap<String, Style>,
    /// `w:style[@w:type="paragraph"][@w:default="1"]`, if the document has one.
    default_paragraph: Option<String>,
    /// The theme part and its font scheme, for `@w:cstheme`.
    theme: Option<(String, FontScheme)>,
}

impl StyleSheet {
    /// Read the stylesheet and theme a package's main document names.
    pub fn read(package: &Package) -> Result<Self> {
        let graph = RelationshipGraph::read(package)?;
        Self::from_graph(package, &graph)
    }

    /// The same, for a caller that already holds the graph.
    ///
    /// Both parts are reached by the relationship pointing at them rather than
    /// by their conventional path: a package is free to store `styles.xml`
    /// anywhere, and a reader that hard-codes `word/styles.xml` silently
    /// resolves nothing on one that does ([`crate::rels`]).
    pub fn from_graph(package: &Package, graph: &RelationshipGraph) -> Result<Self> {
        let Some(document) = graph.office_document().map(str::to_string) else {
            return Ok(Self::default());
        };
        let Some(part) = graph
            .first_part_of_kind(&document, "styles")
            .map(str::to_string)
        else {
            return Ok(Self::default());
        };

        let mut sheet = Self::parse(&part, &package.read_text(&part)?)?;
        if let Some(theme) = graph
            .first_part_of_kind(&document, "theme")
            .map(str::to_string)
        {
            let scheme = FontScheme::parse(&theme, &package.read_text(&theme)?)?;
            if !scheme.is_empty() {
                sheet = sheet.with_theme(theme, scheme);
            }
        }
        Ok(sheet)
    }

    /// Point this stylesheet at the theme its `@w:cstheme` references resolve
    /// against.
    ///
    /// The package path takes this from the relationship graph; a caller
    /// holding hand-built parts states it directly.
    pub fn with_theme(mut self, theme: impl Into<String>, scheme: FontScheme) -> Self {
        self.theme = Some((theme.into(), scheme));
        self
    }

    /// Whether this stylesheet supplies anything at all.
    ///
    /// A style that states no formatting is not something it supplies: a table
    /// style whose every property this module leaves alone is read, filed and
    /// reachable by name, and answers nothing.
    pub fn is_empty(&self) -> bool {
        self.defaults.is_empty() && self.styles.values().all(|s| s.formatting.is_empty())
    }

    /// Read one `word/styles.xml`.
    pub fn parse(part: &str, xml: &str) -> Result<Self> {
        let mut reader = Reader::from_str(xml);
        let mut sheet = Self {
            part: part.to_string(),
            ..Self::default()
        };
        // The element path, outermost first, holding the *ancestors* of the
        // element being read: a start tag is pushed after it is read, and an
        // empty one is never pushed at all.
        let mut stack: Vec<String> = Vec::new();
        let mut open: Option<StyleBuilder> = None;

        loop {
            let event = match reader.read_event() {
                Err(e) => return Err(Error::Format(format!("{part}: {e}"))),
                Ok(event) => event,
            };
            match event {
                Event::Eof => break,

                Event::Start(ref e) | Event::Empty(ref e) => {
                    let opening = matches!(event, Event::Start(_));
                    let name = e.name();
                    let name = name.as_ref();
                    sheet.read_element(&mut open, &stack, name, e, opening);
                    if opening {
                        stack.push(name.to_string());
                    }
                }

                Event::End(_) => {
                    // Well-formedness is quick-xml's to enforce, so a pop here
                    // always answers the start that pushed it.
                    if stack.pop().as_deref() == Some("w:style") {
                        sheet.commit(open.take());
                    }
                }

                _ => {}
            }
        }
        Ok(sheet)
    }

    /// Read one start-ish tag into whichever source encloses it.
    fn read_element(
        &mut self,
        open: &mut Option<StyleBuilder>,
        stack: &[String],
        name: &str,
        e: &BytesStart<'_>,
        opening: bool,
    ) {
        match name {
            // A self-closing `w:style` states nothing, and opening a builder
            // for one would leave it uncommitted and leak into the next style.
            "w:style" if opening && ends_with(stack, &["w:styles"]) => {
                *open = Some(StyleBuilder {
                    id: non_empty_attribute(e, "w:styleId"),
                    // `@w:type` defaults to `paragraph` exactly as
                    // `ST_StyleType` does.
                    kind: attribute(e, "w:type").unwrap_or_else(|| "paragraph".to_string()),
                    // An `ST_OnOff` *attribute*, unlike the elements above:
                    // absent means false, and only a present value can be on.
                    default: attribute(e, "w:default").is_some_and(|v| is_true(&v)),
                    ..Default::default()
                });
            }
            "w:basedOn" if ends_with(stack, &["w:style"]) => {
                if let Some(builder) = open.as_mut() {
                    builder.style.based_on = non_empty_attribute(e, "w:val");
                }
            }
            "w:bidi" => {
                let direction = if on_off_element(e) {
                    Direction::Rtl
                } else {
                    Direction::Ltr
                };
                if let Some(formatting) = paragraph_source(stack).and_then(|s| self.target(open, s))
                {
                    formatting.direction.get_or_insert(direction);
                }
            }
            "w:jc" => {
                let alignment = attribute(e, "w:val").as_deref().and_then(parse_alignment);
                if let Some(alignment) = alignment
                    && let Some(formatting) =
                        paragraph_source(stack).and_then(|s| self.target(open, s))
                {
                    formatting.alignment.get_or_insert(alignment);
                }
            }
            // A level within a list, or `w:numId="0"` removing one. Matched on
            // the `w:numId` inside rather than on `w:numPr` itself, because the
            // two say opposite things and only the child tells them apart.
            "w:numId" if ends_with(stack, &["w:pPr", "w:numPr"]) => {
                let bullet = attribute(e, "w:val").as_deref().map(bullet_of);
                // `stack` ends `.../w:pPr/w:numPr` here, so the paragraph
                // source is two hops further out than the property elements.
                let source = paragraph_source(&stack[..stack.len() - 1]);
                if let Some(bullet) = bullet
                    && let Some(formatting) = source.and_then(|s| self.target(open, s))
                {
                    formatting.bullet.get_or_insert(bullet);
                }
            }
            "w:rFonts" => {
                let font = FontRef::read(e);
                if let Some(formatting) = run_source(stack).and_then(|s| self.target(open, s)) {
                    formatting.complex_font.merge(font);
                }
            }
            _ => {}
        }
    }

    /// The formatting one source accumulates into.
    fn target<'a>(
        &'a mut self,
        open: &'a mut Option<StyleBuilder>,
        source: Source,
    ) -> Option<&'a mut Formatting> {
        match source {
            Source::Style => open.as_mut().map(|b| &mut b.style.formatting),
            Source::Defaults => Some(&mut self.defaults),
        }
    }

    /// File a finished `w:style` under its id.
    fn commit(&mut self, builder: Option<StyleBuilder>) {
        let Some(builder) = builder else { return };
        let Some(id) = builder.id else { return };
        if builder.default && builder.kind == "paragraph" {
            // First writer wins: a document naming two default paragraph
            // styles is malformed, and Word applies the first.
            self.default_paragraph.get_or_insert_with(|| id.clone());
        }
        self.styles.insert(id, builder.style);
    }

    /// One style and everything it is `w:basedOn`, nearest first.
    ///
    /// A style that names itself, or a loop of styles that name each other, is
    /// walked once and then stopped: a malformed stylesheet is one to report
    /// on, not one to hang on.
    fn chain(&self, id: Option<&str>) -> Vec<(&str, &Style)> {
        let mut chain = Vec::new();
        let mut seen = BTreeSet::new();
        let mut here = id;

        while let Some(name) = here {
            if chain.len() >= MAX_DEPTH || !seen.insert(name) {
                break;
            }
            let Some((id, style)) = self.styles.get_key_value(name) else {
                break;
            };
            chain.push((id.as_str(), style));
            here = style.based_on.as_deref();
        }
        chain
    }

    /// Resolve one `@w:cstheme` reference, naming the theme it came from.
    ///
    /// The theme is named rather than the part that wrote `minorBidi`, because
    /// the theme is where the typeface a reader will see is written and
    /// invariant 6 asks for the one look that checks the claim.
    pub fn theme_font(&self, reference: (ThemeFont, ThemeScript)) -> Option<(String, Origin)> {
        let (font, script) = reference;
        let (part, scheme) = self.theme.as_ref()?;
        let name = scheme.typeface(font, script)?;
        Some((
            name.to_string(),
            Origin::new(part, FontScheme::property(font, script)),
        ))
    }

    /// Fill in every property the paragraph left unset, from the nearest
    /// source above it that states one.
    ///
    /// `style` is the paragraph's `w:pPr/w:pStyle` and `run_style` the
    /// `w:rPr/w:rStyle` of the first run that named one. A property already
    /// resolved is left alone, so direct formatting still wins.
    pub fn resolve(&self, style: Option<&str>, run_style: Option<&str>, props: &mut Properties) {
        for (id, source) in self.chain(run_style) {
            self.take(props, &source.formatting, Some(id));
        }
        // A paragraph that names a style does not also take the default one;
        // the chain it named is the whole answer.
        let paragraph = style.or(self.default_paragraph.as_deref());
        for (id, source) in self.chain(paragraph) {
            self.take(props, &source.formatting, Some(id));
        }
        self.take(props, &self.defaults, None);
    }

    /// Take from one source whatever the paragraph has not resolved yet,
    /// recording which part and property supplied it.
    ///
    /// `id` names the `w:style` the values came from, or `None` for
    /// `w:docDefaults`, which is the difference between the two property paths
    /// a finding can cite.
    fn take(&self, props: &mut Properties, formatting: &Formatting, id: Option<&str>) {
        let (paragraph, run) = match id {
            Some(id) => (format!("style[{id}]/pPr"), format!("style[{id}]/rPr")),
            None => (
                "docDefaults/pPrDefault/pPr".to_string(),
                "docDefaults/rPrDefault/rPr".to_string(),
            ),
        };

        if props.direction.is_unset()
            && let Some(direction) = formatting.direction
        {
            props.direction = Resolved::Inherited(
                direction,
                Origin::new(&self.part, format!("{paragraph}@bidi")),
            );
        }
        if props.alignment.is_unset()
            && let Some(alignment) = formatting.alignment
        {
            props.alignment = Resolved::Inherited(
                alignment,
                Origin::new(&self.part, format!("{paragraph}@jc")),
            );
        }
        // No `Resolved` wrapper and so no origin to record. That is sound only
        // because a resolved list can never *raise* a finding: `literal-bullet`
        // is silent on a paragraph that has one, so nothing a reviewer would
        // need to check is being claimed here.
        if props.bullet == Bullet::None
            && let Some(bullet) = formatting.bullet
        {
            props.bullet = bullet;
        }
        if props.complex_font.is_unset() {
            props.complex_font = self.font(&formatting.complex_font, &run);
        }
    }

    /// Resolve one source's complex-script slot: the theme reference first,
    /// then the typeface named beside it.
    fn font(&self, reference: &FontRef, property: &str) -> Resolved<String> {
        if let Some(reference) = reference.theme
            && let Some((name, origin)) = self.theme_font(reference)
        {
            return Resolved::Inherited(name, origin);
        }
        match &reference.named {
            Some(name) => Resolved::Inherited(
                name.clone(),
                Origin::new(&self.part, format!("{property}/rFonts@cs")),
            ),
            None => Resolved::Unset,
        }
    }
}
