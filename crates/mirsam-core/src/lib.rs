//! # mirsam-core
//!
//! The Arabic script, bidirectional-order and typography correctness engine.
//!
//! This crate is the hexagon's interior: it knows about Unicode, about the
//! bidirectional algorithm and about what correct Arabic typesetting requires,
//! and it knows nothing whatsoever about files, ZIP archives or XML. Document
//! formats reach it by implementing [`ports::DocumentReader`] and lowering
//! their native structure into [`text::TextUnit`]s.
//!
//! ## Why the indirection
//!
//! PPTX, DOCX, XLSX, HTML and PDF disagree about almost everything except the
//! thing that actually matters here: each one ultimately presents a run of
//! text with a base direction, a language and a font. Expressing the rules
//! once against that shared shape is what keeps five format adapters from
//! becoming five divergent rule sets.
//!
//! [`shape`] is where that "and a font" is cashed in: it shapes a run through
//! a real OpenType shaper and reports what came back, so the tool can catch
//! Arabic that is correct in every attribute and still renders as a row of
//! disconnected letters. [`coverage`] asks the question underneath it — does
//! the font have the letter at all? — because a font with no Arabic renders
//! empty boxes that no shaping table would have saved. Both take font *bytes*:
//! finding the file is a [`ports::FontSource`]'s business, and this crate
//! still opens nothing.
//!
//! `font-coverage` and `shaping-broken` are what those two become as findings,
//! and they are the only checks here that ask about the *machine* rather than
//! the document. So [`Engine::with_options`] leaves them registered and unrun,
//! [`Engine::with_fonts`] arms them against a source, and a caller that does
//! not arm them must say so in its report: silence from a check that never ran
//! is not a pass.
//!
//! ```
//! use mirsam_core::{Engine, TextUnit, Direction, Resolved, Properties};
//!
//! // A mixed Arabic/Latin sentence left to render left-to-right.
//! let unit = TextUnit::new("slide1#p1", "ارتفع الأداء بنسبة 25% في Q4 2026.")
//!     .with_props(Properties {
//!         direction: Resolved::Explicit(Direction::Ltr),
//!         ..Default::default()
//!     });
//!
//! let report = Engine::with_default_rules().audit(&[unit]);
//! assert!(report.is_blocking(false));
//! ```

pub mod bidi;
pub mod charname;
pub mod controls;
pub mod coverage;
pub mod diagnostic;
pub mod error;
pub mod fix;
pub mod joining;
pub mod ports;
pub mod rules;
pub mod script;
pub mod shape;
pub mod text;

pub use coverage::{Coverage, MissingChar};
pub use diagnostic::{Diagnostic, Evidence, Report, RuleId, Severity};
pub use error::{Error, Result};
pub use fix::{Fix, Repair};
pub use joining::JoiningForm;
pub use ports::{DocumentReader, DocumentWriter, FontFile, FontSource};
pub use rules::{Engine, RepairOptions, Rule};
pub use shape::{Font, ShapedLetter, Shaping};
pub use text::{
    Alignment, Bullet, Direction, Location, Origin, Properties, Resolved, TextUnit, UnitId,
    UnitKind,
};
