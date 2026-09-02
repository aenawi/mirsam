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
    assert_eq!(rules.len(), 8, "expected the eight documented rules");
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
