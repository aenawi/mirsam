//! The page tree: §7.7.3, and the four attributes a page inherits down it.
//!
//! A PDF's pages are the leaves of a tree of `/Pages` nodes, and four
//! attributes — `/Resources`, `/MediaBox`, `/CropBox` and `/Rotate` — may be
//! stated on any node and are inherited by everything below it. `/Resources` is
//! the one that matters here: it is where a page's fonts are, and a text layer
//! that read it from the leaf alone would find no fonts at all on the very
//! common document that states them once at the root.
//!
//! So the walk carries them down and a [`Page`] is handed over with them
//! already filled in. A page's own value always wins; an inherited one is only
//! ever an answer to a question the page did not answer itself.
//!
//! ## The tree is walked, and scanned for only if that fails
//!
//! `/Kids` can point at anything, including back up at itself. The walk keeps
//! a set of the nodes it has been through and stops at a repeat, which is the
//! difference between reading a malformed document and hanging on one. If it
//! yields nothing at all — no catalog `/Pages`, or a root whose kids are all
//! missing — the objects are scanned for `/Type /Page` instead, and the
//! document says so through [`crate::Reconstruction::PageTree`].

use std::collections::BTreeSet;

use crate::document::Pdf;
use crate::object::{Dictionary, Name, Object, ObjectId};

/// §7.7.3.4's inheritable page attributes, in the order the specification
/// tabulates them.
const INHERITED: [&str; 4] = ["Resources", "MediaBox", "CropBox", "Rotate"];

/// How deep the tree is walked. A page tree is balanced by every writer that
/// exists; this is a bound on malice, not on documents.
const MAX_DEPTH: usize = 64;

/// One page, with what it inherited already on it.
#[derive(Clone, Debug)]
pub struct Page {
    /// Position in the document, from zero — which is the number a person
    /// counts pages by, and not the `/PageLabels` the document may print.
    pub index: usize,
    pub id: ObjectId,
    /// The page dictionary, plus any of the four inheritable attributes an
    /// ancestor supplied and the page did not state.
    pub dict: Dictionary,
}

impl Page {
    /// The page's resource dictionary, where its fonts are named.
    pub fn resources<'a>(&'a self, pdf: &'a Pdf) -> Option<&'a Dictionary> {
        pdf.get(&self.dict, "Resources").and_then(Object::as_dict)
    }

    /// The page's content streams, decoded and joined.
    ///
    /// `/Contents` is one stream or an array of them, and §7.8.2 says an array
    /// is to be treated as a single stream *concatenated at the boundaries* —
    /// so the join is a newline, without which the last operator of one stream
    /// and the first of the next would run together into a token neither of
    /// them wrote.
    ///
    /// A part behind a filter this crate does not decode contributes nothing
    /// and is recorded in [`Pdf::unread`]: a page drawn as a scanned image
    /// comes back empty *and named*, never merely empty.
    ///
    /// [`Pdf::unread`]: crate::Pdf::unread
    pub fn content(&self, pdf: &Pdf) -> Vec<u8> {
        let Some(contents) = self.dict.get("Contents") else {
            return Vec::new();
        };
        let parts: Vec<&Object> = match pdf.resolve(contents) {
            Object::Array(items) => items.iter().collect(),
            _ => vec![contents],
        };

        let mut out = Vec::new();
        for part in parts {
            let Some(id) = part.as_reference() else {
                // §7.8.2 requires a content stream to be indirect. A direct
                // one is not something to fail over, but there is nothing to
                // decode: a stream object cannot appear inline.
                continue;
            };
            match pdf.decoded(id) {
                Ok(Some(bytes)) => {
                    out.extend_from_slice(&bytes);
                    out.push(b'\n');
                }
                Ok(None) => {}
                Err(e) => pdf.note_unread(Some(id), format!("content stream: {e}")),
            }
        }
        out
    }
}

/// Every page, in reading order, and whether the tree had to be reconstructed.
pub(crate) fn tree(pdf: &Pdf) -> (Vec<Page>, bool) {
    let mut pages = Vec::new();
    let mut seen = BTreeSet::new();

    if let Some(root) = pdf.catalog().get("Pages") {
        let inherited = Dictionary::new();
        descend(pdf, root, &inherited, 0, &mut seen, &mut pages);
    }
    if !pages.is_empty() {
        return (pages, false);
    }

    // Nothing walkable. Take the objects that call themselves pages, in object
    // order — which is the order they were written, and the closest thing to
    // reading order a file with no tree has.
    let mut scanned = Vec::new();
    for (id, object) in pdf.objects() {
        if !object.is_type("Page") {
            continue;
        }
        let Some(dict) = object.as_dict() else {
            continue;
        };
        scanned.push(Page {
            index: scanned.len(),
            id,
            dict: dict.clone(),
        });
    }
    let rebuilt = !scanned.is_empty();
    (scanned, rebuilt)
}

fn descend(
    pdf: &Pdf,
    node: &Object,
    inherited: &Dictionary,
    depth: usize,
    seen: &mut BTreeSet<ObjectId>,
    pages: &mut Vec<Page>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    if let Some(id) = node.as_reference()
        && !seen.insert(id)
    {
        return;
    }
    let id = node.as_reference().unwrap_or_default();
    let Some(dict) = pdf.resolve(node).as_dict() else {
        return;
    };

    // What this node passes down: what it was given, overridden by what it
    // states itself.
    let mut passing = inherited.clone();
    for key in INHERITED {
        if let Some(value) = dict.get(key) {
            passing.insert(Name::from(key), value.clone());
        }
    }

    // A node with `/Kids` is an internal node whatever its `/Type` says, and a
    // great many writers omit `/Type` entirely. Reading the shape rather than
    // the label is the only way to walk those files.
    if let Some(kids) = pdf.get(dict, "Kids").and_then(Object::as_array) {
        for kid in kids {
            descend(pdf, kid, &passing, depth + 1, seen, pages);
        }
        return;
    }

    let mut page = dict.clone();
    for key in INHERITED {
        if !page.contains_key(key)
            && let Some(value) = passing.get(key)
        {
            page.insert(Name::from(key), value.clone());
        }
    }
    pages.push(Page {
        index: pages.len(),
        id,
        dict: page,
    });
}
