//! PDF object layer for mirsam.
//!
//! The package layer of a format that is not a package. `mirsam-ooxml` opens a
//! ZIP and asks for a part by name; this crate opens a PDF and asks for an
//! object by number — from the body, from inside a compressed object stream,
//! or from a scan of the file when the cross-reference table has gone stale.
//!
//! What it does: classic cross-reference tables and cross-reference streams,
//! `/Prev` chains, hybrid-reference files, object streams, the general-purpose
//! stream filters, the page tree with its inheritable attributes, and content
//! streams decoded and joined.
//!
//! What it deliberately does not do: **decrypt**, **rasterise**, and **write**.
//! An encrypted document is refused by name at [`Pdf::open`], an image codec
//! is named rather than decoded, and there is no writer here or planned — a
//! broken Arabic PDF is rebuilt from its source document, not edited in place.
//! `DocumentWriter` is not implemented, and the absence is the design.
//!
//! ```no_run
//! use mirsam_pdf::Pdf;
//!
//! let pdf = Pdf::open(std::path::Path::new("report.pdf"))?;
//! for page in pdf.pages() {
//!     println!("page {} — {} bytes of content", page.index + 1, page.content(&pdf).len());
//! }
//! for source in pdf.unread() {
//!     println!("not read: {source}");
//! }
//! # Ok::<(), mirsam_core::error::Error>(())
//! ```
//!
//! Text extraction sits on top of this and is PLAN §6.2: what this crate hands
//! over is the object graph and the bytes of the content streams, never a
//! string. A PDF stores glyphs with positions, and turning those back into
//! text is a reconstruction with its own rules about what may honestly be
//! claimed.
//!
//! See [`docs/adr/0010`] for why the parser is written here rather than taken.
//!
//! [`docs/adr/0010`]: https://github.com/aenawi/mirsam/blob/main/docs/adr/0010-write-the-pdf-object-layer-take-the-decompressor.md

#![forbid(unsafe_code)]

pub mod document;
pub mod encrypt;
pub mod filter;
pub mod lexer;
pub mod object;
pub mod page;
pub mod xref;

pub use document::{Pdf, Reconstruction, Unread};
pub use object::{Dictionary, Name, Object, ObjectId, Stream};
pub use page::Page;
