//! A view as what it says (one-view-design.md, 3.1): each element's kind
//! as the view wrote it, its options typed, its events, its children; each
//! control with what it is bound to; a table's rows with their options; a
//! chart with what it was given; a widget; text; and markup that arrives
//! as HTML, kept whole. One builder makes the tree (`element` and the
//! control functions) and two readers read it: the HTML renderer (`html`,
//! `vnode`), which writes the page as it has always been written, and the
//! data form (`to_json`), which a drawn renderer reads.
//!
//! The tree keeps no tag, class or style: those are the HTML renderer's,
//! made from the tree as it writes the page, through the layout kernel
//! (crow_runtime::layout) both modes share.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crow_runtime::frame::CrowFrame;
use crow_runtime::layout;

use super::{escape_attr, escape_text};
/// The page's nodes, which the HTML renderer writes (`vnode`).
pub use super::CrowVNode;

/// One node of a view.
#[derive(Debug, Clone)]
pub enum Node {
    /// An element of the vocabulary (`stack`, `row`, `button`, `p`, …).
    Element(Element),
    /// A frame's rows inside a `table`, with the table's options.
    Rows(Box<Rows>),
    /// A bound input, a progress bar or an icon.
    Control(Control),
    /// `chart[…]`, as it was given.
    Chart(Box<Chart>),
    /// `widget[name, props, on]`: drawn by the program's own module.
    Widget(Widget),
    /// Text, shown as it is.
    Text(String),
    /// Markup that arrived as HTML (`doc.markdown`, `menu`, `pager`, a
    /// markup value): kept as it is, for the HTML renderer to place.
    Markup(String),
    /// A value that failed: the text the page shows in its place (an id
    /// the server's log has the reason under).
    Failed(String),
    /// The page's own nodes, made as HTML rather than by the builder (a
    /// program of crow_web's that builds `CrowVNode`s by hand, the runtime's
    /// tests): written as they are.
    Html(CrowVNode),
}

impl Node {
    /// The node as the page's markup (`html`).
    pub fn to_html(&self) -> String {
        html(self)
    }
}

impl From<CrowVNode> for Node {
    fn from(v: CrowVNode) -> Node {
        Node::Html(v)
    }
}

/// A node shows as its markup, as a view called outside views does —
/// logged, handed to `doc.document`.
impl crow_runtime::display::CrowDisplay for Node {
    fn crow_repr(&self) -> String {
        self.to_html()
    }
}

/// An option's value, typed as the view gave it.
#[derive(Debug, Clone, PartialEq)]
pub enum Opt {
    Text(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Opt {
    /// The value as an attribute writes it.
    pub fn shown(&self) -> String {
        match self {
            Opt::Text(s) => s.clone(),
            Opt::Int(n) => n.to_string(),
            Opt::Float(x) => x.to_string(),
            Opt::Bool(b) => b.to_string(),
        }
    }

    fn json(&self) -> serde_json::Value {
        match self {
            Opt::Text(s) => serde_json::Value::String(s.clone()),
            Opt::Int(n) => serde_json::Value::from(*n),
            Opt::Float(x) => crow_runtime::json::float_to_json(*x),
            Opt::Bool(b) => serde_json::Value::Bool(*b),
        }
    }
}

/// What an element does when the reader acts on it: `click`, `input`,
/// `submit` or `change`, the handler it calls, and the arguments its call
/// froze (`click = pick[3]`), each as the page sends it back; `None` for a
/// handler named alone.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub on: String,
    pub handler: String,
    pub args: Option<Vec<serde_json::Value>>,
}

/// An element: its kind as written, its options in name order, its
/// events, the question a click asks first, and its children.
#[derive(Debug, Clone)]
pub struct Element {
    pub kind: String,
    pub options: Vec<(String, Opt)>,
    pub events: Vec<Event>,
    pub confirm: Option<String>,
    pub children: Vec<Arc<Node>>,
    /// The view this element is the result of read a value still being
    /// computed: the page shows it is catching up (`data-crow-stale`).
    pub stale: bool,
    /// Its places whose value is a binding (one-view-design.md 3.5): a
    /// text child, a control child or an option the program computes from
    /// the session's scalar facts and derivations alone, by the binding's
    /// id in the program's table. A live page sets such a place when its
    /// value moves, with no view run.
    pub binds: Vec<Bind>,
    /// It or an element inside it has a bound place (`rebind` passes over
    /// the rest).
    pub bound_within: bool,
}

/// Whether any of the children is an element with a bound place in it.
fn binds_inside(children: &[Arc<Node>]) -> bool {
    children.iter().any(|c| matches!(c.as_ref(), Node::Element(e) if e.bound_within))
}

/// A bound place of an element: a child (its place among the element's
/// children) or an option (by name), and the binding's id.
#[derive(Debug, Clone, PartialEq)]
pub enum Bind {
    Child { at: usize, id: u32 },
    Option { name: String, id: u32 },
}

/// Where a program says a binding stands among what it hands the builder:
/// a child by its place among the children given (before lists are
/// spliced), an option by its name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BindAt<'a> {
    Child(usize),
    Option(&'a str),
}

/// A bound place's value as the program computes it again: a text child's
/// text, an option's value (none: the option is left out), a control.
#[derive(Debug, Clone)]
pub enum Bound {
    Text(String),
    Opt(Option<Opt>),
    Node(Node),
}

impl Bound {
    /// Two values a place shows alike: texts and options by their value, a
    /// control by its markup.
    pub fn same(&self, other: &Bound) -> bool {
        match (self, other) {
            (Bound::Text(a), Bound::Text(b)) => a == b,
            (Bound::Opt(a), Bound::Opt(b)) => a == b,
            (Bound::Node(a), Bound::Node(b)) => html(a) == html(b),
            _ => false,
        }
    }

    /// A child's value as the builder places it, when it is one a bound
    /// place holds: one text (a number, a truth and any other value as it
    /// shows), or a control.
    pub fn of_child(g: Given) -> Option<Bound> {
        let mut kids = Vec::with_capacity(1);
        splice(&mut kids, g);
        match kids.pop() {
            Some(n) if kids.is_empty() => match &*n {
                Node::Text(t) => Some(Bound::Text(t.clone())),
                Node::Control(_) => Some(Bound::Node((*n).clone())),
                _ => None,
            },
            _ => None,
        }
    }

    /// An option's value as the builder keeps it: none where it leaves the
    /// option out.
    pub fn of_option(g: Given) -> Option<Bound> {
        Some(Bound::Opt(opt_of(g)))
    }

    /// A control's node as a bound place's value.
    pub fn of_node(n: Node) -> Option<Bound> {
        matches!(n, Node::Control(_)).then_some(Bound::Node(n))
    }
}

/// The bindings placed in a node, each once: its elements' bound places,
/// within the elements that hold one (`bound_within`).
pub fn bound_ids(node: &Node, out: &mut std::collections::BTreeSet<u32>) {
    if let Node::Element(e) = node
        && e.bound_within
    {
        for b in &e.binds {
            out.insert(match b {
                Bind::Child { id, .. } | Bind::Option { id, .. } => *id,
            });
        }
        for c in &e.children {
            bound_ids(c, out);
        }
    }
}

/// The node with each bound place whose binding `values` holds set to the
/// value it holds: none when no place changed (each place compared with
/// what it shows, so a value computed again to the same leaves its place
/// and the subtrees around it as they are). `set` counts the places that
/// changed. An option the layout vocabulary refuses is the caller's to
/// render as the program does (`Err`).
pub fn rebind(node: &Node, values: &HashMap<u32, Bound>, set: &mut usize) -> Result<Option<Node>, String> {
    let Node::Element(e) = node else { return Ok(None) };
    if !e.bound_within {
        return Ok(None);
    }
    let mut kids: Option<Vec<Arc<Node>>> = None;
    let mut opts: Option<Vec<(String, Opt)>> = None;
    for b in &e.binds {
        match b {
            Bind::Child { at, id } => {
                let (Some(v), Some(now)) = (values.get(id), e.children.get(*at)) else { continue };
                let same = match (v, now.as_ref()) {
                    (Bound::Text(t), Node::Text(o)) => t == o,
                    (Bound::Node(n), o @ Node::Control(_)) => html(n) == html(o),
                    _ => false,
                };
                if !same {
                    let placed = match v {
                        Bound::Text(t) => Node::Text(t.clone()),
                        Bound::Node(n) => n.clone(),
                        Bound::Opt(_) => return Err("a binding's option value given for a child".to_string()),
                    };
                    kids.get_or_insert_with(|| e.children.clone())[*at] = Arc::new(placed);
                    *set += 1;
                }
            }
            Bind::Option { name, id } => {
                let Some(v) = values.get(id) else { continue };
                let Bound::Opt(v) = v else { return Err("a binding's child value given for an option".to_string()) };
                let now = e.options.iter().find(|(k, _)| k == name).map(|(_, o)| o);
                if now != v.as_ref() {
                    let o = opts.get_or_insert_with(|| e.options.clone());
                    match (o.iter().position(|(k, _)| k == name), v) {
                        (Some(i), Some(v)) => o[i].1 = v.clone(),
                        (Some(i), None) => {
                            o.remove(i);
                        }
                        (None, Some(v)) => {
                            let i = o.partition_point(|(k, _)| k.as_str() < name.as_str());
                            o.insert(i, (name.clone(), v.clone()));
                        }
                        (None, None) => {}
                    }
                    *set += 1;
                }
            }
        }
    }
    for (i, c) in e.children.iter().enumerate() {
        if let Some(n) = rebind(c, values, set)? {
            kids.get_or_insert_with(|| e.children.clone())[i] = Arc::new(n);
        }
    }
    if kids.is_none() && opts.is_none() {
        return Ok(None);
    }
    if let Some(o) = &opts {
        // the options the layout vocabulary refuses stop the render where
        // the view is built
        let attrs: Vec<(String, String)> = o.iter().map(|(k, v)| (k.clone(), v.shown())).collect();
        if let Some(why) = layout::render(&e.kind, &attrs).errors.first() {
            return Err(format!("{}: {}", e.kind, why));
        }
    }
    let mut out = e.clone_bare();
    out.children = kids.unwrap_or_else(|| e.children.clone());
    if let Some(o) = opts {
        out.options = o;
    }
    Ok(Some(Node::Element(out)))
}

/// Whether two nodes are one tree: an element of the same options whose
/// children are the same subtrees, held by both (a view's node and the
/// copy its caller placed).
pub fn same_tree(a: &Node, b: &Node) -> bool {
    match (a, b) {
        (Node::Element(x), Node::Element(y)) => {
            x.kind == y.kind
                && x.options == y.options
                && x.stale == y.stale
                && x.events == y.events
                && x.confirm == y.confirm
                && x.binds == y.binds
                && x.children.len() == y.children.len()
                && x.children.iter().zip(&y.children).all(|(p, q)| Arc::ptr_eq(p, q))
        }
        (Node::Text(x), Node::Text(y)) => x == y,
        _ => false,
    }
}

/// Whether an element's places can be bindings at all: a widget's are its
/// module's, and a table's children are its rows.
pub fn element_binds(kind: &str) -> bool {
    !matches!(kind, "widget" | "table")
}

/// A control a binding can be (derive::bind_sites), by its name and the
/// arguments written: `Some(Some(i))` for a bound input whose variable is
/// argument `i`, `Some(None)` for a progress bar, none for any other — a
/// `field` of a file is an upload, and a `select` of a map is a menu.
pub fn control_binds(name: &str, args: usize, first_is_map: bool, field_kind: Option<&str>) -> Option<Option<usize>> {
    let arity = match name {
        "field" if field_kind.is_some_and(|k| k != "file") => 4,
        "slider" => 5,
        "checkbox" | "toggle" | "radio" | "tabs" | "progress" => 2,
        "select" if !first_is_map => 2,
        _ => return None,
    };
    (args == arity).then_some(if name == "progress" { None } else { Some(1) })
}

/// Whether an option of an element can be a bound place: one the page
/// writes as attributes alone, which neither the events the page offers
/// nor the matching of children reads (`open`, `hidden`, an input's or an
/// option's `value` and `type`, `key`, `id`), nor the links a `nav` marks
/// (`href`, `active`), nor a table's own options.
pub fn option_binds(kind: &str, name: &str) -> bool {
    if EVENTS.contains(&name) || matches!(name, "confirm" | "seek" | "key" | "id" | "open" | "hidden" | "href" | "active") {
        return false;
    }
    if matches!(name, "value" | "type" | "name" | "checked" | "selected") && matches!(kind, "input" | "option" | "select" | "textarea" | "button") {
        return false;
    }
    kind != "table"
}

/// A frame's rows in a table, and what the table was told of them:
/// `click` with the `key` column's cell, `mark`, `delta`, `actions` (label,
/// handler) with the `confirm` each asks, `hide`, and the session's page
/// and order where the table is paged (`page = n`) or sorted (`sort = true`).
#[derive(Debug, Clone)]
pub struct Rows {
    pub frame: CrowFrame,
    pub click: Option<String>,
    pub key: Option<String>,
    pub mark: Option<String>,
    pub delta: Vec<String>,
    pub actions: Vec<(String, String)>,
    pub hide: Vec<String>,
    pub confirm: Vec<(String, String)>,
    pub page: Option<layout::Paging>,
    pub sort: Option<layout::Sorting>,
}

/// A control: what it shows and the fact it is bound to (`bind`), with the
/// value the fact holds as it shows (`shown`) and as data (`value`).
#[derive(Debug, Clone)]
pub enum Control {
    /// `field[label, var, type, lines]`: a text, number or file input, or
    /// a textarea of `lines` rows.
    Field { label: String, bind: String, value: serde_json::Value, shown: String, kind: String, lines: i64 },
    /// `slider[label, var, min, max, step]`.
    Slider { label: String, bind: String, value: serde_json::Value, shown: String, min: f64, max: f64, step: f64 },
    /// `checkbox[label, var]`, `toggle[label, var]`.
    Check { label: String, bind: String, on: bool, toggle: bool },
    /// `radio[options, var]`, `tabs[options, var]`.
    Choice { options: Vec<String>, bind: String, current: String, tabs: bool },
    /// `select[options, var]`.
    Select { options: Vec<String>, bind: String, current: String },
    /// `progress[value, max]`.
    Progress { value: f64, max: f64 },
    /// `icon[name, size, color]`, from the kernel's closed set.
    Icon { name: String, size: String, color: String },
}

impl Control {
    /// The control's name as the view wrote it.
    pub fn kind(&self) -> &'static str {
        match self {
            Control::Field { .. } => "field",
            Control::Slider { .. } => "slider",
            Control::Check { toggle: false, .. } => "checkbox",
            Control::Check { toggle: true, .. } => "toggle",
            Control::Choice { tabs: false, .. } => "radio",
            Control::Choice { tabs: true, .. } => "tabs",
            Control::Select { .. } => "select",
            Control::Progress { .. } => "progress",
            Control::Icon { .. } => "icon",
        }
    }
}

/// `chart[frame, x, candles, lines, bars, marker, tz, height, panes,
/// levels, points, seek]`, as it was given.
#[derive(Debug, Clone)]
pub struct Chart {
    pub frame: CrowFrame,
    pub x: String,
    pub candles: Vec<String>,
    pub lines: Vec<String>,
    pub bars: String,
    /// The marker's time; NaN for none.
    pub marker: f64,
    pub tz: String,
    pub height: f64,
    pub panes: Vec<Vec<String>>,
    pub levels: BTreeMap<String, Vec<f64>>,
    pub points: Option<CrowFrame>,
    pub seek: Option<String>,
}

/// `widget[name, props = value, on = map[event = handler]]`.
#[derive(Debug, Clone)]
pub struct Widget {
    pub module: String,
    pub props: serde_json::Value,
    pub on: Vec<(String, String)>,
}

/// A value handed to the builder: an option's or a child's, typed as the
/// program held it. A mode reads its own values into these, and the
/// builder decides what each is to the element.
#[derive(Debug, Clone)]
pub enum Given {
    Text(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    /// A function named alone: a handler.
    Handler(String),
    /// A handler with the arguments its call froze, each as JSON.
    Call(String, Vec<serde_json::Value>),
    Map(Vec<(String, Given)>),
    List(Vec<Given>),
    Frame(Box<CrowFrame>),
    /// A node a view, an element or a control made.
    Node(Arc<Node>),
    /// Markup that arrived as HTML.
    Markup(String),
    /// A child that failed: its reason.
    Failed(String),
    /// Any other value, as it shows (a record, a moment, a map among the
    /// children).
    Shown(String),
}

/// A node a view, an element or a control made, as a child.
impl From<Node> for Given {
    fn from(n: Node) -> Given {
        Given::Node(Arc::new(n))
    }
}

/// A node's markup, held as text where a program keeps a node in a
/// variable (the compiled program's locals): markup, not text.
impl From<String> for Given {
    fn from(m: String) -> Given {
        Given::Markup(m)
    }
}

impl Given {
    /// A node made by a view, an element or a control, as a child or an
    /// option.
    pub fn node(n: Node) -> Given {
        Given::Node(Arc::new(n))
    }

    /// A frame's column among the children: each cell as text, a missing
    /// one as a value that failed.
    pub fn column(c: &crow_runtime::frame::CrowColumn) -> Given {
        use crow_runtime::display::CrowDisplay;
        use crow_runtime::frame::Column;
        Given::List(
            (0..c.len())
                .map(|i| {
                    if !c.valid[i] {
                        return Given::Failed("missing".to_string());
                    }
                    Given::Text(match c.data.as_ref() {
                        Column::Int(v) => v[i].crow_fmt(),
                        Column::Float(v) => v[i].crow_fmt(),
                        Column::Str(v) => v[i].to_string(),
                        Column::Bool(v) => v[i].crow_fmt(),
                        Column::DateTime(..) | Column::Vectors(..) => c.data.fmt_cell(i, true),
                    })
                })
                .collect(),
        )
    }

    /// The value as text, as the program shows it.
    fn shown(&self) -> String {
        match self {
            Given::Text(s) | Given::Markup(s) | Given::Shown(s) => s.clone(),
            Given::Int(n) => n.to_string(),
            Given::Float(x) => x.to_string(),
            Given::Bool(b) => b.to_string(),
            Given::Handler(h) => format!("<fn {}>", h),
            Given::Call(h, _) => format!("<fn {}>", h),
            Given::Map(_) | Given::List(_) => String::new(),
            Given::Frame(f) => format!("{}", f),
            Given::Node(n) => html(n),
            Given::Failed(why) => crate::core::response::failed_in_view(why),
        }
    }
}

/// What a session holds of a table as it renders: which paged table of the
/// render this is and the page it is on, which sorted one and its order.
#[derive(Debug, Clone, Default)]
pub struct TableState {
    pub page: Option<(usize, usize)>,
    pub sort: Option<(usize, Option<(String, bool)>)>,
}

/// The events an element takes; `seek` is a chart's, which an element
/// does not draw.
const EVENTS: &[&str] = &["click", "input", "submit", "change"];

/// An element from its name, its options (in name order) and its children:
/// what the view said, sorted into options, events and children. A layout
/// option the vocabulary refuses (`gap = 'huge'`) is the render's stop.
pub fn element(kind: &str, options: Vec<(String, Given)>, children: Vec<Given>, tables: TableState) -> Result<Node, String> {
    element_bound(kind, options, children, tables, &[])
}

/// `element`, with the places the program says are bindings (`BindAt`, by
/// the id of each): a child kept as a bound place where it is placed as
/// one text or one control, an option where the element keeps it and
/// `option_binds` allows it.
pub fn element_bound(kind: &str, options: Vec<(String, Given)>, children: Vec<Given>, tables: TableState, binds: &[(BindAt<'_>, u32)]) -> Result<Node, String> {
    let mut opts: Vec<(String, Opt)> = Vec::new();
    let mut events: Vec<Event> = Vec::new();
    let mut confirm: Option<String> = None;
    let table = kind == "table";
    // what a table's options say of its rows, for the frame among its children
    let mut rows = Rows {
        frame: CrowFrame { columns: Vec::new(), data: Vec::new(), validity: Vec::new() },
        click: None,
        key: None,
        mark: None,
        delta: Vec::new(),
        actions: Vec::new(),
        hide: Vec::new(),
        confirm: Vec::new(),
        page: None,
        sort: None,
    };
    let mut sort = false;
    for (k, v) in options {
        if table {
            match (k.as_str(), &v) {
                ("click", Given::Handler(h)) => {
                    rows.click = Some(h.clone());
                    continue;
                }
                ("key", Given::Text(s)) => {
                    rows.key = Some(s.clone());
                    continue;
                }
                ("mark", Given::Text(s)) => {
                    rows.mark = Some(s.clone());
                    continue;
                }
                ("delta", Given::List(items)) => {
                    rows.delta = items.iter().map(Given::shown).collect();
                    continue;
                }
                ("actions", Given::Map(acts)) => {
                    rows.actions = acts.iter().filter_map(|(label, h)| if let Given::Handler(h) = h { Some((label.clone(), h.clone())) } else { None }).collect();
                    continue;
                }
                ("hide", Given::List(items)) => {
                    rows.hide = items.iter().map(Given::shown).collect();
                    continue;
                }
                ("confirm", Given::Map(texts)) => {
                    rows.confirm = texts.iter().map(|(label, t)| (label.clone(), t.shown())).collect();
                    continue;
                }
                ("page", Given::Int(n)) => {
                    let (index, at) = tables.page.unwrap_or((0, 0));
                    rows.page = Some(layout::Paging { size: (*n).max(1) as usize, index, at });
                    continue;
                }
                ("sort", Given::Bool(b)) => {
                    sort = *b;
                    continue;
                }
                _ => {}
            }
        }
        if k == "confirm" {
            confirm = Some(v.shown());
            continue;
        }
        if EVENTS.contains(&k.as_str()) {
            match v {
                Given::Handler(h) => events.push(Event { on: k, handler: h, args: None }),
                Given::Call(h, args) => events.push(Event { on: k, handler: h, args: Some(args) }),
                _ => {}
            }
            continue;
        }
        if k == "seek" {
            continue;
        }
        let Some(o) = opt_of(v) else { continue };
        opts.push((k, o));
    }
    if sort {
        let (index, by) = tables.sort.clone().unwrap_or((0, None));
        rows.sort = Some(layout::Sorting { index, by });
    }
    let mut kids: Vec<Arc<Node>> = Vec::with_capacity(children.len());
    let mut placed: Vec<Bind> = Vec::new();
    for (i, c) in children.into_iter().enumerate() {
        let before = kids.len();
        match c {
            // a table's frame is its rows
            Given::Frame(f) if table => kids.push(Arc::new(Node::Rows(Box::new(Rows { frame: *f, ..rows.clone() })))),
            other => splice(&mut kids, other),
        }
        if kids.len() == before + 1
            && matches!(kids[before].as_ref(), Node::Text(_) | Node::Control(_))
            && let Some((_, id)) = binds.iter().find(|(at, _)| *at == BindAt::Child(i))
        {
            placed.push(Bind::Child { at: before, id: *id });
        }
    }
    for (at, id) in binds {
        if let BindAt::Option(name) = at
            && option_binds(kind, name)
            && opts.iter().any(|(k, _)| k == name)
        {
            placed.push(Bind::Option { name: name.to_string(), id: *id });
        }
    }
    // the options the layout vocabulary refuses stop the render where the
    // view is built, as they always have
    let attrs: Vec<(String, String)> = opts.iter().map(|(k, v)| (k.clone(), v.shown())).collect();
    if let Some(e) = layout::render(kind, &attrs).errors.first() {
        return Err(format!("{}: {}", kind, e));
    }
    let bound_within = !placed.is_empty() || binds_inside(&kids);
    Ok(Node::Element(Element { kind: kind.to_string(), options: opts, events, confirm, children: kids, stale: false, binds: placed, bound_within }))
}

/// An option's value as an element keeps it: none for a value it leaves out.
fn opt_of(v: Given) -> Option<Opt> {
    Some(match v {
        Given::Text(s) | Given::Markup(s) => Opt::Text(s),
        Given::Int(n) => Opt::Int(n),
        Given::Float(x) => Opt::Float(x),
        Given::Bool(b) => Opt::Bool(b),
        _ => return None,
    })
}

/// A child placed: a node as it is, a list item by item, markup as markup,
/// a failed value as the text that stands for it, everything else (a frame
/// outside a table among them) as its text.
fn splice(kids: &mut Vec<Arc<Node>>, c: Given) {
    match c {
        Given::Node(n) => kids.push(n),
        Given::List(items) => {
            for it in items {
                splice(kids, it);
            }
        }
        Given::Markup(s) => kids.push(Arc::new(Node::Markup(s))),
        Given::Failed(why) => kids.push(Arc::new(Node::Failed(crate::core::response::failed_in_view(&why)))),
        other => kids.push(Arc::new(Node::Text(other.shown()))),
    }
}

/// `field[label, var, type, lines]`.
pub fn field(label: &str, bind: &str, value: serde_json::Value, shown: &str, kind: &str, lines: i64) -> Node {
    Node::Control(Control::Field { label: label.into(), bind: bind.into(), value, shown: shown.into(), kind: kind.into(), lines })
}

/// `slider[label, var, min, max, step]`.
pub fn slider(label: &str, bind: &str, value: serde_json::Value, shown: &str, min: f64, max: f64, step: f64) -> Node {
    Node::Control(Control::Slider { label: label.into(), bind: bind.into(), value, shown: shown.into(), min, max, step })
}

/// `checkbox[label, var]`, or `toggle[label, var]`.
pub fn check(label: &str, bind: &str, on: bool, toggle: bool) -> Node {
    Node::Control(Control::Check { label: label.into(), bind: bind.into(), on, toggle })
}

/// `radio[options, var]`, or `tabs[options, var]`.
pub fn choice(options: Vec<String>, bind: &str, current: &str, tabs: bool) -> Node {
    Node::Control(Control::Choice { options, bind: bind.into(), current: current.into(), tabs })
}

/// `select[options, var]`.
pub fn select(options: Vec<String>, bind: &str, current: &str) -> Node {
    Node::Control(Control::Select { options, bind: bind.into(), current: current.into() })
}

/// `progress[value, max]`.
pub fn progress(value: f64, max: f64) -> Node {
    Node::Control(Control::Progress { value, max })
}

/// `icon[name, size, color]`: a name the set does not hold, or a size or a
/// colour the vocabulary does not, is the render's stop.
pub fn icon(name: &str, size: &str, color: &str) -> Result<Node, String> {
    layout::icon_html(name, size, color).map_err(|e| format!("icon: {}", e))?;
    Ok(Node::Control(Control::Icon { name: name.into(), size: size.into(), color: color.into() }))
}

/// `widget[name, props, on]`. A name that is no module's is the text
/// that says so, in its place.
pub fn widget(module: &str, props: serde_json::Value, on: Vec<(String, String)>) -> Node {
    if module.is_empty() || !module.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Node::Text(format!("widget: '{}' is not a module name — letters, digits, _ and -", module));
    }
    Node::Widget(Widget { module: module.into(), props, on })
}

/// `chart[frame, x, candles, lines, bars, marker, tz, height, panes,
/// levels, points, seek]`, as it was given: the marker's time (NaN for
/// none), and the levels of the columns a pane draws, which are the ones it
/// draws them for.
#[allow(clippy::too_many_arguments)]
pub fn chart(
    frame: &CrowFrame,
    x: &str,
    candles: &[String],
    lines: &[String],
    bars: &str,
    marker: f64,
    tz: &str,
    height: f64,
    panes: &[Vec<String>],
    levels: &dyn crow_runtime::chart::LevelMap,
    points: Option<&CrowFrame>,
    seek: Option<&str>,
) -> Node {
    let mut kept: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for col in panes.iter().flatten() {
        let at = levels.levels_for(col);
        if !at.is_empty() {
            kept.insert(col.clone(), at);
        }
    }
    Node::Chart(Box::new(Chart {
        frame: frame.clone(),
        x: x.to_string(),
        candles: candles.to_vec(),
        lines: lines.to_vec(),
        bars: bars.to_string(),
        marker,
        tz: tz.to_string(),
        height,
        panes: panes.to_vec(),
        levels: kept,
        points: points.cloned(),
        seek: seek.map(str::to_string),
    }))
}

/// What a view shows in place of a value that failed.
pub fn failed(reason: &str) -> Node {
    Node::Failed(crate::core::response::failed_in_view(reason))
}

/// What a session holds of the tables a render draws: for a `table`
/// element given `page` (any value) the next paged table's place and the
/// page it is on, for one given `sort = true` the next sorted table's place
/// and its order — as the interpreter has always counted them.
pub fn table_state(
    kind: &str,
    options: &[(String, Given)],
    mut page: impl FnMut() -> (usize, usize),
    mut sort: impl FnMut() -> (usize, Option<(String, bool)>),
) -> TableState {
    let mut t = TableState::default();
    if kind != "table" {
        return t;
    }
    if options.iter().any(|(k, _)| k == "page") {
        t.page = Some(page());
    }
    if options.iter().any(|(k, v)| k == "sort" && matches!(v, Given::Bool(true))) {
        t.sort = Some(sort());
    }
    t
}

/// A view's result marked as read while a value it showed was still being
/// computed: an element says so itself; anything else is held in a `span`
/// that does.
pub fn stale(node: Node) -> Node {
    match node {
        Node::Element(mut e) => {
            e.stale = true;
            Node::Element(e)
        }
        Node::Html(v @ CrowVNode::Element { .. }) => Node::Html(v.attr("data-crow-stale", "")),
        other => {
            let children = vec![Arc::new(other)];
            let bound_within = binds_inside(&children);
            Node::Element(Element { kind: "span".to_string(), options: Vec::new(), events: Vec::new(), confirm: None, children, stale: true, binds: Vec::new(), bound_within })
        }
    }
}

/// The node with `child` as its last child, where the runtime draws on the
/// page (a toast, a dialog): inside the root element, or beside a page that
/// has none — as the page's nodes had it before views were trees.
pub fn with_child(node: Node, child: Node) -> Node {
    match node {
        Node::Element(mut e) => {
            e.children.push(Arc::new(child));
            Node::Element(e)
        }
        Node::Markup(m) => match m.rfind("</") {
            Some(at) if m.ends_with('>') => Node::Markup(format!("{}{}{}", &m[..at], html(&child), &m[at..])),
            _ => {
                let children = vec![Arc::new(Node::Markup(m)), Arc::new(child)];
                let bound_within = binds_inside(&children);
                Node::Element(Element { kind: "div".to_string(), options: Vec::new(), events: Vec::new(), confirm: None, children, stale: false, binds: Vec::new(), bound_within })
            }
        },
        // (a control, a chart, a widget at the root: their own nodes)
        other => Node::Html(match vnode(&other) {
            CrowVNode::Element { tag, attrs, mut children } => {
                children.push(vnode(&child));
                CrowVNode::Element { tag, attrs, children }
            }
            CrowVNode::Raw(m) if m.ends_with('>') && m.contains("</") => {
                let at = m.rfind("</").unwrap_or(0);
                CrowVNode::Raw(format!("{}{}{}", &m[..at], html(&child), &m[at..]))
            }
            v => CrowVNode::tag("div", vec![v, vnode(&child)]),
        }),
    }
}

// ---------------------------------------------------------------------
// The HTML renderer
// ---------------------------------------------------------------------

/// An element's tag and attributes as the page writes them: the layout
/// kernel's tag, class, style and data attributes, the wiring of its
/// events (a click that asks first offers the question), a tab stop for a
/// click the browser does not press itself; in name order.
pub(crate) fn written(e: &Element) -> (String, Vec<(String, String)>) {
    let attrs: Vec<(String, String)> = e.options.iter().map(|(k, v)| (k.clone(), v.shown())).collect();
    let rendered = layout::render(&e.kind, &attrs);
    let mut out = rendered.passthrough;
    if !rendered.class.is_empty() {
        out.push(("class".to_string(), rendered.class));
    }
    if !rendered.style.is_empty() {
        out.push(("style".to_string(), rendered.style));
    }
    out.extend(rendered.data);
    let mut wiring: Vec<(String, String)> = Vec::new();
    for ev in &e.events {
        wiring.push((format!("data-crow-{}", ev.on), ev.handler.clone()));
        if let Some(args) = &ev.args {
            wiring.push(("data-crow-args".to_string(), serde_json::Value::Array(args.clone()).to_string()));
        }
    }
    out.extend(layout::confirm_wiring(wiring, e.confirm.as_deref()));
    if out.iter().any(|(k, _)| k == "data-crow-click") {
        let extra = layout::press_attrs(&rendered.tag, &out);
        out.extend(extra);
    }
    if e.stale {
        out.push(("data-crow-stale".to_string(), String::new()));
    }
    out.sort();
    (rendered.tag, out)
}

/// The value of an element's `active` option (`nav[…, active = '/path']`).
fn active(e: &Element) -> Option<String> {
    e.options.iter().find(|(k, _)| k == "active").map(|(_, v)| v.shown())
}

/// The node as the page's markup: what the interpreter's view has always
/// answered (a view logged, or handed to `doc.document`, is its HTML).
pub fn html(node: &Node) -> String {
    let mut out = String::new();
    write_html(node, &mut out);
    out
}

fn write_html(node: &Node, out: &mut String) {
    match node {
        Node::Element(e) => {
            let (tag, attrs) = written(e);
            out.push('<');
            out.push_str(&tag);
            for (k, v) in &attrs {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                out.push_str(&escape_attr(v));
                out.push('"');
            }
            out.push('>');
            let lower = tag.to_ascii_lowercase();
            let nav = active(e).filter(|_| e.kind == "nav");
            let drops = super::DROPS_NEWLINE.contains(&lower.as_str());
            if nav.is_some() || drops {
                let mut inner = String::new();
                for c in &e.children {
                    write_html(c, &mut inner);
                }
                if let Some(path) = nav {
                    inner = layout::nav_active_html(&inner, &path);
                }
                // the browser drops the newline that opens a `pre` or a
                // `textarea`: one more keeps the text's own (as the page's
                // nodes write it, `CrowVNode::to_html`)
                if drops && inner.starts_with('\n') {
                    out.push('\n');
                }
                out.push_str(&inner);
            } else {
                for c in &e.children {
                    write_html(c, out);
                }
            }
            // a void element has no end tag
            if !layout::VOID.contains(&lower.as_str()) {
                out.push_str("</");
                out.push_str(&tag);
                out.push('>');
            }
        }
        Node::Text(t) => out.push_str(&escape_text(t)),
        Node::Markup(m) | Node::Failed(m) => out.push_str(m),
        Node::Rows(r) => {
            for p in layout::frame_table_parts(&table_of(r)) {
                p.write(out);
            }
        }
        // the kernel's own markup, as it writes it
        Node::Control(Control::Icon { name, size, color }) => match layout::icon_html(name, size, color) {
            Ok(svg) => out.push_str(&svg),
            Err(e) => out.push_str(&escape_text(&format!("error: {}", e))),
        },
        Node::Chart(c) => out.push_str(&chart_markup(c).to_html()),
        Node::Html(v) => out.push_str(&v.to_html()),
        other => out.push_str(&vnode(other).to_html()),
    }
}

/// What a table's rows node hands the kernel.
pub(crate) fn table_of(r: &Rows) -> layout::TableOf<'_> {
    layout::TableOf {
        frame: &r.frame,
        click: r.click.as_deref(),
        key: r.key.as_deref(),
        mark: r.mark.as_deref(),
        delta: &r.delta,
        actions: &r.actions,
        hide: &r.hide,
        confirm: &r.confirm,
        page: r.page,
        sort: r.sort.as_ref(),
    }
}

/// A chart as the kernel draws it: its SVG, or the text that says why not.
fn chart_markup(c: &Chart) -> CrowVNode {
    CrowVNode::chart_from_frame(&c.frame, &c.x, &c.candles, &c.lines, &c.bars, c.marker, &c.tz, c.height, &c.panes, &c.levels, c.points.as_ref(), c.seek.as_deref())
}

/// A part of the kernel's markup as the page's node, as the browser reads
/// that markup: its attributes as they read back, a text as written where
/// it holds an escape (`&quot;` stays `&quot;`), an empty one gone.
pub(crate) fn part_node(p: &layout::Part) -> CrowVNode {
    let children = p
        .kids
        .iter()
        .filter_map(|k| match k {
            layout::Kid::Part(q) => Some(part_node(q)),
            layout::Kid::Text(t) if t.is_empty() => None,
            layout::Kid::Text(t) => {
                let written = layout::escape(t);
                Some(if written.contains('&') { CrowVNode::Raw(written) } else { CrowVNode::Text(t.clone()) })
            }
        })
        .collect();
    CrowVNode::Element { tag: p.tag.to_string(), attrs: p.attrs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(), children }
}

/// A drawing the kernel wrote as markup (a chart's SVG), read once into
/// the page's nodes, as the browser reads it; markup it would repair stays
/// markup.
fn read_drawing(v: CrowVNode) -> CrowVNode {
    match v {
        CrowVNode::Raw(svg) => {
            let (mut nodes, repaired) = super::read_markup(&svg);
            if repaired.is_none() && nodes.len() == 1 { nodes.pop().unwrap_or(CrowVNode::Raw(svg)) } else { CrowVNode::Raw(svg) }
        }
        other => other,
    }
}

/// The links of a `nav[…, active = path]` that go to `path` marked as the
/// page the reader is on (`aria-current="page"`), as the kernel marks the
/// nav's markup (`layout::nav_active_html`): a link whose start tag, as
/// written, names that `href` and is not marked already.
fn mark_current(nodes: &mut [CrowVNode], path: &str) {
    if path.is_empty() {
        return;
    }
    let needle = format!("href=\"{}\"", path);
    for n in nodes.iter_mut() {
        match n {
            CrowVNode::Element { tag, attrs, children } => {
                if tag == "a" && !attrs.is_empty() {
                    let mut sorted: Vec<(&String, &String)> = attrs.iter().collect();
                    sorted.sort();
                    let start: String = sorted.iter().map(|(k, v)| format!(" {}=\"{}\"", k, escape_attr(v))).collect();
                    if start.contains(&needle) && !start.contains("aria-current") {
                        attrs.insert("aria-current".to_string(), "page".to_string());
                    }
                }
                mark_current(children, path);
            }
            CrowVNode::Raw(m) => *m = layout::nav_active_html(m, path),
            CrowVNode::Text(_) => {}
            // a shared node that holds the link is the page's own from here
            CrowVNode::Shared(sh) => {
                if sh.html().contains(&format!("href=\"{}\"", escape_attr(path))) || sh.html().contains(&needle) {
                    let mut own = vec![sh.node().clone()];
                    mark_current(&mut own, path);
                    if let Some(o) = own.pop() {
                        *n = o;
                    }
                }
            }
        }
    }
}

/// The node as the page's nodes, for the live runtime: an element as an
/// element, a table's rows, an icon and a chart as the elements their
/// markup reads back as, a control and a widget as their builders make
/// them, markup as markup. An element whose attributes name one twice (a
/// tip beside a title) is its markup, which keeps both, as the page has
/// always had them.
pub fn vnode(node: &Node) -> CrowVNode {
    match node {
        // (rows stand inside a table; alone, they are its markup)
        Node::Rows(_) => CrowVNode::Raw(html(node)),
        other => {
            let mut out = Vec::with_capacity(1);
            push_vnode(other, &mut out);
            out.pop().unwrap_or_else(|| CrowVNode::text(""))
        }
    }
}

/// The page's nodes a served session made of the subtrees it holds more
/// than once — a kept view's (live/views.rs) — by the subtree, so a kept
/// view's nodes are made once and shared from render to render
/// (`CrowVNode::Shared`), and the diff passes over them.
#[derive(Default)]
pub struct VnodeMemo {
    made: HashMap<usize, (Arc<Node>, Vec<CrowVNode>)>,
    used: std::collections::HashSet<usize>,
}

impl std::fmt::Debug for VnodeMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VnodeMemo({} subtrees)", self.made.len())
    }
}

thread_local! {
    static MEMO: std::cell::RefCell<Option<VnodeMemo>> = const { std::cell::RefCell::new(None) };
}

/// A tree's size, as a session's budget for the views it keeps counts it:
/// an element, a text, a control, a chart or a widget is one, a table's
/// rows one each.
pub fn weight(node: &Node) -> usize {
    match node {
        Node::Element(e) => 1 + e.children.iter().map(|c| weight(c)).sum::<usize>(),
        Node::Rows(r) => 1 + r.frame.nrows(),
        _ => 1,
    }
}

/// `vnode`, with the subtrees `memo` made last time shared, and what it
/// makes of the subtrees held elsewhere too (a kept view's) kept for next
/// time; a subtree neither this render nor a kept view holds is let go.
pub fn vnode_memo(node: &Node, memo: &mut VnodeMemo) -> CrowVNode {
    memo.used.clear();
    MEMO.with(|m| *m.borrow_mut() = Some(std::mem::take(memo)));
    let out = vnode(node);
    if let Some(back) = MEMO.with(|m| m.borrow_mut().take()) {
        *memo = back;
    }
    let used = std::mem::take(&mut memo.used);
    // (one a kept view still holds, which a later render may place again)
    memo.made.retain(|k, (held, _)| used.contains(k) || Arc::strong_count(held) > 1);
    memo.used = used;
    out
}

/// A child's nodes from the memo, or made and kept there: only a subtree
/// something else holds too (a kept view's), and only while `vnode_memo`
/// runs. False when the child is not one.
fn memo_push(c: &Arc<Node>, out: &mut Vec<CrowVNode>) -> bool {
    if Arc::strong_count(c) < 2 || !matches!(**c, Node::Element(_) | Node::Rows(_)) {
        return false;
    }
    let key = Arc::as_ptr(c) as usize;
    let found = MEMO.with(|m| {
        let mut m = m.borrow_mut();
        let memo = m.as_mut()?;
        memo.used.insert(key);
        Some(memo.made.get(&key).map(|(_, parts)| parts.clone()))
    });
    match found {
        None => false,
        Some(Some(parts)) => {
            out.extend(parts);
            true
        }
        Some(None) => {
            let mut parts = Vec::new();
            push_vnode(c, &mut parts);
            let parts: Vec<CrowVNode> = parts.into_iter().map(|p| if matches!(p, CrowVNode::Element { .. }) { CrowVNode::Shared(super::Shared::new(p)) } else { p }).collect();
            MEMO.with(|m| {
                if let Some(memo) = m.borrow_mut().as_mut() {
                    memo.made.insert(key, (c.clone(), parts.clone()));
                }
            });
            out.extend(parts);
            true
        }
    }
}

fn push_vnode(node: &Node, out: &mut Vec<CrowVNode>) {
    match node {
        Node::Element(e) => {
            let (tag, attrs) = written(e);
            let mut map: HashMap<String, String> = HashMap::with_capacity(attrs.len());
            for (k, v) in attrs {
                if map.insert(k, v).is_some() {
                    out.push(CrowVNode::Raw(html(node)));
                    return;
                }
            }
            let mut children = Vec::with_capacity(e.children.len());
            for c in &e.children {
                if !memo_push(c, &mut children) {
                    push_vnode(c, &mut children);
                }
            }
            if let Some(path) = active(e).filter(|_| e.kind == "nav") {
                mark_current(&mut children, &path);
            }
            out.push(CrowVNode::Element { tag, attrs: map, children });
        }
        Node::Text(t) => out.push(CrowVNode::Text(t.clone())),
        Node::Markup(m) => out.push(CrowVNode::Raw(m.clone())),
        Node::Failed(t) => out.push(CrowVNode::Text(t.clone())),
        Node::Html(v) => out.push(v.clone()),
        Node::Rows(r) => out.extend(layout::frame_table_parts(&table_of(r)).iter().map(part_node)),
        Node::Control(c) => out.push(match c {
            Control::Field { label, bind, shown, kind, lines, .. } => CrowVNode::field_typed(label, bind, shown, kind, *lines),
            Control::Slider { label, bind, shown, min, max, step, .. } => CrowVNode::slider(label, bind, shown, *min, *max, *step),
            Control::Check { label, bind, on, toggle } => CrowVNode::check_input(label, bind, *on, *toggle),
            Control::Choice { options, bind, current, tabs } => CrowVNode::choice_input(options, bind, current, *tabs),
            Control::Select { options, bind, current } => CrowVNode::select_input(options, bind, current),
            Control::Progress { value, max } => CrowVNode::progress(*value, *max),
            Control::Icon { name, size, color } => match layout::icon_part(name, size, color) {
                Ok(p) => part_node(&p),
                Err(e) => CrowVNode::text(&format!("error: {}", e)),
            },
        }),
        Node::Chart(c) => out.push(read_drawing(chart_markup(c))),
        Node::Widget(w) => {
            let pairs: Vec<(&str, &str)> = w.on.iter().map(|(e, h)| (e.as_str(), h.as_str())).collect();
            out.push(CrowVNode::widget(&w.module, &w.props, &pairs));
        }
    }
}

/// The page's head taken out of a render (`CrowVNode::split_head`, over the
/// tree): each `title` and `meta` element it places anywhere, markup
/// included, lands in the head, and the rest is the body.
pub fn split_head(node: Node) -> (Node, super::Head) {
    let mut head = super::Head::default();
    let body = match strip(&node, &mut head) {
        Stripped::Same => node,
        Stripped::Into(n) => n,
        Stripped::Gone => Node::Text(String::new()),
    };
    (body, head)
}

enum Stripped {
    Same,
    Into(Node),
    Gone,
}

fn strip(node: &Node, head: &mut super::Head) -> Stripped {
    match node {
        Node::Element(e) if e.kind.eq_ignore_ascii_case("title") => {
            head.title = Some(e.children.iter().map(|c| text_of(c)).collect());
            Stripped::Gone
        }
        Node::Element(e) if e.kind.eq_ignore_ascii_case("meta") => {
            head.metas.push(written(e).1);
            Stripped::Gone
        }
        Node::Element(e) => {
            let mut changed = false;
            let mut kids: Vec<Arc<Node>> = Vec::with_capacity(e.children.len());
            // where each child is among those left (a bound one keeps its place)
            let mut now_at: Vec<Option<usize>> = Vec::with_capacity(e.children.len());
            for c in &e.children {
                match strip(c, head) {
                    Stripped::Same => {
                        now_at.push(Some(kids.len()));
                        kids.push(c.clone());
                    }
                    Stripped::Into(n) => {
                        changed = true;
                        now_at.push(Some(kids.len()));
                        kids.push(Arc::new(n));
                    }
                    Stripped::Gone => {
                        changed = true;
                        now_at.push(None);
                    }
                }
            }
            if !changed {
                return Stripped::Same;
            }
            let mut bare = e.clone_bare();
            bare.binds = e
                .binds
                .iter()
                .filter_map(|b| match b {
                    Bind::Child { at, id } => now_at.get(*at).copied().flatten().map(|at| Bind::Child { at, id: *id }),
                    other => Some(other.clone()),
                })
                .collect();
            bare.bound_within = !bare.binds.is_empty() || binds_inside(&kids);
            Stripped::Into(Node::Element(Element { children: kids, ..bare }))
        }
        // markup and the page's own nodes: as the nodes are split
        Node::Markup(m) if m.contains("<title") || m.contains("<meta") => match CrowVNode::Raw(m.clone()).split_into(head) {
            Some(CrowVNode::Raw(r)) if &r == m => Stripped::Same,
            Some(CrowVNode::Raw(r)) => Stripped::Into(Node::Markup(r)),
            Some(v) => Stripped::Into(Node::Html(v)),
            None => Stripped::Gone,
        },
        Node::Html(v) => match v.clone().split_into(head) {
            Some(n) => Stripped::Into(Node::Html(n)),
            None => Stripped::Gone,
        },
        _ => Stripped::Same,
    }
}

impl Element {
    /// The element without its children (they are given again).
    fn clone_bare(&self) -> Element {
        Element { kind: self.kind.clone(), options: self.options.clone(), events: self.events.clone(), confirm: self.confirm.clone(), children: Vec::new(), stale: self.stale, binds: self.binds.clone(), bound_within: self.bound_within }
    }
}

/// The text a title's children hold, as a browser reads it.
fn text_of(node: &Node) -> String {
    match node {
        Node::Text(t) | Node::Failed(t) => t.clone(),
        Node::Element(e) => e.children.iter().map(|c| text_of(c)).collect(),
        other => super::text_of(&[vnode(other)]),
    }
}

// ---------------------------------------------------------------------
// The tree as data (one-view-design.md, "The tree as data")
// ---------------------------------------------------------------------

/// The node as data, for a renderer that draws it: every kind, option,
/// event, binding, value and child, and nothing of HTML's.
pub fn to_json(node: &Node) -> serde_json::Value {
    use serde_json::{json, Map, Value};
    match node {
        Node::Element(e) => {
            let options: Map<String, Value> = e.options.iter().map(|(k, v)| (k.clone(), v.json())).collect();
            let events: Vec<Value> = e.events.iter().map(event_json).collect();
            let mut out = json!({
                "node": "element",
                "kind": e.kind,
                "options": options,
                "events": events,
                "children": e.children.iter().map(|c| to_json(c)).collect::<Vec<_>>(),
            });
            if let Some(text) = &e.confirm {
                out["confirm"] = json!(text);
                // what the click that asks first sends, as the page's
                // wiring writes it (`layout::confirm_wiring`)
                if let Some(click) = e.events.iter().find(|ev| ev.on == "click") {
                    let args = click.args.as_ref().map(|a| Value::Array(a.clone()).to_string());
                    out["ask"] = json!(layout::ask_args(&click.handler, args.as_deref(), text));
                }
            }
            if e.stale {
                out["stale"] = json!(true);
            }
            if !e.binds.is_empty() {
                // which places are bindings, by id: a drawn renderer sets
                // them from a `values` message (one-view-design.md 5a)
                out["binds"] = Value::Array(
                    e.binds
                        .iter()
                        .map(|b| match b {
                            Bind::Child { at, id } => json!({ "child": at, "id": id }),
                            Bind::Option { name, id } => json!({ "option": name, "id": id }),
                        })
                        .collect(),
                );
            }
            out
        }
        Node::Rows(r) => rows_json(r),
        Node::Control(c) => {
            let mut out = match c {
                Control::Field { label, bind, value, kind, lines, .. } => json!({ "label": label, "bind": bind, "value": value, "type": if kind.is_empty() { "text" } else { kind }, "lines": lines }),
                Control::Slider { label, bind, value, min, max, step, .. } => json!({ "label": label, "bind": bind, "value": value, "min": min, "max": max, "step": step }),
                Control::Check { label, bind, on, .. } => json!({ "label": label, "bind": bind, "value": on }),
                Control::Choice { options, bind, current, .. } | Control::Select { options, bind, current } => json!({ "options": options, "bind": bind, "value": current }),
                Control::Progress { value, max } => json!({ "value": value, "max": max }),
                Control::Icon { name, size, color } => json!({ "name": name, "size": size, "color": color }),
            };
            out["node"] = json!("control");
            out["kind"] = json!(c.kind());
            out
        }
        Node::Chart(c) => chart_json(c),
        Node::Widget(w) => {
            let on: Map<String, Value> = w.on.iter().map(|(e, h)| (e.clone(), json!(h))).collect();
            json!({ "node": "widget", "module": w.module, "props": w.props, "on": on })
        }
        Node::Text(t) => json!({ "node": "text", "text": t }),
        // (markup, and the page's own nodes made as HTML, are as they are
        // written, and read back as the tree's own kinds where they are
        // what the vocabulary writes: `read`)
        Node::Markup(m) => json!({ "node": "markup", "html": m, "read": read_back(&super::parse_markup(m)) }),
        Node::Html(v) => json!({ "node": "markup", "html": v.to_html(), "read": read_back(std::slice::from_ref(v)) }),
        Node::Failed(t) => json!({ "node": "failed", "text": t }),
    }
}

// ---------------------------------------------------------------------
// Markup read back (one-view-design.md, phase 4)
// ---------------------------------------------------------------------

/// Markup as the tree's data, for a renderer that draws the page and reads
/// no HTML: what the vocabulary wrote read back as its own kinds — a bound
/// field, a check, a choice, a select, a slider, a progress bar, an icon,
/// a frame's table (its rows, row clicks, actions, sorted headers and
/// pager, as `rows` says them) — and any other element as its tag (a
/// layout kind's `div` as that kind), with its wiring as events and what
/// its class and style say as options. A compiled page hands a node it
/// kept in a variable on as markup (§"What is still markup"); this is what
/// lets a drawn page draw it.
pub fn read_back(nodes: &[CrowVNode]) -> Vec<serde_json::Value> {
    nodes.iter().filter_map(read_node).collect()
}

const LAYOUT_KINDS: &[&str] = &["row", "stack", "grid", "scroll", "spacer", "center", "overlay", "modal", "drawer", "toast", "card", "pill", "badge"];

fn read_node(v: &CrowVNode) -> Option<serde_json::Value> {
    use serde_json::json;
    match v {
        CrowVNode::Text(t) => Some(json!({ "node": "text", "text": t })),
        CrowVNode::Raw(r) => {
            if r.contains('<') {
                Some(json!({ "node": "markup", "html": r }))
            } else {
                Some(json!({ "node": "text", "text": super::text_of(std::slice::from_ref(v)) }))
            }
        }
        CrowVNode::Shared(s) => read_node(s.node()),
        CrowVNode::Element { tag, attrs, children } => {
            let mut e = read_element(tag, attrs, children);
            // (a picture the markup draws itself — an `svg` that is not an
            // icon — carries itself as an image's address, which a drawn
            // renderer loads as a texture: it has no HTML of its own)
            if tag == "svg" && e["node"] == "element" {
                e["options"]["src"] = serde_json::json!(svg_address(&v.to_html()));
            }
            Some(e)
        }
    }
}

/// An `svg`'s markup as a `data:` address an image is loaded from.
fn svg_address(markup: &str) -> String {
    let mut out = String::from("data:image/svg+xml;charset=utf-8,");
    let mut markup = markup.to_string();
    if !markup.contains("xmlns=") {
        markup = markup.replacen("<svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"", 1);
    }
    for b in markup.bytes() {
        if b.is_ascii_alphanumeric() || b" -_.!~*'()/:;=,".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn classes(attrs: &HashMap<String, String>) -> Vec<&str> {
    attrs.get("class").map(|c| c.split_whitespace().collect()).unwrap_or_default()
}

/// The elements under `kids` with `tag`, depth first.
fn find_all<'a>(kids: &'a [CrowVNode], tag: &str, out: &mut Vec<(&'a HashMap<String, String>, &'a [CrowVNode])>) {
    for k in kids {
        match k {
            CrowVNode::Element { tag: t, attrs, children } => {
                if t == tag {
                    out.push((attrs, children));
                }
                find_all(children, tag, out);
            }
            CrowVNode::Shared(s) => find_all(std::slice::from_ref(s.node()), tag, out),
            _ => {}
        }
    }
}

fn first<'a>(kids: &'a [CrowVNode], tag: &str) -> Option<(&'a HashMap<String, String>, &'a [CrowVNode])> {
    let mut all = Vec::new();
    find_all(kids, tag, &mut all);
    all.into_iter().next()
}

fn words(kids: &[CrowVNode]) -> String {
    super::text_of(kids).trim().to_string()
}

fn bound(attrs: &HashMap<String, String>) -> String {
    let wired = attrs.get("data-crow-input").or_else(|| attrs.get("data-crow-change")).cloned().unwrap_or_default();
    wired.strip_prefix("__field:").or_else(|| wired.strip_prefix("__select:")).unwrap_or(&wired).to_string()
}

fn number(attrs: &HashMap<String, String>, name: &str, otherwise: f64) -> f64 {
    attrs.get(name).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(otherwise)
}

fn read_element(tag: &str, attrs: &HashMap<String, String>, children: &[CrowVNode]) -> serde_json::Value {
    use serde_json::{json, Map, Value};
    let class = classes(attrs);
    let has = |c: &str| class.contains(&c);
    // the vocabulary's controls, as their builders write them
    if tag == "label"
        && has("crow-check")
        && let Some((input, _)) = first(children, "input")
    {
        let kind = if has("crow-toggle") { "toggle" } else { "checkbox" };
        return json!({ "node": "control", "kind": kind, "label": words(children), "bind": bound(input), "value": input.contains_key("checked") });
    }
    if tag == "div" && (has("crow-radio") || has("crow-tabs")) {
        let mut inputs = Vec::new();
        find_all(children, "input", &mut inputs);
        let options: Vec<String> = inputs.iter().map(|(a, _)| a.get("value").cloned().unwrap_or_default()).collect();
        let current = inputs.iter().find(|(a, _)| a.contains_key("checked")).and_then(|(a, _)| a.get("value").cloned()).unwrap_or_default();
        let bind = inputs.first().map(|(a, _)| bound(a)).unwrap_or_default();
        let kind = if has("crow-tabs") { "tabs" } else { "radio" };
        return json!({ "node": "control", "kind": kind, "options": options, "bind": bind, "value": current });
    }
    if tag == "div"
        && has("crow-select")
        && let Some((sel, opts)) = first(children, "select")
    {
        let mut all = Vec::new();
        find_all(opts, "option", &mut all);
        let options: Vec<String> = all.iter().map(|(a, k)| a.get("value").cloned().unwrap_or_else(|| words(k))).collect();
        let current = all.iter().find(|(a, _)| a.contains_key("selected")).map(|(a, k)| a.get("value").cloned().unwrap_or_else(|| words(k))).unwrap_or_default();
        return json!({ "node": "control", "kind": "select", "options": options, "bind": bound(sel), "value": current });
    }
    if tag == "div" && has("crow-field") {
        let label = first(children, "label").map(|(_, k)| words(k)).unwrap_or_default();
        if let Some((input, _)) = first(children, "input") {
            let kind = input.get("type").cloned().unwrap_or_else(|| "text".to_string());
            let value = input.get("value").cloned().unwrap_or_default();
            if has("crow-slider") || kind == "range" {
                let v = value.trim().parse::<f64>().map(|x| json!(x)).unwrap_or(json!(value));
                return json!({ "node": "control", "kind": "slider", "label": label, "bind": bound(input), "value": v, "min": number(input, "min", 0.0), "max": number(input, "max", 100.0), "step": number(input, "step", 0.0) });
            }
            if !input.contains_key("data-crow-upload") {
                return json!({ "node": "control", "kind": "field", "label": label, "bind": bound(input), "value": value, "type": kind, "lines": 0 });
            }
        }
        if let Some((area, k)) = first(children, "textarea") {
            let lines = area.get("rows").and_then(|r| r.parse::<i64>().ok()).unwrap_or(3);
            return json!({ "node": "control", "kind": "field", "label": label, "bind": bound(area), "value": super::text_of(k), "type": "text", "lines": lines });
        }
    }
    if tag == "progress" && has("crow-progress") {
        return json!({ "node": "control", "kind": "progress", "value": number(attrs, "value", 0.0), "max": number(attrs, "max", 1.0) });
    }
    if tag == "svg"
        && has("icon")
        && let Some(name) = class.iter().find_map(|c| c.strip_prefix("icon-"))
    {
        return json!({ "node": "control", "kind": "icon", "name": name, "size": number(attrs, "width", 20.0), "color": "" });
    }
    if tag == "table"
        && let Some(rows) = read_table(children)
    {
        return json!({ "node": "element", "kind": "table", "options": {}, "events": [], "children": [rows] });
    }
    // any other element: a layout kind's div as that kind, else its tag
    let kind = match class.first() {
        Some(c) if (tag == "div" || tag == "span") && LAYOUT_KINDS.contains(c) => c.to_string(),
        _ => tag.to_string(),
    };
    let mut options = Map::new();
    if has("card") {
        options.insert("surface".into(), json!("card"));
    }
    // (its own `id` too: an element a view made in a loop is read back from
    // its markup, and a design's id on it must reach a drawn page as it
    // reaches the HTML one)
    for (a, o) in [("href", "href"), ("title", "tip"), ("role", "role"), ("alt", "alt"), ("aria-label", "label"), ("src", "src"), ("id", "id")] {
        if let Some(v) = attrs.get(a) {
            options.insert(o.into(), json!(v));
        }
    }
    for a in ["disabled", "open"] {
        if attrs.contains_key(a) {
            options.insert(a.into(), json!(true));
        }
    }
    for c in ["up", "down"] {
        if has(c) {
            options.insert("color".into(), json!(if c == "up" { "success" } else { "danger" }));
        }
    }
    if let Some(style) = attrs.get("style") {
        for part in style.split(';') {
            let Some((k, v)) = part.split_once(':') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "color" => {
                    if let Some(name) = v.strip_prefix("var(--crow-").and_then(|r| r.strip_suffix(')')) {
                        options.insert("color".into(), json!(name));
                    }
                }
                "font-weight" if v == "700" || v == "bold" => {
                    options.insert("weight".into(), json!("bold"));
                }
                "font-size" => {
                    options.insert("size".into(), json!(v));
                }
                "border" => {
                    options.insert("border".into(), json!(true));
                }
                "gap" | "width" | "height" => {
                    options.insert(k.into(), json!(v));
                }
                "padding" => {
                    options.insert("pad".into(), json!(v));
                }
                "flex-wrap" if v == "wrap" => {
                    options.insert("wrap".into(), json!(true));
                }
                "justify-content" => {
                    options.insert("justify".into(), json!(v));
                }
                "align-items" => {
                    options.insert("align".into(), json!(v));
                }
                "grid-template-columns" => {
                    if let Some(n) = v.strip_prefix("repeat(").and_then(|r| r.split(',').next()).and_then(|n| n.trim().parse::<i64>().ok()) {
                        options.insert("columns".into(), json!(n));
                    }
                }
                _ => {}
            }
        }
    }
    // (a picture's size as its markup gives it, where no style did)
    if tag == "img" || tag == "svg" {
        for a in ["width", "height"] {
            if let Some(v) = attrs.get(a)
                && !options.contains_key(a)
            {
                let v = v.trim();
                let v = if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.') { format!("{}px", v) } else { v.to_string() };
                options.insert(a.into(), json!(v));
            }
        }
    }
    let mut events = Vec::new();
    for (a, on) in [("data-crow-click", "click"), ("data-crow-submit", "submit"), ("data-crow-change", "change"), ("data-crow-input", "input")] {
        if let Some(h) = attrs.get(a) {
            let mut e = json!({ "on": on, "handler": h });
            if on == "click"
                && let Some(args) = attrs.get("data-crow-args")
            {
                if let Ok(Value::Array(parsed)) = serde_json::from_str::<Value>(args) {
                    e["args"] = Value::Array(parsed);
                }
                e["value"] = json!(args);
            }
            events.push(e);
        }
    }
    json!({ "node": "element", "kind": kind, "options": options, "events": events, "children": read_back(children) })
}

/// A frame's table as `frame_table_parts` writes it, read back as `rows`
/// says one: the headers, each row's cells and sends (its click, then its
/// actions), the sorted headers' values and the pager's. None for a table
/// it did not write.
fn read_table(kids: &[CrowVNode]) -> Option<serde_json::Value> {
    use serde_json::{json, Value};
    let (_, head) = first(kids, "thead")?;
    let (_, body) = first(kids, "tbody")?;
    let mut ths = Vec::new();
    find_all(head, "th", &mut ths);
    let mut cols = Vec::new();
    let mut sorts = Vec::new();
    let mut sort = Value::Null;
    for (a, k) in &ths {
        let name = words(k);
        if name.is_empty() {
            continue;
        }
        let numeric = classes(a).contains(&"num");
        if let Some((b, _)) = first(k, "button")
            && let Some(args) = b.get("data-crow-args")
        {
            sorts.push(json!([name, args]));
            let index = serde_json::from_str::<Value>(args).ok().and_then(|v| v.get(0).cloned()).unwrap_or(json!(0));
            if sort.is_null() {
                sort = json!({ "index": index });
            }
            if let Some(how) = a.get("aria-sort") {
                sort = json!({ "index": index, "by": name, "desc": how == "descending" });
            }
        }
        cols.push(json!({ "name": name, "numeric": numeric }));
    }
    let mut shown = Vec::new();
    let mut sends = Vec::new();
    let mut click = Value::Null;
    let mut actions: Vec<Value> = Vec::new();
    let mut refused = false;
    for k in body {
        let CrowVNode::Element { tag, attrs, children } = k else { continue };
        if tag != "tr" {
            continue;
        }
        let mut cells = Vec::new();
        let mut pairs = Vec::new();
        if let Some(h) = attrs.get("data-crow-click") {
            click = json!(h);
            pairs.push(json!([h, attrs.get("data-crow-args").cloned().unwrap_or_default()]));
        }
        for td in children {
            let CrowVNode::Element { tag, attrs: ta, children: tk } = td else { continue };
            if tag != "td" {
                continue;
            }
            let tc = classes(ta);
            if tc.contains(&"crow-refused") {
                refused = true;
                continue;
            }
            if tc.contains(&"actions") {
                let mut buttons = Vec::new();
                find_all(tk, "button", &mut buttons);
                let first_row = actions.is_empty();
                for (b, bk) in buttons {
                    let h = b.get("data-crow-click").cloned().unwrap_or_default();
                    pairs.push(json!([h, b.get("data-crow-args").cloned().unwrap_or_default()]));
                    if first_row {
                        actions.push(json!({ "label": words(bk), "handler": h }));
                    }
                }
                continue;
            }
            cells.push(json!(super::text_of(tk)));
        }
        shown.push(Value::Array(cells));
        sends.push(Value::Array(pairs));
    }
    let mut out = json!({
        "node": "rows",
        "frame": { "columns": cols, "rows": shown.clone() },
        "total": shown.len(),
        "delta": [],
        "hide": [],
        "actions": actions,
        "confirm": {},
        "sends": sends,
        "shown": shown,
        "sorts": sorts,
    });
    if refused {
        out["refused"] = json!(true);
    }
    if !click.is_null() {
        out["click"] = click;
    }
    if !sort.is_null() {
        out["sort"] = sort;
    }
    // the pager: its steps' values and the rows it says it shows
    if let Some((_, foot)) = first(kids, "tfoot") {
        let mut buttons = Vec::new();
        find_all(foot, "button", &mut buttons);
        let step = |i: usize| buttons.get(i).and_then(|(b, _)| b.get("data-crow-args").cloned()).map(Value::String).unwrap_or(Value::Null);
        out["steps"] = json!([step(0), step(1)]);
        let said = first(foot, "span").map(|(_, k)| words(k)).unwrap_or_default();
        // ("1–20 of 419": the first row shown and how many there are)
        let nums: Vec<usize> = said.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect();
        let (at, total) = match nums.as_slice() {
            [first, _, total] => (first.saturating_sub(1), *total),
            _ => (0, shown.len()),
        };
        let index = buttons.iter().find_map(|(b, _)| b.get("data-crow-args")).and_then(|a| serde_json::from_str::<Value>(a).ok()).and_then(|v| v.get(0).cloned()).unwrap_or(json!(0));
        out["total"] = json!(total);
        out["page"] = json!({ "size": shown.len().max(1), "index": index, "at": at });
    }
    Some(out)
}

/// A chart as data: what `chart[…]` was given, with its x column's moments
/// as seconds (`ts`) and, when it seeks, what each bucket's press sends
/// (`seeks`, as the chart's seek bands carry it: `chart::seek_clicks`).
fn chart_json(c: &Chart) -> serde_json::Value {
    use serde_json::{json, Value};
    let ts = c.frame.col_native(&c.x).map(|col| col.to_f64_vec()).unwrap_or_default();
    let mut out = json!({
            "node": "chart",
            "frame": frame_json(&c.frame, None),
            "x": c.x,
            "candles": c.candles,
            "lines": c.lines,
            "bars": c.bars,
            "marker": if c.marker.is_nan() { Value::Null } else { json!(c.marker) },
            "tz": c.tz,
            "height": c.height,
            "panes": c.panes,
            "levels": c.levels,
            "points": c.points.as_ref().map(|p| frame_json(p, None)).unwrap_or(Value::Null),
            "seek": c.seek,
            "ts": ts.iter().map(|t| crow_runtime::json::float_to_json(*t)).collect::<Vec<_>>(),
    });
    if let Some(h) = &c.seek {
        let spec = crow_runtime::chart::ChartSpec {
            ts: &ts,
            candles: (&[], &[], &[], &[]),
            lines: vec![],
            volume: &[],
            marker: f64::NAN,
            tz: &c.tz,
            width: 0.0,
            height: 0.0,
            panes: vec![],
            points: vec![],
            seek: Some(h),
        };
        out["seeks"] = Value::Array(crow_runtime::chart::seek_clicks(&spec).into_iter().map(|(_, v)| Value::String(v)).collect());
    }
    out
}

/// A bound place's value as data, for a drawn renderer's `values` message
/// (one-view-design.md 5a): a child's value is the node it places, as
/// `to_json` gives it (a text, a control); an option's is the option's
/// value, none where the option is left out.
pub fn bound_json(b: &Bound) -> serde_json::Value {
    match b {
        Bound::Text(t) => serde_json::json!({ "node": "text", "text": t }),
        Bound::Node(n) => to_json(n),
        Bound::Opt(Some(o)) => o.json(),
        Bound::Opt(None) => serde_json::Value::Null,
    }
}

// ---------------------------------------------------------------------
// What a drawn renderer is sent (one-view-design.md 5a, phase 4)
// ---------------------------------------------------------------------

/// The node as a drawn renderer is sent it: `to_json`, less what such a
/// renderer never reads. A markup node read back (`read`) is sent without
/// its `html`, which only a renderer that parses HTML could use, and a
/// table's rows that carry each cell as the page shows it (`shown`) are
/// sent without the frame's own cells beside them (its columns stay).
pub fn drawn_json(node: &Node) -> serde_json::Value {
    let mut v = to_json(node);
    drawn_trim(&mut v);
    v
}

/// A bound place's value as a drawn renderer is sent it (`bound_json`,
/// trimmed as `drawn_json` trims a node).
pub fn drawn_bound_json(b: &Bound) -> serde_json::Value {
    let mut v = bound_json(b);
    drawn_trim(&mut v);
    v
}

fn drawn_trim(v: &mut serde_json::Value) {
    let Some(o) = v.as_object_mut() else { return };
    match o.get("node").and_then(|n| n.as_str()) {
        Some("markup") if o.contains_key("read") => {
            o.remove("html");
        }
        Some("rows") if o.contains_key("shown") => {
            if let Some(f) = o.get_mut("frame").and_then(|f| f.as_object_mut()) {
                f.remove("rows");
            }
        }
        _ => {}
    }
    for key in ["children", "read"] {
        if let Some(kids) = o.get_mut(key).and_then(|k| k.as_array_mut()) {
            for k in kids {
                drawn_trim(k);
            }
        }
    }
}

/// The list a node holds its children in, as a drawn renderer reads them:
/// an element's `children`, and a markup node's `read` (drawn as a block
/// of what was read back). Nothing else holds nodes.
fn kids_key(v: &serde_json::Value) -> Option<&'static str> {
    match v.get("node")?.as_str()? {
        "element" => Some("children"),
        "markup" if v.get("read").is_some_and(|r| r.is_array()) => Some("read"),
        _ => None,
    }
}

/// The node's own fields, its children left out.
fn own_of(v: &serde_json::Value, key: &str) -> serde_json::Value {
    let mut o = v.as_object().cloned().unwrap_or_default();
    o.remove(key);
    serde_json::Value::Object(o)
}

/// What changed from one tree a drawn renderer holds to the next, as the
/// changes that make the one the other, in order. A change names the node
/// it is made at by its path (`at`: child indices from the root, through
/// an element's `children` and a markup node's `read`) and is one of:
///
/// - `{"at": path, "node": n}`: the node there replaced by `n`;
/// - `{"at": path, "own": n}`: its own fields (kind, options, events,
///   bindings, the question its click asks) set from `n`, its children
///   kept;
/// - `{"at": path, "splice": [from, drop], "put": [n, …]}`: of its
///   children, `drop` taken out at `from` and the nodes given put there.
///
/// Children are matched by place where their count is the same, else by
/// the run they begin and end with alike, and what lies between is
/// replaced. A node alike in both is never sent.
pub fn drawn_changes(old: &serde_json::Value, new: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    changes_at(old, new, &mut path, &mut out);
    out
}

fn changes_at(old: &serde_json::Value, new: &serde_json::Value, path: &mut Vec<usize>, out: &mut Vec<serde_json::Value>) {
    use serde_json::json;
    if old == new {
        return;
    }
    let key = kids_key(new);
    let alike = key.is_some() && key == kids_key(old) && old.get("node") == new.get("node");
    let (Some(key), true) = (key, alike) else {
        out.push(json!({ "at": path, "node": new }));
        return;
    };
    let empty = Vec::new();
    let a = old.get(key).and_then(|k| k.as_array()).unwrap_or(&empty);
    let b = new.get(key).and_then(|k| k.as_array()).unwrap_or(&empty);
    if own_of(old, key) != own_of(new, key) {
        out.push(json!({ "at": path, "own": own_of(new, key) }));
    }
    if a.len() == b.len() {
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            path.push(i);
            changes_at(x, y, path, out);
            path.pop();
        }
        return;
    }
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let most = a.len().min(b.len()) - head;
    let tail = a.iter().rev().zip(b.iter().rev()).take(most).take_while(|(x, y)| x == y).count();
    out.push(json!({ "at": path, "splice": [head, a.len() - head - tail], "put": &b[head..b.len() - tail] }));
}

/// The changes made to a tree, as a drawn renderer makes them
/// (`drawn_changes` undone, for tests): false where a change leads
/// nowhere.
pub fn apply_drawn_changes(tree: &mut serde_json::Value, changes: &[serde_json::Value]) -> bool {
    for ch in changes {
        let Some(path) = ch["at"].as_array() else { return false };
        let mut at = &mut *tree;
        for i in path {
            let (Some(i), Some(key)) = (i.as_u64(), kids_key(at)) else { return false };
            let Some(next) = at.get_mut(key).and_then(|k| k.as_array_mut()).and_then(|k| k.get_mut(i as usize)) else { return false };
            at = next;
        }
        if let Some(n) = ch.get("node") {
            *at = n.clone();
        } else if let Some(own) = ch.get("own").and_then(|o| o.as_object()) {
            let Some(key) = kids_key(at) else { return false };
            let kids = at.get(key).cloned().unwrap_or(serde_json::Value::Array(Vec::new()));
            let mut o = own.clone();
            o.insert(key.to_string(), kids);
            *at = serde_json::Value::Object(o);
        } else if let Some(s) = ch.get("splice").and_then(|s| s.as_array()) {
            let (Some(from), Some(drop), Some(key)) = (s.first().and_then(|v| v.as_u64()), s.get(1).and_then(|v| v.as_u64()), kids_key(at)) else { return false };
            let put = ch.get("put").and_then(|p| p.as_array()).cloned().unwrap_or_default();
            let Some(kids) = at.get_mut(key).and_then(|k| k.as_array_mut()) else { return false };
            let (from, drop) = (from as usize, drop as usize);
            if from + drop > kids.len() {
                return false;
            }
            kids.splice(from..from + drop, put);
        } else {
            return false;
        }
    }
    true
}

fn event_json(e: &Event) -> serde_json::Value {
    let mut out = serde_json::json!({ "on": e.on, "handler": e.handler });
    if let Some(args) = &e.args {
        out["args"] = serde_json::Value::Array(args.clone());
        // the event's value as the page sends it, byte for byte (the
        // offer a session checks it against is this text)
        out["value"] = serde_json::Value::String(serde_json::Value::Array(args.clone()).to_string());
    }
    out
}

/// A frame as data: its columns (each with whether it holds numbers) and
/// the rows given (all of them for `None`), each a list of the cells as
/// data, a missing one `null`.
fn frame_json(f: &CrowFrame, rows: Option<&[usize]>) -> serde_json::Value {
    use crow_runtime::frame::Column;
    use serde_json::{json, Value};
    let columns: Vec<Value> = f
        .columns
        .iter()
        .zip(&f.data)
        .map(|(name, c)| json!({ "name": name, "numeric": matches!(c.as_ref(), Column::Int(_) | Column::Float(_)) }))
        .collect();
    let all: Vec<usize>;
    let rows = match rows {
        Some(r) => r,
        None => {
            all = (0..f.nrows()).collect();
            &all
        }
    };
    let cell = |c: usize, r: usize| -> Value {
        if !f.validity[c][r] {
            return Value::Null;
        }
        match f.data[c].as_ref() {
            Column::Int(v) => Value::from(v[r]),
            Column::Float(v) => crow_runtime::json::float_to_json(v[r]),
            Column::Bool(v) => Value::Bool(v[r]),
            Column::Str(v) => Value::String(v[r].to_string()),
            Column::DateTime(ms, offsets) => Value::String(crow_runtime::frame::format_datetime(ms[r], offsets[r])),
            Column::Vectors(w, v) => Value::Array(v[r * w..(r + 1) * w].iter().map(|x| crow_runtime::json::float_to_json(*x)).collect()),
        }
    };
    let data: Vec<Value> = rows.iter().map(|&r| Value::Array((0..f.ncols()).map(|c| cell(c, r)).collect())).collect();
    json!({ "columns": columns, "rows": data })
}

/// A table's rows as data: the rows the table shows, in the order it shows
/// them (the session's page of a paged table, its order if sorted), how
/// many there are in all, and the table's options.
fn rows_json(r: &Rows) -> serde_json::Value {
    use serde_json::{json, Map, Value};
    let shown = layout::table_rows(&r.frame, r.page, r.sort.as_ref());
    let mut out = json!({
        "node": "rows",
        "frame": frame_json(&r.frame, shown.as_deref()),
        "total": r.frame.nrows(),
        "delta": r.delta,
        "hide": r.hide,
        "actions": r.actions.iter().map(|(label, h)| json!({ "label": label, "handler": h })).collect::<Vec<_>>(),
        "confirm": r.confirm.iter().map(|(label, t)| (label.clone(), json!(t))).collect::<Map<String, Value>>(),
    });
    if shown.is_none() {
        // more rows than a table shows unpaged: the table says so in their place
        out["frame"]["rows"] = json!([]);
        out["refused"] = json!(true);
    }
    // what a drawn renderer sends from it and shows in each cell, as the
    // table's parts carry them (`TableOf::sends`): a row's click and
    // actions, a sorted header's, the pager's steps
    if let Some(s) = table_of(r).sends() {
        let pairs = |v: &[(String, String)]| v.iter().map(|(e, val)| json!([e, val])).collect::<Vec<_>>();
        out["sends"] = Value::Array(s.rows.iter().map(|(sends, _)| Value::Array(pairs(sends))).collect());
        out["shown"] = Value::Array(s.rows.iter().map(|(_, texts)| json!(texts)).collect());
        out["sorts"] = Value::Array(pairs(&s.sorts));
        out["steps"] = json!(s.steps);
    }
    for (name, v) in [("click", &r.click), ("key", &r.key), ("mark", &r.mark)] {
        if let Some(v) = v {
            out[name] = json!(v);
        }
    }
    if let Some(p) = r.page {
        out["page"] = json!({ "size": p.size, "index": p.index, "at": p.at });
    }
    if let Some(s) = &r.sort {
        out["sort"] = match &s.by {
            Some((by, desc)) => json!({ "index": s.index, "by": by, "desc": desc }),
            None => json!({ "index": s.index }),
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Markup a compiled page hands on is read back as the tree's own
    /// kinds: a bound field and select, a link's icon, and a frame's
    /// table with its row clicks, actions, sorted headers and pager.
    #[test]
    fn markup_is_read_back_as_the_trees_kinds() {
        let field = super::super::CrowVNode::field_typed("Qty", "qty", "3", "number", 0).to_html();
        let select = super::super::CrowVNode::select_input(&["a".into(), "b".into()], "pick", "b").to_html();
        let html = format!("<a href=\"/x\"><svg class=\"icon icon-chart\" width=\"20\"></svg>X</a>{}{}<table><thead><tr><th><button data-crow-args=\"[3,&quot;sym&quot;]\" data-crow-click=\"__sort\">sym</button></th><th class=\"num\">px</th><th></th></tr></thead><tbody><tr data-crow-click=\"focus\" data-crow-args=\"[&quot;A&quot;]\" tabindex=\"0\"><td>A</td><td class=\"num\">1.5</td><td class=\"actions\"><button data-crow-click=\"drop\" data-crow-args=\"[&quot;A&quot;]\">Drop</button></td></tr></tbody><tfoot><tr><td class=\"crow-pager\"><button disabled=\"\">Previous</button><span>1–1 of 3</span><button data-crow-args=\"[3,1]\" data-crow-click=\"__page\">Next</button></td></tr></tfoot></table>", field, select);
        let read = read_back(&super::super::parse_markup(&html));
        assert_eq!(read[0]["kind"], "a");
        assert_eq!(read[0]["options"]["href"], "/x");
        assert_eq!(read[0]["children"][0], json!({"node": "control", "kind": "icon", "name": "chart", "size": 20.0, "color": ""}));
        assert_eq!(read[1], json!({"node": "control", "kind": "field", "label": "Qty", "bind": "qty", "value": "3", "type": "number", "lines": 0}));
        assert_eq!(read[2], json!({"node": "control", "kind": "select", "options": ["a", "b"], "bind": "pick", "value": "b"}));
        let rows = &read[3]["children"][0];
        assert_eq!(rows["node"], "rows");
        assert_eq!(rows["frame"]["columns"], json!([{"name": "sym", "numeric": false}, {"name": "px", "numeric": true}]));
        assert_eq!(rows["shown"], json!([["A", "1.5"]]));
        assert_eq!(rows["sends"], json!([[["focus", "[\"A\"]"], ["drop", "[\"A\"]"]]]));
        assert_eq!(rows["click"], "focus");
        assert_eq!(rows["actions"], json!([{"label": "Drop", "handler": "drop"}]));
        assert_eq!(rows["sorts"], json!([["sym", "[3,\"sym\"]"]]));
        assert_eq!(rows["steps"], json!([null, "[3,1]"]));
        assert_eq!(rows["page"], json!({"size": 1, "index": 3, "at": 0}));
        assert_eq!(rows["total"], 3);
    }

    /// A picture in markup is read back with what a drawn renderer loads
    /// it from: an `img` its address and size, an `svg` that is no icon
    /// its own markup as a `data:` address (an icon stays an icon).
    #[test]
    fn a_picture_in_markup_is_read_back_with_its_address() {
        let html = "<img src=\"/logo.png\" alt=\"logo\" width=\"120\" height=\"40\"><svg viewBox=\"0 0 10 10\" width=\"30\"><circle cx=\"5\" cy=\"5\" r=\"4\"></circle></svg><svg class=\"icon icon-chart\" width=\"20\"></svg>";
        let read = read_back(&super::super::parse_markup(html));
        assert_eq!(read[0]["kind"], "img");
        assert_eq!(read[0]["options"], json!({"src": "/logo.png", "alt": "logo", "width": "120px", "height": "40px"}));
        assert_eq!(read[1]["kind"], "svg");
        assert_eq!(read[1]["options"]["width"], "30px");
        let src = read[1]["options"]["src"].as_str().expect("an address");
        assert!(src.starts_with("data:image/svg+xml;charset=utf-8,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22"), "{}", src);
        assert!(src.contains("circle"), "{}", src);
        assert_eq!(read[2]["kind"], "icon");
    }

    fn text(s: &str) -> Given {
        Given::Text(s.to_string())
    }

    fn opts(pairs: &[(&str, Given)]) -> Vec<(String, Given)> {
        let mut v: Vec<(String, Given)> = pairs.iter().map(|(k, g)| (k.to_string(), g.clone())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    fn el(kind: &str, pairs: &[(&str, Given)], children: Vec<Given>) -> Node {
        element(kind, opts(pairs), children, TableState::default()).expect("builds")
    }

    fn frame() -> CrowFrame {
        use crow_runtime::frame::Column;
        let mut f = CrowFrame { columns: Vec::new(), data: Vec::new(), validity: Vec::new() };
        f = f.with_column("sym", Column::Str(vec!["IMRN".to_string(), "AAPL".to_string()].into())).unwrap();
        f.with_column("px", Column::Float(vec![1.5, -2.0])).unwrap()
    }

    /// The renderer writes the page as the interpreter's string builder
    /// had: the layout kernel's class and style, events wired, attributes
    /// in name order, a void element without its end tag.
    #[test]
    fn an_element_is_written_as_the_page_has_always_had_it() {
        let row = el("row", &[("gap", text("m")), ("class", text("bar"))], vec![text("a < b")]);
        assert_eq!(html(&row), "<div class=\"row bar\" style=\"display:flex;flex-direction:row;flex-wrap:nowrap;align-items:center;gap:16px\">a &lt; b</div>");
        let hr = el("divider", &[], vec![]);
        assert_eq!(html(&hr), "<hr class=\"divider\">");
        let modal = el("modal", &[("open", Given::Bool(false))], vec![]);
        assert_eq!(html(&modal), "<div aria-modal=\"true\" class=\"modal\" data-open=\"false\" role=\"dialog\"></div>");
        let press = el("button", &[("click", Given::Call("pick".into(), vec![json!(3)])), ("confirm", text("Sure?"))], vec![text("Go")]);
        assert_eq!(html(&press), "<button data-crow-args=\"[&quot;pick&quot;,&quot;[3]&quot;,&quot;Sure?&quot;]\" data-crow-click=\"__ask\">Go</button>");
        let card = el("div", &[("click", Given::Handler("open".into()))], vec![]);
        assert_eq!(html(&card), "<div data-crow-click=\"open\" role=\"button\" tabindex=\"0\"></div>");
    }

    /// A child is placed as it was given: a node as itself, markup as
    /// markup, a list item by item, a failed value as its id, anything
    /// else as text.
    #[test]
    fn children_are_placed_as_they_were_given() {
        let inner = Arc::new(el("span", &[], vec![text("x")]));
        let p = el("p", &[], vec![Given::Node(inner), Given::Markup("<b>m</b>".into()), Given::List(vec![Given::Int(1), text("<")]), Given::Shown("map[a = 1]".into())]);
        assert_eq!(html(&p), "<p><span>x</span><b>m</b>1&lt;map[a = 1]</p>");
        let failed = el("p", &[], vec![Given::Failed("why".into())]);
        assert!(html(&failed).starts_with("<p>error: "), "{}", html(&failed));
    }

    /// The page's nodes read back as the browser builds them are the ones
    /// its markup reads back as: the live runtime's tree is the page's.
    #[test]
    fn the_nodes_read_back_as_the_markup_does() {
        let table = element("table", opts(&[("click", Given::Handler("focus".into())), ("page", Given::Int(1))]), vec![Given::Frame(Box::new(frame()))], TableState { page: Some((0, 1)), sort: None }).unwrap();
        let nav = el("nav", &[("active", text("/b"))], vec![Given::Node(Arc::new(el("a", &[("href", text("/a"))], vec![text("A")]))), Given::Node(Arc::new(el("a", &[("href", text("/b"))], vec![text("B")])))]);
        let page = el(
            "stack",
            &[("gap", text("s"))],
            vec![
                Given::Node(Arc::new(nav)),
                Given::Node(Arc::new(table)),
                Given::Node(Arc::new(field("Qty", "qty", json!(100), "100", "number", 0))),
                Given::Node(Arc::new(el("pre", &[], vec![text("\nkept")]))),
                Given::Node(Arc::new(icon("check", "", "").unwrap())),
                Given::Node(Arc::new(progress(2.0, 4.0))),
            ],
        );
        let markup = html(&page);
        assert!(markup.contains("<a aria-current=\"page\" href=\"/b\">B</a>"), "{}", markup);
        assert!(markup.contains("<input aria-label=\"Qty\" data-crow-input=\"__field:qty\" type=\"number\" value=\"100\">"), "{}", markup);
        assert_eq!(CrowVNode::Raw(markup.clone()).as_built().to_html(), vnode(&page).as_built().to_html());
    }

    /// An option the layout vocabulary refuses stops the build, named.
    #[test]
    fn a_refused_option_stops_the_build() {
        let e = element("row", opts(&[("gap", text("huge"))]), vec![], TableState::default()).unwrap_err();
        assert!(e.starts_with("row: gap = 'huge'"), "{}", e);
        assert!(icon("nope", "", "").unwrap_err().starts_with("icon: icon 'nope'"));
    }

    /// The tree as data: kinds, typed options, events with their frozen
    /// arguments, bindings and values, a table's rows as the table shows
    /// them, markup marked as markup — and nothing HTML's.
    #[test]
    fn the_tree_as_data_says_what_the_view_said() {
        let sorted = TableState { page: None, sort: Some((0, Some(("px".to_string(), false)))) };
        let table = element("table", opts(&[("click", Given::Handler("focus".into())), ("key", text("sym")), ("sort", Given::Bool(true))]), vec![Given::Frame(Box::new(frame()))], sorted).unwrap();
        let page = el(
            "row",
            &[("gap", text("m")), ("width", Given::Int(240)), ("disabled", Given::Bool(true))],
            vec![
                Given::Node(Arc::new(el("button", &[("click", Given::Call("pick".into(), vec![json!(3), json!("a")]))], vec![text("Go")]))),
                Given::Node(Arc::new(check("Live", "live", true, true))),
                Given::Node(Arc::new(table)),
                Given::Markup("<b>m</b>".into()),
                Given::Node(Arc::new(widget("grid", json!({"rows": 2}), vec![("pick".into(), "focus".into())]))),
            ],
        );
        let data = to_json(&page);
        assert_eq!(data["node"], "element");
        assert_eq!(data["kind"], "row");
        assert_eq!(data["options"], json!({"disabled": true, "gap": "m", "width": 240}));
        let kids = data["children"].as_array().unwrap();
        assert_eq!(kids[0]["events"], json!([{"on": "click", "handler": "pick", "args": [3, "a"], "value": "[3,\"a\"]"}]));
        assert_eq!(kids[0]["children"][0], json!({"node": "text", "text": "Go"}));
        assert_eq!(kids[1], json!({"node": "control", "kind": "toggle", "label": "Live", "bind": "live", "value": true}));
        assert_eq!(kids[2]["kind"], "table");
        let rows = &kids[2]["children"][0];
        assert_eq!(rows["node"], "rows");
        assert_eq!(rows["click"], "focus");
        assert_eq!(rows["key"], "sym");
        assert_eq!(rows["sort"], json!({"index": 0, "by": "px", "desc": false}));
        assert_eq!(rows["frame"]["columns"], json!([{"name": "sym", "numeric": false}, {"name": "px", "numeric": true}]));
        // the rows in the order the table shows them, sorted by px
        assert_eq!(rows["frame"]["rows"], json!([["AAPL", -2.0], ["IMRN", 1.5]]));
        assert_eq!(rows["total"], 2);
        assert_eq!(kids[3], json!({"node": "markup", "html": "<b>m</b>", "read": [{"node": "element", "kind": "b", "options": {}, "events": [], "children": [{"node": "text", "text": "m"}]}]}));
        // (an element read back from markup keeps its id)
        let read = read_back(&super::super::parse_markup("<a href=\"/x\" id=\"nav-x\">X</a>"));
        assert_eq!(read[0]["options"]["id"], json!("nav-x"), "{}", read[0]);
        assert_eq!(kids[4], json!({"node": "widget", "module": "grid", "props": {"rows": 2}, "on": {"pick": "focus"}}));
        // no tag, class, style or wiring of the page's anywhere but markup
        let mut without_markup = data.clone();
        without_markup["children"][3] = json!(null);
        let written = without_markup.to_string();
        for html_only in ["\"div\"", "class", "style", "data-crow", "display:flex", "tabindex"] {
            assert!(!written.contains(html_only), "{} in {}", html_only, written);
        }
    }

    /// The newline that opens a `pre` or a `textarea` is the text's own:
    /// the browser drops the first newline there, so the page writes one
    /// more, as markup and as the page's nodes, in both modes.
    #[test]
    fn the_newline_that_opens_a_pre_is_kept() {
        for kind in ["pre", "textarea"] {
            let n = el(kind, &[], vec![text("\nkept")]);
            let tag = if kind == "pre" { "pre" } else { "textarea" };
            assert_eq!(html(&n), format!("<{t}>\n\nkept</{t}>", t = tag));
            assert_eq!(vnode(&n).to_html(), html(&n));
            match CrowVNode::Raw(html(&n)).as_built() {
                CrowVNode::Element { children, .. } => assert!(matches!(children.as_slice(), [CrowVNode::Text(t)] if t == "\nkept"), "{}: {:?}", kind, children),
                other => panic!("{:?}", other),
            }
        }
    }

    /// A class, written or computed, joins the element's own as the layout
    /// table joins them: the element's, then each option's in name order
    /// (`class` before `surface`), each word once as written, an empty
    /// one adding nothing.
    #[test]
    fn a_class_joins_as_the_layout_table_joins_it() {
        let card = el("row", &[("surface", text("card")), ("class", text("x"))], vec![]);
        assert!(html(&card).starts_with("<div class=\"row x card\""), "{}", html(&card));
        let empty = el("div", &[("class", text(""))], vec![]);
        assert_eq!(html(&empty), "<div></div>");
        let two = el("div", &[("class", text(" a  b "))], vec![]);
        assert_eq!(html(&two), "<div class=\"a b\"></div>");
    }

    /// Every event an element takes is wired, `change` among them, a
    /// handler with the arguments its call froze as well as one alone.
    #[test]
    fn every_event_is_wired() {
        for on in EVENTS {
            let n = el("div", &[(on, Given::Handler("h".into()))], vec![]);
            assert!(html(&n).contains(&format!("data-crow-{}=\"h\"", on)), "{}", html(&n));
        }
        let pick = el("select", &[("change", Given::Call("pick".into(), vec![json!(2)]))], vec![]);
        assert_eq!(html(&pick), "<select data-crow-args=\"[2]\" data-crow-change=\"pick\"></select>");
        // (a text is no handler: it is not wired, in either mode)
        assert_eq!(html(&el("button", &[("click", text("inc"))], vec![])), "<button></button>");
    }

    /// A table's rows, an icon and a chart are the page's nodes as their
    /// markup reads back, built as nodes: no markup is left to read.
    #[test]
    fn rows_icons_and_charts_are_nodes() {
        fn markup_in(v: &CrowVNode) -> bool {
            match v {
                CrowVNode::Raw(_) => true,
                CrowVNode::Element { children, .. } => children.iter().any(markup_in),
                CrowVNode::Text(_) => false,
                CrowVNode::Shared(s) => markup_in(s.node()),
            }
        }
        use crow_runtime::frame::Column;
        let ts = CrowFrame { columns: Vec::new(), data: Vec::new(), validity: Vec::new() }
            .with_column("ts", Column::Int(vec![1_700_000_000, 1_700_000_060]))
            .unwrap()
            .with_column("c", Column::Float(vec![1.0, 2.5]))
            .unwrap();
        let sorted = TableState { page: Some((0, 0)), sort: Some((0, Some(("px".to_string(), true)))) };
        let parts = vec![
            element("table", opts(&[("click", Given::Handler("focus".into())), ("key", text("sym")), ("page", Given::Int(1)), ("sort", Given::Bool(true))]), vec![Given::Frame(Box::new(frame()))], sorted).unwrap(),
            icon("check", "l", "accent").unwrap(),
            chart(&ts, "ts", &[], &["c".to_string()], "", f64::NAN, "America/New_York", 200.0, &[], &(), None, Some("seek")),
        ];
        for n in parts {
            let v = vnode(&n);
            assert!(!markup_in(&v), "{:?}", v);
            assert!(CrowVNode::changes(&CrowVNode::Raw(html(&n)).as_built(), &v).is_empty(), "{}", html(&n));
            assert_eq!(v.to_html(), CrowVNode::Raw(html(&n)).as_built().to_html());
        }
    }

    /// A control's data carries what it is bound to and its value.
    #[test]
    fn a_controls_data_names_its_binding() {
        assert_eq!(to_json(&field("Qty", "qty", json!(100), "100", "number", 0)), json!({"node": "control", "kind": "field", "label": "Qty", "bind": "qty", "value": 100, "type": "number", "lines": 0}));
        assert_eq!(to_json(&select(vec!["a".into(), "b".into()], "pick", "b")), json!({"node": "control", "kind": "select", "options": ["a", "b"], "bind": "pick", "value": "b"}));
        assert_eq!(to_json(&progress(2.0, 4.0)), json!({"node": "control", "kind": "progress", "value": 2.0, "max": 4.0}));
        assert_eq!(to_json(&failed("why"))["node"], "failed");
    }

    /// A drawn renderer's changes make the tree it held the next one,
    /// over a thousand random pairs of trees (an element's children and a
    /// markup node's read alike, children added, taken out, moved and
    /// changed); a node alike in both is never in them, and a tree alike
    /// sends none. Markup read back is sent without its HTML, and a
    /// table's rows without the frame's cells beside the cells shown.
    #[test]
    fn a_drawn_renderers_changes_make_the_next_tree() {
        struct Rng(u64);
        impl Rng {
            fn next(&mut self, n: u64) -> u64 {
                self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (self.0 >> 33) % n
            }
        }
        fn node(r: &mut Rng, depth: u32) -> serde_json::Value {
            match r.next(if depth == 0 { 2 } else { 5 }) {
                0 => json!({"node": "text", "text": format!("t{}", r.next(6))}),
                1 => json!({"node": "control", "kind": "field", "bind": "b", "value": r.next(3)}),
                2 => json!({"node": "markup", "read": (0..r.next(4)).map(|_| node(r, depth - 1)).collect::<Vec<_>>()}),
                _ => {
                    let kind = ["row", "stack", "p"][r.next(3) as usize];
                    json!({"node": "element", "kind": kind, "options": {"gap": r.next(2)}, "events": [], "children": (0..r.next(5)).map(|_| node(r, depth - 1)).collect::<Vec<_>>()})
                }
            }
        }
        fn changed(r: &mut Rng, v: &serde_json::Value, depth: u32) -> serde_json::Value {
            let mut v = v.clone();
            if r.next(5) == 0 {
                return node(r, depth);
            }
            if let Some(key) = kids_key(&v) {
                if key == "children" && r.next(4) == 0 {
                    v["options"]["gap"] = json!(r.next(3));
                }
                let kids = v[key].as_array_mut().unwrap();
                match r.next(4) {
                    0 if depth > 0 => {
                        let at = r.next(kids.len() as u64 + 1) as usize;
                        kids.insert(at, node(r, depth - 1));
                    }
                    1 if !kids.is_empty() => {
                        let at = r.next(kids.len() as u64) as usize;
                        kids.remove(at);
                    }
                    _ => {
                        for k in kids.iter_mut() {
                            if r.next(3) == 0 {
                                *k = changed(r, k, depth.saturating_sub(1));
                            }
                        }
                    }
                }
            }
            v
        }
        let mut r = Rng(7);
        for _ in 0..1000 {
            let a = json!({"node": "element", "kind": "stack", "options": {}, "events": [], "children": (0..r.next(6)).map(|_| node(&mut r, 3)).collect::<Vec<_>>()});
            let b = changed(&mut r, &a, 3);
            assert!(drawn_changes(&a, &a).is_empty());
            let ch = drawn_changes(&a, &b);
            let mut held = a.clone();
            assert!(apply_drawn_changes(&mut held, &ch), "{} to {}: {:?}", a, b, ch);
            assert_eq!(held, b, "{} to {}: {:?}", a, b, ch);
            assert!(serde_json::Value::Array(ch).to_string().len() <= b.to_string().len() + 40 * 8, "{} to {}", a, b);
        }
        // a row changed among many: the change is that row, by its path
        let rows = |n: u64| json!({"node": "element", "kind": "stack", "options": {}, "events": [], "children": (0..20).map(|i| json!({"node": "element", "kind": "p", "options": {}, "events": [], "children": [{"node": "text", "text": format!("row {}", if i == 7 { n } else { i })}]})).collect::<Vec<_>>()});
        assert_eq!(drawn_changes(&rows(7), &rows(70)), vec![json!({"at": [7, 0], "node": {"node": "text", "text": "row 70"}})]);
        let read = drawn_json(&Node::Markup("<p>a</p>".into()));
        assert!(read.get("html").is_none() && read["read"][0]["kind"] == "p", "{}", read);
        let bare = drawn_json(&Node::Text("t".into()));
        assert_eq!(bare, json!({"node": "text", "text": "t"}));
    }
}
