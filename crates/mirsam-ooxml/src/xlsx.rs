//! Excel (XLSX) adapter.
//!
//! ## A cell is a paragraph, and the sheet around it is the container
//!
//! This is the question the format had to answer before a line of it could be
//! written, because a spreadsheet is the first format here whose text does not
//! arrive in paragraphs at all.
//!
//! **A cell is a paragraph.** It is one run of text with one set of properties
//! deciding how that text renders, which is exactly what [`TextUnit`] is for. A
//! cell lays out no other unit: what is inside it is characters, not
//! paragraphs, and every container this model has — a table, a multi-column
//! body, a chart axis — is a thing whose direction decides where *other* units
//! begin. A cell is not one of those.
//!
//! **The worksheet is.** `sheetView/@rightToLeft` decides which side column A
//! sits on, which is the same sentence `a:tblPr/@rtl` and `w:tblPr/w:bidiVisual`
//! state in the other two formats, so a sheet is a [`UnitKind::Table`] and
//! carries the id shape a table carries. It is emitted only where the sheet
//! holds text in **two or more columns**, for the reason a text body is a
//! `Columns` container only at `numCol >= 2`: with one column there is no
//! question of which column a reader starts in, and a container reported on a
//! single column of text would be a finding on a margin.
//!
//! **The sheet is also the chain above its cells**, and that is where Excel
//! differs from the other two. Neither PowerPoint nor Word makes a cell's text
//! inherit the table's column order — `AGENTS.md` says so, and the rules rely
//! on it — but `rightToLeft` is not only a column order: it is the reading
//! order every cell in the sheet falls back to when its own format record
//! states none. So a cell with no `readingOrder` of its own resolves to the
//! sheet's, [`Resolved::Inherited`] naming the worksheet part, and is then
//! judged the way ADR 0007 judges any inherited value.
//!
//! ## A formula is a source this adapter cannot read
//!
//! `<c><f>A1&" "&B1</f><v>…</v></c>` stores a *cached result* beside the
//! formula that produced it. The cache is not the document's text: Excel
//! recomputes it on open, so a finding on it is a finding on a string no author
//! wrote and no repair can change — and a re-audit after a recalculation could
//! honestly disagree with the first one. So a formula cell produces no unit.
//!
//! It is not silently dropped either. Standing rule 4 says a source that was
//! not read has to be sayable, and [ADR 0009] is where `unread_sources` came
//! from: a formula cell whose cached result carries Arabic is named in the
//! report as `Sheet1!C5`, so an absent finding on it can never look like a
//! clean one. Cells whose cache holds no Arabic are not named, because there is
//! nothing there this tool would have judged.
//!
//! ## What SpreadsheetML cannot state, and what follows
//!
//! - **No language.** There is no tag on a cell, a run, a font or a style. The
//!   one place a workbook states a language is `docProps/core.xml`, so that is
//!   what every cell inherits ([`crate::workbook`]) — and a repair does not
//!   write it: one tag answers for the whole file, so setting it for an Arabic
//!   cell would relabel every English one too.
//! - **One typeface, not two.** A cell's font is one `name@val` and it answers
//!   for every script in the cell, exactly as a CSS `font-family` does. It is
//!   lowered into *both* slots of the shared model, so `complex-font-missing`'s
//!   precondition — a filled Latin slot beside an empty complex-script one — is
//!   unreachable here for the same reason it is unreachable on HTML.
//! - **No list.** Excel has no list feature: a bullet in a cell is a glyph
//!   somebody typed, always, so `literal-bullet` fires and `--convert-bullets`
//!   has nothing to convert to.
//! - **No physical inset.** `alignment/@indent` is measured from the start edge
//!   like every other OOXML indent, so `inset` stays `Unset`.
//!
//! ## Repairing: the cell's format record is shared
//!
//! `<c s="3"/>` names an index into `cellXfs`, and forty cells may name the
//! same one. Writing the repaired alignment into that record would re-align all
//! forty, including the English text among them. So a repair **appends** a
//! record — the cell's own, with the one attribute changed — and repoints that
//! cell's `@s` at it; identical requests share one appended record rather than
//! each getting its own. See [`crate::sheet`].
//!
//! `<f>` and `xl/workbook.xml`'s `<definedNames>` survive this untouched, and
//! not by care: a repair changes `@s` on a `<c>`, `@rightToLeft` on a
//! `sheetView`, or one `<si>` appended to the shared strings, and
//! [`crate::token`] passes every other byte of the part through as it was.
//! `tests/xlsx.rs` asserts it on a workbook holding both.
//!
//! [ADR 0009]: https://github.com/aenawi/mirsam/blob/main/docs/adr/0009-a-source-the-adapter-could-not-read-is-part-of-the-report.md

use crate::package::{Edits, Package};
use crate::sheet::{self, Inherited, SheetPlan, Strings, StyleTable};
use crate::token::is_true;
use crate::workbook::{SHARED_STRINGS, STYLES, Sheet, Workbook, unescaped};
use mirsam_core::error::{Error, Result};
use mirsam_core::fix::{Fix, Repair};
use mirsam_core::ports::{DocumentReader, DocumentWriter};
use mirsam_core::script;
use mirsam_core::text::{
    Bullet, Direction, Location, Properties, Resolved, TextUnit, UnitId, UnitKind,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The unit id this adapter issues for a cell: the worksheet part and the
/// cell's 1-based ordinal.
///
/// The ordinal counts **every** `<c>` in the sheet, including the numeric and
/// empty ones that produce no unit — exactly as the PowerPoint adapter's
/// paragraph ordinal counts every `a:p`. A numbering that skipped the cells
/// this adapter has nothing to say about would drift the moment a number was
/// typed beside the text, and the rewriter would repair the wrong cell.
fn unit_id(part: &str, index: usize) -> String {
    format!("{part}#p{index}")
}

/// The unit id for a worksheet's grid. One per part, so the ordinal is always
/// `1`; it is written out so the shape matches the `#tbl<n>` every other
/// adapter issues.
fn grid_id(part: &str) -> String {
    format!("{part}#tbl1")
}

/// What a unit id this adapter issued points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Cell(usize),
    Grid,
}

/// Recover the part and target from an id this adapter issued.
///
/// `#` cannot occur in an OPC part name, so the last `#` is unambiguous.
fn parse_unit_id(id: &UnitId) -> Option<(&str, Target)> {
    let (part, rest) = id.0.rsplit_once('#')?;
    let target = if rest == "tbl1" {
        Target::Grid
    } else {
        Target::Cell(
            rest.strip_prefix('p')?
                .parse::<usize>()
                .ok()
                .filter(|n| *n > 0)?,
        )
    };
    (!part.is_empty()).then_some((part, target))
}

/// Read an attribute's normalised value.
fn attribute(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

/// The column an A1-style reference names, zero-based: `A` is 0, `AA` is 26.
///
/// `None` for a reference that names no column, which is a reference this
/// adapter did not understand rather than one it may guess at.
fn column_of(reference: &str) -> Option<usize> {
    let letters: String = reference
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    if letters.is_empty() {
        return None;
    }
    letters
        .bytes()
        .try_fold(0usize, |acc, b| {
            acc.checked_mul(26)?
                .checked_add(usize::from(b.to_ascii_uppercase() - b'A') + 1)
        })
        .map(|n| n - 1)
}

/// One cell, as the worksheet scan accumulates it.
#[derive(Default)]
struct CellScan {
    /// The ordinal the unit id carries, fixed when the cell opens.
    index: usize,
    /// `@r`, the A1 reference. Optional in the schema, which is why the column
    /// falls back to the cell's position in its row.
    reference: Option<String>,
    /// `@t`, the cell's type.
    kind: Option<String>,
    /// `@s`, the index into `cellXfs`. Absent means `0`, which is the record
    /// Excel formats an unstyled cell with.
    style: usize,
    /// Whether the cell carries an `<f>`, and so holds a cached result rather
    /// than text somebody wrote.
    formula: bool,
    /// `<v>` content.
    value: String,
    /// `<is>` content, for an inline string.
    inline: String,
    /// The column the cell is in: from `@r`, or from its position in the row.
    column: usize,
}

/// One cell that produced text.
struct Cell {
    index: usize,
    text: String,
    style: usize,
    column: usize,
    /// `Sheet1!C5`, as a person would type it into the Name Box.
    reference: String,
}

/// One worksheet's reading: its declared grid direction, its cells, and the
/// formula results this adapter did not judge.
struct SheetScan {
    direction: Option<Direction>,
    cells: Vec<Cell>,
    unread: Vec<String>,
}

/// Parse one worksheet part.
fn scan_sheet(name: &str, xml: &str, workbook: &Workbook) -> Result<SheetScan> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();

    let mut direction: Option<Direction> = None;
    let mut seen_view = false;
    let mut in_data = false;
    let mut cells = Vec::new();
    let mut unread = Vec::new();
    let mut seen = 0usize;
    let mut in_row = 0usize;
    let mut open: Option<CellScan> = None;
    let mut in_value = false;
    let mut in_inline = false;
    let mut in_text = false;
    let mut phonetic = 0usize;

    loop {
        let event = match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(event) => event,
            Err(e) => return Err(Error::Format(format!("{name}: {e}"))),
        };
        let empty = matches!(event, Event::Empty(_));
        match &event {
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                // The first view is the one Excel opens on; a second is
                // another window onto the same sheet, and reading it as the
                // sheet's word would let a split pane decide the report.
                "sheetView" if !seen_view => {
                    seen_view = true;
                    direction = attribute(e, "rightToLeft").map(|v| {
                        if is_true(&v) {
                            Direction::Rtl
                        } else {
                            Direction::Ltr
                        }
                    });
                }
                "sheetData" => in_data = true,
                "row" if in_data => in_row = 0,
                "c" if in_data => {
                    seen += 1;
                    let reference = attribute(e, "r");
                    let column = reference.as_deref().and_then(column_of).unwrap_or(in_row);
                    in_row += 1;
                    let cell = CellScan {
                        index: seen,
                        reference,
                        kind: attribute(e, "t"),
                        style: attribute(e, "s")
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0),
                        column,
                        ..CellScan::default()
                    };
                    if empty {
                        // `<c r="A1"/>` is a formatted cell with no content.
                        let _ = cell;
                    } else {
                        open = Some(cell);
                    }
                }
                "f" if open.is_some() => {
                    if let Some(cell) = open.as_mut() {
                        cell.formula = true;
                    }
                }
                "v" if open.is_some() && !empty => in_value = true,
                "is" if open.is_some() => in_inline = true,
                "rPh" if in_inline => phonetic += 1,
                "t" if in_inline && phonetic == 0 && !empty => in_text = true,
                _ => {}
            },
            Event::End(e) => match e.name().as_ref() {
                "sheetData" => in_data = false,
                "v" => in_value = false,
                "is" => in_inline = false,
                "rPh" => phonetic = phonetic.saturating_sub(1),
                "t" => in_text = false,
                "c" => {
                    if let Some(cell) = open.take() {
                        finish_cell(name, cell, workbook, &mut cells, &mut unread);
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if let Some(cell) = open.as_mut() {
                    let decoded = unescaped(text);
                    if in_value {
                        cell.value.push_str(&decoded);
                    } else if in_text {
                        cell.inline.push_str(&decoded);
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(SheetScan {
        direction,
        cells,
        unread,
    })
}

/// Resolve one finished cell into text, or into a source this adapter did not
/// read, or into nothing.
fn finish_cell(
    sheet: &str,
    cell: CellScan,
    workbook: &Workbook,
    cells: &mut Vec<Cell>,
    unread: &mut Vec<String>,
) {
    let reference = cell
        .reference
        .clone()
        .unwrap_or_else(|| format!("cell {}", cell.index));
    let named = format!("{sheet}!{reference}");

    let text = match cell.kind.as_deref() {
        // A shared string: the `<v>` is an index into `xl/sharedStrings.xml`.
        Some("s") => cell
            .value
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|i| workbook.shared_string(i))
            .map(str::to_string),
        Some("inlineStr") => Some(cell.inline.clone()),
        // A formula's cached string result. Handled below.
        Some("str") => Some(cell.value.clone()),
        // A number, a boolean, an error or a date: nothing this tool judges.
        _ => None,
    };
    let Some(text) = text.filter(|t| !t.is_empty()) else {
        return;
    };

    if cell.formula || cell.kind.as_deref() == Some("str") {
        if script::has_arabic(&text) {
            unread.push(named);
        }
        return;
    }

    cells.push(Cell {
        index: cell.index,
        text,
        style: cell.style,
        column: cell.column,
        reference: named,
    });
}

/// Lower one worksheet's reading into units.
fn units_of(sheet: &Sheet, scan: &SheetScan, workbook: &Workbook) -> Vec<TextUnit> {
    let inherited = scan.direction.map(|d| (d, sheet.part.as_str()));
    let mut units: Vec<TextUnit> = scan
        .cells
        .iter()
        .map(|cell| {
            let font = workbook.font(cell.style);
            TextUnit::new(unit_id(&sheet.part, cell.index), cell.text.clone())
                .with_props(Properties {
                    direction: workbook.direction(cell.style, inherited),
                    alignment: workbook.alignment(cell.style),
                    // `alignment/@indent` is measured from the start edge, so
                    // there is no physical inset here to state.
                    inset: Resolved::Unset,
                    language: workbook.language(),
                    complex_font: font.clone(),
                    latin_font: font,
                    // Excel has no list feature; see the module documentation.
                    bullet: Bullet::None,
                    reversed: None,
                })
                .with_location(Location {
                    part: sheet.part.clone(),
                    paragraph: Some(cell.index),
                    container: Some(cell.reference.clone()),
                })
        })
        .collect();

    // The grid, where there is a column order to get wrong.
    let columns: BTreeSet<usize> = scan.cells.iter().map(|cell| cell.column).collect();
    if columns.len() >= 2 {
        let text = scan
            .cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        units.push(
            TextUnit::new(grid_id(&sheet.part), text)
                .with_kind(UnitKind::Table)
                .with_props(Properties {
                    direction: match scan.direction {
                        Some(direction) => Resolved::Explicit(direction),
                        None => Resolved::Unset,
                    },
                    ..Properties::default()
                })
                .with_location(Location {
                    part: sheet.part.clone(),
                    paragraph: None,
                    container: Some(format!("sheet {:?}", sheet.name)),
                }),
        );
    }

    units
}

/// An Excel workbook opened for auditing.
pub struct XlsxDocument {
    package: Package,
    /// Parts staged by [`DocumentWriter::apply`], written by
    /// [`DocumentWriter::write`].
    edits: Edits,
    /// The formula results the last scan did not judge.
    unread: Vec<String>,
}

impl XlsxDocument {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            package: Package::open(path)?,
            edits: Edits::new(),
            unread: Vec::new(),
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

    /// The workbook's style, string and sheet sources, read from the package
    /// once.
    ///
    /// Read on demand rather than cached with the document, for the reason the
    /// other two adapters give: a scan reads it once, and a field every `open`
    /// pays for is a cost `mirsam rules` should not carry.
    pub fn workbook(&self) -> Result<Workbook> {
        Workbook::read(&self.package)
    }

    /// One part's XML, taking a staged edit over the package's own bytes so
    /// that repairs applied in two rounds compose.
    fn part_text(&self, part: &str) -> Result<String> {
        match self.edits.get(part) {
            Some(bytes) => {
                String::from_utf8(bytes.clone()).map_err(|e| Error::Format(format!("{part}: {e}")))
            }
            None => self.package.read_text(part),
        }
    }
}

impl DocumentReader for XlsxDocument {
    fn format(&self) -> &'static str {
        "xlsx"
    }

    fn scan(&mut self) -> Result<Vec<TextUnit>> {
        let workbook = self.workbook()?;
        let mut units = Vec::new();
        let mut unread = Vec::new();
        for sheet in workbook.sheets() {
            let Ok(xml) = self.part_text(&sheet.part) else {
                // A `<sheet>` naming a part the package does not hold. The
                // relationship resolved, so this is a broken package rather
                // than a sheet to invent a reading for.
                continue;
            };
            let scan = scan_sheet(&sheet.name, &xml, &workbook)?;
            unread.extend(scan.unread.iter().cloned());
            units.extend(units_of(sheet, &scan, &workbook));
        }
        self.unread = unread;
        Ok(units)
    }

    /// The formula results this scan did not judge, as a person would name
    /// them: `Sheet1!C5`.
    fn unread_sources(&self) -> Vec<String> {
        self.unread.clone()
    }
}

impl DocumentWriter for XlsxDocument {
    /// Whether SpreadsheetML can express `fix`.
    ///
    /// The three refusals are the format's rather than this adapter's, and
    /// each is argued in the module documentation: a language tag is the
    /// workbook's and not the cell's, a typeface is one slot and not two, and
    /// there is no list to convert a typed bullet into.
    fn supports(&self, fix: &Fix) -> bool {
        !matches!(
            fix,
            Fix::SetLanguage(_) | Fix::SetComplexFont(_) | Fix::ConvertLiteralBullet { .. }
        )
    }

    fn apply(&mut self, repairs: &[Repair]) -> Result<usize> {
        let mut by_part: BTreeMap<String, SheetPlan> = BTreeMap::new();
        for repair in repairs {
            let Some((part, target)) = parse_unit_id(&repair.unit) else {
                return Err(Error::Format(format!(
                    "{}: not a unit this adapter produced",
                    repair.unit
                )));
            };
            let plan = by_part.entry(part.to_string()).or_default();
            match target {
                Target::Cell(index) => plan.cells.entry(index).or_default(),
                Target::Grid => &mut plan.grid,
            }
            .push(repair.fix.clone());
        }

        let workbook = self.workbook()?;
        // The two shared parts a cell repair reaches into. Both are built from
        // whatever is staged already, so a second round of repairs appends to
        // the first round's records rather than discarding them.
        let mut styles = StyleTable::read(&self.part_text(STYLES).unwrap_or_default())?;
        let mut strings = Strings::read(&self.part_text(SHARED_STRINGS).unwrap_or_default())?;

        let mut staged = Edits::new();
        let mut applied = 0usize;
        for (part, plan) in &by_part {
            let xml = self.part_text(part)?;

            // What each cell inherits — from the sheet it is in, and from the
            // cell style above its own record — resolved by the same scanner
            // that produced the units the rules judged. The rewriter can see
            // neither from inside a `<c>`, and a direction-relative alignment
            // cannot be lowered onto `horizontal` without it.
            let sheet = workbook
                .sheets()
                .iter()
                .find(|sheet| &sheet.part == part)
                .ok_or_else(|| {
                    Error::Format(format!("{part}: not a worksheet of this workbook"))
                })?;
            let inherited: Inherited =
                units_of(sheet, &scan_sheet(&sheet.name, &xml, &workbook)?, &workbook)
                    .into_iter()
                    .filter_map(|unit| {
                        let Resolved::Inherited(direction, _) = unit.props.direction else {
                            return None;
                        };
                        match parse_unit_id(&unit.id) {
                            Some((_, Target::Cell(index))) => Some((index, direction)),
                            _ => None,
                        }
                    })
                    .collect();

            applied += plan.len();
            let rewritten = sheet::apply(part, &xml, plan, &inherited, &mut styles, &mut strings)?;
            staged.insert(part.clone(), rewritten.into_bytes());
        }

        if styles.changed() {
            staged.insert(STYLES.to_string(), styles.write(STYLES)?.into_bytes());
        }
        if strings.changed() {
            staged.insert(
                SHARED_STRINGS.to_string(),
                strings.write(SHARED_STRINGS)?.into_bytes(),
            );
        }

        self.edits.extend(staged);
        Ok(applied)
    }

    fn write(&mut self, dest: &Path) -> Result<()> {
        self.package.rewrite(dest, &self.edits)?;
        Ok(())
    }
}

/// Parse an in-memory worksheet into every unit this adapter produces for it.
///
/// Exposed for tests and for callers that already hold the XML. `workbook` is
/// the chain above the sheet; without one a property the cell's format record
/// does not state comes back `Unset`, because there is no `xl/styles.xml` here
/// to resolve it against.
pub fn scan_xml(name: &str, part: &str, xml: &str, workbook: &Workbook) -> Result<Vec<TextUnit>> {
    let sheet = Sheet {
        name: name.to_string(),
        part: part.to_string(),
    };
    let scan = scan_sheet(name, xml, workbook)?;
    Ok(units_of(&sheet, &scan, workbook))
}

#[cfg(test)]
mod unit_id_tests {
    use super::*;

    #[test]
    fn a_unit_id_round_trips_through_its_own_parser() {
        let part = "xl/worksheets/sheet1.xml";
        assert_eq!(
            parse_unit_id(&UnitId(unit_id(part, 7))),
            Some((part, Target::Cell(7)))
        );
        assert_eq!(
            parse_unit_id(&UnitId(grid_id(part))),
            Some((part, Target::Grid))
        );
    }

    #[test]
    fn an_id_this_adapter_did_not_issue_is_rejected() {
        for id in [
            "xl/worksheets/sheet1.xml",
            "#p1",
            "xl/worksheets/sheet1.xml#p0",
            "xl/worksheets/sheet1.xml#px",
            "xl/worksheets/sheet1.xml#cols1",
        ] {
            assert_eq!(parse_unit_id(&UnitId(id.into())), None, "{id}");
        }
    }

    #[test]
    fn a_reference_names_the_column_excel_names() {
        assert_eq!(column_of("A1"), Some(0));
        assert_eq!(column_of("B5"), Some(1));
        assert_eq!(column_of("Z9"), Some(25));
        assert_eq!(column_of("AA1"), Some(26));
        assert_eq!(column_of("1"), None);
        assert_eq!(column_of(""), None);
    }
}
