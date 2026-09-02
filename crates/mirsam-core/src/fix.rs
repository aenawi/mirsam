//! Declarative repairs.
//!
//! The domain decides *what* must change; an adapter decides *how* to express
//! that in DrawingML, WordprocessingML or CSS. Keeping `Fix` free of format
//! vocabulary is what lets one rule serve every document type.

use crate::text::{Alignment, Direction, UnitId};

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
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

/// A repair bound to the unit it applies to.
#[derive(Debug, Clone)]
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
