//! Views: elements, their children and options, the widget, the bound
//! inputs and charts, each made by the one builder both modes call
//! (the web crate's `view::tree`).

use super::program::{fn_named, map_literal_pairs};
use super::*;

impl Emitter {
    /// An element, as the one builder makes it (`tree::element`). The
    /// options are computed first, in the order the program wrote them, as
    /// the interpreter runs them; so a child that moves a local an option
    /// reads comes after the option has its value. They are handed over in
    /// name order, then the children, and the builder decides what each is
    /// to the element. An option the layout vocabulary refuses stops the
    /// render.
    pub(crate) fn tree(&mut self, element: &str, attrs: &[(String, Expr)], children: &[Expr], binds: &[(crate::ir::BindAt, u32)]) -> String {
        if element == "widget" {
            return self.widget(attrs, children);
        }
        let mut out = String::from("{ ");
        let mut named: Vec<(String, String)> = Vec::new();
        for (i, (k, v)) in attrs.iter().enumerate() {
            let g = self.given_option(v);
            let local = format!("_o{}·", i);
            out.push_str(&format!("let {}: tree::Given = {}; ", local, g));
            // (the last of a name given twice, as a map keeps it)
            named.retain(|(n, _)| n != k);
            named.push((k.clone(), local));
        }
        named.sort_by(|a, b| a.0.cmp(&b.0));
        let pairs: Vec<String> = named.iter().map(|(k, l)| format!("({}, {})", super::exprs::rust_str_lit(k), l)).collect();
        out.push_str(&format!("let _o·: Vec<(String, tree::Given)> = vec![{}]; ", pairs.join(", ")));
        // a table's place among the render's paged and sorted ones, and the
        // page and order the session holds for it
        let tables = if element == "table" && self.reactive_render_fns.contains(&self.cur_fn) {
            format!("state.table_state({:?}, &_o·)", element)
        } else {
            "tree::TableState::default()".to_string()
        };
        out.push_str(&format!("let _t· = {}; ", tables));
        let kids: Vec<String> = children.iter().map(|c| self.given_child(element, c)).collect();
        out.push_str(&format!("let _k·: Vec<tree::Given> = vec![{}]; ", kids.join(", ")));
        // its bindings (derive::bind_sites), placed by a served render
        // alone: a node a handler builds holds none
        if !binds.is_empty() && self.in_live() {
            let bs: Vec<String> = binds
                .iter()
                .map(|(at, id)| match at {
                    crate::ir::BindAt::Child(i) => format!("(tree::BindAt::Child({}), {})", i, id),
                    crate::ir::BindAt::Option(n) => format!("(tree::BindAt::Option({:?}), {})", n, id),
                })
                .collect();
            out.push_str(&format!(
                "tree::element_bound({:?}, _o·, _k·, _t·, if state.binding() {{ &[{}] }} else {{ &[] }}){} }}",
                element,
                bs.join(", "),
                self.or_stop()
            ));
            return out;
        }
        out.push_str(&format!("tree::element({:?}, _o·, _k·, _t·){} }}", element, self.or_stop()));
        out
    }

    /// The program's bindings (derive::bind_sites) as a served page's
    /// values turn asks for them (the live runtime's `Values`): what each reads,
    /// and each computed for a session as a view's place computes it — the
    /// same code the view's element is given, in a render's context.
    /// Whether the program's bindings are emitted (`emit_values`).
    pub(crate) fn values_emitted(&self) -> bool {
        !self.part.bindings.is_empty() && self.needs_ui && self.has_page_routes
    }

    pub(crate) fn emit_values(&mut self) {
        let bindings = self.part.bindings.clone();
        if !self.values_emitted() {
            return;
        }
        self.needs_crow_runtime = true;
        let name = "__crow_bound".to_string();
        self.views.insert(name.clone());
        self.reactive_render_fns.insert(name.clone());
        let saved = (std::mem::take(&mut self.scopes), std::mem::replace(&mut self.cur_fn, name.clone()), self.facts_in_state, self.cur_ret.take(), self.returns_unit, self.view_mark);
        self.facts_in_state = true;
        self.view_mark = false;
        self.line("#[allow(unused_variables, unused_mut, unused_braces, non_snake_case, unreachable_code, clippy::all)]");
        self.line("fn __crow_bound(state: &CrowLiveState, id: u32) -> Option<tree::Bound> {");
        self.indent += 1;
        self.line("match id {");
        self.indent += 1;
        for b in &bindings {
            self.scopes = vec![HashMap::new()];
            self.cur_ret = Some(Ty::Unknown);
            self.returns_unit = false;
            self.line(&format!("{} => {{", b.id));
            self.indent += 1;
            let g = if b.option { self.given_option(&b.expr) } else { self.given_child(&b.element, &b.expr) };
            self.line(&format!("let _g·: tree::Given = {};", g));
            self.line(if b.option { "tree::Bound::of_option(_g·)" } else { "tree::Bound::of_child(_g·)" });
            self.indent -= 1;
            self.line("}");
        }
        self.line("_ => None,");
        self.indent -= 1;
        self.line("}");
        self.indent -= 1;
        self.line("}");
        self.line("#[allow(dead_code)]");
        self.line("fn __crow_values_eval(state: &CrowLiveState, ids: &[u32]) -> Option<Vec<tree::Bound>> {");
        self.indent += 1;
        self.line("// (one that stops is the render's to stop, as the program does)");
        self.line("crow_runtime::stops::guarded(|| ids.iter().map(|id| __crow_bound(state, *id)).collect::<Option<Vec<_>>>()).ok().flatten()");
        self.indent -= 1;
        self.line("}");
        self.line("struct __CrowValues;");
        self.line("impl crow_web::live::Values for __CrowValues {");
        self.indent += 1;
        self.line("fn inputs(&self, id: u32) -> &'static [&'static str] {");
        self.indent += 1;
        self.line("match id {");
        self.indent += 1;
        for b in &bindings {
            let names: Vec<String> = b.inputs.iter().map(|n| format!("{:?}", n)).collect();
            self.line(&format!("{} => &[{}],", b.id, names.join(", ")));
        }
        self.line("_ => &[],");
        self.indent -= 1;
        self.line("}");
        self.indent -= 1;
        self.line("}");
        self.line("fn refresh(&self, state: &CrowLiveState, name: &str) {");
        self.indent += 1;
        self.line("match name {");
        self.indent += 1;
        for d in self.part.derives.clone() {
            self.line(&format!("{:?} => {{ let _ = __derive_{}(state); }}", d.name, d.name));
        }
        self.line("_ => {}");
        self.indent -= 1;
        self.line("}");
        self.indent -= 1;
        self.line("}");
        self.line("fn eval(&self, state: &CrowLiveState, ids: &[u32]) -> Option<Vec<tree::Bound>> {");
        self.indent += 1;
        self.line("__crow_values_eval(state, ids)");
        self.indent -= 1;
        self.line("}");
        self.indent -= 1;
        self.line("}");
        let (scopes, cur_fn, facts, ret, unit, mark) = saved;
        self.scopes = scopes;
        self.cur_fn = cur_fn;
        self.facts_in_state = facts;
        self.cur_ret = ret;
        self.returns_unit = unit;
        self.view_mark = mark;
        self.views.remove(&name);
        self.reactive_render_fns.remove(&name);
    }

    /// An option's value as the builder takes it (`tree::Given`): a
    /// handler by its name, a held call with the arguments it froze as
    /// the page sends them back, a literal map or list item by item, and
    /// a value by its type; one that failed is left out.
    fn given_option(&mut self, v: &Expr) -> String {
        match Emitter::raw(v) {
            Expr::FnRef(f, _) => return format!("tree::Given::Handler({})", super::exprs::rust_str_lit(f)),
            Expr::Deferred { name, args } => {
                self.needs_serde_json = true;
                let strs: Vec<String> = args.iter().map(|x| format!("serde_json::json!(({}))", self.expr(x))).collect();
                return format!("tree::Given::Call({}, vec![{}])", super::exprs::rust_str_lit(name), strs.join(", "));
            }
            Expr::ListLit(items, _) => {
                let gs: Vec<String> = items.iter().map(|i| self.given_option(i)).collect();
                return format!("tree::Given::List(vec![{}])", gs.join(", "));
            }
            _ => {}
        }
        if let Some(pairs) = map_literal_pairs(v) {
            let gs: Vec<String> = pairs.iter().map(|(k, x)| format!("({}, {})", super::exprs::rust_str_lit(k), self.given_option(x))).collect();
            return format!("tree::Given::Map(vec![{}])", gs.join(", "));
        }
        let ty = self.tt(v);
        let x = self.expr(v);
        match ty {
            Ty::Result(inner) => format!("{{ let _v· = &({}); if _v·.ok {{ {} }} else {{ tree::Given::Shown(String::new()) }} }}", x, Self::given_value("_v·.value", &inner)),
            other => Self::given_value(&format!("({})", x), &other),
        }
    }

    /// A value of a type as an option's: text, a number, a truth, a list
    /// of them; any other as it shows, which no option reads.
    fn given_value(x: &str, ty: &Ty) -> String {
        match ty {
            Ty::Str | Ty::Markup => format!("tree::Given::Text({}.to_string())", x),
            Ty::Int => format!("tree::Given::Int({} as i64)", x),
            Ty::Float => format!("tree::Given::Float({} as f64)", x),
            Ty::Bool => format!("tree::Given::Bool({})", x),
            Ty::List(item) if matches!(item.as_ref(), Ty::Str | Ty::Markup) => format!("tree::Given::List({}.iter().map(|_i·| tree::Given::Text(_i·.to_string())).collect())", x),
            _ => format!("{{ let _ = &{}; tree::Given::Shown(String::new()) }}", x),
        }
    }

    /// A child as the builder takes it: a value that failed as its reason,
    /// a node, markup (a node a local holds is its markup), a list item by
    /// item, a frame's column cell by cell, a table's frame as its rows,
    /// text; anything else as `log` writes it.
    fn given_child(&mut self, element: &str, c: &Expr) -> String {
        let raw = Emitter::raw(c);
        // a literal text, or text with spans: a failed span shows its reason
        if matches!(raw, Expr::Const(Const::Str(_), _) | Expr::Interp(_)) {
            let s = self.expr(raw);
            if matches!(self.tt(raw), Ty::Result(_)) {
                return format!("{{ let _c· = &({}); if !_c·.ok {{ tree::Given::Failed(_c·.reason.clone()) }} else {{ tree::Given::Text(_c·.value.to_string()) }} }}", s);
            }
            return format!("tree::Given::Text(({}).to_string())", s);
        }
        // (`[]` contributes nothing, whatever its items would have been)
        if matches!(raw, Expr::ListLit(items, _) if items.is_empty()) {
            return "tree::Given::List(Vec::new())".to_string();
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
            return format!("tree::Given::from({})", self.expr(raw));
        }
        // a local reads as the type it was declared with (markup when it
        // holds a node's markup); anything else as the IR types it
        let ty = match raw {
            Expr::Local(n, _) | Expr::Move(n, _) => self.local_ty(n).unwrap_or_else(|| self.tt_full(raw)),
            _ => self.tt_full(raw),
        };
        let s = self.expr(raw);
        match ty {
            Ty::Result(inner) => format!("{{ let _c· = &({}); if !_c·.ok {{ tree::Given::Failed(_c·.reason.clone()) }} else {{ {} }} }}", s, Self::given_kid("(&_c·.value)", &inner, element)),
            other => Self::given_kid(&format!("(&{})", s), &other, element),
        }
    }

    /// A child's value, `x` an expression that lends it, by its type.
    fn given_kid(x: &str, ty: &Ty, element: &str) -> String {
        match ty {
            Ty::Markup => format!("tree::Given::Markup({}.to_string())", x),
            Ty::Str => format!("tree::Given::Text({}.to_string())", x),
            Ty::Int | Ty::Big | Ty::Decimal | Ty::Fraction | Ty::Float | Ty::Bool => format!("tree::Given::Text(format!(\"{{}}\", {}))", x),
            Ty::Node => format!("tree::Given::from({}.clone())", x),
            Ty::List(item) => format!("tree::Given::List({}.iter().map(|_i·| {}).collect())", x, Self::given_kid("_i·", item, element)),
            // (a frame's column contributes each cell, as a list does)
            Ty::Column(_) => format!("tree::Given::column({})", x),
            Ty::Frame(_) if element == "table" => format!("tree::Given::Frame(Box::new({}.clone()))", x),
            // anything else is text, as `log` writes it (spec, Views): a
            // tensor, a map, a record, a moment, a parsed value, a frame
            // outside a table
            _ => format!("tree::Given::Text(crow_runtime::display::CrowDisplay::crow_fmt({}))", x),
        }
    }

    /// `widget[name, props = …, on = map[event = handler]]`.
    fn widget(&mut self, attrs: &[(String, Expr)], children: &[Expr]) -> String {
        let module = children.first().and_then(|c| if let Expr::Const(Const::Str(s), _) = Emitter::raw(c) { Some(s.clone()) } else { None }).unwrap_or_default();
        let mut props = "serde_json::Value::Null".to_string();
        // props that can fail: the reason in the widget's place when they do
        let mut failed: Option<String> = None;
        let mut on: Vec<String> = Vec::new();
        for (k, v) in attrs {
            match k.as_str() {
                "props" => {
                    self.needs_serde_json = true;
                    if matches!(self.tt(v), Ty::Result(_)) {
                        failed = Some(self.expr(v));
                        props = "serde_json::to_value(&_p·.value).unwrap_or(serde_json::Value::Null)".to_string();
                    } else {
                        props = format!("serde_json::to_value(&{}).unwrap_or(serde_json::Value::Null)", self.expr(v));
                    }
                }
                "on" => {
                    for (event, h) in map_literal_pairs(v).unwrap_or_default() {
                        if let Some(f) = fn_named(&h) {
                            on.push(format!("({}, {})", super::exprs::rust_str_lit(&event), super::exprs::rust_str_lit(f)));
                        }
                    }
                }
                _ => {}
            }
        }
        let widget = format!("tree::widget({:?}, {}, vec![{}])", module, props, on.join(", "));
        match failed {
            Some(p) => format!("{{ let _p· = &({}); if !_p·.ok {{ tree::failed(&_p·.reason) }} else {{ {} }} }}", p, widget),
            None => widget,
        }
    }

    /// The bound inputs, a progress bar, an icon and a chart.
    pub(crate) fn ui_builtin(&mut self, name: &str, args: &[Expr], a: &[String]) -> Option<String> {
        if !self.needs_ui {
            return None;
        }
        let n = a.len();
        // a bound input given what any element takes, after its own
        // arguments: the control, then those on its outermost element
        if let Some(base) = crate::ops::tree::control_arity(name)
            && n == base + crate::ops::tree::CONTROL_OWN.len()
        {
            let control = self.ui_builtin(name, &args[..base], &a[..base])?;
            // (`change = h`: a handler of the program's; none given is 0)
            let change = match Emitter::raw(&args[base + 3]) {
                Expr::FnRef(f, _) if self.fns.contains_key(f) => format!("Some(\"{}\")", f),
                _ => "None".to_string(),
            };
            return Some(format!("tree::owned({}, &{}, &{}, &{}, {}){}", control, a[base], a[base + 1], a[base + 2], change, self.or_stop()));
        }
        Some(match name {
            "slider" if n == 5 => {
                let key = Self::bound_key_of(&args[1]);
                let json = self.json_of(&args[1], "_v·");
                format!("{{ let _v· = &({}); tree::slider(&{}, \"{}\", {}, &format!(\"{{}}\", _v·), ({}) as f64, ({}) as f64, ({}) as f64) }}", a[1], a[0], key, json, a[2], a[3], a[4])
            }
            "field" if n == 4 => {
                let key = Self::bound_key_of(&args[1]);
                if matches!(Emitter::raw(&args[2]), Expr::Const(Const::Str(s), _) if s == "file") {
                    return Some(format!("tree::field(&{}, \"{}\", serde_json::Value::Null, &({}).get(\"name\").cloned().unwrap_or_default(), \"file\", 0)", a[0], key, a[1]));
                }
                let kind = match Emitter::raw(&args[2]) {
                    Expr::Const(Const::Str(s), _) if s.is_empty() => match self.tt(&args[1]) {
                        Ty::Int | Ty::Float => "\"number\"".to_string(),
                        _ => "\"text\"".to_string(),
                    },
                    _ => format!("&{}", a[2]),
                };
                let json = self.json_of(&args[1], "_v·");
                format!("{{ let _v· = &({}); tree::field(&{}, \"{}\", {}, &format!(\"{{}}\", _v·), {}, ({}) as i64) }}", a[1], a[0], key, json, kind, a[3])
            }
            "checkbox" | "toggle" if n == 2 => {
                let key = Self::bound_key_of(&args[1]);
                format!("tree::check(&{}, \"{}\", format!(\"{{}}\", {}) == \"true\", {})", a[0], key, a[1], name == "toggle")
            }
            "radio" | "tabs" if n == 2 => {
                let key = Self::bound_key_of(&args[1]);
                format!("tree::choice(({}).iter().map(|_i·| _i·.to_string()).collect(), \"{}\", &format!(\"{{}}\", {}), {})", a[0], key, a[1], name == "tabs")
            }
            "progress" if n == 2 => format!("tree::progress(({}) as f64, ({}) as f64)", a[0], a[1]),
            // (a name, a size or a colour the set does not hold is the
            // render's stop, as the interpreter has it)
            "icon" if n == 3 => format!("tree::icon(&{}, &{}, &{}){}", a[0], a[1], a[2], self.or_stop()),
            "select" if n == 2 => {
                let key = Self::bound_key_of(&args[1]);
                format!("tree::select(({}).iter().map(|_i·| _i·.to_string()).collect(), \"{}\", &format!(\"{{}}\", {}))", a[0], key, a[1])
            }
            "chart" if n == 12 => self.chart(args, a),
            _ => return None,
        })
    }

    /// A bound variable's value as the data form holds it, `at` the
    /// expression that lends it: text, a number, a truth as themselves.
    fn json_of(&mut self, var: &Expr, at: &str) -> String {
        self.needs_serde_json = true;
        match self.tt(var) {
            Ty::Str | Ty::Int | Ty::Float | Ty::Bool => format!("crow_runtime::json::ToJson::to_json({})", at),
            _ => format!("serde_json::Value::String(format!(\"{{}}\", {}))", at),
        }
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
            "tree::chart(_f·, &{x}, &{cd}, &{ln}, &{by}, {{ let _m·: f64 = ({mk}) as f64; if _m· <= 0.0 {{ f64::NAN }} else {{ _m· }} }}, &{tz}, ({h}) as f64, &{pn}, &{lv}, _pts·, {sk})",
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
            Expr::Const(Const::Int(0), _) => format!("{{ let _pts·: Option<&crow_runtime::frame::CrowFrame> = None; {} }}", core),
            _ if matches!(self.tt(&args[10]), Ty::Result(_)) => format!("{{ let _pr· = &({}); let _pts· = if _pr·.ok {{ Some(&_pr·.value) }} else {{ None }}; {} }}", a[10], core),
            _ => format!("{{ let _pf·: &crow_runtime::frame::CrowFrame = &{}; let _pts· = Some(_pf·); {} }}", a[10], core),
        };
        if matches!(frame_ty, Ty::Result(_)) {
            return format!("{{ let _fr· = &({}); if !_fr·.ok {{ tree::failed(&_fr·.reason) }} else {{ let _f· = &_fr·.value; {} }} }}", a[0], with_points);
        }
        format!("{{ let _f· = &{}; {} }}", a[0], with_points)
    }
}
