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

pub mod chart;
pub mod package;
pub mod pptx;
pub mod rels;
pub mod rewrite;

pub use package::Package;
pub use pptx::PptxDocument;
pub use rels::RelationshipGraph;
