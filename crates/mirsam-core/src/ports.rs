//! Hexagonal boundary: the traits adapters implement.
//!
//! Note the deliberate split between reading and writing. PDF, and any future
//! read-only target, implements [`DocumentReader`] alone and is never obliged
//! to supply a meaningless `apply`. That is Interface Segregation doing real
//! work rather than decoration — the domain genuinely has read-only formats.

use crate::error::Result;
use crate::fix::{Fix, Repair};
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
    /// Whether this adapter can express `fix` in its own vocabulary.
    ///
    /// Every writer is expected to lower every variant eventually. Until one
    /// does, saying so here lets a caller report the repair as *not made*
    /// instead of discovering it as a failure part-way through [`apply`],
    /// which must reject a fix it cannot express rather than drop it.
    ///
    /// [`apply`]: DocumentWriter::apply
    fn supports(&self, fix: &Fix) -> bool {
        let _ = fix;
        true
    }

    /// Stage repairs, returning how many were staged.
    ///
    /// Implementations must be byte-preserving for every part of the document
    /// not addressed by a repair, and must fail — staging nothing from the
    /// call — on a repair they cannot express, so that the count returned is
    /// never smaller than the caller believes.
    fn apply(&mut self, repairs: &[Repair]) -> Result<usize>;

    /// Write the repaired document to `dest`.
    ///
    /// Implementations must refuse to overwrite their own source: the original
    /// is always preserved.
    fn write(&mut self, dest: &Path) -> Result<()>;
}
