//! Output rendering. Text for humans, JSON for agents.
//!
//! Both are first-class: an agent reading `--format json` should never have to
//! scrape the human output, and a human should never be handed raw JSON.

use mirsam_core::{Direction, Engine, Report, Severity, bidi};
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

pub fn report(path: &Path, format: &str, report: &Report, strict: bool, as_json: bool) {
    if as_json {
        let payload = json!({
            "file": path.display().to_string(),
            "format": format,
            "strict": strict,
            "blocking": report.is_blocking(strict),
            "summary": {
                "units_scanned": report.units_scanned,
                "arabic_units": report.arabic_units,
                "mixed_units": report.mixed_units,
                "errors": report.count(Severity::Error),
                "warnings": report.count(Severity::Warning),
                "notes": report.count(Severity::Note),
            },
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

    let blocking = report.is_blocking(strict);
    println!(
        "{}: errors={} warnings={} notes={} strict={}",
        if blocking { "FAIL" } else { "PASS" },
        report.count(Severity::Error),
        report.count(Severity::Warning),
        report.count(Severity::Note),
        if strict { "yes" } else { "no" }
    );
}
