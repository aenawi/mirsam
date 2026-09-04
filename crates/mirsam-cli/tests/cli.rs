//! The exit-code contract.
//!
//! `README.md` and `AGENTS.md` both tell callers to branch on the exit code and
//! never to parse the human output. That makes these four numbers a public API,
//! and public APIs need tests: the codes were previously decided by searching
//! the rendered error message for a substring, so `mirsam audit notes.md`
//! returned 3 ("document unreadable") where the documentation promised 2 ("bad
//! invocation") — a silent breach no unit test could see, because the defect
//! lived in the wiring between `run` and `main`.

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_mirsam");

/// The documented codes, restated here so a change to `main.rs` has to be a
/// deliberate change to this contract as well.
mod exit {
    pub const OK: i32 = 0;
    pub const FINDINGS: i32 = 1;
    pub const USAGE: i32 = 2;
    pub const UNREADABLE: i32 = 3;
}

struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Output {
    let out = Command::new(BIN)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("could not run {BIN}: {e}"));
    Output {
        code: out.status.code().expect("process was killed by a signal"),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("mirsam-cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn file(&self, name: &str, contents: &[u8]) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, contents).unwrap();
        path.to_string_lossy().into_owned()
    }
    /// A path inside the directory, not yet created.
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
    /// A private copy of a fixture, for tests that must not touch the original.
    fn copy_of(&self, fixture_name: &str) -> String {
        let path = self.0.join(fixture_name);
        std::fs::copy(fixture(fixture_name), &path).unwrap();
        path.to_string_lossy().into_owned()
    }
}

fn parse_json(out: &Output) -> serde_json::Value {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "--format json emitted invalid JSON: {e}\nstdout:\n{}\nstderr:\n{}",
            out.stdout, out.stderr
        )
    })
}

fn rule_ids(diagnostics: &serde_json::Value) -> Vec<String> {
    diagnostics
        .as_array()
        .expect("a diagnostics array")
        .iter()
        .map(|d| d["rule"].as_str().unwrap().to_string())
        .collect()
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ------------------------------------------------------------------ exit 0

#[test]
fn a_correctly_marked_deck_exits_zero() {
    let out = run(&["audit", &fixture("clean.pptx")]);
    assert_eq!(out.code, exit::OK, "stdout:\n{}", out.stdout);
    assert!(out.stdout.contains("PASS"), "stdout:\n{}", out.stdout);
}

#[test]
fn a_correctly_marked_deck_exits_zero_under_strict() {
    // --strict promotes warnings to blocking. A deck with nothing to report
    // must survive it, or `--strict` is unusable in CI.
    let out = run(&["audit", &fixture("clean.pptx"), "--strict"]);
    assert_eq!(out.code, exit::OK, "stdout:\n{}", out.stdout);
}

#[test]
fn rules_and_explain_exit_zero() {
    assert_eq!(run(&["rules"]).code, exit::OK);
    assert_eq!(run(&["explain", "GPS يعتمد عليه النظام"]).code, exit::OK);
}

// ------------------------------------------------------------------ exit 1

#[test]
fn a_deck_with_findings_exits_one() {
    let out = run(&["audit", &fixture("torture.pptx")]);
    assert_eq!(out.code, exit::FINDINGS, "stdout:\n{}", out.stdout);
    assert!(out.stdout.contains("FAIL"), "stdout:\n{}", out.stdout);
}

// ------------------------------------------------------------------ exit 2

#[test]
fn an_unsupported_extension_is_a_usage_error_not_an_unreadable_document() {
    // The regression this suite exists for. A format mirsam does not read yet
    // is the caller's mistake, not a broken file: 2, never 3.
    let scratch = Scratch::new("ext");
    let path = scratch.file("notes.md", b"# not a deck");

    let out = run(&["audit", &path]);
    assert_eq!(
        out.code,
        exit::USAGE,
        "expected USAGE for an unsupported extension\nstderr:\n{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("md"),
        "the error should name the extension:\n{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("ROADMAP"),
        "the error should point at the roadmap:\n{}",
        out.stderr
    );
}

#[test]
fn a_file_that_merely_looks_unreadable_is_still_classified_by_type() {
    // Guards against classifying by substring again: this document's own text
    // contains the words a message-matching implementation would key on, yet it
    // is a genuinely unreadable .pptx and must be 3.
    let scratch = Scratch::new("decoy");
    let path = scratch.file("decoy.pptx", b"no adapter for extension");

    assert_eq!(run(&["audit", &path]).code, exit::UNREADABLE);
}

#[test]
fn a_missing_subcommand_or_argument_is_a_usage_error() {
    assert_eq!(run(&[]).code, exit::USAGE);
    assert_eq!(run(&["audit"]).code, exit::USAGE);
    assert_eq!(run(&["nosuchcommand"]).code, exit::USAGE);
}

// ------------------------------------------------------------------ exit 3

#[test]
fn a_missing_file_is_unreadable() {
    let out = run(&["audit", "/nonexistent/deck.pptx"]);
    assert_eq!(out.code, exit::UNREADABLE, "stderr:\n{}", out.stderr);
    assert!(
        out.stderr.contains("no such file"),
        "stderr:\n{}",
        out.stderr
    );
    // The path belongs in the context, once.
    assert_eq!(
        out.stderr.matches("no such file").count(),
        1,
        "the message stutters:\n{}",
        out.stderr
    );
}

#[test]
fn a_file_that_is_not_a_package_is_unreadable() {
    let scratch = Scratch::new("notzip");
    let path = scratch.file("broken.pptx", b"this is not a ZIP archive");
    assert_eq!(run(&["audit", &path]).code, exit::UNREADABLE);
}

// ------------------------------------------------------------------- output

#[test]
fn json_output_is_machine_readable() {
    let out = run(&["audit", &fixture("torture.pptx"), "--format", "json"]);
    assert_eq!(out.code, exit::FINDINGS);

    let value: serde_json::Value = serde_json::from_str(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "audit --format json emitted invalid JSON: {e}\n{}",
            out.stdout
        )
    });

    for key in [
        "file",
        "format",
        "strict",
        "blocking",
        "summary",
        "diagnostics",
    ] {
        assert!(value.get(key).is_some(), "missing key {key:?} in {value}");
    }
    assert_eq!(value["format"], "pptx");
    assert_eq!(value["blocking"], true);

    let diagnostics = value["diagnostics"].as_array().expect("diagnostics array");
    assert!(!diagnostics.is_empty(), "the torture deck has findings");
    for d in diagnostics {
        assert!(d.get("rule").is_some(), "a finding with no rule id: {d}");
        assert!(
            d.get("evidence").is_some(),
            "a finding a reviewer cannot verify: {d}"
        );
    }
}

#[test]
fn rules_json_lists_every_rule_with_an_id() {
    let out = run(&["rules", "--format", "json"]);
    assert_eq!(out.code, exit::OK);
    let value: serde_json::Value = serde_json::from_str(&out.stdout).expect("valid JSON");
    let rules = value.as_array().expect("an array of rules");
    assert_eq!(rules.len(), 10, "expected the ten documented rules");
    for rule in rules {
        assert!(rule.get("id").is_some(), "a rule with no id: {rule}");
    }
}

#[test]
fn nothing_is_written_to_stdout_on_failure() {
    // A pipeline reading stdout must not receive a half-report when the
    // document could not be read at all.
    let out = run(&["audit", "/nonexistent/deck.pptx"]);
    assert!(
        out.stdout.is_empty(),
        "stdout was not empty:\n{}",
        out.stdout
    );
}

// ------------------------------------------------------------------- repair

#[test]
fn repairing_the_m0_fixture_clears_every_fixable_finding_and_is_then_a_no_op() {
    // PLAN M1 1.3 acceptance, through the binary.
    let scratch = Scratch::new("repair");
    let once = scratch.path("once.pptx");

    let out = run(&[
        "repair",
        &fixture("broken-arabic.pptx"),
        &once,
        "--convert-bullets",
        "--align",
        "--format",
        "json",
    ]);
    assert_eq!(
        out.code,
        exit::OK,
        "stdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
    let value = parse_json(&out);
    assert_eq!(value["format"], "pptx");
    assert_eq!(value["blocking"], false);
    assert!(
        value["before"]["summary"]["errors"].as_u64().unwrap() > 0,
        "the fixture must start with errors: {}",
        value["before"]
    );
    assert!(
        !value["repairs"]["applied"].as_array().unwrap().is_empty(),
        "nothing was applied"
    );
    assert!(
        value["repairs"]["skipped"].as_array().unwrap().is_empty(),
        "the adapter skipped repairs it should express: {}",
        value["repairs"]["skipped"]
    );

    let after = value["after"]["diagnostics"].as_array().unwrap();
    let fixable_left: Vec<_> = after.iter().filter(|d| d["fixable"] == true).collect();
    assert!(
        fixable_left.is_empty(),
        "fixable findings survived the repair: {fixable_left:#?}"
    );
    assert!(after.is_empty(), "findings remain: {after:#?}");

    // The written file audits clean on its own, through the ordinary path.
    assert_eq!(run(&["audit", &once, "--strict"]).code, exit::OK);

    // Repairing the repaired deck finds nothing to do and reproduces it byte
    // for byte.
    let twice = scratch.path("twice.pptx");
    let again = run(&[
        "repair",
        &once,
        &twice,
        "--convert-bullets",
        "--align",
        "--format",
        "json",
    ]);
    assert_eq!(again.code, exit::OK, "stderr:\n{}", again.stderr);
    let value = parse_json(&again);
    assert!(
        value["repairs"]["applied"].as_array().unwrap().is_empty(),
        "a second repair still had work: {}",
        value["repairs"]
    );
    assert!(
        std::fs::read(&once).unwrap() == std::fs::read(&twice).unwrap(),
        "a second repair changed the bytes"
    );
}

#[test]
fn repair_align_writes_the_start_edge_only_when_asked() {
    // `quarterly-report.pptx` sits on an English master whose `bodyStyle` and
    // `otherStyle` say `algn="l"`, so its Arabic paragraphs are left on the
    // edge a reader does not start from and state no alignment of their own.
    // Without the flag that is a note — reported, never blocking, never
    // written. With it, the start edge is written and the note is gone.
    //
    // Not `broken-arabic.pptx`, which since M2 has nothing to report here: it
    // sits on an `algn="r"` master, and a paragraph that inherits a coherent
    // alignment is the layout doing its job (ADR 0007 §4).
    let scratch = Scratch::new("align");

    let out = run(&[
        "repair",
        &fixture("quarterly-report.pptx"),
        &scratch.path("plain.pptx"),
        "--convert-bullets",
        // Everything else this deck can be repaired for, so that what remains
        // under `--strict` below is notes and nothing else.
        "--font",
        "Dubai",
        "--format",
        "json",
    ]);
    assert_eq!(out.code, exit::OK, "stderr:\n{}", out.stderr);
    let value = parse_json(&out);
    assert_eq!(value["options"]["align"], false);
    let notes: Vec<_> = value["after"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["rule"] == "alignment-unset")
        .collect();
    assert!(
        !notes.is_empty(),
        "the fixture must carry paragraphs with no alignment of their own"
    );
    assert!(
        notes
            .iter()
            .all(|d| d["severity"] == "note" && d["fixable"] == false),
        "{notes:#?}"
    );
    // Scoped to the units this rule is about: the same deck carries
    // `alignment-incoherent` findings, whose repair writes an alignment
    // unconditionally and always has.
    let noted: Vec<&str> = notes.iter().map(|d| d["unit"].as_str().unwrap()).collect();
    assert!(
        !value["repairs"]["applied"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["fix"]["kind"] == "set_alignment"
                && noted.contains(&r["unit"].as_str().unwrap())),
        "an alignment was written on a noted paragraph without --align: {}",
        value["repairs"]["applied"]
    );
    // A note never blocks, strict or not.
    assert_eq!(
        run(&["audit", &scratch.path("plain.pptx"), "--strict"]).code,
        exit::OK
    );

    let out = run(&[
        "repair",
        &fixture("quarterly-report.pptx"),
        &scratch.path("aligned.pptx"),
        "--convert-bullets",
        "--font",
        "Dubai",
        "--align",
        "--format",
        "json",
    ]);
    assert_eq!(out.code, exit::OK, "stderr:\n{}", out.stderr);
    let value = parse_json(&out);
    assert_eq!(value["options"]["align"], true);
    assert!(
        value["repairs"]["applied"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["fix"]["kind"] == "set_alignment"),
        "{}",
        value["repairs"]
    );
    assert!(
        value["after"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["rule"] != "alignment-unset"),
        "{}",
        value["after"]
    );
}

#[test]
fn repair_never_modifies_its_input() {
    let scratch = Scratch::new("input");
    let input = scratch.copy_of("broken-arabic.pptx");
    let before = std::fs::read(&input).unwrap();

    let out = run(&[
        "repair",
        &input,
        &scratch.path("out.pptx"),
        "--convert-bullets",
    ]);
    assert_eq!(out.code, exit::OK, "stderr:\n{}", out.stderr);
    assert_eq!(std::fs::read(&input).unwrap(), before, "the input changed");
}

#[test]
fn repair_leaves_a_typed_bullet_alone_unless_asked() {
    // Converting a bullet edits the text itself, so it is opt-in. Without the
    // flag the finding must remain — reported, not silently dropped. `--align`
    // is given so the alignment notes on the same paragraphs do not crowd
    // the assertion; that flag has its own test.
    let scratch = Scratch::new("bullet");
    let out = run(&[
        "repair",
        &fixture("broken-arabic.pptx"),
        &scratch.path("out.pptx"),
        "--align",
        "--format",
        "json",
    ]);
    assert_eq!(out.code, exit::OK, "stderr:\n{}", out.stderr);
    let value = parse_json(&out);

    assert_eq!(rule_ids(&value["after"]["diagnostics"]), ["literal-bullet"]);
    assert!(
        !value["repairs"]["applied"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["fix"]["kind"] == "convert_literal_bullet"),
        "{}",
        value["repairs"]
    );
    assert_eq!(value["options"]["convert_bullets"], false);
}

#[test]
fn repair_writes_the_requested_language_tag() {
    let scratch = Scratch::new("lang");
    let out = run(&[
        "repair",
        &fixture("broken-arabic.pptx"),
        &scratch.path("out.pptx"),
        "--lang",
        "ar-AE",
        "--format",
        "json",
    ]);
    assert_eq!(out.code, exit::OK, "stderr:\n{}", out.stderr);
    let value = parse_json(&out);

    let languages: Vec<_> = value["repairs"]["applied"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["fix"]["kind"] == "set_language")
        .map(|r| r["fix"]["value"].clone())
        .collect();
    assert!(!languages.is_empty(), "{}", value["repairs"]);
    assert!(languages.iter().all(|l| l == "ar-AE"), "{languages:?}");
    assert!(
        !rule_ids(&value["after"]["diagnostics"]).contains(&"language-missing".to_string()),
        "{}",
        value["after"]
    );
}

#[test]
fn repair_rejects_a_language_tag_the_rule_would_still_report() {
    // Writing `en-US` and then reporting it as missing would make the second
    // pass not a no-op, and the first pass a lie. The caller's mistake: 2.
    let scratch = Scratch::new("badlang");
    let output = scratch.path("out.pptx");
    let out = run(&[
        "repair",
        &fixture("broken-arabic.pptx"),
        &output,
        "--lang",
        "en-US",
    ]);
    assert_eq!(out.code, exit::USAGE, "stderr:\n{}", out.stderr);
    assert!(out.stderr.contains("en-US"), "{}", out.stderr);
    assert!(
        !Path::new(&output).exists(),
        "a refused repair wrote output"
    );
}

#[test]
fn repair_refuses_to_overwrite_its_source_even_when_forced() {
    let scratch = Scratch::new("source");
    let deck = scratch.copy_of("broken-arabic.pptx");
    let before = std::fs::read(&deck).unwrap();

    for args in [
        vec!["repair", &deck, &deck],
        vec!["repair", &deck, &deck, "--force"],
    ] {
        let out = run(&args);
        assert_eq!(out.code, exit::USAGE, "{args:?}\nstderr:\n{}", out.stderr);
        assert!(
            out.stderr.contains("overwrite the source"),
            "{args:?}: the error should say why:\n{}",
            out.stderr
        );
        assert_eq!(
            std::fs::read(&deck).unwrap(),
            before,
            "{args:?}: the source changed"
        );
    }
}

#[test]
fn repair_refuses_an_existing_output_unless_forced() {
    let scratch = Scratch::new("exists");
    let output = scratch.file("out.pptx", b"precious");

    let out = run(&["repair", &fixture("broken-arabic.pptx"), &output]);
    assert_eq!(out.code, exit::USAGE, "stderr:\n{}", out.stderr);
    assert!(out.stderr.contains("--force"), "{}", out.stderr);
    assert_eq!(
        std::fs::read(&output).unwrap(),
        b"precious",
        "the existing file was replaced without --force"
    );

    let out = run(&["repair", &fixture("broken-arabic.pptx"), &output, "--force"]);
    assert_eq!(out.code, exit::OK, "stderr:\n{}", out.stderr);
    assert_eq!(run(&["audit", &output]).code, exit::OK);
}

#[test]
fn repair_refuses_an_output_of_a_different_extension() {
    // The repaired document is the same format as its input; a `.docx` name
    // on a PPTX package would mislead the next reader — including this tool.
    let scratch = Scratch::new("ext");
    let output = scratch.path("out.docx");
    let out = run(&["repair", &fixture("broken-arabic.pptx"), &output]);
    assert_eq!(out.code, exit::USAGE, "stderr:\n{}", out.stderr);
    assert!(!Path::new(&output).exists());
}

#[test]
fn repair_follows_the_audit_exit_codes_for_what_remains() {
    // Without --convert-bullets the torture deck keeps one warning after
    // repair: not blocking by default, blocking under --strict.
    let scratch = Scratch::new("codes");
    let lenient = scratch.path("lenient.pptx");
    let out = run(&["repair", &fixture("torture.pptx"), &lenient]);
    assert_eq!(
        out.code,
        exit::OK,
        "stdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );

    let strict = scratch.path("strict.pptx");
    let out = run(&["repair", &fixture("torture.pptx"), &strict, "--strict"]);
    assert_eq!(out.code, exit::FINDINGS, "stdout:\n{}", out.stdout);
    assert!(out.stdout.contains("literal-bullet"), "{}", out.stdout);
    assert!(
        Path::new(&strict).exists(),
        "a blocking after-audit is a verdict on the output, not a reason to withhold it"
    );
}

#[test]
fn repair_usage_and_unreadable_errors_match_audit() {
    let scratch = Scratch::new("errors");

    let output = scratch.path("out.md");
    let out = run(&[
        "repair",
        &scratch.file("notes.md", b"# not a deck"),
        &output,
    ]);
    assert_eq!(out.code, exit::USAGE, "stderr:\n{}", out.stderr);
    assert!(!Path::new(&output).exists());

    let output = scratch.path("out.pptx");
    let out = run(&["repair", "/nonexistent/deck.pptx", &output]);
    assert_eq!(out.code, exit::UNREADABLE, "stderr:\n{}", out.stderr);
    assert!(out.stderr.contains("no such file"), "{}", out.stderr);
    assert!(!Path::new(&output).exists());
    assert!(
        out.stdout.is_empty(),
        "stdout was not empty:\n{}",
        out.stdout
    );

    assert_eq!(
        run(&["repair", &fixture("broken-arabic.pptx")]).code,
        exit::USAGE
    );
}

#[test]
fn repair_text_output_reports_both_audits_and_what_changed() {
    let scratch = Scratch::new("text");
    let out = run(&[
        "repair",
        &fixture("broken-arabic.pptx"),
        &scratch.path("out.pptx"),
        "--convert-bullets",
    ]);
    assert_eq!(out.code, exit::OK, "stderr:\n{}", out.stderr);
    for needle in [
        "mirsam repair",
        // Five, not the seven before M2: the two title paragraphs inherit
        // `rtl="1" algn="r"` from this deck's master and need no direction or
        // alignment written on them.
        "applied 5 repair(s)",
        "ppt/slides/slide1.xml:paragraph-2:Title 1",
        "remove 1 explicit bidi control(s)",
        "set direction rtl",
        "set language ar-SA",
        "convert typed '•' to a native bullet",
        // The two-column body is repaired beside the paragraphs, and is
        // named by its shape rather than by a paragraph number.
        "ppt/slides/slide1.xml:Columns 2",
        "before  errors=1 warnings=4",
        "after   errors=0 warnings=0",
        "PASS",
    ] {
        assert!(
            out.stdout.contains(needle),
            "missing {needle:?} in:\n{}",
            out.stdout
        );
    }
    assert!(!out.stdout.contains("not applied"), "{}", out.stdout);
}
