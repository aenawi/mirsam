//! DrawingML's repair vocabulary: which element and which attribute each
//! `Fix` lands on, and where a created element goes.
//!
//! [`crate::package`] guarantees that a part a repair does not touch survives
//! byte for byte, and [`crate::token`] owes the same guarantee one level down:
//! inside a part that *is* rewritten, every token the repair did not address
//! comes out exactly as it went in. This module is the layer above both. It
//! knows that a paragraph is `a:p`, that its direction is `a:pPr/@rtl`, that a
//! table's is `a:tblPr/@rtl`, and that `CT_TextParagraphProperties` is an
//! `xsd:sequence` whose order `PPR_ORDER` states. It performs no XML editing
//! of its own: every mutation below is a scaffold call with a DrawingML name
//! in it.
//!
//! That split is what makes a second format cheap. A WordprocessingML repair
//! is the same scaffold with `w:p`, `w:pPr/w:bidi` and `w:jc` in place of the
//! names here; nothing in [`crate::token`] has to learn about it.

use crate::chart::ChartText;
use crate::token::is_true;
use crate::token::{
    self, child_or_insert, edit_tag, element_ranges, find_direct_child, get_attribute,
    insert_children, set_attribute,
};
use mirsam_core::Fix;
use mirsam_core::error::{Error, Result};
use mirsam_core::script;
use mirsam_core::text::{Alignment, Direction};
use quick_xml::events::{BytesEnd, BytesStart, Event};
use std::collections::BTreeMap;

/// Repairs for one part, keyed by paragraph index.
///
/// The index counts every `a:p` in the part, 1-based, exactly as the scanner
/// counts them — including paragraphs that produced no text unit, so the two
/// numberings cannot drift.
pub type PartFixes = BTreeMap<usize, Vec<Fix>>;

/// Repairs for one part's tables, keyed by table index: every `a:tbl` in the
/// part, 1-based, as the scanner counts them.
pub type TableFixes = BTreeMap<usize, Vec<Fix>>;

/// Repairs for one part's text bodies, keyed by body index: every
/// `a:bodyPr` in the part, 1-based, as the scanner counts them — including
/// the single-column bodies that produce no unit, so the two numberings
/// cannot drift.
pub type ColumnFixes = BTreeMap<usize, Vec<Fix>>;

/// Repairs for one part's chart text containers, keyed by the container's
/// kind and its 1-based ordinal among the elements of that kind in the part.
pub type ChartTextFixes = BTreeMap<(ChartText, usize), Vec<Fix>>;

/// Everything to change in one part.
#[derive(Debug, Default, Clone)]
pub struct PartPlan {
    pub paragraphs: PartFixes,
    pub tables: TableFixes,
    pub columns: ColumnFixes,
    pub chart_text: ChartTextFixes,
}

impl PartPlan {
    /// How many repairs the plan carries.
    pub fn len(&self) -> usize {
        [&self.paragraphs, &self.tables, &self.columns]
            .into_iter()
            .flat_map(|fixes| fixes.values())
            .map(Vec::len)
            .sum::<usize>()
            + self.chart_text.values().map(Vec::len).sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The direction each paragraph inherits from its container, by index, for
/// paragraphs that declare none of their own.
///
/// A paragraph's own `a:pPr/@rtl` is visible from inside the paragraph; an
/// `a:bodyPr/@rtlCol` two levels up, or a layout's list style in another part
/// entirely, is not. The adapter resolves those exactly as its scanner does
/// and passes the result here, so a direction-relative repair is lowered
/// against the direction the rule actually reasoned about rather than
/// defaulting to left-to-right and reproducing the defect it was sent to fix.
pub type Inherited = BTreeMap<usize, Direction>;

// ---------------------------------------------------------------- schema order

/// `CT_TextParagraphProperties`, in schema sequence order.
const PPR_ORDER: &[&str] = &[
    "a:lnSpc",
    "a:spcBef",
    "a:spcAft",
    "a:buClrTx",
    "a:buClr",
    "a:buSzTx",
    "a:buSzPct",
    "a:buSzPts",
    "a:buFontTx",
    "a:buFont",
    "a:buNone",
    "a:buAutoNum",
    "a:buChar",
    "a:tabLst",
    "a:defRPr",
    "a:extLst",
];

/// `CT_TextCharacterProperties`, in schema sequence order.
const RPR_ORDER: &[&str] = &[
    "a:ln",
    "a:noFill",
    "a:solidFill",
    "a:gradFill",
    "a:blipFill",
    "a:pattFill",
    "a:grpFill",
    "a:effectLst",
    "a:effectDag",
    "a:highlight",
    "a:uLnTx",
    "a:uLn",
    "a:uFillTx",
    "a:uFill",
    "a:latin",
    "a:ea",
    "a:cs",
    "a:sym",
    "a:hlinkClick",
    "a:hlinkMouseOver",
    "a:rtl",
    "a:extLst",
];

/// `CT_Table`: `a:tblPr` first, then the grid, then the rows.
const TBL_ORDER: &[&str] = &["a:tblPr", "a:tblGrid", "a:tr"];

/// `CT_CatAx`, in schema sequence order. `c:txPr` sits between `c:spPr` and
/// `c:crossAx`, and an axis with it anywhere else is a chart PowerPoint
/// refuses to draw.
const CAT_AX_ORDER: &[&str] = &[
    "c:axId",
    "c:scaling",
    "c:delete",
    "c:axPos",
    "c:majorGridlines",
    "c:minorGridlines",
    "c:title",
    "c:numFmt",
    "c:majorTickMark",
    "c:minorTickMark",
    "c:tickLblPos",
    "c:spPr",
    "c:txPr",
    "c:crossAx",
    "c:crosses",
    "c:crossesAt",
    "c:auto",
    "c:lblAlgn",
    "c:lblOffset",
    "c:tickLblSkip",
    "c:tickMarkSkip",
    "c:noMultiLvlLbl",
    "c:extLst",
];

/// `CT_Legend`, in schema sequence order.
const LEGEND_ORDER: &[&str] = &[
    "c:legendPos",
    "c:legendEntry",
    "c:layout",
    "c:overlay",
    "c:spPr",
    "c:txPr",
    "c:extLst",
];

/// `CT_DLbls`, in schema sequence order: the individually formatted labels
/// first, then the settings that govern all of them.
const DLBLS_ORDER: &[&str] = &[
    "c:dLbl",
    "c:delete",
    "c:numFmt",
    "c:spPr",
    "c:txPr",
    "c:dLblPos",
    "c:showLegendKey",
    "c:showVal",
    "c:showCatName",
    "c:showSerName",
    "c:showPercent",
    "c:showBubbleSize",
    "c:separator",
    "c:showLeaderLines",
    "c:leaderLines",
    "c:extLst",
];

/// `CT_TextBody` as a chart declares it: all three children, in this order.
const TXPR_ORDER: &[&str] = &["a:bodyPr", "a:lstStyle", "a:p"];

/// The DrawingML namespace, for the rare chart part that does not already
/// declare a prefix for it.
const DML_NAMESPACE: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

/// `CT_TextParagraph`: `a:pPr` precedes every run.
const A_P_ORDER: &[&str] = &["a:pPr"];
/// `CT_RegularTextRun`: `a:rPr` precedes `a:t`.
const A_R_ORDER: &[&str] = &["a:rPr"];

/// The element a run's characters live in.
const A_T: &str = "a:t";

/// Right indent and hanging indent applied when a typed bullet is converted to
/// a native list. PowerPoint's own default for a first-level bullet, in EMU.
const BULLET_INDENT_EMU: i64 = 342_900;

/// The paragraph's `a:pPr`, created in schema position if it has none.
fn ensure_ppr(para: &mut Vec<Event<'static>>) -> usize {
    child_or_insert(para, 0, A_P_ORDER, "a:pPr")
}

/// Lower a direction-relative alignment onto DrawingML's `algn`, which has only
/// physical values.
///
/// `Start` is the side reading begins on: the right in RTL, the left in LTR.
fn drawingml_alignment(alignment: Alignment, rtl: bool) -> &'static str {
    match alignment {
        Alignment::Start if rtl => "r",
        Alignment::Start => "l",
        Alignment::End if rtl => "l",
        Alignment::End => "r",
        Alignment::Left => "l",
        Alignment::Right => "r",
        Alignment::Center => "ctr",
        Alignment::Justify => "just",
        Alignment::Distributed => "dist",
    }
}

/// Apply `f` to every run-property element in the paragraph, creating `a:rPr`
/// for runs that lack one.
///
/// Children are visited back to front: creating an `a:rPr` shifts every index
/// after it, and the ones still to be visited must not move.
fn for_each_run_property(
    para: &mut Vec<Event<'static>>,
    mut f: impl FnMut(&mut Vec<Event<'static>>, usize),
) {
    for i in token::direct_children(para).into_iter().rev() {
        match token::name_of(&para[i]).unwrap_or_default().as_str() {
            "a:r" => {
                let rpr = child_or_insert(para, i, A_R_ORDER, "a:rPr");
                f(para, rpr);
            }
            "a:endParaRPr" => f(para, i),
            "a:pPr" => {
                if let Some(default) = find_direct_child(para, i, "a:defRPr") {
                    f(para, default);
                }
            }
            _ => {}
        }
    }
}

/// Apply every repair for one paragraph.
///
/// `inherited` is the direction the paragraph takes from its container when
/// it declares none itself; see [`Inherited`].
fn apply_to_paragraph(
    para: &mut Vec<Event<'static>>,
    fixes: &[Fix],
    inherited: Option<Direction>,
) -> Result<()> {
    // Text first: these replace text events in place, so they neither move nor
    // are moved by the structural edits that follow. Controls before anything
    // else, whatever order the fixes arrived in: `RemoveControls` carries byte
    // offsets into the text as it was scanned, and both stripping a marker
    // from the front and mapping a three-byte form to a two-byte letter would
    // shift them.
    for fix in fixes {
        if let Fix::RemoveControls(offsets) = fix {
            token::remove_at_offsets(para, A_T, offsets);
        }
    }
    if fixes.contains(&Fix::NormalizePresentationForms) {
        // The mapping is the domain's: one character at a time, so a combining
        // mark, a Latin ligature or a word ligature beside a form is left
        // exactly as the author stored it. A run containing no form is not
        // rewritten at all, and keeps its character references verbatim.
        token::map_runs(para, A_T, script::normalize_presentation_forms);
    }
    for fix in fixes {
        if let Fix::ConvertLiteralBullet { marker } = fix {
            token::strip_leading_marker(para, A_T, *marker);
        }
    }

    // Direction is resolved before alignment, because a direction-relative
    // alignment cannot be lowered onto DrawingML without knowing it.
    let declared_rtl = fixes.iter().find_map(|f| match f {
        Fix::SetDirection(d) => Some(*d == Direction::Rtl),
        _ => None,
    });

    for fix in fixes {
        match fix {
            Fix::SetDirection(direction) => {
                let ppr = ensure_ppr(para);
                let value = if *direction == Direction::Rtl {
                    "1"
                } else {
                    "0"
                };
                edit_tag(para, ppr, |tag| set_attribute(tag, "rtl", value));
            }

            Fix::SetAlignment(alignment) => {
                let ppr = ensure_ppr(para);
                // The direction being repaired to, else the paragraph's own,
                // else what it inherits. Only then left-to-right.
                let rtl = declared_rtl.unwrap_or_else(|| {
                    let own = match &para[ppr] {
                        Event::Start(tag) | Event::Empty(tag) => {
                            get_attribute(tag, "rtl").map(|v| is_true(&v))
                        }
                        _ => None,
                    };
                    own.unwrap_or(inherited == Some(Direction::Rtl))
                });
                let value = drawingml_alignment(*alignment, rtl);
                edit_tag(para, ppr, |tag| set_attribute(tag, "algn", value));
            }

            Fix::SetLanguage(tag_value) => {
                for_each_run_property(para, |events, at| {
                    edit_tag(events, at, |tag| set_attribute(tag, "lang", tag_value));
                });
            }

            Fix::SetComplexFont(typeface) => {
                for_each_run_property(para, |events, at| {
                    let cs = child_or_insert(events, at, RPR_ORDER, "a:cs");
                    edit_tag(events, cs, |tag| set_attribute(tag, "typeface", typeface));
                });
            }

            Fix::ConvertLiteralBullet { marker } => {
                let ppr = ensure_ppr(para);
                // A hanging indent, so wrapped lines clear the marker.
                edit_tag(para, ppr, |tag| {
                    set_attribute(tag, "marR", &BULLET_INDENT_EMU.to_string())
                });
                edit_tag(para, ppr, |tag| {
                    set_attribute(tag, "indent", &(-BULLET_INDENT_EMU).to_string())
                });
                let bullet = child_or_insert(para, ppr, PPR_ORDER, "a:buChar");
                let marker = marker.to_string();
                edit_tag(para, bullet, |tag| set_attribute(tag, "char", &marker));
            }

            Fix::RemoveControls(_) | Fix::NormalizePresentationForms => {}
        }
    }

    Ok(())
}

/// Every `a:p` in the part, counting paragraphs that produced no text unit.
fn paragraph_ranges(events: &[Event<'static>]) -> Vec<(usize, std::ops::Range<usize>)> {
    element_ranges(events, "a:p")
}

/// Every `a:tbl` in the part.
fn table_ranges(events: &[Event<'static>]) -> Vec<(usize, std::ops::Range<usize>)> {
    element_ranges(events, "a:tbl")
}

/// Every `a:bodyPr` in the part, counting the bodies that produced no unit.
fn body_ranges(events: &[Event<'static>]) -> Vec<(usize, std::ops::Range<usize>)> {
    element_ranges(events, "a:bodyPr")
}

/// Apply every repair for one table, whose `a:tbl` starts at `at`.
///
/// A table has one property of its own this tool reasons about — its
/// direction, `a:tblPr/@rtl`, which decides which side the first column sits
/// on. Anything else a plan names on a table is a mistake upstream and is
/// refused rather than guessed at.
fn apply_to_table(
    part: &str,
    events: &mut Vec<Event<'static>>,
    at: usize,
    fixes: &[Fix],
) -> Result<()> {
    for fix in fixes {
        match fix {
            Fix::SetDirection(direction) => {
                let tblpr = child_or_insert(events, at, TBL_ORDER, "a:tblPr");
                let value = if *direction == Direction::Rtl {
                    "1"
                } else {
                    "0"
                };
                edit_tag(events, tblpr, |tag| set_attribute(tag, "rtl", value));
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

/// Apply every repair for one multi-column text body, whose `a:bodyPr` is at
/// `at`.
///
/// Like a table, such a body has one property of its own this tool reasons
/// about: `a:bodyPr/@rtlCol`, which decides whether the reader starts in the
/// leftmost column or the rightmost. The paragraphs inside keep their own
/// direction and are repaired as paragraphs.
fn apply_to_columns(
    part: &str,
    events: &mut [Event<'static>],
    at: usize,
    fixes: &[Fix],
) -> Result<()> {
    for fix in fixes {
        match fix {
            Fix::SetDirection(direction) => {
                let value = if *direction == Direction::Rtl {
                    "1"
                } else {
                    "0"
                };
                edit_tag(events, at, |tag| set_attribute(tag, "rtlCol", value));
            }
            other => {
                return Err(Error::Format(format!(
                    "{part}: cannot {other} on a text body; only its column direction is the body's own"
                )));
            }
        }
    }
    Ok(())
}

/// Every `c:catAx`, `c:legend` or `c:dLbls` in the part.
fn chart_text_ranges(
    events: &[Event<'static>],
    kind: ChartText,
) -> Vec<(usize, std::ops::Range<usize>)> {
    element_ranges(events, kind.element())
}

/// The schema sequence of the element a chart text container is.
fn chart_text_order(kind: ChartText) -> &'static [&'static str] {
    match kind {
        ChartText::CategoryAxis => CAT_AX_ORDER,
        ChartText::Legend => LEGEND_ORDER,
        ChartText::DataLabels => DLBLS_ORDER,
    }
}

/// Whether the part's root element declares a prefix bound to DrawingML.
///
/// A chart written by an application always does — its title is DrawingML —
/// but a chart with no text at all need not, and a `c:txPr` created in such a
/// part would carry an undeclared prefix and make the document unreadable.
/// When the declaration is missing the created element brings its own.
fn declares_drawingml(events: &[Event<'static>]) -> bool {
    events.iter().find_map(|event| match event {
        Event::Start(tag) | Event::Empty(tag) => Some(get_attribute(tag, "xmlns:a").is_some()),
        _ => None,
    }) == Some(true)
}

/// A `c:txPr` stating one direction and nothing else: the minimum a chart
/// container needs for its strings to have a direction at all.
fn text_properties(rtl: &str, declare_namespace: bool) -> Vec<Event<'static>> {
    let mut txpr = BytesStart::new("c:txPr");
    if declare_namespace {
        txpr.push_attribute(("xmlns:a", DML_NAMESPACE));
    }
    let mut ppr = BytesStart::new("a:pPr");
    ppr.push_attribute(("rtl", rtl));
    vec![
        Event::Start(txpr),
        Event::Empty(BytesStart::new("a:bodyPr")),
        Event::Empty(BytesStart::new("a:lstStyle")),
        Event::Start(BytesStart::new("a:p")),
        Event::Empty(ppr),
        Event::Empty(BytesStart::new("a:endParaRPr")),
        Event::End(BytesEnd::new("a:p")),
        Event::End(BytesEnd::new("c:txPr")),
    ]
}

/// Apply every repair for one chart text container, whose element starts at
/// `at`.
///
/// Its one property is the direction its strings are laid out in, held in
/// `c:txPr/a:p/a:pPr/@rtl`. Most charts have no `c:txPr`, so the usual case
/// is creating one; when there is one already, only that attribute changes.
fn apply_to_chart_text(
    part: &str,
    events: &mut Vec<Event<'static>>,
    at: usize,
    kind: ChartText,
    fixes: &[Fix],
    declare_namespace: bool,
) -> Result<()> {
    for fix in fixes {
        let Fix::SetDirection(direction) = fix else {
            return Err(Error::Format(format!(
                "{part}: cannot {fix} on a chart's {}; only its direction is the container's own",
                kind.label()
            )));
        };
        let rtl = if *direction == Direction::Rtl {
            "1"
        } else {
            "0"
        };

        let Some(txpr) = find_direct_child(events, at, "c:txPr") else {
            insert_children(
                events,
                at,
                chart_text_order(kind),
                text_properties(rtl, declare_namespace),
            );
            continue;
        };
        // A `c:txPr` with no paragraph is not a document an application
        // writes, but the direction still has to land somewhere.
        let Some(paragraph) = find_direct_child(events, txpr, "a:p") else {
            let mut ppr = BytesStart::new("a:pPr");
            ppr.push_attribute(("rtl", rtl));
            insert_children(
                events,
                txpr,
                TXPR_ORDER,
                vec![
                    Event::Start(BytesStart::new("a:p")),
                    Event::Empty(ppr),
                    Event::End(BytesEnd::new("a:p")),
                ],
            );
            continue;
        };
        let ppr = child_or_insert(events, paragraph, A_P_ORDER, "a:pPr");
        edit_tag(events, ppr, |tag| set_attribute(tag, "rtl", rtl));
    }
    Ok(())
}

/// Apply repairs to a part, leaving every token they do not address untouched.
///
/// Every paragraph is taken to declare its own direction or to be
/// left-to-right; use [`apply_with`] when the adapter knows better.
pub fn apply(part: &str, xml: &str, fixes: &PartFixes) -> Result<String> {
    apply_with(part, xml, fixes, &Inherited::new())
}

/// [`apply`], with the direction each paragraph inherits from its container.
pub fn apply_with(
    part: &str,
    xml: &str,
    fixes: &PartFixes,
    inherited: &Inherited,
) -> Result<String> {
    let plan = PartPlan {
        paragraphs: fixes.clone(),
        ..Default::default()
    };
    apply_plan(part, xml, &plan, inherited)
}

/// Apply a whole part's plan — paragraphs and tables — leaving every token it
/// does not address untouched.
pub fn apply_plan(part: &str, xml: &str, plan: &PartPlan, inherited: &Inherited) -> Result<String> {
    if plan.is_empty() {
        return token::passthrough(part, xml);
    }

    let mut events = token::read_events(part, xml)?;

    if let Some(missing) = plan
        .paragraphs
        .keys()
        .find(|k| !paragraph_ranges(&events).iter().any(|(i, _)| i == *k))
    {
        return Err(Error::Format(format!(
            "{part}: no paragraph {missing}; the document and the report disagree"
        )));
    }
    if let Some(missing) = plan
        .tables
        .keys()
        .find(|k| !table_ranges(&events).iter().any(|(i, _)| i == *k))
    {
        return Err(Error::Format(format!(
            "{part}: no table {missing}; the document and the report disagree"
        )));
    }
    if let Some(missing) = plan
        .columns
        .keys()
        .find(|k| !body_ranges(&events).iter().any(|(i, _)| i == *k))
    {
        return Err(Error::Format(format!(
            "{part}: no text body {missing}; the document and the report disagree"
        )));
    }
    if let Some((kind, missing)) = plan.chart_text.keys().find(|(kind, index)| {
        !chart_text_ranges(&events, *kind)
            .iter()
            .any(|(i, _)| i == index)
    }) {
        return Err(Error::Format(format!(
            "{part}: no {} {missing}; the document and the report disagree",
            kind.label()
        )));
    }

    // Back to front: splicing a paragraph changes every index after it.
    for (index, range) in paragraph_ranges(&events).into_iter().rev() {
        let Some(list) = plan.paragraphs.get(&index) else {
            continue;
        };
        let mut para: Vec<Event<'static>> = events[range.clone()].to_vec();
        apply_to_paragraph(&mut para, list, inherited.get(&index).copied())?;
        events.splice(range, para);
    }

    // Tables after paragraphs, and again back to front: a paragraph edit
    // inside a table moved its range, and a `a:tblPr` created on one table
    // moves every table after it.
    for (index, range) in table_ranges(&events).into_iter().rev() {
        if let Some(list) = plan.tables.get(&index) {
            apply_to_table(part, &mut events, range.start, list)?;
        }
    }

    // Text bodies next, their ranges taken after both of the above have
    // moved them. A body's own repair is one attribute on one tag, so it
    // moves nothing itself.
    for (index, range) in body_ranges(&events).into_iter().rev() {
        if let Some(list) = plan.columns.get(&index) {
            apply_to_columns(part, &mut events, range.start, list)?;
        }
    }

    // Chart containers last of all, because creating a `c:txPr` adds both a
    // paragraph and a text body to the part, and every ordinal above counts
    // those. Ranges are taken per kind and applied back to front.
    let declare_namespace = !declares_drawingml(&events);
    for kind in ChartText::all() {
        for (index, range) in chart_text_ranges(&events, kind).into_iter().rev() {
            if let Some(list) = plan.chart_text.get(&(kind, index)) {
                apply_to_chart_text(
                    part,
                    &mut events,
                    range.start,
                    kind,
                    list,
                    declare_namespace,
                )?;
            }
        }
    }

    token::write_events(part, &events)
}
