//! Format-agnostic text model.
//!
//! Every adapter (PPTX, DOCX, HTML, PDF) lowers its native structure into
//! [`TextUnit`]s so that the rule engine never learns a file format.

use std::fmt;

/// Where an inherited value came from: the part that stated it, and the
/// property within that part that did.
///
/// A finding on an inherited value has to name its source, or a reviewer
/// cannot check the claim without opening the application (invariant 6, and
/// ADR 0007 §5). `ppt/slideMasters/slideMaster1.xml bodyStyle/lvl1pPr@rtl` is
/// one look.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Origin {
    /// The part that stated the value: `ppt/slideLayouts/slideLayout1.xml`.
    pub part: String,
    /// The property path within it: `titleStyle/lvl1pPr@algn`.
    pub property: String,
}

impl Origin {
    pub fn new(part: impl Into<String>, property: impl Into<String>) -> Self {
        Self {
            part: part.into(),
            property: property.into(),
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.part, self.property)
    }
}

/// A document property that may be stated outright, inherited from an ancestor
/// (a PowerPoint layout/master, a CSS cascade, a Word style), or genuinely absent.
///
/// Modelling inheritance explicitly is what keeps the engine from reporting an
/// inherited-and-correct value as a missing one — the single largest source of
/// false positives in attribute-only Arabic linters.
///
/// Resolving a value is not the same as establishing that anyone chose it: an
/// English template's untouched `rtl="0"` under Arabic is a default nobody
/// aimed at the text. So an inherited value carries its [`Origin`], and a rule
/// fires on it only where it *contradicts* the text (ADR 0007).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "state", content = "value"))]
pub enum Resolved<T> {
    /// Set directly on this unit.
    Explicit(T),
    /// Not set here, but supplied by the named ancestor.
    Inherited(T, Origin),
    /// Not set anywhere in the chain. The renderer falls back to its own default.
    Unset,
}

impl<T> Resolved<T> {
    /// The effective value, whether stated here or inherited.
    pub fn effective(&self) -> Option<&T> {
        match self {
            Self::Explicit(v) | Self::Inherited(v, _) => Some(v),
            Self::Unset => None,
        }
    }

    /// Where an inherited value came from; `None` for one stated here or
    /// absent everywhere.
    pub fn origin(&self) -> Option<&Origin> {
        match self {
            Self::Inherited(_, origin) => Some(origin),
            _ => None,
        }
    }

    pub fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }

    pub fn is_explicit(&self) -> bool {
        matches!(self, Self::Explicit(_))
    }

    pub fn is_inherited(&self) -> bool {
        matches!(self, Self::Inherited(..))
    }
}

/// Base direction of a paragraph or container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Direction {
    Rtl,
    Ltr,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Rtl => "rtl",
            Self::Ltr => "ltr",
        })
    }
}

/// Paragraph alignment, normalised across formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Alignment {
    Start,
    End,
    Left,
    Right,
    Center,
    Justify,
    Distributed,
}

impl Alignment {
    /// Whether this alignment is coherent with an RTL base direction.
    ///
    /// A hard `Left` on Arabic body text is the suspicious case; centre,
    /// justified and direction-relative alignments are all legitimate.
    pub fn is_rtl_coherent(self) -> bool {
        !matches!(self, Self::Left)
    }
}

impl fmt::Display for Alignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Start => "start",
            Self::End => "end",
            Self::Left => "left",
            Self::Right => "right",
            Self::Center => "center",
            Self::Justify => "justify",
            Self::Distributed => "distributed",
        })
    }
}

/// Which edge a leading inset — an indent, a margin, a padding — is measured
/// from.
///
/// The same distinction [`Alignment`] already draws, and for the same reason.
/// `Start` and `End` follow the direction of the text, so a paragraph indented
/// from the start edge is indented on the right in Arabic and on the left in
/// English. `Left` and `Right` do not follow anything: they are edges of the
/// page, and an inset measured from one of them lands wherever it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Inset {
    Start,
    End,
    Left,
    Right,
}

impl Inset {
    /// Whether this inset is coherent with an RTL base direction.
    ///
    /// The start edge of right-to-left text is the right one, so a `Left`
    /// inset puts the indent at the *end* of the line — where a reader who
    /// reads right to left arrives, rather than where they begin. `Right` is
    /// the correct edge by accident and `Start`/`End` are correct by
    /// construction; only `Left` is reported.
    pub fn is_rtl_coherent(self) -> bool {
        !matches!(self, Self::Left)
    }
}

impl fmt::Display for Inset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Start => "start",
            Self::End => "end",
            Self::Left => "left",
            Self::Right => "right",
        })
    }
}

/// What a document says about how the bidirectional algorithm should treat one
/// run of a unit's text.
///
/// The algorithm is the default and the right answer almost always: it resolves
/// a run's order from the characters and the base direction, and a document
/// that says nothing gets it. The other two variants are the document taking
/// that decision away, in the two different ways a document can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "bidi", content = "direction"))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SpanBidi {
    /// Nothing stated: the run is resolved together with everything around it,
    /// which is what a plain `<span>`, a `<b>` or an OOXML run gets.
    Plain,
    /// An island. Nothing inside changes the order outside it and nothing
    /// outside changes the order within — `<bdi>`, or `unicode-bidi: isolate`,
    /// or the isolation a browser gives any element carrying `dir`.
    Isolated,
    /// The order is *imposed* rather than resolved: every character in the run
    /// is laid out in this direction whatever the algorithm would have made of
    /// it. `<bdo>`, `unicode-bidi: bidi-override`, and the markup equivalent
    /// of an embedded U+202E RIGHT-TO-LEFT OVERRIDE.
    Imposed(Direction),
}

/// One inline run of a unit's text that the document delimits.
///
/// A paragraph is not a flat string to the format that stored it: PowerPoint
/// and Word build one out of runs, and a web page out of inline elements. The
/// shared model keeps the text flat because every rule written so far judges
/// the paragraph as a whole — but two things can only be said about a *part* of
/// it. Whether the order of a part was imposed rather than resolved, and
/// whether a part is isolated from its surroundings, are both properties of a
/// range rather than of a paragraph, and neither can be inferred from the
/// characters.
///
/// `offset` and `len` are byte offsets into [`TextUnit::text`] as the adapter
/// produced it — the same coordinates `evidence.offenders` uses for a tatweel
/// run, and for the same reason: a reviewer can find the run in the string
/// without counting characters.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Span {
    /// Byte offset of the run's first character in the unit's text.
    pub offset: usize,
    /// Length of the run in bytes.
    pub len: usize,
    pub bidi: SpanBidi,
    /// What delimited the run, for a finding's evidence: `page.html bdo@dir`.
    pub origin: Origin,
}

impl Span {
    pub fn new(offset: usize, len: usize, bidi: SpanBidi, origin: Origin) -> Self {
        Self {
            offset,
            len,
            bidi,
            origin,
        }
    }

    /// The run's own text, or `None` if the range does not fall on character
    /// boundaries of `text`.
    ///
    /// A refusal rather than a panic: an adapter that miscounted must cost a
    /// finding, never a crash on a user's document.
    pub fn text<'a>(&self, text: &'a str) -> Option<&'a str> {
        text.get(self.offset..self.offset.checked_add(self.len)?)
    }
}

/// How a list marker is produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Bullet {
    /// A real list feature of the format.
    Native,
    /// Explicitly suppressed.
    Suppressed,
    /// No list formatting declared.
    None,
}

/// The declared typographic and linguistic properties of one text unit.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Properties {
    pub direction: Resolved<Direction>,
    pub alignment: Resolved<Alignment>,
    /// The edge a leading inset is measured from, where the unit has one.
    ///
    /// Distinct from `alignment`, which decides where the line sits in the
    /// box; this is where the box's own text starts within it. A format that
    /// spells only one of the two — OOXML's indents are direction-relative and
    /// have no physical spelling — leaves this `Unset`.
    pub inset: Resolved<Inset>,
    /// BCP-47 language tag.
    pub language: Resolved<String>,
    /// Complex-script font slot (OOXML `cs`, CSS font stack for Arabic).
    pub complex_font: Resolved<String>,
    pub latin_font: Resolved<String>,
    pub bullet: Bullet,
    /// Present when a container displays its boxes in the reverse of the order
    /// the document stores them, naming what stated the reversal.
    ///
    /// A container's direction already decides which end a reader starts from,
    /// and stating it is how the layout and the reading order stay the same
    /// fact. A separate reversal is a second, silent answer to the same
    /// question — the layout moves and nothing else does — which is why the
    /// model records it rather than folding it into `direction`.
    pub reversed: Option<Origin>,
}

impl Default for Properties {
    fn default() -> Self {
        Self {
            direction: Resolved::Unset,
            alignment: Resolved::Unset,
            inset: Resolved::Unset,
            language: Resolved::Unset,
            complex_font: Resolved::Unset,
            latin_font: Resolved::Unset,
            bullet: Bullet::None,
            reversed: None,
        }
    }
}

/// Opaque, adapter-owned address of a text unit.
///
/// The engine only ever echoes this back when proposing a fix; it never parses it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UnitId(pub String);

impl fmt::Display for UnitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Human-facing position, for reports.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Location {
    /// Package part, file, or page: `ppt/slides/slide1.xml`.
    pub part: String,
    /// 1-based ordinal within the part.
    pub paragraph: Option<usize>,
    /// Enclosing shape, table cell, or element, when the format names one.
    pub container: Option<String>,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.part)?;
        if let Some(p) = self.paragraph {
            write!(f, ":paragraph-{p}")?;
        }
        if let Some(c) = &self.container {
            write!(f, ":{c}")?;
        }
        Ok(())
    }
}

/// What a unit is, so a rule can say which kind it judges.
///
/// A paragraph is the unit the rules were written for. Every other kind is a
/// *container*: something whose own direction decides which side its contents
/// start on, while the paragraphs inside it keep their own direction and
/// alignment and remain units in their own right. A container's text is the
/// text it lays out, so the same "mostly Arabic" judgement applies (ADR 0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum UnitKind {
    #[default]
    Paragraph,
    /// A table, whose direction decides which side the first column sits on.
    Table,
    /// A text body laid out in two or more columns, whose direction decides
    /// which column the reader starts in.
    Columns,
    /// A chart container — an axis, a legend, a set of data labels — whose
    /// direction decides how every string it draws is laid out. Its strings
    /// are not paragraphs: they are the values the chart caches, and the
    /// container's text properties are the only place their direction can be
    /// stated.
    ChartText,
}

/// One directional run of text plus the properties governing how it renders.
#[derive(Debug, Clone)]
pub struct TextUnit {
    pub id: UnitId,
    pub kind: UnitKind,
    /// Logical-order Unicode. Never visually reordered, never pre-shaped.
    pub text: String,
    pub props: Properties,
    /// The inline runs the document delimits within `text`, in document order.
    ///
    /// Empty for an adapter that has nothing to say about the parts of a
    /// paragraph, which is not the same as a paragraph made of one plain run:
    /// the rules that read this ask what the document *stated* about a range,
    /// and an adapter that states nothing leaves them silent rather than
    /// answering for it.
    pub spans: Vec<Span>,
    pub location: Location,
}

impl TextUnit {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: UnitId(id.into()),
            kind: UnitKind::Paragraph,
            text: text.into(),
            props: Properties::default(),
            spans: Vec::new(),
            location: Location::default(),
        }
    }

    pub fn with_kind(mut self, kind: UnitKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_props(mut self, props: Properties) -> Self {
        self.props = props;
        self
    }

    pub fn with_spans(mut self, spans: Vec<Span>) -> Self {
        self.spans = spans;
        self
    }

    pub fn with_location(mut self, location: Location) -> Self {
        self.location = location;
        self
    }
}
