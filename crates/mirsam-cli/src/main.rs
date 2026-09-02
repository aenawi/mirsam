//! `mirsam` — audit and repair Arabic text in documents.
//!
//! The CLI is a driving adapter and nothing more: it parses arguments, selects
//! a format adapter, hands units to the engine and renders the report. It holds
//! no correctness logic of its own, which is what keeps a future language
//! server or library caller from having to reimplement any of it.

mod render;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use mirsam_core::error::Error as CoreError;
use mirsam_core::{DocumentReader, Engine};
use mirsam_ooxml::PptxDocument;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Exit codes, stable across releases so CI can branch on them.
mod exit {
    /// No blocking findings.
    pub const OK: u8 = 0;
    /// The document has blocking findings.
    pub const FINDINGS: u8 = 1;
    /// Bad invocation.
    pub const USAGE: u8 = 2;
    /// The document could not be read.
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

fn open(path: &Path) -> Result<Box<dyn DocumentReader>> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "pptx" => Ok(Box::new(
            PptxDocument::open(path).with_context(|| format!("opening {}", path.display()))?,
        )),
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
    match error.downcast_ref::<CoreError>() {
        Some(CoreError::UnknownFormat(_)) => (
            exit::USAGE,
            Some(format!(
                "mirsam {} reads .pptx; the other formats are scheduled in docs/ROADMAP.md",
                env!("CARGO_PKG_VERSION")
            )),
        ),
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
            format,
        } => {
            let mut document = open(&file)?;
            let units = document
                .scan()
                .with_context(|| format!("reading {}", file.display()))?;

            let report = Engine::with_default_rules().audit(&units);
            let blocking = report.is_blocking(strict);
            render::report(
                &file,
                document.format(),
                &report,
                strict,
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
