//! Format-agnostic text model.
//!
//! Every adapter (PPTX, DOCX, HTML, PDF) lowers its native structure into
//! [`TextUnit`]s so that the rule engine never learns a file format.

use std::fmt;

/// A document property that may be stated outright, inherited from an ancestor
/// (a PowerPoint layout/master, a CSS cascade, a Word style), or genuinely absent.
///
/// Modelling inheritance explicitly is what keeps the engine from reporting an
/// inherited-and-correct value as a missing one — the single largest source of
/// false positives in attribute-only Arabic linters.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "state", content = "value"))]
pub enum Resolved<T> {
    /// Set directly on this unit.
    Explicit(T),
    /// Not set here, but supplied by an ancestor. Correct, and must not be "fixed".
    Inherited(T),
    /// Not set anywhere in the chain. The renderer falls back to its own default.
    Unset,
}

impl<T> Resolved<T> {
    /// The effective value, whether stated here or inherited.
    pub fn effective(&self) -> Option<&T> {
        match self {
            Self::Explicit(v) | Self::Inherited(v) => Some(v),
            Self::Unset => None,
        }
    }

    pub fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }

    pub fn is_explicit(&self) -> bool {
        matches!(self, Self::Explicit(_))
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
    /// BCP-47 language tag.
    pub language: Resolved<String>,
    /// Complex-script font slot (OOXML `cs`, CSS font stack for Arabic).
    pub complex_font: Resolved<String>,
    pub latin_font: Resolved<String>,
    pub bullet: Bullet,
}

impl Default for Properties {
    fn default() -> Self {
        Self {
            direction: Resolved::Unset,
            alignment: Resolved::Unset,
            language: Resolved::Unset,
            complex_font: Resolved::Unset,
            latin_font: Resolved::Unset,
            bullet: Bullet::None,
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
/// A paragraph is the unit the rules were written for. A table is a
/// container whose *direction* is its own — it decides which side the first
/// column sits on — while its cells' paragraphs keep their own direction and
/// alignment and are units in their own right. Its text is every cell's
/// text, so the same "mostly Arabic" judgement applies (ADR 0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum UnitKind {
    #[default]
    Paragraph,
    Table,
}

/// One directional run of text plus the properties governing how it renders.
#[derive(Debug, Clone)]
pub struct TextUnit {
    pub id: UnitId,
    pub kind: UnitKind,
    /// Logical-order Unicode. Never visually reordered, never pre-shaped.
    pub text: String,
    pub props: Properties,
    pub location: Location,
}

impl TextUnit {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: UnitId(id.into()),
            kind: UnitKind::Paragraph,
            text: text.into(),
            props: Properties::default(),
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

    pub fn with_location(mut self, location: Location) -> Self {
        self.location = location;
        self
    }
}
