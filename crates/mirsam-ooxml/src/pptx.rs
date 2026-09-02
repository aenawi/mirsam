//! PowerPoint (PPTX) adapter.

use mirsam_core::error::{Error, Result};
use mirsam_core::ports::DocumentReader;
use mirsam_core::text::{Alignment, Bullet, Direction, Location, Properties, Resolved, TextUnit};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

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

fn is_true(value: &str) -> bool {
    matches!(value, "1" | "true" | "on")
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
        TextUnit::new(format!("{part}#p{index}"), self.text)
            .with_props(self.props)
            .with_location(Location {
                part: part.to_string(),
                paragraph: Some(index),
                container: self.shape,
            })
    }
}

/// A PowerPoint package opened for auditing.
pub struct PptxDocument {
    path: PathBuf,
}

impl PptxDocument {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if !path.exists() {
            return Err(Error::Format(format!("no such file: {}", path.display())));
        }
        Ok(Self { path })
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
                            body_rtl = e
                                .attributes()
                                .flatten()
                                .find(|a| a.key.as_ref() == "rtlCol")
                                .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                                .map(|v| is_true(&v));
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
                                                Resolved::Explicit(if is_true(&value) {
                                                    Direction::Rtl
                                                } else {
                                                    Direction::Ltr
                                                });
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

                Ok(Event::End(e)) => match e.name().as_ref() {
                    "a:t" => in_text = false,
                    "a:p" => {
                        if let Some(b) = current.take()
                            && !b.text.trim().is_empty()
                        {
                            units.push(b.finish(part, paragraph_index));
                        }
                    }
                    "p:sp" | "p:graphicFrame" | "p:pic" | "p:cxnSp" => shape = None,
                    "a:txBody" | "c:txPr" => body_rtl = None,
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
        let file = File::open(&self.path)?;
        let mut archive = ZipArchive::new(file)
            .map_err(|e| Error::Format(format!("not a readable OOXML package: {e}")))?;

        let names: Vec<String> = archive.file_names().map(str::to_string).collect();
        if !names.iter().any(|n| n == "[Content_Types].xml") {
            return Err(Error::Format(
                "not an OOXML package: [Content_Types].xml is missing".into(),
            ));
        }

        let mut parts: Vec<String> = names
            .into_iter()
            .filter(|n| n.starts_with("ppt/") && n.ends_with(".xml"))
            .collect();
        parts.sort();

        let mut units = Vec::new();
        for part in parts {
            let mut buf = String::new();
            archive
                .by_name(&part)
                .map_err(|e| Error::Format(format!("{part}: {e}")))?
                .read_to_string(&mut buf)?;
            units.extend(Self::scan_part(&part, &buf)?);
        }
        Ok(units)
    }
}

/// Parse an in-memory part. Exposed for tests and for callers that already
/// hold the XML.
pub fn scan_xml(part: &str, xml: &str) -> Result<Vec<TextUnit>> {
    PptxDocument::scan_part(part, xml)
}

/// Reads a part out of a package without going through the filesystem twice.
pub fn read_part(bytes: &[u8], part: &str) -> Result<String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| Error::Format(format!("not a readable OOXML package: {e}")))?;
    let mut buf = String::new();
    archive
        .by_name(part)
        .map_err(|e| Error::Format(format!("{part}: {e}")))?
        .read_to_string(&mut buf)?;
    Ok(buf)
}

/// The path a document was opened from.
pub fn source_path(doc: &PptxDocument) -> &Path {
    &doc.path
}
