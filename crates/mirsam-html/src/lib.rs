//! HTML adapter for mirsam.
//!
//! The web's answer to the same question the OOXML adapters answer: what
//! direction, language and typeface govern this run of text, and did anyone
//! actually decide them? HTML states it in two places at once — the `dir` and
//! `lang` attributes, and the CSS cascade — so this crate reads both and
//! resolves them the way a browser does before handing the result to
//! `mirsam-core`.
//!
//! ```no_run
//! use mirsam_core::DocumentReader;
//! use mirsam_html::HtmlDocument;
//!
//! let mut document = HtmlDocument::open(std::path::Path::new("report.html"))?;
//! for unit in document.scan()? {
//!     println!("{} {}", unit.id, unit.text);
//! }
//! # Ok::<(), mirsam_core::error::Error>(())
//! ```
//!
//! [`DocumentWriter`] is not implemented: this adapter reads. See
//! [`html`] for what is read, what is deliberately not, and what a caller may
//! not conclude from the silence of either.
//!
//! [`DocumentWriter`]: mirsam_core::ports::DocumentWriter

#![forbid(unsafe_code)]

pub mod css;
pub mod dom;
pub mod html;

pub use css::Stylesheet;
pub use dom::Document;
pub use html::{HtmlDocument, Unread};
