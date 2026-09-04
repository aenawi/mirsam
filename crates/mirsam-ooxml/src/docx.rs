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
//! `@w:ascii`.
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
//! ## Reading only, for now
//!
//! [`DocumentWriter`] is deliberately not implemented. `DocumentReader` and
//! `DocumentWriter` are separate ports precisely so an adapter can arrive one
//! half at a time, and claiming a repair this crate cannot yet express in
//! WordprocessingML would be the kind of inferred capability `AGENTS.md`
//! forbids.
//!
//! [`DocumentWriter`]: mirsam_core::ports::DocumentWriter
//! [MS-OE376]: https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/26ecf09a-0f0b-4574-9907-ebd1ddf3015f

use crate::package::Package;
use crate::token::is_true;
use mirsam_core::error::{Error, Result};
use mirsam_core::ports::DocumentReader;
use mirsam_core::text::{Alignment, Bullet, Direction, Location, Properties, Resolved, TextUnit};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::path::{Path, PathBuf};

/// Alignment values WordprocessingML understands, as *Word* reads them.
///
/// `left`/`right` are the Transitional spelling of `start`/`end`, not physical
/// edges; see the module documentation for why, and for what follows from it.
/// The kashida forms are Arabic justification and read correctly in either
/// direction, as does `numTab`, which aligns at the numbering tab — the start
/// side of whichever direction the paragraph runs.
fn parse_alignment(value: &str) -> Option<Alignment> {
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
}

impl ParagraphBuilder {
    fn finish(self, part: &str) -> TextUnit {
        let index = self.index;
        TextUnit::new(unit_id(part, index), self.text)
            .with_props(self.props)
            .with_location(Location {
                part: part.to_string(),
                paragraph: Some(index),
                // Word names no enclosing shape for body text. A table cell
                // would be one, and tables are PLAN §3.4.
                container: None,
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
}

impl PartScan {
    /// Whether events at this point describe content this adapter reads.
    fn reading(&self) -> bool {
        self.fallback == 0
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

    fn close_paragraph(&mut self, part: &str) {
        if let Some(b) = self.open.pop()
            && !b.text.trim().is_empty()
        {
            self.units.push(b.finish(part));
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
                    self.open.push(ParagraphBuilder {
                        index: self.seen,
                        ..Default::default()
                    });
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
            "w:numPr" if !in_section => {
                if let Some(b) = self.current() {
                    b.props.bullet = Bullet::Native;
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
            // `@w:cstheme` and `@w:asciiTheme` name the theme's font scheme
            // rather than a typeface, and the theme is PLAN §3.3. Recording
            // one here would put `majorBidi` in a report as though it named a
            // font.
            "w:rFonts" => {
                let complex = non_empty_attribute(e, "w:cs");
                let latin = non_empty_attribute(e, "w:ascii");
                if let Some(b) = self.current() {
                    for (face, slot) in [
                        (complex, &mut b.props.complex_font),
                        (latin, &mut b.props.latin_font),
                    ] {
                        if slot.is_unset()
                            && let Some(face) = face
                        {
                            *slot = Resolved::Explicit(face);
                        }
                    }
                }
            }
            "w:t" => self.in_text = has_content,
            _ => {}
        }
    }
}

/// A Word package opened for auditing.
pub struct DocxDocument {
    package: Package,
}

impl DocxDocument {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            package: Package::open(path)?,
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

    /// Parse one `word/**/*.xml` part into text units.
    ///
    /// Every property is `Explicit` or `Unset`: this reads what the paragraph
    /// itself states. `docDefaults` and the style chain above it are PLAN
    /// §3.3, and until they are read an unstated property is honestly absent
    /// rather than guessed at.
    fn scan_part(part: &str, xml: &str) -> Result<Vec<TextUnit>> {
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
                    "w:t" if state.reading() => state.in_text = false,
                    // Guarded, because a `w:p` inside a fallback would
                    // otherwise close the paragraph that encloses it.
                    "w:p" if state.reading() => state.close_paragraph(part),
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
        let mut units = Vec::new();
        for part in self.text_parts()? {
            let xml = self.package.read_text(&part)?;
            units.extend(Self::scan_part(&part, &xml)?);
        }
        Ok(units)
    }
}

/// Parse an in-memory part into every unit this adapter produces for it.
///
/// Exposed for tests and for callers that already hold the XML.
pub fn scan_xml(part: &str, xml: &str) -> Result<Vec<TextUnit>> {
    DocxDocument::scan_part(part, xml)
}
