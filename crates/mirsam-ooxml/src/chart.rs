//! Chart text containers: the strings a chart draws that are not paragraphs.
//!
//! A chart's category labels, series names and data labels are not `a:p`
//! elements. They are values the chart caches — `c:strCache/c:pt/c:v` — and
//! the direction they are laid out in comes from the *container* that draws
//! them: `c:catAx/c:txPr`, `c:legend/c:txPr`, `c:dLbls/c:txPr`, each an
//! `a:bodyPr` + `a:lstStyle` + one `a:p` whose `a:pPr/@rtl` governs every
//! string in that container. Most generated charts have no `c:txPr` at all,
//! so Arabic labels are drawn with no direction selected — found by opening
//! the corpus deck in PowerPoint 2016 (#18).
//!
//! The paragraph scanner cannot see any of this: it finds the chart title,
//! which *is* an `a:p`, and nothing else. This module is the second pass that
//! finds the containers, works out which strings each one actually draws, and
//! lowers them into container units the same rule judges.
//!
//! **What each container draws is read from the file, never assumed.** A
//! category axis draws the categories of the chart group that names it in
//! `c:axId`; a legend draws its series' names, or its categories when the
//! chart is a pie; a set of data labels draws whichever of those its
//! `c:showCatName` / `c:showSerName` flags turn on. A container whose strings
//! cannot be identified produces no unit, because a finding on text the tool
//! cannot show is a finding a reviewer cannot check.
//!
//! **Deliberately not covered.** A value axis draws formatted numbers, and
//! the chart-space-level `c:txPr` is a default for text other containers
//! draw rather than a container that draws any of its own. Neither has
//! strings of its own to judge, so neither is a unit; both would have to
//! guess, and this tool reports only what it can show.

use mirsam_core::error::{Error, Result};
use mirsam_core::text::{Direction, Location, Properties, Resolved, TextUnit, UnitKind};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use crate::token::is_true;

/// A chart element whose `c:txPr` decides how the strings it draws are laid
/// out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChartText {
    CategoryAxis,
    Legend,
    DataLabels,
}

impl ChartText {
    /// The element this container is.
    pub fn element(self) -> &'static str {
        match self {
            Self::CategoryAxis => "c:catAx",
            Self::Legend => "c:legend",
            Self::DataLabels => "c:dLbls",
        }
    }

    /// The fragment its unit id carries, after the part name.
    pub fn tag(self) -> &'static str {
        match self {
            Self::CategoryAxis => "catax",
            Self::Legend => "legend",
            Self::DataLabels => "dlbls",
        }
    }

    /// What a report calls it.
    pub fn label(self) -> &'static str {
        match self {
            Self::CategoryAxis => "category axis",
            Self::Legend => "legend",
            Self::DataLabels => "data labels",
        }
    }

    /// Every container this module knows.
    pub fn all() -> [Self; 3] {
        [Self::CategoryAxis, Self::Legend, Self::DataLabels]
    }
}

/// The unit id this adapter issues for a chart text container: the part, the
/// container's kind, and its 1-based ordinal among the elements of that kind
/// in the part — which is exactly how the rewriter finds it again.
pub fn unit_id(part: &str, kind: ChartText, index: usize) -> String {
    format!("{part}#{}{index}", kind.tag())
}

/// Chart types whose legend lists the categories rather than the series.
const PIE_FAMILY: &[&str] = &[
    "c:pieChart",
    "c:pie3DChart",
    "c:ofPieChart",
    "c:doughnutChart",
];

/// One series' cached strings.
#[derive(Default)]
struct Series {
    name: Option<String>,
    categories: Vec<String>,
}

/// One chart group — `c:barChart`, `c:lineChart` and the rest — with the
/// strings its series cache and the axes it names.
#[derive(Default)]
struct Group {
    series: Vec<Series>,
    ax_ids: Vec<String>,
    pie: bool,
}

impl Group {
    /// Every category the group's series name, once each and in order: the
    /// series of one group normally cache the same list.
    fn categories(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for value in self.series.iter().flat_map(|s| &s.categories) {
            if !out.contains(&value.as_str()) {
                out.push(value);
            }
        }
        out
    }

    fn names(&self) -> Vec<&str> {
        self.series
            .iter()
            .filter_map(|s| s.name.as_deref())
            .collect()
    }
}

/// Where a set of data labels sits, and so which strings it draws.
#[derive(Clone, Copy)]
enum Scope {
    /// One series of one group.
    Series(usize, usize),
    /// Every series of one group.
    Group(usize),
    /// Neither — a `c:dLbls` outside any chart group, which draws nothing
    /// this module can name.
    Loose,
}

/// A container found in the part, before the strings it draws are resolved.
struct Container {
    kind: ChartText,
    index: usize,
    direction: Resolved<Direction>,
    /// `c:catAx` only: the axis identifier the chart groups refer to.
    ax_id: Option<String>,
    /// `c:dLbls` only.
    scope: Scope,
    show_categories: bool,
    show_series: bool,
}

/// Join the strings a container draws exactly as a table joins its cells.
fn join(values: &[&str]) -> String {
    values.join("\n")
}

/// Read an attribute's normalised value.
fn attribute(tag: &BytesStart, name: &str) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

/// A chart group's element name: `c:barChart`, `c:scatterChart`, and the
/// rest. `c:chart` and `c:chartSpace` are deliberately not among them.
fn is_chart_group(name: &str) -> bool {
    name.starts_with("c:") && name.ends_with("Chart")
}

/// Whether this part is a chart. Reads only as far as the root element.
fn is_chart_part(xml: &str) -> bool {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                return e.name().as_ref() == "c:chartSpace";
            }
            Ok(Event::Eof) | Err(_) => return false,
            Ok(_) => {}
        }
    }
}

/// Find the chart text containers of one part and lower them into units.
///
/// Returns nothing at all for a part that is not a chart, so the caller can
/// hand it every part of the package without knowing which is which.
pub fn scan(part: &str, xml: &str) -> Result<Vec<TextUnit>> {
    if !is_chart_part(xml) {
        return Ok(Vec::new());
    }

    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<String> = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    let mut containers: Vec<Container> = Vec::new();
    let mut counts = [0usize; 3];
    let mut group: Option<usize> = None;
    // The container currently being parsed, as an index into `containers`.
    // Chart containers do not nest inside one another.
    let mut open: Option<usize> = None;
    // Whether we are inside the open container's *own* `c:txPr` — not a
    // title's, and not an individual data label's.
    let mut in_text_properties = false;
    let mut value: Option<String> = None;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| Error::Format(format!("{part}: {e}")))?;

        // An empty element opens and closes in one event; walking the stack
        // for both keeps every "what is my parent" test in one place.
        let (name, tag, opens, closes) = match &event {
            Event::Eof => break,
            Event::Start(e) => (e.name().as_ref().to_string(), Some(e), true, false),
            Event::Empty(e) => (e.name().as_ref().to_string(), Some(e), true, true),
            Event::End(e) => (e.name().as_ref().to_string(), None, false, true),
            Event::Text(e) => {
                if let Some(v) = value.as_mut() {
                    let raw = e.xml10_content();
                    match quick_xml::escape::unescape(raw.as_ref()) {
                        Ok(text) => v.push_str(text.as_ref()),
                        Err(_) => v.push_str(raw.as_ref()),
                    }
                }
                continue;
            }
            // Office writes Arabic as character references as readily as it
            // writes it raw; the paragraph scanner learned this the hard way.
            Event::GeneralRef(e) => {
                if let Some(v) = value.as_mut() {
                    let reference = e.as_ref();
                    match quick_xml::escape::unescape(&format!("&{reference};")) {
                        Ok(text) => v.push_str(text.as_ref()),
                        Err(_) => {
                            v.push('&');
                            v.push_str(reference);
                            v.push(';');
                        }
                    }
                }
                continue;
            }
            _ => continue,
        };

        if opens {
            stack.push(name.clone());
            let parent = || stack.get(stack.len().wrapping_sub(2)).map(String::as_str);
            let within = |element: &str| stack.iter().any(|n| n == element);

            if is_chart_group(&name) {
                groups.push(Group {
                    pie: PIE_FAMILY.contains(&name.as_str()),
                    ..Default::default()
                });
                group = Some(groups.len() - 1);
            } else if name == "c:ser" {
                if let Some(g) = group {
                    groups[g].series.push(Series::default());
                }
            } else if name == "c:axId" {
                let id = tag.and_then(|t| attribute(t, "val"));
                match (parent(), id) {
                    (Some(p), Some(id)) if is_chart_group(p) => {
                        if let Some(g) = group {
                            groups[g].ax_ids.push(id);
                        }
                    }
                    (Some("c:catAx"), Some(id)) => {
                        if let Some(c) = open {
                            containers[c].ax_id = Some(id);
                        }
                    }
                    _ => {}
                }
            } else if let Some(kind) = ChartText::all().into_iter().find(|k| k.element() == name) {
                counts[kind as usize] += 1;
                let scope = match (group, within("c:ser")) {
                    (Some(g), true) => Scope::Series(g, groups[g].series.len().saturating_sub(1)),
                    (Some(g), false) => Scope::Group(g),
                    (None, _) => Scope::Loose,
                };
                containers.push(Container {
                    kind,
                    index: counts[kind as usize],
                    direction: Resolved::Unset,
                    ax_id: None,
                    scope,
                    show_categories: false,
                    show_series: false,
                });
                open = Some(containers.len() - 1);
            } else if name == "c:showCatName" || name == "c:showSerName" {
                if let Some(c) = open
                    && parent() == Some(containers[c].kind.element())
                    && let Some(on) = tag.and_then(|t| attribute(t, "val"))
                {
                    let slot = if name == "c:showCatName" {
                        &mut containers[c].show_categories
                    } else {
                        &mut containers[c].show_series
                    };
                    *slot = is_true(&on);
                }
            } else if name == "c:txPr" {
                // The container's own text properties, not those of a title
                // or of one individually formatted label inside it.
                in_text_properties =
                    open.is_some_and(|c| parent() == Some(containers[c].kind.element()));
            } else if name == "a:pPr" {
                if in_text_properties
                    && let Some(c) = open
                    && containers[c].direction.is_unset()
                    && let Some(rtl) = tag.and_then(|t| attribute(t, "rtl"))
                {
                    containers[c].direction = Resolved::Explicit(if is_true(&rtl) {
                        Direction::Rtl
                    } else {
                        Direction::Ltr
                    });
                }
            } else if name == "c:v" {
                value = Some(String::new());
            }
        }

        if closes {
            if name == "c:v" {
                let text = value.take().unwrap_or_default();
                // Only the string caches: a numeric cache holds the plotted
                // values, which have no direction to get wrong.
                if stack.iter().any(|n| n == "c:strCache")
                    && let Some(g) = group
                    && let Some(series) = groups[g].series.last_mut()
                    && !text.trim().is_empty()
                {
                    if stack.iter().any(|n| n == "c:cat") {
                        series.categories.push(text);
                    } else if stack.iter().any(|n| n == "c:tx") && series.name.is_none() {
                        series.name = Some(text);
                    }
                }
            } else if name == "c:txPr" {
                in_text_properties = false;
            } else if ChartText::all().iter().any(|k| k.element() == name) {
                open = None;
            } else if is_chart_group(&name) {
                group = None;
            }
            stack.pop();
        }
    }

    Ok(containers
        .into_iter()
        .filter_map(|container| {
            let strings = match container.kind {
                // The categories of the chart group that names this axis.
                ChartText::CategoryAxis => {
                    let id = container.ax_id.as_deref()?;
                    let group = groups.iter().find(|g| g.ax_ids.iter().any(|a| a == id))?;
                    join(&group.categories())
                }
                // Every group's series names — or its categories, on a pie,
                // where the legend lists the slices.
                ChartText::Legend => {
                    let mut drawn: Vec<&str> = Vec::new();
                    for group in &groups {
                        let strings = if group.pie {
                            group.categories()
                        } else {
                            group.names()
                        };
                        for value in strings {
                            if !drawn.contains(&value) {
                                drawn.push(value);
                            }
                        }
                    }
                    join(&drawn)
                }
                // Whichever of them the labels are set to show. Labels
                // showing only values draw numbers, and are not a unit.
                ChartText::DataLabels => {
                    let group = match container.scope {
                        Scope::Series(g, _) | Scope::Group(g) => groups.get(g)?,
                        Scope::Loose => return None,
                    };
                    let mut drawn: Vec<&str> = Vec::new();
                    if container.show_categories {
                        match container.scope {
                            Scope::Series(_, s) => drawn.extend(
                                group
                                    .series
                                    .get(s)
                                    .into_iter()
                                    .flat_map(|s| s.categories.iter().map(String::as_str)),
                            ),
                            _ => drawn.extend(group.categories()),
                        }
                    }
                    if container.show_series {
                        match container.scope {
                            Scope::Series(_, s) => {
                                drawn.extend(group.series.get(s).and_then(|s| s.name.as_deref()))
                            }
                            _ => drawn.extend(group.names()),
                        }
                    }
                    join(&drawn)
                }
            };
            if strings.trim().is_empty() {
                return None;
            }
            Some(
                TextUnit::new(unit_id(part, container.kind, container.index), strings)
                    .with_kind(UnitKind::ChartText)
                    .with_props(Properties {
                        direction: container.direction,
                        ..Default::default()
                    })
                    .with_location(Location {
                        part: part.to_string(),
                        paragraph: None,
                        container: Some(container.kind.label().to_string()),
                    }),
            )
        })
        .collect())
}
