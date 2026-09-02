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
pub mod controls;
pub mod diagnostic;
pub mod error;
pub mod fix;
pub mod ports;
pub mod rules;
pub mod script;
pub mod text;

pub use diagnostic::{Diagnostic, Evidence, Report, RuleId, Severity};
pub use error::{Error, Result};
pub use fix::{Fix, Repair};
pub use ports::{DocumentReader, DocumentWriter};
pub use rules::{Engine, Rule};
pub use text::{Alignment, Bullet, Direction, Location, Properties, Resolved, TextUnit, UnitId};
