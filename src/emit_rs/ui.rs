//! Views: elements, their children and attributes, frame tables, the
//! widget, the bound inputs and charts.

use super::program::{fn_named, map_literal_pairs};
use super::*;

/// Elements with no children.
const VOID_ELEMENTS: &[&str] = &["input", "img", "divider"];

impl Emitter {
    fn lit(e: &Expr) -> Option<String> {
        match Emitter::raw(e) {
            Expr::Const(Const::Str(s), _) => Some(s.clone()),
            Expr::Const(Const::Int(n), _) => Some(n.to_string()),
            Expr::Const(Const::Bool(b), _) => Some(b.to_string()),
            _ => None,
        }
    }

    pub(crate) fn tree(&mut self, element: &str, attrs: &[(String, Expr)], children: &[Expr]) -> String {
        if element == "widget" {
            return self.widget(attrs, children);
        }
        // table[frame]: the frame's rows; a table of element children is an
        // ordinary element
        if element == "table" && children.len() == 1 && matches!(Self::inner(&self.tt(&children[0])), CrowType::Frame) {
            return self.frame_table(attrs, &children[0]);
        }
        let is_container = crow_runtime::layout::CONTAINERS.contains(&element);
        let rust_name = if element == "main" { "main_el" } else { crow_runtime::layout::html_tag(element) };
        let is_void = VOID_ELEMENTS.contains(&element);

        // children: a list child is spliced, each item a node
        let mut spliced: Vec<bool> = Vec::new();
        let mut child_strs: Vec<String> = Vec::new();
        for c in children {
            let (s, sp) = self.child(c);
            child_strs.push(s);
            spliced.push(sp);
        }
        let base = if is_void {
            format!("CrowVNode::{}()", rust_name)
        } else if spliced.iter().any(|s| *s) {
            let pushes: Vec<String> = child_strs.iter().zip(&spliced).map(|(c, s)| if *s { format!("_k.extend({});", c) } else { format!("_k.push({});", c) }).collect();
            format!("CrowVNode::{}({{ let mut _k: Vec<CrowVNode> = Vec::new(); {} _k }})", rust_name, pushes.join(" "))
        } else {
            format!("CrowVNode::{}(vec![{}])", rust_name, child_strs.join(", "))
        };

        // layout attributes (literals) through the kernel's table, as the
        // interpreter renders them; the rest are attributes and events
        let mut literal_layout: Vec<(String, String)> = Vec::new();
        let mut other: Vec<(String, Expr)> = Vec::new();
        for (k, v) in attrs {
            let is_layout = crow_runtime::layout::LAYOUT_ATTRS.contains(&k.as_str()) || (is_container && k == "class");
            match (is_layout, Self::lit(v)) {
                (true, Some(s)) => literal_layout.push((k.clone(), s)),
                _ => other.push((k.clone(), v.clone())),
            }
        }
        let rendered = crow_runtime::layout::render(element, &literal_layout);
        let mut result = base;
        if !rendered.class.is_empty() {
            result = format!("{}.attr(\"class\", \"{}\")", result, rendered.class);
        }
        if !rendered.style.is_empty() {
            result = format!("{}.attr(\"style\", \"{}\")", result, rendered.style.replace('\\', "\\\\").replace('"', "\\\""));
        }
        for (k, v) in &rendered.data {
            result = format!("{}.attr(\"{}\", \"{}\")", result, k, v);
        }
        for (k, v) in &rendered.passthrough {
            result = format!("{}.attr(\"{}\", \"{}\")", result, k, v.replace('"', "\\\""));
        }
        if element == "nav" {
            if let Some((_, a)) = literal_layout.iter().find(|(k, _)| k == "active") {
                result = format!("{}.mark_active(\"{}\")", result, a);
            }
        }
        let mut confirm: Option<String> = None;
        for (k, v) in &other {
            if k == "confirm" {
                confirm = Some(self.expr(v));
                continue;
            }
            // a handler with its arguments: frozen at render, called at the event
            if k == "click" {
                if let Expr::Deferred { name, args } = Emitter::raw(v) {
                    if self.reactive_event_handlers.contains(name) {
                        let strs: Vec<String> = args.iter().map(|x| format!("({})", self.expr(x))).collect();
                        self.needs_serde_json = true;
                        result = format!("{}.on_click_args(\"{}\", &serde_json::json!([{}]).to_string())", result, name, strs.join(", "));
                        continue;
                    }
                }
            }
            let is_fn_ref = matches!(Emitter::raw(v), Expr::FnRef(f, _) if self.reactive_event_handlers.contains(f));
            let event = if is_fn_ref {
                format!("\"{}\"", fn_named(v).unwrap_or_default())
            } else {
                self.expr(v)
            };
            match k.as_str() {
                "open" => {
                    let o = self.expr(v);
                    result = format!("{}.open_attr({{ let _o = {}; _o }})", result, o);
                }
                "active" if element == "nav" => {
                    let a = self.expr(v);
                    result = format!("{}.attr(\"data-active\", &{a}).mark_active(&{a})", result, a = a);
                }
                "tip" => {
                    let t = self.expr(v);
                    result = format!("{}.attr(\"title\", &format!(\"{{}}\", {}))", result, t);
                }
                b if crow_runtime::layout::BOOL_ATTRS.contains(&b) && !matches!(Emitter::raw(v), Expr::Const(Const::Str(_), _)) => {
                    let x = self.expr(v);
                    result = format!("{}.bool_attr(\"{}\", {{ let _b: bool = {}; _b }})", result, b, x);
                }
                "click" => result = if is_fn_ref { format!("{}.on_click({})", result, event) } else { format!("{}.on_click(&{})", result, event) },
                "input" => result = if is_fn_ref { format!("{}.on_input({})", result, event) } else { format!("{}.on_input(&{})", result, event) },
                "submit" => result = if is_fn_ref { format!("{}.on_submit({})", result, event) } else { format!("{}.on_submit(&{})", result, event) },
                _ => {
                    let x = self.expr(v);
                    result = format!("{}.attr(&{}, &{})", result, super::exprs::rust_str_lit(k), x);
                }
            }
        }
        if let Some(t) = confirm {
            result = format!("{}.confirm(&format!(\"{{}}\", {}))", result, t);
        }
        result
    }

    /// A child as a node, and whether it is a list to splice.
    fn child(&mut self, c: &Expr) -> (String, bool) {
        let raw = Emitter::raw(c);
        // a literal text, or text with spans: a failed span shows its reason
        if matches!(raw, Expr::Const(Const::Str(_), _) | Expr::Interp(_)) {
            let s = self.expr(raw);
            if matches!(self.tt(raw), CrowType::Result(_)) {
                return (format!("{{ let _c = &({}); if !_c.ok {{ CrowVNode::text(&format!(\"error: {{}}\", _c.reason)) }} else {{ CrowVNode::text(&_c.value) }} }}", s), false);
            }
            return (format!("CrowVNode::text(&{})", s), false);
        }
        // trees: an element, a view, a chart, a bound input
        let is_node = match raw {
            Expr::Tree { .. } => true,
            Expr::Call { target: Target::User(v), .. } => self.views.contains(v),
            Expr::Call { target: Target::Runtime(n), .. } => {
                matches!(n.as_str(), "chart" | "field" | "select" | "slider" | "checkbox" | "toggle" | "radio" | "tabs" | "progress" | "icon")
            }
            _ => false,
        };
        if is_node {
            return (self.expr(raw), false);
        }
        // a local reads as the type it was declared with (markup when it
        // holds a node's markup); anything else as the IR types it
        let ty = match raw {
            Expr::Local(n, _) | Expr::Move(n, _) => self.local_ty(n).unwrap_or_else(|| self.tt_full(raw)),
            _ => self.tt_full(raw),
        };
        let fallible = matches!(ty, CrowType::Result(_));
        let inner = Self::inner(&ty);
        let s = self.expr(raw);
        if let CrowType::Array(item) = &inner {
            let place = if matches!(item.as_ref(), CrowType::Markup) { "crow_web::view::markup_children" } else { "crow_web::view::children_of" };
            if fallible {
                return (format!("{{ let _c = &({}); if !_c.ok {{ vec![CrowVNode::text(&format!(\"error: {{}}\", _c.reason))] }} else {{ {}(&_c.value) }} }}", s, place), true);
            }
            return (format!("{}(&{})", place, s), true);
        }
        let text = match inner {
            CrowType::Markup if fallible => format!("{{ let _c = &({}); if !_c.ok {{ CrowVNode::text(&format!(\"error: {{}}\", _c.reason)) }} else {{ CrowVNode::raw(&_c.value) }} }}", s),
            CrowType::Str if fallible => format!("{{ let _c = &({}); if !_c.ok {{ CrowVNode::text(&format!(\"error: {{}}\", _c.reason)) }} else {{ CrowVNode::text(&_c.value) }} }}", s),
            CrowType::Int | CrowType::Float | CrowType::Bool if fallible => {
                format!("{{ let _c = &({}); if !_c.ok {{ CrowVNode::text(&format!(\"error: {{}}\", _c.reason)) }} else {{ CrowVNode::text(&format!(\"{{}}\", _c.value)) }} }}", s)
            }
            // markup a view, an element or doc made is markup; a string is text
            CrowType::Markup => format!("CrowVNode::raw(&{})", s),
            CrowType::Str => format!("CrowVNode::text(&{})", s),
            CrowType::Int | CrowType::Float | CrowType::Bool => format!("CrowVNode::text(&format!(\"{{}}\", {}))", s),
            _ => s,
        };
        (text, false)
    }

    /// `widget[name, props = …, on = map[event = handler]]`.
    fn widget(&mut self, attrs: &[(String, Expr)], children: &[Expr]) -> String {
        let module = children.first().and_then(|c| if let Expr::Const(Const::Str(s), _) = Emitter::raw(c) { Some(s.clone()) } else { None }).unwrap_or_default();
        let mut props = "serde_json::Value::Null".to_string();
        let mut on: Vec<String> = Vec::new();
        for (k, v) in attrs {
            match k.as_str() {
                "props" => {
                    self.needs_serde_json = true;
                    props = format!("serde_json::to_value(&{}).unwrap_or(serde_json::Value::Null)", self.expr(v));
                }
                "on" => {
                    for (event, h) in map_literal_pairs(v).unwrap_or_default() {
                        if let Some(f) = fn_named(&h) {
                            on.push(format!("({:?}, {:?})", event, f));
                        }
                    }
                }
                _ => {}
            }
        }
        format!("CrowVNode::widget({:?}, &{}, &[{}])", module, props, on.join(", "))
    }

    /// `table[frame, click = f, key = 'col', …]`: the kernel's rows.
    fn frame_table(&mut self, attrs: &[(String, Expr)], frame: &Expr) -> String {
        let frame_ty = self.tt(frame);
        let f = self.expr(frame);
        let (mut click, mut key, mut mark) = ("None".to_string(), "None".to_string(), "None".to_string());
        let (mut delta, mut actions, mut hide, mut confirm) = ("&[]".to_string(), "&[]".to_string(), "&[]".to_string(), "&[]".to_string());
        let mut page = "None".to_string();
        let live = self.reactive_render_fns.contains(&self.cur_fn);
        let names = |e: &Expr| -> Vec<String> {
            match Emitter::raw(e) {
                Expr::ListLit(items, _) => items.iter().filter_map(|i| if let Expr::Const(Const::Str(s), _) = Emitter::raw(i) { Some(format!("\"{}\".to_string()", s)) } else { None }).collect(),
                _ => vec![],
            }
        };
        for (k, v) in attrs {
            match k.as_str() {
                "click" => {
                    if let Some(h) = fn_named(v) {
                        click = format!("Some(\"{}\")", h);
                    }
                }
                "key" => {
                    if let Some(s) = Self::lit(v) {
                        key = format!("Some(\"{}\")", s);
                    }
                }
                "mark" => {
                    if let Some(s) = Self::lit(v) {
                        mark = format!("Some(\"{}\")", s);
                    }
                }
                "delta" => delta = format!("&[{}]", names(v).join(", ")),
                "hide" => hide = format!("&[{}]", names(v).join(", ")),
                "actions" => {
                    if let Some(pairs) = map_literal_pairs(v) {
                        let ps: Vec<String> = pairs.iter().filter_map(|(label, h)| fn_named(h).map(|f| format!("(\"{}\".to_string(), \"{}\".to_string())", label, f))).collect();
                        actions = format!("&[{}]", ps.join(", "));
                    }
                }
                "confirm" => {
                    if let Some(pairs) = map_literal_pairs(v) {
                        let ps: Vec<String> = pairs
                            .iter()
                            .filter_map(|(label, t)| if let Expr::Const(Const::Str(text), _) = Emitter::raw(t) { Some(format!("(\"{}\".to_string(), {:?}.to_string())", label, text)) } else { None })
                            .collect();
                        confirm = format!("&[{}]", ps.join(", "));
                    }
                }
                "page" => {
                    let size = format!("(({}) as i64).max(1) as usize", self.expr(v));
                    page = if live { format!("Some(state.table_page({}, _f.nrows()))", size) } else { format!("Some(crow_runtime::layout::Paging {{ size: {}, index: 0, at: 0 }})", size) };
                }
                _ => {}
            }
        }
        let rows = format!(
            "CrowVNode::table(vec![CrowVNode::raw(&crow_runtime::layout::frame_table_html(_f, {}, {}, {}, {}, {}, {}, {}, {}))])",
            click, key, mark, delta, actions, hide, confirm, page
        );
        if matches!(frame_ty, CrowType::Result(_)) {
            return format!("{{ let _fr = &({}); if !_fr.ok {{ CrowVNode::text(&format!(\"error: {{}}\", _fr.reason)) }} else {{ let _f = &_fr.value; {} }} }}", f, rows);
        }
        format!("{{ let _f = &{}; {} }}", f, rows)
    }

    /// The bound inputs, a progress bar, an icon and a chart.
    pub(crate) fn ui_builtin(&mut self, name: &str, args: &[Expr], a: &[String]) -> Option<String> {
        if !self.needs_ui {
            return None;
        }
        let n = a.len();
        Some(match name {
            "slider" if n == 5 => {
                let key = Self::bound_key_of(&args[1]);
                format!("CrowVNode::slider(&{}, \"{}\", &format!(\"{{}}\", {}), ({}) as f64, ({}) as f64, ({}) as f64)", a[0], key, a[1], a[2], a[3], a[4])
            }
            "field" if n == 4 => {
                let key = Self::bound_key_of(&args[1]);
                if matches!(Emitter::raw(&args[2]), Expr::Const(Const::Str(s), _) if s == "file") {
                    return Some(format!("CrowVNode::field_typed(&{}, \"{}\", &({}).get(\"name\").cloned().unwrap_or_default(), \"file\", 0)", a[0], key, a[1]));
                }
                let kind = match Emitter::raw(&args[2]) {
                    Expr::Const(Const::Str(s), _) if s.is_empty() => match self.tt(&args[1]) {
                        CrowType::Int | CrowType::Float => "\"number\"".to_string(),
                        _ => "\"text\"".to_string(),
                    },
                    _ => format!("&{}", a[2]),
                };
                format!("CrowVNode::field_typed(&{}, \"{}\", &format!(\"{{}}\", {}), {}, ({}) as i64)", a[0], key, a[1], kind, a[3])
            }
            "checkbox" | "toggle" if n == 2 => {
                let key = Self::bound_key_of(&args[1]);
                format!("CrowVNode::check_input(&{}, \"{}\", format!(\"{{}}\", {}) == \"true\", {})", a[0], key, a[1], name == "toggle")
            }
            "radio" | "tabs" if n == 2 => {
                let key = Self::bound_key_of(&args[1]);
                format!("CrowVNode::choice_input(&{}, \"{}\", &format!(\"{{}}\", {}), {})", a[0], key, a[1], name == "tabs")
            }
            "progress" if n == 2 => format!("CrowVNode::progress(({}) as f64, ({}) as f64)", a[0], a[1]),
            "icon" if n == 3 => format!("CrowVNode::icon(&{}, &{}, &{})", a[0], a[1], a[2]),
            "select" if n == 2 => {
                let key = Self::bound_key_of(&args[1]);
                format!("CrowVNode::select_input(&{}, \"{}\", &format!(\"{{}}\", {}))", a[0], key, a[1])
            }
            "chart" if n == 12 => self.chart(args, a),
            _ => return None,
        })
    }

    fn bound_key_of(e: &Expr) -> String {
        match Emitter::raw(e) {
            Expr::Load(n, _) | Expr::Local(n, _) | Expr::Move(n, _) => n.clone(),
            Expr::Field { name, .. } => name.clone(),
            _ => "unknown".to_string(),
        }
    }

    fn chart(&mut self, args: &[Expr], a: &[String]) -> String {
        let frame_ty = self.tt(&args[0]);
        let levels = match Emitter::raw(&args[9]) {
            Expr::Const(Const::Int(0), _) => "()".to_string(),
            _ => a[9].clone(),
        };
        let seek = match fn_named(&args[11]) {
            Some(f) if self.fns.contains_key(f) => format!("Some(\"{}\")", f),
            _ => "None".to_string(),
        };
        let core = format!(
            "CrowVNode::chart_from_frame(_f, &{x}, &{cd}, &{ln}, &{by}, {{ let _m: f64 = ({mk}) as f64; if _m <= 0.0 {{ f64::NAN }} else {{ _m }} }}, &{tz}, ({h}) as f64, &{pn}, &{lv}, _pts, {sk})",
            x = a[1],
            cd = a[2],
            ln = a[3],
            by = a[4],
            mk = a[5],
            tz = a[6],
            h = a[7],
            pn = a[8],
            lv = levels,
            sk = seek
        );
        let with_points = match Emitter::raw(&args[10]) {
            Expr::Const(Const::Int(0), _) => format!("{{ let _pts: Option<&crow_runtime::frame::CrowFrame> = None; {} }}", core),
            _ if matches!(self.tt(&args[10]), CrowType::Result(_)) => format!("{{ let _pr = &({}); let _pts = if _pr.ok {{ Some(&_pr.value) }} else {{ None }}; {} }}", a[10], core),
            _ => format!("{{ let _pf: &crow_runtime::frame::CrowFrame = &{}; let _pts = Some(_pf); {} }}", a[10], core),
        };
        if matches!(frame_ty, CrowType::Result(_)) {
            return format!("{{ let _fr = &({}); if !_fr.ok {{ CrowVNode::text(&format!(\"chart error: {{}}\", _fr.reason)) }} else {{ let _f = &_fr.value; {} }} }}", a[0], with_points);
        }
        format!("{{ let _f = &{}; {} }}", a[0], with_points)
    }
}
