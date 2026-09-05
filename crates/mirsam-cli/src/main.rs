//! `mirsam` — audit and repair Arabic text in documents.
//!
//! The CLI is a driving adapter and nothing more: it parses arguments, selects
//! a format adapter, hands units to the engine and renders the report. It holds
//! no correctness logic of its own, which is what keeps a future language
//! server or library caller from having to reimplement any of it.

mod render;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use mirsam_core::error::Error as CoreError;
use mirsam_core::rules::{DEFAULT_LOCALE, is_arabic_tag};
use mirsam_core::{DocumentReader, DocumentWriter, Engine, FontSource, Repair, RepairOptions};
use mirsam_fonts::SystemFonts;
use mirsam_html::HtmlDocument;
use mirsam_ooxml::{DocxDocument, PptxDocument};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

/// Exit codes, stable across releases so CI can branch on them.
mod exit {
    /// No blocking findings.
    pub const OK: u8 = 0;
    /// The document has blocking findings.
    pub const FINDINGS: u8 = 1;
    /// Bad invocation.
    pub const USAGE: u8 = 2;
    /// The document could not be read, or the output could not be written.
    pub const UNREADABLE: u8 = 3;
}

#[derive(Parser)]
#[command(
    name = "mirsam",
    version,
    about = "Audit and repair Arabic RTL, bidi and typography in documents",
    long_about = "mirsam resolves what Arabic text will actually look like when rendered, \
                  using the Unicode bidirectional algorithm, and reports a defect only when \
                  the resolved order is wrong — not merely when an attribute is absent."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect a document without modifying it.
    Audit {
        file: PathBuf,
        /// Treat warnings as blocking.
        #[arg(long)]
        strict: bool,
        #[command(flatten)]
        fonts: FontFlags,
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Write a repaired copy of a document, then audit the copy.
    ///
    /// Every repair is byte-preserving: whatever no repair addresses passes
    /// through untouched. The input is never modified. The report describes
    /// the file actually written, re-read from disk, next to the audit of the
    /// input it came from.
    Repair {
        /// The document to repair. Never modified.
        input: PathBuf,
        /// Where to write the repaired copy. Must not be INPUT.
        output: PathBuf,
        /// BCP-47 tag written where Arabic text carries no Arabic language tag.
        #[arg(long, value_name = "TAG", default_value = DEFAULT_LOCALE, value_parser = arabic_tag)]
        lang: String,
        /// Complex-script typeface written where a Latin font is set and the
        /// Arabic slot is empty. Without it those findings are reported, not
        /// repaired: choosing a typeface is an authoring decision.
        #[arg(long, value_name = "TYPEFACE", value_parser = non_empty)]
        font: Option<String>,
        /// Replace typed bullet glyphs with native list formatting. Opt-in,
        /// because it edits the text itself rather than the properties around it.
        #[arg(long)]
        convert_bullets: bool,
        /// Delete typed tatweel that pads text to a width. Opt-in, and more
        /// cautiously than --convert-bullets: it deletes characters the author
        /// typed. Only the padding goes — a tatweel carrying a harakat, or one
        /// showing a letter's contextual form, is never reported and never
        /// touched.
        #[arg(long)]
        strip_tatweel: bool,
        /// Write an explicit start-edge alignment on right-to-left paragraphs
        /// that have none of their own. Opt-in: what such a paragraph inherits
        /// may be a layout's design, such as a centred title, and the tool
        /// cannot yet read the layout to tell.
        #[arg(long)]
        align: bool,
        /// Replace OUTPUT if it already exists. Never permits OUTPUT to be INPUT.
        #[arg(long)]
        force: bool,
        /// Treat warnings remaining after the repair as blocking.
        #[arg(long)]
        strict: bool,
        #[command(flatten)]
        fonts: FontFlags,
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Show how text resolves under each base direction.
    ///
    /// Takes text directly, so a defect can be reproduced without a document.
    Explain {
        text: String,
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// List every rule the engine can apply.
    Rules {
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Format {
    Text,
    Json,
}

/// Whether to shape the document's Arabic through the fonts this machine has.
///
/// Off by default, and that is a position rather than caution. Every other
/// check in this tool reads the document and would give the same answer on any
/// computer; `font-coverage` and `shaping-broken` read the *machine*, and an
/// audit whose result depended on which fonts happened to be installed on the
/// runner would be one nobody could reproduce. So the caller asks, and the
/// report says which of the two audits it is — a check that did not run is
/// `NOT RUN`, never an implied pass.
///
/// Note which way the claim runs when they are on. A font that is *here* and
/// cannot draw the text will not draw it anywhere, because a `cmap` and a
/// `GSUB` travel with the font. A font that is here and draws it perfectly
/// proves nothing about the reader's machine, and silence is not a promise
/// that the document will render.
#[derive(Args, Clone, Debug)]
struct FontFlags {
    /// Resolve each paragraph's complex-script typeface against the fonts
    /// installed here, and report Arabic the resolved font cannot draw or
    /// cannot join. Machine-dependent: see --font-dir.
    #[arg(long = "fonts")]
    enabled: bool,
    /// Search these directories instead of the platform's font directories.
    /// Implies --fonts, and is what makes the check reproducible: a fixed
    /// directory gives a fixed answer on any machine.
    #[arg(long = "font-dir", value_name = "DIR")]
    dirs: Vec<PathBuf>,
}

impl FontFlags {
    fn requested(&self) -> bool {
        self.enabled || !self.dirs.is_empty()
    }

    /// The source to arm the font rules with, or `None` to leave them unrun.
    ///
    /// Built once per invocation and shared: `SystemFonts` indexes lazily and
    /// keeps what it found, so a document of forty paragraphs walks the font
    /// directories once rather than forty times.
    fn source(&self) -> Option<Arc<dyn FontSource>> {
        if !self.requested() {
            return None;
        }
        Some(Arc::new(if self.dirs.is_empty() {
            SystemFonts::new()
        } else {
            SystemFonts::in_dirs(self.dirs.clone())
        }))
    }
}

/// The engine, with the font rules armed only if the caller asked for them.
fn engine(options: &RepairOptions, fonts: &FontFlags) -> Engine {
    match fonts.source() {
        Some(source) => Engine::with_fonts(options, source),
        None => Engine::with_options(options),
    }
}

/// `--lang` must be a tag the `language-missing` rule accepts, or the repair
/// would write a value the re-audit immediately reports again.
fn arabic_tag(value: &str) -> std::result::Result<String, String> {
    if is_arabic_tag(value) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "{value:?} is not an Arabic language tag; expected ar, ar-SA, ar-AE, …"
        ))
    }
}

fn non_empty(value: &str) -> std::result::Result<String, String> {
    if value.trim().is_empty() {
        Err("must not be empty".to_string())
    } else {
        Ok(value.to_string())
    }
}

/// A refusal that is the caller's to resolve. Nothing was read or written.
#[derive(Debug)]
enum Refusal {
    OutputExists(PathBuf),
    OutputExtension {
        input: PathBuf,
        output: PathBuf,
    },
    /// A format this build can audit but cannot yet repair. Distinct from an
    /// unknown extension: the document was understood, and saying so is the
    /// difference between "mirsam does not know what this is" and "mirsam
    /// knows, and has not learned to write it back yet".
    ReadOnlyFormat(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutputExists(path) => write!(
                f,
                "{} already exists; pass --force to replace it",
                path.display()
            ),
            Self::OutputExtension { input, output } => write!(
                f,
                "{} must keep the .{} extension of {}: the repaired document is the same format",
                output.display(),
                extension(input),
                input.display()
            ),
            Self::ReadOnlyFormat(format) => write!(
                f,
                ".{format} can be audited but not yet repaired; \
                 `mirsam audit` reads it, and the writer is scheduled in docs/ROADMAP.md"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// True when both paths name the same existing file, through symlinks.
///
/// A path that cannot be canonicalised is compared literally, which is safe:
/// an output that does not exist yet cannot be the input.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// The extensions `open` accepts, for the hint an unknown one gets.
const READABLE: &[&str] = &["pptx", "docx", "html", "htm"];

fn open(path: &Path) -> Result<Box<dyn DocumentReader>> {
    let context = || format!("opening {}", path.display());
    match extension(path).as_str() {
        "pptx" => Ok(Box::new(PptxDocument::open(path).with_context(context)?)),
        "docx" => Ok(Box::new(DocxDocument::open(path).with_context(context)?)),
        "html" | "htm" => Ok(Box::new(HtmlDocument::open(path).with_context(context)?)),
        other => Err(CoreError::UnknownFormat(other.to_string()).into()),
    }
}

/// The formats that can be repaired in place are a subset of those that can
/// be read; this is where a read-only adapter is turned away.
///
/// A readable format without a writer is refused as such rather than as an
/// unknown one, so the message does not deny knowing a document the audit
/// path reads perfectly well.
fn open_for_repair(path: &Path) -> Result<Box<dyn DocumentWriter>> {
    match extension(path).as_str() {
        "pptx" => Ok(Box::new(
            PptxDocument::open(path).with_context(|| format!("opening {}", path.display()))?,
        )),
        readable if READABLE.contains(&readable) => {
            Err(Refusal::ReadOnlyFormat(readable.to_string()).into())
        }
        other => Err(CoreError::UnknownFormat(other.to_string()).into()),
    }
}

/// Map a failure onto its documented exit code, and any hint worth adding.
///
/// Classified by the error's *type*, never by searching its rendered message.
/// Exit codes are a stable contract that CI branches on; deciding one by
/// substring match makes it hostage to wording, and to whatever text happens to
/// appear in a user's document.
fn classify(error: &anyhow::Error) -> (u8, Option<String>) {
    if error.downcast_ref::<Refusal>().is_some() {
        return (exit::USAGE, None);
    }
    match error.downcast_ref::<CoreError>() {
        Some(CoreError::UnknownFormat(_)) => (
            exit::USAGE,
            Some(format!(
                "mirsam {} reads {}; the other formats are scheduled in docs/ROADMAP.md",
                env!("CARGO_PKG_VERSION"),
                READABLE
                    .iter()
                    .map(|e| format!(".{e}"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            )),
        ),
        Some(CoreError::WouldOverwriteSource) => (exit::USAGE, None),
        _ => (exit::UNREADABLE, None),
    }
}

fn run() -> Result<u8> {
    match Cli::parse().command {
        Command::Rules { format } => {
            render::rules(&Engine::with_default_rules(), format == Format::Json);
            Ok(exit::OK)
        }

        Command::Explain { text, format } => {
            render::explain(&text, format == Format::Json);
            Ok(exit::OK)
        }

        Command::Audit {
            file,
            strict,
            fonts,
            format,
        } => {
            let mut document = open(&file)?;
            let units = document
                .scan()
                .with_context(|| format!("reading {}", file.display()))?;

            let report = engine(&RepairOptions::default(), &fonts).audit(&units);
            let blocking = report.is_blocking(strict);
            render::report(
                &file,
                document.format(),
                &report,
                strict,
                fonts.requested(),
                &document.unread_sources(),
                format == Format::Json,
            );

            Ok(if blocking { exit::FINDINGS } else { exit::OK })
        }

        Command::Repair {
            input,
            output,
            lang,
            font,
            convert_bullets,
            strip_tatweel,
            align,
            force,
            strict,
            fonts,
            format,
        } => {
            // Refuse before reading anything. The writer refuses the first case
            // again on its own — that check is the guarantee; this one is the
            // courtesy of saying so up front, and of not letting --force be
            // mistaken for permission.
            if same_file(&input, &output) {
                return Err(CoreError::WouldOverwriteSource.into());
            }
            if extension(&input) != extension(&output) {
                return Err(Refusal::OutputExtension { input, output }.into());
            }
            if output.exists() && !force {
                return Err(Refusal::OutputExists(output).into());
            }

            let options = RepairOptions {
                language: lang,
                complex_font: font,
                convert_bullets,
                strip_tatweel,
                align,
            };
            let engine = engine(&options, &fonts);

            let mut document = open_for_repair(&input)?;
            let units = document
                .scan()
                .with_context(|| format!("reading {}", input.display()))?;
            let before = engine.audit(&units);

            // A repair the adapter cannot express is reported as not made, and
            // the rest still go ahead; one pre-shaped paragraph must not stop
            // the other forty from being fixed.
            let (applied, skipped): (Vec<Repair>, Vec<Repair>) = engine
                .plan(&units)
                .into_iter()
                .partition(|repair| document.supports(&repair.fix));

            let staged = document
                .apply(&applied)
                .with_context(|| format!("repairing {}", input.display()))?;
            if staged != applied.len() {
                bail!(
                    "the {} adapter staged {staged} of {} repairs and did not say why",
                    document.format(),
                    applied.len()
                );
            }
            document
                .write(&output)
                .with_context(|| format!("writing {}", output.display()))?;

            // Audit what was written, not what was intended: the second report
            // describes the file on disk, re-read through the same path a
            // later `audit` would take.
            let mut repaired = open(&output)?;
            let after = engine.audit(
                &repaired
                    .scan()
                    .with_context(|| format!("re-reading {}", output.display()))?,
            );

            let blocking = after.is_blocking(strict);
            render::repair(
                &render::Repaired {
                    input: &input,
                    output: &output,
                    format: document.format(),
                    options: &options,
                    units: &units,
                    applied: &applied,
                    skipped: &skipped,
                    before: &before,
                    after: &after,
                    strict,
                    fonts_checked: fonts.requested(),
                },
                format == Format::Json,
            );

            Ok(if blocking { exit::FINDINGS } else { exit::OK })
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            // Distinguish "could not read the document" from "found problems",
            // so a pipeline can retry the former and fail the latter.
            let (code, hint) = classify(&error);
            eprintln!("error: {error:#}");
            if let Some(hint) = hint {
                eprintln!("  {hint}");
            }
            ExitCode::from(code)
        }
    }
}
