//! The document tree, built by `html5ever` into a sink this crate owns.
//!
//! ## Why a tree at all, when the other adapters stream
//!
//! [`crate::css`] and the direction cascade both need *ancestors*. `dir` is
//! inherited — a `dir="rtl"` on `<body>` governs every paragraph under it that
//! does not restate it — and a CSS descendant selector asks the same question
//! backwards. A streaming scanner can carry a stack of open elements and get
//! that right for well-formed input; HTML is not well-formed input, and the
//! places it is not are exactly the places the ancestor chain moves.
//!
//! ## Why `html5ever`, and why not its DOM
//!
//! HTML's tree construction is not "close the tag you opened". `<p>a<p>b`
//! is two sibling paragraphs, not a nested one; a `<div>` inside a `<p>`
//! closes it; and text that lands between `<table>` and its first `<tr>` is
//! *foster-parented* out of the table and becomes the table's preceding
//! sibling. Each of those moves a node's ancestors, and this adapter reads
//! `dir` off ancestors — so a hand-rolled nesting stack would not merely be
//! approximate, it would resolve direction from a chain no browser has. The
//! whole design rests on reporting what a reader will see (ADR 0004), and that
//! obliges the adapter to build the tree the reader's browser builds.
//!
//! `markup5ever_rcdom`, the DOM that ships beside the parser, says of itself
//! that it is unsupported, unfuzzed and not for production. This module is the
//! ~200 lines that avoid depending on that: a reference-counted tree carrying
//! the four node kinds this crate reads and nothing else.

use html5ever::interface::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::{Attribute, LocalName, QualName};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::mem;
use std::rc::{Rc, Weak};

/// A reference to a node in the tree.
pub type Handle = Rc<Node>;

/// What a node is. Comments, doctypes and processing instructions collapse
/// into [`NodeData::Other`]: they hold no text a reader sees and no property
/// a rule judges, and keeping them as one variant means the walk in
/// [`crate::html`] has one case to skip rather than three.
#[derive(Debug)]
pub enum NodeData {
    /// The root.
    Document,
    /// An element and the attributes written on it.
    Element {
        name: QualName,
        attrs: RefCell<Vec<Attribute>>,
        /// A `<template>`'s contents, which the parser keeps off to one side.
        template: RefCell<Option<Handle>>,
    },
    /// Character data, as written — never normalised here. Whitespace is
    /// meaningful until the caller decides it is not.
    Text(RefCell<String>),
    /// A comment, doctype or processing instruction.
    Other,
}

/// One node of the document tree.
///
/// Parents are weak, children strong, which is [`Drop`]'s problem below and
/// nobody else's.
pub struct Node {
    pub data: NodeData,
    parent: Cell<Option<Weak<Node>>>,
    children: RefCell<Vec<Handle>>,
}

impl Node {
    fn new(data: NodeData) -> Handle {
        Rc::new(Node {
            data,
            parent: Cell::new(None),
            children: RefCell::new(Vec::new()),
        })
    }

    /// This node's children, in document order.
    pub fn children(&self) -> std::cell::Ref<'_, Vec<Handle>> {
        self.children.borrow()
    }

    /// The element's local name, lowercased by the parser: `div`, `p`, `td`.
    /// `None` for everything that is not an element.
    pub fn local_name(&self) -> Option<&LocalName> {
        match &self.data {
            NodeData::Element { name, .. } => Some(&name.local),
            _ => None,
        }
    }

    /// Whether this element is the named HTML element.
    pub fn is(&self, tag: &str) -> bool {
        self.local_name().is_some_and(|name| &**name == tag)
    }

    /// One attribute's value, by local name. Attribute names arrive
    /// lowercased for HTML, so `DIR` and `dir` are the same attribute — which
    /// is what a browser does and therefore what this must do.
    pub fn attribute(&self, name: &str) -> Option<String> {
        match &self.data {
            NodeData::Element { attrs, .. } => attrs
                .borrow()
                .iter()
                .find(|a| &*a.name.local == name)
                .map(|a| a.value.to_string()),
            _ => None,
        }
    }

    /// The text this node holds directly, if it is a text node.
    pub fn text(&self) -> Option<String> {
        match &self.data {
            NodeData::Text(contents) => Some(contents.borrow().clone()),
            _ => None,
        }
    }

    /// A `<template>`'s contents, which the tree builder parks outside the
    /// tree. Text inside one is still text an author wrote and a reader may
    /// see, so the walk descends into it.
    pub fn template_contents(&self) -> Option<Handle> {
        match &self.data {
            NodeData::Element { template, .. } => template.borrow().clone(),
            _ => None,
        }
    }

    fn parent(&self) -> Option<Weak<Node>> {
        let parent = self.parent.take();
        self.parent.set(parent.clone());
        parent
    }
}

/// Hand-written because the parent pointer is a `Cell`, which is only `Debug`
/// for a `Copy` payload — and printing parents would recurse for ever anyway.
impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("data", &self.data)
            .field("children", &self.children)
            .finish()
    }
}

/// Drop iteratively rather than recursively.
///
/// A document nested ten thousand deep is a legal document, and the naive
/// recursive drop of a strong-child tree overflows the stack on one. The same
/// guard `markup5ever_rcdom` carries, and for the same reason.
impl Drop for Node {
    fn drop(&mut self) {
        let mut pending = mem::take(&mut *self.children.borrow_mut());
        while let Some(node) = pending.pop() {
            let children = mem::take(&mut *node.children.borrow_mut());
            pending.extend(children);
            if let Some(contents) = node.template_contents() {
                pending.push(contents);
            }
        }
    }
}

/// The parsed document.
pub struct Document {
    /// The root node.
    pub root: Handle,
    /// Whether the parser had to guess: a document with no doctype is parsed
    /// in quirks mode by every browser, and this records that rather than
    /// acting on it.
    pub quirks: QuirksMode,
}

impl Document {
    /// Parse a whole HTML document.
    ///
    /// Infallible by design, which is HTML's design and not this crate's: the
    /// parsing specification defines a tree for every byte sequence, including
    /// the empty one. A document that is "broken" still has a tree, and
    /// refusing to give one would be refusing to audit exactly the documents
    /// most likely to be wrong.
    pub fn parse(html: &str) -> Document {
        use html5ever::tendril::TendrilSink;
        html5ever::parse_document(Sink::default(), Default::default()).one(html)
    }

    /// Every node in the tree, in document order, with its ancestors already
    /// visited. `<template>` contents are visited where the element sits.
    pub fn walk(&self, mut visit: impl FnMut(&Handle)) {
        fn descend(node: &Handle, visit: &mut impl FnMut(&Handle)) {
            visit(node);
            if let Some(contents) = node.template_contents() {
                for child in contents.children().iter() {
                    descend(child, visit);
                }
            }
            for child in node.children().iter() {
                descend(child, visit);
            }
        }
        descend(&self.root, &mut visit);
    }
}

/// The tree builder's other half: it calls, this stores.
struct Sink {
    document: Handle,
    quirks: Cell<QuirksMode>,
}

impl Default for Sink {
    fn default() -> Self {
        Sink {
            document: Node::new(NodeData::Document),
            quirks: Cell::new(QuirksMode::NoQuirks),
        }
    }
}

/// Attach a parentless node to a parent.
fn append(parent: &Handle, child: Handle) {
    let previous = child.parent.replace(Some(Rc::downgrade(parent)));
    debug_assert!(previous.is_none(), "child already had a parent");
    parent.children.borrow_mut().push(child);
}

/// A node's parent and its position among that parent's children.
fn locate(target: &Handle) -> Option<(Handle, usize)> {
    let parent = target.parent()?.upgrade()?;
    let index = parent
        .children
        .borrow()
        .iter()
        .position(|child| Rc::ptr_eq(child, target))?;
    Some((parent, index))
}

/// Merge text into `node` when `node` is a text node, as the sink contract
/// requires: two adjacent text nodes are one run of characters, and a scanner
/// that saw them separately would split a word.
fn merge_text(node: &Handle, text: &str) -> bool {
    match &node.data {
        NodeData::Text(contents) => {
            contents.borrow_mut().push_str(text);
            true
        }
        _ => false,
    }
}

fn detach(target: &Handle) {
    if let Some((parent, index)) = locate(target) {
        parent.children.borrow_mut().remove(index);
        target.parent.set(None);
    }
}

impl TreeSink for Sink {
    type Handle = Handle;
    type Output = Document;
    type ElemName<'a>
        = &'a QualName
    where
        Self: 'a;

    fn finish(self) -> Document {
        Document {
            root: self.document,
            quirks: self.quirks.get(),
        }
    }

    /// Parse errors are discarded on purpose.
    ///
    /// Every one of them is a statement about the *markup*, and this tool
    /// reports Arabic correctness. A document that recovers from a stray `</b>`
    /// renders exactly as its author sees it render, and a finding about it
    /// would be noise between the reader and the defect they came for.
    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> Handle {
        self.document.clone()
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> &'a QualName {
        match &target.data {
            NodeData::Element { name, .. } => name,
            _ => panic!("elem_name on a node that is not an element"),
        }
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        Node::new(NodeData::Element {
            name,
            attrs: RefCell::new(attrs),
            // Parented to nothing: a template's contents sit beside the tree,
            // and `Node::drop` reaches them through the element.
            template: RefCell::new(flags.template.then(|| Node::new(NodeData::Document))),
        })
    }

    fn create_comment(&self, _text: StrTendril) -> Handle {
        Node::new(NodeData::Other)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        Node::new(NodeData::Other)
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        if let NodeOrText::AppendText(text) = &child
            && let Some(last) = parent.children.borrow().last()
            && merge_text(last, text)
        {
            return;
        }
        let node = match child {
            NodeOrText::AppendText(text) => {
                Node::new(NodeData::Text(RefCell::new(text.to_string())))
            }
            NodeOrText::AppendNode(node) => node,
        };
        append(parent, node);
    }

    fn append_before_sibling(&self, sibling: &Handle, new_node: NodeOrText<Handle>) {
        let Some((parent, index)) = locate(sibling) else {
            // The tree builder promises a parent here; without one there is
            // no insertion point, and dropping the node silently is better
            // than panicking on a document somebody merely wanted audited.
            return;
        };

        let node = match (new_node, index) {
            (NodeOrText::AppendText(text), 0) => {
                Node::new(NodeData::Text(RefCell::new(text.to_string())))
            }
            (NodeOrText::AppendText(text), index) => {
                let merged = parent
                    .children
                    .borrow()
                    .get(index - 1)
                    .is_some_and(|prev| merge_text(prev, &text));
                if merged {
                    return;
                }
                Node::new(NodeData::Text(RefCell::new(text.to_string())))
            }
            (NodeOrText::AppendNode(node), _) => node,
        };

        detach(&node);
        node.parent.set(Some(Rc::downgrade(&parent)));
        parent.children.borrow_mut().insert(index, node);
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        if element.parent().is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {
        append(&self.document, Node::new(NodeData::Other));
    }

    fn get_template_contents(&self, target: &Handle) -> Handle {
        target
            .template_contents()
            .expect("get_template_contents on a node that is not a template")
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        Rc::ptr_eq(x, y)
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks.set(mode);
    }

    /// The tree builder re-opens a formatting element by copying the
    /// attributes it has not already got. Attributes already present win,
    /// which is the spec's rule and matters here: the first `dir` an element
    /// was given is the one the browser keeps.
    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let NodeData::Element {
            attrs: existing, ..
        } = &target.data
        else {
            return;
        };
        let mut existing = existing.borrow_mut();
        for attr in attrs {
            if !existing.iter().any(|a| a.name == attr.name) {
                existing.push(attr);
            }
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        detach(target);
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let moved = mem::take(&mut *node.children.borrow_mut());
        for child in moved {
            child.parent.set(Some(Rc::downgrade(new_parent)));
            new_parent.children.borrow_mut().push(child);
        }
    }
}
