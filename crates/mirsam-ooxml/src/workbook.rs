//! Excel's style chain: where a cell's formatting is written, and what it
//! inherits.
//!
//! A cell states almost nothing about itself. `<c r="B5" s="3"/>` carries one
//! number, and that number is an index into `xl/styles.xml` — the record that
//! actually holds the alignment, the reading order and the typeface. The
//! record may itself defer to a *named cell style* through `@xfId`, and what
//! neither of them states about direction is decided once for the whole sheet
//! by `sheetView/@rightToLeft`. This module reads those sources and fills in
//! what the cell did not say, exactly as [`crate::inherit`] does for
//! PowerPoint's masters and [`crate::style`] for Word's named styles, over a
//! vocabulary that shares not one element name with either.
//!
//! ## The order, nearest first
//!
//! 1. The cell's own format record — `cellXfs/xf[@s]`, and the
//!    `fonts/font[@fontId]` it points at. Direct formatting: a value here is
//!    [`Resolved::Explicit`], because the cell's `@s` is the cell's own word
//!    even though the bytes live in another part.
//! 2. The named cell style that record defers to — `cellStyleXfs/xf[@xfId]`,
//!    which is the `Normal` style in almost every workbook. A value from here
//!    is [`Resolved::Inherited`] naming `xl/styles.xml`.
//! 3. For direction only, `sheetView/@rightToLeft`, read by [`crate::xlsx`]
//!    from the worksheet part and passed in: it is the sheet's word for every
//!    cell in it, and it is [`Resolved::Inherited`] naming the sheet.
//!
//! ## `applyAlignment` and `applyFont` are read, and that is the cautious way
//!
//! ECMA-376 gives each `xf` a family of `apply*` flags saying whether the
//! formatting written beside them is the formatting Excel uses. An `<alignment
//! horizontal="left"/>` under `applyAlignment="0"` is a value the application
//! ignores — and reporting `alignment-incoherent` on formatting nobody sees is
//! precisely the false positive this project treats as worse than a miss. So an
//! explicit `0` suppresses the value beside it.
//!
//! An *absent* flag does not: many writers omit it and mean the formatting they
//! wrote, and reading an omission as suppression would silence findings on real
//! defects. The asymmetry is deliberate and runs the safe way in both
//! directions — a flag that is present is obeyed, and an absence is never read
//! as a decision.
//!
//! ## The one thing SpreadsheetML cannot say
//!
//! There is no language tag on a cell, a run, a font or a style: the schema has
//! no slot for one. The only place a workbook states a language is
//! `docProps/core.xml`, whose `dc:language` is the document's, and that is what
//! [`Workbook::language`] returns — inherited, naming that part, for every cell
//! in the file. A repair does not write it (see [`crate::xlsx`]): one tag
//! answers for the whole workbook, so setting it to satisfy an Arabic cell
//! would also relabel every English one.

use mirsam_core::error::Result;
use mirsam_core::text::{Alignment, Direction, Origin, Resolved};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use crate::package::Package;
use crate::rels::RelationshipGraph;
use crate::token::is_true;

/// The part every workbook writes its formatting into.
pub const STYLES: &str = "xl/styles.xml";

/// The part shared cell text lives in.
pub const SHARED_STRINGS: &str = "xl/sharedStrings.xml";

/// The OPC core properties, which is where a workbook's language is stated.
pub const CORE_PROPERTIES: &str = "docProps/core.xml";

/// Read an attribute's normalised value, the same way the part scanners do.
fn attribute(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

/// The same, discarding an attribute that is present but empty.
fn non_empty_attribute(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    attribute(tag, name).filter(|v| !v.is_empty())
}

/// A non-negative index attribute — `@s`, `@fontId`, `@xfId`.
fn index_attribute(tag: &BytesStart<'_>, name: &str) -> Option<usize> {
    attribute(tag, name)?.trim().parse().ok()
}

/// One text event's characters, with entity references resolved.
///
/// An entity this parser cannot resolve keeps its raw form rather than
/// vanishing: dropping it would shorten a string the report gives byte offsets
/// into. The same choice [`crate::token::read_content`] makes.
pub(crate) fn unescaped(text: &quick_xml::events::BytesText<'_>) -> String {
    let raw = text.xml10_content();
    match quick_xml::escape::unescape(raw.as_ref()) {
        Ok(decoded) => decoded.into_owned(),
        Err(_) => raw.into_owned(),
    }
}

/// Whether an `apply*` flag permits the formatting written beside it.
///
/// Absent means yes; see the module documentation for why the asymmetry runs
/// this way.
fn applies(tag: &BytesStart<'_>, name: &str) -> bool {
    attribute(tag, name).is_none_or(|v| is_true(&v))
}

/// Alignment values `alignment/@horizontal` understands.
///
/// `general` is left out on purpose rather than mapped to
/// [`Alignment::Start`]. It is not the start edge: it is Excel's *type*-driven
/// default, which puts text at the start edge and numbers at the end one, and
/// a cell carrying it has stated nothing about where its text sits. `Unset` is
/// that sentence, and reporting it as a direction-relative alignment somebody
/// chose would silence `alignment-unset` on the commonest cell in any workbook.
///
/// `fill` and `centerContinuous` are left out for the plainer reason that
/// neither names an edge: one repeats the text across the cell's width and the
/// other centres it across a span of cells.
pub(crate) fn parse_alignment(value: &str) -> Option<Alignment> {
    Some(match value {
        "left" => Alignment::Left,
        "right" => Alignment::Right,
        "center" => Alignment::Center,
        "justify" => Alignment::Justify,
        "distributed" => Alignment::Distributed,
        _ => return None,
    })
}

/// The direction an `ST_ReadingOrder` value names.
///
/// `0` is *context dependent*, which asks the application to take the
/// direction from the first strong character — the same sentence
/// [`Resolved::Unset`] already carries, and the same reading the HTML adapter
/// gives `dir="auto"`. It is `None` here rather than a direction, so Arabic
/// under it stays a `direction-unset` warning instead of passing as a decision
/// somebody made.
pub(crate) fn parse_reading_order(value: &str) -> Option<Direction> {
    match value.trim() {
        "1" => Some(Direction::Ltr),
        "2" => Some(Direction::Rtl),
        _ => None,
    }
}

/// SpreadsheetML's spelling of an alignment, for a repair.
pub(crate) fn horizontal(alignment: Alignment, rtl: bool) -> &'static str {
    match alignment {
        Alignment::Left => "left",
        Alignment::Right => "right",
        Alignment::Center => "center",
        Alignment::Justify => "justify",
        Alignment::Distributed => "distributed",
        // `horizontal` names physical edges only, so a direction-relative
        // alignment is lowered against the direction the rule reasoned about
        // — the same lowering `crate::rewrite` performs for DrawingML.
        Alignment::Start if rtl => "right",
        Alignment::Start => "left",
        Alignment::End if rtl => "left",
        Alignment::End => "right",
    }
}

/// What one `<xf>` record states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Format {
    /// `@xfId`: the named cell style this record defers to. `cellXfs` only —
    /// a `cellStyleXfs` record is already the end of the chain.
    style: Option<usize>,
    /// `@fontId`, when `applyFont` permits it.
    font: Option<usize>,
    /// `alignment/@horizontal`, when `applyAlignment` permits it and the value
    /// names an edge.
    alignment: Option<Alignment>,
    /// `alignment/@readingOrder`, on the same terms.
    direction: Option<Direction>,
}

/// One worksheet, as `xl/workbook.xml` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    /// The name on the tab, which is how a person finds the cell a finding
    /// names.
    pub name: String,
    /// The part holding it: `xl/worksheets/sheet1.xml`.
    pub part: String,
}

/// Everything outside a worksheet that decides what a cell in it looks like.
#[derive(Debug, Clone, Default)]
pub struct Workbook {
    sheets: Vec<Sheet>,
    /// `xl/sharedStrings.xml`, resolved to text and indexed as a cell's `<v>`
    /// indexes it.
    strings: Vec<String>,
    /// `cellXfs`, the per-cell format records.
    cell_formats: Vec<Format>,
    /// `cellStyleXfs`, the named cell styles those records defer to.
    style_formats: Vec<Format>,
    /// `fonts/font/name@val`, by font id.
    fonts: Vec<Option<String>>,
    /// `docProps/core.xml` `dc:language`.
    language: Option<String>,
}

impl Workbook {
    /// Read every source in one package.
    ///
    /// A part that is absent is not an error: a workbook with no styled cell
    /// has no `xl/styles.xml`, one with no text has no `xl/sharedStrings.xml`,
    /// and a package written by a generator need carry no core properties.
    /// Each of those leaves the corresponding source empty, and an empty
    /// source resolves nothing — which can only cost a finding.
    pub fn read(package: &Package) -> Result<Self> {
        let mut workbook = Self::default();
        let graph = RelationshipGraph::read(package)?;
        if let Some(main) = graph.office_document() {
            let main = main.to_string();
            workbook.read_sheets(package, &graph, &main)?;
        }
        if let Ok(xml) = package.read_text(STYLES) {
            workbook.read_styles(&xml)?;
        }
        if let Ok(xml) = package.read_text(SHARED_STRINGS) {
            workbook.read_strings(&xml)?;
        }
        if let Ok(xml) = package.read_text(CORE_PROPERTIES) {
            workbook.language = read_language(&xml)?;
        }
        Ok(workbook)
    }

    /// Build one from already-parsed XML, so tests and callers holding the
    /// parts need no package on disk.
    pub fn from_xml(
        styles: Option<&str>,
        strings: Option<&str>,
        core: Option<&str>,
    ) -> Result<Self> {
        let mut workbook = Self::default();
        if let Some(xml) = styles {
            workbook.read_styles(xml)?;
        }
        if let Some(xml) = strings {
            workbook.read_strings(xml)?;
        }
        if let Some(xml) = core {
            workbook.language = read_language(xml)?;
        }
        Ok(workbook)
    }

    /// The worksheets, in the order the workbook lists them — which is the
    /// order of the tabs, and so the order a reader meets them.
    pub fn sheets(&self) -> &[Sheet] {
        &self.sheets
    }

    /// The shared string at one index, as a cell's `<v>` names it.
    pub fn shared_string(&self, index: usize) -> Option<&str> {
        self.strings.get(index).map(String::as_str)
    }

    /// The record a cell's `@s` names, and the named style behind it.
    ///
    /// `style` is an index rather than an option because `@s` has a default:
    /// a cell that states none is formatted by record `0`, which is what Excel
    /// draws it with. Reading an absent `@s` as *no record* would leave the
    /// commonest cell in any workbook resolving nothing.
    fn chain(&self, style: usize) -> (Option<&Format>, Option<&Format>) {
        let record = self.cell_formats.get(style);
        let named = record
            .and_then(|f| f.style)
            .and_then(|s| self.style_formats.get(s));
        (record, named)
    }

    /// The direction governing a cell, given the sheet it sits in.
    ///
    /// `sheet_direction` is the worksheet's `sheetView/@rightToLeft` with the
    /// part that stated it, which [`crate::xlsx`] has already read; passing it
    /// in rather than reaching for it keeps this module free of any element a
    /// worksheet declares.
    pub fn direction(
        &self,
        style: usize,
        sheet_direction: Option<(Direction, &str)>,
    ) -> Resolved<Direction> {
        let (record, named) = self.chain(style);
        if let Some(direction) = record.and_then(|f| f.direction) {
            return Resolved::Explicit(direction);
        }
        if let Some(direction) = named.and_then(|f| f.direction) {
            return Resolved::Inherited(
                direction,
                Origin::new(STYLES, "cellStyleXfs/xf/alignment@readingOrder"),
            );
        }
        match sheet_direction {
            Some((direction, part)) => {
                Resolved::Inherited(direction, Origin::new(part, "sheetView@rightToLeft"))
            }
            None => Resolved::Unset,
        }
    }

    /// The alignment governing a cell.
    ///
    /// The sheet has nothing to say here: `rightToLeft` decides which side the
    /// grid starts on, not where text sits inside a cell.
    pub fn alignment(&self, style: usize) -> Resolved<Alignment> {
        let (record, named) = self.chain(style);
        if let Some(alignment) = record.and_then(|f| f.alignment) {
            return Resolved::Explicit(alignment);
        }
        match named.and_then(|f| f.alignment) {
            Some(alignment) => Resolved::Inherited(
                alignment,
                Origin::new(STYLES, "cellStyleXfs/xf/alignment@horizontal"),
            ),
            None => Resolved::Unset,
        }
    }

    /// The typeface governing a cell.
    ///
    /// One name, which answers for every script in the cell: SpreadsheetML has
    /// no second slot for complex-script text the way OOXML's runs do, so this
    /// is what [`crate::xlsx`] puts in *both* slots of the shared model. See
    /// that module for what follows from it.
    pub fn font(&self, style: usize) -> Resolved<String> {
        let (record, named) = self.chain(style);
        if let Some(name) = record
            .and_then(|f| f.font)
            .and_then(|id| self.font_name(id))
        {
            return Resolved::Explicit(name.to_string());
        }
        match named.and_then(|f| f.font).and_then(|id| self.font_name(id)) {
            Some(name) => Resolved::Inherited(
                name.to_string(),
                Origin::new(STYLES, "cellStyleXfs/xf@fontId"),
            ),
            None => Resolved::Unset,
        }
    }

    fn font_name(&self, id: usize) -> Option<&str> {
        self.fonts.get(id)?.as_deref()
    }

    /// The language every cell in this workbook carries.
    ///
    /// Always inherited, and always from the same part, because there is
    /// nowhere else in SpreadsheetML to state one.
    pub fn language(&self) -> Resolved<String> {
        match &self.language {
            Some(tag) => Resolved::Inherited(
                tag.clone(),
                Origin::new(CORE_PROPERTIES, "coreProperties/dc:language"),
            ),
            None => Resolved::Unset,
        }
    }

    /// `xl/workbook.xml`'s `<sheet>` list, with each `r:id` resolved to the
    /// part it names.
    ///
    /// A `<sheet>` whose relationship is missing is dropped rather than
    /// guessed at: an entry naming no part is a part this tool cannot open,
    /// and inventing `xl/worksheets/sheet3.xml` from a `sheetId` would put a
    /// location in a report that the package may not hold.
    fn read_sheets(
        &mut self,
        package: &Package,
        graph: &RelationshipGraph,
        main: &str,
    ) -> Result<()> {
        let xml = package.read_text(main)?;
        let mut reader = Reader::from_str(&xml);
        let mut buf = Vec::new();
        let mut in_sheets = false;
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Eof) | Err(_) => break,
                Ok(Event::Start(e)) if e.name().as_ref() == "sheets" => in_sheets = true,
                Ok(Event::End(e)) if e.name().as_ref() == "sheets" => in_sheets = false,
                Ok(Event::Start(e) | Event::Empty(e))
                    if in_sheets && e.name().as_ref() == "sheet" =>
                {
                    let name = attribute(&e, "name").unwrap_or_default();
                    let Some(id) = non_empty_attribute(&e, "r:id") else {
                        continue;
                    };
                    if let Some(part) = graph.target_of(main, &id) {
                        self.sheets.push(Sheet {
                            name,
                            part: part.to_string(),
                        });
                    }
                }
                _ => {}
            }
            buf.clear();
        }
        Ok(())
    }

    /// `xl/styles.xml`: the font names, and the two tables of format records.
    ///
    /// The section is tracked rather than the element name alone, because
    /// `<font>` occurs in `<dxfs>` as well — a *differential* format, applied
    /// by conditional formatting to whichever cells a rule happens to match.
    /// Reading one as though it were a cell's font would attribute a typeface
    /// to text that may never be drawn in it.
    fn read_styles(&mut self, xml: &str) -> Result<()> {
        #[derive(PartialEq, Eq, Clone, Copy)]
        enum Section {
            None,
            Fonts,
            CellXfs,
            CellStyleXfs,
        }

        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut section = Section::None;
        let mut depth = 0usize;
        // The depth the open section's element sits at, so that a record is
        // read only where it is a *direct* child of it. A `<font>` inside a
        // `<dxfs>` nested in one of these would otherwise be counted as the
        // section's own.
        let mut section_depth = 0usize;
        // The record being built, and whether its `<alignment>` may be read.
        let mut open: Option<(Format, bool)> = None;
        let mut font: Option<Option<String>> = None;

        loop {
            let event = match reader.read_event_into(&mut buf) {
                Ok(Event::Eof) | Err(_) => break,
                Ok(event) => event,
            };
            match &event {
                Event::Start(e) | Event::Empty(e) => {
                    let empty = matches!(event, Event::Empty(_));
                    let name = e.name().as_ref().to_string();
                    match name.as_str() {
                        "fonts" if section == Section::None => {
                            section = Section::Fonts;
                            section_depth = depth;
                        }
                        "cellXfs" if section == Section::None => {
                            section = Section::CellXfs;
                            section_depth = depth;
                        }
                        "cellStyleXfs" if section == Section::None => {
                            section = Section::CellStyleXfs;
                            section_depth = depth;
                        }
                        "font" if section == Section::Fonts && depth == section_depth + 1 => {
                            font = Some(None);
                            if empty {
                                self.fonts.push(None);
                                font = None;
                            }
                        }
                        "name" if font.is_some() => {
                            if let Some(value) = non_empty_attribute(e, "val") {
                                font = Some(Some(value));
                            }
                        }
                        "xf" if matches!(section, Section::CellXfs | Section::CellStyleXfs)
                            && depth == section_depth + 1 =>
                        {
                            let mut record = Format {
                                style: (section == Section::CellXfs)
                                    .then(|| index_attribute(e, "xfId"))
                                    .flatten(),
                                font: applies(e, "applyFont")
                                    .then(|| index_attribute(e, "fontId"))
                                    .flatten(),
                                ..Format::default()
                            };
                            let read_alignment = applies(e, "applyAlignment");
                            if empty {
                                record.alignment = None;
                                self.push_format(section == Section::CellXfs, record);
                            } else {
                                open = Some((record, read_alignment));
                            }
                        }
                        "alignment" => {
                            if let Some((record, read_alignment)) = open.as_mut()
                                && *read_alignment
                            {
                                record.alignment = attribute(e, "horizontal")
                                    .as_deref()
                                    .and_then(parse_alignment);
                                record.direction = attribute(e, "readingOrder")
                                    .as_deref()
                                    .and_then(parse_reading_order);
                            }
                        }
                        _ => {}
                    }
                    if !empty {
                        depth += 1;
                    }
                }
                Event::Text(text) => {
                    // A `<name>` states its typeface in an attribute, never as
                    // content; nothing here reads text. Kept explicit so a
                    // future reader does not mistake silence for an omission.
                    let _ = text;
                }
                Event::End(e) => {
                    depth = depth.saturating_sub(1);
                    match e.name().as_ref() {
                        "fonts" | "cellXfs" | "cellStyleXfs" => section = Section::None,
                        "font" if section == Section::Fonts && depth == section_depth + 1 => {
                            self.fonts.push(font.take().flatten());
                        }
                        "xf" if open.is_some() && depth == section_depth + 1 => {
                            if let Some((record, _)) = open.take() {
                                self.push_format(section == Section::CellXfs, record);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            buf.clear();
        }
        Ok(())
    }

    fn push_format(&mut self, cell: bool, record: Format) {
        if cell {
            self.cell_formats.push(record);
        } else {
            self.style_formats.push(record);
        }
    }

    /// `xl/sharedStrings.xml`: each `<si>` resolved to the text it holds.
    ///
    /// A shared string may be one `<t>` or a sequence of `<r>` runs, and the
    /// cell shows their concatenation, so that is what a unit's text is. The
    /// phonetic runs beside them — `<rPh>`, East Asian reading guides — are
    /// skipped: they are an annotation over the text rather than part of it,
    /// and folding them in would produce a string no reader ever sees.
    fn read_strings(&mut self, xml: &str) -> Result<()> {
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut current: Option<String> = None;
        let mut phonetic = 0usize;
        let mut in_text = false;
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Eof) | Err(_) => break,
                Ok(Event::Start(e)) => match e.name().as_ref() {
                    "si" => current = Some(String::new()),
                    "rPh" => phonetic += 1,
                    "t" if phonetic == 0 => in_text = true,
                    _ => {}
                },
                Ok(Event::End(e)) => match e.name().as_ref() {
                    "si" => {
                        if let Some(text) = current.take() {
                            self.strings.push(text);
                        }
                    }
                    "rPh" => phonetic = phonetic.saturating_sub(1),
                    "t" => in_text = false,
                    _ => {}
                },
                Ok(Event::Text(text)) if in_text => {
                    if let Some(current) = current.as_mut() {
                        current.push_str(&unescaped(&text));
                    }
                }
                _ => {}
            }
            buf.clear();
        }
        Ok(())
    }
}

/// `docProps/core.xml`'s `dc:language`, if it names one.
fn read_language(xml: &str) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut in_language = false;
    let mut tag = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) if e.name().as_ref() == "dc:language" => in_language = true,
            Ok(Event::End(e)) if e.name().as_ref() == "dc:language" => in_language = false,
            Ok(Event::Text(text)) if in_language => tag.push_str(&unescaped(&text)),
            _ => {}
        }
        buf.clear();
    }
    let tag = tag.trim().to_string();
    Ok((!tag.is_empty()).then_some(tag))
}
