//! Declarative repairs.
//!
//! The domain decides *what* must change; an adapter decides *how* to express
//! that in DrawingML, WordprocessingML or CSS. Keeping `Fix` free of format
//! vocabulary is what lets one rule serve every document type.

use crate::text::{Alignment, Direction, UnitId};
use std::fmt;

/// One mechanical change to a text unit.
///
/// Serialised adjacently tagged — `{"kind": "set_direction", "value": "rtl"}`
/// — because an internal tag cannot carry a newtype variant whose payload is
/// a string or a list, and `SetDirection` and `RemoveControls` are exactly
/// that. The shape matches [`crate::text::Resolved`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "kind", content = "value", rename_all = "snake_case")
)]
pub enum Fix {
    SetDirection(Direction),
    SetAlignment(Alignment),
    /// BCP-47 tag, e.g. `ar-SA`.
    SetLanguage(String),
    SetComplexFont(String),
    /// Remove explicit bidi controls. Byte offsets into the unit's text,
    /// ascending; the adapter must apply them back-to-front.
    RemoveControls(Vec<usize>),
    /// Replace a typed marker glyph with the format's native list feature.
    ConvertLiteralBullet {
        marker: char,
    },
    /// Replace pre-shaped presentation forms with logical-order codepoints.
    NormalizePresentationForms,
}

impl fmt::Display for Fix {
    /// One line, imperative, for the human repair report.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetDirection(direction) => write!(f, "set direction {direction}"),
            Self::SetAlignment(alignment) => write!(f, "set alignment {alignment}"),
            Self::SetLanguage(tag) => write!(f, "set language {tag}"),
            Self::SetComplexFont(typeface) => write!(f, "set complex-script font {typeface:?}"),
            Self::RemoveControls(offsets) => {
                write!(f, "remove {} explicit bidi control(s)", offsets.len())
            }
            Self::ConvertLiteralBullet { marker } => {
                write!(f, "convert typed {marker:?} to a native bullet")
            }
            Self::NormalizePresentationForms => f.write_str("normalise presentation forms"),
        }
    }
}

/// A repair bound to the unit it applies to.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Repair {
    pub unit: UnitId,
    pub fix: Fix,
}

impl Repair {
    pub fn new(unit: &UnitId, fix: Fix) -> Self {
        Self {
            unit: unit.clone(),
            fix,
        }
    }
}
