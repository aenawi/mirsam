//! Output rendering. Text for humans, JSON for agents.
//!
//! Both are first-class: an agent reading `--format json` should never have to
//! scrape the human output, and a human should never be handed raw JSON.

use mirsam_core::{
    Direction, Engine, Repair, RepairOptions, Report, Severity, TextUnit, UnitId, bidi,
};
use serde_json::json;
use std::path::Path;

pub fn rules(engine: &Engine, as_json: bool) {
    let rules: Vec<_> = engine.rules().collect();
    if as_json {
        let payload: Vec<_> = rules
            .iter()
            .map(|(id, description)| json!({ "id": id.0, "description": description }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return;
    }
    for (id, description) in rules {
        println!("{:<22} {}", id.0, description);
    }
}

pub fn explain(text: &str, as_json: bool) {
    let dominant = bidi::dominant_direction(text);
    let auto = bidi::auto_direction(text);
    let rtl = bidi::resolve(text, Direction::Rtl);
    let ltr = bidi::resolve(text, Direction::Ltr);
    let differs = rtl.visual != ltr.visual;

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "logical": text,
                "dominant_direction": dominant.to_string(),
                "auto_direction": auto.to_string(),
                "order_differs": differs,
                "visual": { "rtl": rtl.visual, "ltr": ltr.visual },
            }))
            .unwrap_or_default()
        );
        return;
    }

    println!("logical text      {text}");
    println!("dominant direction {dominant}");
    println!("auto-detected      {auto}");
    println!(
        "base direction     {}",
        if differs {
            "changes the rendering — declaring it is required"
        } else {
            "does not change the rendering for this text"
        }
    );
    if differs {
        // Codepoint escapes, not the glyphs: a terminal re-applies bidi to
        // already-reordered text and would show something misleading.
        println!("  as rtl           {}", escape(&rtl.visual));
        println!("  as ltr           {}", escape(&ltr.visual));
    }
}

/// Render visual-order text unambiguously for a terminal.
fn escape(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c.to_string()
            } else {
                format!("\\u{{{:04X}}}", c as u32)
            }
        })
        .collect()
}

/// The counts an agent branches on, in the same shape for every report.
fn summary(report: &Report) -> serde_json::Value {
    json!({
        "units_scanned": report.units_scanned,
        "arabic_units": report.arabic_units,
        "mixed_units": report.mixed_units,
        "errors": report.count(Severity::Error),
        "warnings": report.count(Severity::Warning),
        "notes": report.count(Severity::Note),
    })
}

/// One block per finding, as `audit` shows them.
fn findings(report: &Report) {
    for diagnostic in &report.diagnostics {
        println!(
            "{:<7} [{}] {}",
            diagnostic.severity.to_string(),
            diagnostic.rule,
            diagnostic.location
        );
        println!("        {}", diagnostic.message);
        if let Some(logical) = &diagnostic.evidence.logical {
            println!("        text: {logical}");
        }
        if !diagnostic.evidence.offenders.is_empty() {
            println!(
                "        found: {}",
                diagnostic.evidence.offenders.join(", ")
            );
        }
        println!();
    }
}

/// The closing line: the verdict, and the counts it was reached from.
fn verdict(report: &Report, strict: bool) {
    println!(
        "{}: errors={} warnings={} notes={} strict={}",
        if report.is_blocking(strict) {
            "FAIL"
        } else {
            "PASS"
        },
        report.count(Severity::Error),
        report.count(Severity::Warning),
        report.count(Severity::Note),
        if strict { "yes" } else { "no" }
    );
}

pub fn report(path: &Path, format: &str, report: &Report, strict: bool, as_json: bool) {
    if as_json {
        let payload = json!({
            "file": path.display().to_string(),
            "format": format,
            "strict": strict,
            "blocking": report.is_blocking(strict),
            "summary": summary(report),
            "diagnostics": report.diagnostics,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return;
    }

    println!("mirsam audit  {}  [{format}]", path.display());
    println!(
        "units {} | arabic {} | mixed {}",
        report.units_scanned, report.arabic_units, report.mixed_units
    );
    println!();
    findings(report);
    verdict(report, strict);
}

/// Everything a repair produced, for rendering.
pub struct Repaired<'a> {
    pub input: &'a Path,
    pub output: &'a Path,
    pub format: &'a str,
    pub options: &'a RepairOptions,
    /// The input's units, so a repair can be reported at a human location
    /// rather than by its opaque id.
    pub units: &'a [TextUnit],
    pub applied: &'a [Repair],
    /// Planned, but not expressible by this adapter.
    pub skipped: &'a [Repair],
    /// The audit of the input.
    pub before: &'a Report,
    /// The audit of the output, re-read from disk.
    pub after: &'a Report,
    pub strict: bool,
}

/// Where a unit is, in the words `audit` uses; its id if it is not known.
fn locate(units: &[TextUnit], id: &UnitId) -> String {
    units
        .iter()
        .find(|unit| &unit.id == id)
        .map_or_else(|| id.to_string(), |unit| unit.location.to_string())
}

/// Repairs grouped under the unit they touched, in the order they were made.
fn repairs(units: &[TextUnit], repairs: &[Repair]) {
    let mut current: Option<&UnitId> = None;
    for repair in repairs {
        if current != Some(&repair.unit) {
            println!("  {}", locate(units, &repair.unit));
            current = Some(&repair.unit);
        }
        println!("    {}", repair.fix);
    }
}

pub fn repair(r: &Repaired<'_>, as_json: bool) {
    if as_json {
        let payload = json!({
            "file": r.input.display().to_string(),
            "output": r.output.display().to_string(),
            "format": r.format,
            "strict": r.strict,
            "blocking": r.after.is_blocking(r.strict),
            "options": r.options,
            "repairs": {
                "applied": r.applied,
                "skipped": r.skipped,
            },
            "before": {
                "summary": summary(r.before),
                "diagnostics": r.before.diagnostics,
            },
            "after": {
                "summary": summary(r.after),
                "diagnostics": r.after.diagnostics,
            },
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return;
    }

    println!(
        "mirsam repair  {} -> {}  [{}]",
        r.input.display(),
        r.output.display(),
        r.format
    );
    println!(
        "units {} | arabic {} | mixed {}",
        r.before.units_scanned, r.before.arabic_units, r.before.mixed_units
    );
    println!(
        "language {} | font {} | convert-bullets {}",
        r.options.language,
        r.options.complex_font.as_deref().unwrap_or("(none)"),
        if r.options.convert_bullets {
            "yes"
        } else {
            "no"
        }
    );
    println!();

    println!("applied {} repair(s)", r.applied.len());
    repairs(r.units, r.applied);
    if !r.skipped.is_empty() {
        println!();
        println!(
            "not applied {} — the {} adapter cannot express these yet; see docs/PLAN.md",
            r.skipped.len(),
            r.format
        );
        repairs(r.units, r.skipped);
    }
    println!();

    println!(
        "before  errors={} warnings={} notes={}",
        r.before.count(Severity::Error),
        r.before.count(Severity::Warning),
        r.before.count(Severity::Note)
    );
    println!(
        "after   errors={} warnings={} notes={}",
        r.after.count(Severity::Error),
        r.after.count(Severity::Warning),
        r.after.count(Severity::Note)
    );
    println!();

    // What remains is what the reader has to act on, so it gets the full
    // treatment; what was fixed is summarised above.
    findings(r.after);
    verdict(r.after, r.strict);
}
