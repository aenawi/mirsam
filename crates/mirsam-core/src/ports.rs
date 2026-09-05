//! Hexagonal boundary: the traits adapters implement.
//!
//! Note the deliberate split between reading and writing. PDF, and any future
//! read-only target, implements [`DocumentReader`] alone and is never obliged
//! to supply a meaningless `apply`. That is Interface Segregation doing real
//! work rather than decoration — the domain genuinely has read-only formats.

use crate::error::Result;
use crate::fix::{Fix, Repair};
use crate::shape::Font;
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

/// A font file this machine actually has, and the name it gives itself.
///
/// The bytes are owned because the source that found them is free to forget
/// them: a caller shapes with a font for as long as it holds this, and no
/// longer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFile {
    /// The file's bytes, whole.
    pub data: Vec<u8>,
    /// Which face within a collection: `.ttc` files hold several, and the
    /// index is part of the answer rather than an implementation detail.
    pub index: u32,
    /// Where the bytes came from, as a path on this machine.
    ///
    /// Evidence, and the reason the field exists. "The Arabic will not render"
    /// is a claim a reviewer cannot check; "the Arabic will not render, and
    /// the file that answered for `Calibri` was
    /// `/System/Library/Fonts/Supplemental/Calibri.ttf`" is one they can
    /// (invariant 6).
    pub path: String,
    /// The family the file names itself, which is not always the string that
    /// was asked for: a machine with no `Calibri` may answer with a
    /// substitute, and a finding that reported the requested name alone would
    /// describe a font nobody has.
    pub family: String,
}

impl FontFile {
    /// Parse the bytes for shaping and coverage.
    ///
    /// `None` if this shaper cannot read the file — a fact about the file,
    /// reported by whoever supplied it, and never a finding about a document.
    pub fn font(&self) -> Option<Font<'_>> {
        Font::parse(&self.data, self.index)
    }
}

/// Where the typeface a document names is found on the machine reading it.
///
/// This is the boundary the shaping and coverage checks need and the domain
/// cannot cross. A paragraph names a family — `Properties::complex_font`, or
/// whatever theme or style supplied it — and which file on which machine
/// answers to that name is a question about the world, not about Arabic.
/// Invariant 1 holds: `mirsam-core` states the port and opens nothing.
///
/// A source answering `None` is saying the machine has no such font, which is
/// a real and reportable state — text set in a font nobody has renders in
/// whatever the application substitutes, and the tool can no longer say what
/// the reader will see. It is not an error, and must not be reported as one.
pub trait FontSource {
    /// The file that answers to the typeface `family` names.
    ///
    /// Matching is the implementation's business, and is expected to be at
    /// least case-insensitive: documents say `Arial`, `arial` and
    /// `Arial Regular` for one file.
    fn load(&self, family: &str) -> Result<Option<FontFile>>;
}
