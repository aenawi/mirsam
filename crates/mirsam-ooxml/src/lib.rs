//! # mirsam-ooxml
//!
//! Adapter mapping Office Open XML packages onto the `mirsam-core` ports.
//!
//! ## Why a pull parser and not a DOM
//!
//! The job is to change a handful of attributes and leave every other byte
//! exactly as it was. A DOM round-trip cannot promise that: serialising a
//! parsed tree renames namespace prefixes, reorders declarations and rewrites
//! the XML prolog. That matters in OOXML more than in most XML dialects,
//! because Markup Compatibility attributes such as `mc:Ignorable` reference
//! namespace prefixes *by name*, as attribute string values. Rename the prefix
//! and the document becomes invalid in a way no schema check catches.
//!
//! So the adapter streams tokens and passes through everything it is not
//! explicitly asked to change.
//!
//! ## The shared layer and the vocabularies
//!
//! Two modules know nothing about any one OOXML format and are what a second
//! format reuses rather than reimplements: [`package`] holds the ZIP container
//! and its byte-preserving rewrite, and [`token`] the token-stream editing
//! inside a part. Everything else names elements, and an element name belongs
//! to exactly one format: [`rewrite`] is DrawingML's repair vocabulary,
//! [`pptx`] its reader, [`inherit`] the layout/master chain it resolves,
//! [`chart`] the chart parts a deck references, and [`docx`] and [`style`]
//! WordprocessingML's reader and its own style chain — which share
//! [`package`] and [`token`] with the rest and name not one DrawingML
//! element. The theme is the exception both formats really do share: a
//! `a:fontScheme` is DrawingML wherever it is stored, so [`inherit`] reads
//! Word's theme part unchanged.
//!
//! SpreadsheetML is the third vocabulary, and it reuses those same two shared
//! modules and nothing else: [`xlsx`] is Excel's reader and writer,
//! [`workbook`] the format-record chain a cell resolves against, and [`sheet`]
//! its repair vocabulary — what [`rewrite`] is for DrawingML, over a module
//! that has to *append* a record where DrawingML edits an attribute in place,
//! because a cell's formatting is shared between cells and a paragraph's is
//! not.
//!
//! A part a repair does not touch survives byte for byte ([`package`]); a
//! token a repair does not address survives byte for byte ([`token`]). The
//! guarantee is the same one stated at two scales, and a format that reads the
//! package or edits tokens through a second code path would hold it only
//! where that path happened to agree.

pub mod chart;
pub mod docx;
pub mod inherit;
pub mod package;
pub mod pptx;
pub mod rels;
pub mod rewrite;
pub mod sheet;
pub mod style;
pub mod token;
pub mod workbook;
pub mod xlsx;

pub use docx::DocxDocument;
pub use inherit::StyleIndex;
pub use package::Package;
pub use pptx::PptxDocument;
pub use rels::RelationshipGraph;
pub use style::StyleSheet;
pub use workbook::Workbook;
pub use xlsx::XlsxDocument;
