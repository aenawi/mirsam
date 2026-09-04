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
//! ## Which list level answers
//!
//! Every style source states its properties nine times over, once per list
//! level: `a:lvl1pPr` through `a:lvl9pPr`. A paragraph picks one with
//! `a:pPr/@lvl`, which is **zero-based** — `lvl="1"` is the second level and
//! reads `a:lvl2pPr` — and a paragraph that states no `@lvl` is at level 0.
//!
//! A level is answered only by the same level above it. A source whose
//! `bodyStyle` states `lvl1pPr` and nothing else supplies nothing to a
//! paragraph at level 2, and the walk carries on to the next source rather
//! than falling back to level 1 here: PowerPoint's own fallback for a level a
//! master does not state is its application default, not that master's first
//! level, and inventing the second would report a value no reader will see.
//!
//! ## What is resolved, and what is not
//!
//! Direction, alignment, and the complex-script font slot.
//!
//! [ADR 0007] decides what to conclude from an inherited value by asking
//! whether it agrees with the text, and states that test for direction and
//! alignment. It says nothing about an inherited language tag, so resolving
//! one would be inventing the semantics rather than implementing them; `lang`
//! stays unresolved.
//!
//! The `cs` slot needs no such test, because the rule reading it
//! (`complex-font-missing`) asks whether *any* complex-script font is named,
//! not whether the named one suits the text. A master that names one has named
//! one for the paragraph below it, so resolving the slot can only ever make
//! the tool quieter — never louder — and it cannot fire on a value the author
//! did not choose. The Latin slot is deliberately *not* resolved for the same
//! reason read the other way: `complex-font-missing` fires only where a Latin
//! font is chosen, and a template's `+mn-lt` is not a choice anyone made about
//! this paragraph. Inheriting it would manufacture the rule's precondition on
//! every paragraph in every deck.
//!
//! A real master writes `<a:cs typeface="+mn-cs"/>` rather than a typeface
//! name: a reference into the theme's `a:fontScheme`, which is why the slot
//! could not be resolved before [`crate::rels`] could reach the theme part.
//! [`Typeface`] carries the two forms apart and [`FontScheme`] answers the
//! reference. A reference the theme answers with an empty typeface — which is
//! what the stock Office theme states for `cs` — is a theme naming no
//! complex-script font, and stays unresolved.
//!
//! [ADR 0007]: https://github.com/aenawi/mirsam/blob/main/docs/adr/0007-an-inherited-default-is-not-a-choice.md

use mirsam_core::error::{Error, Result};
use mirsam_core::text::{Alignment, Direction, Origin, Properties, Resolved};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::collections::BTreeMap;

use crate::package::Package;
use crate::pptx::parse_alignment;
use crate::rels::{RelationshipGraph, Role};
use crate::token::is_true;

/// List levels a style source may state: `a:lvl1pPr` … `a:lvl9pPr`.
const LEVELS: usize = 9;

/// Read an attribute's normalised value, the same way the part scanners do.
fn attribute(tag: &BytesStart, name: &str) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

/// The zero-based level an `a:lvl1pPr` … `a:lvl9pPr` element states.
fn level_of_element(name: &str) -> Option<usize> {
    let digit = name.strip_prefix("a:lvl")?.strip_suffix("pPr")?;
    let level: usize = digit.parse().ok()?;
    (1..=LEVELS).contains(&level).then(|| level - 1)
}

/// The zero-based level a paragraph's `a:pPr/@lvl` selects.
///
/// Out-of-range values are clamped rather than rejected: `@lvl` is a
/// `ST_TextIndentLevelType`, so 0–8 is all a valid document writes, and a
/// document that writes more is still laid out at *some* level by the
/// application. Clamping resolves it against the nearest level that exists;
/// refusing would silently drop the whole chain for that paragraph.
pub fn level_of_attribute(value: &str) -> usize {
    value.trim().parse::<usize>().unwrap_or(0).min(LEVELS - 1)
}

/// Which of the theme's two fonts a `+mj-`/`+mn-` reference names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeFont {
    /// `+mj-…`, `a:majorFont`: headings.
    Major,
    /// `+mn-…`, `a:minorFont`: body text.
    Minor,
}

impl ThemeFont {
    /// The element carrying it in a theme's `a:fontScheme`.
    fn element(self) -> &'static str {
        match self {
            Self::Major => "majorFont",
            Self::Minor => "minorFont",
        }
    }
}

/// Which script slot of a theme font a reference names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeScript {
    /// `-lt`, `a:latin`.
    Latin,
    /// `-ea`, `a:ea`.
    EastAsian,
    /// `-cs`, `a:cs`.
    Complex,
}

impl ThemeScript {
    /// The element carrying it under `a:majorFont` / `a:minorFont`.
    fn element(self) -> &'static str {
        match self {
            Self::Latin => "latin",
            Self::EastAsian => "ea",
            Self::Complex => "cs",
        }
    }
}

/// A typeface as a document states it.
///
/// DrawingML lets either form stand where a font is named, and they are not
/// interchangeable: `Dubai` is a font, `+mn-cs` is a pointer at whatever the
/// theme's minor font names for complex scripts. Recording the pointer as if
/// it were a font name is what produced `complex_font: "+mn-cs"` in a report,
/// which names no font anyone has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typeface {
    /// A typeface named outright.
    Named(String),
    /// A reference into the theme's `a:fontScheme`.
    Theme(ThemeFont, ThemeScript),
}

impl Typeface {
    /// Read a `@typeface` attribute value.
    ///
    /// `None` for an empty value — which is how the stock Office theme states
    /// "no complex-script font" — and for a `+` form naming no slot this
    /// module knows, because a name beginning with `+` is reserved for the
    /// reference syntax and is not a font either way.
    pub fn parse(value: &str) -> Option<Self> {
        if value.is_empty() {
            return None;
        }
        let Some(reference) = value.strip_prefix('+') else {
            return Some(Self::Named(value.to_string()));
        };
        let (font, script) = reference.split_once('-')?;
        let font = match font {
            "mj" => ThemeFont::Major,
            "mn" => ThemeFont::Minor,
            _ => return None,
        };
        let script = match script {
            "lt" => ThemeScript::Latin,
            "ea" => ThemeScript::EastAsian,
            "cs" => ThemeScript::Complex,
            _ => return None,
        };
        Some(Self::Theme(font, script))
    }
}

/// The three script slots one theme font states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FontSlots {
    latin: Option<String>,
    east_asian: Option<String>,
    complex: Option<String>,
}

impl FontSlots {
    fn slot(&mut self, script: ThemeScript) -> &mut Option<String> {
        match script {
            ThemeScript::Latin => &mut self.latin,
            ThemeScript::EastAsian => &mut self.east_asian,
            ThemeScript::Complex => &mut self.complex,
        }
    }

    fn get(&self, script: ThemeScript) -> Option<&str> {
        match script {
            ThemeScript::Latin => self.latin.as_deref(),
            ThemeScript::EastAsian => self.east_asian.as_deref(),
            ThemeScript::Complex => self.complex.as_deref(),
        }
    }

    fn is_empty(&self) -> bool {
        self.latin.is_none() && self.east_asian.is_none() && self.complex.is_none()
    }
}

/// A theme's `a:fontScheme`: the two fonts every `+mj-`/`+mn-` reference in
/// the parts below it resolves against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontScheme {
    major: FontSlots,
    minor: FontSlots,
}

impl FontScheme {
    /// Read the font scheme out of a theme part.
    ///
    /// Only the `a:fontScheme` region is read. `a:latin` and `a:cs` also occur
    /// in a theme's `a:objectDefaults`, where they are defaults for shapes
    /// rather than the scheme references point at, so the region has to be
    /// tracked rather than the element names matched on their own.
    pub fn parse(part: &str, xml: &str) -> Result<Self> {
        let mut reader = Reader::from_str(xml);
        let mut scheme = Self::default();
        let mut in_scheme = false;
        let mut font: Option<ThemeFont> = None;

        loop {
            let event = match reader.read_event() {
                Err(e) => return Err(Error::Format(format!("{part}: {e}"))),
                Ok(event) => event,
            };
            match event {
                Event::Eof => break,

                Event::Start(ref e) | Event::Empty(ref e) => {
                    let opening = matches!(event, Event::Start(_));
                    match e.name().as_ref() {
                        "a:fontScheme" if opening => in_scheme = true,
                        "a:majorFont" if in_scheme && opening => font = Some(ThemeFont::Major),
                        "a:minorFont" if in_scheme && opening => font = Some(ThemeFont::Minor),
                        name => {
                            // `a:font script="Arab"` sits beside these and is
                            // the application's per-script fallback, not the
                            // slot a `+mn-cs` reference names. It is left
                            // alone: what it means for a `cs` slot the theme
                            // leaves empty is not a question this module has
                            // an answer to.
                            let script = match name {
                                "a:latin" => ThemeScript::Latin,
                                "a:ea" => ThemeScript::EastAsian,
                                "a:cs" => ThemeScript::Complex,
                                _ => continue,
                            };
                            let Some(font) = font else { continue };
                            let slots = match font {
                                ThemeFont::Major => &mut scheme.major,
                                ThemeFont::Minor => &mut scheme.minor,
                            };
                            *slots.slot(script) = attribute(e, "typeface")
                                .filter(|v| !v.is_empty())
                                // A scheme slot naming another slot would be
                                // circular; only a name is taken.
                                .filter(|v| !v.starts_with('+'));
                        }
                    }
                }

                Event::End(ref e) => match e.name().as_ref() {
                    "a:fontScheme" => in_scheme = false,
                    "a:majorFont" | "a:minorFont" => font = None,
                    _ => {}
                },

                _ => {}
            }
        }
        Ok(scheme)
    }

    /// The typeface one slot names, if it names one.
    pub fn typeface(&self, font: ThemeFont, script: ThemeScript) -> Option<&str> {
        match font {
            ThemeFont::Major => self.major.get(script),
            ThemeFont::Minor => self.minor.get(script),
        }
        .filter(|v| !v.is_empty())
    }

    /// How a finding cites one slot: `fontScheme/minorFont/cs@typeface`.
    fn property(font: ThemeFont, script: ThemeScript) -> String {
        format!(
            "fontScheme/{}/{}@typeface",
            font.element(),
            script.element()
        )
    }

    fn is_empty(&self) -> bool {
        self.major.is_empty() && self.minor.is_empty()
    }
}

/// The properties one style source states at one list level.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Level {
    pub direction: Option<Direction>,
    pub alignment: Option<Alignment>,
    /// `a:defRPr/a:cs/@typeface`, which is a theme reference more often than
    /// it is a name.
    pub complex_font: Option<Typeface>,
}

impl Level {
    /// Read `rtl` and `algn` off an `a:lvlNpPr`. The font slot arrives later:
    /// it is stated by a child element, not by an attribute.
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
            complex_font: None,
        }
    }

    fn is_empty(&self) -> bool {
        self.direction.is_none() && self.alignment.is_none() && self.complex_font.is_none()
    }
}

/// The list levels one style source states, keyed as `a:pPr/@lvl` numbers
/// them: level 0 is `a:lvl1pPr`. Levels stating nothing are absent.
type Levels = BTreeMap<usize, Level>;

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

    /// How a finding cites it: `bodyStyle/lvl2pPr`.
    fn property(self, level: usize) -> String {
        format!("{}/lvl{}pPr", &self.element()["p:".len()..], level + 1)
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

    /// How a finding cites it: `ph[type=body,idx=1]/lstStyle/lvl2pPr`.
    fn property(&self, level: usize) -> String {
        let level = level + 1;
        match self.idx {
            Some(idx) => format!("ph[type={},idx={idx}]/lstStyle/lvl{level}pPr", self.kind),
            None => format!("ph[type={}]/lstStyle/lvl{level}pPr", self.kind),
        }
    }
}

/// Every style one part supplies to the parts below it.
#[derive(Debug, Clone, Default)]
pub struct PartStyles {
    /// Each placeholder shape's own `a:lstStyle`, in document order.
    placeholders: Vec<(Placeholder, Levels)>,
    /// The master's named text styles.
    text_styles: BTreeMap<TextStyle, Levels>,
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
        // The placeholder the shape being read declares, and the levels its
        // own list style states. Both are discarded when the shape closes.
        let mut shape: Option<Placeholder> = None;
        let mut shape_levels = Levels::new();
        let mut in_list_style = false;
        let mut named: Option<TextStyle> = None;
        // The `a:lvlNpPr` being read: its level and what it has stated so far.
        // Held open rather than committed at the start tag because the font
        // slot is a child element.
        let mut open_level: Option<(usize, Level)> = None;

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
                    let name = e.name();
                    let name = name.as_ref();
                    match name {
                        "p:ph" => shape = Some(Placeholder::read(e)),
                        "a:lstStyle" if opening => in_list_style = true,
                        // The complex-script slot for the level being read.
                        // Guarded on the level rather than matched on its own,
                        // because `a:cs` also occurs in the part's own runs,
                        // which are the scanner's text and not a style source.
                        "a:cs" => {
                            if let Some((_, level)) = open_level.as_mut()
                                && level.complex_font.is_none()
                                && let Some(typeface) = attribute(e, "typeface")
                                    .as_deref()
                                    .and_then(Typeface::parse)
                            {
                                level.complex_font = Some(typeface);
                            }
                        }
                        _ => {
                            if let Some(index) = level_of_element(name) {
                                // The same element under two parents: one of
                                // the master's named styles, and a shape's own
                                // list style. Which parent it is under decides
                                // where it goes, and that is settled on close.
                                let level = Level::read(e);
                                if opening {
                                    open_level = Some((index, level));
                                } else {
                                    commit(
                                        &mut styles,
                                        &mut shape_levels,
                                        named,
                                        in_list_style,
                                        index,
                                        level,
                                    );
                                }
                            } else if opening && let Some(style) = TextStyle::from_element(name) {
                                named = Some(style);
                            }
                        }
                    }
                }

                Event::End(ref e) => {
                    let name = e.name();
                    let name = name.as_ref();
                    match name {
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
                                && !shape_levels.is_empty()
                            {
                                styles
                                    .placeholders
                                    .push((placeholder, std::mem::take(&mut shape_levels)));
                            }
                            shape_levels.clear();
                        }
                        _ => {
                            if level_of_element(name).is_some() {
                                if let Some((index, level)) = open_level.take() {
                                    commit(
                                        &mut styles,
                                        &mut shape_levels,
                                        named,
                                        in_list_style,
                                        index,
                                        level,
                                    );
                                }
                            } else if TextStyle::from_element(name).is_some() {
                                named = None;
                            }
                        }
                    }
                }

                _ => {}
            }
        }
        Ok(styles)
    }

    /// The levels this part's matching placeholder shape states.
    fn placeholder(&self, want: &Placeholder) -> Option<(&Placeholder, &Levels)> {
        self.placeholders
            .iter()
            .find(|(here, _)| want.matches(here))
            .map(|(here, levels)| (here, levels))
    }

    /// The named text style governing `want` here, and which one it turned
    /// out to be.
    ///
    /// A notes master has one style for everything it lays out, so it answers
    /// with `p:notesStyle` whatever was asked for. No slide master carries
    /// one, so the preference costs nothing there.
    fn named(&self, want: TextStyle) -> Option<(TextStyle, &Levels)> {
        self.text_styles
            .get_key_value(&TextStyle::Notes)
            .or_else(|| self.text_styles.get_key_value(&want))
            .map(|(style, levels)| (*style, levels))
    }

    fn is_empty(&self) -> bool {
        self.placeholders.is_empty() && self.text_styles.is_empty()
    }
}

/// File a finished `a:lvlNpPr` under whichever style source encloses it.
fn commit(
    styles: &mut PartStyles,
    shape_levels: &mut Levels,
    named: Option<TextStyle>,
    in_list_style: bool,
    index: usize,
    level: Level,
) {
    if level.is_empty() {
        return;
    }
    if let Some(style) = named {
        styles
            .text_styles
            .entry(style)
            .or_default()
            .insert(index, level);
    } else if in_list_style {
        shape_levels.insert(index, level);
    }
}

/// Every style source in one package, and the chain each part resolves along.
#[derive(Debug, Clone, Default)]
pub struct StyleIndex {
    styles: BTreeMap<String, PartStyles>,
    /// Each part's chain, nearest first and including the part itself, with
    /// the parts that supply no styles already dropped.
    chains: BTreeMap<String, Vec<String>>,
    /// Each theme part's font scheme.
    schemes: BTreeMap<String, FontScheme>,
    /// The theme part each part resolves a `+mn-cs` reference against. Kept
    /// apart from `chains` because a theme states no paragraph properties: it
    /// is not a hop in the walk, it is what the walk's font references mean.
    themes: BTreeMap<String, String>,
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

        for theme in package.parts_where(|name| graph.role_of(name) == Role::Theme)? {
            let scheme = FontScheme::parse(&theme, &package.read_text(&theme)?)?;
            if !scheme.is_empty() {
                index.schemes.insert(theme, scheme);
            }
        }
        for part in &parts {
            if let Some(theme) = graph.theme_of(part)
                && index.schemes.contains_key(&theme)
            {
                index.themes.insert(part.clone(), theme);
            }
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
            ..Self::default()
        };
        for (part, chain) in chains {
            index.link(&part, chain);
        }
        index
    }

    /// Point parts at the theme their font references resolve against.
    ///
    /// The package path takes this from the relationship graph; a caller
    /// holding hand-built parts states it directly.
    pub fn with_theme(
        mut self,
        theme: impl Into<String>,
        scheme: FontScheme,
        parts: impl IntoIterator<Item = String>,
    ) -> Self {
        let theme = theme.into();
        self.schemes.insert(theme.clone(), scheme);
        for part in parts {
            self.themes.insert(part, theme.clone());
        }
        self
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

    /// The font scheme a part's `+mj-`/`+mn-` references resolve against.
    fn scheme_for(&self, part: &str) -> Option<(&str, &FontScheme)> {
        let theme = self.themes.get(part)?;
        Some((theme.as_str(), self.schemes.get(theme)?))
    }

    /// Resolve one theme reference a part states, naming the theme it came
    /// from.
    ///
    /// The theme is named rather than the part that wrote `+mn-cs`, because
    /// the theme is where the typeface a reader will see is written and
    /// invariant 6 asks for the one look that checks the claim. A reference
    /// the theme answers with nothing resolves to nothing.
    pub fn theme_font(&self, part: &str, typeface: &Typeface) -> Option<(String, Origin)> {
        let Typeface::Theme(font, script) = typeface else {
            return None;
        };
        let (theme, scheme) = self.scheme_for(part)?;
        let name = scheme.typeface(*font, *script)?;
        Some((
            name.to_string(),
            Origin::new(theme, FontScheme::property(*font, *script)),
        ))
    }

    /// Fill in every property the paragraph left unset, from the nearest
    /// source above it that states one.
    ///
    /// `placeholder` is the one its shape declares, or `None` for a shape that
    /// is not a placeholder — a text box, a table, a chart's fallback drawing
    /// — which takes the master's `otherStyle` and nothing else. `level` is
    /// the paragraph's zero-based `a:pPr/@lvl`, and only that level of each
    /// source answers it.
    ///
    /// Nearest first: the part's own placeholder list style, the layout's, the
    /// master's, and last the master's named text style. A property already
    /// resolved is left alone, so a value the paragraph wrote, or one it takes
    /// from the text body enclosing it, still wins over the chain.
    pub fn resolve(
        &self,
        part: &str,
        placeholder: Option<&Placeholder>,
        level: usize,
        props: &mut Properties,
    ) {
        let Some(chain) = self.chains.get(part) else {
            return;
        };
        let want = placeholder.map_or(TextStyle::Other, Placeholder::text_style);
        let theme = self.scheme_for(part);

        for name in chain {
            let Some(styles) = self.styles.get(name) else {
                continue;
            };
            if let Some(ph) = placeholder
                && let Some((matched, levels)) = styles.placeholder(ph)
                && let Some(stated) = levels.get(&level)
            {
                take(props, stated, name, &matched.property(level), theme);
            }
            // A master's named styles sit below its own placeholder shapes and
            // below everything a layout said, and are consulted on the same
            // hop rather than after the walk: only one part in a chain carries
            // them, so the two orderings differ nowhere.
            if let Some((style, levels)) = styles.named(want)
                && let Some(stated) = levels.get(&level)
            {
                take(props, stated, name, &style.property(level), theme);
            }
        }
    }
}

/// Take from one style source whatever the paragraph has not resolved yet,
/// recording which part and property supplied it.
fn take(
    props: &mut Properties,
    level: &Level,
    part: &str,
    property: &str,
    theme: Option<(&str, &FontScheme)>,
) {
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
    if props.complex_font.is_unset()
        && let Some(typeface) = &level.complex_font
    {
        props.complex_font = match typeface {
            Typeface::Named(name) => Resolved::Inherited(
                name.clone(),
                Origin::new(part, format!("{property}/defRPr/cs@typeface")),
            ),
            Typeface::Theme(font, script) => match theme {
                Some((theme, scheme)) => match scheme.typeface(*font, *script) {
                    Some(name) => Resolved::Inherited(
                        name.to_string(),
                        Origin::new(theme, FontScheme::property(*font, *script)),
                    ),
                    None => Resolved::Unset,
                },
                None => Resolved::Unset,
            },
        };
    }
}
