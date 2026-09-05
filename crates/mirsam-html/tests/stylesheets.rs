//! Linked stylesheets: the ones that can be read, and the honest report of
//! the ones that cannot (PLAN §5.1).
//!
//! A stylesheet the tool did not read is a stylesheet nobody applied, and a
//! `direction-unset` finding on such a document may be answered by CSS that
//! was never seen. That is a `NOT RUN`, and it has to be sayable — which is
//! what [`HtmlDocument::unread_stylesheets`] is for.

use mirsam_core::{Direction, DocumentReader};
use mirsam_html::HtmlDocument;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const ARABIC: &str = "ارتفع الأداء في الربع الرابع";

/// A directory of this test's own, removed by the operating system rather
/// than by a `Drop` that a panicking test would skip.
fn workspace(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mirsam-html-{}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        name
    ));
    fs::create_dir_all(&dir).expect("a temporary directory");
    dir
}

fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("a directory for the file");
    }
    fs::write(&path, contents).expect("writing a fixture");
    path
}

#[test]
fn a_relative_stylesheet_beside_the_document_is_read() {
    let dir = workspace("relative");
    write(&dir, "site.css", "body { direction: rtl }");
    let page = write(
        &dir,
        "report.html",
        &format!(r#"<link rel="stylesheet" href="site.css"><body><p>{ARABIC}</p></body>"#),
    );

    let mut document = HtmlDocument::open(&page).expect("the document opens");
    let units = document.scan().expect("it scans");
    assert_eq!(
        units[0].props.direction.effective(),
        Some(&Direction::Rtl),
        "the linked sheet states the direction"
    );
    assert!(document.unread_stylesheets().0.is_empty());
}

#[test]
fn a_stylesheet_in_a_subdirectory_resolves_against_the_document() {
    let dir = workspace("subdir");
    write(&dir, "css/site.css", "p { direction: rtl }");
    let page = write(
        &dir,
        "report.html",
        &format!(r#"<link rel="stylesheet" href="css/site.css"><p>{ARABIC}</p>"#),
    );

    let mut document = HtmlDocument::open(&page).expect("the document opens");
    let units = document.scan().expect("it scans");
    assert_eq!(units[0].props.direction.effective(), Some(&Direction::Rtl));
}

#[test]
fn a_stylesheet_on_the_network_is_not_fetched_and_is_named() {
    let dir = workspace("remote");
    let page = write(
        &dir,
        "report.html",
        &format!(r#"<link rel="stylesheet" href="https://example.test/site.css"><p>{ARABIC}</p>"#),
    );

    let mut document = HtmlDocument::open(&page).expect("the document opens");
    let units = document.scan().expect("it scans");
    assert!(units[0].props.direction.is_unset());
    assert_eq!(
        document.unread_stylesheets().0,
        ["https://example.test/site.css"],
        "the audit says what it did not read"
    );
}

#[test]
fn a_root_relative_stylesheet_names_a_root_nobody_stated() {
    let dir = workspace("root-relative");
    let page = write(
        &dir,
        "report.html",
        &format!(r#"<link rel="stylesheet" href="/css/site.css"><p>{ARABIC}</p>"#),
    );

    let mut document = HtmlDocument::open(&page).expect("the document opens");
    document.scan().expect("it scans");
    assert_eq!(document.unread_stylesheets().0, ["/css/site.css"]);
}

#[test]
fn a_missing_stylesheet_is_reported_rather_than_failing_the_audit() {
    let dir = workspace("missing");
    let page = write(
        &dir,
        "report.html",
        &format!(r#"<link rel="stylesheet" href="gone.css"><p>{ARABIC}</p>"#),
    );

    let mut document = HtmlDocument::open(&page).expect("the document opens");
    document
        .scan()
        .expect("a missing stylesheet is not a broken document");
    assert_eq!(document.unread_stylesheets().0, ["gone.css"]);
}

#[test]
fn a_link_that_is_not_a_stylesheet_is_not_a_stylesheet() {
    let dir = workspace("icon");
    let page = write(
        &dir,
        "report.html",
        &format!(r#"<link rel="icon" href="favicon.ico"><p>{ARABIC}</p>"#),
    );

    let mut document = HtmlDocument::open(&page).expect("the document opens");
    document.scan().expect("it scans");
    assert!(document.unread_stylesheets().0.is_empty());
}

#[test]
fn a_document_held_in_memory_can_resolve_nothing_relative() {
    let mut document = HtmlDocument::from_source(
        "page.html",
        r#"<link rel="stylesheet" href="site.css"><p>text</p>"#,
    );
    document.scan().expect("it scans");
    assert_eq!(
        document.unread_stylesheets().0,
        ["site.css"],
        "there is no directory to resolve it against, and saying so is the answer"
    );
}

#[test]
fn a_document_that_is_not_utf8_is_refused_rather_than_guessed_at() {
    let dir = workspace("encoding");
    let page = dir.join("report.html");
    // Windows-1256, the encoding an Arabic document is most often mis-saved
    // in. Lossy decoding would turn these bytes into replacement characters
    // and then report on them.
    fs::write(&page, [b'<', b'p', b'>', 0xE3, 0xD1, 0xCD, 0xC8]).expect("writing bytes");

    let error = HtmlDocument::open(&page).expect_err("it is refused");
    assert!(
        error.to_string().contains("UTF-8"),
        "the refusal says what is wrong: {error}"
    );
}

#[test]
fn a_document_that_is_not_there_says_so() {
    let dir = workspace("absent");
    let error = HtmlDocument::open(&dir.join("nothing.html")).expect_err("it is refused");
    assert!(matches!(error, mirsam_core::error::Error::NotFound));
}
