//! One suite, every adapter (PLAN §3.5).
//!
//! The other test files ask whether an adapter reads its own format. This one
//! asks the question the architecture rests on: **do the adapters agree?** A
//! defect an Arabic author can produce in PowerPoint can be produced just as
//! easily in Word, and the tool is only worth trusting in either if the same
//! situation comes back as the same finding, with the same evidence, whichever
//! application wrote the file.
//!
//! So a case here states a situation **once**, in the shared model's own
//! vocabulary — "a paragraph of Arabic with no direction declared, under a
//! chain that states nothing" — and each [`Vocabulary`] lowers it into a real
//! package on disk, which the suite opens through [`DocumentReader`] and
//! nothing else. **No case that asserts what the tool reports names an
//! element, an attribute or a format**; the only assertions that name one are
//! the two refusals below, which exist precisely to say where the formats
//! differ. That is the design of the file: a case that had to know which
//! adapter it was looking at would be [ADR 0001]'s hexagon leaking, and the
//! thing to fix would be the abstraction rather than the case.
//!
//! ## A format that cannot state the situation says so
//!
//! The formats are not identical and pretending otherwise would be the second
//! way to make this file lie. Word's `w:jc` is direction-relative — its `left`
//! is the *start* edge — so a hard left edge cannot be written in Word at all;
//! DrawingML is the mirror image, with physical edges and no direction-relative
//! spelling. Neither is a gap in an adapter, and neither may be silently
//! skipped: a vocabulary that cannot state a situation returns
//! [`Inexpressible`] with the reason, the suite runs the case against the
//! formats that *can*, and [`every_refusal_is_one_the_design_intended`] holds
//! the full list of refusals against the committed one. A format that quietly
//! stopped expressing something would fail there.
//!
//! [ADR 0001]: ../../../docs/adr/0001-hexagonal-architecture.md

use mirsam_core::{
    Alignment, Bullet, Direction, DocumentReader, Engine, Inset, Resolved, Severity, SpanBidi,
    TextUnit, UnitKind,
};
use mirsam_html::HtmlDocument;
use mirsam_ooxml::{DocxDocument, Package, PptxDocument};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

/// Arabic that reads right to left by any measure.
const ARABIC: &str = "ارتفع الأداء في الربع الرابع";

/// Arabic with Latin and digits in it, which is the case where the declared
/// direction changes what a reader sees: the numbers and the `Q4` move.
const MIXED: &str = "ارتفع الأداء بنسبة 25% في Q4 2026.";

/// English, for the cases that ask what the tool does to text it should not
/// touch.
const ENGLISH: &str = "Performance rose in the fourth quarter";

// ------------------------------------------------------- the shared situation

/// One paragraph, stated in the shared model rather than in either format's
/// vocabulary.
#[derive(Clone, Default)]
struct Paragraph {
    text: &'static str,
    direction: Option<Direction>,
    alignment: Option<Alignment>,
    /// The edge the paragraph's leading inset is measured from.
    inset: Option<Inset>,
    /// A complex-script language tag.
    language: Option<&'static str>,
    latin_font: Option<&'static str>,
    complex_font: Option<&'static str>,
    /// A list marker produced by the format's own list feature.
    bullet: bool,
    /// A run within `text` that the document delimits, and what it says about
    /// how the bidirectional algorithm should treat it.
    run: Option<Run>,
}

/// One run within a paragraph, stated in the shared model's vocabulary.
///
/// The substring rather than an offset, because a case here states a situation
/// and a byte count is an implementation of one. Each format finds the run in
/// its own text and delimits it in its own way — or says it cannot.
#[derive(Clone, Copy)]
struct Run {
    text: &'static str,
    bidi: SpanBidi,
}

/// A container made to *look* right to left by displaying its boxes in the
/// reverse of the order it stores them, rather than by declaring a direction.
///
/// Stated as its own situation rather than as a flag on a table, because the
/// question it asks is whether a format can reverse a layout *without* its
/// direction — and two of the three cannot reverse a layout at all.
#[derive(Clone)]
struct Reversed {
    boxes: &'static [&'static str],
    /// What the container declares, beside the reversal.
    direction: Option<Direction>,
}

impl Paragraph {
    fn of(text: &'static str) -> Self {
        Self {
            text,
            ..Self::default()
        }
    }

    /// Arabic marked the way an author who knew what they were doing marks it.
    ///
    /// Centred, because the centre is the one alignment both formats spell and
    /// both direction rules accept: the start edge is Word's word and the right
    /// edge is PowerPoint's, and picking either would state a situation the
    /// other format has to refuse.
    fn correct_arabic() -> Self {
        Self::of(ARABIC)
            .direction(Direction::Rtl)
            .alignment(Alignment::Center)
            .language("ar-SA")
            .complex_font("Dubai")
    }

    /// The same paragraph carrying different text: the marking is correct and
    /// what is under test is the text itself.
    fn saying(mut self, text: &'static str) -> Self {
        self.text = text;
        self
    }

    fn direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = Some(alignment);
        self
    }

    fn inset(mut self, inset: Inset) -> Self {
        self.inset = Some(inset);
        self
    }

    /// Delimit `text` within the paragraph, saying what the document states
    /// about it.
    fn run(mut self, text: &'static str, bidi: SpanBidi) -> Self {
        self.run = Some(Run { text, bidi });
        self
    }

    fn language(mut self, tag: &'static str) -> Self {
        self.language = Some(tag);
        self
    }

    fn latin_font(mut self, typeface: &'static str) -> Self {
        self.latin_font = Some(typeface);
        self
    }

    fn complex_font(mut self, typeface: &'static str) -> Self {
        self.complex_font = Some(typeface);
        self
    }

    /// Empty the complex-script slot a correctly marked paragraph filled.
    fn without_complex_font(mut self) -> Self {
        self.complex_font = None;
        self
    }

    fn bullet(mut self) -> Self {
        self.bullet = true;
        self
    }
}

/// One table: the order its columns are displayed in, and the text of its
/// cells in the order the file stores them.
#[derive(Clone)]
struct Table {
    direction: Option<Direction>,
    cells: &'static [&'static str],
    /// What each cell's one paragraph states, so a table can be surrounded by
    /// correctly marked text and judged on its own.
    cell: Paragraph,
}

impl Table {
    fn of(cells: &'static [&'static str]) -> Self {
        Self {
            direction: None,
            cells,
            cell: Paragraph::default(),
        }
    }

    fn direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    /// Mark every cell's paragraph correctly, so the only thing left to report
    /// is the table's own column order.
    fn correct_cells(mut self) -> Self {
        self.cell = Paragraph::correct_arabic();
        self
    }
}

/// What the chain above the body states.
///
/// Both formats have one and they agree about nothing except that it exists —
/// PowerPoint's runs through a layout to a master, Word's through a named
/// style to the document defaults. The shared model's claim is that a value
/// either chain supplies arrives as [`Resolved::Inherited`] naming the part
/// that supplied it, and that is all a case here is allowed to know.
#[derive(Clone, Copy, Default)]
struct Chain {
    direction: Option<Direction>,
    alignment: Option<Alignment>,
}

impl Chain {
    fn stating(direction: Direction) -> Self {
        Self {
            direction: Some(direction),
            alignment: None,
        }
    }

    fn and_alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = Some(alignment);
        self
    }
}

/// A document: the text it holds, and what the chain above that text states.
#[derive(Clone, Default)]
struct Document {
    paragraphs: Vec<Paragraph>,
    tables: Vec<Table>,
    reversed: Vec<Reversed>,
    chain: Chain,
}

impl Document {
    fn of(paragraphs: impl IntoIterator<Item = Paragraph>) -> Self {
        Self {
            paragraphs: paragraphs.into_iter().collect(),
            ..Self::default()
        }
    }

    fn one(paragraph: Paragraph) -> Self {
        Self::of([paragraph])
    }

    fn with_table(mut self, table: Table) -> Self {
        self.tables.push(table);
        self
    }

    fn with_reversed(mut self, reversed: Reversed) -> Self {
        self.reversed.push(reversed);
        self
    }

    fn under(mut self, chain: Chain) -> Self {
        self.chain = chain;
        self
    }
}

// ----------------------------------------------------------- the vocabularies

/// A situation a format genuinely cannot state, and why.
///
/// Never a gap in an adapter: the reason is a property of the file format, and
/// it is carried rather than swallowed so that
/// [`every_refusal_is_one_the_design_intended`] can hold every one of them
/// against the list this project decided on.
struct Inexpressible(&'static str);

/// The three situations neither OOXML format can state, checked by both.
///
/// Written once because the reason is the same one twice: PresentationML and
/// WordprocessingML both build a paragraph out of runs and both give a run a
/// direction, and neither has anything to say about isolating one run from its
/// neighbours, imposing an order on one, insetting a box from a physical edge,
/// or displaying a container's boxes in any order but the one it stores them
/// in. Every one of those is a property of the file formats rather than a gap
/// in an adapter, which is what makes it a refusal rather than a skip.
mod ooxml {
    use super::{Document, Inexpressible, Paragraph};

    pub fn run(p: &Paragraph) -> Result<(), Inexpressible> {
        if p.run.is_some() {
            return Err(Inexpressible(
                "OOXML builds a paragraph out of runs but says nothing about how the \
                 bidirectional algorithm should treat one: there is no isolate, and no \
                 override. A run is text with properties, not a bidi boundary",
            ));
        }
        Ok(())
    }

    pub fn inset(p: &Paragraph) -> Result<(), Inexpressible> {
        if p.inset.is_some() {
            return Err(Inexpressible(
                "OOXML's paragraph indents are direction-relative — a:pPr/@marL and \
                 w:ind/@start are the start edge whatever they are called — so an inset \
                 measured from a physical edge cannot be written",
            ));
        }
        Ok(())
    }

    pub fn reversed(doc: &Document) -> Result<(), Inexpressible> {
        if !doc.reversed.is_empty() {
            return Err(Inexpressible(
                "neither format can display a container's boxes in any order but the one it \
                 stores them in: a table or a multi-column body states a direction, and the \
                 direction is the only thing that decides which end a reader starts from",
            ));
        }
        Ok(())
    }
}

/// A document of some format, written to disk.
struct Written {
    path: PathBuf,
    /// The parts it holds, spelled as a unit's `location.part` spells them.
    /// A format that is one file holds one part: the file.
    parts: Vec<String>,
}

/// One document format, as this suite needs to use it: something that can
/// write a document stating a [`Document`], and open it again as a
/// [`DocumentReader`].
///
/// Note that [`write`] and not "the parts of a package" is the operation.
/// Two of the three formats here are ZIP packages and one is a single file,
/// and a suite whose vocabulary trait assumed the first would be a suite that
/// could only ever hold OOXML adapters to the contract.
///
/// [`write`]: Vocabulary::write
trait Vocabulary: Sync {
    /// The name the adapter reports for itself.
    fn format(&self) -> &'static str;

    /// The file extension a document of this format carries.
    fn extension(&self) -> &'static str;

    /// Write a document stating `doc` into `dir`, or refuse with the reason
    /// this format cannot state it.
    fn write(&self, dir: &Path, doc: &Document) -> Result<Written, Inexpressible>;

    fn open(&self, path: &Path) -> mirsam_core::Result<Box<dyn DocumentReader>>;

    /// The parts an existing document on disk holds, for the corpus check.
    fn parts_of(&self, path: &Path) -> Vec<String>;
}

/// Every adapter this build has. A case runs against all of them; adding a
/// format here is how a new adapter is held to the same contract.
fn vocabularies() -> Vec<Box<dyn Vocabulary>> {
    vec![Box::new(Pptx), Box::new(Docx), Box::new(Html)]
}

// ------------------------------------------------------------ PresentationML

struct Pptx;

const DRAWINGML: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

impl Pptx {
    /// DrawingML's spelling of an alignment, or the reason it has none.
    ///
    /// `start` and `end` are exactly the two it lacks: `a:pPr/@algn` names
    /// physical edges, so a direction-relative alignment cannot be written.
    fn algn(alignment: Alignment) -> Result<&'static str, Inexpressible> {
        Ok(match alignment {
            Alignment::Left => "l",
            Alignment::Right => "r",
            Alignment::Center => "ctr",
            Alignment::Justify => "just",
            Alignment::Distributed => "dist",
            Alignment::Start | Alignment::End => {
                return Err(Inexpressible(
                    "DrawingML's a:pPr/@algn names physical edges and has no \
                     direction-relative spelling",
                ));
            }
        })
    }

    fn paragraph(p: &Paragraph) -> Result<String, Inexpressible> {
        ooxml::run(p)?;
        ooxml::inset(p)?;
        let mut properties = String::new();
        if let Some(direction) = p.direction {
            properties.push_str(&format!(
                r#" rtl="{}""#,
                u8::from(direction == Direction::Rtl)
            ));
        }
        if let Some(alignment) = p.alignment {
            properties.push_str(&format!(r#" algn="{}""#, Self::algn(alignment)?));
        }
        let bullet = if p.bullet {
            r#"<a:buChar char="•"/>"#
        } else {
            ""
        };

        let mut run = String::new();
        if let Some(tag) = p.language {
            run.push_str(&format!(r#" lang="{tag}""#));
        }
        let mut fonts = String::new();
        if let Some(typeface) = p.latin_font {
            fonts.push_str(&format!(r#"<a:latin typeface="{typeface}"/>"#));
        }
        if let Some(typeface) = p.complex_font {
            fonts.push_str(&format!(r#"<a:cs typeface="{typeface}"/>"#));
        }

        Ok(format!(
            "<a:p><a:pPr{properties}>{bullet}</a:pPr>\
             <a:r><a:rPr{run}>{fonts}</a:rPr><a:t>{}</a:t></a:r></a:p>",
            escape(p.text)
        ))
    }

    fn shape(name: &str, body: &str) -> String {
        format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="{name}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{body}</p:txBody></p:sp>"#
        )
    }

    fn table(t: &Table) -> Result<String, Inexpressible> {
        let mut properties = String::new();
        if let Some(direction) = t.direction {
            properties.push_str(&format!(
                r#" rtl="{}""#,
                u8::from(direction == Direction::Rtl)
            ));
        }
        let mut cells = String::new();
        for text in t.cells {
            let paragraph = Paragraph {
                text,
                ..t.cell.clone()
            };
            cells.push_str(&format!(
                r#"<a:tc><a:txBody><a:bodyPr/><a:lstStyle/>{}</a:txBody><a:tcPr/></a:tc>"#,
                Self::paragraph(&paragraph)?
            ));
        }
        Ok(format!(
            r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="3" name="Table 3"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr{properties}/><a:tblGrid/><a:tr h="0">{cells}</a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#
        ))
    }

    /// The master's text styles, which is where PowerPoint's chain ends.
    ///
    /// `p:otherStyle` because the shapes above hold no `p:ph`: a shape that is
    /// not a placeholder is answered by the master's other style, which is the
    /// one hop this suite needs. Which style a placeholder reaches is
    /// `inherit.rs`'s question, not this file's.
    fn text_styles(chain: &Chain) -> Result<String, Inexpressible> {
        let mut properties = String::new();
        if let Some(direction) = chain.direction {
            properties.push_str(&format!(
                r#" rtl="{}""#,
                u8::from(direction == Direction::Rtl)
            ));
        }
        if let Some(alignment) = chain.alignment {
            properties.push_str(&format!(r#" algn="{}""#, Self::algn(alignment)?));
        }
        Ok(format!(
            r#"<p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle><a:lvl1pPr{properties}/></p:otherStyle></p:txStyles>"#
        ))
    }
}

impl Packaged for Pptx {
    fn parts(&self, doc: &Document) -> Result<Vec<(String, String)>, Inexpressible> {
        ooxml::reversed(doc)?;
        let mut shapes = String::new();
        if !doc.paragraphs.is_empty() {
            let mut body = String::new();
            for paragraph in &doc.paragraphs {
                body.push_str(&Self::paragraph(paragraph)?);
            }
            shapes.push_str(&Self::shape("Body 2", &body));
        }
        for table in &doc.tables {
            shapes.push_str(&Self::table(table)?);
        }

        let slide = format!(
            r#"<p:sld {DRAWINGML}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{shapes}</p:spTree></p:cSld><p:clrMapOvr/></p:sld>"#
        );
        let layout = format!(
            r#"<p:sldLayout {DRAWINGML}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld></p:sldLayout>"#
        );
        let master = format!(
            r#"<p:sldMaster {DRAWINGML}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" hlink="hlink" folHlink="folHlink"/>{}</p:sldMaster>"#,
            Self::text_styles(&doc.chain)?
        );

        Ok(vec![
            (
                "[Content_Types].xml".into(),
                content_types(&[
                    (
                        "/ppt/presentation.xml",
                        "presentationml.presentation.main+xml",
                    ),
                    ("/ppt/slides/slide1.xml", "presentationml.slide+xml"),
                    (
                        "/ppt/slideLayouts/slideLayout1.xml",
                        "presentationml.slideLayout+xml",
                    ),
                    (
                        "/ppt/slideMasters/slideMaster1.xml",
                        "presentationml.slideMaster+xml",
                    ),
                ]),
            ),
            (
                "_rels/.rels".into(),
                relationships(&[("officeDocument", "ppt/presentation.xml")]),
            ),
            (
                "ppt/presentation.xml".into(),
                format!(
                    r#"<p:presentation {DRAWINGML}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId2"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>"#
                ),
            ),
            (
                "ppt/_rels/presentation.xml.rels".into(),
                relationships(&[
                    ("slide", "slides/slide1.xml"),
                    ("slideMaster", "slideMasters/slideMaster1.xml"),
                ]),
            ),
            ("ppt/slides/slide1.xml".into(), slide),
            (
                "ppt/slides/_rels/slide1.xml.rels".into(),
                relationships(&[("slideLayout", "../slideLayouts/slideLayout1.xml")]),
            ),
            ("ppt/slideLayouts/slideLayout1.xml".into(), layout),
            (
                "ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(),
                relationships(&[("slideMaster", "../slideMasters/slideMaster1.xml")]),
            ),
            ("ppt/slideMasters/slideMaster1.xml".into(), master),
        ])
    }
}

impl Vocabulary for Pptx {
    fn format(&self) -> &'static str {
        "pptx"
    }

    fn extension(&self) -> &'static str {
        "pptx"
    }

    fn write(&self, dir: &Path, doc: &Document) -> Result<Written, Inexpressible> {
        Ok(zip_package(dir, self.extension(), &self.parts(doc)?))
    }

    fn parts_of(&self, path: &Path) -> Vec<String> {
        package_parts(path)
    }

    fn open(&self, path: &Path) -> mirsam_core::Result<Box<dyn DocumentReader>> {
        Ok(Box::new(PptxDocument::open(path)?))
    }
}

// --------------------------------------------------------- WordprocessingML

struct Docx;

const WORDPROCESSINGML: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

impl Docx {
    /// Word's spelling of an alignment, or the reason it has none.
    ///
    /// `left` and `right` are exactly the two it lacks. Word writes those two
    /// words, but they are the Transitional spelling of `start` and `end` —
    /// ECMA-376 Part 1 §17.18.44 says the value is "interpreted [...] based on
    /// the value of the bidi element" — so writing `w:jc w:val="left"` states
    /// the start edge, not the left one.
    fn jc(alignment: Alignment) -> Result<&'static str, Inexpressible> {
        Ok(match alignment {
            Alignment::Start => "start",
            Alignment::End => "end",
            Alignment::Center => "center",
            Alignment::Justify => "both",
            Alignment::Distributed => "distribute",
            Alignment::Left | Alignment::Right => {
                return Err(Inexpressible(
                    "Word's w:jc is direction-relative whatever it is spelled, \
                     so a physical edge cannot be stated",
                ));
            }
        })
    }

    /// The `w:pPr` and `w:rPr` bodies stating one paragraph's properties.
    fn formatting(p: &Paragraph) -> Result<(String, String), Inexpressible> {
        ooxml::run(p)?;
        ooxml::inset(p)?;
        let mut paragraph = String::new();
        if let Some(direction) = p.direction {
            paragraph.push_str(&format!(
                r#"<w:bidi w:val="{}"/>"#,
                u8::from(direction == Direction::Rtl)
            ));
        }
        if p.bullet {
            paragraph.push_str(r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>"#);
        }
        if let Some(alignment) = p.alignment {
            paragraph.push_str(&format!(r#"<w:jc w:val="{}"/>"#, Self::jc(alignment)?));
        }

        let mut run = String::new();
        let mut fonts = String::new();
        if let Some(typeface) = p.latin_font {
            fonts.push_str(&format!(r#" w:ascii="{typeface}""#));
        }
        if let Some(typeface) = p.complex_font {
            fonts.push_str(&format!(r#" w:cs="{typeface}""#));
        }
        if !fonts.is_empty() {
            run.push_str(&format!("<w:rFonts{fonts}/>"));
        }
        if let Some(tag) = p.language {
            run.push_str(&format!(r#"<w:lang w:bidi="{tag}"/>"#));
        }

        Ok((paragraph, run))
    }

    fn paragraph(p: &Paragraph) -> Result<String, Inexpressible> {
        let (paragraph, run) = Self::formatting(p)?;
        Ok(format!(
            "<w:p><w:pPr>{paragraph}</w:pPr><w:r><w:rPr>{run}</w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
            escape(p.text)
        ))
    }

    fn table(t: &Table) -> Result<String, Inexpressible> {
        let mut properties = String::new();
        if let Some(direction) = t.direction {
            properties.push_str(&format!(
                r#"<w:bidiVisual w:val="{}"/>"#,
                u8::from(direction == Direction::Rtl)
            ));
        }
        let mut cells = String::new();
        for text in t.cells {
            let paragraph = Paragraph {
                text,
                ..t.cell.clone()
            };
            cells.push_str(&format!(
                "<w:tc><w:tcPr/>{}</w:tc>",
                Self::paragraph(&paragraph)?
            ));
        }
        Ok(format!(
            "<w:tbl><w:tblPr>{properties}</w:tblPr><w:tblGrid/><w:tr>{cells}</w:tr></w:tbl>"
        ))
    }

    /// `w:docDefaults`, which is where Word's chain ends.
    fn styles(chain: &Chain) -> Result<String, Inexpressible> {
        let mut properties = String::new();
        if let Some(direction) = chain.direction {
            properties.push_str(&format!(
                r#"<w:bidi w:val="{}"/>"#,
                u8::from(direction == Direction::Rtl)
            ));
        }
        if let Some(alignment) = chain.alignment {
            properties.push_str(&format!(r#"<w:jc w:val="{}"/>"#, Self::jc(alignment)?));
        }
        Ok(format!(
            r#"<w:styles {WORDPROCESSINGML}><w:docDefaults><w:pPrDefault><w:pPr>{properties}</w:pPr></w:pPrDefault></w:docDefaults></w:styles>"#
        ))
    }
}

impl Packaged for Docx {
    fn parts(&self, doc: &Document) -> Result<Vec<(String, String)>, Inexpressible> {
        ooxml::reversed(doc)?;
        let mut body = String::new();
        for paragraph in &doc.paragraphs {
            body.push_str(&Self::paragraph(paragraph)?);
        }
        for table in &doc.tables {
            body.push_str(&Self::table(table)?);
        }

        Ok(vec![
            (
                "[Content_Types].xml".into(),
                content_types(&[
                    ("/word/document.xml", "wordprocessingml.document.main+xml"),
                    ("/word/styles.xml", "wordprocessingml.styles+xml"),
                ]),
            ),
            (
                "_rels/.rels".into(),
                relationships(&[("officeDocument", "word/document.xml")]),
            ),
            (
                "word/document.xml".into(),
                format!("<w:document {WORDPROCESSINGML}><w:body>{body}</w:body></w:document>"),
            ),
            (
                "word/_rels/document.xml.rels".into(),
                relationships(&[("styles", "styles.xml")]),
            ),
            ("word/styles.xml".into(), Self::styles(&doc.chain)?),
        ])
    }
}

impl Vocabulary for Docx {
    fn format(&self) -> &'static str {
        "docx"
    }

    fn extension(&self) -> &'static str {
        "docx"
    }

    fn write(&self, dir: &Path, doc: &Document) -> Result<Written, Inexpressible> {
        Ok(zip_package(dir, self.extension(), &self.parts(doc)?))
    }

    fn parts_of(&self, path: &Path) -> Vec<String> {
        package_parts(path)
    }

    fn open(&self, path: &Path) -> mirsam_core::Result<Box<dyn DocumentReader>> {
        Ok(Box::new(DocxDocument::open(path)?))
    }
}

// ------------------------------------------------------------------- HTML

struct Html;

impl Html {
    /// The `style` declarations a paragraph's marking needs, if any.
    ///
    /// CSS states a *physical* left and right as well as a direction-relative
    /// start and end, so HTML is the one format here that refuses neither
    /// alignment. What it has no spelling for is `distributed`: CSS's
    /// `text-align` has no value that stretches every line to the full
    /// measure, the nearest thing being a `text-justify` that is not the same
    /// property and is not the same instruction.
    fn text_align(alignment: Alignment) -> Result<&'static str, Inexpressible> {
        Ok(match alignment {
            Alignment::Left => "left",
            Alignment::Right => "right",
            Alignment::Center => "center",
            Alignment::Justify => "justify",
            Alignment::Start => "start",
            Alignment::End => "end",
            Alignment::Distributed => {
                return Err(Inexpressible(
                    "CSS text-align has no distributed value; the nearest is \
                     text-justify, which is a different property",
                ));
            }
        })
    }

    /// The one font stack CSS gives an element, or the reason a pair of slots
    /// cannot be stated.
    ///
    /// OOXML gives a run a Latin slot and a complex-script slot. CSS gives it
    /// one `font-family`, which answers for every script on the element, so a
    /// document that fills one slot and leaves the other empty — or fills them
    /// with different typefaces — is a document the web cannot write.
    fn font_family(p: &Paragraph) -> Result<Option<&'static str>, Inexpressible> {
        match (p.latin_font, p.complex_font) {
            (None, None) => Ok(None),
            // The complex slot alone: one stack answers for the Arabic, which
            // is the whole of what that slot was for.
            (None, Some(family)) => Ok(Some(family)),
            (Some(latin), Some(complex)) if latin == complex => Ok(Some(latin)),
            _ => Err(Inexpressible(
                "CSS has one font-family per element, which answers for every \
                 script; it cannot fill a Latin slot and leave the complex-script \
                 one empty, nor state two different typefaces",
            )),
        }
    }

    /// A block element carrying one paragraph's text and marking.
    fn block(tag: &str, p: &Paragraph) -> Result<String, Inexpressible> {
        let mut attributes = String::new();
        if let Some(direction) = p.direction {
            attributes.push_str(&format!(
                r#" dir="{}""#,
                if direction == Direction::Rtl {
                    "rtl"
                } else {
                    "ltr"
                }
            ));
        }
        if let Some(tag) = p.language {
            attributes.push_str(&format!(r#" lang="{tag}""#));
        }

        let mut style = String::new();
        if let Some(alignment) = p.alignment {
            style.push_str(&format!("text-align:{};", Self::text_align(alignment)?));
        }
        if let Some(family) = Self::font_family(p)? {
            style.push_str(&format!("font-family:{family};"));
        }
        if let Some(inset) = p.inset {
            // The property CSS spells the edge with, which is the whole of what
            // this situation is about: the physical pair does not follow the
            // text and the logical pair does.
            style.push_str(&format!(
                "{}:2rem;",
                match inset {
                    Inset::Left => "margin-left",
                    Inset::Right => "margin-right",
                    Inset::Start => "margin-inline-start",
                    Inset::End => "margin-inline-end",
                }
            ));
        }
        if !style.is_empty() {
            attributes.push_str(&format!(r#" style="{style}""#));
        }

        Ok(format!("<{tag}{attributes}>{}</{tag}>", Self::content(p)))
    }

    /// A paragraph's text, with the run it delimits wrapped in the element that
    /// states what the document says about it.
    ///
    /// `<span>` says nothing, which is the un-isolated case; `<bdi>` isolates;
    /// `<bdo>` imposes an order. All three are markup a browser reads out of
    /// its own stylesheet, so none of them needs CSS here.
    fn content(p: &Paragraph) -> String {
        let Some(run) = p.run else {
            return escape(p.text);
        };
        let Some(at) = p.text.find(run.text) else {
            panic!("the run {:?} is not in {:?}", run.text, p.text);
        };
        let (open, close) = match run.bidi {
            SpanBidi::Plain => ("<span>".to_string(), "</span>"),
            SpanBidi::Isolated => ("<bdi>".to_string(), "</bdi>"),
            SpanBidi::Imposed(direction) => (
                format!(
                    r#"<bdo dir="{}">"#,
                    if direction == Direction::Rtl {
                        "rtl"
                    } else {
                        "ltr"
                    }
                ),
                "</bdo>",
            ),
        };
        format!(
            "{}{open}{}{close}{}",
            escape(&p.text[..at]),
            escape(run.text),
            escape(&p.text[at + run.text.len()..])
        )
    }

    /// A container whose boxes are displayed backwards without its direction
    /// being what says so.
    ///
    /// A flex row reversed by `flex-direction`, which is the web's way of making
    /// a layout look right to left while every other order in the document —
    /// reading, selection, fallback — stays as it was.
    fn reversed(r: &Reversed) -> String {
        let mut style = "display:flex;flex-direction:row-reverse;".to_string();
        if let Some(direction) = r.direction {
            style.push_str(&format!(
                "direction:{};",
                if direction == Direction::Rtl {
                    "rtl"
                } else {
                    "ltr"
                }
            ));
        }
        let boxes: String = r
            .boxes
            .iter()
            .map(|text| format!("<div>{}</div>", escape(text)))
            .collect();
        format!(r#"<div style="{style}">{boxes}</div>"#)
    }

    fn paragraph(p: &Paragraph) -> Result<String, Inexpressible> {
        // A list marker the format produces itself is `<li>` inside a list,
        // which is the web's only native one.
        if p.bullet {
            return Ok(format!("<ul>{}</ul>", Self::block("li", p)?));
        }
        Self::block("p", p)
    }

    fn table(t: &Table) -> Result<String, Inexpressible> {
        let mut attributes = String::new();
        if let Some(direction) = t.direction {
            attributes.push_str(&format!(
                r#" dir="{}""#,
                if direction == Direction::Rtl {
                    "rtl"
                } else {
                    "ltr"
                }
            ));
        }
        let mut cells = String::new();
        for text in t.cells {
            let paragraph = Paragraph {
                text,
                ..t.cell.clone()
            };
            // The cell *is* the block that holds the text, so the paragraph's
            // marking goes on the `<td>` rather than on something inside it.
            cells.push_str(&Self::block("td", &paragraph)?);
        }
        Ok(format!("<table{attributes}><tr>{cells}</tr></table>"))
    }

    /// What the chain above the text states: in HTML, an ancestor element.
    ///
    /// The web's cascade has no separate part to put it in — no master, no
    /// `styles.xml` — so the ancestor is where a value the paragraph does not
    /// state comes from, and `<body>` is the ancestor every paragraph has.
    fn body_attributes(chain: &Chain) -> Result<String, Inexpressible> {
        let mut attributes = String::new();
        if let Some(direction) = chain.direction {
            attributes.push_str(&format!(
                r#" dir="{}""#,
                if direction == Direction::Rtl {
                    "rtl"
                } else {
                    "ltr"
                }
            ));
        }
        if let Some(alignment) = chain.alignment {
            attributes.push_str(&format!(
                r#" style="text-align:{};""#,
                Self::text_align(alignment)?
            ));
        }
        Ok(attributes)
    }
}

impl Vocabulary for Html {
    fn format(&self) -> &'static str {
        "html"
    }

    fn extension(&self) -> &'static str {
        "html"
    }

    fn write(&self, dir: &Path, doc: &Document) -> Result<Written, Inexpressible> {
        let mut body = String::new();
        for paragraph in &doc.paragraphs {
            body.push_str(&Self::paragraph(paragraph)?);
        }
        for table in &doc.tables {
            body.push_str(&Self::table(table)?);
        }
        for reversed in &doc.reversed {
            body.push_str(&Self::reversed(reversed));
        }

        // No `<title>`: it is text a reader sees, so the adapter reports it as
        // a unit, and a suite that added one to every document would be
        // stating a paragraph no case asked for.
        let html = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"></head>\
             <body{}>{body}</body></html>",
            Self::body_attributes(&doc.chain)?
        );

        let path = dir.join(format!("document.{}", self.extension()));
        fs::write(&path, html).expect("writing the document");
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the file has a name")
            .to_string();
        Ok(Written {
            path,
            parts: vec![name],
        })
    }

    fn parts_of(&self, path: &Path) -> Vec<String> {
        // One file, one part, named as a unit's location names it.
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| vec![name.to_string()])
            .unwrap_or_default()
    }

    fn open(&self, path: &Path) -> mirsam_core::Result<Box<dyn DocumentReader>> {
        Ok(Box::new(HtmlDocument::open(path)?))
    }
}

// ------------------------------------------------------------ package writing

/// A format that is a ZIP of XML parts — which both OOXML formats are, and
/// HTML is not.
///
/// Kept separate from [`Vocabulary`] so that the suite's idea of "a document"
/// is a file on disk rather than a package, and a single-file format needs no
/// exception to join it.
trait Packaged {
    /// The parts of a package stating `doc`, or the reason this format cannot
    /// state it.
    fn parts(&self, doc: &Document) -> Result<Vec<(String, String)>, Inexpressible>;
}

/// Write parts into a ZIP named for the format.
fn zip_package(dir: &Path, extension: &str, parts: &[(String, String)]) -> Written {
    let path = dir.join(format!("document.{extension}"));
    let mut zip = ZipWriter::new(File::create(&path).expect("creating the package"));
    let options = SimpleFileOptions::default();
    for (name, body) in parts {
        zip.start_file(name.as_str(), options).expect("a part");
        zip.write_all(body.as_bytes()).expect("writing a part");
    }
    zip.finish().expect("finishing the package");
    Written {
        path,
        parts: parts.iter().map(|(name, _)| name.clone()).collect(),
    }
}

/// The parts an OOXML package on disk holds.
fn package_parts(path: &Path) -> Vec<String> {
    Package::open(path)
        .and_then(|package| package.part_names())
        .unwrap_or_default()
}

/// The relationship namespace every type below is a member of.
const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A `[Content_Types].xml` declaring the relationship default every package
/// needs, plus one override per part.
fn content_types(overrides: &[(&str, &str)]) -> String {
    let mut xml = String::from(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#,
    );
    for (part, kind) in overrides {
        xml.push_str(&format!(
            r#"<Override PartName="{part}" ContentType="application/vnd.openxmlformats-officedocument.{kind}"/>"#
        ));
    }
    xml.push_str("</Types>");
    xml
}

/// One `_rels/*.rels` item: the relationship kinds and their targets, numbered
/// `rId1` upward in the order given.
fn relationships(items: &[(&str, &str)]) -> String {
    let mut xml = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (index, (kind, target)) in items.iter().enumerate() {
        xml.push_str(&format!(
            r#"<Relationship Id="rId{}" Type="{OFFICE}/{kind}" Target="{target}"/>"#,
            index + 1
        ));
    }
    xml.push_str("</Relationships>");
    xml
}

/// A scratch directory that removes itself.
///
/// Numbered per use rather than per case: the cases run on parallel threads,
/// and a shared directory would have one of them removing what another is
/// still reading.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static SERIAL: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mirsam-conformance-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ------------------------------------------------------------ running a case

/// One format's reading of one situation: the units it produced, and enough of
/// the package to check that what they name is really there.
struct Reading {
    format: &'static str,
    units: Vec<TextUnit>,
    parts: Vec<String>,
    /// Kept alive: the package is on disk beneath it.
    _scratch: Scratch,
}

impl Reading {
    /// Every rule the default engine reports on these units, by id, sorted and
    /// deduplicated — the shape a case compares across formats.
    fn rules(&self) -> Vec<&'static str> {
        let mut found: Vec<&'static str> = Engine::with_default_rules()
            .audit(&self.units)
            .diagnostics
            .iter()
            .map(|d| d.rule.0)
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// The rules reported on units of one kind, by id.
    fn rules_on(&self, kind: UnitKind) -> Vec<&'static str> {
        let ids: BTreeSet<&str> = self
            .units
            .iter()
            .filter(|u| u.kind == kind)
            .map(|u| u.id.0.as_str())
            .collect();
        let mut found: Vec<&'static str> = Engine::with_default_rules()
            .audit(&self.units)
            .diagnostics
            .iter()
            .filter(|d| ids.contains(d.unit.0.as_str()))
            .map(|d| d.rule.0)
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    fn of_kind(&self, kind: UnitKind) -> Vec<&TextUnit> {
        self.units.iter().filter(|u| u.kind == kind).collect()
    }

    fn only(&self, kind: UnitKind) -> &TextUnit {
        let found = self.of_kind(kind);
        assert_eq!(
            found.len(),
            1,
            "{}: expected exactly one {kind:?} unit, got {:#?}",
            self.format,
            self.units
        );
        found[0]
    }
}

/// Read one situation with every format that can state it.
///
/// A format that cannot is left out and its reason recorded by
/// [`refusals`]; a situation *no* format can state is a broken case rather
/// than a passing one, and fails here.
fn read(doc: &Document) -> Vec<Reading> {
    let mut readings = Vec::new();
    for vocabulary in vocabularies() {
        let scratch = Scratch::new();
        let Ok(written) = vocabulary.write(&scratch.0, doc) else {
            continue;
        };

        let mut document = vocabulary
            .open(&written.path)
            .unwrap_or_else(|e| panic!("{}: the document did not open: {e}", vocabulary.format()));
        assert_eq!(
            document.format(),
            vocabulary.format(),
            "the adapter and this suite disagree about what the format is called"
        );
        let units = document
            .scan()
            .unwrap_or_else(|e| panic!("{}: the scan failed: {e}", vocabulary.format()));

        readings.push(Reading {
            format: vocabulary.format(),
            units,
            parts: written.parts,
            _scratch: scratch,
        });
    }

    assert!(
        !readings.is_empty(),
        "no format in this build can state the situation; the case proves nothing"
    );
    readings
}

/// Read a situation every format must be able to state, and assert they all
/// did.
///
/// The default for a case: a situation one adapter quietly stopped expressing
/// would otherwise pass as agreement between the formats that remained.
fn read_all(doc: &Document) -> Vec<Reading> {
    let readings = read(doc);
    assert_eq!(
        readings.len(),
        vocabularies().len(),
        "a format refused a situation every format is expected to state: {:?} answered",
        readings.iter().map(|r| r.format).collect::<Vec<_>>()
    );
    readings
}

/// Which formats cannot state `doc`, and why.
fn refusals(doc: &Document) -> Vec<(&'static str, &'static str)> {
    vocabularies()
        .iter()
        .filter_map(|v| {
            let scratch = Scratch::new();
            match v.write(&scratch.0, doc) {
                Err(Inexpressible(reason)) => Some((v.format(), reason)),
                Ok(_) => None,
            }
        })
        .collect()
}

/// Assert that every format reports the same rules for the same situation.
///
/// The claim the whole file exists to make, and the one that fails first when
/// an adapter lowers its format onto a shape the rules read differently.
fn agree_on(doc: &Document) -> Vec<&'static str> {
    let readings = read_all(doc);
    let expected = readings[0].rules();
    for reading in &readings[1..] {
        assert_eq!(
            reading.rules(),
            expected,
            "{} and {} disagree about the same situation:\n{} reports {:?}\n{} reports {:?}\n\n{:#?}",
            readings[0].format,
            reading.format,
            readings[0].format,
            expected,
            reading.format,
            reading.rules(),
            reading.units,
        );
    }
    expected
}

// ------------------------------------------------------------- the port's own

#[test]
fn every_adapter_names_itself_and_keeps_the_name_across_a_scan() {
    for reading in read_all(&Document::one(Paragraph::of(ARABIC))) {
        assert!(
            !reading.format.is_empty()
                && reading.format == reading.format.to_ascii_lowercase()
                && reading.format.chars().all(|c| c.is_ascii_alphanumeric()),
            "{:?} is not a stable, human-facing format name",
            reading.format
        );
    }
}

#[test]
fn every_unit_is_addressed_by_a_part_the_package_actually_holds() {
    // A location naming a part that is not there is a diagnostic a reviewer
    // cannot check, which invariant 6 forbids.
    let doc = Document::of([Paragraph::of(ARABIC), Paragraph::of(ENGLISH)])
        .with_table(Table::of(&["المؤشر", "الربع"]));
    for reading in read_all(&doc) {
        for unit in &reading.units {
            assert!(
                reading.parts.contains(&unit.location.part),
                "{}: {} names {}, which the package does not hold: {:?}",
                reading.format,
                unit.id,
                unit.location.part,
                reading.parts,
            );
            assert!(
                unit.id.0.starts_with(&unit.location.part),
                "{}: the id {} does not name the part {} it came from",
                reading.format,
                unit.id,
                unit.location.part,
            );
        }
    }
}

#[test]
fn unit_ids_are_unique_within_a_document_and_stable_across_two_scans() {
    // The id is the address a repair is written back to. Two units sharing one
    // would have a repair land on whichever the adapter found first.
    let doc = Document::of([Paragraph::of(ARABIC), Paragraph::of(ARABIC)])
        .with_table(Table::of(&["المؤشر", "الربع"]))
        .with_table(Table::of(&["الأول", "الثاني"]));

    let first: Vec<Vec<String>> = read_all(&doc)
        .iter()
        .map(|r| r.units.iter().map(|u| u.id.0.clone()).collect())
        .collect();
    let second: Vec<Vec<String>> = read_all(&doc)
        .iter()
        .map(|r| r.units.iter().map(|u| u.id.0.clone()).collect())
        .collect();

    for ids in &first {
        let unique: BTreeSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "an id is issued twice: {ids:?}");
    }
    assert_eq!(
        first, second,
        "the same document scanned twice moved its ids"
    );
}

#[test]
fn a_paragraph_carries_an_ordinal_and_a_container_carries_none() {
    // The ordinal is what a report shows a human, so it counts paragraphs from
    // one. A container is not at a paragraph position, and claiming one would
    // send a reviewer to a paragraph that is not the thing reported on.
    let doc = Document::of([Paragraph::of(ARABIC), Paragraph::of(ARABIC)])
        .with_table(Table::of(&["المؤشر"]));
    for reading in read_all(&doc) {
        for unit in &reading.units {
            match unit.kind {
                UnitKind::Paragraph => assert!(
                    unit.location.paragraph.is_some_and(|n| n >= 1),
                    "{}: {} is a paragraph with no ordinal",
                    reading.format,
                    unit.id
                ),
                _ => assert_eq!(
                    unit.location.paragraph, None,
                    "{}: the container {} claims a paragraph position",
                    reading.format, unit.id
                ),
            }
        }
    }
}

#[test]
fn the_text_that_comes_back_is_the_text_that_went_in_in_logical_order() {
    // Invariant 5: storage is logical-order Unicode. A reader that reversed a
    // string or shaped it would produce units the rules then judge against
    // text no author ever wrote.
    for reading in read_all(&Document::one(Paragraph::of(ARABIC))) {
        let unit = reading.only(UnitKind::Paragraph);
        assert_eq!(unit.text, ARABIC, "{}", reading.format);
        assert!(
            !unit
                .text
                .chars()
                .any(mirsam_core::script::is_presentation_form),
            "{}: the reader introduced presentation forms",
            reading.format
        );
        assert!(
            mirsam_core::controls::scan(&unit.text).is_empty(),
            "{}: the reader introduced bidi controls",
            reading.format
        );
    }
}

// --------------------------------------------------------- the shared model

#[test]
fn a_direction_the_paragraph_states_is_explicit_in_every_format() {
    for direction in [Direction::Rtl, Direction::Ltr] {
        for reading in read_all(&Document::one(Paragraph::of(ARABIC).direction(direction))) {
            assert_eq!(
                reading.only(UnitKind::Paragraph).props.direction,
                Resolved::Explicit(direction),
                "{}",
                reading.format
            );
        }
    }
}

#[test]
fn a_direction_nothing_states_is_unset_in_every_format() {
    // `Unset` and `Inherited(Ltr)` are different facts and ADR 0007 judges them
    // differently, so an adapter that filled in its renderer's default here
    // would make the tool quieter than the document deserves.
    for reading in read_all(&Document::one(Paragraph::of(ARABIC))) {
        assert!(
            reading.only(UnitKind::Paragraph).props.direction.is_unset(),
            "{}: {:#?}",
            reading.format,
            reading.only(UnitKind::Paragraph).props,
        );
    }
}

#[test]
fn a_direction_the_chain_supplies_is_inherited_and_names_a_part_that_is_there() {
    // Each format's chain is its own — a master over a layout, a style over the
    // document defaults — and neither is named here. What both must produce is
    // `Inherited` carrying an origin a reviewer can open.
    let doc = Document::one(Paragraph::of(ARABIC)).under(Chain::stating(Direction::Rtl));
    for reading in read_all(&doc) {
        let unit = reading.only(UnitKind::Paragraph);
        assert_eq!(
            unit.props.direction.effective(),
            Some(&Direction::Rtl),
            "{}: {:#?}",
            reading.format,
            unit.props,
        );
        let origin = unit
            .props
            .direction
            .origin()
            .unwrap_or_else(|| panic!("{}: an inherited value with no origin", reading.format));
        assert!(
            reading.parts.contains(&origin.part),
            "{}: the origin names {}, which the package does not hold",
            reading.format,
            origin.part,
        );
        assert!(
            !origin.property.is_empty(),
            "{}: the origin names a part but no property in it",
            reading.format
        );
    }
}

#[test]
fn a_paragraph_that_states_its_own_direction_takes_nothing_from_the_chain() {
    let doc = Document::one(Paragraph::of(ARABIC).direction(Direction::Rtl))
        .under(Chain::stating(Direction::Ltr));
    for reading in read_all(&doc) {
        assert_eq!(
            reading.only(UnitKind::Paragraph).props.direction,
            Resolved::Explicit(Direction::Rtl),
            "{}",
            reading.format
        );
    }
}

#[test]
fn a_table_is_a_container_beside_the_paragraphs_in_its_cells() {
    // The shape both formats lower a table onto: one container whose text is
    // what it lays out, and the cells' paragraphs still units of their own.
    // Neither format makes a cell's text inherit the table's column order, so
    // reporting them together would blame the wrong thing.
    let doc = Document::default().with_table(Table::of(&["المؤشر", "الربع"]));
    for reading in read_all(&doc) {
        let paragraphs = reading.of_kind(UnitKind::Paragraph);
        assert_eq!(
            paragraphs.len(),
            2,
            "{}: {:#?}",
            reading.format,
            reading.units
        );
        assert_eq!(paragraphs[0].text, "المؤشر", "{}", reading.format);
        assert_eq!(paragraphs[1].text, "الربع", "{}", reading.format);

        let table = reading.only(UnitKind::Table);
        assert_eq!(table.text, "المؤشر\nالربع", "{}", reading.format);
    }
}

#[test]
fn a_column_order_the_table_states_is_explicit_and_one_it_does_not_is_unset() {
    let stated = Document::default().with_table(Table::of(&["المؤشر"]).direction(Direction::Rtl));
    for reading in read_all(&stated) {
        assert_eq!(
            reading.only(UnitKind::Table).props.direction,
            Resolved::Explicit(Direction::Rtl),
            "{}",
            reading.format
        );
    }

    let unstated = Document::default().with_table(Table::of(&["المؤشر"]));
    for reading in read_all(&unstated) {
        assert!(
            reading.only(UnitKind::Table).props.direction.is_unset(),
            "{}",
            reading.format
        );
    }
}

#[test]
fn a_list_the_format_produces_itself_is_a_native_bullet_in_every_format() {
    // What separates a real list from a glyph somebody typed, which is the one
    // distinction `literal-bullet` rests on.
    let doc = Document::one(Paragraph::of(ARABIC).bullet());
    for reading in read_all(&doc) {
        assert_eq!(
            reading.only(UnitKind::Paragraph).props.bullet,
            Bullet::Native,
            "{}",
            reading.format
        );
    }

    for reading in read_all(&Document::one(Paragraph::of(ARABIC))) {
        assert_eq!(
            reading.only(UnitKind::Paragraph).props.bullet,
            Bullet::None,
            "{}",
            reading.format
        );
    }
}

// --------------------------------------------------- the rules see one shape

#[test]
fn arabic_with_nothing_declared_is_reported_the_same_way_in_every_format() {
    // Three findings, not one: nothing says which way it reads, nothing says
    // it is Arabic, and nothing says which edge it starts on. Both formats
    // reach all three from a paragraph that states nothing at all.
    assert_eq!(
        agree_on(&Document::one(Paragraph::of(ARABIC))),
        ["alignment-unset", "direction-unset", "language-missing"]
    );
}

#[test]
fn arabic_declared_left_to_right_is_reported_the_same_way_in_every_format() {
    // The flagship finding: the direction is declared, and declared wrongly.
    // Mixed text, because that is where a wrong base direction actually moves
    // something: the digits and the `Q4` land in the wrong place, and the two
    // resolved orders below are the proof of it.
    let doc = Document::one(
        Paragraph::correct_arabic()
            .saying(MIXED)
            .direction(Direction::Ltr),
    );
    assert_eq!(agree_on(&doc), ["direction-mismatch"]);

    // And it carries the proof, in both, without either being asked what it is.
    for reading in read_all(&doc) {
        let report = Engine::with_default_rules().audit(&reading.units);
        let finding = report
            .diagnostics
            .iter()
            .find(|d| d.rule.0 == "direction-mismatch")
            .expect("the rule fired a moment ago");
        assert_eq!(finding.severity, Severity::Error, "{}", reading.format);
        assert!(finding.fixable, "{}", reading.format);
        assert!(
            finding.evidence.visual_declared.is_some()
                && finding.evidence.visual_declared != finding.evidence.visual_expected,
            "{}: the finding carries no proof",
            reading.format
        );
    }
}

#[test]
fn correctly_marked_arabic_is_silent_in_every_format() {
    assert_eq!(
        agree_on(&Document::one(Paragraph::correct_arabic())),
        Vec::<&str>::new()
    );
}

#[test]
fn english_is_silent_in_every_format() {
    // The rules are about Arabic. English left entirely undeclared is what
    // every document in the world is full of, and reporting it would make the
    // tool unusable on a mixed document.
    assert_eq!(
        agree_on(&Document::one(Paragraph::of(ENGLISH))),
        Vec::<&str>::new()
    );
}

#[test]
fn an_arabic_table_with_no_column_order_is_reported_on_the_table_and_not_its_cells() {
    let doc = Document::default().with_table(Table::of(&["المؤشر", "الربع"]).correct_cells());
    assert_eq!(agree_on(&doc), ["container-direction"]);

    for reading in read_all(&doc) {
        assert_eq!(
            reading.rules_on(UnitKind::Table),
            ["container-direction"],
            "{}",
            reading.format
        );
        assert!(
            reading.rules_on(UnitKind::Paragraph).is_empty(),
            "{}: the cells were blamed for the table's column order",
            reading.format
        );
    }
}

#[test]
fn an_arabic_table_that_states_its_column_order_is_silent_in_every_format() {
    let doc = Document::default().with_table(
        Table::of(&["المؤشر", "الربع"])
            .direction(Direction::Rtl)
            .correct_cells(),
    );
    assert_eq!(agree_on(&doc), Vec::<&str>::new());
}

#[test]
fn a_chain_that_agrees_with_the_text_silences_the_finding_in_every_format() {
    // ADR 0007's first half: a right-to-left chain over Arabic is the template
    // doing its job, and reporting it is the false positive that motivated
    // `Resolved` in the first place.
    let doc = Document::one(
        Paragraph::of(ARABIC)
            .language("ar-SA")
            .complex_font("Dubai"),
    )
    .under(Chain::stating(Direction::Rtl).and_alignment(Alignment::Center));
    assert_eq!(agree_on(&doc), Vec::<&str>::new());
}

#[test]
fn a_chain_that_contradicts_the_text_still_reports_and_names_itself_in_every_format() {
    // ADR 0007's second half: an English template's untouched default under
    // Arabic is a value nobody aimed at the text, and is reported exactly as an
    // absent one — naming the part that supplied it, so the reviewer knows the
    // defect is in the template.
    // The chain's alignment is left agreeing with the text, so the one thing
    // under test here is what a contradicting *direction* does.
    let doc = Document::one(
        Paragraph::of(ARABIC)
            .language("ar-SA")
            .complex_font("Dubai"),
    )
    .under(Chain::stating(Direction::Ltr).and_alignment(Alignment::Center));
    assert_eq!(agree_on(&doc), ["direction-unset"]);

    for reading in read_all(&doc) {
        let report = Engine::with_default_rules().audit(&reading.units);
        let finding = &report.diagnostics[0];
        let cited = finding
            .evidence
            .inherited_from
            .as_ref()
            .unwrap_or_else(|| panic!("{}: the finding names no source", reading.format));
        assert!(
            reading.parts.iter().any(|part| cited.starts_with(part)),
            "{}: the finding cites {cited:?}, which names no part the package holds",
            reading.format,
        );
        // Repaired on the unit, never on the source: writing to the chain would
        // change every paragraph under it, including correct English text.
        assert_eq!(
            finding.unit.0,
            reading.only(UnitKind::Paragraph).id.0,
            "{}",
            reading.format
        );
    }
}

#[test]
fn a_typed_bullet_glyph_is_reported_in_every_format_and_a_real_list_is_not() {
    let typed = Document::one(Paragraph::correct_arabic().saying("• بند أول"));
    assert!(agree_on(&typed).contains(&"literal-bullet"));

    let native = Document::one(Paragraph::correct_arabic().bullet());
    assert!(!agree_on(&native).contains(&"literal-bullet"));
}

#[test]
fn an_explicit_bidi_control_is_reported_in_every_format() {
    // Invariant 4: direction belongs to the container. A control character
    // smuggled into the text is the workaround this tool exists to replace.
    let doc = Document::one(Paragraph::correct_arabic().saying("\u{202B}ارتفع الأداء\u{202C}"));
    assert!(agree_on(&doc).contains(&"bidi-control"));
}

#[test]
fn pre_shaped_presentation_forms_are_reported_in_every_format() {
    // Text pasted out of a PDF. It looks right until anything tries to search,
    // copy or re-shape it.
    let doc = Document::one(Paragraph::correct_arabic().saying("ﺍﻟﺘﻘﺮﻳﺮ"));
    assert!(agree_on(&doc).contains(&"presentation-forms"));
}

#[test]
fn typed_tatweel_padding_a_heading_is_reported_in_every_format() {
    // PLAN §4.4's acceptance. `PADDED` is العنوان with five tatweel pushed onto
    // the end of it, spelled out because a run of tatweel in source cannot be
    // counted by eye.
    const PADDED: &str = "العنوان\u{0640}\u{0640}\u{0640}\u{0640}\u{0640}";
    let doc = Document::one(Paragraph::correct_arabic().saying(PADDED));
    assert!(agree_on(&doc).contains(&"tatweel-padding"));

    // With the offsets, which is what makes the finding checkable: the run
    // starts where العنوان ends, and is five long.
    for reading in read_all(&doc) {
        let report = Engine::with_default_rules().audit(&reading.units);
        let finding = report
            .diagnostics
            .iter()
            .find(|d| d.rule.0 == "tatweel-padding")
            .unwrap_or_else(|| panic!("{}: nothing reported", reading.format));
        assert_eq!(
            finding.evidence.offenders,
            vec![format!(
                "U+0640 ARABIC TATWEEL \u{d7}5 @{}",
                "العنوان".len()
            )],
            "{}",
            reading.format
        );
    }
}

/// The four situations M5 §5.2 added, each stated once here.
///
/// All four run against one format, and that is the formats' answer rather than
/// the adapter's: a run that is a bidi boundary, an inset from a named edge and
/// a layout reversed without a direction are three things OOXML has no
/// vocabulary for, and `every_refusal_is_one_the_design_intended` holds those
/// refusals to the committed list. What these cases add is the other half —
/// that the format which *can* state each one reports it, and reports the
/// silence beside it.
mod runs_insets_and_reversal {
    use super::*;

    /// Every format that can state the situation, and what it reported.
    fn only_html(doc: &Document) -> Vec<&'static str> {
        let readings = read(doc);
        assert_eq!(
            readings.iter().map(|r| r.format).collect::<Vec<_>>(),
            ["html"],
            "the one format with a vocabulary for this"
        );
        readings.into_iter().next().expect("one reading").rules()
    }

    #[test]
    fn an_imposed_order_is_reported_and_a_declared_one_is_not() {
        // The same direction, said two ways. Overriding lays the digits out
        // backwards; declaring leaves the algorithm to them.
        let imposed = Document::one(
            Paragraph::correct_arabic()
                .saying(MIXED)
                .run("Q4 2026", SpanBidi::Imposed(Direction::Rtl)),
        );
        assert!(only_html(&imposed).contains(&"bidi-override"));

        let declared = Document::one(
            Paragraph::correct_arabic()
                .saying(MIXED)
                .run("Q4 2026", SpanBidi::Isolated),
        );
        assert!(!only_html(&declared).contains(&"bidi-override"));
    }

    #[test]
    fn a_run_that_decides_its_surroundings_is_reported_unless_it_is_isolated() {
        // A Latin name in an Arabic line with a neutral after it: the classic
        // case `<bdi>` was added to the language for.
        const INTERPOLATED: &str = "المالك: John Smith - 5";
        let unisolated = Document::one(
            Paragraph::correct_arabic()
                .saying(INTERPOLATED)
                .run("John Smith", SpanBidi::Plain),
        );
        assert!(only_html(&unisolated).contains(&"isolation-missing"));

        let isolated = Document::one(
            Paragraph::correct_arabic()
                .saying(INTERPOLATED)
                .run("John Smith", SpanBidi::Isolated),
        );
        assert!(!only_html(&isolated).contains(&"isolation-missing"));
    }

    #[test]
    fn a_physical_inset_is_reported_and_the_direction_relative_one_is_not() {
        let physical = Document::one(Paragraph::correct_arabic().inset(Inset::Left));
        assert!(only_html(&physical).contains(&"inset-physical"));

        let logical = Document::one(Paragraph::correct_arabic().inset(Inset::Start));
        assert!(!only_html(&logical).contains(&"inset-physical"));
    }

    #[test]
    fn a_layout_reversed_instead_of_directed_is_reported() {
        const BOXES: &[&str] = &["المؤشر", "الربع الرابع"];
        let faked = Document::default().with_reversed(Reversed {
            boxes: BOXES,
            direction: None,
        });
        assert!(only_html(&faked).contains(&"order-reversed"));

        // And declaring the direction as well does not answer it: the two
        // reversals cancel, so the boxes come back out the way they started.
        let both = Document::default().with_reversed(Reversed {
            boxes: BOXES,
            direction: Some(Direction::Rtl),
        });
        assert!(only_html(&both).contains(&"order-reversed"));
    }

    #[test]
    fn correctly_marked_arabic_stays_silent_with_all_four_registered() {
        // The check that matters most: four rules were added to the engine
        // every format's units pass through, and none of them may have made a
        // clean document noisy.
        let clean = Document::one(Paragraph::correct_arabic());
        assert!(agree_on(&clean).is_empty());
    }
}

#[test]
fn arabic_justified_by_its_font_is_reported_by_nobody() {
    // The other half of §4.4's acceptance, and the reason the rule needed a
    // threshold at all. Kashida justification is the font's, applied at layout
    // time: it never reaches the stored string, so a justified paragraph has
    // nothing in it to report.
    let justified = Document::one(Paragraph::correct_arabic().alignment(Alignment::Justify));
    assert!(!agree_on(&justified).contains(&"tatweel-padding"));

    // And the cases that would fail a rule which regressed to "any tatweel is
    // a defect": a fatha written on its own, medial heh as a primer shows it,
    // and a rule drawn with the character that draws rules.
    for legitimate in [
        "\u{0640}\u{064E}",
        "\u{0640}ه\u{0640}",
        "مرحبا \u{0640}\u{0640}\u{0640}\u{0640} عالم",
    ] {
        let doc = Document::one(Paragraph::correct_arabic().saying(legitimate));
        assert!(
            !agree_on(&doc).contains(&"tatweel-padding"),
            "{legitimate:?} was reported"
        );
    }
}

#[test]
fn a_latin_font_with_an_empty_arabic_slot_is_reported_by_every_format_that_can_state_it() {
    // The commonest silent defect there is: the Arabic renders in whatever the
    // application substitutes, which is not the typeface anybody chose.
    //
    // HTML is left out, and that is the format's answer rather than the
    // adapter's. CSS gives an element one `font-family` for every script, so
    // there is no pair of slots to fill unevenly and no defect here to write.
    let doc = Document::one(
        Paragraph::correct_arabic()
            .without_complex_font()
            .latin_font("Calibri"),
    );
    let readings = read(&doc);
    assert_eq!(
        readings.iter().map(|r| r.format).collect::<Vec<_>>(),
        ["pptx", "docx"],
        "the two formats with two font slots state it, and the one with a \
         single stack cannot"
    );
    for reading in &readings {
        assert_eq!(
            reading.rules(),
            ["complex-font-missing"],
            "{}",
            reading.format
        );
    }
}

// ------------------------------------------------------------- the refusals

#[test]
fn every_refusal_is_one_the_design_intended() {
    // The list of situations a format genuinely cannot state. Every entry is a
    // property of a file format rather than of an adapter, and the list is
    // committed here so that a format which quietly stopped expressing
    // something else fails rather than passes.
    let physical = Document::one(Paragraph::of(ARABIC).alignment(Alignment::Left));
    let relative = Document::one(Paragraph::of(ARABIC).alignment(Alignment::Start));
    let distributed = Document::one(Paragraph::of(ARABIC).alignment(Alignment::Distributed));
    let one_slot_filled = Document::one(Paragraph::of(ARABIC).latin_font("Calibri"));
    let delimited_run = Document::one(Paragraph::of(MIXED).run("Q4", SpanBidi::Plain));
    let physical_inset = Document::one(Paragraph::of(ARABIC).inset(Inset::Left));
    let logical_inset = Document::one(Paragraph::of(ARABIC).inset(Inset::Start));
    let boxes_reversed = Document::default().with_reversed(Reversed {
        boxes: &["المؤشر", "الربع الرابع"],
        direction: None,
    });

    let refusing = |doc: &Document| {
        refusals(doc)
            .iter()
            .map(|(format, _)| *format)
            .collect::<Vec<_>>()
    };

    assert_eq!(
        refusing(&physical),
        ["docx"],
        "a physical edge is Word's alone to refuse: its w:jc is direction-relative"
    );
    assert_eq!(
        refusing(&relative),
        ["pptx"],
        "a direction-relative edge is PowerPoint's alone to refuse: a:pPr/@algn \
         names physical edges"
    );
    assert_eq!(
        refusing(&distributed),
        ["html"],
        "distributed text is the one alignment CSS has no value for"
    );
    assert_eq!(
        refusing(&one_slot_filled),
        ["html"],
        "CSS has one font stack per element, so a filled Latin slot beside an \
         empty complex-script one is not a document the web can write"
    );
    assert_eq!(
        refusing(&delimited_run),
        ["pptx", "docx"],
        "a run that is a bidi boundary is the web's alone: OOXML's runs carry \
         properties and say nothing about isolation or override"
    );
    for inset in [&physical_inset, &logical_inset] {
        assert_eq!(
            refusing(inset),
            ["pptx", "docx"],
            "a paragraph inset from a named edge is the web's alone: OOXML's \
             indents are direction-relative and have no physical spelling"
        );
    }
    assert_eq!(
        refusing(&boxes_reversed),
        ["pptx", "docx"],
        "displaying a container's boxes in the reverse of the stored order is \
         the web's alone: the other two state a direction and nothing else"
    );

    // Everything else every format states.
    for doc in [
        Document::one(Paragraph::of(ARABIC)),
        Document::one(Paragraph::correct_arabic()),
        Document::one(Paragraph::of(ARABIC).alignment(Alignment::Center)),
        Document::one(Paragraph::of(ARABIC).alignment(Alignment::Justify)),
        Document::one(Paragraph::of(ARABIC).bullet()),
        Document::default().with_table(Table::of(&["المؤشر"])),
        Document::one(Paragraph::of(ARABIC)).under(Chain::stating(Direction::Rtl)),
    ] {
        assert!(
            refusals(&doc).is_empty(),
            "a format refused a situation the design expects every format to state: {:?}",
            refusals(&doc)
        );
    }
}

#[test]
fn a_hard_left_edge_under_arabic_is_reported_by_every_format_that_can_state_it() {
    // The one asymmetry a user could mistake for missing coverage, so it is
    // asserted rather than left implied: `alignment-incoherent` is structurally
    // silent on Word not because the adapter is thin but because Word has no
    // way to write the defect. Both formats that *can* write it report it, and
    // they report the same thing — PowerPoint's `algn="l"` and CSS's
    // `text-align: left` are the same instruction under Arabic.
    let doc = Document::one(Paragraph::correct_arabic().alignment(Alignment::Left));
    let readings = read(&doc);
    assert_eq!(
        readings.iter().map(|r| r.format).collect::<Vec<_>>(),
        ["pptx", "html"],
        "the formats with a physical left edge"
    );
    for reading in &readings {
        assert_eq!(
            reading.rules(),
            ["alignment-incoherent"],
            "{}",
            reading.format
        );
    }
}

/// A corpus deck is not needed to know that both adapters read a real package:
/// this asserts it against the committed ones, which is the only place a
/// format's own writer has been anywhere near the bytes.
#[test]
fn every_committed_corpus_document_reads_through_the_same_port() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut seen = Vec::new();
    for entry in fs::read_dir(&fixtures).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.contains(".out.") {
            continue;
        }
        let Some(vocabulary) = vocabularies()
            .into_iter()
            .find(|v| path.extension().is_some_and(|e| e == v.extension()))
        else {
            continue;
        };

        let mut document = vocabulary.open(&path).unwrap();
        let units = document
            .scan()
            .unwrap_or_else(|e| panic!("{name}: the scan failed: {e}"));
        let parts = vocabulary.parts_of(&path);
        for unit in &units {
            assert!(
                parts.contains(&unit.location.part),
                "{name}: {} names a part the package does not hold",
                unit.id
            );
        }
        seen.push(document.format());
    }

    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen.len(),
        vocabularies().len(),
        "the corpus does not hold a document of every format this build reads: {seen:?}"
    );
}
