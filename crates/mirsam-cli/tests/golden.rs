//! The golden corpus (PLAN §1.4, and its second format §3.5).
//!
//! Every `.pptx` and `.docx` under `tests/fixtures/` is a corpus document, and
//! every one of them has a committed expected report beside it,
//! `<document>.expected.json`, recording what the binary does to it: the
//! audit, the repair under the corpus options, which package entries the
//! repair changed and how, and the exit codes a pipeline would branch on.
//! This suite regenerates the report and compares it to the committed one byte
//! for byte, so any change in what `mirsam` reports or writes shows up as a
//! diff against a real document rather than as a passing unit test somewhere
//! else.
//!
//! ## A format the tool reads but cannot write
//!
//! `repair` refuses a `.docx`, and refuses it as a *readable format without a
//! writer* rather than as an unknown extension. That refusal is recorded here
//! rather than worked around: the report holds the message and the exit code
//! it produced, so the day the Word writer lands, the corpus shows the change
//! as a diff on a real document instead of as a test somebody remembered to
//! update.
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
//! Adding a document is dropping it in `tests/fixtures/` and regenerating. A
//! document without a report fails, and so does a report without a document.

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

/// The extensions the binary reads, and so the extensions that make a file in
/// this directory a member of the corpus.
const CORPUS: &[&str] = &["pptx", "docx"];

/// The same, in the order a sorted, deduplicated list of them comes back — so
/// a test can compare against it directly.
fn corpus_formats() -> Vec<String> {
    let mut formats: Vec<String> = CORPUS.iter().map(|e| e.to_string()).collect();
    formats.sort();
    formats
}

/// Every corpus document, in name order.
///
/// `*.out.*` is skipped: `.gitignore` reserves that pattern for the output
/// of a manual `repair` run in this directory, and such a file is not a
/// member of the corpus.
fn documents() -> Vec<PathBuf> {
    let mut documents: Vec<PathBuf> = fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| extension(path).is_some_and(|e| CORPUS.contains(&e.as_str())))
        .filter(|path| !file_name(path).contains(".out."))
        .collect();
    documents.sort();
    documents
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// A report is named for the document's stem, so two documents in this
/// directory may not share one; `no_two_corpus_documents_share_a_report`
/// holds that.
fn expected_path(document: &Path) -> PathBuf {
    document.with_extension("expected.json")
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Run the binary from the fixtures directory, so a document is named by its
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

/// A run that produced a report. Anything else — an unreadable document — is
/// not a corpus result; it is a broken corpus. A `repair` a readable format
/// with no writer refuses goes to [`refusal`] instead.
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
/// Named uniquely per use, not per document: two tests observe the same
/// document on parallel threads, and a shared directory would have one of them
/// removing what the other is about to compare.
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

/// The refusal the binary printed, as the report records it.
///
/// The whole of stderr less the `error: ` prefix, so the corpus carries the
/// sentence a user reads rather than a paraphrase of it. Recorded for exit
/// `2` alone — an unreadable document is a broken corpus, not a result.
fn refusal(run: &Run, what: &str) -> Value {
    assert_eq!(
        run.code,
        2, // exit::USAGE
        "{what}: exit {} is neither a report nor a refusal\nstderr:\n{}",
        run.code,
        run.stderr
    );
    let message = run
        .stderr
        .trim()
        .strip_prefix("error: ")
        .unwrap_or(run.stderr.trim())
        .to_string();
    assert!(!message.is_empty(), "{what}: refused and said nothing");
    json!({ "refused": message })
}

/// What the binary does to one corpus document, as the committed report
/// records it.
///
/// A format with no writer takes the second branch: the repair is refused, the
/// refusal is what the report holds in place of a repair report, and nothing
/// is compared against an output file that was never written.
fn observe(document: &Path) -> Value {
    let name = file_name(document);
    let stem = document.file_stem().unwrap().to_string_lossy().into_owned();
    let extension = extension(document).expect("a corpus document has an extension");
    let scratch = Scratch::new(&stem);

    let audit = run(&["audit", &name, "--format", "json"]);
    let audit_strict = run(&["audit", &name, "--strict", "--format", "json"]);

    // The repaired copy keeps the input's extension: `repair` refuses any
    // other, because the repaired document is the same format.
    let output = scratch.path(&format!("repaired.{extension}"));
    let mut args = vec!["repair", &name, &output];
    args.extend_from_slice(REPAIR_OPTIONS);
    args.extend_from_slice(&["--format", "json"]);
    let repair = run(&args);

    let output_strict = scratch.path(&format!("repaired-strict.{extension}"));
    let mut args = vec!["repair", &name, &output_strict];
    args.extend_from_slice(REPAIR_OPTIONS);
    args.extend_from_slice(&["--strict", "--format", "json"]);
    let repair_strict = run(&args);

    let audit_report = report_of(&audit, &format!("{name}: audit"));
    report_of(&audit_strict, &format!("{name}: audit --strict"));

    let (repair_report, changed) = if repair.code <= 1 {
        let mut report = report_of(&repair, &format!("{name}: repair"));
        report_of(&repair_strict, &format!("{name}: repair --strict"));

        // Two repairs of one input are the same file; the corpus would
        // otherwise record whichever it happened to compare.
        assert!(
            fs::read(&output).unwrap() == fs::read(&output_strict).unwrap(),
            "{name}: two repairs with the same options wrote different files"
        );

        // The output path is a scratch location; the report names it stably.
        report["output"] = json!(format!("{stem}.repaired.{extension}"));
        let changed = changed_parts(document, Path::new(&output));
        (report, json!(changed))
    } else {
        let refused = refusal(&repair, &format!("{name}: repair"));
        assert_eq!(
            refusal(&repair_strict, &format!("{name}: repair --strict")),
            refused,
            "{name}: --strict changed a refusal that has nothing to do with severity"
        );
        assert!(
            !Path::new(&output).exists() && !Path::new(&output_strict).exists(),
            "{name}: the repair was refused and wrote a file anyway"
        );
        (refused, json!(null))
    };

    json!({
        "document": name,
        "audit": audit_report,
        "repair": repair_report,
        "changed_parts": changed,
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
fn every_document_matches_its_expected_report() {
    let documents = documents();
    assert!(
        !documents.is_empty(),
        "no documents under {}",
        fixtures().display()
    );

    let update = updating();
    let mut failures = Vec::new();
    for document in &documents {
        let name = file_name(document);
        let expected = expected_path(document);
        let actual = canonical(&observe(document));

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
fn every_expected_report_has_a_document() {
    let orphans: Vec<String> = fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| file_name(path).ends_with(".expected.json"))
        .filter(|report| !documents().iter().any(|d| expected_path(d) == *report))
        .map(|path| file_name(&path))
        .collect();
    assert!(
        orphans.is_empty(),
        "expected reports with no document: {orphans:?} — restore the document or \
         delete the report"
    );
}

#[test]
fn no_two_corpus_documents_share_a_report() {
    // A report is named for the document's stem, so `report.pptx` and
    // `report.docx` in this directory would overwrite one another's expected
    // output and the corpus would silently record only one of them.
    let mut seen: BTreeMap<PathBuf, String> = BTreeMap::new();
    for document in documents() {
        let name = file_name(&document);
        if let Some(other) = seen.insert(expected_path(&document), name.clone()) {
            panic!("{name} and {other} would share one expected report; rename one");
        }
    }
}

#[test]
fn the_corpus_holds_a_document_of_every_format_the_binary_reads() {
    // PLAN §3.5: the conformance suite proves the adapters agree on packages a
    // test built. This proves the corpus itself covers both, so a change in
    // what Word documents report shows up as a diff on committed bytes.
    let mut formats: Vec<String> = documents().iter().filter_map(|d| extension(d)).collect();
    formats.sort();
    formats.dedup();
    assert_eq!(
        formats,
        corpus_formats(),
        "the corpus does not hold a document of every readable format"
    );
}

/// Whether the binary left this document completely alone.
///
/// No finding at any severity, and — for a format it can also write — nothing
/// applied, nothing skipped and no entry of the package changed. A format
/// with no writer cannot be asked the second half, and asking it anyway would
/// read a refusal as a clean repair.
fn untouched(report: &Value) -> bool {
    let silent = report["audit"]["diagnostics"]
        .as_array()
        .unwrap()
        .is_empty()
        && report["exit_codes"]["audit --strict"] == 0;
    if report["repair"]["refused"].is_string() {
        return silent;
    }
    silent
        && report["repair"]["repairs"]["applied"]
            .as_array()
            .unwrap()
            .is_empty()
        && report["repair"]["repairs"]["skipped"]
            .as_array()
            .unwrap()
            .is_empty()
        && report["changed_parts"].as_object().unwrap().is_empty()
        && report["exit_codes"]["repair --strict"] == 0
}

#[test]
fn every_format_the_binary_reads_has_a_corpus_document_it_leaves_alone() {
    // PLAN §1.4: "include at least one deck the tool should leave completely
    // alone" — now once per format, because exit code 0 on a correct document
    // is a claim about each adapter separately. Verified against the binary,
    // not against the committed reports.
    let mut proven: Vec<String> = documents()
        .iter()
        .filter(|document| untouched(&observe(document)))
        .filter_map(|document| extension(document))
        .collect();
    proven.sort();
    proven.dedup();
    assert_eq!(
        proven,
        corpus_formats(),
        "a format the binary reads has no corpus document it leaves completely \
         alone; the corpus must prove for each that a correct document produces \
         no findings and no edits"
    );
}

#[test]
fn a_format_with_no_writer_is_refused_as_one_and_writes_nothing() {
    // The refusal is a usage error (exit 2), not an unreadable document (3):
    // the audit above it read the file perfectly well, and saying otherwise
    // would deny knowing a format the tool supports.
    let read_only: Vec<String> = documents()
        .iter()
        .map(|document| observe(document))
        .filter(|report| report["repair"]["refused"].is_string())
        .map(|report| {
            assert_eq!(report["exit_codes"]["repair"], 2, "{report:#?}");
            assert!(
                report["exit_codes"]["audit"].as_i64().unwrap() <= 1,
                "{report:#?}"
            );
            assert!(report["changed_parts"].is_null(), "{report:#?}");
            report["document"].as_str().unwrap().to_string()
        })
        .collect();
    assert!(
        !read_only.is_empty(),
        "no corpus document exercises the refusal `repair` gives a readable \
         format with no writer; when the last writer lands, delete this test \
         rather than weakening it"
    );
}

#[test]
fn the_corpus_includes_a_document_with_alternate_content() {
    // ROADMAP M1: "the corpus must include one". A Markup Compatibility block
    // is what a prefix-renaming serialiser breaks.
    let carrying: Vec<String> = documents()
        .iter()
        .filter(|document| {
            let pkg = Package::open(document).unwrap();
            pkg.parts_where(|n| n.ends_with(".xml"))
                .unwrap()
                .iter()
                .any(|part| {
                    pkg.read_text(part)
                        .is_ok_and(|xml| xml.contains("mc:AlternateContent"))
                })
        })
        .map(|document| file_name(document))
        .collect();
    assert!(
        !carrying.is_empty(),
        "no corpus document carries mc:AlternateContent"
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
