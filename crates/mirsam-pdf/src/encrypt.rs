//! Encryption: what it is, said precisely, so the document can be refused.
//!
//! mirsam does not decrypt. That is not a gap to be filled later — a tool that
//! opened protected documents by trying the empty password would be a tool
//! nobody could run on a file they were given, and one that asked for a
//! password would have to hold it.
//!
//! What matters is what the refusal *says*. An encrypted PDF whose strings and
//! streams cannot be read produces no text, and a reader that stopped there
//! would report a document with no defects — the exact failure ADR 0009 exists
//! to prevent, arriving as a whole file rather than one source inside it. So
//! the refusal names the algorithm, from the `/Encrypt` dictionary alone, which
//! is never itself encrypted (§7.6.1). A person reading it knows whether they
//! are looking at a permissions-only wrapper they can strip or a document they
//! need a password for, without opening anything.

use crate::object::{Dictionary, Object};

/// The `/Encrypt` dictionary in one line: the algorithm, the handler and the
/// version the file states.
///
/// Nothing here is inferred. Where the dictionary does not say, the answer says
/// so — an undeclared algorithm is reported as undeclared, never guessed at,
/// because the point of the message is that a reader can trust it.
pub fn describe(encrypt: &Dictionary) -> String {
    let handler = encrypt
        .get("Filter")
        .and_then(Object::as_name)
        .map(ToString::to_string)
        .unwrap_or_else(|| "/Standard".into());
    let v = encrypt.get("V").and_then(Object::as_i64);
    let r = encrypt.get("R").and_then(Object::as_i64);
    let bits = encrypt.get("Length").and_then(Object::as_i64).unwrap_or(40);

    let algorithm = match v {
        Some(1) => "RC4 40-bit".to_string(),
        Some(2) | Some(3) => format!("RC4 {bits}-bit"),
        Some(4) | Some(5) => crypt_filter(encrypt).unwrap_or_else(|| match v {
            Some(5) => "AES-256".into(),
            _ => format!("RC4 {bits}-bit"),
        }),
        Some(0) => "an algorithm the document does not state".to_string(),
        _ => "an undeclared algorithm".to_string(),
    };

    let version = match (v, r) {
        (Some(v), Some(r)) => format!(", V{v} R{r}"),
        (Some(v), None) => format!(", V{v}"),
        (None, Some(r)) => format!(", R{r}"),
        (None, None) => String::new(),
    };
    format!("{algorithm} ({handler} security handler{version})")
}

/// §7.6.5's crypt filters, where PDF 1.5 and later actually state the method.
///
/// `/StdCF` is the name every writer uses, but the *default* filter for streams
/// is whatever `/StmF` names, so that is asked first: a file whose stream
/// filter is not the one called `StdCF` would otherwise be described by a
/// dictionary entry nothing in it uses.
fn crypt_filter(encrypt: &Dictionary) -> Option<String> {
    let filters = encrypt.get("CF")?.as_dict()?;
    let named = encrypt
        .get("StmF")
        .and_then(Object::as_name)
        .and_then(|name| name.as_str().map(str::to_string))
        .unwrap_or_else(|| "StdCF".into());
    let filter = filters
        .get(&named)
        .or_else(|| filters.get("StdCF"))
        .and_then(Object::as_dict)?;

    let method = filter.get("CFM").and_then(Object::as_name)?;
    let bits = filter
        .get("Length")
        .and_then(Object::as_i64)
        // §7.6.5 lets a crypt filter state its length in bytes; the trailer
        // dictionary states it in bits. Both spellings appear.
        .map(|n| if n <= 64 { n * 8 } else { n });

    Some(match method {
        m if m.is("AESV2") => "AES-128".into(),
        m if m.is("AESV3") => "AES-256".into(),
        m if m.is("V2") => format!("RC4 {}-bit", bits.unwrap_or(128)),
        m if m.is("None") => "no stream encryption".into(),
        other => format!("{other}"),
    })
}
