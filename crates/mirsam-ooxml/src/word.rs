//! WordprocessingML's repair vocabulary: which element and which attribute
//! each `Fix` lands on, and where a created element goes.
//!
//! [`crate::rewrite`] is this module's opposite number for DrawingML and
//! [`crate::sheet`] Excel's; the three share [`crate::token`] and
//! [`crate::package`] and not one element name. The guarantee is the same at
//! both scales: a part no repair touches survives byte for byte, and inside a
//! part that *is* rewritten, a token no repair addresses comes out as it went
//! in.
//!
//! ## Word's alignment is already direction-relative, so nothing is lowered
//!
//! `a:pPr/@algn` names physical edges, so the PowerPoint rewriter has to be
//! told the direction each paragraph reads in before it can write a `Start`
//! down — that is what [`crate::rewrite::Inherited`] is for, and getting it
//! wrong reproduces the defect being repaired.
//!
//! **This module needs no such map, and that is Word's doing rather than a
//! shortcut.** `w:jc`'s values are evaluated against the paragraph's own
//! `w:bidi` ([MS-OE376] Part 4 §2.3.1.13, note b), so `Start` is written as
//! `start` whichever way the paragraph runs and stays the start edge if the
//! direction is changed afterwards. The same fact read from the other side is
//! why [`crate::docx`] never produces `Alignment::Left`: a physical edge is
//! not a thing WordprocessingML can say, so [`Alignment::Left`] and
//! [`Alignment::Right`] are *refused* here rather than lowered onto the edge
//! they happen to mean today. The conformance suite states that refusal for
//! the reading side, and this is its writing half.
//!
//! ## `<w:bidi/>` is on, so left-to-right must be written out
//!
//! An `ST_OnOff` element with no `w:val` is true, which is the form Word
//! writes far more often than the explicit one. A repair setting a paragraph
//! left-to-right therefore writes `w:val="0"` and never an empty element:
//! creating `<w:bidi/>` to mean *off* would turn the repair into the defect.
//!
//! ## A typed bullet becomes a list this document already has
//!
//! DrawingML answers `ConvertLiteralBullet` with `a:buChar`, an attribute on
//! the paragraph. Word has no such thing: a list is a `w:numPr` pointing into
//! `word/numbering.xml`, and a paragraph cannot carry the definition itself.
//! [`bullet_list`] finds a definition the document already stores — preferring
//! one that draws the marker the author typed — and the repair points at it.
//! A document with no bulleted list at all is *refused*, because
//! [`crate::package`] rewrites the entries a package has and creating
//! `word/numbering.xml`, its content-type override and its relationship is
//! three new parts rather than an edit.
//!
//! ## Which runs belong to the paragraph being repaired
//!
//! A `w:txbxContent` nests whole paragraphs inside a run, and each of them is
//! a unit in its own right with its own text, its own ordinal and its own
//! repairs. An `mc:Fallback` spells out the same paragraphs as the
//! `mc:Choice` beside it and [`crate::docx`] reads neither it nor them. So
//! both are stepped over here — when the elements are counted, so a repair
//! lands on the paragraph the report named, and when a paragraph's runs are
//! edited, so an offset the domain computed cannot reach text nobody scanned.
//!
//! [MS-OE376]: https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/26ecf09a-0f0b-4574-9907-ebd1ddf3015f

use crate::token::{
    self, child_or_insert, edit_tag, element_range, element_ranges_outside, find_direct_child,
    get_attribute, remove_attribute, set_attribute,
};
use mirsam_core::Fix;
use mirsam_core::error::{Error, Result};
use mirsam_core::script;
use mirsam_core::text::{Alignment, Direction};
use quick_xml::events::Event;
use std::collections::BTreeMap;
use std::ops::Range;

/// Repairs for one part, keyed by paragraph index.
///
/// The index counts every `w:p` in the part, 1-based, exactly as
/// [`crate::docx`] counts them — including the paragraphs that produced no
/// text unit, and excluding everything inside an `mc:Fallback`, so the two
/// numberings cannot drift.
pub type PartFixes = BTreeMap<usize, Vec<Fix>>;

/// Repairs for one part's tables, keyed by table index: every `w:tbl` in the
/// part, 1-based, as the scanner counts them.
pub type TableFixes = BTreeMap<usize, Vec<Fix>>;

/// The list a typed marker is converted to, by the marker that was typed.
///
/// Supplied by the adapter, which is the half of this that can read
/// `word/numbering.xml`; see [`bullet_list`].
pub type Bullets = BTreeMap<char, String>;

/// Everything to change in one part.
#[derive(Debug, Default, Clone)]
pub struct PartPlan {
    pub paragraphs: PartFixes,
    pub tables: TableFixes,
}

impl PartPlan {
    /// How many repairs the plan carries.
    pub fn len(&self) -> usize {
        [&self.paragraphs, &self.tables]
            .into_iter()
            .flat_map(|fixes| fixes.values())
            .map(Vec::len)
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------- schema order

/// `CT_PPr`, in schema sequence order: `CT_PPrBase` followed by the paragraph
/// mark's run properties, the section properties and the revision record.
const PPR_ORDER: &[&str] = &[
    "w:pStyle",
    "w:keepNext",
    "w:keepLines",
    "w:pageBreakBefore",
    "w:framePr",
    "w:widowControl",
    "w:numPr",
    "w:suppressLineNumbers",
    "w:pBdr",
    "w:shd",
    "w:tabs",
    "w:suppressAutoHyphens",
    "w:kinsoku",
    "w:wordWrap",
    "w:overflowPunct",
    "w:topLinePunct",
    "w:autoSpaceDE",
    "w:autoSpaceDN",
    "w:bidi",
    "w:adjustRightInd",
    "w:snapToGrid",
    "w:spacing",
    "w:ind",
    "w:contextualSpacing",
    "w:mirrorIndents",
    "w:suppressOverlap",
    "w:jc",
    "w:textDirection",
    "w:textAlignment",
    "w:textboxTightWrap",
    "w:outlineLvl",
    "w:divId",
    "w:cnfStyle",
    "w:rPr",
    "w:sectPr",
    "w:pPrChange",
];

/// `CT_RPr`, in schema sequence order, with the four revision markers
/// `CT_ParaRPr` puts in front of the same sequence.
///
/// One list for both, because the paragraph mark's run properties and a run's
/// carry the same elements in the same order; the markers simply never occur
/// in a `w:rPr` that is not a paragraph mark's, and an element that never
/// occurs cannot be ranked wrongly.
const RPR_ORDER: &[&str] = &[
    "w:ins",
    "w:del",
    "w:moveFrom",
    "w:moveTo",
    "w:rStyle",
    "w:rFonts",
    "w:b",
    "w:bCs",
    "w:i",
    "w:iCs",
    "w:caps",
    "w:smallCaps",
    "w:strike",
    "w:dstrike",
    "w:outline",
    "w:shadow",
    "w:emboss",
    "w:imprint",
    "w:noProof",
    "w:snapToGrid",
    "w:vanish",
    "w:webHidden",
    "w:color",
    "w:spacing",
    "w:w",
    "w:kern",
    "w:position",
    "w:sz",
    "w:szCs",
    "w:highlight",
    "w:u",
    "w:effect",
    "w:bdr",
    "w:shd",
    "w:fitText",
    "w:vertAlign",
    "w:rtl",
    "w:cs",
    "w:em",
    "w:lang",
    "w:eastAsianLayout",
    "w:specVanish",
    "w:oMath",
    "w:rPrChange",
];

/// `CT_TblPrBase`, in schema sequence order, with the revision record
/// `CT_TblPr` adds after it.
const TBLPR_ORDER: &[&str] = &[
    "w:tblStyle",
    "w:tblpPr",
    "w:tblOverlap",
    "w:bidiVisual",
    "w:tblStyleRowBandSize",
    "w:tblStyleColBandSize",
    "w:tblW",
    "w:jc",
    "w:tblCellSpacing",
    "w:tblInd",
    "w:tblBorders",
    "w:shd",
    "w:tblLayout",
    "w:tblCellMar",
    "w:tblLook",
    "w:tblCaption",
    "w:tblDescription",
    "w:tblPrChange",
];

/// `CT_Tbl`: the properties, then the grid, then the rows.
const TBL_ORDER: &[&str] = &["w:tblPr", "w:tblGrid", "w:tr"];

/// `CT_NumPr`, in schema sequence order.
const NUMPR_ORDER: &[&str] = &["w:ilvl", "w:numId", "w:numberingChange", "w:ins"];

/// `CT_P`: `w:pPr` precedes every run.
const P_ORDER: &[&str] = &["w:pPr"];

/// `CT_R`: `w:rPr` precedes the run's content.
const R_ORDER: &[&str] = &["w:rPr"];

/// The element a run's characters live in.
const W_T: &str = "w:t";

/// Regions of a part whose paragraphs are not the paragraph being repaired:
/// the text boxes that nest whole units inside a run, and the fallback
/// [`crate::docx`] steps over.
const NESTED: &[&str] = &["w:txbxContent", "mc:Fallback"];

/// Regions of a part that [`crate::docx`] does not count, and so regions this
/// module must not count either.
///
/// A `w:txbxContent` is missing on purpose: the paragraphs in a text box *are*
/// counted by the scanner and are units of their own, and only their text and
/// their run properties belong to the inner paragraph rather than the outer.
const UNCOUNTED: &[&str] = &["mc:Fallback"];

// ------------------------------------------------------------------- lexemes

/// An `ST_OnOff` value, written out in full.
///
/// Never the empty element: `<w:bidi/>` is *on*, so a left-to-right repair has
/// to say `w:val="0"` or it writes the opposite of what it was asked for.
fn on_off(direction: Direction) -> &'static str {
    if direction == Direction::Rtl {
        "1"
    } else {
        "0"
    }
}

/// Lower an alignment onto `w:jc`, which names the paragraph's own start and
/// end edges rather than the page's.
///
/// `None` for a physical edge. Word evaluates `left` and `right` against the
/// paragraph's `w:bidi`, so there is no value here that means "the left of the
/// page whatever the direction"; writing the relative spelling that happens to
/// land left today would be answering a different question from the one asked,
/// and one that comes apart the moment the direction changes.
fn word_alignment(alignment: Alignment) -> Option<&'static str> {
    Some(match alignment {
        Alignment::Start => "start",
        Alignment::End => "end",
        Alignment::Center => "center",
        Alignment::Justify => "both",
        Alignment::Distributed => "distribute",
        Alignment::Left | Alignment::Right => return None,
    })
}

// ------------------------------------------------------------------ numbering

/// One attribute of the tag at `at`.
fn attribute(events: &[Event<'static>], at: usize, name: &str) -> Option<String> {
    match &events[at] {
        Event::Start(tag) | Event::Empty(tag) => get_attribute(tag, name),
        _ => None,
    }
}

/// The direct children of the element at `at`, as absolute indices.
fn children(events: &[Event<'static>], at: usize) -> Vec<usize> {
    let range = element_range(events, at);
    token::direct_children(&events[range.clone()])
        .into_iter()
        .map(|i| range.start + i)
        .collect()
}

/// The `w:numId` of a list in `word/numbering.xml` that draws a bullet,
/// preferring one whose first level draws `marker` itself.
///
/// A repair converting a typed glyph into a real list has to point at a
/// definition, and this is the only honest place to get one: the definitions
/// the document already carries. Preferring the marker the author typed keeps
/// the repaired page looking like the page they were aiming at; falling back
/// to any bulleted list keeps the repair possible when it does not, because
/// the defect being fixed is the glyph in the text and not which glyph it is.
///
/// `None` when the document defines no bulleted list, which is a refusal
/// rather than a licence to invent one: a `w:num` this module wrote would name
/// an `w:abstractNum` it also wrote, and neither is a repair to a paragraph.
///
/// A `w:lvlOverride` inside a `w:num` is not read. It can restate a level's
/// format, and a definition this function accepted on the strength of its
/// abstract while an override said otherwise would be a list drawing something
/// else — so only a `w:num` whose abstract says `bullet` and which overrides
/// nothing at level zero is offered.
pub fn bullet_list(part: &str, xml: &str, marker: char) -> Result<Option<String>> {
    let events = token::read_events(part, xml)?;

    // Abstract definitions whose first level draws a bullet, and the glyph
    // each of them draws.
    let mut abstracts: BTreeMap<String, Option<String>> = BTreeMap::new();
    for at in token::element_starts_outside(&events, "w:abstractNum", &[]) {
        let Some(id) = attribute(&events, at, "w:abstractNumId") else {
            continue;
        };
        let Some(level) = children(&events, at).into_iter().find(|&i| {
            token::name_of(&events[i]).as_deref() == Some("w:lvl")
                && attribute(&events, i, "w:ilvl").as_deref() == Some("0")
        }) else {
            continue;
        };
        let format = find_direct_child(&events, level, "w:numFmt")
            .and_then(|i| attribute(&events, i, "w:val"));
        if format.as_deref() != Some("bullet") {
            continue;
        }
        let text = find_direct_child(&events, level, "w:lvlText")
            .and_then(|i| attribute(&events, i, "w:val"));
        abstracts.insert(id, text);
    }

    // The lists pointing at them, in document order, with the ones that draw
    // the typed marker first.
    let typed = marker.to_string();
    let mut exact = None;
    let mut any = None;
    for at in token::element_starts_outside(&events, "w:num", &[]) {
        let Some(num_id) = attribute(&events, at, "w:numId") else {
            continue;
        };
        if children(&events, at)
            .into_iter()
            .any(|i| token::name_of(&events[i]).as_deref() == Some("w:lvlOverride"))
        {
            continue;
        }
        let Some(abstract_id) = find_direct_child(&events, at, "w:abstractNumId")
            .and_then(|i| attribute(&events, i, "w:val"))
        else {
            continue;
        };
        let Some(drawn) = abstracts.get(&abstract_id) else {
            continue;
        };
        let slot = if drawn.as_deref() == Some(typed.as_str()) {
            &mut exact
        } else {
            &mut any
        };
        if slot.is_none() {
            *slot = Some(num_id);
        }
    }

    Ok(exact.or(any))
}

// ------------------------------------------------------------------- editing

/// The paragraph's `w:pPr`, created in schema position if it has none.
fn ensure_ppr(para: &mut Vec<Event<'static>>) -> usize {
    child_or_insert(para, 0, P_ORDER, "w:pPr")
}

/// The content ranges of the runs that are this paragraph's own.
fn own_text(para: &[Event<'static>]) -> Vec<Range<usize>> {
    token::text_content_ranges_outside(para, W_T, NESTED)
}

/// Apply `f` to every run-property element the paragraph owns, creating
/// `w:rPr` for runs that lack one.
///
/// Runs are visited back to front: creating a `w:rPr` shifts every index after
/// it, and the ones still to be visited must not move. The paragraph mark's
/// own properties come last, because `w:pPr` sits in front of every run and so
/// does not move when one of them gains a child — and they are edited only
/// where they already exist, matching how the DrawingML rewriter treats an
/// `a:endParaRPr`: the mark is the pilcrow's formatting, and a paragraph that
/// never stated any is not made to.
fn for_each_run_property(
    para: &mut Vec<Event<'static>>,
    mut f: impl FnMut(&mut Vec<Event<'static>>, usize),
) {
    for at in token::element_starts_outside(para, "w:r", NESTED)
        .into_iter()
        .rev()
    {
        let rpr = child_or_insert(para, at, R_ORDER, "w:rPr");
        f(para, rpr);
    }

    if let Some(ppr) = find_direct_child(para, 0, "w:pPr")
        && let Some(rpr) = find_direct_child(para, ppr, "w:rPr")
    {
        f(para, rpr);
    }
}

/// Mark the runs whose text changed as significant-whitespace, where it now
/// is.
///
/// WordprocessingML is the one vocabulary here where this matters. A `w:t`
/// without `xml:space="preserve"` has its leading and trailing whitespace
/// collapsed away, so a repair that strips a typed `•` and leaves the space
/// behind it — or deletes a control character that was standing in front of
/// one — would silently lose a space it never meant to touch. Only runs this
/// repair actually rewrote are marked, so nothing else in the paragraph moves.
fn preserve_space(para: &mut Vec<Event<'static>>, before: &[String]) {
    let ranges = own_text(para);
    if ranges.len() != before.len() {
        return;
    }
    let after = token::read_runs(para, &ranges);
    for (i, range) in ranges.iter().enumerate() {
        let changed = before[i] != after[i];
        let padded =
            after[i].starts_with(char::is_whitespace) || after[i].ends_with(char::is_whitespace);
        if changed && padded && range.start > 0 {
            edit_tag(para, range.start - 1, |tag| {
                set_attribute(tag, "xml:space", "preserve")
            });
        }
    }
}

/// Apply every repair for one paragraph.
fn apply_to_paragraph(
    part: &str,
    para: &mut Vec<Event<'static>>,
    fixes: &[Fix],
    bullets: &Bullets,
) -> Result<()> {
    // Text first, on the same terms the DrawingML rewriter states: these
    // replace text events in place, so they neither move nor are moved by the
    // structural edits that follow; deletions come before anything else
    // whatever order the fixes arrived in, because both kinds carry byte
    // offsets into the text as it was scanned; and both sets are deleted in
    // one pass, because they index the same original string.
    let text_before = token::read_runs(para, &own_text(para));

    let deletions: Vec<usize> = fixes
        .iter()
        .filter_map(|fix| match fix {
            Fix::RemoveControls(offsets) | Fix::RemoveTatweel(offsets) => Some(offsets),
            _ => None,
        })
        .flatten()
        .copied()
        .collect();
    if !deletions.is_empty() {
        let ranges = own_text(para);
        token::remove_at_offsets_in(para, &ranges, &deletions);
    }
    if fixes.contains(&Fix::NormalizePresentationForms) {
        let ranges = own_text(para);
        token::map_runs_in(para, &ranges, script::normalize_presentation_forms);
    }
    for fix in fixes {
        if let Fix::ConvertLiteralBullet { marker } = fix {
            let ranges = own_text(para);
            token::strip_leading_marker_in(para, &ranges, *marker);
        }
    }
    preserve_space(para, &text_before);

    for fix in fixes {
        match fix {
            Fix::SetDirection(direction) => {
                let ppr = ensure_ppr(para);
                let bidi = child_or_insert(para, ppr, PPR_ORDER, "w:bidi");
                edit_tag(para, bidi, |tag| {
                    set_attribute(tag, "w:val", on_off(*direction))
                });
            }

            Fix::SetAlignment(alignment) => {
                let Some(value) = word_alignment(*alignment) else {
                    return Err(Error::Format(format!(
                        "{part}: cannot {fix}; w:jc names the paragraph's own start \
                         and end edges, and Word has no spelling for a physical one"
                    )));
                };
                let ppr = ensure_ppr(para);
                let jc = child_or_insert(para, ppr, PPR_ORDER, "w:jc");
                edit_tag(para, jc, |tag| set_attribute(tag, "w:val", value));
            }

            // The complex-script language, `w:lang/@w:bidi`. `@w:val` is the
            // Latin one and is left exactly as it was, for the reason
            // [`crate::docx`] reads the one and not the other: Arabic tagged
            // `en-US` in `@w:val` and `ar-SA` in `@w:bidi` is correctly
            // tagged, and this repair has nothing to say about the Latin.
            Fix::SetLanguage(tag_value) => {
                for_each_run_property(para, |events, at| {
                    let lang = child_or_insert(events, at, RPR_ORDER, "w:lang");
                    edit_tag(events, lang, |tag| set_attribute(tag, "w:bidi", tag_value));
                });
            }

            // `w:rFonts/@w:cs`, and the theme reference beside it goes.
            // `@w:cstheme` is what Word renders and `@w:cs` the resolved value
            // it caches for consumers that do not implement themes; writing
            // the typeface into the cache and leaving the reference standing
            // would change what the file says and not what a reader sees.
            Fix::SetComplexFont(typeface) => {
                for_each_run_property(para, |events, at| {
                    let fonts = child_or_insert(events, at, RPR_ORDER, "w:rFonts");
                    edit_tag(events, fonts, |tag| set_attribute(tag, "w:cs", typeface));
                    edit_tag(events, fonts, |tag| remove_attribute(tag, "w:cstheme"));
                });
            }

            // The marker itself was stripped above; what is left is to point
            // the paragraph at a list. The indent comes from the numbering
            // definition's own `w:lvl/w:pPr/w:ind`, so unlike DrawingML there
            // is nothing for the paragraph to state about it.
            Fix::ConvertLiteralBullet { marker } => {
                let Some(num_id) = bullets.get(marker) else {
                    return Err(Error::Format(format!(
                        "{part}: cannot {fix}; this document defines no bulleted list \
                         for a paragraph to join"
                    )));
                };
                let ppr = ensure_ppr(para);
                let numpr = child_or_insert(para, ppr, PPR_ORDER, "w:numPr");
                let level = child_or_insert(para, numpr, NUMPR_ORDER, "w:ilvl");
                edit_tag(para, level, |tag| set_attribute(tag, "w:val", "0"));
                let num = child_or_insert(para, numpr, NUMPR_ORDER, "w:numId");
                edit_tag(para, num, |tag| set_attribute(tag, "w:val", num_id));
            }

            Fix::RemoveControls(_) | Fix::RemoveTatweel(_) | Fix::NormalizePresentationForms => {}
        }
    }

    Ok(())
}

/// Apply every repair for one table, whose `w:tbl` starts at `at`.
///
/// A table has one property of its own this tool reasons about — its
/// direction, `w:tblPr/w:bidiVisual`, which decides which side the first cell
/// in the file is displayed on. Anything else a plan names on a table is a
/// mistake upstream and is refused rather than guessed at.
fn apply_to_table(
    part: &str,
    events: &mut Vec<Event<'static>>,
    at: usize,
    fixes: &[Fix],
) -> Result<()> {
    for fix in fixes {
        match fix {
            Fix::SetDirection(direction) => {
                let tblpr = child_or_insert(events, at, TBL_ORDER, "w:tblPr");
                let bidi = child_or_insert(events, tblpr, TBLPR_ORDER, "w:bidiVisual");
                edit_tag(events, bidi, |tag| {
                    set_attribute(tag, "w:val", on_off(*direction))
                });
            }
            other => {
                return Err(Error::Format(format!(
                    "{part}: cannot {other} on a table; only its direction is a table's own"
                )));
            }
        }
    }
    Ok(())
}

/// Every `w:p` in the part, counted as [`crate::docx`] counts them.
fn paragraph_ranges(events: &[Event<'static>]) -> Vec<(usize, Range<usize>)> {
    element_ranges_outside(events, "w:p", UNCOUNTED)
}

/// Every `w:tbl` in the part, on the same terms.
fn table_ranges(events: &[Event<'static>]) -> Vec<(usize, Range<usize>)> {
    element_ranges_outside(events, "w:tbl", UNCOUNTED)
}

/// The event range of the `index`th element of a kind, or the error that says
/// the report and the document have come apart.
fn locate(
    part: &str,
    ranges: Vec<(usize, Range<usize>)>,
    index: usize,
    what: &str,
) -> Result<Range<usize>> {
    ranges
        .into_iter()
        .find(|(i, _)| *i == index)
        .map(|(_, range)| range)
        .ok_or_else(|| {
            Error::Format(format!(
                "{part}: no {what} {index}; the document and the report disagree"
            ))
        })
}

/// Apply repairs to a part's paragraphs, leaving every token they do not
/// address untouched.
pub fn apply(part: &str, xml: &str, fixes: &PartFixes) -> Result<String> {
    let plan = PartPlan {
        paragraphs: fixes.clone(),
        ..Default::default()
    };
    apply_plan(part, xml, &plan, &Bullets::new())
}

/// Apply a whole part's plan — paragraphs and tables — leaving every token it
/// does not address untouched.
///
/// `bullets` names the list each typed marker is converted to; see
/// [`bullet_list`]. There is no inherited direction to pass: `w:jc` is
/// relative to the paragraph's own, so nothing here has to know it.
pub fn apply_plan(part: &str, xml: &str, plan: &PartPlan, bullets: &Bullets) -> Result<String> {
    if plan.is_empty() {
        return token::passthrough(part, xml);
    }

    let mut events = token::read_events(part, xml)?;

    // Every index the plan names, checked before anything is edited, so a plan
    // that does not fit the document is refused whole rather than half-applied.
    for index in plan.paragraphs.keys() {
        locate(part, paragraph_ranges(&events), *index, "paragraph")?;
    }
    for index in plan.tables.keys() {
        locate(part, table_ranges(&events), *index, "table")?;
    }

    // Back to front: splicing a paragraph changes every index after it. The
    // ranges are taken afresh each time rather than once, because a paragraph
    // in a text box sits *inside* another paragraph, and repairing the inner
    // one moves the end of the outer one's range.
    for index in plan.paragraphs.keys().rev() {
        let range = locate(part, paragraph_ranges(&events), *index, "paragraph")?;
        let mut para: Vec<Event<'static>> = events[range.clone()].to_vec();
        apply_to_paragraph(part, &mut para, &plan.paragraphs[index], bullets)?;
        events.splice(range, para);
    }

    // Tables after paragraphs, and again back to front: a paragraph edit
    // inside a table moved its range, and a `w:tblPr` created on one table
    // moves every table after it.
    for index in plan.tables.keys().rev() {
        let range = locate(part, table_ranges(&events), *index, "table")?;
        apply_to_table(part, &mut events, range.start, &plan.tables[index])?;
    }

    token::write_events(part, &events)
}
