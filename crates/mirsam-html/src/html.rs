//! HTML adapter — the reader.
//!
//! HTML lowered onto the same [`TextUnit`] the PowerPoint and Word adapters
//! produce. Nothing about the *shape* of that model changed to fit the web —
//! a page is paragraphs and containers, exactly as a deck is — which is the
//! claim M5 set out to test, the same way M3 tested it with Word (PLAN §5.1).
//!
//! The model did gain vocabulary in §5.2, and the distinction is worth keeping:
//! [`TextUnit::spans`], [`Properties::inset`] and [`Properties::reversed`] are
//! three things the web can *say* that OOXML cannot, not three things the web
//! needed the abstraction bent for. They are read by rules in `mirsam-core`
//! like every other property, and the other two adapters leave them empty, so
//! the conformance suite carries refusals rather than skips.
//!
//! ## Where an HTML paragraph comes from
//!
//! There is no `<paragraph>` element. What a reader sees as a paragraph is a
//! *block box* with text directly in it, so that is what this adapter emits:
//! every block-level element, carrying the text of its inline content and
//! stopping at the next block-level element inside it. `<div>Hello <b>you</b>
//! <p>Again</p></div>` is two units — `Hello you` and `Again` — which is two
//! boxes on the screen and two runs of text a direction can be wrong about.
//!
//! A `<table>` is a container in HTML exactly as it is in the other two
//! formats, and `<th>`/`<td>` paragraphs inside it stay units in their own
//! right. So is an element laid out in columns: `column-count: 2` puts the
//! first column on the left in a left-to-right box and on the right in a
//! right-to-left one, which is the same statement `a:bodyPr` makes in
//! PowerPoint.
//!
//! ## `dir` is a hint the stylesheet can overrule
//!
//! In a browser, `dir="rtl"` is a rule in the *user agent's* stylesheet, and
//! an author's `direction: ltr` beats it. Reading `dir` as the last word would
//! report the wrong direction for a document whose CSS disagrees with its
//! markup — and would then propose a repair that changes nothing a reader
//! sees. [`crate::css`] therefore runs a real, if small, cascade, and `dir`
//! enters it at the origin a browser gives it.
//!
//! ## `dir="auto"` is the renderer picking, which is what `Unset` means
//!
//! `auto` does not state a direction. It asks the browser to take the first
//! strong character and use its direction, which is precisely the sentence
//! [`Resolved::Unset`] carries — *nothing anywhere; the renderer picks*. So
//! `dir="auto"` lowers to `Unset`, and Arabic under it comes back as
//! `direction-unset`: a **warning**, the fragile tier, not the broken one.
//!
//! That is the honest reading rather than a harsh one. First-strong detection
//! gets `مرحبا 2026` right and `2026 مرحبا` wrong — the second begins with a
//! number, resolves left-to-right, and puts the year on the wrong side — so a
//! paragraph is correct under `auto` when its first strong character happens
//! to agree, which is a property of today's text and not a decision about the
//! element. Marking it `Explicit` would silence the tool on exactly the text
//! it exists to catch.
//!
//! ## CSS has one font stack, so `complex-font-missing` cannot fire here
//!
//! OOXML gives a run two font slots, a Latin one and a complex-script one, and
//! `complex-font-missing` reports the deck that filled the first and left the
//! second empty. CSS has no such pair: one `font-family` list serves every
//! script on the element. The stack is therefore lowered into **both** slots,
//! the rule's precondition — a Latin font set beside an empty complex slot —
//! is unreachable, and the rule is structurally silent on HTML.
//!
//! That is not a gap in this adapter. It is a defect a web author cannot
//! write, in the way a hard left edge is a defect a Word author cannot write,
//! and the conformance suite records it as a refusal rather than a skip.
//!
//! ## What is read, and what a reader should not conclude from its silence
//!
//! Stylesheets are read from `<style>` elements and from `<link
//! rel="stylesheet">` whose `href` is a **relative path on this machine**,
//! resolved against the document. A stylesheet named by an absolute URL is not
//! fetched — this tool performs no network I/O, and inventing one would make
//! an audit's result depend on a server — and one named by a root-relative
//! path (`/css/site.css`) cannot be resolved without knowing the document
//! root. Rules in a sheet that was not read are rules nobody applied, so a
//! property those rules would have set comes back `Unset`, and a
//! `direction-unset` finding on such a document may be answered by CSS the
//! tool never saw. [`HtmlDocument::unread_stylesheets`] names every one of
//! them, so a caller can say so rather than imply otherwise.
//!
//! ## Reading only
//!
//! [`DocumentWriter`] is deliberately not implemented. HTML can be edited
//! faithfully — it is text, and a token-preserving rewrite is the same problem
//! `mirsam-ooxml` already solved — but a writer is its own work item, and
//! claiming a repair this crate cannot yet perform is the kind of inferred
//! capability `AGENTS.md` forbids.
//!
//! [`DocumentWriter`]: mirsam_core::ports::DocumentWriter
//! [`Resolved::Unset`]: mirsam_core::text::Resolved::Unset

use crate::css::{CascadeOrigin, Computed, Declaration, Element, Match, Stylesheet, cascade};
use crate::dom::{Document, Handle, Node};
use mirsam_core::error::{Error, Result};
use mirsam_core::ports::DocumentReader;
use mirsam_core::text::{
    Alignment, Bullet, Direction, Inset, Location, Origin, Properties, Resolved, Span, SpanBidi,
    TextUnit, UnitKind,
};
use std::fs;
use std::path::{Path, PathBuf};

/// Elements that generate a block box, and so a unit of their own.
///
/// The list a browser's default stylesheet gives `display: block`, plus the
/// table parts and the list item. `<template>` is absent: its contents are
/// walked, but the element itself lays nothing out.
const BLOCK: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "caption",
    "dd",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "html",
    "legend",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "ul",
];

/// Elements whose character data is not text a reader sees.
///
/// Script and stylesheet source, and the markup `<noscript>` holds for a
/// browser that will not run the first. Reading any of it would report
/// findings about code.
const NOT_PROSE: &[&str] = &["script", "style", "noscript"];

fn is_block(node: &Node) -> bool {
    node.local_name()
        .is_some_and(|name| BLOCK.contains(&&**name))
}

/// The `href`s of stylesheets this adapter did not read, in document order.
///
/// A fact about what was audited, not a defect: see the module documentation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unread(pub Vec<String>);

// ------------------------------------------------------- selector matching

/// An element as [`crate::css`] asks about it.
struct Subject<'a> {
    node: &'a Node,
    root: bool,
}

impl Element for Subject<'_> {
    fn tag(&self) -> &str {
        self.node.local_name().map_or("", |name| name)
    }

    fn attribute(&self, name: &str) -> Option<String> {
        self.node.attribute(name)
    }

    fn is_root(&self) -> bool {
        self.root
    }
}

// ----------------------------------------------------------- the document

/// An HTML document opened for auditing.
#[derive(Debug, Clone)]
pub struct HtmlDocument {
    /// The name a finding's `location.part` carries: the file, as given.
    part: String,
    source: String,
    /// The directory a relative stylesheet href resolves against. `None` for
    /// a document that came from memory rather than from disk, where a
    /// relative href names nothing this process can find.
    base: Option<PathBuf>,
    unread: Unread,
}

impl HtmlDocument {
    /// Open an HTML file.
    ///
    /// The bytes are decoded as UTF-8. A document in another encoding is
    /// refused rather than lossily decoded: replacement characters would
    /// become Arabic the tool then reported on, and a finding about text
    /// nobody wrote is worse than no finding at all.
    pub fn open(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(Error::NotFound);
        }
        let bytes = fs::read(path)?;
        let source = String::from_utf8(bytes).map_err(|_| {
            Error::Format(format!(
                "{}: not UTF-8. Re-save the document as UTF-8; \
                 mirsam will not guess an encoding and report on the guess",
                path.display()
            ))
        })?;
        let part = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document.html")
            .to_string();
        Ok(Self {
            part,
            source,
            base: path.parent().map(Path::to_path_buf),
            unread: Unread::default(),
        })
    }

    /// A document held in memory, named by the caller.
    ///
    /// There is no directory to resolve a relative stylesheet against, so
    /// every `<link>` is unread and says so.
    pub fn from_source(part: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            part: part.into(),
            source: source.into(),
            base: None,
            unread: Unread::default(),
        }
    }

    /// The stylesheets the last [`scan`] did not read, in document order.
    ///
    /// Empty before the first scan, which is the honest answer: nothing has
    /// been looked at yet.
    ///
    /// [`scan`]: DocumentReader::scan
    pub fn unread_stylesheets(&self) -> &Unread {
        &self.unread
    }

    /// The part name a finding on this document carries.
    pub fn part(&self) -> &str {
        &self.part
    }

    /// Every stylesheet the document states, in the order the cascade applies
    /// them, plus the hrefs that could not be read.
    fn stylesheets(&self, document: &Document) -> (Vec<Stylesheet>, Unread) {
        let mut sheets = Vec::new();
        let mut unread = Vec::new();
        let mut embedded = 0;

        document.walk(|node| {
            if node.is("style") {
                embedded += 1;
                let css: String = node
                    .children()
                    .iter()
                    .filter_map(|child| child.text())
                    .collect();
                let name = if embedded == 1 {
                    "<style>".to_string()
                } else {
                    format!("<style> {embedded}")
                };
                sheets.push(Stylesheet::parse(&css, name));
            }

            if node.is("link") {
                let rel = node.attribute("rel").unwrap_or_default();
                if !rel
                    .split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("stylesheet"))
                {
                    return;
                }
                let Some(href) = node.attribute("href") else {
                    return;
                };
                match self.local_stylesheet(&href) {
                    Some(css) => sheets.push(Stylesheet::parse(&css, href)),
                    None => unread.push(href),
                }
            }
        });

        (sheets, Unread(unread))
    }

    /// Read a linked stylesheet, if it names a file this process can reach.
    ///
    /// A scheme (`https:`, `data:`) or a root-relative path answers `None`:
    /// the first would need the network this tool does not use, and the second
    /// a document root it has not been told.
    fn local_stylesheet(&self, href: &str) -> Option<String> {
        let href = href.split(['?', '#']).next().unwrap_or(href).trim();
        if href.is_empty()
            || href.starts_with('/')
            || href.contains("://")
            || href.split_once(':').is_some_and(|(scheme, _)| {
                scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+')
            })
        {
            return None;
        }
        let base = self.base.as_ref()?;
        fs::read_to_string(base.join(href)).ok()
    }
}

impl DocumentReader for HtmlDocument {
    fn format(&self) -> &'static str {
        "html"
    }

    fn scan(&mut self) -> Result<Vec<TextUnit>> {
        let document = Document::parse(&self.source);
        let (sheets, unread) = self.stylesheets(&document);
        self.unread = unread;

        let mut walk = Walk {
            part: self.part.clone(),
            sheets,
            units: Vec::new(),
            paragraphs: 0,
            tables: 0,
            columns: 0,
            chain: Vec::new(),
            tables_open: Vec::new(),
        };
        for child in document.root.children().iter() {
            walk.descend(child, &Inherited::default());
        }
        Ok(walk.units)
    }

    /// The linked stylesheets this scan did not read. See the module
    /// documentation for which ones those are and why.
    fn unread_sources(&self) -> Vec<String> {
        self.unread.0.clone()
    }
}

// ------------------------------------------------------------ the cascade

/// A property's value as it stands at this point in the tree, and where it was
/// stated.
///
/// CSS calls `direction`, `text-align` and `font-family` *inherited*
/// properties: an element that declares none of them computes to whatever its
/// parent computed. That is the same sentence `Resolved::Inherited` carries,
/// so the walk keeps one of these per property and hands it down.
#[derive(Debug, Clone)]
struct Carried<T> {
    value: Option<T>,
    /// Where the value came from, absent when the element it applies to is the
    /// one that stated it.
    origin: Option<Origin>,
}

/// Nothing stated, by anyone, anywhere — which is where every property starts.
/// Derived `Default` would demand one of `T`, and no direction is the default
/// direction.
impl<T> Default for Carried<T> {
    fn default() -> Self {
        Self {
            value: None,
            origin: None,
        }
    }
}

impl<T: Clone> Carried<T> {
    /// The value as this unit sees it: stated here, or handed down from the
    /// element named in `origin`.
    fn resolved(&self) -> Resolved<T> {
        match (&self.value, &self.origin) {
            (Some(value), None) => Resolved::Explicit(value.clone()),
            (Some(value), Some(origin)) => Resolved::Inherited(value.clone(), origin.clone()),
            (None, _) => Resolved::Unset,
        }
    }

    /// The same value as seen by a *descendant*: whatever it is, it was not
    /// stated there.
    fn descended(&self, here: &Origin) -> Self {
        Self {
            value: self.value.clone(),
            origin: Some(self.origin.clone().unwrap_or_else(|| here.clone())),
        }
    }
}

/// Everything the walk carries down the tree.
#[derive(Debug, Clone, Default)]
struct Inherited {
    direction: Carried<Direction>,
    alignment: Carried<Alignment>,
    language: Carried<String>,
    font: Carried<String>,
    /// The edge this element's own leading inset is measured from.
    ///
    /// Not carried down, unlike everything above it: a margin is not an
    /// inherited CSS property, and a page whose wrapper is indented has not
    /// indented every paragraph inside it. [`Walk::state`] clears this on the
    /// way in for exactly that reason, and a rule that saw it inherited would
    /// report a page gutter as a paragraph's indent.
    inset: Carried<Inset>,
    /// The list marker an `<li>` would get, which `list-style-type` on an
    /// ancestor list decides.
    list_marker_suppressed: bool,
    /// Whether whitespace is preserved, as inside `<pre>`.
    preformatted: bool,
}

impl Inherited {
    /// The same state as a *child* sees it: every value still stated here is
    /// now a value stated by `here`, which is the element the child cites.
    fn descended(&self, here: &Origin) -> Self {
        Self {
            direction: self.direction.descended(here),
            alignment: self.alignment.descended(here),
            language: self.language.descended(here),
            font: self.font.descended(here),
            // See the field: an inset belongs to the box that states it.
            inset: Carried::default(),
            list_marker_suppressed: self.list_marker_suppressed,
            preformatted: self.preformatted,
        }
    }
}

struct TableCursor {
    index: usize,
    row: usize,
    cell: usize,
}

struct Walk {
    part: String,
    sheets: Vec<Stylesheet>,
    units: Vec<TextUnit>,
    paragraphs: usize,
    tables: usize,
    columns: usize,
    chain: Vec<Handle>,
    tables_open: Vec<TableCursor>,
}

impl Walk {
    /// Visit one node, emitting whatever units it and its subtree produce.
    fn descend(&mut self, node: &Handle, inherited: &Inherited) {
        if node.text().is_some() || node.local_name().is_none() {
            return;
        }
        let name = node.local_name().expect("element").to_string();

        if NOT_PROSE.contains(&name.as_str()) {
            return;
        }
        // The head holds metadata, and only the title of it is text a reader
        // sees — in the tab, in a bookmark, in a search result, all of them
        // places Arabic can come out backwards.
        if name == "head" {
            for child in node.children().iter() {
                if child.is("title") {
                    self.descend(child, inherited);
                }
            }
            return;
        }

        self.chain.push(node.clone());
        let computed = self.computed_of(&self.chain);
        let here = self.here(node, &name);
        let inherited = self.state(node, &name, inherited, &computed, &here);

        if name == "table" {
            self.tables += 1;
            self.tables_open.push(TableCursor {
                index: self.tables,
                row: 0,
                cell: 0,
            });
        }
        if name == "tr"
            && let Some(table) = self.tables_open.last_mut()
        {
            table.row += 1;
            table.cell = 0;
        }
        if (name == "td" || name == "th")
            && let Some(table) = self.tables_open.last_mut()
        {
            table.cell += 1;
        }

        if is_block(node) {
            self.emit_paragraph(node, &name, &inherited);
        }

        // What the children see: everything still stated *here* is, to them,
        // stated by this element, and cites it.
        let descended = inherited.descended(&here);
        let mut children: Vec<Handle> = node
            .template_contents()
            .map(|contents| contents.children().iter().cloned().collect())
            .unwrap_or_default();
        children.extend(node.children().iter().cloned());
        for child in children {
            self.descend(&child, &descended);
        }

        // Containers are emitted on the way out, after the paragraphs they
        // lay out, which is the order the other adapters produce.
        if name == "table" {
            let cursor = self.tables_open.pop();
            self.emit_table(node, cursor, &inherited);
        }
        // One unit either way. An element can lay its text out in columns and
        // reverse them as well, and that is one box arranging its contents,
        // not two.
        let reversed = self.reversal(&computed, node, &name);
        if reversed.is_some() || column_count(&computed).is_some_and(|count| count >= 2) {
            self.emit_columns(node, &inherited, reversed);
        }

        self.chain.pop();
    }

    /// The declarations that apply to the element at the end of `chain`.
    ///
    /// Takes the chain rather than reading `self.chain`, because the walk is
    /// not the only thing that asks: gathering a box's text descends into the
    /// inline elements inside it, and each of those has a cascade of its own
    /// that decides whether it isolates or overrides what it holds.
    fn computed_of(&self, chain: &[Handle]) -> Computed {
        let Some(node) = chain.last() else {
            return Computed::default();
        };
        let subjects: Vec<Subject<'_>> = chain
            .iter()
            .map(|handle| Subject {
                node: handle,
                root: handle.is("html"),
            })
            .collect();
        let elements: Vec<&dyn Element> = subjects.iter().map(|s| s as &dyn Element).collect();
        cascade(
            &elements,
            &self.sheets,
            presentational(node),
            &node.attribute("style").unwrap_or_default(),
        )
    }

    /// The `Origin` a descendant cites when it takes a value from this
    /// element's attributes: `div#main@dir`.
    fn here(&self, node: &Handle, name: &str) -> Origin {
        Origin::new(self.part.clone(), format!("{}@", describe(node, name)))
    }

    /// Fold this element's own declarations into what it inherited.
    fn state(
        &self,
        node: &Handle,
        name: &str,
        inherited: &Inherited,
        computed: &Computed,
        here: &Origin,
    ) -> Inherited {
        // `inherited` arrives already stamped with whichever ancestor stated
        // each value, so what is left is to overwrite the ones this element
        // states itself.
        let mut state = Inherited {
            preformatted: inherited.preformatted || name == "pre",
            ..inherited.clone()
        };

        match computed
            .get("direction")
            .and_then(|matched| Some((parse_direction(&matched.declaration.value)?, matched)))
        {
            Some((direction, matched)) => {
                state.direction = stated(direction, matched.origin, &matched.to_string(), here);
            }
            // `dir="auto"` does not inherit — it *replaces* the inherited
            // direction with one the browser computes from the content. So it
            // has to clear what came down the tree, not fall through to it:
            // a paragraph of Arabic marked `auto` under an `ltr` body renders
            // right to left, and reporting it as contradicted-by-the-chain
            // would be a finding on text the browser gets right.
            None if node
                .attribute("dir")
                .is_some_and(|dir| dir.trim().eq_ignore_ascii_case("auto")) =>
            {
                state.direction = Carried::default();
            }
            None => {}
        }
        if let Some(matched) = computed.get("text-align")
            && let Some(alignment) = parse_alignment(&matched.declaration.value)
        {
            state.alignment = stated(alignment, matched.origin, &matched.to_string(), here);
        }
        if let Some(matched) = computed.get("font-family")
            && let Some(family) = first_family(&matched.declaration.value)
        {
            state.font = stated(family, matched.origin, &matched.to_string(), here);
        }
        if let Some((inset, matched)) = inset(computed) {
            state.inset = stated(inset, matched.origin, &matched.to_string(), here);
        }
        if let Some(tag) = node
            .attribute("lang")
            .or_else(|| node.attribute("xml:lang"))
            .filter(|tag| !tag.trim().is_empty())
        {
            state.language = Carried {
                value: Some(tag.trim().to_string()),
                origin: None,
            };
        }
        if let Some(value) = computed
            .value("list-style-type")
            .or_else(|| computed.value("list-style"))
        {
            state.list_marker_suppressed = value.split_ascii_whitespace().any(|v| v == "none");
        }

        state
    }

    /// A box's own inline text, and the runs the markup delimits within it.
    ///
    /// The second half is what `bidi-override` and `isolation-missing` read.
    /// Neither can be answered from the characters — whether a range is
    /// isolated, and whether its order was imposed, are things only the
    /// document says — so every inline element inside the box is recorded with
    /// what its own cascade makes of it.
    fn inline_text(&self, node: &Handle, state: &Inherited) -> (String, Vec<Span>) {
        let mut text = Text::new(state.preformatted);
        let mut chain = self.chain.clone();
        self.collect_runs(node, &mut chain, &mut text);
        text.finish()
    }

    fn collect_runs(&self, node: &Handle, chain: &mut Vec<Handle>, out: &mut Text) {
        for child in node.children().iter() {
            if let Some(text) = child.text() {
                out.push(&text);
                continue;
            }
            let Some(name) = child.local_name() else {
                continue;
            };
            let name = name.to_string();
            if NOT_PROSE.contains(&name.as_str()) || is_block(child) {
                continue;
            }
            if name == "br" {
                out.push(" ");
                continue;
            }

            chain.push(child.clone());
            let start = out.start_of_next();
            let treatment = self.treatment(chain, child, &name);
            self.collect_runs(child, chain, out);
            chain.pop();

            // An element that laid out no characters delimits no run. `<img>`,
            // an empty `<span>` and one holding nothing but whitespace are
            // markup rather than text, and a zero-length range is one no
            // finding could point at.
            if let Some(len) = out.len().checked_sub(start).filter(|len| *len > 0) {
                let (bidi, origin) = treatment;
                out.spans.push(Span::new(start, len, bidi, origin));
            }
        }
    }

    /// What one inline element says about how its content should be ordered.
    ///
    /// `<bdo>` and `<bdi>` are `unicode-bidi` declarations in the user agent's
    /// stylesheet, as is the isolation a browser gives *any* element carrying
    /// `dir`, so all three arrive through the cascade rather than as special
    /// cases here — and an author's own `unicode-bidi` beats them, which is
    /// what a reader would get.
    fn treatment(&self, chain: &[Handle], node: &Handle, name: &str) -> (SpanBidi, Origin) {
        let computed = self.computed_of(chain);
        let element = Origin::new(self.part.clone(), format!("{}@", describe(node, name)));

        let Some(matched) = computed.get("unicode-bidi") else {
            return (SpanBidi::Plain, element);
        };
        let bidi = match matched
            .declaration
            .value
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            // The order is imposed. Which direction it is imposed in is the
            // element's own `direction`, which `dir` supplies for a `<bdo>`.
            "bidi-override" | "isolate-override" => SpanBidi::Imposed(
                computed
                    .value("direction")
                    .and_then(|value| parse_direction(&value))
                    .unwrap_or(Direction::Ltr),
            ),
            // `plaintext` isolates too: it resolves the run on its own first
            // strong character, which is a decision taken inside the run and
            // sealed off from everything outside it.
            "isolate" | "plaintext" => SpanBidi::Isolated,
            // `embed` states a direction without isolating, and `normal` states
            // nothing. Neither seals the run off from its surroundings, which
            // is the only question `isolation-missing` asks.
            _ => SpanBidi::Plain,
        };
        // A declaration the author wrote is cited where they wrote it; the
        // user agent's rules have no selector worth naming, so the element is.
        let origin = match matched.origin {
            CascadeOrigin::Author => Origin::new(self.part.clone(), matched.to_string()),
            CascadeOrigin::Inline | CascadeOrigin::UserAgent => element,
        };
        (bidi, origin)
    }

    fn emit_paragraph(&mut self, node: &Handle, name: &str, state: &Inherited) {
        let (text, spans) = self.inline_text(node, state);
        if text.is_empty() {
            return;
        }
        self.paragraphs += 1;
        let font = state.font.resolved();
        let unit = TextUnit::new(format!("{}#p{}", self.part, self.paragraphs), text)
            .with_props(Properties {
                direction: state.direction.resolved(),
                alignment: state.alignment.resolved(),
                inset: state.inset.resolved(),
                language: state.language.resolved(),
                // One CSS font stack answers for every script on the element;
                // see the module documentation.
                complex_font: font.clone(),
                latin_font: font,
                bullet: self.bullet(name, state),
                reversed: None,
            })
            .with_spans(spans)
            .with_location(Location {
                part: self.part.clone(),
                paragraph: Some(self.paragraphs),
                container: Some(self.container(node, name)),
            });
        self.units.push(unit);
    }

    fn emit_table(&mut self, node: &Handle, cursor: Option<TableCursor>, state: &Inherited) {
        let text = container_text(node, state.preformatted);
        if text.is_empty() {
            return;
        }
        let index = cursor.map_or(self.tables, |cursor| cursor.index);
        self.units.push(
            TextUnit::new(format!("{}#tbl{}", self.part, index), text)
                .with_kind(UnitKind::Table)
                .with_props(Properties {
                    direction: state.direction.resolved(),
                    ..Default::default()
                })
                .with_location(Location {
                    part: self.part.clone(),
                    paragraph: None,
                    container: None,
                }),
        );
    }

    fn emit_columns(&mut self, node: &Handle, state: &Inherited, reversed: Option<Origin>) {
        let text = container_text(node, state.preformatted);
        if text.is_empty() {
            return;
        }
        self.columns += 1;
        self.units.push(
            TextUnit::new(format!("{}#cols{}", self.part, self.columns), text)
                .with_kind(UnitKind::Columns)
                .with_props(Properties {
                    direction: state.direction.resolved(),
                    reversed,
                    ..Default::default()
                })
                .with_location(Location {
                    part: self.part.clone(),
                    paragraph: None,
                    container: None,
                }),
        );
    }

    /// What reverses the order this element displays its boxes in, if anything
    /// does.
    ///
    /// `flex-direction: row-reverse` on a flex container is the web's way of
    /// making a row look right-to-left without saying that it is, which is the
    /// same defect as reversing a string one level up. The `display` is checked
    /// because `flex-direction` on a box that is not a flex container does
    /// nothing at all, and a finding on a declaration nobody applied would be
    /// exactly the failure `crate::css` was written to avoid.
    fn reversal(&self, computed: &Computed, node: &Handle, name: &str) -> Option<Origin> {
        let display = computed.value("display")?;
        if !matches!(display.trim(), "flex" | "inline-flex") {
            return None;
        }
        let matched = computed
            .get("flex-direction")
            .or_else(|| computed.get("flex-flow"))?;
        if !matched
            .declaration
            .value
            .split([' ', '\t'])
            .any(|token| matches!(token.trim(), "row-reverse" | "column-reverse"))
        {
            return None;
        }
        Some(match matched.origin {
            CascadeOrigin::Author => Origin::new(self.part.clone(), matched.to_string()),
            CascadeOrigin::Inline | CascadeOrigin::UserAgent => {
                Origin::new(self.part.clone(), format!("{}@", describe(node, name)))
            }
        })
    }

    /// Whether this paragraph carries a list marker the format produced.
    fn bullet(&self, name: &str, state: &Inherited) -> Bullet {
        match name {
            "li" if state.list_marker_suppressed => Bullet::Suppressed,
            "li" => Bullet::Native,
            _ => Bullet::None,
        }
    }

    /// What encloses this paragraph, for the human location.
    ///
    /// A cell says which cell, in the words the Word adapter already uses, so
    /// a consumer that reads one reads the other. Everything else names the
    /// element the box came from.
    fn container(&self, node: &Handle, name: &str) -> String {
        match self.tables_open.last() {
            Some(cursor) if cursor.row > 0 && cursor.cell > 0 => format!(
                "table {} row {} cell {}",
                cursor.index, cursor.row, cursor.cell
            ),
            _ => describe(node, name),
        }
    }
}

/// Where a declaration puts a value: on this element, or on the ancestor the
/// selector or attribute names.
///
/// A `style` attribute and a `dir` attribute are written *on* the unit, so
/// they are `Explicit`. A stylesheet rule is not: it is a decision taken
/// elsewhere about this element, which is the same relation a Word named style
/// has to a paragraph, and it is cited the same way (ADR 0007).
fn stated<T>(value: T, origin: CascadeOrigin, cited: &str, here: &Origin) -> Carried<T> {
    match origin {
        CascadeOrigin::Inline | CascadeOrigin::UserAgent => Carried {
            value: Some(value),
            origin: None,
        },
        CascadeOrigin::Author => Carried {
            value: Some(value),
            origin: Some(Origin::new(here.part.clone(), cited.to_string())),
        },
    }
}

/// The user-agent declarations an element's presentational markup stands for.
///
/// `dir` and `align` are attributes a browser implements as rules in its own
/// stylesheet; modelling them as such is what lets an author's `direction`
/// beat them, which is what a reader will see.
///
/// The same is true of the three elements and one attribute that decide
/// isolation. The HTML standard's rendering section states them as CSS —
/// `bdo { unicode-bidi: isolate-override }`, `bdi { unicode-bidi: isolate }`,
/// `[dir] { unicode-bidi: isolate }` — and putting them anywhere else would
/// make an author's own `unicode-bidi` lose to markup it beats in a browser.
fn presentational(node: &Handle) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    let declare = |property: &str, value: &str| Declaration {
        property: property.to_string(),
        value: value.to_string(),
        important: false,
    };

    match node.local_name().map(|name| name.to_string()).as_deref() {
        Some("bdo") => declarations.push(declare("unicode-bidi", "isolate-override")),
        Some("bdi") => declarations.push(declare("unicode-bidi", "isolate")),
        // Any element that names a direction is isolated from its
        // surroundings — the browser's rule, and the reason `<span dir="ltr">`
        // is already the repair `isolation-missing` asks for.
        _ if node.attribute("dir").is_some() => {
            declarations.push(declare("unicode-bidi", "isolate"));
        }
        _ => {}
    }

    if let Some(dir) = node.attribute("dir") {
        // `auto` states nothing: see the module documentation.
        if let Some(direction) = parse_direction(&dir) {
            declarations.push(Declaration {
                property: "direction".to_string(),
                value: match direction {
                    Direction::Rtl => "rtl",
                    Direction::Ltr => "ltr",
                }
                .to_string(),
                important: false,
            });
        }
    }

    if let Some(align) = node.attribute("align")
        && parse_alignment(&align).is_some()
    {
        declarations.push(Declaration {
            property: "text-align".to_string(),
            value: align.trim().to_ascii_lowercase(),
            important: false,
        });
    }

    declarations
}

/// A short, stable description of an element for a location or an origin:
/// `div#main`, `p.lead`, `td`.
fn describe(node: &Handle, name: &str) -> String {
    if let Some(id) = node.attribute("id").filter(|id| !id.trim().is_empty()) {
        return format!("{name}#{}", id.trim());
    }
    match node
        .attribute("class")
        .and_then(|class| class.split_ascii_whitespace().next().map(str::to_string))
    {
        Some(class) => format!("{name}.{class}"),
        None => name.to_string(),
    }
}

// ------------------------------------------------------------------- text

/// A box's text as it is gathered, collapsing whitespace the way CSS does so
/// that a run's offsets are offsets into the string a finding will report.
///
/// The collapsing used to happen at the end, over the finished string. It
/// cannot any more: an inline element's text has to be located *within* the
/// paragraph, and a span recorded against the raw characters would name a range
/// of a string nobody ever sees once three spaces became one. Collapsing as the
/// text is gathered keeps every offset in the coordinates the model uses.
struct Text {
    out: String,
    spans: Vec<Span>,
    preformatted: bool,
    /// Whitespace seen since the last character, waiting to become the single
    /// space CSS collapses it to — and never emitted at the start of a box,
    /// which is how the leading whitespace is trimmed without moving anything.
    pending_space: bool,
}

impl Text {
    fn new(preformatted: bool) -> Self {
        Self {
            out: String::new(),
            spans: Vec::new(),
            preformatted,
            pending_space: false,
        }
    }

    fn len(&self) -> usize {
        self.out.len()
    }

    /// Where the next character this box lays out will land.
    ///
    /// Not simply the length. A space waiting to be collapsed in is written by
    /// whoever writes the character *after* it, so an element that begins right
    /// after one would be recorded as starting at a space it did not produce —
    /// and a finding would point one character to the left of the run it is
    /// about.
    fn start_of_next(&self) -> usize {
        self.out.len() + usize::from(self.pending_space)
    }

    fn push(&mut self, text: &str) {
        if self.preformatted {
            self.out.push_str(text);
            return;
        }
        for c in text.chars() {
            if c.is_whitespace() {
                self.pending_space = !self.out.is_empty();
                continue;
            }
            if self.pending_space {
                self.out.push(' ');
                self.pending_space = false;
            }
            self.out.push(c);
        }
    }

    /// The finished text, and the runs within it.
    ///
    /// A `<pre>` is the one case where the string still has to be trimmed after
    /// the fact, so the spans are moved with it rather than left pointing at
    /// characters that are gone.
    fn finish(mut self) -> (String, Vec<Span>) {
        if !self.preformatted {
            return (self.out, self.spans);
        }
        let start = self.out.len() - self.out.trim_start_matches('\n').len();
        // `max`, because a box holding nothing but newlines is trimmed away
        // from both ends and the two offsets cross. An empty box is an empty
        // box; it is not a panic on somebody's document.
        let end = self.out.trim_end_matches('\n').len().max(start);
        self.spans.retain_mut(|span| {
            let (from, to) = (span.offset.max(start), (span.offset + span.len).min(end));
            if from >= to {
                return false;
            }
            span.offset = from - start;
            span.len = to - from;
            true
        });
        (self.out[start..end].to_string(), self.spans)
    }
}

/// The text of an element's own inline content, stopping at the next block.
fn inline_text(node: &Handle, preformatted: bool) -> String {
    let mut text = Text::new(preformatted);
    collect(node, &mut text);
    text.finish().0
}

/// The text a container lays out: every block box inside it, one per line.
///
/// One line per box, because that is what a container *is* — a thing that
/// arranges boxes — and because the other adapters produce exactly this for a
/// table. Running the cells together into one line would change what the
/// bidirectional algorithm resolves over, and the conformance suite would
/// then be comparing two different questions.
fn container_text(node: &Handle, preformatted: bool) -> String {
    fn boxes(node: &Handle, preformatted: bool, lines: &mut Vec<String>) {
        let own = inline_text(node, preformatted);
        if !own.is_empty() {
            lines.push(own);
        }
        for child in node.children().iter() {
            if child
                .local_name()
                .is_some_and(|name| is_block(child) && !NOT_PROSE.contains(&&**name))
            {
                boxes(child, preformatted, lines);
            }
        }
    }

    let mut lines = Vec::new();
    boxes(node, preformatted, &mut lines);
    lines.join("\n")
}

/// Gather one box's own inline characters, without asking what any of the
/// elements inside it declare.
///
/// What a container lays out is text and nothing more, so this is the version
/// the containers use. [`Walk::inline_text`] is the one that also records the
/// runs, and needs the cascade to do it.
fn collect(node: &Handle, out: &mut Text) {
    for child in node.children().iter() {
        if let Some(text) = child.text() {
            out.push(&text);
            continue;
        }
        let Some(name) = child.local_name() else {
            continue;
        };
        // A block is a box of its own and belongs to no other box's text.
        if NOT_PROSE.contains(&&**name) || is_block(child) {
            continue;
        }
        // `<br>` ends a line; the text either side of it is not one word.
        if &**name == "br" {
            out.push(" ");
        }
        collect(child, out);
    }
}

// --------------------------------------------------------------- values

fn parse_direction(value: &str) -> Option<Direction> {
    match value.trim().to_ascii_lowercase().as_str() {
        "rtl" => Some(Direction::Rtl),
        "ltr" => Some(Direction::Ltr),
        _ => None,
    }
}

/// `text-align`, lowered onto the shared vocabulary.
///
/// CSS keeps both spellings and means both things: `left` and `right` are
/// *physical* edges, `start` and `end` direction-relative ones. So HTML can
/// state the hard left edge on Arabic that Word cannot, and
/// `alignment-incoherent` is live on this format.
fn parse_alignment(value: &str) -> Option<Alignment> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" => Some(Alignment::Left),
        "right" => Some(Alignment::Right),
        "center" | "centre" => Some(Alignment::Center),
        "justify" | "justify-all" => Some(Alignment::Justify),
        "start" => Some(Alignment::Start),
        "end" => Some(Alignment::End),
        _ => None,
    }
}

/// The first family of a font stack, unquoted.
///
/// The first is the one the reader gets when it is installed, and every
/// question the font rules ask is about the font that will actually be used.
fn first_family(value: &str) -> Option<String> {
    let family = value
        .split(',')
        .next()?
        .trim()
        .trim_matches(['"', '\''])
        .trim();
    (!family.is_empty()).then(|| family.to_string())
}

/// Which edge this element's leading inset is measured from, and the
/// declaration that decided it.
///
/// Only an *asymmetric* inset is one. A box inset by the same amount on both
/// sides is a gutter, and a gutter is direction-neutral: it looks the same
/// whichever way the text runs, and reporting one would be a finding on a page
/// margin. What this is looking for is the box pushed in from one side only —
/// an indent — because that is the one an author meant to put at the start of
/// the line and a physical property puts on the left.
///
/// The physical pair is checked first: where a box states both, the physical
/// one is the defect and the logical one is not.
fn inset(computed: &Computed) -> Option<(Inset, &Match)> {
    let left = edge(computed, ["margin-left", "padding-left"]);
    let right = edge(computed, ["margin-right", "padding-right"]);
    match (left, right) {
        (Some(matched), None) => return Some((Inset::Left, matched)),
        (None, Some(matched)) => return Some((Inset::Right, matched)),
        _ => {}
    }
    match (
        edge(computed, ["margin-inline-start", "padding-inline-start"]),
        edge(computed, ["margin-inline-end", "padding-inline-end"]),
    ) {
        (Some(matched), None) => Some((Inset::Start, matched)),
        (None, Some(matched)) => Some((Inset::End, matched)),
        _ => None,
    }
}

/// The declaration that insets the box from one edge, if either property does.
fn edge<'a>(computed: &'a Computed, properties: [&str; 2]) -> Option<&'a Match> {
    properties.into_iter().find_map(|property| {
        computed
            .get(property)
            .filter(|matched| insets(&matched.declaration.value))
    })
}

/// Whether a value moves the box's edge at all.
///
/// `0`, `0px` and `auto` do not, and neither does anything this cannot read —
/// a `calc()`, a custom property, a value in units nobody parsed. Every one of
/// those omissions can only cost a finding, which is the test the whole of
/// [`crate::css`] was held to.
fn insets(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    let number: String = value
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))
        .collect();
    number.parse::<f64>().is_ok_and(|length| length != 0.0)
}

/// How many columns the element lays its text out in, if it says.
fn column_count(computed: &Computed) -> Option<u32> {
    if let Some(value) = computed.value("column-count")
        && let Ok(count) = value.trim().parse::<u32>()
    {
        return Some(count);
    }
    // The `columns` shorthand states a width, a count, or both, in either
    // order; the count is the integer.
    let shorthand = computed.value("columns")?;
    shorthand
        .split_ascii_whitespace()
        .find_map(|token| token.parse::<u32>().ok())
}
