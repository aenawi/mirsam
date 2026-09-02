//! Hexagonal boundary: the traits adapters implement.
//!
//! Note the deliberate split between reading and writing. PDF, and any future
//! read-only target, implements [`DocumentReader`] alone and is never obliged
//! to supply a meaningless `apply`. That is Interface Segregation doing real
//! work rather than decoration — the domain genuinely has read-only formats.

use crate::error::Result;
use crate::fix::Repair;
use crate::text::TextUnit;
use std::path::Path;

/// A document that can be lowered into format-agnostic text units.
pub trait DocumentReader {
    /// Stable, human-facing name of the format: `pptx`, `docx`, `html`.
    fn format(&self) -> &'static str;

    /// Lower the document into text units, resolving property inheritance.
    fn scan(&mut self) -> Result<Vec<TextUnit>>;
}

/// A document whose text units can be mechanically repaired.
///
/// Implemented only by formats where a faithful in-place edit is possible.
pub trait DocumentWriter: DocumentReader {
    /// Stage repairs. Implementations must be byte-preserving for every part
    /// of the document not addressed by a repair.
    fn apply(&mut self, repairs: &[Repair]) -> Result<usize>;

    /// Write the repaired document to `dest`.
    ///
    /// Implementations must refuse to overwrite their own source: the original
    /// is always preserved.
    fn write(&mut self, dest: &Path) -> Result<()>;
}
