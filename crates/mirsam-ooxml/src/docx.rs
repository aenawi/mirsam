//! Word (DOCX) adapter — the reader.
//!
//! WordprocessingML's vocabulary, lowered onto the same [`TextUnit`] the
//! PowerPoint adapter produces. Nothing new is asked of `mirsam-core`: that is
//! the claim M3 is testing, and a core change needed to make Word fit would
//! mean the abstraction was wrong (PLAN §3.5).
//!
//! What is read here is the paragraph and the properties the rules judge:
//! `w:p`, its `w:pPr/w:bidi` and `w:pPr/w:jc`, and from the run properties the
//! complex-script language `w:lang/@w:bidi` and the fonts `w:rFonts/@w:cs` and
//! `@w:ascii`. A `w:tbl` is a unit of its own beside them.
//!
//! ## A table is a container, and `w:bidiVisual` is its direction
//!
//! `w:tblPr/w:bidiVisual` says the cells are displayed right to left: the
//! ordering in the file is unchanged, and "the first logical cell with text is
//! stored first in the file format, and displayed on the rightmost"
//! ([ECMA-376] Part 1 §17.4.1). That is the same statement DrawingML spells
//! `a:tblPr/@rtl`, so a Word table lowers onto the same [`UnitKind::Table`]
//! container the PowerPoint adapter produces, under the same id shape
//! (`word/document.xml#tbl1`), and `container-direction` judges it from the
//! text it lays out. **A table needs `w:bidiVisual` exactly where the text in
//! it reads right to left**, which is the rule's question and not this
//! adapter's.
//!
//! The paragraphs in the cells stay units in their own right: Word does not
//! make a cell's text inherit the table's column order, so both have to be
//! right and both are reported separately. What each of them gains here is a
//! location that names its cell — `table 1 row 2 cell 3` — because a finding
//! on one paragraph of a large table is otherwise a hunt.
//!
//! ## Revision records are not the document
//!
//! `w:pPrChange`, `w:tblPrChange` and the rest of the `*Change` family hold
//! the properties as they stood *before* a tracked change, in the same
//! elements that state them now and written after them. Reading one would
//! report a value the author has already replaced — and on a table, would take
//! the column order from the layout somebody has just corrected. Everything
//! inside one is skipped.
//!
//! ## `w:jc` is direction-relative, so this adapter never reports a hard left
//!
//! The standard says the values of `w:jc/@w:val` "are always specified
//! relative to the page, and do not change semantic from right-to-left and
//! left-to-right documents". Word does not implement that. Its own
//! implementation note is explicit: *"Word evaluates the value of this
//! attribute based on the value of the bidi element: Left is the right side of
//! a right-to-left paragraph, and right is the left side of a right-to-left
//! paragraph"* ([MS-OE376] Part 4 §2.3.1.13, note b).
//!
//! So `left` in Word is the *start* edge and `right` is the *end* edge — the
//! same pair ISO 29500 Strict later spelled `start` and `end`. Both forms are
//! lowered to [`Alignment::Start`] and [`Alignment::End`] here, and
//! consequently **no Word paragraph ever produces [`Alignment::Left`]**, so
//! `alignment-incoherent` is structurally silent on DOCX. That is not a gap:
//! a Word author cannot write the defect that rule reports, because the
//! attribute they would have to write to do it is direction-relative. Arabic
//! that starts on the wrong edge in Word is a `w:bidi` defect, and
//! `direction-mismatch` and `direction-unset` are what report it.
//!
//! Mapping `left` onto [`Alignment::Left`] instead would manufacture
//! `alignment-incoherent` on every left-aligned Arabic paragraph in Word,
//! which is invariant 2 — a rule firing on formatting the author chose —
//! reached through the adapter rather than through the rule.
//!
//! ## What the paragraph does not state
//!
//! `w:docDefaults`, the named styles above it and the theme its `@w:cstheme`
//! points at are [`crate::style`]'s, and a property this scanner leaves unset
//! is filled in from there. Without a stylesheet — [`scan_xml`], and any
//! caller holding one part and no package — an unstated property stays
//! `Unset` rather than being guessed at.
//!
//! ## Writing
//!
//! [`DocumentWriter`] lands the repairs, and [`crate::word`] is where every
//! element name they are written with lives — this module groups them by the
//! part and the paragraph a unit id names and hands the rest over.
//!
//! Two of the shape it takes are worth reading before the code. It needs no
//! map of inherited directions, unlike the PowerPoint adapter: `w:jc` is
//! relative to the paragraph's own direction, so there is nothing to lower a
//! `Start` against. And a typed bullet is converted by pointing the paragraph
//! at a list the document already defines, so [`supports`] answers *no* for
//! `ConvertLiteralBullet` on a document that defines none — a `w:numPr` naming
//! a list that does not exist is a document Word offers to repair.
//!
//! [`supports`]: mirsam_core::ports::DocumentWriter::supports
//! [`DocumentWriter`]: mirsam_core::ports::DocumentWriter
//! [ECMA-376]: https://ecma-international.org/publications-and-standards/standards/ecma-376/
//! [MS-OE376]: https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/26ecf09a-0f0b-4574-9907-ebd1ddf3015f

use crate::inherit::{ThemeFont, ThemeScript};
use crate::package::{Edits, Package};
use crate::rels::RelationshipGraph;
use crate::style::{StyleSheet, theme_reference};
use crate::token::is_true;
use crate::word;
use mirsam_core::error::{Error, Result};
use mirsam_core::ports::{DocumentReader, DocumentWriter};
use mirsam_core::text::{
    Alignment, Bullet, Direction, Location, Properties, Resolved, TextUnit, UnitKind,
};
use mirsam_core::{Fix, Repair, UnitId};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Alignment values WordprocessingML understands, as *Word* reads them.
///
/// `left`/`right` are the Transitional spelling of `start`/`end`, not physical
/// edges; see the module documentation for why, and for what follows from it.
/// The kashida forms are Arabic justification and read correctly in either
/// direction, as does `numTab`, which aligns at the numbering tab — the start
/// side of whichever direction the paragraph runs.
pub(crate) fn parse_alignment(value: &str) -> Option<Alignment> {
    Some(match value {
        "start" | "left" | "numTab" => Alignment::Start,
        "end" | "right" => Alignment::End,
        "center" => Alignment::Center,
        "both" | "mediumKashida" | "highKashida" | "lowKashida" => Alignment::Justify,
        "distribute" | "thaiDistribute" => Alignment::Distributed,
        _ => return None,
    })
}

/// Whether an `ST_OnOff` *element* — `w:bidi`, `w:rtl` — is on.
///
/// The attribute is optional and its absence means true: `<w:bidi/>` turns
/// right-to-left layout on, which is the form Word writes far more often than
/// the explicit `w:val="1"`. Reading a missing attribute as false would make
/// the commonest correctly-marked Arabic paragraph in Word look undeclared,
/// and every such paragraph would be reported.
fn on_off_element(tag: &BytesStart<'_>) -> bool {
    attribute(tag, "w:val").is_none_or(|v| is_true(&v))
}

/// One attribute's value, unescaped.
fn attribute(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

/// The same, discarding an attribute that is present but empty — `w:cs=""`
/// names no typeface.
fn non_empty_attribute(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    attribute(tag, name).filter(|v| !v.is_empty())
}

/// The unit id this adapter issues for a paragraph: the part name and the
/// paragraph's 1-based ordinal.
///
/// The same shape the PowerPoint adapter issues, and for the same reason —
/// it is what a rewriter needs to find the paragraph again. Ids stay opaque
/// to the engine either way.
fn unit_id(part: &str, index: usize) -> String {
    format!("{part}#p{index}")
}

/// The unit id this adapter issues for a table: the part name and the table's
/// 1-based ordinal. The same shape `ppt/slides/slide1.xml#tbl2` carries, so a
/// consumer that already echoes PowerPoint's ids needs nothing new for Word's.
fn table_id(part: &str, index: usize) -> String {
    format!("{part}#tbl{index}")
}

/// What a unit id this adapter issued points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Paragraph(usize),
    Table(usize),
}

/// Recover the part and target from an id this adapter issued.
///
/// `#` cannot occur in an OPC part name, so the last `#` is unambiguous, and
/// `tbl` is tried before `p` because `p` is one letter. The same shapes the
/// PowerPoint adapter issues parse the same way here; an id from any other
/// adapter is refused rather than half-understood, because a repair that
/// landed on "whatever paragraph 3 of this part is" would be a repair the
/// report never asked for.
fn parse_unit_id(id: &UnitId) -> Option<(&str, Target)> {
    let (part, rest) = id.0.rsplit_once('#')?;
    let ordinal = |digits: &str| digits.parse::<usize>().ok().filter(|n| *n > 0);

    let target = if let Some(n) = rest.strip_prefix("tbl") {
        Target::Table(ordinal(n)?)
    } else {
        Target::Paragraph(ordinal(rest.strip_prefix('p')?)?)
    };

    (!part.is_empty()).then_some((part, target))
}

/// Accumulates a table while the rows, cells and paragraphs inside it are
/// being parsed.
///
/// Held on a stack for the reason [`ParagraphBuilder`] is: `w:tbl` nests — a
/// table sits in a cell of another — and a single slot would let the inner
/// `</w:tbl>` emit the inner table and leave the outer one with nothing to
/// close.
struct TableBuilder {
    /// The ordinal the unit id carries, fixed when the table opens.
    index: usize,
    /// Every paragraph this table lays out, one per line — including the ones
    /// in a table nested inside it, because the outer table lays those out
    /// too. Each table judges that text under its own direction, and both
    /// have to be right.
    text: String,
    /// `w:tblPr/w:bidiVisual`, or what the table style chain supplies.
    direction: Resolved<Direction>,
    /// `w:tblPr/w:tblStyle/@w:val`: the table style this resolves against.
    style: Option<String>,
    /// 1-based `w:tr` and `w:tc` ordinals, for the location a paragraph in a
    /// cell carries.
    row: usize,
    cell: usize,
}

impl TableBuilder {
    fn new(index: usize) -> Self {
        Self {
            index,
            text: String::new(),
            direction: Resolved::Unset,
            style: None,
            row: 0,
            cell: 0,
        }
    }

    fn push(&mut self, paragraph: &str) {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(paragraph);
    }

    /// Where in this table a paragraph opening now sits, for the location the
    /// finding on it carries.
    ///
    /// `None` until a cell has actually opened: a `w:p` between rows is not
    /// something the schema allows, and inventing coordinates for one would
    /// put a claim in a report that nothing in the file supports.
    fn cell_location(&self) -> Option<String> {
        (self.row > 0 && self.cell > 0)
            .then(|| format!("table {} row {} cell {}", self.index, self.row, self.cell))
    }

    /// The unit, unless the table laid out no text at all.
    ///
    /// The style chain is walked here rather than when `w:tblStyle` is read,
    /// because a stylesheet is what answers it and the caller holds that.
    fn finish(mut self, part: &str, styles: Option<&StyleSheet>) -> Option<TextUnit> {
        if self.text.trim().is_empty() {
            return None;
        }
        if let Some(styles) = styles {
            styles.resolve_table(self.style.as_deref(), &mut self.direction);
        }
        Some(
            TextUnit::new(table_id(part, self.index), self.text)
                .with_kind(UnitKind::Table)
                .with_props(Properties {
                    direction: self.direction,
                    ..Default::default()
                })
                .with_location(Location {
                    part: part.to_string(),
                    paragraph: None,
                    // The table *is* the unit; there is nothing enclosing it
                    // that Word names. A cell inside it is named on the
                    // paragraphs the cell holds.
                    container: None,
                }),
        )
    }
}

/// Accumulates the properties of the paragraph currently being parsed.
///
/// Held on a stack rather than in a single slot, because WordprocessingML
/// paragraphs nest: a text box is a `w:txbxContent` inside a run, and the
/// paragraphs inside it sit within the paragraph that anchors the box. A
/// single slot would let the inner `</w:p>` emit the inner paragraph and leave
/// the outer one with nothing to close, dropping its text entirely.
#[derive(Default)]
struct ParagraphBuilder {
    /// The ordinal the unit id carries, fixed when the paragraph opens so a
    /// nested one cannot renumber the paragraph enclosing it.
    index: usize,
    text: String,
    props: Properties,
    /// `w:pPr/w:pStyle/@w:val`: the paragraph style this resolves against.
    style: Option<String>,
    /// `w:rPr/w:rStyle/@w:val` of the first run that named one.
    run_style: Option<String>,
    /// A `@w:cstheme` this paragraph's own runs wrote, held apart from the
    /// resolved slot because a theme reference is not a typeface: recording
    /// `minorBidi` as the font would put a slot name in a report as though a
    /// reader had that font installed.
    complex_reference: Option<(ThemeFont, ThemeScript)>,
    /// A `@w:cs` this paragraph's own runs wrote. Held rather than written
    /// straight into the properties because the reference beside it wins where
    /// a theme answers it, and which one that is is not known until the
    /// paragraph closes.
    complex_named: Option<String>,
    /// The table cell this paragraph sits in — `table 1 row 2 cell 3` — fixed
    /// when the paragraph opens, so a nested table cannot rewrite it.
    cell: Option<String>,
}

impl ParagraphBuilder {
    /// Settle the complex-script slot, and resolve whatever the paragraph left
    /// unset to the stylesheet above it.
    ///
    /// The theme reference this paragraph's own runs wrote comes first, and
    /// the name beside it is the fallback: `@w:cstheme` is what Word renders,
    /// and `@w:cs` is the resolved value it caches beside it for consumers
    /// that do not implement themes. Both are the paragraph's own statement,
    /// so both outrank every style — a chain that overwrote them would report
    /// a style's font on a paragraph that named one itself.
    ///
    /// `table` is `Some` when this paragraph is inside a table, holding the
    /// style that table names; see [`StyleSheet::resolve`] for where in the
    /// chain it sits.
    fn settle(&mut self, styles: Option<&StyleSheet>, table: Option<Option<&str>>) {
        self.props.complex_font = match (
            self.complex_reference
                .and_then(|r| styles.and_then(|s| s.theme_font(r))),
            self.complex_named.take(),
        ) {
            (Some((name, origin)), _) => Resolved::Inherited(name, origin),
            (None, Some(name)) => Resolved::Explicit(name),
            (None, None) => Resolved::Unset,
        };
        if let Some(styles) = styles {
            styles.resolve(
                self.style.as_deref(),
                self.run_style.as_deref(),
                table,
                &mut self.props,
            );
        }
    }

    fn finish(self, part: &str) -> TextUnit {
        let index = self.index;
        TextUnit::new(unit_id(part, index), self.text)
            .with_props(self.props)
            .with_location(Location {
                part: part.to_string(),
                paragraph: Some(index),
                // Word names no enclosing shape for body text. A table cell
                // is the one thing it does name, and a finding that does not
                // say which cell sends a reviewer through the whole table.
                container: self.cell,
            })
    }
}

/// The state one part's scan carries between events.
#[derive(Default)]
struct PartScan {
    units: Vec<TextUnit>,
    /// Open paragraphs, outermost first. See [`ParagraphBuilder`].
    open: Vec<ParagraphBuilder>,
    /// Paragraphs opened so far, which is what a unit id's ordinal counts.
    seen: usize,
    /// Open tables, outermost first. See [`TableBuilder`].
    tables: Vec<TableBuilder>,
    /// Tables opened so far, which is what a table id's ordinal counts.
    tables_seen: usize,
    in_text: bool,
    /// Open `w:sectPr` elements. A section's `w:bidi` and `w:jc` are the
    /// section's, not the paragraph's — and the last section's `w:sectPr`
    /// lives *inside* a paragraph's `w:pPr`, so without this the section
    /// properties of a document would be read as that paragraph's own.
    section: usize,
    /// Open `mc:Fallback` elements.
    ///
    /// Markup Compatibility says a consumer that understands the `mc:Choice`
    /// ignores the fallback beside it, and both spell out the same text — a
    /// text box's content appears once in each. Reading both would produce two
    /// units for one paragraph, and so report every defect in it twice.
    fallback: usize,
    /// Open revision records — `w:pPrChange`, `w:tblPrChange` and the rest of
    /// the `*Change` family.
    ///
    /// A revision record holds the properties as they were *before* a tracked
    /// change, in the same elements that state them now, and it is written
    /// after them: `w:pPr/w:pPrChange/w:pPr/w:jc` is the alignment the author
    /// replaced. Reading one would report the superseded value as the
    /// document's, and on a `w:tblPr/w:tblPrChange` it would take the table's
    /// direction from the layout somebody has already corrected.
    revision: usize,
}

/// Revision records: elements holding the properties a tracked change
/// replaced. Every `*Change` in the schema that carries a property this
/// adapter reads, plus the section one for symmetry with `w:sectPr`.
const REVISION_RECORDS: [&str; 7] = [
    "w:pPrChange",
    "w:rPrChange",
    "w:sectPrChange",
    "w:tblPrChange",
    "w:tblPrExChange",
    "w:tcPrChange",
    "w:trPrChange",
];

impl PartScan {
    /// Whether events at this point describe content this adapter reads.
    fn reading(&self) -> bool {
        self.fallback == 0 && self.revision == 0
    }

    /// The innermost open paragraph, if any.
    fn current(&mut self) -> Option<&mut ParagraphBuilder> {
        self.open.last_mut()
    }

    fn push_text(&mut self, text: &str) {
        if let Some(b) = self.current() {
            b.text.push_str(text);
        }
    }

    fn close_paragraph(&mut self, part: &str, styles: Option<&StyleSheet>) {
        let Some(mut b) = self.open.pop() else { return };
        // Every open table lays this text out, the outer ones included: a
        // table nested in a cell puts its text on both.
        for table in &mut self.tables {
            table.push(&b.text);
        }
        if b.text.trim().is_empty() {
            return;
        }
        let table = self.tables.last().map(|t| t.style.clone());
        b.settle(styles, table.as_ref().map(Option::as_deref));
        self.units.push(b.finish(part));
    }

    fn close_table(&mut self, part: &str, styles: Option<&StyleSheet>) {
        if let Some(table) = self.tables.pop() {
            self.units.extend(table.finish(part, styles));
        }
    }

    /// Read one start-ish tag.
    ///
    /// `has_content` distinguishes `<w:p>` from `<w:p/>`. The two elements
    /// that open a scope — the paragraph and the run text — must not open one
    /// when they are written self-closing, or the `End` that would have shut
    /// it closes something else instead.
    fn open(&mut self, e: &BytesStart<'_>, has_content: bool) {
        // A section's properties are not the enclosing paragraph's; only
        // `w:p` itself is read through one, and a paragraph cannot occur there.
        let in_section = self.section > 0;
        match e.name().as_ref() {
            "w:p" => {
                // Counted whether or not it holds anything, so an empty
                // paragraph does not shift the ordinals after it.
                self.seen += 1;
                if has_content {
                    let cell = self.tables.last().and_then(TableBuilder::cell_location);
                    self.open.push(ParagraphBuilder {
                        index: self.seen,
                        cell,
                        ..Default::default()
                    });
                }
            }
            // Counted like `w:p`, for the same reason. A self-closing `w:tbl`
            // lays out nothing and so opens no builder; the `End` that would
            // have closed it never arrives.
            "w:tbl" => {
                self.tables_seen += 1;
                if has_content {
                    self.tables.push(TableBuilder::new(self.tables_seen));
                }
            }
            "w:tr" => {
                if let Some(t) = self.tables.last_mut() {
                    t.row += 1;
                    t.cell = 0;
                }
            }
            "w:tc" => {
                if let Some(t) = self.tables.last_mut() {
                    t.cell += 1;
                }
            }
            // `w:tblPr/w:bidiVisual` reverses the columns: the first cell in
            // the file is the one displayed on the right ([ECMA-376] Part 1
            // §17.4.1). That is the same statement DrawingML spells
            // `a:tblPr/@rtl`, so it lowers onto the same container direction.
            //
            // Matched on the element rather than the path because it occurs in
            // exactly two places — a table's `w:tblPr` and a table style's —
            // and only the first has a table open here. A revision record is
            // the third, and `reading()` has already ruled it out.
            "w:bidiVisual" => {
                let direction = if on_off_element(e) {
                    Direction::Rtl
                } else {
                    Direction::Ltr
                };
                if let Some(t) = self.tables.last_mut() {
                    t.direction = Resolved::Explicit(direction);
                }
            }
            // The table style this table resolves against; what it supplies is
            // [`crate::style`]'s to say. First writer wins, as everywhere else
            // here — a `w:tblPr` states it once.
            "w:tblStyle" => {
                let id = non_empty_attribute(e, "w:val");
                if let Some(t) = self.tables.last_mut()
                    && t.style.is_none()
                {
                    t.style = id;
                }
            }
            "w:bidi" if !in_section => {
                let direction = if on_off_element(e) {
                    Direction::Rtl
                } else {
                    Direction::Ltr
                };
                if let Some(b) = self.current() {
                    b.props.direction = Resolved::Explicit(direction);
                }
            }
            "w:jc" if !in_section => {
                let alignment = attribute(e, "w:val").as_deref().and_then(parse_alignment);
                if let (Some(a), Some(b)) = (alignment, self.open.last_mut()) {
                    b.props.alignment = Resolved::Explicit(a);
                }
            }
            // A real list, whatever it draws. `literal-bullet` exists to catch
            // a glyph typed in place of one, and a paragraph that has a list
            // is not that paragraph.
            //
            // `w:numId w:val="0"` inside it says the opposite — it *removes*
            // the list a style would otherwise supply — which is
            // [`Bullet::Suppressed`], and a paragraph that suppressed its list
            // and then typed a glyph is exactly the defect the rule reports.
            "w:numPr" if !in_section => {
                if let Some(b) = self.current() {
                    b.props.bullet = Bullet::Native;
                }
            }
            "w:numId" if !in_section => {
                if let Some(value) = attribute(e, "w:val")
                    && value.trim() == "0"
                    && let Some(b) = self.current()
                {
                    b.props.bullet = Bullet::Suppressed;
                }
            }
            // The style this paragraph resolves against, and the character
            // style its runs do. Both are ids into `word/styles.xml`, not
            // formatting: what they supply is [`crate::style`]'s to say.
            "w:pStyle" if !in_section => {
                let id = non_empty_attribute(e, "w:val");
                if let Some(b) = self.current()
                    && b.style.is_none()
                {
                    b.style = id;
                }
            }
            "w:rStyle" if !in_section => {
                let id = non_empty_attribute(e, "w:val");
                if let Some(b) = self.current()
                    && b.run_style.is_none()
                {
                    b.run_style = id;
                }
            }
            // The complex-script language, not `@w:val`, which is the Latin
            // one: Arabic tagged `en-US` in `@w:val` and `ar-SA` in `@w:bidi`
            // is correctly tagged, and reading the wrong attribute would
            // report it. First writer wins — `w:pPr/w:rPr` describes the
            // paragraph, and the first run is what it goes on to say.
            "w:lang" => {
                let tag = non_empty_attribute(e, "w:bidi");
                if let Some(b) = self.current()
                    && b.props.language.is_unset()
                    && let Some(tag) = tag
                {
                    b.props.language = Resolved::Explicit(tag);
                }
            }
            // `@w:cstheme` names a slot of the theme's font scheme rather than
            // a typeface, so it is kept as a reference and resolved against
            // the theme when the paragraph closes. `@w:asciiTheme` is read by
            // nobody: the Latin slot is not resolved through any chain, for
            // the reason [`crate::style`] states.
            "w:rFonts" => {
                let reference = non_empty_attribute(e, "w:cstheme")
                    .as_deref()
                    .and_then(theme_reference);
                let complex = non_empty_attribute(e, "w:cs");
                let latin = non_empty_attribute(e, "w:ascii");
                if let Some(b) = self.current() {
                    // First writer wins per slot, as `w:lang` does above.
                    if b.complex_reference.is_none() {
                        b.complex_reference = reference;
                    }
                    if b.complex_named.is_none() {
                        b.complex_named = complex;
                    }
                    if b.props.latin_font.is_unset()
                        && let Some(latin) = latin
                    {
                        b.props.latin_font = Resolved::Explicit(latin);
                    }
                }
            }
            "w:t" => self.in_text = has_content,
            _ => {}
        }
    }
}

/// A Word package opened for auditing and repair.
pub struct DocxDocument {
    package: Package,
    /// Parts rewritten by [`DocumentWriter::apply`], awaiting
    /// [`DocumentWriter::write`]. Everything else is copied raw.
    edits: Edits,
    /// The list each typed marker converts to, answered once per marker.
    ///
    /// [`DocumentWriter::supports`] takes `&self` and is asked the same
    /// question for every literal bullet in the document; reading and parsing
    /// `word/numbering.xml` once per finding would be a cost that grows with
    /// the defect count for an answer that cannot change.
    bullets: RefCell<BTreeMap<char, Option<String>>>,
}

impl DocxDocument {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            package: Package::open(path)?,
            edits: Edits::new(),
            bullets: RefCell::new(BTreeMap::new()),
        })
    }

    /// The path this document was opened from.
    pub fn path(&self) -> &Path {
        self.package.path()
    }

    /// The package underneath, for callers that need part-level access.
    pub fn package(&self) -> &Package {
        &self.package
    }

    /// Parts this adapter reads: Word's own XML, excluding relationships.
    ///
    /// Every `word/**/*.xml` part rather than `document.xml` alone, because a
    /// header, a footer, a footnote and a comment all carry `w:p` and all
    /// carry Arabic. The parts that carry none — `styles.xml`, `settings.xml`,
    /// the theme — produce no units, so enumerating widely costs a parse and
    /// risks nothing, while enumerating narrowly would silently skip text.
    fn text_parts(&self) -> Result<Vec<String>> {
        let mut parts = self
            .package
            .parts_where(|n| n.starts_with("word/") && n.ends_with(".xml"))?;
        parts.sort();
        Ok(parts)
    }

    /// The document's style sources, read from the package once.
    ///
    /// Read on demand rather than cached with the document, for the reason
    /// [`crate::pptx::PptxDocument::styles`] gives: a scan reads it once, and
    /// a field every `open` pays for is a cost no other command should carry.
    pub fn styles(&self) -> Result<StyleSheet> {
        StyleSheet::read(&self.package)
    }

    /// One part's XML, taking a staged edit over the package's own bytes so
    /// that repairs applied in two rounds compose rather than the second round
    /// discarding the first.
    fn part_text(&self, part: &str) -> Result<String> {
        match self.edits.get(part) {
            Some(bytes) => {
                String::from_utf8(bytes.clone()).map_err(|e| Error::Format(format!("{part}: {e}")))
            }
            None => self.package.read_text(part),
        }
    }

    /// The `w:numId` a paragraph joins when a typed `marker` is converted, or
    /// `None` when this document defines no bulleted list.
    ///
    /// The numbering part is reached by the relationship pointing at it rather
    /// than by its conventional path, for the reason [`crate::style`] gives
    /// about `styles.xml`: a package is free to store it anywhere, and a
    /// writer that hard-codes `word/numbering.xml` refuses a repair it could
    /// have made on a document that stores it elsewhere.
    ///
    /// An unreadable or malformed numbering part answers `None`. That is the
    /// same answer as "there is no list here", and it is the right one: the
    /// repair is reported as *not made* rather than attempted against a part
    /// this adapter could not understand.
    fn bullet_list(&self, marker: char) -> Option<String> {
        if let Some(known) = self.bullets.borrow().get(&marker) {
            return known.clone();
        }
        let found = self.numbering_part().and_then(|part| {
            let xml = self.package.read_text(&part).ok()?;
            word::bullet_list(&part, &xml, marker).ok().flatten()
        });
        self.bullets.borrow_mut().insert(marker, found.clone());
        found
    }

    /// The part this document defines its lists in.
    fn numbering_part(&self) -> Option<String> {
        let graph = RelationshipGraph::read(&self.package).ok()?;
        let document = graph.office_document()?.to_string();
        graph
            .first_part_of_kind(&document, "numbering")
            .map(str::to_string)
    }

    /// Parse one `word/**/*.xml` part into text units.
    ///
    /// Direction, alignment, the font slots and the language are recorded as
    /// `Explicit` only when the paragraph itself carries them. What it leaves
    /// unset is filled in from `styles` — `w:docDefaults`, the named styles
    /// above it and the theme — as `Inherited`, naming the part and property
    /// that supplied it. Without a stylesheet the chain is simply absent, and
    /// an unresolved property stays `Unset`.
    fn scan_part(part: &str, xml: &str, styles: Option<&StyleSheet>) -> Result<Vec<TextUnit>> {
        let mut reader = Reader::from_str(xml);
        let mut state = PartScan::default();

        loop {
            match reader.read_event() {
                Err(e) => return Err(Error::Format(format!("{part}: {e}"))),
                Ok(Event::Eof) => break,

                Ok(Event::Start(e)) => {
                    // Counted before the skip, so the matching `End` finds the
                    // depth it expects however deeply the two are nested.
                    match e.name().as_ref() {
                        "mc:Fallback" => state.fallback += 1,
                        "w:sectPr" => state.section += 1,
                        name if REVISION_RECORDS.contains(&name) => state.revision += 1,
                        _ => {}
                    }
                    if state.reading() {
                        state.open(&e, true);
                    }
                }

                Ok(Event::Empty(e)) => {
                    if state.reading() {
                        state.open(&e, false);
                    }
                }

                Ok(Event::Text(e)) if state.in_text => {
                    let raw = e.xml10_content();
                    match quick_xml::escape::unescape(raw.as_ref()) {
                        Ok(text) => state.push_text(text.as_ref()),
                        // Unresolvable custom entity: keep the raw form rather
                        // than dropping the run's text entirely.
                        Err(_) => state.push_text(raw.as_ref()),
                    }
                }

                // Word writes Arabic as character references at least as often
                // as PowerPoint does, and quick-xml reports each one as its own
                // event. Ignoring these empties the run, and an empty run is
                // dropped — which turns a defective paragraph into no finding
                // at all.
                Ok(Event::GeneralRef(e)) if state.in_text => {
                    let reference = e.as_ref();
                    match quick_xml::escape::unescape(&format!("&{reference};")) {
                        Ok(text) => state.push_text(text.as_ref()),
                        Err(_) => state.push_text(&format!("&{reference};")),
                    }
                }

                Ok(Event::End(e)) => match e.name().as_ref() {
                    "mc:Fallback" => state.fallback = state.fallback.saturating_sub(1),
                    "w:sectPr" => state.section = state.section.saturating_sub(1),
                    name if REVISION_RECORDS.contains(&name) => {
                        state.revision = state.revision.saturating_sub(1);
                    }
                    "w:t" if state.reading() => state.in_text = false,
                    // Guarded, because a `w:p` inside a fallback would
                    // otherwise close the paragraph that encloses it.
                    "w:p" if state.reading() => state.close_paragraph(part, styles),
                    "w:tbl" if state.reading() => state.close_table(part, styles),
                    _ => {}
                },

                Ok(_) => {}
            }
        }
        Ok(state.units)
    }
}

impl DocumentReader for DocxDocument {
    fn format(&self) -> &'static str {
        "docx"
    }

    fn scan(&mut self) -> Result<Vec<TextUnit>> {
        let styles = self.styles()?;
        let mut units = Vec::new();
        for part in self.text_parts()? {
            let xml = self.part_text(&part)?;
            units.extend(Self::scan_part(&part, &xml, Some(&styles))?);
        }
        Ok(units)
    }
}

impl DocumentWriter for DocxDocument {
    /// Whether WordprocessingML can express `fix`.
    ///
    /// Two refusals, and both are the format's rather than this adapter's.
    ///
    /// A **physical edge** is not something a Word paragraph can state:
    /// `w:jc`'s values are evaluated against the paragraph's own `w:bidi`, so
    /// there is no value meaning "the left of the page whatever the direction"
    /// to write [`Alignment::Left`] as. This is the writing half of the
    /// refusal the conformance suite states for reading, and it is why no
    /// Word unit ever comes back carrying one.
    ///
    /// A **typed bullet** is converted by pointing the paragraph at a list,
    /// and the list has to be one this document already defines. A repair
    /// cannot supply it: [`crate::package`] replaces the entries a package
    /// holds, and a numbering part that is not there needs a part, a
    /// content-type override and a relationship, none of which is an edit to
    /// the paragraph the finding named.
    fn supports(&self, fix: &Fix) -> bool {
        match fix {
            Fix::SetAlignment(Alignment::Left | Alignment::Right) => false,
            Fix::ConvertLiteralBullet { marker } => self.bullet_list(*marker).is_some(),
            _ => true,
        }
    }

    /// Stage repairs against the parts they name.
    ///
    /// Repairs are grouped by part and then by paragraph, so a part is read
    /// and rewritten once however many paragraphs it carries. Nothing is
    /// staged unless every part succeeds: a failure half-way through must not
    /// leave a document that is partly repaired and reports otherwise.
    ///
    /// There is no inheritance pass here, and its absence is Word's doing.
    /// The PowerPoint adapter has to re-run its scanner over each part to
    /// learn the direction every paragraph takes from its container, because
    /// `a:pPr/@algn` names physical edges and a `Start` cannot be written
    /// without one. `w:jc` is already relative to the paragraph's own
    /// direction, so the rewriter needs nothing the paragraph does not carry.
    fn apply(&mut self, repairs: &[Repair]) -> Result<usize> {
        let mut by_part: BTreeMap<String, word::PartPlan> = BTreeMap::new();
        let mut bullets = word::Bullets::new();

        for repair in repairs {
            let Some((part, target)) = parse_unit_id(&repair.unit) else {
                return Err(Error::Format(format!(
                    "{}: not a unit this adapter produced",
                    repair.unit
                )));
            };
            // Resolved here rather than in the rewriter, which holds no
            // package and so cannot see the numbering part. `supports` has
            // already answered this question for every repair the caller
            // staged; a `Fix` that reached `apply` anyway is refused rather
            // than written as a `w:numPr` naming nothing.
            if let Fix::ConvertLiteralBullet { marker } = &repair.fix {
                let Some(num_id) = self.bullet_list(*marker) else {
                    return Err(Error::Format(format!(
                        "{part}: cannot {}; this document defines no bulleted list \
                         for a paragraph to join",
                        repair.fix
                    )));
                };
                bullets.insert(*marker, num_id);
            }

            let plan = by_part.entry(part.to_string()).or_default();
            match target {
                Target::Paragraph(index) => plan.paragraphs.entry(index).or_default(),
                Target::Table(index) => plan.tables.entry(index).or_default(),
            }
            .push(repair.fix.clone());
        }

        let mut staged = Edits::new();
        let mut applied = 0usize;
        for (part, plan) in &by_part {
            // A part edited by an earlier call is edited again from its staged
            // bytes, so repairs applied in two rounds compose.
            let xml = self.part_text(part)?;
            let rewritten = word::apply_plan(part, &xml, plan, &bullets)?;
            applied += plan.len();
            staged.insert(part.clone(), rewritten.into_bytes());
        }

        self.edits.extend(staged);
        Ok(applied)
    }

    fn write(&mut self, dest: &Path) -> Result<()> {
        self.package.rewrite(dest, &self.edits)?;
        Ok(())
    }
}

/// Parse an in-memory part into every unit this adapter produces for it.
///
/// Exposed for tests and for callers that already hold the XML. There is no
/// package here and so no stylesheet: a property the paragraph does not state
/// comes back `Unset`, not `Inherited`. Use [`scan_xml_with`] to resolve one.
pub fn scan_xml(part: &str, xml: &str) -> Result<Vec<TextUnit>> {
    scan_xml_with(part, xml, None)
}

/// The same, resolving each paragraph against a stylesheet the caller holds.
pub fn scan_xml_with(part: &str, xml: &str, styles: Option<&StyleSheet>) -> Result<Vec<TextUnit>> {
    DocxDocument::scan_part(part, xml, styles)
}

#[cfg(test)]
mod unit_id_tests {
    use super::*;

    #[test]
    fn a_unit_id_round_trips_through_its_own_parser() {
        let part = "word/document.xml";
        assert_eq!(
            parse_unit_id(&UnitId(unit_id(part, 3))),
            Some((part, Target::Paragraph(3)))
        );
        assert_eq!(
            parse_unit_id(&UnitId(table_id(part, 2))),
            Some((part, Target::Table(2)))
        );
    }

    #[test]
    fn an_id_this_adapter_did_not_issue_is_rejected() {
        for foreign in [
            "",
            "document",
            "#p1",
            "word/document.xml#p0",
            "x#px",
            "x#p-1",
            "x#tbl0",
            "x#tblx",
            "x#t1",
            // The containers the PowerPoint adapter issues and this one does
            // not: a Word paragraph is not laid out in columns, and a chart in
            // a Word document is a part the DrawingML reader answers for.
            "x#cols1",
            "x#catax1",
            "x#legend1",
            "x#dlbls1",
        ] {
            assert_eq!(parse_unit_id(&UnitId(foreign.into())), None, "{foreign:?}");
        }
    }
}
