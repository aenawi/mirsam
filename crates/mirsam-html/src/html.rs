//! HTML adapter — the reader.
//!
//! HTML lowered onto the same [`TextUnit`] the PowerPoint and Word adapters
//! produce. Nothing new is asked of `mirsam-core`: that is the claim M5 tests,
//! the same way M3 tested it with Word, and a core change needed to make the
//! web fit would mean the abstraction was wrong (PLAN §5.1).
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

use crate::css::{CascadeOrigin, Computed, Declaration, Element, Stylesheet, cascade};
use crate::dom::{Document, Handle, Node};
use mirsam_core::error::{Error, Result};
use mirsam_core::ports::DocumentReader;
use mirsam_core::text::{
    Alignment, Bullet, Direction, Location, Origin, Properties, Resolved, TextUnit, UnitKind,
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
        let computed = self.computed(node);
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
        if let Some(count) = column_count(&computed)
            && count >= 2
        {
            self.emit_columns(node, &inherited);
        }

        self.chain.pop();
    }

    /// The declarations that apply to `node`, cascade already resolved.
    fn computed(&self, node: &Handle) -> Computed {
        let root = node.is("html");
        let subjects: Vec<Subject<'_>> = self
            .chain
            .iter()
            .map(|handle| Subject {
                node: handle,
                root: handle.is("html"),
            })
            .collect();
        let chain: Vec<&dyn Element> = subjects.iter().map(|s| s as &dyn Element).collect();
        let _ = root;
        cascade(
            &chain,
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

    fn emit_paragraph(&mut self, node: &Handle, name: &str, state: &Inherited) {
        let text = inline_text(node, state.preformatted);
        if text.is_empty() {
            return;
        }
        self.paragraphs += 1;
        let font = state.font.resolved();
        let unit = TextUnit::new(format!("{}#p{}", self.part, self.paragraphs), text)
            .with_props(Properties {
                direction: state.direction.resolved(),
                alignment: state.alignment.resolved(),
                language: state.language.resolved(),
                // One CSS font stack answers for every script on the element;
                // see the module documentation.
                complex_font: font.clone(),
                latin_font: font,
                bullet: self.bullet(name, state),
            })
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

    fn emit_columns(&mut self, node: &Handle, state: &Inherited) {
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
                    ..Default::default()
                })
                .with_location(Location {
                    part: self.part.clone(),
                    paragraph: None,
                    container: None,
                }),
        );
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
fn presentational(node: &Handle) -> Vec<Declaration> {
    let mut declarations = Vec::new();

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

/// The text of an element's own inline content, stopping at the next block.
fn inline_text(node: &Handle, preformatted: bool) -> String {
    let mut out = String::new();
    collect(node, &mut out);
    finish(out, preformatted)
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

/// Gather one box's own inline characters, as written. Whitespace is left
/// alone here; [`finish`] is where a box decides whether to collapse it.
fn collect(node: &Handle, out: &mut String) {
    for child in node.children().iter() {
        if let Some(text) = child.text() {
            out.push_str(&text);
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
            out.push(' ');
        }
        collect(child, out);
    }
}

/// Collapse whitespace the way CSS does, unless the box preserves it.
fn finish(text: String, preformatted: bool) -> String {
    if preformatted {
        return text.trim_matches('\n').to_string();
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
