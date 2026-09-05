//! SpreadsheetML's repair vocabulary: which element and which attribute each
//! `Fix` lands on, and what has to be appended rather than edited.
//!
//! [`crate::rewrite`] is this module's opposite number for DrawingML, and the
//! two share [`crate::token`] and [`crate::package`] and not one element name.
//! The guarantee is the same at both scales: a part no repair touches survives
//! byte for byte, and inside a part that is rewritten, a token no repair
//! addresses comes out as it went in.
//!
//! ## Why a repair appends instead of editing
//!
//! Excel does not store a cell's formatting in the cell. `<c r="B5" s="3"/>`
//! names record 3 of `cellXfs`, and forty cells across four sheets may name the
//! same record. Writing a repaired `readingOrder` into record 3 would set the
//! reading order of all forty — including the English among them, which is the
//! defect this project treats as worse than a miss.
//!
//! So [`StyleTable::derive`] **appends**: it clones the record the cell already
//! names, changes the one attribute the repair addresses, and hands back the
//! index of the copy for the cell's `@s` to point at. Every other cell keeps
//! the record it had, byte for byte. Identical requests — the same base record
//! and the same wanted values — are answered with one appended record rather
//! than one each, so a sheet of two hundred cells needing the same fix grows
//! `cellXfs` by one.
//!
//! [`Strings::derive`] does the same for text. A cell of type `s` holds an
//! index into `xl/sharedStrings.xml`, and that string may be shown in a dozen
//! places; the repaired text is a *new* `<si>`, cloned from the old one so its
//! rich-text runs survive the edit, with the cell's `<v>` repointed at it.
//!
//! ## What is never touched
//!
//! A `<f>` is a formula and this module contains no code that can reach one: a
//! cell repair edits `@s` on the `<c>` tag or the content of its `<v>`, and a
//! formula cell produces no unit for a repair to name in the first place
//! ([`crate::xlsx`]). `xl/workbook.xml` is not in the set of parts a plan can
//! address at all, so `<definedNames>` is never rewritten — nor is any name a
//! formula depends on.

use crate::token::{
    self, edit_tag, element_range, element_ranges, find_direct_child, get_attribute,
    insert_children, name_of, read_content, set_attribute,
};
use crate::workbook::horizontal;
use mirsam_core::Fix;
use mirsam_core::error::{Error, Result};
use mirsam_core::script;
use mirsam_core::text::Direction;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use std::collections::BTreeMap;
use std::ops::Range;

/// Repairs for one cell, in the order the engine planned them.
pub type CellFixes = Vec<Fix>;

/// Everything to change in one worksheet.
#[derive(Debug, Default, Clone)]
pub struct SheetPlan {
    /// Keyed by the cell's 1-based ordinal among every `<c>` in the part,
    /// exactly as the scanner counts them.
    pub cells: BTreeMap<usize, CellFixes>,
    /// The sheet's own grid direction.
    pub grid: Vec<Fix>,
}

/// The direction each cell inherits, by ordinal, for cells that state none of
/// their own.
///
/// A cell's own `readingOrder` is reachable from its format record; the
/// sheet's `rightToLeft` two elements above it, and the named cell style in
/// another part entirely, are not. The adapter resolves those exactly as its
/// scanner does and passes the result here, so a direction-relative alignment
/// is lowered against the direction the rule actually reasoned about rather
/// than defaulting to left-to-right and writing the defect it was sent to fix.
/// The same map [`crate::rewrite::Inherited`] is, for the same reason.
pub type Inherited = BTreeMap<usize, Direction>;

impl SheetPlan {
    pub fn len(&self) -> usize {
        self.cells.values().map(Vec::len).sum::<usize>() + self.grid.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------- schema order

/// `CT_Worksheet`, as far as `sheetViews`: enough to put a created one in
/// schema position, and no further, since nothing below is ever created here.
const WORKSHEET_ORDER: &[&str] = &[
    "sheetPr",
    "dimension",
    "sheetViews",
    "sheetFormatPr",
    "cols",
    "sheetData",
];

/// `CT_SheetViews` holds `sheetView` and then `extLst`.
const SHEET_VIEWS_ORDER: &[&str] = &["sheetView", "extLst"];

/// `CT_Xf`: `alignment` precedes `protection`.
const XF_ORDER: &[&str] = &["alignment", "protection", "extLst"];

/// The element a shared or inline string's characters live in. Unprefixed:
/// SpreadsheetML is written in the default namespace.
const T: &str = "t";

/// The phonetic guide beside a string. Its `<t>` elements hold a reading
/// annotation rather than the text, so [`crate::xlsx`] leaves them out of a
/// unit's text and every offset here has to leave them out too — otherwise a
/// deletion computed against the string a reader sees would land in the
/// furigana beside it.
const RPH: &str = "rPh";

// ------------------------------------------------------------------- helpers

/// The index of the element named `name` at the top of a part.
fn root(events: &[Event<'static>], name: &str) -> Option<usize> {
    events
        .iter()
        .position(|e| name_of(e).as_deref() == Some(name))
}

/// Turn `<x/>` into `<x></x>` so children can be appended, and report the index
/// its `End` now sits at.
fn open_element(events: &mut Vec<Event<'static>>, at: usize) -> usize {
    if let Event::Empty(tag) = events[at].clone() {
        let end = BytesEnd::new(tag.name().0.to_string());
        events[at] = Event::Start(tag);
        events.insert(at + 1, Event::End(end));
    }
    element_range(events, at).end - 1
}

/// Append `children` as the last children of the element at `at`.
///
/// Appending rather than inserting in schema position, because the elements
/// this is used for — `cellXfs`'s records and `sst`'s items — are addressed by
/// their ordinal. A record inserted anywhere but the end would renumber every
/// record after it, and every cell in the workbook pointing at one.
fn append_children(events: &mut Vec<Event<'static>>, at: usize, children: Vec<Event<'static>>) {
    let end = open_element(events, at);
    events.splice(end..end, children);
}

/// Replace an element's content with one text node.
fn set_element_text(events: &mut Vec<Event<'static>>, at: usize, text: &str) {
    let range = element_range(events, at);
    if matches!(events[at], Event::Empty(_)) {
        let end = open_element(events, at);
        events.splice(end..end, [token::write_text(text)]);
        return;
    }
    events.splice(range.start + 1..range.end - 1, [token::write_text(text)]);
}

/// Set a `count`-style attribute to a number, if the element carries one.
///
/// Only if: `count` is optional, and adding one to an element that had none
/// would be this tool writing a number no application asked it for.
fn bump_count(events: &mut [Event<'static>], at: usize, name: &str, value: usize) {
    let present = match &events[at] {
        Event::Start(tag) | Event::Empty(tag) => get_attribute(tag, name).is_some(),
        _ => false,
    };
    if present {
        let value = value.to_string();
        edit_tag(events, at, |tag| set_attribute(tag, name, &value));
    }
}

/// The content ranges of every `<t>` in `events` that is not inside an `<rPh>`.
///
/// The same coordinates [`crate::xlsx`] scanned the text in, which is what
/// makes a byte offset in a finding mean the same thing here as it did there.
fn text_ranges(events: &[Event<'static>]) -> Vec<Range<usize>> {
    let phonetic: Vec<Range<usize>> = element_ranges(events, RPH)
        .into_iter()
        .map(|(_, range)| range)
        .collect();
    token::text_content_ranges(events, T)
        .into_iter()
        .filter(|range| !phonetic.iter().any(|p| p.contains(&range.start)))
        .collect()
}

/// Apply every text repair in `fixes` to the string held in `events`.
///
/// Deletions first and in one pass, whatever order the fixes arrived in: both
/// sets index the text as it was scanned, so applying one and then the other
/// would delete the second from a string the first had already shortened. The
/// same argument [`crate::rewrite`] makes, and the same failure it guards.
fn repair_text(events: &mut Vec<Event<'static>>, fixes: &[Fix]) {
    let ranges = text_ranges(events);
    if ranges.is_empty() {
        return;
    }

    let deletions: Vec<usize> = fixes
        .iter()
        .filter_map(|fix| match fix {
            Fix::RemoveControls(offsets) | Fix::RemoveTatweel(offsets) => Some(offsets),
            _ => None,
        })
        .flatten()
        .copied()
        .collect();

    let before = token::read_runs(events, &ranges);
    let mut runs = before.clone();

    if !deletions.is_empty() {
        let mut sorted = deletions;
        sorted.sort_unstable();
        for offset in sorted.into_iter().rev() {
            let mut base = 0usize;
            for text in runs.iter_mut() {
                if offset < base + text.len() {
                    let local = offset - base;
                    if text.is_char_boundary(local)
                        && let Some(c) = text[local..].chars().next()
                    {
                        text.replace_range(local..local + c.len_utf8(), "");
                    }
                    break;
                }
                base += text.len();
            }
        }
    }

    if fixes.contains(&Fix::NormalizePresentationForms) {
        for text in runs.iter_mut() {
            *text = script::normalize_presentation_forms(text);
        }
    }

    token::replace_content(events, &ranges, &before, &runs);
}

/// Whether a set of fixes changes the text rather than the formatting.
fn touches_text(fixes: &[Fix]) -> bool {
    fixes.iter().any(|fix| {
        matches!(
            fix,
            Fix::RemoveControls(_) | Fix::RemoveTatweel(_) | Fix::NormalizePresentationForms
        )
    })
}

/// A key identifying one text repair, so two cells asking for the same edit to
/// the same string are answered with one appended `<si>`.
fn text_key(fixes: &[Fix]) -> String {
    let mut parts: Vec<String> = fixes
        .iter()
        .filter(|fix| {
            matches!(
                fix,
                Fix::RemoveControls(_) | Fix::RemoveTatweel(_) | Fix::NormalizePresentationForms
            )
        })
        .map(|fix| format!("{fix:?}"))
        .collect();
    parts.sort();
    parts.join("|")
}

// -------------------------------------------------------------- xl/styles.xml

/// `xl/styles.xml`, opened so that a repair can append a format record.
pub struct StyleTable {
    events: Vec<Event<'static>>,
    /// Index of the `cellXfs` `Start`, once it is known to exist.
    section: Option<usize>,
    /// How many `<xf>` records `cellXfs` holds now.
    records: usize,
    /// How many it held when the part was read.
    original: usize,
    /// Requests already answered, so an identical one appends nothing.
    derived: BTreeMap<(usize, Option<String>, Option<String>), usize>,
}

impl StyleTable {
    /// Read the part. Empty input is a workbook with no `xl/styles.xml`, which
    /// is a workbook no repair can give a cell a format record in — reported
    /// when one is asked for, rather than guessed at here.
    pub fn read(xml: &str) -> Result<Self> {
        if xml.trim().is_empty() {
            return Ok(Self {
                events: Vec::new(),
                section: None,
                records: 0,
                original: 0,
                derived: BTreeMap::new(),
            });
        }
        let events = token::read_events(crate::workbook::STYLES, xml)?;
        let section = root(&events, "cellXfs");
        let records = section.map_or(0, |at| {
            let range = element_range(&events, at);
            element_ranges(&events[range], "xf").len()
        });
        Ok(Self {
            events,
            section,
            records,
            original: records,
            derived: BTreeMap::new(),
        })
    }

    /// Whether anything was appended, and so whether the part must be written.
    pub fn changed(&self) -> bool {
        self.records != self.original
    }

    pub fn write(&self, part: &str) -> Result<String> {
        token::write_events(part, &self.events)
    }

    /// The event range of the `<xf>` at a 0-based `cellXfs` index.
    fn record(&self, index: usize) -> Option<Range<usize>> {
        let at = self.section?;
        let range = element_range(&self.events, at);
        element_ranges(&self.events[range.clone()], "xf")
            .into_iter()
            .find(|(ordinal, _)| *ordinal == index + 1)
            .map(|(_, inner)| range.start + inner.start..range.start + inner.end)
    }

    /// A record like the one at `base` but stating `alignment` and/or
    /// `reading_order`, appended to `cellXfs`, and its index.
    ///
    /// A request that changes nothing answers with `base` itself, so a cell
    /// whose repairs were all text keeps the `@s` it had.
    pub fn derive(
        &mut self,
        base: usize,
        alignment: Option<&str>,
        reading_order: Option<&str>,
    ) -> Result<usize> {
        if alignment.is_none() && reading_order.is_none() {
            return Ok(base);
        }
        let key = (
            base,
            alignment.map(str::to_string),
            reading_order.map(str::to_string),
        );
        if let Some(index) = self.derived.get(&key) {
            return Ok(*index);
        }

        let section = self.section.ok_or_else(|| {
            Error::Format(format!(
                "{}: no cellXfs to append a cell format to",
                crate::workbook::STYLES
            ))
        })?;
        let range = self.record(base).ok_or_else(|| {
            Error::Format(format!(
                "{}: no cell format {base}; the document and the report disagree",
                crate::workbook::STYLES
            ))
        })?;

        let mut record: Vec<Event<'static>> = self.events[range].to_vec();
        // Excel applies an `xf`'s own alignment only where the flag says so,
        // and a record cloned from one that never carried alignment carries no
        // flag either.
        edit_tag(&mut record, 0, |tag| {
            set_attribute(tag, "applyAlignment", "1")
        });
        let at = token::child_or_insert(&mut record, 0, XF_ORDER, "alignment");
        if let Some(value) = alignment {
            edit_tag(&mut record, at, |tag| {
                set_attribute(tag, "horizontal", value)
            });
        }
        if let Some(value) = reading_order {
            edit_tag(&mut record, at, |tag| {
                set_attribute(tag, "readingOrder", value)
            });
        }

        append_children(&mut self.events, section, record);
        self.records += 1;
        let index = self.records - 1;
        bump_count(&mut self.events, section, "count", self.records);
        self.derived.insert(key, index);
        Ok(index)
    }
}

// ------------------------------------------------------ xl/sharedStrings.xml

/// `xl/sharedStrings.xml`, opened so that a repair can append a string.
pub struct Strings {
    events: Vec<Event<'static>>,
    section: Option<usize>,
    items: usize,
    original: usize,
    derived: BTreeMap<(usize, String), usize>,
}

impl Strings {
    pub fn read(xml: &str) -> Result<Self> {
        if xml.trim().is_empty() {
            return Ok(Self {
                events: Vec::new(),
                section: None,
                items: 0,
                original: 0,
                derived: BTreeMap::new(),
            });
        }
        let events = token::read_events(crate::workbook::SHARED_STRINGS, xml)?;
        let section = root(&events, "sst");
        let items = element_ranges(&events, "si").len();
        Ok(Self {
            events,
            section,
            items,
            original: items,
            derived: BTreeMap::new(),
        })
    }

    pub fn changed(&self) -> bool {
        self.items != self.original
    }

    pub fn write(&self, part: &str) -> Result<String> {
        token::write_events(part, &self.events)
    }

    fn item(&self, index: usize) -> Option<Range<usize>> {
        element_ranges(&self.events, "si")
            .into_iter()
            .find(|(ordinal, _)| *ordinal == index + 1)
            .map(|(_, range)| range)
    }

    /// A copy of the string at `index` with `fixes` applied, appended to the
    /// table, and its index.
    ///
    /// A copy rather than an edit: the string may be shown in cells this repair
    /// says nothing about. Cloned from the original rather than rebuilt from
    /// its text, so a string whose runs carry their own formatting keeps them.
    pub fn derive(&mut self, index: usize, fixes: &[Fix]) -> Result<usize> {
        let key = (index, text_key(fixes));
        if let Some(found) = self.derived.get(&key) {
            return Ok(*found);
        }

        let section = self.section.ok_or_else(|| {
            Error::Format(format!(
                "{}: no sst to append a string to",
                crate::workbook::SHARED_STRINGS
            ))
        })?;
        let range = self.item(index).ok_or_else(|| {
            Error::Format(format!(
                "{}: no shared string {index}; the document and the report disagree",
                crate::workbook::SHARED_STRINGS
            ))
        })?;

        let mut item: Vec<Event<'static>> = self.events[range].to_vec();
        repair_text(&mut item, fixes);

        append_children(&mut self.events, section, item);
        self.items += 1;
        let new = self.items - 1;
        // `count` counts cell references and none was added: one cell stopped
        // pointing at a string and started pointing at another. `uniqueCount`
        // counts the strings themselves, and there is one more of those.
        bump_count(&mut self.events, section, "uniqueCount", self.items);
        self.derived.insert(key, new);
        Ok(new)
    }
}

// ------------------------------------------------------------- the worksheet

/// Apply one worksheet's plan, leaving every token it does not address
/// untouched.
///
/// `styles` and `strings` are the two shared parts a cell repair reaches into;
/// they are the caller's because one repair round may span several sheets and
/// all of them append to the same two tables.
pub fn apply(
    part: &str,
    xml: &str,
    plan: &SheetPlan,
    inherited: &Inherited,
    styles: &mut StyleTable,
    strings: &mut Strings,
) -> Result<String> {
    if plan.is_empty() {
        return token::passthrough(part, xml);
    }

    let mut events = token::read_events(part, xml)?;

    if let Some(missing) = plan
        .cells
        .keys()
        .find(|k| !element_ranges(&events, "c").iter().any(|(i, _)| i == *k))
    {
        return Err(Error::Format(format!(
            "{part}: no cell {missing}; the document and the report disagree"
        )));
    }

    // Cells back to front: repairing one changes every index after it.
    for (index, range) in element_ranges(&events, "c").into_iter().rev() {
        let Some(fixes) = plan.cells.get(&index) else {
            continue;
        };
        let mut cell: Vec<Event<'static>> = events[range.clone()].to_vec();
        apply_to_cell(
            part,
            &mut cell,
            fixes,
            inherited.get(&index).copied(),
            styles,
            strings,
        )?;
        events.splice(range, cell);
    }

    // The grid last: its `sheetViews` sits before `sheetData`, so creating one
    // moves every cell — which is safe only now that the cells are done.
    if !plan.grid.is_empty() {
        apply_to_grid(part, &mut events, &plan.grid)?;
    }

    token::write_events(part, &events)
}

/// Apply every repair for one cell, whose `<c>` is `cell[0]`.
fn apply_to_cell(
    part: &str,
    cell: &mut Vec<Event<'static>>,
    fixes: &[Fix],
    inherited: Option<Direction>,
    styles: &mut StyleTable,
    strings: &mut Strings,
) -> Result<()> {
    // Direction first: `horizontal` names physical edges, so a
    // direction-relative alignment cannot be lowered without it.
    let declared = fixes.iter().find_map(|fix| match fix {
        Fix::SetDirection(direction) => Some(*direction),
        _ => None,
    });
    let reading_order = declared.map(|direction| match direction {
        Direction::Rtl => "2",
        Direction::Ltr => "1",
    });

    let kind = match &cell[0] {
        Event::Start(tag) | Event::Empty(tag) => get_attribute(tag, "t"),
        _ => None,
    };
    let base = match &cell[0] {
        Event::Start(tag) | Event::Empty(tag) => get_attribute(tag, "s")
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0),
        _ => 0,
    };

    let alignment = fixes.iter().find_map(|fix| match fix {
        Fix::SetAlignment(alignment) => {
            // The direction being repaired to, else the one the cell's own
            // record states, else the one it inherits from the sheet or the
            // cell style above it. Only then left-to-right — the same order
            // `crate::rewrite` resolves it in, and the reason `inherited` has
            // to be passed in at all: `alignment-unset` fires on a cell whose
            // direction is inherited *and correct*, so no `SetDirection`
            // arrives beside the alignment to say which way it reads.
            let rtl = declared
                .or_else(|| existing_reading_order(styles, base))
                .or(inherited)
                .is_some_and(|direction| direction == Direction::Rtl);
            Some(horizontal(*alignment, rtl))
        }
        _ => None,
    });

    if alignment.is_some() || reading_order.is_some() {
        let derived = styles.derive(base, alignment, reading_order)?;
        let value = derived.to_string();
        edit_tag(cell, 0, |tag| set_attribute(tag, "s", &value));
    }

    if !touches_text(fixes) {
        return Ok(());
    }

    match kind.as_deref() {
        // A shared string: the repaired text is a new `<si>` and the cell's
        // `<v>` is repointed at it. Nothing else pointing at the old one moves.
        Some("s") => {
            let Some(at) = find_direct_child(cell, 0, "v") else {
                return Ok(());
            };
            let range = element_range(cell, at);
            let index: usize = read_content(&cell[range.start + 1..range.end.saturating_sub(1)])
                .trim()
                .parse()
                .map_err(|_| {
                    Error::Format(format!(
                        "{part}: a shared-string cell whose <v> is not an index"
                    ))
                })?;
            let derived = strings.derive(index, fixes)?;
            set_element_text(cell, at, &derived.to_string());
        }
        // An inline string is the cell's own, so it is edited where it stands.
        Some("inlineStr") => repair_text(cell, fixes),
        // Every other type produced no unit, so no repair can name it. Reached
        // only if a caller invented an id, which is a mistake to report rather
        // than to guess at.
        other => {
            return Err(Error::Format(format!(
                "{part}: a text repair on a cell of type {}, which holds no text this \
                 adapter read",
                other.unwrap_or("n")
            )));
        }
    }

    Ok(())
}

/// The reading order the record at `base` already states, if it states one.
fn existing_reading_order(styles: &StyleTable, base: usize) -> Option<Direction> {
    let range = styles.record(base)?;
    let record = &styles.events[range];
    let at = record
        .iter()
        .position(|e| name_of(e).as_deref() == Some("alignment"))?;
    let value = match &record[at] {
        Event::Start(tag) | Event::Empty(tag) => get_attribute(tag, "readingOrder")?,
        _ => return None,
    };
    crate::workbook::parse_reading_order(&value)
}

/// Apply the grid's repair: which side column A sits on.
fn apply_to_grid(part: &str, events: &mut Vec<Event<'static>>, fixes: &[Fix]) -> Result<()> {
    let mut value = None;
    for fix in fixes {
        match fix {
            Fix::SetDirection(direction) => {
                value = Some(if *direction == Direction::Rtl {
                    "1"
                } else {
                    "0"
                });
            }
            // A sheet has one property this tool reasons about. Anything else
            // named on it is a mistake upstream, and is refused rather than
            // guessed at — the same refusal `crate::rewrite` gives a table.
            other => {
                return Err(Error::Format(format!(
                    "{part}: {other} is not a repair a worksheet's grid can express"
                )));
            }
        }
    }
    let Some(value) = value else {
        return Ok(());
    };

    let sheet = root(events, "worksheet")
        .ok_or_else(|| Error::Format(format!("{part}: no worksheet element")))?;

    match find_direct_child(events, sheet, "sheetViews") {
        Some(views) => match find_direct_child(events, views, "sheetView") {
            Some(view) => edit_tag(events, view, |tag| set_attribute(tag, "rightToLeft", value)),
            None => insert_children(events, views, SHEET_VIEWS_ORDER, vec![sheet_view(value)]),
        },
        None => insert_children(
            events,
            sheet,
            WORKSHEET_ORDER,
            vec![
                Event::Start(BytesStart::new("sheetViews")),
                sheet_view(value),
                Event::End(BytesEnd::new("sheetViews")),
            ],
        ),
    }
    Ok(())
}

/// A `sheetView` stating one direction, for a sheet that has none.
///
/// `workbookViewId` is required by the schema and `0` is the first workbook
/// view, which is the one every single-window workbook has.
fn sheet_view(rtl: &str) -> Event<'static> {
    let mut tag = BytesStart::new("sheetView");
    tag.push_attribute(("workbookViewId", "0"));
    tag.push_attribute(("rightToLeft", rtl));
    Event::Empty(tag)
}
