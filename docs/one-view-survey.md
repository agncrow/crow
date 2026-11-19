# One view: a map of the code as it was on 2026-11-15

A read-only survey made before the work of `one-view-design.md` began.
Line numbers are those of that day's working tree and drift as the work goes on.

```text
CROW `use ui` RENDERING MAP (read-only survey; files as they are on disk now, including uncommitted changes)

All paths are under /Users/vijay/code/rust_projects/crow unless I give an absolute path. I ran no builds or tests and wrote no files.

Design doc: docs/one-view-design.md (216 lines, untracked). Section 2 says: a view lowers to an HTML-shaped CrowVNode; every event or tick re-runs the whole page through served_render plus changes; derives have static read sets with early cutoff, and views do not use that; Lory gui.co is semantic; a widget is the one door.

==================================================
1. HOW A VIEW BECOMES A TREE
==================================================
Vocabulary
- Elements: src/lower.rs:27-45 UI_ELEMENTS = div span p h1-h6 button ul ol li form a nav header footer section tr td th thead tbody option textarea label pre code input img table stack row grid overlay scroll details summary dropdown modal drawer toast divider widget title meta.
- Controls: src/lower.rs:48 UI_CONTROLS = field select slider checkbox toggle radio tabs progress icon.
- chart is a 12-arg runtime call (src/table.rs:3068, core.chart).
- There is no `html` view element. `html` is the web.html response (src/table.rs:3093) and the harness verb html[a] (src/ir_interp.rs:3863-3868).
- Raw markup inside views comes from Markup values: doc.markdown, menu and pager (src/table.rs:2981-2984).
- The HTML `select` element is not in UI_ELEMENTS. Only the `select` control exists.

Lowering (shared by both modes)
- src/lower.rs:1128-1133 and lower_ui_call src/lower.rs:1512-1528: named args become a leading Map.
- src/lower_ir.rs:6811-6874 builds IR `Expr::Tree{element, attrs, children}` (src/ir.rs:884; Ty::Node at src/ir.rs:944).
- Event attrs click/input/submit/change/seek become Expr::Deferred (src/lower_ir.rs:6833-6838).
- Controls stay as Expr::Call with Target::Runtime.
- This IR Tree is the last place where element kind and options exist as structure in both modes.

Shared kernel table: crow_runtime/src/layout.rs
- CONTAINERS :15 (stack row grid overlay scroll modal drawer toast all become div).
- ALIASED_ELEMENTS :19 (divider becomes hr, dropdown becomes details). VOID :24. html_tag :27-30. BOOL_ATTRS :35. LAYOUT_ATTRS :39-45.
- render :155-288 takes the element name plus literal attrs and returns tag, class, style, data and passthrough attrs:
  - container or alias class = its own name :163.
  - modal gets role=dialog aria-modal; drawer gets role=dialog; toast gets role=status (:166-173).
  - base CSS for stack/row/grid/overlay/scroll :174-185. Attrs are sorted :190-191.
  - class :200, gap :201, pad :202-211, align :213, justify :214, width :225.
  - tip becomes title :233. shortcut becomes aria-keyshortcuts :236-239. font :241.
  - open :244-250 (data-open), active :251 (data-active), side :252, surface/card :253.
  - color :257, background :258, size :259, weight :260.
  - BOOL_ATTRS (disabled etc.) :266-270. Everything else passes through :271. Grid columns :274-276.
- confirm_wiring :309-317 rewrites a click to data-crow-click="__ask" with args [handler, args, text].
- NATIVE_PRESS :321; press_attrs :327-336.
- Paging :347, Sorting :357, TABLE_BUDGET :364.
- frame_table_html :367-517 returns a string:
  - click/key :443-463, mark :456-458, delta :469-476.
  - actions/confirm :480-494.
  - sort header buttons (__sort) :422-438; pager (__page) :500-515.
- nav_active_html :522-542. ICONS :546. icon_html :608-622 (string). look_attrs :626. DEFAULT_CSS :648.

Node type and builders: crow_web/src/view/mod.rs (2200 lines)
- CrowVNode :61-73 = Text | Raw | Element{tag, attrs HashMap, children}.
- attr :103, with_children :113.
- with_class :123-134.
- with_layout :142-159 calls layout::render at run time.
- confirm :163-173. on_click :178 with pressable :184-193. on_click_args :198-202. on_input :205. on_submit :210.
- Tag constructors :216-256. open_attr :261-268. mark_active :272-284.
- Control builders:
  - icon :287-292.
  - progress :295-300 (class crow-progress).
  - check_input :303-310 (crow-check / crow-toggle).
  - choice_input :314-331 (crow-radio; crow-tabs with role=tablist, role=tab, aria-selected).
  - field_typed :335-362 (div.crow-field with label plus input/textarea; file becomes data-crow-upload).
  - widget :367-377 (div.crow-widget with data-crow-widget/props/on).
- to_html :380-413 sorts attrs. raw :420.
- chart_from_frame :426-506 calls crow_runtime::chart::render_svg and returns Raw. The svg has class crow-chart; seek rects carry data-crow-click (crow_runtime/src/chart.rs:517, :700).
- table_from_data :508, field :533, slider :545 (crow-field crow-slider).
- select_input :566-581 (div.crow-select; the select gets data-crow-change="__select:key").
- parse_markup :1000, read_markup :1007-1152.
- as_built :1192-1204 does to_html then read_markup. If the markup needed repair or has more than one top node, the whole thing becomes Raw.
- changes :1212-1297, apply :1303-1359, column_children :1392, markup_children :1418, bool_attr :1425.
- Head :1438-1466, split_head :1472, strip_head :1481-1495. escape_text :1544, escape_attr :1549.

Interpreter: two implementations, both string-producing
- src/ir_interp.rs:1254-1282 evaluates Expr::Tree into an attrs map. It injects __page_index/__page_at (:1259-1266) and __sort_index/__sort_by/__sort_desc (:1268-1275) from session_pages/session_sorts, then returns `CrowValue::markup(crate::ops::render_element(element, &vals)?)`.
- Bound inputs: src/ir_interp.rs:1306-1326 (when in_view>0, key = the arg[1] variable name) call ops::render_input.
- Each control has its own path:
  - progress src/ir_interp.rs:2855-2871 is a hand-written string that duplicates CrowVNode::progress.
  - chart :2875-2939 calls CrowVNode::chart_from_frame(...).to_html().
  - icon :2942-2950 calls layout::icon_html directly.
- src/ops.rs render_widget :1213-1245 calls CrowVNode::widget(...).to_html(). If props fail, it renders failed_in_view text.
- src/ops.rs render_element :1247-1444 is a string builder:
  - table options click/key/mark/delta/actions/hide/confirm/page/sort :1297-1360.
  - confirm :1361-1364.
  - events click/input/submit/change, including frozen args [Fn, args] written to data-crow-args :1365-1382. seek is skipped :1383.
  - scalar attrs :1386-1393, markup children :1396.
  - a frame child goes to frame_table_html :1397-1408.
  - nav active :1413-1417.
  - layout::render :1418-1421, confirm_wiring :1430, press_attrs :1432-1435.
  - sort and escape :1436-1437, void :1440-1443.
- src/ops.rs render_input :1449-1493 calls CrowVNode builders, then .to_html().

Compiled: src/emit_rs/ui.rs (emits Rust that builds CrowVNode)
- VOID_ELEMENTS :8.
- tree :20-116:
  - widget :21-23, frame table :26-28.
  - the Rust constructor is chosen through html_tag :29.
  - children go to with_children :39-44.
  - Literal layout attrs are run through layout::render at compile time (:94-109), so class/style strings are baked into the generated code.
  - Computed layout attrs become `.with_layout(element, pairs)` (:80-93).
  - nav mark_active :110-114.
- tree_rest :151-220:
  - confirm :154-157 and :216-218; click with args uses on_click_args :159-167.
  - open_attr :176-179, nav active :180-183, tip :184-187, BOOL_ATTRS via bool_attr :188-191.
  - computed class uses with_class :193-196.
  - events click/input/submit :197-199. There is no "change" arm here, but ops.rs handles change.
  - generic attr :200-213.
- child :223-289: views and controls become nodes :238-248; Markup becomes CrowVNode::raw (:272, :278); lists :258-263. child_items :295-303.
- widget :306-338.
- frame_table :341-407:
  - key :362, actions :374-379, confirm :380-388.
  - page via state.table_page :389-392, sort via state.table_sort :393-395.
  - The result is `CrowVNode::table(vec![CrowVNode::raw(frame_table_html(...))])` (:399-402).
- ui_builtin :410-451, chart :461-493.

What is shared and what exists twice
- Shared: layout::render, confirm_wiring, press_attrs, frame_table_html, nav_active_html, icon_html, and the CrowVNode control/widget/chart builders.
- Two implementations: the generic element and event wiring (ops.rs:1247-1444 vs ui.rs:20-220) and progress (src/ir_interp.rs:2855-2871 vs crow_web/src/view/mod.rs:295-300).

Where each option is applied (interpreter / compiled)
- gap, align, class, color, size, weight, tip:
  - interpreter: layout.rs:200-260 via ops.rs:1418.
  - compiled: literal values at compile time (ui.rs:94-109); computed values via with_layout or with_class (ui.rs:80-93, :193-196); tip at ui.rs:184-187.
- disabled: BOOL_ATTRS layout.rs:266-270; compiled bool_attr ui.rs:188-191.
- click: ops.rs:1365-1382 / ui.rs:159-167, :197.
- confirm: ops.rs:1361-1364 and :1430 / ui.rs:154-157, :216-218. Both call layout::confirm_wiring.
- key: table key column only (ops.rs:1303 / ui.rs:362). There is no general element key; a `key=` attr on an element passes through as a plain attribute (layout.rs:271).
- sort, page: table only.
  - interpreter: state is injected at src/ir_interp.rs:1259-1275 and consumed at ops.rs:1297-1360.
  - compiled: state.table_page / table_sort (ui.rs:389-395).
  - rendering: layout.rs:422-438 (sort) and :500-515 (pager).
- actions: layout.rs:480-494 / ui.rs:374-379.

Where the semantic kind is lost
- Interpreter: at run time, in ops::render_element for each element, which returns a string.
- Compiled: at compile time (ui.rs:29 tag choice, ui.rs:94 baked layout).
- What remains afterwards:
  - class names: stack/row/grid/overlay/scroll/modal/drawer/toast/divider/dropdown, card, crow-field/slider/select/check/toggle/radio/tabs/progress/widget, icon icon-X, crow-chart.
  - roles.
  - data-open/side/active/anchor/below/becomes/hidden-below/columns-then.
  - data-crow-* wiring.
- The harness depends on these traces:
  - src/harness.rs:268-272 maps a container selector to a class.
  - harness_to_lnode src/harness.rs:209-258 rebuilds layout attrs from inline style.

==================================================
2. PAGE RENDER DRIVING (crow_web/src/live/mod.rs, 6702 lines)
==================================================
- served_render :3313-3316 = state.render_with(render).split_head(), then body.as_built().
- render_with :1949-1970:
  - moment, begin_render :1928-1939, render, end_render :1943-1946.
  - adds a toast if a notice is unread (:1958-1963, with_toast :1252).
  - adds a dialog while asking (:1965-1968, with_dialog :1261-1273). append_child :1236-1248.
- Who calls render:
  - GET page :3388-3429: start_session, read address, fetch_sources, next_tick :3414, spawn_blocking served_render, to_html, park, then page_shell :3143-3170 plus page_js.
  - WS open: kept session :3497-3509; fresh session :3517-3535 (next_tick :3526); dropped sign-in :3539-3551.
  - Loop render :3950-3977. A tick runs next_tick (:3958-3960). mark_running :3972-3974.
- Loop :3669-4052. LiveAction :36-52 = Message, PollTick :3743, LaterDelta :3754, RefreshTick :3765 (sets tick), Wake :3770, HandlerDone :3776.
- Event names:
  - __resync :3731.
  - offers check :3804.
  - field updates use apply_field :3812 (fn :1349-1363).
  - __ask :3820, __cancel :3828, __upload :3836, __page :3856, __sort :3865, __confirm :3873.
  - Any other event runs run_handler :1424-1442 on a working copy with HANDLER_GRACE 100ms (:3891).
- How changes is used:
  - :3993 `CrowVNode::changes(&last_vnode, &new_vnode)`, only when both are Element (:3994).
  - Patches are sent only if `patch.to_string().len() * 4 < html.len() * 3` (:3998); otherwise full html is sent.
  - Offers are recomputed (offers_before/offers :4007).
- Offers (server-side event allow-list) :990-1159: collect :1014-1066, collect_markup :1071-1122 (scans the built markup), note_widget :1125-1131, allows :1135-1158. mark_running :1449-1475.
- Nested views:
  - Interpreter: a nested view is a plain call_user/call_walked. The in_view counter is at src/ir_interp.rs:3335-3355, and the call returns a markup string. src/served.rs:186-200 page_render wraps the result as CrowVNode::raw(html).
  - Compiled: a plain Rust fn `v(state, args) -> CrowVNode` (src/emit_rs/stmts.rs:89-90; src/emit_rs/exprs.rs:3617-3644).
    - It opens with `let __view = state.view_enter();` (stmts.rs:173-176) and returns `state.view_exit(__view, …)` (stmts.rs:1468, :521-523).
    - view_exit (live/mod.rs:1991-2019) is used only for data-crow-stale. There is no memo or boundary otherwise.
    - A Node passed as a function argument is turned into .to_html() (exprs.rs:3602-3606).
- Messages server to browser:
  - html {html, head?, ack?} (:3568, :4001).
  - patches {patches, ack?, head?} (:3999).
  - ack (:3942, :4012), head (:4022), session {token} (:3579), url {path, query} (:3295).
  - upload {field, url} (:3846), download {url, name} (:4187), sign {url} (:4210).
  - dev messages (DEV_JS :759).
- Patch ops (crow_web/src/view/mod.rs:1212-1297):
  - text {path, text, in?}
  - attrs {path, tag, set{}, remove[]}
  - insert {path, tag (holder), index, html}
  - remove {path, tag (holder), index}
  - replace {path, html, in?}
  - path[0] = the root element under #crow-root.
  - Child matching is keyed by HTML `id` (view/mod.rs:1238-1243; JS morphKids live/mod.rs:2776-2790).

==================================================
3. DERIVATIONS
==================================================
Read sets (static): src/derive.rs
- ReadSet :13-47, fact_names :53, written_facts :78, session_written :144-180, address_fact :212-235.
- read_sets :240-256, mark_shared :263-292, read_set :296-314, walk_stmts :360.
- walk_expr :419-509:
  - now sets rs.now :431. __reader :434. file effects :439-442.
  - query tables and keys :445-471.
  - follows into called fns :472-487.
- Called from src/lower_ir/checks.rs:559 (v.read_sets) and attached to IR Derive.spec at src/lower_ir.rs:1910-1913.
- Derive struct src/ir.rs:384-398. lower_derive src/lower_ir.rs:2053-2088 (spec starts as Default). src/checker.rs:2271 prints read sets.

Runtime versions, dirtiness and caches (crow_web/src/live/mod.rs)
- DeriveSpec :71-87, DerivedSlot :91-106 (holds the value plus input versions), Inputs :115-133.
- Fact versions :353. set :2436-2441 and touch :2445 bump them. Global VERSION :210-214.
- TABLE_VERSIONS :432, publish_table :278-281, keyed versions and WRITE_LOGS :543-611.
- Where the value is kept: the per-session cache `derived` :364, or SHARED_DERIVED :428 for shared derives (cache_for :2096-2098).
- note_read :1972-1981.
- derive :2114-2124 and derive_with :2129-2141.
- derive_bg :2150-2190 (stale value plus schedule :2032-2085; mark_stale :1984).
- inputs_now :2194-2217.
- lookup :2220-2242: a `now` derive is fresh only when tick_seen == tick (:2236); external reads use the freshness check at :2237.
- compute_and_store :2247-2292.
- compute_now :2297-2335: the version is bumped only if the value is not equal (early cutoff, :2317-2323).
- TICK :613, next_tick :2091-2093.

Compiled emission (src/emit_rs/stmts.rs:287-363)
- Emits __derive_body_X, `static __DERIVE_SPEC_X` (:328-338), and __derive_X.
- __derive_X computes its deps first, then calls state.derive (no tables) or state.derive_bg (tables) (:340-361).
- The eq closure is `|a,b| a==b` only for scalars; otherwise it is `|_a,_b| false` (:324). So early cutoff applies to scalars only.
- A view binds each derivation once at the start: `let d = __derive_d(state);` (stmts.rs:188-199).

Interpreter
- Expr::Derived (src/ir_interp.rs:1243) goes to derived() :3378-3392 and then compute_derived :3428-3446 on every read, with no cache.
- Exceptions:
  - route handlers: route_reads plus app_read :3414-3426, through app.derive_with and app_spec :4292-4319.
  - remote compiled sessions.
- IR binds derivations once only for route handlers (src/lower_ir.rs:1986-2008, :6062-6068).
- The interpreter's `tick` field (near src/ir_interp.rs:145) is incremented only in render_view :3704 and is never read.
- The served interpreter also clones the whole template for each render and each handler (src/served.rs:138-159, :188).
- CONFIRMED: the interpreter recomputes every read; compiled code caches.

now[] and refresh ticks
- Serve refresh: configure crow_web/src/live/mod.rs:642-646 (interpreter calls it at src/served.rs:411; compiled at src/emit_rs/builtins.rs:789). refresh_secs :902, interval :3627-3635.
- RefreshTick sets tick=true, which triggers next_tick (:3958).
- A click render is not a tick (:3703-3706).
- Page load and socket open call next_tick (:3414, :3526).

==================================================
4. STATE
==================================================
Facts
- src/type_infer/classify.rs:309-310: every top-level assignment is a fact. derive::fact_names src/derive.rs:53.
- src/lower_ir.rs:1718-1770: an S::Assign inside holds_state becomes Fact {name, ty, default, session, line, refuses, held} (src/ir.rs:357-374).
- An identifier that names a fact becomes Expr::Load (src/lower_ir.rs:6058-6063). A write becomes Stmt::Store {fact, expr} (src/ir.rs:642).

Handlers
- src/type_infer/classify.rs:200-416:
  - fn refs in views and event calls.
  - page args :241-255.
  - click[] in tests :259-305.
  - functions that assign facts / acts_for_session :315-370.
  - later targets :376-411.
  - view_handlers = handlers minus derives :416. Widget handler param typing :417-420.
- lower_ir kind_of src/lower_ir.rs:1291-1303 assigns View/Handler/Helper/Pure (FnKind src/ir.rs:400-407).

Interpreter reads and writes
- Load: Expr::Load evaluates `self.fact(f)` = frames[0].scopes[0] (src/ir_interp.rs:1242, :1004-1006).
- Store: src/ir_interp.rs:1107-1118. A fact never holds an error; refuse is called instead. Then set_fact :724-726.
- fact_values :729.
- activate :735 swaps harness sessions in and out of the globals.

Compiled reads and writes
- src/emit_rs/program.rs:
  - page render fns, reactive_render_fns and reactive_event_handlers (from click/input/submit/actions/on/chart seek, later, and Handler kind) :275-399.
  - read_facts :657-740. CrowFacts struct :1794-1830+. install_facts :1620-1621. set_fact_in_main :752-760.
  - __crow_handle dispatch :1390-1473: decodes the args JSON array per parameter type and stops on arity.
- Loads: `__facts(&*state).field` (src/emit_rs/exprs.rs:425-434).
- Stores: emit_store_fact with state.touch (src/emit_rs/stmts.rs:1029-1062; Store arm :642). Path writes :998-1022.
- Signatures: handlers take `state: &mut CrowLiveState` (stmts.rs:44-57); renders take `state: &CrowLiveState` (:34-43).

Served interpreter (src/served.rs)
- serve :286-423:
  - initial facts as JSON :293-298, sources :302-307, pages :310-322.
  - the handle closure :330-386: with_session, args from the JSON array :333-337, it.transaction :339, facts copied back :350-357, then notice/downloads/mails/sign/writes.
  - routers :405-410.
- transaction is at about src/ir_interp.rs:3473-3580.

Test harness, interpreter (src/ir_interp.rs harness() :3730+)
- page :3736-3749, serve :3750, route, fetch/poll :3775-3784.
- app :3785-3822: sessions, session_start, clock. remote_open is used if compiled_harness is active.
- html :3863-3868 builds raw, then as_built, then to_html, with no split_head.
- elements :3869-3883.
- change :3884-3928 handles reader, now and fact.
- click :3929+.
- render_view :3703-3722 adds its own toast.

Test harness, compiled
- src/compiled_harness.rs RemoteApp (CROW_TEST_CONTROL :80, post :102) talks to crow_web/src/live/test_control.rs.
- router :116-143 has verbs open, event, field, clock, reader, read, facts, html, downloads, mails.
- open :160-182 calls start_session. event :189-220.
- Session::html :78-100 = render_with plus as_built().to_html(). It re-renders while stale and has no split_head.
- Generated hooks: program.rs:1292-1305 __crow_view, :1331 __crow_derived, :1692 merges test_control::router.

==================================================
5. page[...] AND WIDGETS
==================================================
page parameters
- src/lower.rs:1105-1115: params ["path","view","url"] with resolve_args.
- The error text is at src/lower.rs:1320-1324: "'{}' has no parameter named '{}' (parameters: {})". That produces "'page' has no parameter named 'init'".
- Signature rows src/table.rs:3098-3099 (web.page (Str, Shown) and (Str, Shown, Shown)).
- Address checks src/lower_ir/checks.rs:1825-1841.

Page registration
- Interpreter: harness "page" (src/ir_interp.rs:3736-3749), and src/served.rs:405-410 builds CrowLiveView::new(...).with_url.into_router.
- Compiled: src/emit_rs/builtins.rs:888-924 page_route. Pages registered from a function go through the `later` queue :866-886 (landed :878-886).
- CrowLiveView crow_web/src/live/mod.rs:3199-3343.
- into_router routes :4170-4176: page, ws, _sign, _download, _upload, and _widget (:4149-4166, which serves widgets/<name>.js).

Widgets
- Rendering: interpreter src/ops.rs:1213-1245; compiled src/emit_rs/ui.rs:306-338. Both use CrowVNode::widget (crow_web/src/view/mod.rs:367-377).
- Browser: widgets() crow_web/src/live/mod.rs:3052-3073:
  - imports the module and calls mod.mount(el, props, sendFor).
  - calls update when props change.
  - sendFor sends {event: on[ev], value: JSON.stringify([value])}.
  - morph keeps widget insides (:2808-2819).
- Server: Offers.note_widget :1125-1131 and allows :1146.
- Handler args are decoded from a JSON array (src/served.rs:333-337; src/emit_rs/program.rs:1455-1460).

==================================================
6. EMBEDDED BROWSER SCRIPT
==================================================
- page_js crow_web/src/live/mod.rs:2586-3123: one format! string, about 530 lines and about 23 KB. live_js :2572, live_js_for :2579.
- DEV_JS :759-828 (about 4 KB). LIVE_CSS :3175, OFFLINE_CSS :3189.
- Functions:
  - key/tabToken/wsUrl :2596-2615.
  - answered/remark (busy) :2620-2628.
  - offline :2634-2649.
  - send :2650-2654.
  - applyHead :2658-2668.
  - connect and onmessage :2670-2744.
  - morphChildren :2750-2770, sameKind :2771.
  - morphKids :2774-2802 (keyed by id).
  - morphNode :2803-2844 (widget, whole svg swap, live values of form controls).
  - fragment :2849, disturbs :2860, follow :2868.
  - applyChange :2877-2911, nodeAt :2914.
  - click delegation :2923-2939 (data-crow-busy).
  - keyName/canSee/canPress :2947-2968.
  - dialogs :2972-3006, shortcuts keydown :3007-3037.
  - input :3040-3046, widgets :3052-3073, upload change :3077-3084, select change :3087-3092, submit :3095-3116.
  - init :3118-3120.

==================================================
7. TESTS THAT PIN HTML, AND THE FULL SUITE
==================================================
Full suite
- CLAUDE.md:28-50 lists it:
  - cargo test --workspace; cargo clippy --workspace --all-features.
  - crow test --errors, --parity, --fuzz 20, --fuzz 50 --mistakes.
  - crow check --self; crow test --served, --docs, --book, --modules.
  - docs/book/site.co test plus run -i; tests/layout/conformance.co; tests/parity/185...
  - magpie/pie/raven ./test.sh (local only); scripts/coverage.sh.
  - A release binary is needed.
- .github/workflows/ci.yml adds:
  - `cargo test -p crow_web --features live,http` (:49) and --bench --quick.
  - src/modules/layout.co, examples/layout_dashboard.co, tests/parts/views/sim.co (also --compiled), taskboard tests (also --compiled).
  - harness .co files in both modes (mail, records, refused_assignment, fact_records, failed_helper_argument, failed_statement_parameter, chart, derive...).

Served parity: src/served_parity.rs
- Inputs: tests/served/*.co with .events files (36 pairs, 01_counter through 36_routes_a_function_registers), plus examples/<app>/main.co with main.events.
- Compares compiled and interpreted canonical pages. harness_canonical strips data-crow-stale/busy/aria-busy (src/harness.rs:109-140). Also checks expect/absent.
- Applies patches with CrowVNode::apply (:268-270). run :512-589.

Exact-HTML pins
- tests/ui_primitives.co asserts substrings, e.g. `<input aria-label="Qty" data-crow-input="__field:qty" type="number" value="100">`, `<hr class="divider">`, `aria-modal="true" class="modal" data-open="false" role="dialog"`.
- tests/*_harness.co check `in html[a]`.
- selfcheck views family src/selfcheck/web.rs:68-117 and DRAWN :319-324 (compares the two modes).
- Book blocks with expected text (src/main.rs:626-721; e.g. docs/book ch12-01-views.md:70).
- tests/layout/conformance.co with tests/layout/expected/*.csv (layout read from inline styles).
- Rust unit tests:
  - crow_web/src/view/mod.rs: 23 tests (change_tests :1600+, tests :1992+).
  - crow_web/src/live/mod.rs: 45 tests, including browser tests the_browser_applies_changes :5444 and a_widget_draws... :5265.
  - crow_web/tests/integration.rs: 26 (field/select/table HTML :397-430).
  - crow_runtime/src/layout.rs: 9.
  - src/emit_rs/tests.rs: 306 (UI codegen tests :2011-2200).
  - src/interp_tests.rs: 389.
- tests/parity: 506 .co files, 8 with `use ui`, compared across modes.

==================================================
PLACES A "SEMANTIC TREE WITH TWO RENDERERS" REFACTOR MUST TOUCH
==================================================
1. Semantic source. IR Expr::Tree and control calls: src/ir.rs:884, :944; src/lower_ir.rs:6811-6874; src/lower.rs:27-48, :1105-1133, :1512-1528. The new node kind and options would come from here.
2. Interpreter renderer. src/ops.rs:1206-1493 (render_widget, render_element, render_input) and src/ir_interp.rs:1254-1326 plus :2855-2950 (progress, chart, icon). These must return semantic nodes instead of markup strings.
3. Compiled emitter. src/emit_rs/ui.rs:20-493. It currently bakes layout::render at compile time (:94-109) and chooses the HTML tag at :29. It must emit semantic nodes, not HTML-shaped CrowVNode.
4. Node type and HTML renderer. crow_web/src/view/mod.rs: CrowVNode :61-73, builders :103-581, to_html :380-413, as_built :1192-1204, changes/apply :1212-1359. The new renderer must produce byte-identical HTML.
5. Kernel table. crow_runtime/src/layout.rs: render :155-288, confirm_wiring :309-317, press_attrs :327-336, frame_table_html :367-517, nav_active_html :522-542, icon_html :608-622. All of these produce strings or Raw today.
6. Live loop. crow_web/src/live/mod.rs:
   - Offers :990-1159 (collect_markup scans HTML).
   - append_child/with_toast/with_dialog :1236-1273, mark_running :1449-1475, render_with :1949-1970.
   - served_render :3313-3316 and the diff/send site :3993-4007.
   - Browser morph/applyChange :2746-2920.
7. Interpreter served glue. src/served.rs:186-200 (page_render wraps an HTML string as Raw) and the handle closure :330-386.
8. Harnesses.
   - crow_web/src/live/test_control.rs Session::html :78-100.
   - src/ir_interp.rs html/elements :3863-3883 and render_view :3703-3722.
   - src/harness.rs parse/select/canonical/harness_to_lnode :109-140, :209-272, which rebuilds semantics from CSS.
9. View boundary and derivation reuse.
   - Compiled: src/emit_rs/stmts.rs:173-199 (view_enter, bind-once derives), :287-363 (derive emission), :521-523, :1468; crow_web/src/live/mod.rs:1991-2019 (view_exit, only for stale) and :2114-2335 (derive cache and cutoff).
   - Interpreter: derived()/compute_derived src/ir_interp.rs:3378-3446 has no cache and needs one for parity in phase 2.
10. Handler and fact classification and dispatch, plus the guard tests.
   - src/type_infer/classify.rs:200-420; src/emit_rs/program.rs:275-399, :1390-1473; served args src/served.rs:333-337.
   - Guards: src/served_parity.rs, tests/ui_primitives.co, src/selfcheck/web.rs:68-117/:319-324, book expected blocks, src/emit_rs/tests.rs:2011-2200, crow_web/tests/integration.rs:397-430.

==================================================
SURPRISES AND CONTRADICTIONS WITH docs/one-view-design.md SECTION 2
==================================================
- The interpreter never holds a CrowVNode tree while it renders. Views produce HTML strings (src/ops.rs:1247-1444). A tree exists only after re-parsing (src/served.rs:195 Raw, then as_built).
- Compiled trees are not pure trees either. They contain Raw nodes for frame tables, icons, charts and markup args/lists (src/emit_rs/ui.rs:272, :278, :399-402).
- Both modes go through as_built (to_html then read_markup), so the diff runs on a re-parsed tree in both. If the markup needs repair or has more than one top node, the whole page becomes one Raw (crow_web/src/view/mod.rs:1192-1204), and then it is replaced whole instead of patched.
- The generic element renderer exists twice: run time in src/ops.rs and compile time in src/emit_rs/ui.rs, where literal layout is baked into the generated code. progress is hand-duplicated (src/ir_interp.rs:2855-2871 vs crow_web/src/view/mod.rs:295-300). Compiled tree_rest has no "change" event arm; ops.rs:1365-1382 handles change.
- "Matched by key" in practice means HTML `id`, both in changes (crow_web/src/view/mod.rs:1238-1243) and in JS morphKids (crow_web/src/live/mod.rs:2776-2790). A `key=` on an element is just a passthrough attribute (crow_runtime/src/layout.rs:271). key exists only for table rows.
- Early cutoff exists only for scalar derives. Non-scalar eq is `|_,_| false` (src/emit_rs/stmts.rs:324).
- The interpreter recomputes every derivation read in pages and handlers (src/ir_interp.rs:3378-3446), and also clones the whole template per render and per handler (src/served.rs:138-159). Its `tick` field is dead (src/ir_interp.rs:3704).
- Compiled already has a per-view boundary (view_enter/view_exit, src/emit_rs/stmts.rs:173-176, crow_web/src/live/mod.rs:1991-2019), but it is used only for data-crow-stale. A nested view called with a Node argument stringifies it (src/emit_rs/exprs.rs:3602-3606).
- The test harness html paths (test_control.rs:78-100 and src/ir_interp.rs:3863-3868) skip split_head, unlike served_render (crow_web/src/live/mod.rs:3313-3316). Head handling is not identical across the paths.
- Offers (the event allow-list) are collected by scanning built markup (crow_web/src/live/mod.rs:1071-1122). A semantic tree could produce them directly, but they will move.
- Semantic kind survives only as class/role/data-* strings, and src/harness.rs:209-258 rebuilds layout from inline CSS. Changing the class or style output breaks the harness and tests/layout/expected/*.csv.
- Lory gui.co, which section 2 cites as the semantic example, is not in this repo. It is at /Users/vijay/code/crow_projects/lory/packages/lory/gui.co. The crow repo has only packages/symbolic.
```
