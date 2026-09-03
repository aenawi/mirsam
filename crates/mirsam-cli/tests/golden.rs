//! The golden corpus (PLAN §1.4).
//!
//! Every `.pptx` under `tests/fixtures/` is a corpus deck, and every deck has
//! a committed expected report beside it, `<deck>.expected.json`, recording
//! what the binary does to it: the audit, the repair under the corpus
//! options, which package entries the repair changed and how, and the exit
//! codes a pipeline would branch on. This suite regenerates the report and
//! compares it to the committed one byte for byte, so any change in what
//! `mirsam` reports or writes shows up as a diff against a real document
//! rather than as a passing unit test somewhere else.
//!
//! A diff is a change in behaviour. When it is intended, regenerate the
//! reports and commit them with the change that caused them, so the diff is
//! explained where it is reviewed:
//!
//! ```text
//! MIRSAM_UPDATE_GOLDEN=1 cargo test -p mirsam-cli --test golden    # make golden
//! ```
//!
//! Regeneration refuses to run under CI, so a committed report can only ever
//! have been produced by someone in a position to look at it.
//!
//! Adding a deck is dropping it in `tests/fixtures/` and regenerating. A deck
//! without a report fails, and so does a report without a deck.

use mirsam_ooxml::Package;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_mirsam");
const UPDATE: &str = "MIRSAM_UPDATE_GOLDEN";

/// The repair the corpus records. Both opt-in repairs are on, so every fix
/// the adapter can express is exercised; the options are part of the report,
/// so a reader never has to guess them.
const REPAIR_OPTIONS: &[&str] = &["--convert-bullets", "--font", "Dubai", "--align"];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .canonicalize()
        .expect("tests/fixtures exists")
}

/// Every corpus deck, in name order.
///
/// `*.out.*` is skipped: `.gitignore` reserves that pattern for the output
/// of a manual `repair` run in this directory, and such a file is not a
/// member of the corpus.
fn decks() -> Vec<PathBuf> {
    let mut decks: Vec<PathBuf> = fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "pptx"))
        .filter(|path| !file_name(path).contains(".out."))
        .collect();
    decks.sort();
    decks
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn expected_path(deck: &Path) -> PathBuf {
    deck.with_extension("expected.json")
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Run the binary from the fixtures directory, so a deck is named by its
/// bare file name in the report rather than by a path that depends on the
/// checkout.
fn run(args: &[&str]) -> Run {
    let out = Command::new(BIN)
        .args(args)
        .current_dir(fixtures())
        .output()
        .unwrap_or_else(|e| panic!("could not run {BIN}: {e}"));
    Run {
        code: out.status.code().expect("process was killed by a signal"),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A run that produced a report. Anything else — a refusal, an unreadable
/// deck — is not a corpus result; it is a broken corpus.
fn report_of(run: &Run, what: &str) -> Value {
    assert!(
        run.code <= 1,
        "{what}: exit {} is not a report\nstderr:\n{}",
        run.code,
        run.stderr
    );
    serde_json::from_str(&run.stdout).unwrap_or_else(|e| {
        panic!(
            "{what}: --format json emitted invalid JSON: {e}\nstdout:\n{}\nstderr:\n{}",
            run.stdout, run.stderr
        )
    })
}

/// A scratch directory that removes itself.
///
/// Named uniquely per use, not per deck: two tests observe the same deck on
/// parallel threads, and a shared directory would have one of them removing
/// what the other is about to compare.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        static SERIAL: AtomicUsize = AtomicUsize::new(0);
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "mirsam-golden-{tag}-{}-{serial}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ------------------------------------------------------------------- diffing

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Keep,
    Del,
    Ins,
}

/// A shortest line-level edit script from `old` to `new`.
///
/// Longest-common-subsequence over the region between the common prefix and
/// suffix, which for a report that changed in one place is a few lines. A
/// region too large to table is rendered as a wholesale replacement rather
/// than allowed to exhaust memory.
fn edit_script<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(Op, &'a str)> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let a = &old[prefix..old.len() - suffix];
    let b = &new[prefix..new.len() - suffix];

    let mut script: Vec<(Op, &str)> = old[..prefix].iter().map(|l| (Op::Keep, *l)).collect();

    let (n, m) = (a.len(), b.len());
    if (n + 1) * (m + 1) > 16_000_000 {
        script.extend(a.iter().map(|l| (Op::Del, *l)));
        script.extend(b.iter().map(|l| (Op::Ins, *l)));
    } else {
        let width = m + 1;
        let mut lcs = vec![0u32; (n + 1) * width];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * width + j] = if a[i] == b[j] {
                    lcs[(i + 1) * width + j + 1] + 1
                } else {
                    lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if a[i] == b[j] {
                script.push((Op::Keep, a[i]));
                i += 1;
                j += 1;
            } else if lcs[(i + 1) * width + j] >= lcs[i * width + j + 1] {
                script.push((Op::Del, a[i]));
                i += 1;
            } else {
                script.push((Op::Ins, b[j]));
                j += 1;
            }
        }
        script.extend(a[i..].iter().map(|l| (Op::Del, *l)));
        script.extend(b[j..].iter().map(|l| (Op::Ins, *l)));
    }

    script.extend(old[old.len() - suffix..].iter().map(|l| (Op::Keep, *l)));
    script
}

/// Render an edit script as a unified diff with `context` unchanged lines
/// around each change. With no context, only the changed lines appear and
/// there are no hunk headers.
fn render(script: &[(Op, &str)], context: usize) -> Vec<String> {
    let changes: Vec<usize> = script
        .iter()
        .enumerate()
        .filter(|(_, (op, _))| *op != Op::Keep)
        .map(|(i, _)| i)
        .collect();
    if context == 0 {
        return changes.iter().map(|&i| prefixed(script[i])).collect();
    }

    // Group changes whose context windows touch into one hunk.
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for &i in &changes {
        let (start, end) = (
            i.saturating_sub(context),
            (i + context + 1).min(script.len()),
        );
        match hunks.last_mut() {
            Some((_, last_end)) if start <= *last_end => *last_end = end,
            _ => hunks.push((start, end)),
        }
    }

    let mut out = Vec::new();
    for (start, end) in hunks {
        let old_start = 1 + script[..start]
            .iter()
            .filter(|(op, _)| *op != Op::Ins)
            .count();
        let new_start = 1 + script[..start]
            .iter()
            .filter(|(op, _)| *op != Op::Del)
            .count();
        let old_len = script[start..end]
            .iter()
            .filter(|(op, _)| *op != Op::Ins)
            .count();
        let new_len = script[start..end]
            .iter()
            .filter(|(op, _)| *op != Op::Del)
            .count();
        out.push(format!(
            "@@ -{old_start},{old_len} +{new_start},{new_len} @@"
        ));
        out.extend(script[start..end].iter().map(|entry| prefixed(*entry)));
    }
    out
}

fn prefixed((op, line): (Op, &str)) -> String {
    let sigil = match op {
        Op::Keep => ' ',
        Op::Del => '-',
        Op::Ins => '+',
    };
    format!("{sigil}{line}")
}

fn unified_diff(old: &str, new: &str, context: usize) -> String {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    render(&edit_script(&old, &new), context).join("\n")
}

/// Split XML at every tag, so a part Office wrote on one line diffs by tag
/// rather than as a single line that is entirely different.
fn tags(xml: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, _) in xml.match_indices('<') {
        if i > start {
            out.push(&xml[start..i]);
        }
        start = i;
    }
    if start < xml.len() {
        out.push(&xml[start..]);
    }
    out
}

/// Every entry whose content differs between two packages, with the
/// tag-level diff for a text part. Empty when the repair touched nothing.
fn changed_parts(before: &Path, after: &Path) -> BTreeMap<String, Vec<String>> {
    let before = Package::open(before).unwrap();
    let after = Package::open(after).unwrap();
    let names_before = before.part_names().unwrap();
    let names_after = after.part_names().unwrap();

    let mut changed = BTreeMap::new();
    for name in &names_before {
        if !names_after.contains(name) {
            changed.insert(name.clone(), vec!["<removed>".to_string()]);
            continue;
        }
        let (old, new) = (
            before.read_bytes(name).unwrap(),
            after.read_bytes(name).unwrap(),
        );
        if old == new {
            continue;
        }
        let diff = match (std::str::from_utf8(&old), std::str::from_utf8(&new)) {
            (Ok(old), Ok(new)) => render(&edit_script(&tags(old), &tags(new)), 0),
            _ => vec![format!("<binary: {} -> {} bytes>", old.len(), new.len())],
        };
        changed.insert(name.clone(), diff);
    }
    for name in &names_after {
        if !names_before.contains(name) {
            changed.insert(name.clone(), vec!["<added>".to_string()]);
        }
    }
    changed
}

// ------------------------------------------------------------------ observe

/// What the binary does to one deck, as the committed report records it.
fn observe(deck: &Path) -> Value {
    let name = file_name(deck);
    let stem = deck.file_stem().unwrap().to_string_lossy().into_owned();
    let scratch = Scratch::new(&stem);

    let audit = run(&["audit", &name, "--format", "json"]);
    let audit_strict = run(&["audit", &name, "--strict", "--format", "json"]);

    let output = scratch.path("repaired.pptx");
    let mut args = vec!["repair", &name, &output];
    args.extend_from_slice(REPAIR_OPTIONS);
    args.extend_from_slice(&["--format", "json"]);
    let repair = run(&args);

    let output_strict = scratch.path("repaired-strict.pptx");
    let mut args = vec!["repair", &name, &output_strict];
    args.extend_from_slice(REPAIR_OPTIONS);
    args.extend_from_slice(&["--strict", "--format", "json"]);
    let repair_strict = run(&args);

    let audit_report = report_of(&audit, &format!("{name}: audit"));
    report_of(&audit_strict, &format!("{name}: audit --strict"));
    let mut repair_report = report_of(&repair, &format!("{name}: repair"));
    report_of(&repair_strict, &format!("{name}: repair --strict"));

    // Two repairs of one input are the same file; the corpus would otherwise
    // record whichever it happened to compare.
    assert!(
        fs::read(&output).unwrap() == fs::read(&output_strict).unwrap(),
        "{name}: two repairs with the same options wrote different files"
    );

    // The output path is a scratch location; the report names it stably.
    repair_report["output"] = json!(format!("{stem}.repaired.pptx"));

    json!({
        "deck": name,
        "audit": audit_report,
        "repair": repair_report,
        "changed_parts": changed_parts(deck, Path::new(&output)),
        "exit_codes": {
            "audit": audit.code,
            "audit --strict": audit_strict.code,
            "repair": repair.code,
            "repair --strict": repair_strict.code,
        },
    })
}

/// The committed form: pretty, keys sorted, one trailing newline.
fn canonical(report: &Value) -> String {
    let mut text = serde_json::to_string_pretty(report).unwrap();
    text.push('\n');
    text
}

fn updating() -> bool {
    let requested = std::env::var_os(UPDATE).is_some_and(|v| !v.is_empty() && v != "0");
    if requested && std::env::var_os("CI").is_some() {
        panic!(
            "{UPDATE} is set under CI. Expected reports are regenerated locally, \
             reviewed, and committed with the change that explains them — never by CI."
        );
    }
    requested
}

// -------------------------------------------------------------------- tests

#[test]
fn every_deck_matches_its_expected_report() {
    let decks = decks();
    assert!(!decks.is_empty(), "no decks under {}", fixtures().display());

    let update = updating();
    let mut failures = Vec::new();
    for deck in &decks {
        let name = file_name(deck);
        let expected = expected_path(deck);
        let actual = canonical(&observe(deck));

        if update {
            fs::write(&expected, &actual).unwrap();
            eprintln!("wrote {}", expected.display());
            continue;
        }

        match fs::read_to_string(&expected) {
            Ok(committed) if committed == actual => {}
            Ok(committed) => failures.push(format!(
                "{name}: {} does not match what mirsam now reports.\n\
                 --- expected (committed)\n+++ actual (this build)\n{}",
                file_name(&expected),
                unified_diff(&committed, &actual, 3)
            )),
            Err(_) => failures.push(format!(
                "{name}: no expected report at {}",
                expected.display()
            )),
        }
    }

    assert!(
        failures.is_empty(),
        "\n{}\n\n\
         A diff here is a change in behaviour against a real document. If it is \
         intended, regenerate the expected reports and commit them with the change \
         that explains them:\n\n    {UPDATE}=1 cargo test -p mirsam-cli --test golden    \
         (make golden)\n",
        failures.join("\n\n")
    );
}

#[test]
fn every_expected_report_has_a_deck() {
    let orphans: Vec<String> = fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| file_name(path).ends_with(".expected.json"))
        .filter(|path| {
            let deck = path.with_file_name(file_name(path).replace(".expected.json", ".pptx"));
            !deck.exists()
        })
        .map(|path| file_name(&path))
        .collect();
    assert!(
        orphans.is_empty(),
        "expected reports with no deck: {orphans:?} — restore the deck or delete the report"
    );
}

#[test]
fn the_corpus_includes_a_deck_the_tool_leaves_alone() {
    // PLAN §1.4: "include at least one deck the tool should leave completely
    // alone". Verified against the binary, not against the committed reports:
    // no finding at any severity, nothing applied, nothing skipped, no entry
    // of the package changed by the repair.
    let untouched: Vec<String> = decks()
        .iter()
        .filter(|deck| {
            let report = observe(deck);
            report["audit"]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
                && report["repair"]["repairs"]["applied"]
                    .as_array()
                    .unwrap()
                    .is_empty()
                && report["repair"]["repairs"]["skipped"]
                    .as_array()
                    .unwrap()
                    .is_empty()
                && report["changed_parts"].as_object().unwrap().is_empty()
                && report["exit_codes"]["audit --strict"] == 0
                && report["exit_codes"]["repair --strict"] == 0
        })
        .map(|deck| file_name(deck))
        .collect();
    assert!(
        !untouched.is_empty(),
        "no corpus deck is left completely alone; the corpus must prove that a \
         correct document produces no findings and no edits"
    );
}

#[test]
fn the_corpus_includes_a_deck_with_alternate_content() {
    // ROADMAP M1: "the corpus must include one". A Markup Compatibility block
    // is what a prefix-renaming serialiser breaks.
    let carrying: Vec<String> = decks()
        .iter()
        .filter(|deck| {
            let pkg = Package::open(deck).unwrap();
            pkg.parts_where(|n| n.ends_with(".xml"))
                .unwrap()
                .iter()
                .any(|part| {
                    pkg.read_text(part)
                        .is_ok_and(|xml| xml.contains("mc:AlternateContent"))
                })
        })
        .map(|deck| file_name(deck))
        .collect();
    assert!(
        !carrying.is_empty(),
        "no corpus deck carries mc:AlternateContent"
    );
}

#[test]
fn the_diff_names_exactly_what_changed() {
    // The diff is what a failure shows a reviewer, so it is tested too.
    let old = "a\nb\nc\nd\ne\nf\ng";
    let new = "a\nb\nc\nD\ne\nf\ng";
    assert_eq!(unified_diff(old, new, 1), "@@ -3,3 +3,3 @@\n c\n-d\n+D\n e");
    assert_eq!(unified_diff(old, new, 0), "-d\n+D");
    assert_eq!(unified_diff(old, old, 3), "");

    let xml = r#"<a:p><a:pPr rtl="0"/><a:r><a:t>x</a:t></a:r></a:p>"#;
    let edited = r#"<a:p><a:pPr rtl="1"/><a:r><a:t>x</a:t></a:r></a:p>"#;
    assert_eq!(
        render(&edit_script(&tags(xml), &tags(edited)), 0),
        [r#"-<a:pPr rtl="0"/>"#, r#"+<a:pPr rtl="1"/>"#]
    );
}
