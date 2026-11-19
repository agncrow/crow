//! Views as derivations (one-view-design.md §4.4, phase 2): a view's last
//! node, kept with what it read, handed back when nothing it read moved.
//!
//! A view's read set has two halves. The facts it reads, itself and through
//! every function it calls, are fixed when the program is compiled (the
//! same analysis for both backends, `derive::view_reads`): their versions
//! are compared. The derivations it read are recorded as it runs, in the
//! order it read them: each is brought up to date again, in that order,
//! and its version compared — a derivation recomputed to an equal value
//! keeps its version (early cutoff), so the view is not run again. A view
//! that read every fact and every derivation the same way takes the same
//! branches, so the derivations it reads are the ones it read last time.
//!
//! The paged and sorted tables a view draws are numbered by their place
//! in the render: a view is reused only where it starts at the same
//! numbers and every table it drew is on the page, and in the order, it
//! was, and a reused view moves the numbers on as running it would.
//!
//! A view whose answer depends on the moment, on chance or on something
//! the run holds (`now`, `random`, `captured`, `running`, the notice) runs
//! at every render; so does one called with an argument the cache cannot
//! compare (a function, a node) — its caller's entry still covers it.
//!
//! A view a render did not call stays kept for a later render, within a
//! budget per session (`KEEP_UNUSED` views, `KEEP_WEIGHT` nodes), the least
//! recently used let go first: a tab switched back to finds the views it
//! showed, each checked as a live one is.
//!
//! `CROW_VIEW_CACHE=off` runs every view at every render;
//! `CROW_VIEW_CACHE=check` runs every view too and stops the render when a
//! view the cache would have reused answers differently.

use super::CrowLiveState;
use crate::view::tree::{self, Node};
use std::collections::HashMap;

/// What a view reads that the compiler can tell (see the module's note).
#[derive(Debug)]
pub struct ViewSpec {
    pub name: &'static str,
    /// the facts it reads, itself or through what it calls
    pub facts: &'static [&'static str],
    /// why it runs at every render, if it does
    pub always: Option<&'static str>,
    /// the facts and derivations among those it reads only in its
    /// bindings (derive::bind_sites): one that moves alone is set on the
    /// page where it shows, with no view run (`values_turn`)
    pub bound: &'static [&'static str],
}

/// One call of a view: its name, its arguments' canonical text, and which
/// call of that view with those arguments it is in the render.
type Key = (&'static str, String, usize);

#[derive(Debug, Clone)]
struct Entry {
    node: Node,
    /// the facts it reads (its spec's, in order) and those it reads only in
    /// its bindings
    spec_facts: &'static [&'static str],
    bound: &'static [&'static str],
    facts: Vec<u64>,
    derives: Vec<Read>,
    /// the table numbers it started at, and how many it drew
    drawn_at: (usize, usize),
    drew: (usize, usize),
    /// the page each paged table it drew was on, the order of each sorted one
    pages: Vec<Option<usize>>,
    sorts: Vec<Option<(String, bool)>>,
    /// the cached calls inside it, which a reuse keeps
    inner: Vec<Key>,
    /// the text of the arguments it binds (left out of its key), as its
    /// node's bindings show them: none once a values turn set them anew
    bound_args: Option<String>,
    /// the session's world count (`ViewCache.epoch`) when what it read
    /// was last seen where it is: while the count stands, nothing it read
    /// can have moved
    epoch: u64,
    used: u64,
    /// its tree's size (`tree::weight`), counted once it is first left
    /// unused
    weight: Option<usize>,
}

/// A derivation a view read: its name, the version it saw, and whether it
/// read it where it decides what the view makes (`structural`) or only in
/// bindings — its own (`ViewSpec.bound`) and those of the views it called,
/// a derivation read either way by any of them being structural.
#[derive(Debug, Clone, Copy)]
struct Read {
    name: &'static str,
    version: u64,
    structural: bool,
}

/// A derivation read by a view inside `into`'s: kept once, structural when
/// either read was.
fn merge_read(into: &mut Vec<Read>, d: Read) {
    match into.iter_mut().find(|r| r.name == d.name) {
        Some(r) => r.structural |= d.structural,
        None => into.push(d),
    }
}

/// A view being run: what it read so far.
#[derive(Debug)]
struct Run {
    key: Option<Key>,
    spec_facts: &'static [&'static str],
    bound: &'static [&'static str],
    facts: Vec<u64>,
    derives: Vec<Read>,
    drawn_at: (usize, usize),
    inner: Vec<Key>,
    bound_args: Option<String>,
    /// it, or a view inside it, read a value still being computed
    stale: bool,
    /// the node the cache held, kept to compare under `check`
    expect: Option<Node>,
}

/// A session's kept views.
#[derive(Debug, Default)]
pub(crate) struct ViewCache {
    entries: HashMap<Key, Entry>,
    runs: Vec<Run>,
    calls: HashMap<(&'static str, String), usize>,
    render: u64,
    /// views run and reused in the last render (the dev panel's, a test's)
    pub(crate) ran: usize,
    pub(crate) reused: usize,
    /// the calls the render made outside any view, and the first one's key
    tops: usize,
    root: Option<Key>,
    /// the view whose node the last render's page is, when it is one
    /// view's node alone (`views_page`): what a values turn starts from
    page_root: Option<Key>,
    /// a kept view's bindings are being computed again: what they read is
    /// no read of the view's (its parent takes what the view read)
    quiet: bool,
    /// where the session's world stood at the last settle (`settle`), and
    /// how many times it has moved: the session's facts, the app's table
    /// writes and every derivation's moves
    world: Option<World>,
    epoch: u64,
    /// this render has settled
    settled: bool,
    /// every derivation a render read that answers by the moment (the
    /// clock, a source past its freshness): settled at each render
    timed: std::collections::BTreeSet<&'static str>,
    /// every derivation a render read, by name, for a kept view to note
    /// again as read (the session wakes for the tables they read)
    specs: HashMap<&'static str, SpecOf>,
}

/// A derivation's spec, as the cache holds it by name.
#[derive(Clone, Copy)]
struct SpecOf(&'static super::DeriveSpec);

impl std::fmt::Debug for SpecOf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.name)
    }
}

/// Where everything a derivation reads stood (`ViewCache.world`).
#[derive(Debug, Clone, PartialEq)]
struct World {
    facts: HashMap<String, u64>,
    tables: u64,
    derives: u64,
}

/// A program's bindings, as a values turn asks for them (one-view-design.md
/// 3.5): what each reads, and its value computed for a session. Both
/// modes hand one to the live runtime (`CrowLiveView::with_values`).
pub trait Values: Send + Sync {
    /// the facts and derivations binding `id` reads
    fn inputs(&self, id: u32) -> &'static [&'static str];
    /// bring a derivation up to date by its name, as reading it does
    fn refresh(&self, state: &CrowLiveState, name: &str);
    /// each binding's value for the session, in the order asked; none when
    /// one cannot be computed (a stop): the page renders as the program does
    fn eval(&self, state: &CrowLiveState, ids: &[u32]) -> Option<Vec<tree::Bound>>;
}

/// The switch: `off` runs every view, `check` runs every view and compares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    On,
    Off,
    Check,
}

pub fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("CROW_VIEW_CACHE").as_deref() {
        Ok("off") | Ok("0") => Mode::Off,
        Ok("check") => Mode::Check,
        _ => Mode::On,
    })
}

/// A view's call as the cache answers it: its kept node, or a run to make.
pub enum ViewCall {
    /// the kept node, and the table numbers the render is at after it
    Reuse(Node, (usize, usize)),
    /// run the view, and hand its node to `view_end`
    Run(ViewRunning),
}

/// A run the cache waits on (its place on the view stack).
pub struct ViewRunning {
    depth: usize,
    stale_depth: usize,
}

impl CrowLiveState {
    fn view_cache(&self) -> std::sync::MutexGuard<'_, ViewCache> {
        self.views.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The render's table numbers so far (paged, sorted).
    pub fn tables_drawn(&self) -> (usize, usize) {
        use std::sync::atomic::Ordering::SeqCst;
        (self.tables_drawn.load(SeqCst), self.sorts_drawn.load(SeqCst))
    }

    /// The render's table numbers moved on to where a reused view leaves them.
    pub fn set_tables_drawn(&self, drawn: (usize, usize)) {
        use std::sync::atomic::Ordering::SeqCst;
        self.tables_drawn.store(drawn.0, SeqCst);
        self.sorts_drawn.store(drawn.1, SeqCst);
    }

    /// A render starts: calls are counted afresh.
    pub(crate) fn views_begin(&self) {
        let mut c = self.view_cache();
        c.render += 1;
        c.calls.clear();
        c.runs.clear();
        c.ran = 0;
        c.reused = 0;
        c.tops = 0;
        c.root = None;
        c.page_root = None;
        c.settled = false;
    }

    /// A derivation a render read: kept by name, and among the ones that
    /// answer by the moment when it does.
    pub(crate) fn view_note_spec(&self, spec: &'static super::DeriveSpec) {
        let mut c = self.view_cache();
        if spec.now || spec.external {
            c.timed.insert(spec.name);
        }
        c.specs.entry(spec.name).or_insert(SpecOf(spec));
    }

    /// Before a render's first view, and a values turn's check: the
    /// derivations that answer by the moment are brought up to date, and
    /// when the session's facts, the app's table writes and every
    /// derivation's moves stand where the last settle found them, nothing
    /// any derivation reads has moved: a kept view whose world count is
    /// the session's needs no derivation of its brought up to date
    /// (one-view-design.md §7, phase 3 finished). Otherwise the count moves
    /// on.
    pub(crate) fn settle(&self, refresh: &dyn Fn(&str)) {
        let timed: Vec<&'static str> = {
            let mut c = self.view_cache();
            if c.settled {
                return;
            }
            c.settled = true;
            c.timed.iter().copied().collect()
        };
        for n in timed {
            refresh(n);
        }
        use std::sync::atomic::Ordering::SeqCst;
        let world = World { facts: self.versions.clone(), tables: super::TABLE_MOVES.load(SeqCst), derives: super::DERIVE_MOVES.load(SeqCst) };
        // (CROW_SETTLE=off: every render checks every kept view's
        // derivations, as before — to compare)
        static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let off = *OFF.get_or_init(|| matches!(std::env::var("CROW_SETTLE").as_deref(), Ok("off") | Ok("0")));
        let mut c = self.view_cache();
        if off || c.world.as_ref() != Some(&world) {
            c.epoch += 1;
            c.world = Some(world);
        }
    }

    /// A render is done: views it did not call are kept for a later
    /// render to reuse (a tab switched back to, a row scrolled back), within
    /// the budget (`KEEP_UNUSED`, `KEEP_WEIGHT`), the least recently used
    /// let go first. A kept one is checked on reuse as any other is.
    pub(crate) fn views_end(&self) {
        let mut c = self.view_cache();
        let now = c.render;
        c.runs.clear();
        let mut unused: Vec<(u64, usize, Key)> = Vec::new();
        let mut weight = 0;
        for (k, e) in c.entries.iter_mut() {
            if e.used != now {
                let w = *e.weight.get_or_insert_with(|| tree::weight(&e.node));
                weight += w;
                unused.push((e.used, w, k.clone()));
            }
        }
        if unused.len() <= KEEP_UNUSED && weight <= KEEP_WEIGHT {
            return;
        }
        // the oldest first; among those of one render, any order
        unused.sort_by_key(|(used, _, _)| *used);
        let mut left = unused.len();
        for (_, w, k) in unused {
            if left <= KEEP_UNUSED && weight <= KEEP_WEIGHT {
                break;
            }
            c.entries.remove(&k);
            weight -= w;
            left -= 1;
        }
    }

    /// The views a session keeps now (a test's).
    #[cfg(test)]
    pub(crate) fn views_kept_now(&self) -> usize {
        self.view_cache().entries.len()
    }

    /// A values turn's count: no view run, none reused (the render
    /// `CROW_VIEW_CACHE=check` makes beside it is not counted).
    pub(crate) fn views_counted_zero(&self) {
        let mut c = self.view_cache();
        c.ran = 0;
        c.reused = 0;
    }

    /// Views run and reused in the last render.
    pub fn views_counted(&self) -> (usize, usize) {
        let c = self.view_cache();
        (c.ran, c.reused)
    }

    /// A derivation a render read: the views running record it — unless
    /// it is read inside another derivation's own reading (its inputs
    /// brought up to date, its body computed: `deriving`), which makes it
    /// that derivation's input, not the view's. The view depends on the
    /// derivation it read by its version alone: one brought up to date
    /// inside the view's run had counted all it read among the view's
    /// structural reads, and a text over a derivation of the clock ran its
    /// view at every tick where it is a binding.
    pub(crate) fn view_read_derive(&self, name: &'static str) {
        if !self.rendering() || self.in_derivation() {
            return;
        }
        let version = self.derive_version(name);
        let mut c = self.view_cache();
        if c.quiet {
            return;
        }
        if let Some(top) = c.runs.last_mut() {
            let structural = !top.bound.contains(&plain(name));
            if let Some(slot) = top.derives.iter_mut().find(|r| r.name == name) {
                slot.version = version;
                slot.structural |= structural;
            } else {
                top.derives.push(Read { name, version, structural });
            }
        }
    }

    fn fact_versions(&self, spec: &ViewSpec) -> Vec<u64> {
        spec.facts.iter().map(|f| self.versions.get(*f).copied().unwrap_or(0)).collect()
    }

    /// A view is called (`args`: its arguments' canonical text, none when
    /// they cannot be compared; `drawn`: the render's table numbers;
    /// `refresh`: brings a derivation up to date by its name, as reading it
    /// does). Its kept node when nothing it read moved, else a run.
    pub fn view_begin(&self, spec: &'static ViewSpec, args: Option<String>, drawn: (usize, usize), refresh: &dyn Fn(&str)) -> ViewCall {
        self.view_call(spec.name, spec.facts, spec.always, spec.bound, args, None, drawn, refresh, &|_| None)
    }

    /// `view_begin` for a view's copy that binds some of its arguments
    /// (`bound`: their text, left out of `args`; derive::specialize): a
    /// kept node handed them anew has its bindings computed again
    /// (`eval`, as a values turn computes them) and set.
    #[allow(clippy::too_many_arguments)]
    pub fn view_begin_bound(&self, spec: &'static ViewSpec, args: Option<String>, bound: Option<String>, drawn: (usize, usize), refresh: &dyn Fn(&str), eval: &dyn Fn(&[u32]) -> Option<Vec<tree::Bound>>) -> ViewCall {
        self.view_call(spec.name, spec.facts, spec.always, spec.bound, args, bound, drawn, refresh, eval)
    }

    /// The version of a fact or a derivation a view's argument is the value
    /// of (`derive::ArgUse::Source`): the same version, the same value.
    pub fn source_version(&self, name: &str, derive: bool) -> u64 {
        if derive { self.derive_version(name) } else { self.versions.get(name).copied().unwrap_or(0) }
    }

    /// `view_begin` as the compiled program calls it, its spec's parts
    /// given apart: the run to make, or the kept node and the table numbers
    /// after it.
    #[allow(clippy::too_many_arguments)]
    pub fn view_kept(
        &self,
        name: &'static str,
        facts: &'static [&'static str],
        always: Option<&'static str>,
        bound: &'static [&'static str],
        args: Option<String>,
        bound_args: Option<String>,
        refresh: &dyn Fn(&str),
        eval: &dyn Fn(&[u32]) -> Option<Vec<tree::Bound>>,
    ) -> Result<ViewRunning, Box<Node>> {
        match self.view_call(name, facts, always, bound, args, bound_args, self.tables_drawn(), refresh, eval) {
            ViewCall::Run(r) => Ok(r),
            ViewCall::Reuse(n, d) => {
                self.set_tables_drawn(d);
                Err(Box::new(n))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn view_call(
        &self,
        name: &'static str,
        facts: &'static [&'static str],
        always: Option<&'static str>,
        bound: &'static [&'static str],
        args: Option<String>,
        bound_args: Option<String>,
        drawn: (usize, usize),
        refresh: &dyn Fn(&str),
        eval: &dyn Fn(&[u32]) -> Option<Vec<tree::Bound>>,
    ) -> ViewCall {
        let spec = ViewSpec { name, facts, always, bound };
        let spec = &spec;
        let mode = mode();
        if self.rendering() && mode == Mode::On && !self.copy && !self.app {
            self.settle(refresh);
        }
        let cacheable = self.rendering() && mode != Mode::Off && spec.always.is_none() && !self.copy && !self.app;
        let key = match (cacheable, args) {
            (true, Some(a)) => {
                let mut c = self.view_cache();
                let n = c.calls.entry((spec.name, a.clone())).or_insert(0);
                *n += 1;
                let key = (spec.name, a, *n);
                if let Some(top) = c.runs.last_mut() {
                    top.inner.push(key.clone());
                }
                Some(key)
            }
            _ => None,
        };
        if self.rendering() {
            // a call outside every view: the page's own, the first its root
            let mut c = self.view_cache();
            if c.runs.is_empty() {
                c.tops += 1;
                if c.tops == 1 {
                    c.root = key.clone();
                }
            }
        }
        let facts = if key.is_some() { self.fact_versions(spec) } else { Vec::new() };
        let _ = &spec.name;
        let stale_depth = self.view_enter();
        let depth = {
            let mut c = self.view_cache();
            c.runs.push(Run { key: key.clone(), spec_facts: spec.facts, bound: spec.bound, facts: facts.clone(), derives: Vec::new(), drawn_at: drawn, inner: Vec::new(), bound_args: bound_args.clone(), stale: false, expect: None });
            c.runs.len() - 1
        };
        let running = ViewRunning { depth, stale_depth };
        let Some(key) = key else { return ViewCall::Run(running) };
        let entry = match self.view_cache().entries.get(&key) {
            Some(e) if e.facts.len() == facts.len() && e.drawn_at == drawn => e.clone(),
            _ => return ViewCall::Run(running),
        };
        // a fact that moved and that it reads only in its bindings is set
        // where it shows (below); any other runs it
        let mut moved = false;
        for (i, f) in spec.facts.iter().enumerate() {
            if entry.facts[i] != facts[i] {
                if !spec.bound.contains(f) {
                    return ViewCall::Run(running);
                }
                moved = true;
            }
        }
        // the tables it drew must be where they were
        if !entry.pages.is_empty() {
            let pages = self.table_pages();
            if (0..entry.pages.len()).any(|i| pages.get(&(drawn.0 + i)).copied() != entry.pages[i]) {
                return ViewCall::Run(running);
            }
        }
        if !entry.sorts.is_empty() {
            let mut sorts = self.table_sorts();
            if (0..entry.sorts.len()).any(|i| sorts.remove(&(drawn.1 + i)) != entry.sorts[i]) {
                return ViewCall::Run(running);
            }
        }
        // the derivations it read, brought up to date in the order it read
        // them (each recorded on this run as it is read)
        let mut derives = entry.derives.clone();
        // nothing anywhere moved since it was last seen: what it read is
        // where it was (`settle`), and it is noted as read again
        let still = mode == Mode::On && entry.epoch == self.view_cache().epoch;
        if still {
            let specs: Vec<&'static super::DeriveSpec> = {
                let c = self.view_cache();
                derives.iter().filter_map(|d| c.specs.get(d.name).map(|s| s.0)).collect()
            };
            for spec in specs {
                self.note_read(spec);
            }
        }
        for d in derives.iter_mut().filter(|_| !still) {
            refresh(d.name);
            if self.view_stale_at(stale_depth) {
                return ViewCall::Run(running);
            }
            let now = self.derive_version(d.name);
            if now != d.version {
                if d.structural {
                    return ViewCall::Run(running);
                }
                d.version = now;
                moved = true;
            }
        }
        // handed what it binds anew, or what it reads only in its bindings
        // moved: its bindings computed again from the session and set (a
        // value that cannot be placed runs it), as a values turn sets them
        let mut node = entry.node.clone();
        let handed = bound_args.as_ref().is_some_and(|now| entry.bound_args.as_ref() != Some(now));
        if moved || handed {
            let mut ids = std::collections::BTreeSet::new();
            tree::bound_ids(&node, &mut ids);
            let ids: Vec<u32> = ids.into_iter().collect();
            if !ids.is_empty() {
                self.view_cache().quiet = true;
                let values = eval(&ids);
                self.view_cache().quiet = false;
                let Some(values) = values.filter(|v| v.len() == ids.len()) else { return ViewCall::Run(running) };
                if self.view_stale_at(stale_depth) {
                    return ViewCall::Run(running);
                }
                let values: HashMap<u32, tree::Bound> = ids.into_iter().zip(values).collect();
                let mut set = 0;
                match tree::rebind(&node, &values, &mut set) {
                    Ok(Some(n)) => node = n,
                    Ok(None) => {}
                    Err(_) => return ViewCall::Run(running),
                }
            }
            if mode != Mode::Check {
                let mut c = self.view_cache();
                if let Some(e) = c.entries.get_mut(&key) {
                    e.node = node.clone();
                    e.bound_args = bound_args.clone();
                    e.facts = facts.clone();
                    e.derives = derives.clone();
                    e.weight = None;
                }
            }
        }
        if mode == Mode::Check {
            self.view_cache().runs[depth].expect = Some(node);
            return ViewCall::Run(running);
        }
        // reused: the view stack closes, its parent takes what it read, and
        // the calls inside it are kept and counted as running it counts them
        let _ = self.view_exit(stale_depth, Node::Text(String::new()));
        let mut c = self.view_cache();
        let run = c.runs.pop();
        let now = c.render;
        for k in &entry.inner {
            *c.calls.entry((k.0, k.1.clone())).or_insert(0) += 1;
            if let Some(e) = c.entries.get_mut(k) {
                e.used = now;
            }
        }
        let epoch = c.epoch;
        if let Some(e) = c.entries.get_mut(&key) {
            e.used = now;
            e.epoch = epoch;
        }
        if let Some(parent) = c.runs.last_mut() {
            parent.inner.extend(entry.inner.iter().cloned());
            // (what it read, as it read it: the refresh above read them
            // again under this run)
            let _ = run;
            for d in derives {
                merge_read(&mut parent.derives, d);
            }
        }
        c.reused += 1;
        ViewCall::Reuse(node, (drawn.0 + entry.drew.0, drawn.1 + entry.drew.1))
    }

    fn view_stale_at(&self, depth: usize) -> bool {
        self.view_stale.lock().unwrap_or_else(|p| p.into_inner()).get(depth).copied().unwrap_or(false)
    }

    /// A view's run ended without a node to keep (it stopped, or answered
    /// a value that is no node): its level closes, and it is kept nowhere,
    /// nor is the view it stands in.
    pub fn view_abandon(&self, running: ViewRunning) {
        let _ = self.view_exit(running.stale_depth, Node::Text(String::new()));
        let mut c = self.view_cache();
        c.ran += 1;
        c.runs.truncate(running.depth);
        if let Some(parent) = c.runs.last_mut() {
            parent.stale = true;
        }
    }

    /// A view's run is done (`drawn`: the render's table numbers after it):
    /// its node, marked stale as `view_exit` marks it, kept when the view
    /// can be.
    pub fn view_end(&self, running: ViewRunning, node: impl Into<Node>, drawn: (usize, usize)) -> Node {
        let stale_here = self.view_stale_at(running.stale_depth);
        let node = self.view_exit(running.stale_depth, node);
        let mut c = self.view_cache();
        c.ran += 1;
        if c.runs.len() != running.depth + 1 {
            // a view that stopped partway left its stack behind
            c.runs.truncate(running.depth + 1);
        }
        let Some(run) = c.runs.pop() else { return node };
        let stale = run.stale || stale_here;
        if let Some(parent) = c.runs.last_mut() {
            parent.stale |= stale;
            parent.inner.extend(run.inner.iter().cloned());
            for d in &run.derives {
                merge_read(&mut parent.derives, *d);
            }
        }
        if let Some(expect) = &run.expect {
            let (kept, now) = (tree::html(expect), tree::html(&node));
            assert!(kept == now, "CROW_VIEW_CACHE=check: view {} answered differently from the node the cache kept\nkept: {}\nnow:  {}", run.key.as_ref().map_or("?", |k| k.0), kept, now);
        }
        if let (Some(key), false) = (run.key, stale) {
            let drew = (drawn.0.saturating_sub(run.drawn_at.0), drawn.1.saturating_sub(run.drawn_at.1));
            drop(c);
            let pages = if drew.0 > 0 {
                let p = self.table_pages();
                (0..drew.0).map(|i| p.get(&(run.drawn_at.0 + i)).copied()).collect()
            } else {
                Vec::new()
            };
            let sorts = if drew.1 > 0 {
                let mut s = self.table_sorts();
                (0..drew.1).map(|i| s.remove(&(run.drawn_at.1 + i))).collect()
            } else {
                Vec::new()
            };
            let mut c = self.view_cache();
            let used = c.render;
            let epoch = c.epoch;
            c.entries.insert(key, Entry { node: node.clone(), spec_facts: run.spec_facts, bound: run.bound, facts: run.facts, derives: run.derives, drawn_at: run.drawn_at, drew, pages, sorts, inner: run.inner, bound_args: run.bound_args, epoch, used, weight: None });
        }
        node
    }
}

/// What a values turn found to do (`values_turn`): the page's tree with
/// its bound places set, and how many places changed.
pub(crate) struct ValuesTurn {
    pub page: Node,
    pub set: usize,
    /// the bindings computed again, by id, each with its value (a drawn
    /// renderer's `values` message: one-view-design.md 5a)
    pub values: Vec<(u32, tree::Bound)>,
}

/// A derivation's name as a program's bindings know it (the interpreter's
/// app tag left off).
fn plain(name: &str) -> &str {
    name.split_once('#').map_or(name, |(_, n)| n)
}

impl CrowLiveState {
    /// The render is over and `page` is its tree: when it is the node of
    /// the one view the render called outside every view, a values turn
    /// can start from that view's entry. `wrapped`: the runtime put a toast
    /// or a question around it, which no values turn makes.
    pub(crate) fn views_page(&self, page: &Node, wrapped: bool) {
        let mut c = self.view_cache();
        let render = c.render;
        let root = match (&c.root, c.tops, wrapped) {
            (Some(k), 1, false) => c.entries.get(k).filter(|e| e.used == render && tree::same_tree(&e.node, page)).map(|_| k.clone()),
            _ => None,
        };
        c.page_root = root;
    }

    /// A values turn (one-view-design.md 3.5, phase 3): when every fact and
    /// derivation that moved since the last render is one the views on the
    /// page read only in their bindings, the page is the last one with
    /// those bindings set — no view runs. Each view the last render placed
    /// is checked as a reuse checks it (its facts, its derivations brought
    /// up to date, its tables' pages and orders, nothing stale); a read
    /// that moved outside a binding, or anything else, is a render's
    /// (`None`). The bindings whose inputs moved are computed again, the
    /// kept views' nodes take their values (so a later render reuses them
    /// as a run would make them), and their versions move on.
    pub(crate) fn values_turn(&self, values: &dyn Values) -> Option<ValuesTurn> {
        if mode() == Mode::Off || self.copy || self.app {
            return None;
        }
        self.view_cache().page_root.as_ref()?;
        // (nothing anywhere moved since a view was last seen: it is not
        // checked again, `settle`)
        self.view_cache().settled = false;
        self.settle(&|n| values.refresh(self, n));
        type Live = (Key, &'static [&'static str], &'static [&'static str], Vec<u64>, Vec<Read>, (usize, usize), Vec<Option<usize>>, Vec<Option<(String, bool)>>);
        let (root, live, epoch): (Key, Vec<Live>, u64) = {
            let c = self.view_cache();
            let root = c.page_root.clone()?;
            let render = c.render;
            let epoch = c.epoch;
            let live = c
                .entries
                .iter()
                .filter(|(_, e)| e.used == render)
                .map(|(k, e)| {
                    if e.epoch == epoch {
                        (k.clone(), e.spec_facts, e.bound, Vec::new(), Vec::new(), e.drawn_at, Vec::new(), Vec::new())
                    } else {
                        (k.clone(), e.spec_facts, e.bound, e.facts.clone(), e.derives.clone(), e.drawn_at, e.pages.clone(), e.sorts.clone())
                    }
                })
                .collect();
            (root, live, epoch)
        };
        let current: std::collections::BTreeSet<Key> = {
            let c = self.view_cache();
            live.iter().filter(|(k, ..)| c.entries.get(k).is_some_and(|e| e.epoch == epoch)).map(|(k, ..)| k.clone()).collect()
        };
        let mut moved: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let mut refreshed: HashMap<&'static str, u64> = HashMap::new();
        let pages = self.table_pages();
        let sorts = self.table_sorts();
        // (a derivation read here that is still being computed marks this
        // level: the page would show it catching up, which a render shows)
        let stale_depth = self.view_enter();
        let checked = (|| {
            for (k, spec_facts, bound, facts, derives, drawn_at, ps, ss) in &live {
                if current.contains(k) {
                    continue;
                }
                for (i, f) in spec_facts.iter().enumerate() {
                    if self.versions.get(*f).copied().unwrap_or(0) != facts.get(i).copied().unwrap_or(0) {
                        if !bound.contains(f) {
                            return None;
                        }
                        moved.insert(f);
                    }
                }
                for d in derives {
                    let now = match refreshed.get(d.name) {
                        Some(v) => *v,
                        None => {
                            values.refresh(self, d.name);
                            let v = self.derive_version(d.name);
                            refreshed.insert(d.name, v);
                            v
                        }
                    };
                    if now != d.version {
                        if d.structural {
                            return None;
                        }
                        moved.insert(plain(d.name));
                    }
                }
                if (0..ps.len()).any(|i| pages.get(&(drawn_at.0 + i)).copied() != ps[i]) || (0..ss.len()).any(|i| sorts.get(&(drawn_at.1 + i)) != ss[i].as_ref()) {
                    return None;
                }
            }
            Some(())
        })();
        if checked.is_none() || moved.is_empty() || self.view_stale_at(stale_depth) {
            let _ = self.view_exit(stale_depth, Node::Text(String::new()));
            return None;
        }
        // the bindings placed on the page whose inputs moved, computed again
        let mut placed = std::collections::BTreeSet::new();
        {
            let c = self.view_cache();
            for (k, ..) in &live {
                if let Some(e) = c.entries.get(k) {
                    tree::bound_ids(&e.node, &mut placed);
                }
            }
        }
        let ids: Vec<u32> = placed.into_iter().filter(|id| values.inputs(*id).iter().any(|n| moved.contains(n))).collect();
        let computed = if ids.is_empty() { Some(Vec::new()) } else { values.eval(self, &ids) };
        let stale = self.view_stale_at(stale_depth);
        let _ = self.view_exit(stale_depth, Node::Text(String::new()));
        let computed = computed.filter(|v| v.len() == ids.len() && !stale)?;
        let values: Vec<(u32, tree::Bound)> = ids.into_iter().zip(computed).collect();
        let new: HashMap<u32, tree::Bound> = values.iter().cloned().collect();
        // each live view's node with its bindings set, before any is kept:
        // one that cannot be set leaves every view as it was
        let mut rebound: Vec<(Key, Option<Node>)> = Vec::with_capacity(live.len());
        let mut set = 0;
        {
            let c = self.view_cache();
            for (k, ..) in &live {
                let e = c.entries.get(k)?;
                let mut here = 0;
                let n = tree::rebind(&e.node, &new, &mut here).ok()?;
                if *k == root {
                    set = here;
                }
                rebound.push((k.clone(), n));
            }
        }
        let mut c = self.view_cache();
        for (k, n) in rebound {
            let Some(e) = c.entries.get_mut(&k) else { continue };
            if let Some(n) = n {
                e.node = n;
                e.weight = None;
                // (what it binds is no longer what its node was made from)
                e.bound_args = None;
            }
            // what it read only in its bindings is read as of now
            for (i, f) in e.spec_facts.iter().enumerate() {
                if e.bound.contains(f)
                    && let Some(slot) = e.facts.get_mut(i)
                {
                    *slot = self.versions.get(*f).copied().unwrap_or(0);
                }
            }
            for d in e.derives.iter_mut() {
                if let Some(v) = refreshed.get(d.name) {
                    d.version = *v;
                }
            }
            e.epoch = epoch;
        }
        let page = c.entries.get(&root)?.node.clone();
        c.ran = 0;
        c.reused = 0;
        Some(ValuesTurn { page, set, values })
    }
}

/// An argument's canonical text for the cache's key: the same text for
/// equal values, none for a value the cache does not compare.
pub fn key_of<T: std::fmt::Debug>(args: &T) -> String {
    format!("{:?}", args)
}

/// How many views a render did not call a session keeps at most, and how
/// big their trees may be in all (`tree::weight`: an element, a text or a
/// control is one, a table's rows one each; a view's tree counts the views
/// inside it again, so this overstates what they hold). Past either the
/// least recently used are let go. Live entries — the views the last
/// render called — are never counted against it.
pub const KEEP_UNUSED: usize = 1024;
pub const KEEP_WEIGHT: usize = 50_000;

/// The longest arguments' text a view is kept by. A view handed more
/// (a list of a thousand rows) runs again at every render: writing and
/// comparing its key would cost about what running it does, and writing
/// stops here, so a long argument costs no more than this to refuse.
pub const KEY_MAX: usize = 4096;

/// `args`' canonical text, or none past `KEY_MAX` bytes.
pub fn key_within<T: std::fmt::Debug>(args: &T) -> Option<String> {
    struct Capped(String);
    impl std::fmt::Write for Capped {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            if self.0.len() + s.len() > KEY_MAX {
                return Err(std::fmt::Error);
            }
            self.0.push_str(s);
            Ok(())
        }
    }
    let mut out = Capped(String::new());
    std::fmt::write(&mut out, format_args!("{:?}", args)).ok()?;
    Some(out.0)
}

impl CrowLiveState {
    /// A view's arguments as its key (`key_within`), as the compiled
    /// program asks for them.
    pub fn view_args<T: std::fmt::Debug>(&self, args: &T) -> Option<String> {
        key_within(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{served_render, DeriveSpec, Offers};
    use crate::view::tree::{element, Given, TableState};
    use crate::view::CrowVNode;

    static HALF: DeriveSpec = DeriveSpec { name: "half", facts: &["a"], derives: &[], tables: &[], now: false, external: false, fresh: 0.0, shared: false, keys: &[] };
    static ROOT: ViewSpec = ViewSpec { name: "root", facts: &["a", "b", "names", "flag"], always: Some("it shows the render's count"), bound: &[] };
    static SHOWS_A: ViewSpec = ViewSpec { name: "shows_a", facts: &["a"], always: None, bound: &[] };
    static SHOWS_HALF: ViewSpec = ViewSpec { name: "shows_half", facts: &[], always: None, bound: &[] };
    static ITEM: ViewSpec = ViewSpec { name: "item", facts: &["b"], always: None, bound: &[] };
    static PAGED: ViewSpec = ViewSpec { name: "paged", facts: &[], always: None, bound: &[] };
    /// (its facts are its own and every view it calls, as derive::view_reads
    /// finds them)
    static LIST: ViewSpec = ViewSpec { name: "list", facts: &["names", "b"], always: None, bound: &[] };

    fn int(state: &CrowLiveState, k: &str) -> i64 {
        state.get(k).and_then(|v| v.as_i64()).unwrap_or(0)
    }

    fn half(state: &CrowLiveState) -> i64 {
        state.derive(&HALF, |s| int(s, "a") / 2, |x, y| x == y)
    }

    fn refresh(state: &CrowLiveState) -> impl Fn(&str) + '_ {
        move |n: &str| {
            if n == "half" {
                let _ = half(state);
            }
        }
    }

    fn el(kind: &str, opts: Vec<(&str, Given)>, kids: Vec<Given>) -> Node {
        element(kind, opts.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), kids, TableState::default()).unwrap()
    }

    /// A view as the compiled program runs one: kept, or run and kept.
    fn view(state: &CrowLiveState, spec: &'static ViewSpec, args: Option<String>, body: impl FnOnce() -> Node) -> Node {
        let r = refresh(state);
        match state.view_begin(spec, args, state.tables_drawn(), &r) {
            ViewCall::Reuse(n, d) => {
                state.set_tables_drawn(d);
                n
            }
            ViewCall::Run(running) => {
                let n = body();
                state.view_end(running, n, state.tables_drawn())
            }
        }
    }

    fn shows_a(state: &CrowLiveState) -> Node {
        view(state, &SHOWS_A, Some(String::new()), || el("p", vec![], vec![Given::Text(format!("a {}", int(state, "a")))]))
    }

    fn shows_half(state: &CrowLiveState) -> Node {
        view(state, &SHOWS_HALF, Some(String::new()), || {
            let h = half(state);
            el("p", vec![], vec![Given::Text(format!("half {}", h))])
        })
    }

    fn item(state: &CrowLiveState, name: &str) -> Node {
        view(state, &ITEM, Some(key_of(&(name,))), || el("row", vec![("key", Given::Text(name.to_string()))], vec![Given::Text(format!("{} {}", name, int(state, "b")))]))
    }

    /// A view drawing a paged table (its number counted by the render).
    fn paged(state: &CrowLiveState) -> Node {
        view(state, &PAGED, Some(String::new()), || {
            let p = state.table_page(10, 40);
            el("p", vec![], vec![Given::Text(format!("table {} page {}", p.index, p.at))])
        })
    }

    fn list(state: &CrowLiveState) -> Node {
        view(state, &LIST, Some(String::new()), || {
            let names: Vec<String> = state.get("names").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            let kids = names.iter().map(|n| Given::from(item(state, n))).collect();
            el("stack", vec![], kids)
        })
    }

    fn root(state: &CrowLiveState) -> Node {
        view(state, &ROOT, None, || {
            let flag = state.get("flag").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut kids = vec![Given::from(paged(state)), Given::from(shows_a(state)), Given::from(shows_half(state)), Given::from(list(state))];
            if flag {
                kids.insert(0, Given::from(paged(state)));
            }
            kids.push(Given::from(paged(state)));
            el("stack", vec![], kids)
        })
    }

    fn page(state: &CrowLiveState) -> Node {
        root(state)
    }

    fn fresh() -> CrowLiveState {
        let mut s = CrowLiveState::new();
        s.set("a", serde_json::json!(1));
        s.set("b", serde_json::json!(1));
        s.set("names", serde_json::json!(["x", "y", "z"]));
        s.set("flag", serde_json::json!(false));
        s
    }

    #[test]
    fn a_view_whose_reads_did_not_move_is_kept() {
        let mut s = fresh();
        let _ = s.render_with(&page);
        assert_eq!(s.views_counted(), (9, 0), "the first render runs every view");
        let _ = s.render_with(&page);
        // the root runs at every render; what it calls is kept
        assert_eq!(s.views_counted(), (1, 5));
        // a fact only `item` reads: the list runs only through its items
        s.set("b", serde_json::json!(2));
        let _ = s.render_with(&page);
        assert_eq!(s.views_counted(), (5, 4), "root, list and the three items run");
        // a fact a derivation reads, the derivation's value the same (0 / 2
        // and 1 / 2 are both 0)
        s.set("a", serde_json::json!(0));
        let _ = s.render_with(&page);
        assert_eq!(s.views_counted(), (2, 4), "shows_a runs; shows_half is kept by the derivation's cutoff");
        let html = tree::html(&s.render_with(&page));
        assert!(html.contains("half 0") && html.contains("a 0"), "{}", html);
    }

    #[test]
    fn a_long_argument_is_no_key() {
        assert_eq!(key_within(&(&1i64, &"a")).as_deref(), Some(key_of(&(&1i64, &"a")).as_str()));
        let rows: Vec<i64> = (0..10_000).collect();
        assert_eq!(key_within(&(&rows,)), None);
        let fits: Vec<i64> = (0..100).collect();
        assert_eq!(key_within(&(&fits,)), Some(key_of(&(&fits,))));
    }

    #[test]
    fn a_kept_view_keeps_the_numbers_of_the_tables_it_drew() {
        let mut s = fresh();
        let _ = s.render_with(&page);
        // a table the root draws before the others moves their numbers: the
        // views that drew them run again, numbered as a full render numbers
        s.set("flag", serde_json::json!(true));
        let html = tree::html(&s.render_with(&page));
        let full = tree::html(&s.clone().render_with(&page));
        assert_eq!(html, full);
        assert!(html.contains("table 0 page 0") && html.contains("table 2 page 0"), "{}", html);
        // a page turned: the view that drew that table runs, the rest are kept
        s.turn_page(1, 3);
        let html = tree::html(&s.render_with(&page));
        assert!(html.contains("table 1 page 3"), "{}", html);
        assert_eq!(html, tree::html(&s.clone().render_with(&page)));
    }

    static TABBED: ViewSpec = ViewSpec { name: "tabbed", facts: &["tab", "a", "b", "names"], always: None, bound: &[] };

    /// A page of two tabs: the one `tab` names shows `shows_a` and the list,
    /// the other the half and an item.
    fn tabbed(state: &CrowLiveState) -> Node {
        view(state, &TABBED, Some(String::new()), || {
            let kids = if int(state, "tab") == 0 {
                vec![Given::from(shows_a(state)), Given::from(list(state))]
            } else {
                vec![Given::from(shows_half(state)), Given::from(item(state, "solo"))]
            };
            el("stack", vec![], kids)
        })
    }

    /// A view a render did not call is kept for a later one: switching back
    /// to a tab whose facts did not move reuses its views; one whose facts
    /// moved meanwhile runs, as a live one would.
    #[test]
    fn a_tab_switched_back_to_keeps_its_views() {
        let mut s = fresh();
        s.set("tab", serde_json::json!(0));
        let _ = s.render_with(&tabbed);
        assert_eq!(s.views_counted(), (6, 0), "tabbed, shows_a, list and its three items");
        s.set("tab", serde_json::json!(1));
        let _ = s.render_with(&tabbed);
        assert_eq!(s.views_counted(), (3, 0));
        s.set("tab", serde_json::json!(0));
        let html = tree::html(&s.render_with(&tabbed));
        assert_eq!(s.views_counted(), (1, 2), "the first tab's views are where it left them");
        assert_eq!(html, tree::html(&s.clone().render_with(&tabbed)));
        // a fact the first tab's items read moves while the other tab shows
        s.set("tab", serde_json::json!(1));
        let _ = s.render_with(&tabbed);
        assert_eq!(s.views_counted(), (1, 2));
        s.set("b", serde_json::json!(5));
        let _ = s.render_with(&tabbed);
        s.set("tab", serde_json::json!(0));
        let html = tree::html(&s.render_with(&tabbed));
        assert_eq!(s.views_counted(), (5, 1), "list and its items run again; shows_a is kept");
        assert!(html.contains("x 5"), "{}", html);
        assert_eq!(html, tree::html(&s.clone().render_with(&tabbed)));
    }

    static PICKS: ViewSpec = ViewSpec { name: "picks", facts: &["names", "b"], always: Some("it is the page"), bound: &[] };
    static PICK: ViewSpec = ViewSpec { name: "pick", facts: &["b"], always: None, bound: &[] };

    /// A button per name, its click holding the name and `b`.
    fn pick(state: &CrowLiveState, name: &str) -> Node {
        view(state, &PICK, Some(key_of(&(name,))), || {
            let press = el("button", vec![("click", Given::Call("pick".into(), vec![serde_json::json!(name), serde_json::json!(int(state, "b"))]))], vec![Given::Text(name.to_string())]);
            el("row", vec![], vec![Given::from(press)])
        })
    }

    fn picks(state: &CrowLiveState) -> Node {
        view(state, &PICKS, None, || {
            let names: Vec<String> = state.get("names").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            el("stack", vec![], names.iter().map(|n| Given::from(pick(state, n))).collect())
        })
    }

    /// What a kept view's subtree offers is read once while it is kept, and
    /// the page offers what a full reading of it would: a view run again
    /// offers its new clicks, not its old ones.
    #[test]
    fn a_kept_views_offers_are_read_once() {
        let mut s = fresh();
        let _ = served_render(&s, &picks);
        let (_, _, o) = served_render(&s, &picks);
        assert_eq!(s.views_counted(), (1, 3));
        assert_eq!(o.kept.len(), 3, "each kept button's offers come from the memo");
        assert!(o == Offers::of_tree(&s.clone().render_with(&picks)));
        assert!(o.allows("pick", Some("[\"y\",1]")) && !o.allows("pick", Some("[\"y\",2]")));
        s.set("b", serde_json::json!(2));
        let (_, _, o) = served_render(&s, &picks);
        assert!(o.allows("pick", Some("[\"y\",2]")) && !o.allows("pick", Some("[\"y\",1]")), "{:?}", o);
        assert!(o == Offers::of_tree(&s.clone().render_with(&picks)));
        s.set("names", serde_json::json!(["x"]));
        let (_, _, o) = served_render(&s, &picks);
        assert!(o.allows("pick", Some("[\"x\",2]")) && !o.allows("pick", Some("[\"y\",2]")));
        assert!(o == Offers::of_tree(&s.clone().render_with(&picks)));
        // (the buttons `b = 1` drew are let go; the views of y and z are
        // kept unused, and so are their buttons' offers)
        assert_eq!(s.offers_memo.lock().unwrap().made.len(), 3);
    }

    /// The views a render did not call are kept within the budget, the
    /// least recently used let go first.
    #[test]
    fn unused_views_are_kept_within_a_budget() {
        let mut s = fresh();
        // each render shows one name the last did not: the items before it
        // are left unused, one more each time
        for i in 0..(KEEP_UNUSED + 40) {
            s.set("names", serde_json::json!([format!("n{}", i)]));
            let _ = s.render_with(&page);
        }
        let kept = s.views_kept_now();
        // the six live ones (two paged tables, shows_a, shows_half, list,
        // the item; root is not kept: it reads the render's count) and the
        // budget's unused ones
        assert_eq!(kept, KEEP_UNUSED + 6);
        // the newest unused ones are the ones kept
        s.set("names", serde_json::json!([format!("n{}", KEEP_UNUSED + 38)]));
        let _ = s.render_with(&page);
        assert_eq!(s.views_counted(), (2, 5), "root and list run; the item two renders back is kept");
        s.set("names", serde_json::json!(["n0"]));
        let _ = s.render_with(&page);
        assert_eq!(s.views_counted(), (3, 4), "the oldest was let go: it runs");
    }

    /// An unused view's weight counts toward the budget: a tree past it is
    /// let go once it is left unused.
    #[test]
    fn a_tree_past_the_weight_is_let_go_unused() {
        let mut s = fresh();
        s.set("names", serde_json::json!((0..(KEEP_WEIGHT + 10)).map(|i| format!("r{}", i)).collect::<Vec<_>>()));
        let _ = s.render_with(&page);
        let big = s.views_kept_now();
        s.set("names", serde_json::json!(["x"]));
        let _ = s.render_with(&page);
        // the old list and its items are left unused: past the weight, the
        // list (the heaviest is not chosen: the oldest are) and items go
        // until the rest fit
        assert!(s.views_kept_now() < big);
        let c = s.view_cache();
        let unused: usize = c.entries.values().filter(|e| e.used != c.render).map(|e| e.weight.unwrap_or(0)).sum();
        assert!(unused <= KEEP_WEIGHT, "{}", unused);
    }

    /// Random events against the page: every render with views kept is the
    /// page a render of every view makes — its markup, what it offers, and
    /// the changes from the last page, which make the new one from the old.
    #[test]
    fn kept_views_make_the_page_a_full_render_makes() {
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let mut s = fresh();
        let (mut last, _, _) = served_render(&s, &page);
        let pool = ["x", "y", "z", "w", "v"];
        for _ in 0..400 {
            match next(6) {
                0 => s.set("a", serde_json::json!(next(7) as i64)),
                1 => s.set("b", serde_json::json!(next(3) as i64)),
                2 => {
                    let mut names: Vec<&str> = pool.iter().copied().filter(|_| next(3) != 0).collect();
                    // shuffled: keyed rows move
                    for i in (1..names.len()).rev() {
                        let j = next(i as u64 + 1) as usize;
                        names.swap(i, j);
                    }
                    s.set("names", serde_json::json!(names));
                }
                3 => s.set("flag", serde_json::json!(next(2) == 0)),
                4 => s.turn_page(next(3) as usize, next(4) as usize),
                _ => {}
            }
            let (built, _, offers) = served_render(&s, &page);
            let reference = s.clone();
            let full = tree::vnode(&reference.render_with(&page)).as_built();
            assert_eq!(built.to_html(), full.to_html());
            assert!(offers == Offers::of_tree(&reference.render_with(&page)));
            let mut applied = last.clone();
            applied.apply(&CrowVNode::changes(&last, &built)).unwrap();
            assert_eq!(applied.to_html(), full.to_html());
            last = built;
        }
    }

    // ── values turns (one-view-design.md 3.5) ──────────────────────

    static B_PAGE: ViewSpec = ViewSpec { name: "b_page", facts: &["a", "b", "names"], always: None, bound: &["a", "half"] };
    static B_CLOCK: ViewSpec = ViewSpec { name: "b_clock", facts: &["a"], always: None, bound: &["a"] };
    static B_HALF: ViewSpec = ViewSpec { name: "b_half", facts: &[], always: None, bound: &["half"] };

    /// The page's two bindings: `a` shown as text (0), `half` as the
    /// `gap` of nothing — an option (1) — and as text (2).
    struct TestValues;

    impl Values for TestValues {
        fn inputs(&self, id: u32) -> &'static [&'static str] {
            match id {
                0 => &["a"],
                _ => &["half"],
            }
        }
        fn refresh(&self, state: &CrowLiveState, name: &str) {
            if name == "half" {
                let _ = half(state);
            }
        }
        fn eval(&self, state: &CrowLiveState, ids: &[u32]) -> Option<Vec<tree::Bound>> {
            Some(
                ids.iter()
                    .map(|id| match id {
                        0 => tree::Bound::Text(format!("a {}", int(state, "a"))),
                        1 => tree::Bound::Opt(Some(tree::Opt::Text(if half(state) % 2 == 0 { "s".into() } else { "l".into() }))),
                        _ => tree::Bound::Text(format!("half {}", half(state))),
                    })
                    .collect(),
            )
        }
    }

    fn bound_el(kind: &str, opts: Vec<(&str, Given)>, kids: Vec<Given>, binds: &[(tree::BindAt<'_>, u32)]) -> Node {
        tree::element_bound(kind, opts.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), kids, TableState::default(), binds).unwrap()
    }

    fn b_clock(state: &CrowLiveState) -> Node {
        view(state, &B_CLOCK, Some(String::new()), || bound_el("p", vec![], vec![Given::Text(format!("a {}", int(state, "a")))], &[(tree::BindAt::Child(0), 0)]))
    }

    fn b_half(state: &CrowLiveState) -> Node {
        view(state, &B_HALF, Some(String::new()), || {
            let h = half(state);
            let gap = if h % 2 == 0 { "s" } else { "l" };
            bound_el("row", vec![("gap", Given::Text(gap.into()))], vec![Given::Text(format!("half {}", h))], &[(tree::BindAt::Child(0), 2), (tree::BindAt::Option("gap"), 1)])
        })
    }

    fn b_page(state: &CrowLiveState) -> Node {
        view(state, &B_PAGE, Some(String::new()), || el("stack", vec![], vec![Given::from(b_clock(state)), Given::from(b_half(state)), Given::from(list(state))]))
    }

    /// A turn as the live loop takes one: a values turn where it can be,
    /// else a render; whether it was a values turn.
    fn turn(s: &CrowLiveState) -> (CrowVNode, Offers, bool) {
        match super::super::values_render(s, &b_page, &TestValues) {
            Some((v, _, o, _, _)) => (v, o, true),
            None => {
                let (v, _, o) = served_render(s, &b_page);
                (v, o, false)
            }
        }
    }

    #[test]
    fn a_turn_that_moved_only_bound_reads_runs_no_view() {
        let mut s = fresh();
        let (first, _, _) = served_render(&s, &b_page);
        assert!(first.to_html().contains("a 1") && first.to_html().contains("half 0"), "{}", first.to_html());
        // `a` is read only in bindings here: the clock's text, half's text
        // and gap move, and no view runs
        s.set("a", serde_json::json!(3));
        let turned = super::super::values_render(&s, &b_page, &TestValues).expect("a values turn");
        assert_eq!(turned.3, 3, "three places set");
        assert_eq!(s.views_counted(), (0, 0));
        let full = tree::vnode(&s.clone().render_with(&b_page)).as_built();
        assert_eq!(turned.0.to_html(), full.to_html());
        let changes = CrowVNode::changes(&first, &turned.0);
        assert!(changes.iter().all(|c| c["op"] == "text" || c["op"] == "attrs"), "{:?}", changes);
        // a value computed again to the same sets nothing (`a` 3 to 3)
        s.set("a", serde_json::json!(3));
        // the views were kept with the values set: a render that follows a
        // structural move runs only what read it
        s.set("b", serde_json::json!(5));
        assert!(super::super::values_render(&s, &b_page, &TestValues).is_none(), "b is read by the list's structure");
        let (after, _, _) = served_render(&s, &b_page);
        assert_eq!(after.to_html(), tree::vnode(&s.clone().render_with(&b_page)).as_built().to_html());
        let (ran, kept) = s.views_counted();
        assert_eq!(kept, 2, "the clock and half were kept with their values set ({} ran)", ran);
    }

    /// Random moves of facts read in bindings and of facts read by the
    /// structure: every turn's page, values turn or render, is the page a
    /// render of a fresh copy makes, and the changes sent take the last
    /// page to it.
    #[test]
    fn values_turns_make_the_page_a_full_render_makes() {
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let mut s = fresh();
        let (mut last, _, _) = served_render(&s, &b_page);
        let pool = ["x", "y", "z", "w"];
        let mut valued = 0;
        for _ in 0..400 {
            match next(5) {
                0 | 1 => s.set("a", serde_json::json!(next(9) as i64)),
                2 => s.set("b", serde_json::json!(next(3) as i64)),
                3 => {
                    let names: Vec<&str> = pool.iter().copied().filter(|_| next(3) != 0).collect();
                    s.set("names", serde_json::json!(names));
                }
                _ => {}
            }
            let (built, offers, was) = turn(&s);
            valued += was as usize;
            let reference = s.clone();
            let full = tree::vnode(&reference.render_with(&b_page)).as_built();
            assert_eq!(built.to_html(), full.to_html());
            assert!(offers == Offers::of_tree(&reference.render_with(&b_page)));
            let mut applied = last.clone();
            applied.apply(&CrowVNode::changes(&last, &built)).unwrap();
            assert_eq!(applied.to_html(), full.to_html());
            last = built;
        }
        assert!(valued > 100, "only {} values turns", valued);
    }

    static PARITY: DeriveSpec = DeriveSpec { name: "parity", facts: &[], derives: &["half"], tables: &[], now: false, external: false, fresh: 0.0, shared: false, keys: &[] };
    static SHOWS_PARITY: ViewSpec = ViewSpec { name: "shows_parity", facts: &[], always: None, bound: &[] };

    /// `parity` as the compiled program reads it: its input brought up to
    /// date as its own reading, then its body.
    fn parity(state: &CrowLiveState) -> i64 {
        state.deriving(|| drop(half(state)));
        state.derive(&PARITY, |s| half(s) % 2, |x, y| x == y)
    }

    fn shows_parity(state: &CrowLiveState) -> Node {
        let r = |n: &str| {
            if n == "parity" {
                drop(parity(state));
            }
        };
        match state.view_begin(&SHOWS_PARITY, Some(String::new()), state.tables_drawn(), &r) {
            ViewCall::Reuse(n, d) => {
                state.set_tables_drawn(d);
                n
            }
            ViewCall::Run(running) => {
                let n = el("p", vec![], vec![Given::Text(format!("parity {}", parity(state)))]);
                state.view_end(running, n, state.tables_drawn())
            }
        }
    }

    /// A view that reads a derivation brought up to date inside its run
    /// reads that derivation alone: what the derivation reads (`half`) is
    /// its own, so `half` moving with `parity` the same keeps the view
    /// (one-view-design.md §7, three faults a real app found).
    #[test]
    fn a_derivation_read_in_a_view_brings_none_of_its_reads_into_it() {
        let mut s = fresh();
        let _ = s.render_with(&shows_parity);
        let read: Vec<Vec<&str>> = s.view_cache().entries.values().map(|e| e.derives.iter().map(|d| d.name).collect()).collect();
        assert_eq!(read, vec![vec!["parity"]]);
        // half 0 -> 2 (a 1 -> 4), parity 0 -> 0: the view is kept
        s.set("a", serde_json::json!(4));
        let _ = s.render_with(&shows_parity);
        assert_eq!(s.views_counted(), (0, 1));
        // half 2 -> 3, parity 0 -> 1: it runs
        s.set("a", serde_json::json!(6));
        let html = tree::html(&s.render_with(&shows_parity));
        assert_eq!(s.views_counted(), (1, 0));
        assert!(html.contains("parity 1"), "{}", html);
    }

    static TENS: DeriveSpec = DeriveSpec { name: "tens", facts: &[], derives: &[], tables: &[], now: true, external: false, fresh: 0.0, shared: false, keys: &[] };
    static IDLE_ROOT: ViewSpec = ViewSpec { name: "idle_root", facts: &["a"], always: None, bound: &[] };
    static SHOWS_TENS: ViewSpec = ViewSpec { name: "shows_tens", facts: &[], always: None, bound: &[] };

    thread_local! {
        static REFRESHED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn tens(state: &CrowLiveState) -> i64 {
        state.derive(&TENS, |_| (crow_runtime::moments::now() / 10.0).floor() as i64, |x, y| x == y)
    }

    /// `view`, each derivation the cache asks brought up to date counted.
    fn idle_view(state: &CrowLiveState, spec: &'static ViewSpec, body: impl FnOnce() -> Node) -> Node {
        let r = |n: &str| {
            REFRESHED.with(|c| c.borrow_mut().push(n.to_string()));
            match n {
                "half" => drop(half(state)),
                "tens" => drop(tens(state)),
                _ => {}
            }
        };
        match state.view_begin(spec, Some(String::new()), state.tables_drawn(), &r) {
            ViewCall::Reuse(n, d) => {
                state.set_tables_drawn(d);
                n
            }
            ViewCall::Run(running) => {
                let n = body();
                state.view_end(running, n, state.tables_drawn())
            }
        }
    }

    fn idle_page(state: &CrowLiveState) -> Node {
        idle_view(state, &IDLE_ROOT, || {
            let a = idle_view(state, &SHOWS_HALF, || el("p", vec![], vec![Given::Text(format!("half {}", half(state)))]));
            let t = idle_view(state, &SHOWS_TENS, || el("p", vec![], vec![Given::Text(format!("tens {}", tens(state)))]));
            el("stack", vec![], vec![Given::Text(format!("a {}", int(state, "a"))), Given::from(a), Given::from(t)])
        })
    }

    /// A render in which nothing moved (an idle tick) keeps the page with
    /// no derivation brought up to date but the ones that answer by the
    /// moment (`settle`); one of those that moved runs the views that read
    /// it, and a fact that moved brings the rest up to date again.
    #[test]
    fn an_idle_tick_brings_up_to_date_only_what_answers_by_the_moment() {
        let mut s = fresh();
        let at = |m: f64, s: &CrowLiveState| crow_runtime::moments::at(m, || tree::html(&s.render_with(&idle_page)));
        let first = at(1000.0, &s);
        assert!(first.contains("tens 100") && first.contains("half 0"), "{}", first);
        // (the first render computed both derivations, which moved them:
        // the next render checks every kept view once)
        s.next_tick();
        assert_eq!(at(1000.0, &s), first);
        for i in 0..5 {
            s.next_tick();
            REFRESHED.with(|c| c.borrow_mut().clear());
            let html = at(1000.0 + i as f64, &s);
            assert_eq!(html, first);
            assert_eq!(s.views_counted(), (0, 1), "nothing moved: the page is kept");
            assert_eq!(REFRESHED.with(|c| c.borrow().clone()), vec!["tens".to_string()], "only the timed derivation is brought up to date");
        }
        // the moment moves what the timed derivation answers
        s.next_tick();
        let html = at(1010.0, &s);
        assert!(html.contains("tens 101"), "{}", html);
        assert_eq!(s.views_counted(), (2, 1), "the root and the view reading tens run; the view reading half is kept");
        let reference = s.clone();
        assert_eq!(html, crow_runtime::moments::at(1010.0, || tree::html(&reference.render_with(&idle_page))));
        // a fact that moved: every kept view's derivations are checked again
        s.next_tick();
        s.set("a", serde_json::json!(5));
        REFRESHED.with(|c| c.borrow_mut().clear());
        let html = at(1010.0, &s);
        assert!(html.contains("half 2") && html.contains("a 5"), "{}", html);
        assert!(REFRESHED.with(|c| c.borrow().iter().any(|n| n == "half")), "half is brought up to date");
    }
}
