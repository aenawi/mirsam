//! Object syntax: §7.2 and §7.3, and the malformations every reader tolerates.

use mirsam_pdf::lexer::Lexer;
use mirsam_pdf::object::{Name, Object, ObjectId};

fn parse(source: &str) -> Object {
    Lexer::new(source.as_bytes())
        .object()
        .unwrap_or_else(|e| panic!("{source:?}: {e}"))
}

fn parse_bytes(source: &[u8]) -> Object {
    Lexer::new(source).object().expect("parse")
}

#[test]
fn the_atoms() {
    assert_eq!(parse("null"), Object::Null);
    assert_eq!(parse("true"), Object::Boolean(true));
    assert_eq!(parse("false"), Object::Boolean(false));
    assert_eq!(parse("42"), Object::Integer(42));
    assert_eq!(parse("-17"), Object::Integer(-17));
    assert_eq!(parse("+3"), Object::Integer(3));
    assert_eq!(parse("2.75"), Object::Real(2.75));
    assert_eq!(parse("-.002"), Object::Real(-0.002));
    // §7.3.3 spells this one out: a trailing point is legal.
    assert_eq!(parse("4."), Object::Real(4.0));
}

/// Numbers a writer should not have produced and every reader accepts. A file
/// refused over its punctuation is a file this tool reported unreadable that
/// a viewer opens.
#[test]
fn numbers_no_writer_should_have_written() {
    assert_eq!(parse("--5"), Object::Integer(0));
    assert_eq!(parse("6.-2"), Object::Real(6.0));
    assert_eq!(parse("."), Object::Integer(0));
}

#[test]
fn names_carry_their_escapes() {
    assert_eq!(parse("/Type"), Object::Name(Name::from("Type")));
    // §7.3.5's own example: /A#42 is /AB, and a dictionary lookup depends on
    // the two being one name.
    assert_eq!(parse("/A#42"), Object::Name(Name::from("AB")));
    assert_eq!(
        parse("/Lime#20Green"),
        Object::Name(Name::from("Lime Green"))
    );
    assert_eq!(parse("/"), Object::Name(Name::from("")));
    // A `#` a writer forgot to escape stays a `#` rather than failing the file.
    assert_eq!(parse("/a#zz"), Object::Name(Name::from("a#zz")));
}

#[test]
fn a_name_prints_the_way_the_file_writes_it() {
    assert_eq!(Name::from("Type").to_string(), "/Type");
    assert_eq!(Name::from("Lime Green").to_string(), "/Lime#20Green");
    assert_eq!(Name::from("A#B").to_string(), "/A#23B");
}

#[test]
fn literal_strings() {
    assert_eq!(parse("(hello)").as_string(), Some(&b"hello"[..]));
    // Balanced inner parentheses need no escaping, per §7.3.4.2.
    assert_eq!(parse("(a (b) c)").as_string(), Some(&b"a (b) c"[..]));
    assert_eq!(parse(r"(a\)b)").as_string(), Some(&b"a)b"[..]));
    assert_eq!(parse(r"(a\nb)").as_string(), Some(&b"a\nb"[..]));
    assert_eq!(parse(r"(\101)").as_string(), Some(&b"A"[..]));
    // Octal beyond a byte drops the high bits, which the specification says
    // in as many words.
    assert_eq!(parse(r"(\400)").as_string(), Some(&b"\x00"[..]));
    // A backslash before an end-of-line joins the lines.
    assert_eq!(parse("(a\\\nb)").as_string(), Some(&b"ab"[..]));
    // "\q" is q.
    assert_eq!(parse(r"(\q)").as_string(), Some(&b"q"[..]));
}

#[test]
fn hex_strings_pad_an_odd_final_digit() {
    assert_eq!(parse("<48656C6C6F>").as_string(), Some(&b"Hello"[..]));
    assert_eq!(parse("<48 65 6c>").as_string(), Some(&b"Hel"[..]));
    // §7.3.4.3: a missing final digit is a zero.
    assert_eq!(parse("<9>").as_string(), Some(&[0x90][..]));
    assert_eq!(parse("<>").as_string(), Some(&[][..]));
}

#[test]
fn arrays_and_dictionaries() {
    let array = parse("[1 (two) /three [4]]");
    let items = array.as_array().expect("array");
    assert_eq!(items.len(), 4);
    assert_eq!(items[0], Object::Integer(1));

    let dict = parse("<< /Type /Page /Count 3 /Kids [1 0 R] >>");
    let dict = dict.as_dict().expect("dictionary");
    assert!(dict.get("Type").unwrap().as_name().unwrap().is("Page"));
    assert_eq!(dict.get("Count").unwrap().as_i64(), Some(3));
    assert_eq!(
        dict.get("Kids").unwrap().as_array().unwrap()[0],
        Object::Reference(ObjectId::new(1, 0))
    );
}

/// A repeated key keeps the last value: the only way one arises in practice is
/// a writer appending an entry it meant to supersede an earlier one.
#[test]
fn a_repeated_key_keeps_the_last_value() {
    let dict = parse("<< /N 1 /N 2 >>");
    let dict = dict.as_dict().expect("dictionary");
    assert_eq!(dict.len(), 1);
    assert_eq!(dict.get("N").unwrap().as_i64(), Some(2));
}

#[test]
fn a_reference_is_two_integers_and_an_r() {
    assert_eq!(parse("12 0 R"), Object::Reference(ObjectId::new(12, 0)));
    // Without the R they are two numbers, and the lookahead has to rewind far
    // enough to give both of them back.
    assert_eq!(parse("[12 0]").as_array().unwrap().len(), 2);
    assert_eq!(parse("[12 0 (R)]").as_array().unwrap().len(), 3);
    // And a reference in the middle of an array is one item, not three.
    let array = parse("[1 2 R 3]");
    let items = array.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0], Object::Reference(ObjectId::new(1, 2)));
    assert_eq!(items[1], Object::Integer(3));
}

#[test]
fn comments_are_white_space() {
    assert_eq!(parse("% a comment\n42"), Object::Integer(42));
    let dict = parse("<< /A 1 % why\n/B 2 >>");
    assert_eq!(dict.as_dict().unwrap().len(), 2);
}

#[test]
fn a_stream_takes_the_length_it_states() {
    let source = b"<< /Length 5 >>\nstream\nHELLO\nendstream";
    let stream = parse_bytes(source);
    assert_eq!(stream.as_stream().expect("stream").data, b"HELLO");
}

/// `/Length` may be an indirect reference, which the lexer cannot resolve: the
/// cross-reference table is itself parsed out of streams. The data runs to
/// `endstream` instead, and no caller has to hand over a resolver it has not
/// built yet.
#[test]
fn an_indirect_length_falls_back_to_endstream() {
    let source = b"<< /Length 9 0 R >>\nstream\nHELLO\nendstream";
    let stream = parse_bytes(source);
    assert_eq!(stream.as_stream().expect("stream").data, b"HELLO");
}

/// And a direct length that is simply wrong, which real writers produce.
#[test]
fn a_wrong_length_is_not_believed() {
    let source = b"<< /Length 2 >>\nstream\nHELLO\nendstream";
    let stream = parse_bytes(source);
    assert_eq!(stream.as_stream().expect("stream").data, b"HELLO");
}

/// A stream may end in a newline of its own, and only the one belonging to the
/// syntax is dropped.
#[test]
fn only_the_syntactic_newline_is_taken_off() {
    let source = b"<< /Length 6 >>\nstream\nHELLO\n\nendstream";
    let stream = parse_bytes(source);
    assert_eq!(stream.as_stream().expect("stream").data, b"HELLO\n");
}

#[test]
fn an_indirect_object() {
    let mut lexer = Lexer::new(b"7 0 obj\n<< /Type /Catalog >>\nendobj\n");
    let (id, object) = lexer.indirect().expect("indirect object");
    assert_eq!(id, ObjectId::new(7, 0));
    assert!(object.is_type("Catalog"));
}

/// `endobj` is optional in practice: a writer that omits it produces a file
/// every reader opens, and the object is complete without it.
#[test]
fn endobj_is_not_required() {
    let mut lexer = Lexer::new(b"7 0 obj\n<< /Type /Catalog >>\n8 0 obj\nnull\nendobj\n");
    assert_eq!(lexer.indirect().expect("first").0, ObjectId::new(7, 0));
    assert_eq!(lexer.indirect().expect("second").0, ObjectId::new(8, 0));
}

#[test]
fn a_parse_failure_says_where() {
    let error = Lexer::new(b"<< /A 1").object().unwrap_err().to_string();
    assert!(error.contains("at byte 0"), "{error}");
    assert!(error.contains("no closing `>>`"), "{error}");
}

/// Depth is bounded: a hand-written file of ten thousand open brackets is not
/// a document, and must not be a stack overflow either.
#[test]
fn nesting_is_bounded() {
    let source = "[".repeat(10_000);
    assert!(Lexer::new(source.as_bytes()).object().is_err());
}
