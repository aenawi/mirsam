//! PowerPoint (PPTX) adapter.

use crate::package::{Edits, Package};
use crate::rewrite::{self, Inherited, PartPlan};
use mirsam_core::error::{Error, Result};
use mirsam_core::fix::Repair;
use mirsam_core::ports::{DocumentReader, DocumentWriter};
use mirsam_core::text::{
    Alignment, Bullet, Direction, Location, Properties, Resolved, TextUnit, UnitId, UnitKind,
};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Alignment values DrawingML understands.
fn parse_alignment(value: &str) -> Option<Alignment> {
    Some(match value {
        "l" => Alignment::Left,
        "r" => Alignment::Right,
        "ctr" => Alignment::Center,
        "just" | "justLow" => Alignment::Justify,
        "dist" | "thaiDist" => Alignment::Distributed,
        _ => return None,
    })
}

/// An `ST_OnOff`-style boolean attribute, as both the scanner and the
/// rewriter read it — one definition so the two cannot disagree.
pub(crate) fn is_true(value: &str) -> bool {
    matches!(value, "1" | "true" | "on")
}

/// The unit id this adapter issues for a paragraph: the part name and the
/// paragraph's 1-based ordinal, which is exactly what the rewriter needs to
/// find it again.
fn unit_id(part: &str, index: usize) -> String {
    format!("{part}#p{index}")
}

/// The unit id for a table: the part name and the table's 1-based ordinal.
fn table_id(part: &str, index: usize) -> String {
    format!("{part}#tbl{index}")
}

/// The unit id for a multi-column text body: the part name and the body's
/// 1-based ordinal.
///
/// The ordinal counts *every* `a:bodyPr` in the part, not only the bodies
/// laid out in columns, exactly as the paragraph ordinal counts every `a:p`
/// including the ones that produce no unit. A numbering that skipped the
/// bodies this adapter has nothing to say about would drift the moment a
/// single-column body was added, and the rewriter would edit the wrong one.
fn columns_id(part: &str, index: usize) -> String {
    format!("{part}#cols{index}")
}

/// What a unit id this adapter issued points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Paragraph(usize),
    Table(usize),
    Columns(usize),
}

/// Builds a [`Target`] from the ordinal an id carries.
type MakeTarget = fn(usize) -> Target;

/// Recover the part and target from an id this adapter issued.
///
/// `#` cannot occur in an OPC part name, so the last `#` is unambiguous.
fn parse_unit_id(id: &UnitId) -> Option<(&str, Target)> {
    let (part, rest) = id.0.rsplit_once('#')?;
    let prefixes: [(&str, MakeTarget); 3] = [
        ("tbl", Target::Table),
        ("cols", Target::Columns),
        ("p", Target::Paragraph),
    ];
    let (target, digits) = prefixes
        .into_iter()
        .find_map(|(prefix, target)| Some((target, rest.strip_prefix(prefix)?)))?;
    let index: usize = digits.parse().ok()?;
    (!part.is_empty() && index > 0).then_some((part, target(index)))
}

/// Accumulates a container — a table, or a text body in columns — while the
/// paragraphs inside it are being parsed.
struct ContainerBuilder {
    /// The index the unit id carries: the table's, or the text body's.
    index: usize,
    /// Every enclosed paragraph's text, one per line.
    text: String,
    direction: Resolved<Direction>,
    shape: Option<String>,
}

impl ContainerBuilder {
    fn new(index: usize, direction: Resolved<Direction>, shape: Option<String>) -> Self {
        Self {
            index,
            text: String::new(),
            direction,
            shape,
        }
    }

    fn push(&mut self, paragraph: &str) {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(paragraph);
    }

    /// The unit, unless the container laid out no text at all.
    fn finish(
        self,
        part: &str,
        kind: UnitKind,
        id: impl Fn(&str, usize) -> String,
    ) -> Option<TextUnit> {
        if self.text.trim().is_empty() {
            return None;
        }
        Some(
            TextUnit::new(id(part, self.index), self.text)
                .with_kind(kind)
                .with_props(Properties {
                    direction: self.direction,
                    ..Default::default()
                })
                .with_location(Location {
                    part: part.to_string(),
                    paragraph: None,
                    container: self.shape,
                }),
        )
    }
}

/// The direction an `ST_OnOff` attribute states.
fn direction_of(value: &str) -> Direction {
    if is_true(value) {
        Direction::Rtl
    } else {
        Direction::Ltr
    }
}

/// Accumulates the properties of the paragraph currently being parsed.
#[derive(Default)]
struct ParagraphBuilder {
    text: String,
    props: Properties,
    shape: Option<String>,
}

impl ParagraphBuilder {
    fn finish(self, part: &str, index: usize) -> TextUnit {
        TextUnit::new(unit_id(part, index), self.text)
            .with_props(self.props)
            .with_location(Location {
                part: part.to_string(),
                paragraph: Some(index),
                container: self.shape,
            })
    }
}

/// A PowerPoint package opened for auditing or repair.
///
/// Reading and repair share one [`Package`], deliberately: two ZIP code paths
/// would be two places for the byte-preservation guarantee to be broken, and
/// only one of them is covered by the round-trip test.
pub struct PptxDocument {
    package: Package,
    /// Parts rewritten by [`DocumentWriter::apply`], awaiting
    /// [`DocumentWriter::write`]. Everything else is copied raw.
    edits: Edits,
}

impl PptxDocument {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            package: Package::open(path)?,
            edits: Edits::new(),
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

    /// Parts this adapter reads: PowerPoint's own XML, excluding relationships.
    fn text_parts(&self) -> Result<Vec<String>> {
        let mut parts = self
            .package
            .parts_where(|n| n.starts_with("ppt/") && n.ends_with(".xml"))?;
        parts.sort();
        Ok(parts)
    }

    /// Parse one `ppt/**/*.xml` part into text units.
    ///
    /// Direction, alignment and language are recorded as `Explicit` only when
    /// the paragraph itself carries them. Resolving the layout/master
    /// inheritance chain — which turns many `Unset`s into `Inherited` — is
    /// milestone M2; until then the engine deliberately reports an absent
    /// property as a warning rather than an error.
    fn scan_part(part: &str, xml: &str) -> Result<Vec<TextUnit>> {
        let mut reader = Reader::from_str(xml);
        let mut units = Vec::new();
        let mut current: Option<ParagraphBuilder> = None;
        let mut shape: Option<String> = None;
        let mut body_rtl: Option<bool> = None;
        let mut in_text = false;
        let mut paragraph_index = 0usize;
        let mut table: Option<ContainerBuilder> = None;
        let mut table_index = 0usize;
        let mut columns: Option<ContainerBuilder> = None;
        let mut body_index = 0usize;

        loop {
            match reader.read_event() {
                Err(e) => return Err(Error::Format(format!("{part}: {e}"))),
                Ok(Event::Eof) => break,

                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    let name = e.name().as_ref().to_string();
                    match name.as_str() {
                        "p:cNvPr" => {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == "name" {
                                    shape = attr
                                        .normalized_value(XmlVersion::Implicit1_0)
                                        .ok()
                                        .map(|v| v.into_owned())
                                        .filter(|v| !v.is_empty());
                                }
                            }
                        }
                        "a:bodyPr" => {
                            body_index += 1;
                            let attribute = |name: &str| {
                                e.attributes()
                                    .flatten()
                                    .find(|a| a.key.as_ref() == name)
                                    .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                                    .map(|v| v.into_owned())
                            };
                            body_rtl = attribute("rtlCol").map(|v| is_true(&v));
                            // Only a body actually laid out in columns is a
                            // container of its own: `rtlCol` on a single
                            // column changes nothing a reader sees.
                            let column_count: usize = attribute("numCol")
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(1);
                            columns = (column_count >= 2).then(|| {
                                ContainerBuilder::new(
                                    body_index,
                                    match body_rtl {
                                        Some(true) => Resolved::Explicit(Direction::Rtl),
                                        Some(false) => Resolved::Explicit(Direction::Ltr),
                                        None => Resolved::Unset,
                                    },
                                    shape.clone(),
                                )
                            });
                        }
                        "a:tbl" => {
                            table_index += 1;
                            table = Some(ContainerBuilder::new(
                                table_index,
                                Resolved::Unset,
                                shape.clone(),
                            ));
                        }
                        "a:tblPr" => {
                            if let Some(t) = table.as_mut()
                                && let Some(value) = e
                                    .attributes()
                                    .flatten()
                                    .find(|a| a.key.as_ref() == "rtl")
                                    .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                            {
                                t.direction = Resolved::Explicit(direction_of(&value));
                            }
                        }
                        "a:p" => {
                            paragraph_index += 1;
                            let mut builder = ParagraphBuilder {
                                shape: shape.clone(),
                                ..Default::default()
                            };
                            // A right-to-left text body is inherited context
                            // for every paragraph inside it.
                            if body_rtl == Some(true) {
                                builder.props.direction = Resolved::Inherited(Direction::Rtl);
                            }
                            current = Some(builder);
                        }
                        "a:pPr" => {
                            if let Some(b) = current.as_mut() {
                                for attr in e.attributes().flatten() {
                                    let Ok(value) = attr.normalized_value(XmlVersion::Implicit1_0)
                                    else {
                                        continue;
                                    };
                                    match attr.key.as_ref() {
                                        "rtl" => {
                                            b.props.direction =
                                                Resolved::Explicit(direction_of(&value));
                                        }
                                        "algn" => {
                                            if let Some(a) = parse_alignment(&value) {
                                                b.props.alignment = Resolved::Explicit(a);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        "a:buChar" | "a:buAutoNum" | "a:buBlip" => {
                            if let Some(b) = current.as_mut() {
                                b.props.bullet = Bullet::Native;
                            }
                        }
                        "a:buNone" => {
                            if let Some(b) = current.as_mut() {
                                b.props.bullet = Bullet::Suppressed;
                            }
                        }
                        "a:rPr" | "a:defRPr" | "a:endParaRPr" => {
                            if let Some(b) = current.as_mut() {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == "lang"
                                        && b.props.language.is_unset()
                                        && let Ok(value) =
                                            attr.normalized_value(XmlVersion::Implicit1_0)
                                    {
                                        b.props.language = Resolved::Explicit(value.into_owned());
                                    }
                                }
                            }
                        }
                        "a:cs" | "a:latin" => {
                            if let Some(b) = current.as_mut() {
                                let typeface = e
                                    .attributes()
                                    .flatten()
                                    .find(|a| a.key.as_ref() == "typeface")
                                    .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                                    .map(|v| v.into_owned())
                                    .filter(|v| !v.is_empty());
                                if let Some(typeface) = typeface {
                                    let slot = if name == "a:cs" {
                                        &mut b.props.complex_font
                                    } else {
                                        &mut b.props.latin_font
                                    };
                                    if slot.is_unset() {
                                        *slot = Resolved::Explicit(typeface);
                                    }
                                }
                            }
                        }
                        "a:t" => in_text = true,
                        _ => {}
                    }
                }

                Ok(Event::Text(e)) if in_text => {
                    if let Some(b) = current.as_mut() {
                        let raw = e.xml10_content();
                        match quick_xml::escape::unescape(raw.as_ref()) {
                            Ok(text) => b.text.push_str(text.as_ref()),
                            // Unresolvable custom entity: keep the raw form
                            // rather than dropping the run's text entirely.
                            Err(_) => b.text.push_str(raw.as_ref()),
                        }
                    }
                }

                // A character or entity reference is a separate event from the
                // text around it. Office routinely writes Arabic this way, so
                // ignoring these silently empties the run — and an empty run is
                // dropped, which turns a defective paragraph into no finding at
                // all. This tool exists to reason about the text; it has to see
                // all of it.
                Ok(Event::GeneralRef(e)) if in_text => {
                    if let Some(b) = current.as_mut() {
                        let reference = e.as_ref();
                        match quick_xml::escape::unescape(&format!("&{reference};")) {
                            Ok(text) => b.text.push_str(text.as_ref()),
                            Err(_) => {
                                b.text.push('&');
                                b.text.push_str(reference);
                                b.text.push(';');
                            }
                        }
                    }
                }

                Ok(Event::End(e)) => match e.name().as_ref() {
                    "a:t" => in_text = false,
                    "a:p" => {
                        if let Some(b) = current.take()
                            && !b.text.trim().is_empty()
                        {
                            // An enclosed paragraph is its container's text
                            // too: a container is judged from what it lays
                            // out — a table from its cells, a multi-column
                            // body from its columns.
                            if let Some(t) = table.as_mut() {
                                t.push(&b.text);
                            }
                            if let Some(c) = columns.as_mut() {
                                c.push(&b.text);
                            }
                            units.push(b.finish(part, paragraph_index));
                        }
                    }
                    "a:tbl" => {
                        units.extend(
                            table
                                .take()
                                .and_then(|t| t.finish(part, UnitKind::Table, table_id)),
                        );
                    }
                    "p:sp" | "p:graphicFrame" | "p:pic" | "p:cxnSp" => shape = None,
                    "p:txBody" | "a:txBody" | "c:txPr" => {
                        units.extend(
                            columns
                                .take()
                                .and_then(|c| c.finish(part, UnitKind::Columns, columns_id)),
                        );
                        body_rtl = None;
                    }
                    _ => {}
                },

                Ok(_) => {}
            }
        }
        Ok(units)
    }
}

impl DocumentReader for PptxDocument {
    fn format(&self) -> &'static str {
        "pptx"
    }

    fn scan(&mut self) -> Result<Vec<TextUnit>> {
        let mut units = Vec::new();
        for part in self.text_parts()? {
            let xml = self.package.read_text(&part)?;
            units.extend(Self::scan_part(&part, &xml)?);
        }
        Ok(units)
    }
}

impl DocumentWriter for PptxDocument {
    /// Stage repairs against the parts they name.
    ///
    /// Repairs are grouped by part and then by paragraph, so a part is read
    /// and rewritten once however many paragraphs it carries. Nothing is
    /// staged unless every part succeeds: a failure half-way through must not
    /// leave a document that is partly repaired and reports otherwise.
    fn apply(&mut self, repairs: &[Repair]) -> Result<usize> {
        let mut by_part: BTreeMap<&str, PartPlan> = BTreeMap::new();
        for repair in repairs {
            let Some((part, target)) = parse_unit_id(&repair.unit) else {
                return Err(Error::Format(format!(
                    "{}: not a unit this adapter produced",
                    repair.unit
                )));
            };
            let plan = by_part.entry(part).or_default();
            match target {
                Target::Paragraph(index) => plan.paragraphs.entry(index).or_default(),
                Target::Table(index) => plan.tables.entry(index).or_default(),
                Target::Columns(index) => plan.columns.entry(index).or_default(),
            }
            .push(repair.fix.clone());
        }

        let mut staged = Edits::new();
        let mut applied = 0usize;
        for (part, plan) in by_part {
            // A part edited by an earlier call is edited again from its staged
            // bytes, so repairs applied in two rounds compose rather than the
            // second round discarding the first.
            let xml = match self.edits.get(part) {
                Some(bytes) => String::from_utf8(bytes.clone())
                    .map_err(|e| Error::Format(format!("{part}: {e}")))?,
                None => self.package.read_text(part)?,
            };

            // What each paragraph inherits from its container, resolved by
            // the same scanner that produced the units the rules judged. The
            // rewriter cannot see a container from inside a paragraph.
            let inherited: Inherited = Self::scan_part(part, &xml)?
                .into_iter()
                .filter_map(|unit| {
                    let Resolved::Inherited(direction) = unit.props.direction else {
                        return None;
                    };
                    match parse_unit_id(&unit.id) {
                        Some((_, Target::Paragraph(index))) => Some((index, direction)),
                        _ => None,
                    }
                })
                .collect();

            let rewritten = rewrite::apply_plan(part, &xml, &plan, &inherited)?;
            applied += plan.len();
            staged.insert(part.to_string(), rewritten.into_bytes());
        }

        self.edits.extend(staged);
        Ok(applied)
    }

    fn write(&mut self, dest: &Path) -> Result<()> {
        self.package.rewrite(dest, &self.edits)?;
        Ok(())
    }
}

/// Parse an in-memory part. Exposed for tests and for callers that already
/// hold the XML.
pub fn scan_xml(part: &str, xml: &str) -> Result<Vec<TextUnit>> {
    PptxDocument::scan_part(part, xml)
}

#[cfg(test)]
mod unit_id_tests {
    use super::*;

    #[test]
    fn a_unit_id_round_trips_through_its_own_parser() {
        let id = UnitId(unit_id("ppt/slides/slide1.xml", 3));
        assert_eq!(
            parse_unit_id(&id),
            Some(("ppt/slides/slide1.xml", Target::Paragraph(3)))
        );
        let id = UnitId(table_id("ppt/slides/slide1.xml", 2));
        assert_eq!(
            parse_unit_id(&id),
            Some(("ppt/slides/slide1.xml", Target::Table(2)))
        );
    }

    #[test]
    fn an_id_this_adapter_did_not_issue_is_rejected() {
        for foreign in [
            "",
            "slide1",
            "#p1",
            "ppt/slides/slide1.xml#p0",
            "x#px",
            "x#p-1",
            "x#tbl0",
            "x#tblx",
            "x#t1",
        ] {
            assert_eq!(parse_unit_id(&UnitId(foreign.into())), None, "{foreign:?}");
        }
    }
}
