//! The part of CSS that decides direction, and nothing else.
//!
//! ## Why an adapter for a *document* format parses CSS at all
//!
//! Because in HTML the direction is usually not in the document. `dir="rtl"`
//! exists, but the stylesheet — `body { direction: rtl }`, `.rtl { direction:
//! rtl; text-align: right }` — is where a great many Arabic sites actually
//! state it, and an adapter that read only attributes would report
//! `direction-unset` on every one of them. That is a false positive on
//! formatting the author chose, which is invariant 2, and the single failure
//! mode this project treats as worse than missing a defect.
//!
//! So the cascade is read, and only far enough to answer the questions the
//! rules ask: `direction`, `text-align`, `font-family`, `list-style-type` and
//! the multi-column properties. Colour, layout and everything else is parsed
//! past without being understood, because nothing downstream would judge it.
//!
//! ## What is deliberately not implemented, and what follows from that
//!
//! This is not a browser engine and must not pretend to be one:
//!
//! - **At-rules are skipped whole.** A declaration inside `@media`,
//!   `@supports` or `@layer` applies under a condition this tool cannot
//!   evaluate — a viewport it does not have — so applying one would be
//!   claiming to know the reader's screen.
//! - **Only the selector machinery a direction rule needs is matched**:
//!   type, `*`, `#id`, `.class`, `[attr]`, `[attr=value]`, `:root`, and the
//!   descendant and child combinators. A selector using anything else is
//!   dropped — *that one selector*, never the whole rule — because a
//!   `:hover` or `::before` declaration does not govern the text as stored.
//! - **Sibling combinators and specificity beyond the standard triple are
//!   absent**, for the same reason: they change which of two rules wins, and
//!   both would have to state a direction for that to matter.
//!
//! Every one of those omissions can only cost a *finding*. None of them can
//! manufacture one: a declaration this module fails to see leaves the property
//! where it already was.

use std::fmt;

/// One `property: value` pair, and whether it was marked `!important`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub property: String,
    pub value: String,
    pub important: bool,
}

/// How much of the cascade a declaration outranks.
///
/// CSS's origins, cut down to the two this crate produces. The `dir` attribute
/// is the user agent's — `[dir="rtl"] { direction: rtl }` is a rule in every
/// browser's own stylesheet — which is why an author's `direction: ltr` beats
/// a `dir="rtl"` written beside it, and why this has to be modelled rather
/// than assumed away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CascadeOrigin {
    /// The browser's own stylesheet: what `dir` and `<ul>` mean.
    UserAgent,
    /// A `<style>` element, or a stylesheet the document links.
    Author,
    /// A `style` attribute, which outranks any selector.
    Inline,
}

/// A selector's specificity: ids, then classes and attributes, then types.
type Specificity = (u32, u32, u32);

/// One simple selector within a compound.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Simple {
    /// `*`
    Universal,
    /// `div`
    Type(String),
    /// `#main`
    Id(String),
    /// `.rtl`
    Class(String),
    /// `[dir]`
    HasAttribute(String),
    /// `[dir="rtl"]`
    Attribute(String, String),
    /// `:root`, the one pseudo-class that names a real element rather than a
    /// state the stored document cannot be in.
    Root,
}

/// A run of simple selectors with no combinator between them: `div#main.rtl`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Compound(Vec<Simple>);

/// What separates one compound from the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Combinator {
    /// A space: matches any ancestor.
    Descendant,
    /// `>`: matches the parent alone.
    Child,
}

/// A complex selector, subject last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// The subject of the selector — the element the rule styles.
    subject: Compound,
    /// Ancestor constraints, nearest first, each carrying the combinator that
    /// binds it to the compound *after* it.
    ancestors: Vec<(Combinator, Compound)>,
    specificity: Specificity,
    /// The selector as the author wrote it, for a finding's evidence.
    text: String,
}

/// What an element must be able to answer for a selector to be matched
/// against it, so that matching does not depend on the tree type.
///
/// The DOM in [`crate::dom`] implements this; so does a test that wants to
/// state a chain without building a document.
pub trait Element {
    /// The element's lowercased local name.
    fn tag(&self) -> &str;
    /// One attribute's value.
    fn attribute(&self, name: &str) -> Option<String>;
    /// Whether this is the root element — `<html>`.
    fn is_root(&self) -> bool;
}

impl Compound {
    fn matches(&self, element: &dyn Element) -> bool {
        self.0.iter().all(|simple| match simple {
            Simple::Universal => true,
            Simple::Type(name) => element.tag() == name,
            Simple::Id(id) => element.attribute("id").as_deref() == Some(id.as_str()),
            Simple::Class(class) => element
                .attribute("class")
                .is_some_and(|value| value.split_ascii_whitespace().any(|c| c == class)),
            Simple::HasAttribute(name) => element.attribute(name).is_some(),
            Simple::Attribute(name, value) => element
                .attribute(name)
                .is_some_and(|actual| actual.eq_ignore_ascii_case(value)),
            Simple::Root => element.is_root(),
        })
    }
}

impl Selector {
    /// Whether this selector matches the last element of `chain`.
    ///
    /// `chain` is the element's ancestor path from the root, subject last —
    /// the shape a walk already has on its stack, so matching costs no
    /// tree traversal of its own.
    pub fn matches(&self, chain: &[&dyn Element]) -> bool {
        let Some((subject, ancestors)) = chain.split_last() else {
            return false;
        };
        if !self.subject.matches(*subject) {
            return false;
        }

        // Walked from the subject outwards: a descendant combinator may skip
        // as far up as it likes, a child combinator exactly one step.
        let mut remaining = ancestors;
        for (combinator, compound) in &self.ancestors {
            match combinator {
                Combinator::Child => match remaining.split_last() {
                    Some((parent, rest)) if compound.matches(*parent) => remaining = rest,
                    _ => return false,
                },
                Combinator::Descendant => {
                    match remaining
                        .iter()
                        .rposition(|ancestor| compound.matches(*ancestor))
                    {
                        Some(index) => remaining = &remaining[..index],
                        None => return false,
                    }
                }
            }
        }
        true
    }
}

/// A selector list and the declarations it carries.
#[derive(Debug, Clone)]
pub struct StyleRule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

/// One stylesheet, in source order.
#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
    /// Where the rules came from, for a finding's evidence: `<style>` for an
    /// embedded sheet, or the href of a linked one.
    pub source: String,
}

/// A declaration that matched, with everything the cascade sorts on.
#[derive(Debug, Clone)]
pub struct Match {
    pub declaration: Declaration,
    pub origin: CascadeOrigin,
    specificity: Specificity,
    order: usize,
    /// The selector as written, for the `Origin` a finding cites.
    pub selector: String,
}

impl Match {
    /// The cascade's sort key, ascending: the last one wins.
    ///
    /// `!important` reverses the origin order in real CSS. It is not reversed
    /// here, and the case where that differs — an important user-agent
    /// declaration — does not exist, because the only user-agent rules this
    /// crate synthesises are the presentational ones, none of them important.
    fn rank(&self) -> (bool, CascadeOrigin, Specificity, usize) {
        (
            self.declaration.important,
            self.origin,
            self.specificity,
            self.order,
        )
    }
}

impl fmt::Display for Match {
    /// The form `evidence.inherited_from` prints: the selector and the
    /// property, so a reviewer can find the declaration in the stylesheet.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{{{}}}", self.selector, self.declaration.property)
    }
}

/// The declarations that apply to one element, already cascaded.
///
/// Built by [`cascade`]; read by name.
#[derive(Debug, Clone, Default)]
pub struct Computed(Vec<Match>);

impl Computed {
    /// The winning declaration for `property`, if any declared it.
    pub fn get(&self, property: &str) -> Option<&Match> {
        self.0
            .iter()
            .filter(|m| m.declaration.property == property)
            .max_by_key(|m| m.rank())
    }

    /// The winning value for `property`, lowercased and trimmed.
    pub fn value(&self, property: &str) -> Option<String> {
        self.get(property)
            .map(|m| m.declaration.value.trim().to_ascii_lowercase())
    }
}

/// Every declaration that applies to the element at the end of `chain`.
///
/// `sheets` are matched in the order given, which is the order the document
/// states them; `inline` is the element's own `style` attribute, and
/// `presentational` the user-agent rules the caller synthesised from `dir`,
/// `align` and the list elements.
pub fn cascade(
    chain: &[&dyn Element],
    sheets: &[Stylesheet],
    presentational: Vec<Declaration>,
    inline: &str,
) -> Computed {
    let mut matches = Vec::new();
    let mut order = 0;

    for declaration in presentational {
        matches.push(Match {
            declaration,
            origin: CascadeOrigin::UserAgent,
            specificity: (0, 0, 0),
            order,
            selector: String::new(),
        });
        order += 1;
    }

    for sheet in sheets {
        for rule in &sheet.rules {
            let Some(selector) = rule
                .selectors
                .iter()
                .filter(|selector| selector.matches(chain))
                .max_by_key(|selector| selector.specificity)
            else {
                continue;
            };
            for declaration in &rule.declarations {
                matches.push(Match {
                    declaration: declaration.clone(),
                    origin: CascadeOrigin::Author,
                    specificity: selector.specificity,
                    order,
                    selector: selector.text.clone(),
                });
                order += 1;
            }
        }
    }

    for declaration in parse_declarations(inline) {
        matches.push(Match {
            declaration,
            origin: CascadeOrigin::Inline,
            specificity: (0, 0, 0),
            order,
            selector: "style".to_string(),
        });
        order += 1;
    }

    Computed(matches)
}

// --------------------------------------------------------------- the parser

impl Stylesheet {
    /// Parse a stylesheet's text.
    ///
    /// Never fails: a malformed rule is skipped and parsing resumes at the
    /// next one, which is what CSS itself specifies and what every browser
    /// does. A stylesheet this crate cannot understand leaves properties
    /// exactly where they were.
    pub fn parse(css: &str, source: impl Into<String>) -> Stylesheet {
        let mut rules = Vec::new();
        let text = strip_comments(css);
        let bytes: Vec<char> = text.chars().collect();
        let mut at = 0;

        while at < bytes.len() {
            match bytes[at] {
                c if c.is_whitespace() => at += 1,
                // An at-rule: skipped whole, block and all. See the module
                // documentation for why its declarations are not read.
                '@' => at = skip_at_rule(&bytes, at),
                _ => {
                    let Some(open) = find(&bytes, at, '{') else {
                        break;
                    };
                    let Some(close) = matching_brace(&bytes, open) else {
                        break;
                    };
                    let prelude: String = bytes[at..open].iter().collect();
                    let body: String = bytes[open + 1..close].iter().collect();
                    let selectors = parse_selector_list(&prelude);
                    if !selectors.is_empty() {
                        let declarations = parse_declarations(&body);
                        if !declarations.is_empty() {
                            rules.push(StyleRule {
                                selectors,
                                declarations,
                            });
                        }
                    }
                    at = close + 1;
                }
            }
        }

        Stylesheet {
            rules,
            source: source.into(),
        }
    }
}

/// Replace every `/* … */` with a space, so a comment cannot glue two tokens
/// together and cannot hide a brace from the block scanner.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        out.push(' ');
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            // Unterminated: everything after it is comment, as in CSS.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn find(chars: &[char], from: usize, needle: char) -> Option<usize> {
    chars[from..]
        .iter()
        .position(|c| *c == needle)
        .map(|offset| from + offset)
}

/// The `}` that closes the block opened at `open`, counting nesting so a
/// nested block does not end the outer one early.
fn matching_brace(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, c) in chars.iter().enumerate().skip(open) {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Skip an at-rule: either to its terminating `;` or past its whole block.
fn skip_at_rule(chars: &[char], at: usize) -> usize {
    let brace = find(chars, at, '{');
    let semicolon = find(chars, at, ';');
    match (brace, semicolon) {
        (Some(brace), semicolon) if semicolon.is_none_or(|s| brace < s) => {
            matching_brace(chars, brace).map_or(chars.len(), |close| close + 1)
        }
        (_, Some(semicolon)) => semicolon + 1,
        _ => chars.len(),
    }
}

/// `a: b; c: d !important` — the body of a rule, or a `style` attribute.
pub fn parse_declarations(body: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    for piece in strip_comments(body).split(';') {
        let Some((property, value)) = piece.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();
        if property.is_empty() || value.is_empty() {
            continue;
        }
        let (value, important) = match value.to_ascii_lowercase().rfind("!important") {
            Some(cut) => (value[..cut].trim(), true),
            None => (value, false),
        };
        if value.is_empty() {
            continue;
        }
        declarations.push(Declaration {
            property,
            value: value.to_string(),
            important,
        });
    }
    declarations
}

fn parse_selector_list(prelude: &str) -> Vec<Selector> {
    prelude
        .split(',')
        .filter_map(|selector| parse_selector(selector.trim()))
        .collect()
}

/// Parse one complex selector, or `None` if it uses anything this crate does
/// not match.
///
/// Returning `None` drops one selector of a list rather than the rule, so
/// `.rtl, .rtl:hover { direction: rtl }` still applies to `.rtl`.
fn parse_selector(text: &str) -> Option<Selector> {
    if text.is_empty() {
        return None;
    }

    // Split on combinators, keeping which one introduced each compound.
    let mut compounds: Vec<(Combinator, String)> = Vec::new();
    let mut current = String::new();
    let mut pending = Combinator::Descendant;
    let mut chars = text.chars().peekable();
    let mut seen_gap = false;

    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    seen_gap = true;
                }
            }
            '>' => {
                if current.is_empty() {
                    return None;
                }
                compounds.push((pending, std::mem::take(&mut current)));
                pending = Combinator::Child;
                seen_gap = false;
            }
            // Sibling combinators are not matched; see the module docs.
            '+' | '~' => return None,
            _ => {
                if seen_gap && !current.is_empty() {
                    compounds.push((pending, std::mem::take(&mut current)));
                    pending = Combinator::Descendant;
                }
                seen_gap = false;
                current.push(c);
                // An attribute selector may hold whitespace and combinator
                // characters; take it whole.
                if c == '[' {
                    for c in chars.by_ref() {
                        current.push(c);
                        if c == ']' {
                            break;
                        }
                    }
                }
            }
        }
    }
    if current.is_empty() {
        return None;
    }
    compounds.push((pending, current));

    // A combinator is written to the *left* of the compound it introduces, so
    // the one that binds an ancestor to its descendant is the one stored with
    // that descendant. Walking outwards from the subject, each ancestor is
    // reached by the combinator of the compound that follows it.
    let (subject_combinator, subject_text) = compounds.pop()?;
    let subject = parse_compound(&subject_text)?;

    let mut ancestors = Vec::new();
    let mut binding = subject_combinator;
    for (combinator, compound_text) in compounds.iter().rev() {
        ancestors.push((binding, parse_compound(compound_text)?));
        binding = *combinator;
    }

    let mut specificity = (0, 0, 0);
    for compound in std::iter::once(&subject).chain(ancestors.iter().map(|(_, c)| c)) {
        for simple in &compound.0 {
            match simple {
                Simple::Id(_) => specificity.0 += 1,
                Simple::Class(_)
                | Simple::HasAttribute(_)
                | Simple::Attribute(_, _)
                | Simple::Root => specificity.1 += 1,
                Simple::Type(_) => specificity.2 += 1,
                Simple::Universal => {}
            }
        }
    }

    Some(Selector {
        subject,
        ancestors,
        specificity,
        text: text.to_string(),
    })
}

/// Parse `div#main.rtl[dir="rtl"]`, or `None` on anything unsupported.
fn parse_compound(text: &str) -> Option<Compound> {
    let mut simples = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(&c) = chars.peek() {
        match c {
            '*' => {
                chars.next();
                simples.push(Simple::Universal);
            }
            '#' => {
                chars.next();
                let name = take_identifier(&mut chars);
                if name.is_empty() {
                    return None;
                }
                simples.push(Simple::Id(name));
            }
            '.' => {
                chars.next();
                let name = take_identifier(&mut chars);
                if name.is_empty() {
                    return None;
                }
                simples.push(Simple::Class(name));
            }
            '[' => {
                chars.next();
                let mut body = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == ']' {
                        closed = true;
                        break;
                    }
                    body.push(c);
                }
                if !closed {
                    return None;
                }
                simples.push(parse_attribute_selector(&body)?);
            }
            ':' => {
                // `:root` is the one pseudo-class about the tree rather than
                // about a state the document cannot be in.
                let rest: String = chars.clone().collect();
                let lowered = rest.to_ascii_lowercase();
                if lowered == ":root" {
                    return Some(Compound(vec![Simple::Root]));
                }
                return None;
            }
            c if c.is_alphabetic() || c == '_' || c == '-' => {
                let name = take_identifier(&mut chars);
                simples.push(Simple::Type(name.to_ascii_lowercase()));
            }
            _ => return None,
        }
    }

    (!simples.is_empty()).then_some(Compound(simples))
}

fn parse_attribute_selector(body: &str) -> Option<Simple> {
    let body = body.trim();
    match body.split_once('=') {
        None => {
            let name = body.trim().to_ascii_lowercase();
            (!name.is_empty() && name.chars().all(is_identifier_char))
                .then_some(Simple::HasAttribute(name))
        }
        Some((name, value)) => {
            // Only exact match: `~=`, `|=`, `^=`, `$=`, `*=` are substring
            // operators no direction rule in the wild needs.
            let name = name.trim();
            if name.ends_with(['~', '|', '^', '$', '*']) {
                return None;
            }
            let name = name.to_ascii_lowercase();
            let value = value.trim().trim_matches(['"', '\'']).to_string();
            (!name.is_empty()).then_some(Simple::Attribute(name, value))
        }
    }
}

fn take_identifier(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut name = String::new();
    while let Some(&c) = chars.peek() {
        if is_identifier_char(c) {
            name.push(c);
            chars.next();
        } else {
            break;
        }
    }
    name
}

fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_' || !c.is_ascii()
}
