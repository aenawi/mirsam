//! The refusal, and the sentence it gives.
//!
//! mirsam does not decrypt. What is being tested is that it says *what* it did
//! not decrypt: an encrypted document produces no text, and a reader that
//! stopped there would report a file with no defects.

use mirsam_pdf::Pdf;
use mirsam_pdf::encrypt::describe;
use mirsam_pdf::lexer::Lexer;

fn dict(source: &str) -> mirsam_pdf::Dictionary {
    Lexer::new(source.as_bytes())
        .object()
        .expect("parse")
        .as_dict()
        .expect("dictionary")
        .clone()
}

#[test]
fn the_algorithms_a_standard_handler_states() {
    assert_eq!(
        describe(&dict("<< /Filter /Standard /V 1 /R 2 >>")),
        "RC4 40-bit (/Standard security handler, V1 R2)"
    );
    assert_eq!(
        describe(&dict("<< /Filter /Standard /V 2 /R 3 /Length 128 >>")),
        "RC4 128-bit (/Standard security handler, V2 R3)"
    );
    assert_eq!(
        describe(&dict(
            "<< /Filter /Standard /V 4 /R 4 /CF << /StdCF << /CFM /AESV2 >> >> /StmF /StdCF >>"
        )),
        "AES-128 (/Standard security handler, V4 R4)"
    );
    assert_eq!(
        describe(&dict(
            "<< /Filter /Standard /V 5 /R 6 /CF << /StdCF << /CFM /AESV3 >> >> /StmF /StdCF >>"
        )),
        "AES-256 (/Standard security handler, V5 R6)"
    );
}

/// `/StmF` names the filter streams actually use. A file whose stream filter
/// is not the one conventionally called `StdCF` would otherwise be described
/// by a dictionary entry nothing in it uses.
#[test]
fn the_filter_streams_use_is_the_one_reported() {
    assert_eq!(
        describe(&dict(
            "<< /Filter /Standard /V 4 /R 4 /StmF /Custom \
             /CF << /StdCF << /CFM /AESV2 >> /Custom << /CFM /V2 /Length 16 >> >> >>"
        )),
        "RC4 128-bit (/Standard security handler, V4 R4)"
    );
}

/// A third-party handler is named as itself. Claiming an algorithm it never
/// stated would be exactly the inference standing rule 4 forbids.
#[test]
fn a_handler_this_tool_does_not_know_is_named_rather_than_guessed_at() {
    let answer = describe(&dict("<< /Filter /FOPN_foweb /V 4 /R 4 >>"));
    assert!(answer.contains("/FOPN_foweb security handler"), "{answer}");
}

#[test]
fn an_encryption_that_states_nothing_says_so() {
    let answer = describe(&dict("<< >>"));
    assert_eq!(
        answer,
        "an undeclared algorithm (/Standard security handler)"
    );
}

/// The `/Encrypt` dictionary is refused whether the trailer holds it inline or
/// points at it — §7.6.1 keeps it unencrypted precisely so a reader can find
/// out what it is up against.
#[test]
fn an_inline_encrypt_dictionary_is_refused_too() {
    let mut out = b"%PDF-1.4\n".to_vec();
    let object = out.len();
    out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog >>\nendobj\n");
    let table = out.len();
    out.extend_from_slice(b"xref\n0 2\n0000000000 65535 f \n");
    out.extend_from_slice(format!("{object:010} 00000 n \n").as_bytes());
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size 2 /Root 1 0 R /Encrypt << /Filter /Standard /V 2 /R 3 \
             /Length 128 >> >>\nstartxref\n{table}\n%%EOF\n"
        )
        .as_bytes(),
    );

    let error = Pdf::from_bytes(&out, "test").unwrap_err().to_string();
    assert!(error.contains("RC4 128-bit"), "{error}");
    assert!(error.contains("mirsam does not decrypt"), "{error}");
}
