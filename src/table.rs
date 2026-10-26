//! One row per builtin (docs/bug-classes.md §4, ir-design.md §3.5).
//!
//! A builtin written twice, once as the interpreter's adapter and once as
//! the compiled program's printing, drifts: one mode checks an argument the
//! other does not, or fails with a value where the other stops. A builtin
//! with a row is written once, as a runtime function taking typed arguments
//! and answering `Result`, and the row says the rest:
//!
//! - the parameters' kinds, from which the interpreter reads its arguments
//!   ([`read_int`] and the others) and the emitter prints them
//!   ([`Kind::print`]);
//! - the answer's kind, from which the interpreter builds its value and the
//!   lowering types the call;
//! - what a failure is: a stop (a misuse no value can carry: the runtime's
//!   reason, on the call's line), an error value of a kind, or impossible.
//!
//! Both modes then agree by construction, for every row: the only code per
//! mode is the reading and the printing of each kind, written once here.
//! The IR names a row by its id (`moments.calendar`); the interpreter calls
//! the row, the emitter prints it, inference types it from it.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::type_infer::Ty;
use crate::value::{CrowValue, Value};
use crow_runtime::tensor::CrowTensor;

/// What a parameter or an answer is, as the runtime function takes or
/// gives it.
/// A function a row takes as a value, with its signature — declared on
/// the row, as everything about a builtin is (design rule 24, phase 4;
/// docs/extending-crow.md, section 7): what the function takes, in order
/// (`Item` is the row's walked argument's item, `Request` the request a
/// route's handler is handed), what it answers, whether the row's
/// remaining arguments are handed to it after `params` (`later[f, a, b]`
/// calls `f[a, b]`; `stream[feed, a]` calls `feed[put, a]`), and whether
/// it runs deferred, off the loop (`later`'s work: the function is a
/// handler's deferred half). An option the page calls as an event (a
/// chart's seek) is a `Preset::Event` with its signature. Inference
/// hands a function written where the row takes one its parameters from
/// the signature; the kinds check refuses anything else in the row's
/// words; the width pass, the classification and `crow check` read the
/// same declaration. Each had been a list of the rows' names.
#[derive(Debug, PartialEq)]
pub struct Sig {
    pub params: &'static [Kind],
    pub ret: Kind,
    pub rest: bool,
    pub deferred: bool,
}

/// A handle: a value a module's native rows make and take, opaque to the
/// language — not indexable, not walkable, not JSON, not in a mask, moved
/// not copied, equal to no other, shown as its word (design rule 24,
/// phase 5; docs/extending-crow.md, section 8). Declared once, beside its
/// module's rows: its name (the type's word), how it shows, the Rust type
/// the compiled program holds it as, that type's default, and whether a
/// session may hold it (a fact of it is the session's; else it is the
/// process's, a static). One type, `Ty::Handle`, and one class,
/// `Arg::Handle`, cover every handle; a row's kind (`Kind::Mailer`, …) is
/// the row's bridge to the runtime's own type, reading this declaration.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Handle {
    pub name: &'static str,
    /// The type's name in a message (`type[x]` answers it compiled as the
    /// interpreter's value does).
    pub crow: &'static str,
    pub word: &'static str,
    pub rust: &'static str,
    pub default: &'static str,
    pub session: bool,
    /// Read by its fields: a test's app session, whose fields are the
    /// page's facts (`a.count`); every other handle has none.
    pub fields: bool,
}

/// A database connection, and one inside a transaction (db).
pub const DB: Handle = Handle { name: "db", crow: "db.conn", word: "a database connection", rust: "crow_db::DbHandle", default: "panic!(\"no default db connection\")", session: false, fields: false };
pub const TX: Handle = Handle { name: "tx", crow: "db.tx", word: "a transaction's connection", rust: "&dyn crow_db::DbTx", default: "panic!(\"no default db transaction\")", session: false, fields: false };
/// The layer's socket, byte stream and mailer (sys).
pub const SOCKET: Handle = Handle { name: "socket", crow: "socket", word: "a socket", rust: "crow_runtime::sys::Socket", default: "crow_runtime::sys::Socket::closed()", session: true, fields: false };
pub const STREAM: Handle = Handle { name: "stream", crow: "stream", word: "a stream", rust: "crow_runtime::sys::CrowStream", default: "crow_runtime::sys::CrowStream::closed()", session: true, fields: false };
pub const MAILER: Handle = Handle { name: "mailer", crow: "mailer", word: "a mailer", rust: "crow_runtime::sys::Mailer", default: "crow_runtime::sys::Mailer::default()", session: false, fields: false };
/// A route's response, the server's app, and a test's app session (web).
pub const RESPONSE: Handle = Handle { name: "response", crow: "web.response", word: "a response", rust: "crow_web::core::CrowResponse", default: "crow_web::core::CrowResponse::default()", session: true, fields: false };
pub const WEBAPP: Handle = Handle { name: "web.app", crow: "web.app", word: "the server's app", rust: "crow_web::server::CrowApp", default: "crow_web::server::CrowApp::new()", session: true, fields: false };
pub const APP: Handle = Handle { name: "app", crow: "app", word: "an app session", rust: "()", default: "()", session: true, fields: true };

/// A predicate over a walked collection's items: `any`, `all`, `find`.
pub const PRED: Sig = Sig { params: &[Kind::Item], ret: Kind::Bool, rest: false, deferred: false };
/// Work scheduled off the loop, handed the row's remaining arguments:
/// `later[f, a, …]`, and `running[f]`'s question about it.
pub const DEFERRED: Sig = Sig { params: &[], ret: Kind::Unit, rest: true, deferred: true };
/// A stream's `put`: sends one chunk of text, and answers whether the
/// reader still listens.
pub const PUT: Sig = Sig { params: &[Kind::Str], ret: Kind::Bool, rest: false, deferred: false };
/// A stream's feed: handed `put`, then the row's remaining arguments.
pub const FEED: Sig = Sig { params: &[Kind::FnOf(&PUT)], ret: Kind::Unit, rest: true, deferred: false };
/// A route's handler: the request to a response, called by the server.
pub const HANDLER: Sig = Sig { params: &[Kind::Request], ret: Kind::Response, rest: false, deferred: false };
/// A transaction's body: the connection, inside the transaction, to its value.
pub const TX_BODY: Sig = Sig { params: &[Kind::Tx], ret: Kind::Shown, rest: false, deferred: false };
/// A chart's seek: handed the clicked bucket's moment, as a number.
pub const SEEK: Sig = Sig { params: &[Kind::Float], ret: Kind::Unit, rest: false, deferred: false };

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// A map's key: text or an int, by the map (`set[m, k, v]`, `k in m`);
    /// the compiled program passes `&K` of the map's type.
    Key,
    /// A list of keys of one kind (`map[keys, values]`): `&[K]`.
    Keys,
    /// `i64`: an int, or a float cut toward zero (as `as i64` does).
    Int,
    /// `i64`: an int only — a row beside a float's row of the same name
    /// (`min` of two ints is an int, of a float a float).
    Whole,
    /// `f64`: a float, or an int.
    Float,
    /// `Moment` taken and given (crow_runtime::moments).
    Moment,
    Bool,
    /// `&str` taken, `String` given.
    Str,
    /// `&CrowTensor` taken, `CrowTensor` given: a tensor, or a list of
    /// numbers.
    Tensor,
    /// `BTreeMap<String, f64>` given: a test's numbers by name (`ttest`'s
    /// `t` and `p`).
    FloatMap,
    /// `&[i64]` taken: the rest of the arguments, each a size (`zeros[2, 3]`,
    /// `poisson[1.5, 4]`); the fixed parameters around it take theirs first.
    Sizes,
    /// `&[CrowTensor]` taken, `Vec<CrowTensor>` given: a decomposition's
    /// parts (`svd`, `qr`), a complex number's `[real, imag]`.
    Tensors,
    /// `&[&CrowTensor]` taken: the rest of the arguments, each a tensor
    /// (`anova[a, b, c]`).
    Groups,
    /// Nothing given: a function run for what it sets (`seed`).
    Unit,
    /// `&CrowSparse` taken, `CrowSparse` given.
    Sparse,
    /// `&CrowFrame` taken, `CrowFrame` given.
    Frame,
    /// `&[String]` taken, `Vec<String>` given: a list of text (a frame's
    /// column names, a column's distinct values).
    Strs,
    /// `&[bool]` taken: a mask, one per row.
    Bools,
    /// `&Cell` taken: a number, text or a bool for a frame's cell (`fill`).
    Cell,
    /// `Cells` taken: a column's values — a list, a tensor, another frame's
    /// column with its missing cells, or a cell for every row (`with`).
    Cells,
    /// `&[&[String]]` taken: the rest of the arguments, each a list of
    /// text (`group`'s `[column, how]`).
    Lists,
    /// `&[String]` taken: a list's items as text, a number as it displays
    /// (`join`'s items, `crow_runtime::cell::Words`).
    Words,
    // A collection's builtin is one generic function over its items: the
    // compiled program calls it with its typed items, the interpreter with
    // its values (crow_runtime::core, "the collections a program calls").
    /// `Vec<T>` taken and given: a list the function makes its answer of,
    /// moved when the statement assigns the variable it reads (`xs =
    /// push[xs, x]` stays in place), copied otherwise.
    List,
    /// `&[T]` taken: a list read.
    Seq,
    /// `T` taken: an item a list or a map keeps (a variable read is copied;
    /// into a list whose items can fail, it is an item that did not).
    Item,
    /// `&T` taken: an item compared (`contains`).
    ItemRef,
    /// `BTreeMap<String, T>` taken and given, moved or copied as a list is.
    MapOf,
    /// `&BTreeMap<String, T>` taken: a map read.
    MapRef,
    /// `&BTreeMap<String, String>` taken: a map's values as text, each as
    /// it shows (a request's headers, query, form and files).
    TextMap,
    /// `Vec<i64>` taken and given, moved or copied as a list is.
    Ints,
    /// `&crow_runtime::int::Int` taken, `Int` given: a whole number of any
    /// size, for the hidden rows that keep every digit (`__abs_big`) the
    /// width pass calls where an int may grow past 64 bits (int_width).
    Big,
    /// `Vec<crow_runtime::int::Int>` taken and given, moved or copied as a
    /// list is: a list of whole numbers of any size (`__sort_big`).
    Bigs,
    /// `&crow_runtime::decimal::Dec` taken, `Dec` given: an exact decimal.
    Decimal,
    /// `&crow_runtime::fraction::Frac` taken, `Frac` given: an exact ratio.
    Fraction,
    /// `Vec<f64>` taken and given, moved or copied as a list is.
    Floats,
    /// `Vec<String>` taken and given, moved or copied as a list is.
    Texts,
    /// `&serde_json::Value` taken: a parsed JSON value, which only the
    /// compiled program holds as one (the interpreter holds what it parsed
    /// to, and picks the row of that).
    Json,
    /// `&CrowColumn` taken: a frame's column, which only the compiled
    /// program holds as one (the interpreter holds its cells as a list).
    Column,
    /// `&Bytes` taken, `Bytes` given (crow_runtime::sys): what a
    /// connection reads and writes.
    Bytes,
    /// `&CrowStream` taken, `CrowStream` given: a connection `sys.open`
    /// made.
    Stream,
    /// A list of bytes: a connection's chunks, gathered.
    Chunks,
    /// `Answer` given (crow_runtime::sys): a connection's answer, the
    /// record `sys.exchange` and `sys.fetch` make.
    Answer,
    /// `CrowResponse` given (crow_web::core): a response a route answers,
    /// made by a row (`sys.binary`, `msgpack`).
    Response,
    /// `&Socket` taken, `Socket` given (crow_runtime::sys): a WebSocket
    /// client's connection, `socket[url]`'s.
    Socket,
    /// `&Mailer` taken, `Mailer` given (crow_runtime::sys): a mailer
    /// `smtp[…]` made.
    Mailer,
    /// `String` given: markup a view places as markup (`doc.markdown`).
    Markup,
    /// `Vec<BTreeMap<String, String>>` given: rows of text by column
    /// (`io.csv`).
    TextRows,
    /// `crow_runtime::core::Shell` given: a command's status and output,
    /// the compiled program's `CrowShell` and the interpreter's record.
    Shell,
    /// `crow_runtime::sources::Reply` given: a request's status, body and
    /// headers, the compiled program's `CrowReply` and the interpreter's
    /// record.
    Reply,
    /// A function, with the signature the row declares (`Sig`).
    FnOf(&'static Sig),
    /// What a loop walks: a list, a tensor, a frame's column — the
    /// argument whose items a function parameter's `Item` names.
    Walked,
    /// A route's request (the record a handler is handed).
    Request,
    /// A database connection.
    Db,
    /// A connection inside a transaction (a transaction's body is handed one).
    Tx,
    /// A text search's match: its groups.
    Found,
    /// `&T` taken, any value that shows (`str`): the compiled program's
    /// `CrowDisplay`, the interpreter's value's.
    Shown,
    /// `&T` taken, any value JSON can hold (`encode`): the compiled
    /// program's `ToJson`, the interpreter's value's.
    Encoded,
    /// `Vec<serde_json::Value>` given: a parsed JSON object's values.
    JsonList,
    /// `&[&dyn CrowDisplay]` taken: the rest of the arguments, each any
    /// value that shows (`log`).
    Shows,
    /// Each of `Shows`: `&dyn CrowDisplay`.
    ShownDyn,
    /// `&[crow_runtime::fused::Operand]` taken: the rest of the arguments,
    /// each a tensor or a number — a fused chain's operands.
    Operands,
    /// Each of `Operands`.
    Operand,
    /// `&[crow_runtime::mask::Cell]` taken: the rest of the arguments, each
    /// a number, text or a bool — the values a fused mask
    /// compares its columns with.
    Scalars,
    /// Each of `Scalars`.
    Scalar,
}

/// What a failure of the function is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fails {
    /// It cannot fail: the function answers its value, not a `Result`.
    Never,
    /// A misuse no value can carry: the program stops with the reason on
    /// the call's line (spec, When a Program Stops).
    Stops,
    /// An error value of this kind (spec §10, the kinds the language
    /// produces).
    Value(&'static str),
}

/// A named option's default, where the row has one for it: the lowering
/// fills in what a call leaves out, so the row is handed every argument
/// (`request[url]` reaches its row with a method, a body, four maps, a
/// path and a count). An option with no default is given, or the row is
/// not the call's: `sum[x, axis = 0]` reaches `sum_axis`, `sum[x]` never
/// does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Preset {
    Int(i64),
    Float(f64),
    Str(&'static str),
    Bool(bool),
    /// An empty list.
    List,
    /// An empty map.
    Map,
    /// No default: the row applies to a call that gives the option.
    Asked,
    /// A function the page calls as an event (a chart's `seek`), with its
    /// signature; none by default (0). The function is a handler.
    Event(&'static Sig),
}

/// A row's options: its last parameters, each by name with its default.
pub type Options = &'static [(&'static str, Preset)];

/// The positions, in a call of the name lowered to its positional form,
/// of the options the page calls as events (`Preset::Event`), each with
/// its signature — the one declaration of what an event handler on a row
/// takes.
pub fn event_positions(name: &str, argc: usize) -> Vec<(usize, &'static Sig)> {
    let bare = name.rsplit('.').next().unwrap_or(name);
    // (a row's options describe its last parameters, by name)
    for r in rows_named(bare) {
        if r.options.is_empty() || r.params.len() != argc || r.options.len() > r.params.len() {
            continue;
        }
        let first = r.params.len() - r.options.len();
        let found: Vec<(usize, &'static Sig)> = r
            .options
            .iter()
            .enumerate()
            .filter_map(|(i, (_, p))| match p {
                Preset::Event(sig) => Some((first + i, *sig)),
                _ => None,
            })
            .collect();
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// The positions of the arguments a call of the name hands to a row's
/// deferred work (`Sig::deferred`: `later[f, …]`, `running[f]`).
pub fn deferred_positions(name: &str, argc: usize) -> Vec<usize> {
    fn_params(name, argc).into_iter().filter(|(_, s)| s.deferred).map(|(i, _)| i).collect()
}

/// The range of the int a row answers, for the width pass (int_width):
/// which rule reads its arguments' ranges into its answer's. Any other
/// row's int is 64-bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntRange {
    /// Any 64-bit int.
    I64,
    /// A count: 0 to 2^48.
    Count,
    /// A position, or -1 for none: -1 to 2^48.
    Position,
    /// The first argument's, made non-negative.
    Abs,
    /// The smaller of two arguments'.
    Min,
    /// The larger of two arguments'.
    Max,
    /// The first argument's (a whole number kept, or rounded).
    Keeps,
    /// The first argument itself, as it is: the identity.
    Same,
    /// The bits two arguments share.
    And,
    /// The bits of either of two arguments.
    OrXor,
    /// A character's code: 0 to 0x10FFFF.
    Code,
    /// The first argument's bits moved by the second.
    Shift,
    /// 0 or 1.
    Bool,
    /// A whole number of any size: read from text or a float, or rounded
    /// from an exact number.
    Any,
}

/// What a builtin does beyond answering its value, declared on its row
/// (design rule 24): each had been a list of names in the compiler, read
/// by name — any row of the name has the property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prop {
    /// Reaches outside the program's values, in this kind (crate::effects):
    /// its call lowers to `Expr::Effect`, which the session layer performs;
    /// what the kind implies — held to a handler's commit, its answer
    /// stamped with its capture moment, recorded by a rehearsal, waiting,
    /// acting for the session, allowed in a view or a derivation — is the
    /// kind's rule, stated once there.
    Does(crate::effects::Effect),
    /// Its order shows (an assertion's stop names its round, a look at
    /// background work changes as the work does): a loop over it keeps its
    /// order, as a loop over an effect does.
    Ordered,
    /// Answers by the run, not by its arguments alone (a value's capture
    /// moment, whether background work runs): a function `crow check`
    /// probes as pure cannot call it.
    Varies,
    /// A page's data source, which the session loads.
    Source,
    /// A form whose `ret` is its answer: inference types the call by the
    /// row, as a runtime row's call is typed (a form answers nothing by
    /// default, its answer a rule's).
    Answers,
    /// Binds the fact in this argument to an input.
    Binds(usize),
    /// Only inside a test block.
    TestOnly,
    /// Takes its collection by value, so `x = f[x, …]` moves `x` in and
    /// grows it where it lies (the text forms through `taken`).
    InPlace,
    /// This argument is a mask, one bool per row: a frame's column of bools
    /// there is read as the mask it makes.
    Mask(usize),
    /// Answers the whole numbers from its first bound to its last, in order
    /// (`range`): a loop over it is counted, its items lie between the bounds.
    Range,
    /// This argument names a table by its path: the format the path may
    /// name is built into the runtime (crow_runtime::sources::table_feature).
    Table(usize),
    /// A function `grad` differentiates may call it with a traced value:
    /// it is differentiated, or read and never differentiated (a length, a
    /// conversion), or piecewise constant.
    Differentiable,
    /// Makes a constant of the tape: its arguments are sizes or lists.
    Constructs,
    /// A dual number (forward mode) answers it with its derivative, by the
    /// function's own rule.
    Dual,
    /// Its name is not reserved by its module's import: a frame's `select`
    /// shares the control's.
    Unreserved,
    /// Opens a connection: the literal URL it is given names the database
    /// a provider asks about the program's statements, and the tables the
    /// program declares on the connection are declared there.
    Connects,
    /// Writes a frame's column where the frame lies: the row `df.name =
    /// values` lowers to, and the literal's columns (`frame.new[a = xs]`)
    /// — found by this property (`table::column_write`), so the lowering
    /// names no row.
    ColumnWrite,
    /// Reads a frame's column by a name the program computes (`df.`name``,
    /// `df.lag_`k``): found by this property (`table::column_read`).
    ColumnRead,
}

/// The shape of the tensor a row answers, declared on the row from a fixed
/// vocabulary and read where the call is typed (lower_ir/shapes.rs): the
/// rank every answer has (spec, Arrays, Design Note "rank is in the type")
/// and the sizes that follow from what the program states — a literal
/// size, an argument's own shape. A dimension the program computes stays
/// open. Each had been an arm of a match over the builtins' names in the
/// compiler (design rule 24).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Not stated: the rank shows as the program runs (and a row that
    /// answers no tensor says nothing).
    Open,
    /// Argument i's own shape: an element-wise map, a sort, a system's
    /// solution shaped like its right-hand side.
    Same(usize),
    /// The element-wise shape of these arguments, by broadcasting.
    Broadcast(&'static [usize]),
    /// A dimension per size argument from position i on, the last `k`
    /// arguments being no size (`full`'s value): `zeros[2, 3]`.
    Sizes(usize, usize),
    /// Each dimension declared (`Dim`): a square of a literal (`eye[n]`),
    /// a row per target and a column per class (`onehot`), a vector or a
    /// matrix of unknown sizes.
    Dims(&'static [Dim]),
    /// Argument i's shape without the axis argument j names: a reduction
    /// along an axis.
    Reduce(usize, usize),
    /// Argument i's shape, its first dimension this instead: one row more
    /// or fewer (`push`, `pop`, `slice`: open), a least-squares solution
    /// with the matrix's columns as its rows.
    Like(usize, Dim),
    /// Argument i's shape with the dimension open that the axis argument
    /// j names, the first where no argument does: a join along an axis.
    Along(usize, Option<usize>),
    /// A vector of all argument i's numbers.
    Flat(usize),
    /// Argument i's last two axes swapped.
    Transposed(usize),
    /// One dimension more than argument i's items have (a list of rows
    /// stacked), a tensor's own restacked.
    Stacked(usize),
    /// `Stacked`, every size open: stacked along a chosen axis.
    Restacked(usize),
    /// A vector's square matrix, a matrix's diagonal vector.
    Diag(usize),
    /// The shape of argument i's items, a list of tensors shaped alike (a
    /// complex tensor's parts).
    Items(usize),
    /// The pieces a decomposition answers, by position, each its dimensions
    /// (`svd`'s U, singular values and Vt; `eigen`'s values and vectors).
    Pieces(&'static [&'static [Dim]]),
}

/// How a row's frame gets its columns before the program runs: a function
/// of the row's runtime crate (`query`'s in crow_db), handed the literal
/// URL the connection argument was opened with, the literal statement and
/// the `CREATE TABLE`s the program declares on that connection, answering
/// each column's name and kind as the runtime writes it (`int`, `float?`,
/// …; `cell_type_of_kind` reads one into a type). A statement that is not
/// a literal gives an open frame.
pub struct Provider {
    /// The argument that is the connection.
    pub connection: usize,
    /// The argument that is the statement.
    pub statement: usize,
    pub columns: fn(&str, &str, &[String]) -> Result<Vec<(String, String)>, String>,
    /// Whether the statement writes: handed the literal statement, or
    /// nothing where the statement is built as the program runs, which
    /// writes for all the compiler can tell. A write takes the row's
    /// writing twin (`writing_twin`): the hidden form written as the same
    /// name, whose effect is the write's.
    pub writes: fn(Option<&str>) -> bool,
}

/// One dimension of a declared shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dim {
    /// Not stated.
    Open,
    /// The literal int written at argument i.
    Size(usize),
    /// Argument i's dimension j.
    Of(usize, usize),
}

pub struct Row {
    /// The id the IR names the row by (`Target::Row`, an effect's `row`).
    pub id: &'static str,
    /// The Crow name, and the module it is in ("" for the core).
    pub name: &'static str,
    pub module: &'static str,
    /// The runtime function, as `stringify!` wrote it (`rust_path` is
    /// what the emitter prints).
    pub path: &'static str,
    pub params: &'static [Kind],
    pub ret: Kind,
    pub fails: Fails,
    /// The last of its parameters that a call names, each with its
    /// default (`Preset`); empty for a row every argument of which is
    /// positional.
    pub options: Options,
    /// What it does beyond answering (`Prop`).
    pub props: &'static [Prop],
    /// The runtime function its call is printed as when its first argument
    /// is a text nothing else holds, or a tensor pushed onto: changed where
    /// it lies, as the interpreter's `take` bridge changes it.
    pub taken: Option<&'static str>,
    /// The id of the row that keeps every digit in place of this one, which
    /// answers 64 bits (int_width: `int` of text, `floor`, `abs`, …).
    pub big: Option<&'static str>,
    /// The range of the int it answers (`IntRange`).
    pub range: IntRange,
    /// The shape of the tensor it answers (`Shape`).
    pub shape: Shape,
    /// The columns of the frame it answers, where its kernel fixes them
    /// (doc's chapters, chart's macd); empty where they follow from the
    /// arguments or a provider.
    pub columns: &'static [(&'static str, Kind)],
    /// Who says the columns of the frame it answers (`Provider`).
    pub provider: Option<&'static Provider>,
    /// The crate feature its call needs built in, by crate: a response as
    /// Arrow, the GPU, the Parquet kernels.
    pub feature: Option<(&'static str, &'static str)>,
    /// The name a program writes to reach a row whose own name is hidden
    /// (`__fmt_time` is `fmt` with a time style: one name, the row an
    /// option picks, and its lowered form told from the plain row's by the
    /// hidden name); "" where the row's name is the written one.
    pub written: &'static str,
    /// How the program writes a hidden row that is a form of the language
    /// — `df.name = values` for the column write — which a refusal names
    /// in place of the row's hidden name (`table::form`); "" for a row the
    /// program calls by its name.
    pub form: &'static str,
    /// The interpreter's call: the arguments read by kind, the function
    /// applied, the failure dispatched as the row says. `trace` is where an
    /// error value is made.
    pub call: fn(&[CrowValue], &str) -> Result<CrowValue, String>,
    /// The same call, given the arguments to keep: a collection the
    /// function makes its answer of is moved into it, not copied (the
    /// interpreter's `xs = push[xs, x]` grows the list where it lies, as the
    /// compiled program's does).
    pub take: fn(Vec<CrowValue>, &str) -> Result<CrowValue, String>,
}

impl std::fmt::Debug for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Row({})", self.id)
    }
}

impl Row {
    /// The runtime function's path as the emitter prints it.
    pub fn rust_path(&self) -> String {
        self.path.replace(' ', "")
    }

    /// Whether the row's body is a function of the standard library written
    /// in Crow (`crow!`): the lowering calls that function where it would
    /// call a Rust row's runtime function.
    pub fn is_crow(&self) -> bool {
        self.path == IN_CROW
    }

    /// The call's type, for inference and the lowering: the answer's, able
    /// to fail when a failure is a value.
    pub fn ty(&self) -> Ty {
        let t = self.ret.ty();
        match self.fails {
            Fails::Value(_) => Ty::Result(Box::new(t)),
            _ => t,
        }
    }
}

/// How a parameter takes an int past 64 bits (`Kind::takes_big`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TakesBig {
    AsIs,
    AsFloat,
    Refused,
}

impl Kind {
    pub fn ty(self) -> Ty {
        match self {
            Kind::Int | Kind::Whole => Ty::Int,
            Kind::Big => Ty::Big,
            Kind::Decimal => Ty::Decimal,
            Kind::Fraction => Ty::Fraction,
            Kind::Float => Ty::Float,
            Kind::Moment => Ty::Moment,
            Kind::Bool => Ty::Bool,
            Kind::Str => Ty::Str,
            // (a key is text or an int by its map: the rules say which)
            Kind::Key | Kind::Keys => Ty::Unknown,
            Kind::Tensor => Ty::tensor(Ty::Float),
            Kind::FloatMap => Ty::Map(crate::ir::Key::Text, Box::new(Ty::Float)),
            Kind::Sizes => Ty::List(Box::new(Ty::Int)),
            Kind::Tensors | Kind::Groups => Ty::List(Box::new(Ty::tensor(Ty::Float))),
            Kind::Unit => Ty::Unit,
            Kind::Sparse => Ty::Sparse,
            Kind::Frame => Ty::Frame(None),
            Kind::Strs => Ty::List(Box::new(Ty::Str)),
            Kind::Chunks => Ty::List(Box::new(Ty::Bytes)),
            Kind::Bools => Ty::List(Box::new(Ty::Bool)),
            Kind::Cell | Kind::Cells => Ty::Unknown,
            Kind::Lists => Ty::List(Box::new(Ty::List(Box::new(Ty::Str)))),
            Kind::Words | Kind::Texts => Ty::List(Box::new(Ty::Str)),
            Kind::Ints => Ty::List(Box::new(Ty::Int)),
            Kind::Bigs => Ty::List(Box::new(Ty::Big)),
            Kind::Floats => Ty::List(Box::new(Ty::Float)),
            Kind::Json => Ty::Json,
            Kind::Column => Ty::Column(Box::new(Ty::Unknown)),
            Kind::Bytes => Ty::Bytes,
            Kind::Stream => Ty::Handle(&STREAM),
            Kind::Markup => Ty::Markup,
            Kind::TextRows => Ty::List(Box::new(Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str)))),
            Kind::Shell => Ty::Record("CrowShell".to_string()),
            Kind::Reply => Ty::Record("CrowReply".to_string()),
            Kind::Answer => Ty::Record("CrowAnswer".to_string()),
            Kind::Response => Ty::Handle(&RESPONSE),
            Kind::FnOf(sig) => Ty::Fn(sig.params.iter().map(|k| k.ty()).collect(), Box::new(sig.ret.ty())),
            Kind::Walked => Ty::Unknown,
            Kind::Request => Ty::WebRequest,
            Kind::Db => Ty::Handle(&DB),
            Kind::Tx => Ty::Handle(&TX),
            Kind::Found => Ty::Record("CrowFindResult".to_string()),
            Kind::Socket => Ty::Handle(&SOCKET),
            Kind::Mailer => Ty::Handle(&MAILER),
            Kind::TextMap => Ty::Map(crate::ir::Key::Text, Box::new(Ty::Str)),
            Kind::Shown | Kind::Encoded | Kind::Shows | Kind::ShownDyn | Kind::Operands | Kind::Operand | Kind::Scalars | Kind::Scalar => Ty::Unknown,
            Kind::JsonList => Ty::List(Box::new(Ty::Json)),
            // what the arguments' items are: inference types the call from
            // them (a generic row's answer is the list or map it was given)
            Kind::List | Kind::Seq | Kind::Item | Kind::ItemRef | Kind::MapOf | Kind::MapRef => Ty::Unknown,
        }
    }

    /// Whether the kind is over a collection's items, whatever they are: the
    /// call's type is its arguments', which inference reads.
    pub fn generic(self) -> bool {
        // (a map's keys are the map's kind: the rule reads it from the argument)
        matches!(self, Kind::List | Kind::Seq | Kind::Item | Kind::ItemRef | Kind::MapOf | Kind::MapRef | Kind::Key | Kind::Keys | Kind::Walked)
    }

    /// Whether the function takes the collection itself, to make its
    /// answer of: the compiled program moves the variable the statement
    /// assigns and copies any other.
    pub fn owned(self) -> bool {
        matches!(self, Kind::List | Kind::MapOf | Kind::Ints | Kind::Bigs | Kind::Floats | Kind::Texts)
    }

    /// An argument printed the way the runtime function takes it (each of
    /// a rest parameter's, as its [`Kind::each`]); `line` is where a stop
    /// in reading it stops (a frame's column's missing cell among words).
    pub fn print(self, arg: &str, line: &str) -> String {
        match self.each() {
            Kind::Int | Kind::Whole => format!("({}) as i64", arg),
            Kind::Float => format!("({}) as f64", arg),
            Kind::Moment => format!("({})", arg),
            Kind::Bool => format!("({})", arg),
            Kind::Str => match (literal_text(arg), borrowed_index(arg)) {
                (Some(lit), _) => lit,
                (None, Some(item)) => format!("&*({})", item),
                _ => format!("&*({})", arg),
            },
            // a map's key: `&K` of the map's type (a text literal owned)
            Kind::Key => match (literal_text(arg), borrowed_index(arg)) {
                (Some(lit), _) => format!("&String::from({})", lit),
                (None, Some(item)) => format!("({})", item),
                _ => format!("&({})", arg),
            },
            Kind::Keys => format!("&({})[..]", borrowed_index(arg).unwrap_or_else(|| arg.to_string())),
            Kind::Unit => format!("({})", arg),
            Kind::Strs | Kind::Bools | Kind::Chunks => format!("&({})[..]", borrowed_index(arg).unwrap_or_else(|| arg.to_string())),
            Kind::Cell => format!("&crow_runtime::cell::Cell::from(({}).clone())", arg),
            Kind::Words => format!("&crow_runtime::cell::words(&({})).unwrap_or_else(|e| crow_stop({}, e))[..]", arg, line),
            Kind::Cells => format!("crow_runtime::frames::Cells::from(({}).clone())", arg),
            Kind::Seq => format!("&({})[..]", borrowed_index(arg).unwrap_or_else(|| arg.to_string())),
            Kind::ShownDyn => match borrowed_index(arg) {
                Some(item) => format!("({}) as &dyn crow_runtime::display::CrowDisplay", item),
                None => format!("&({}) as &dyn crow_runtime::display::CrowDisplay", arg),
            },
            Kind::TextMap => format!("&crow_runtime::sources::text_map(&({}))", arg),
            Kind::Operand => format!("crow_runtime::fused::Operand::from(&({}))", arg),
            Kind::Scalar => format!("crow_runtime::mask::Cell::from(&({}))", arg),
            // moved or copied (an item kept) where the call is made
            Kind::List | Kind::MapOf | Kind::Ints | Kind::Bigs | Kind::Floats | Kind::Texts | Kind::Item => format!("({})", arg),
            Kind::FnOf(_) | Kind::Walked => format!("({})", arg),
            // (an item of a list a name holds, read where it lies)
            _ => match borrowed_index(arg) {
                Some(item) => format!("({})", item),
                None => format!("&({})", arg),
            },
        }
    }

    /// Whether the parameter reads its argument where it lies (printed as a
    /// reference): a value it is handed is not kept, nor copied.
    pub fn borrows(self) -> bool {
        !self.owned() && !matches!(self.each(), Kind::Int | Kind::Whole | Kind::Float | Kind::Moment | Kind::Bool | Kind::Unit | Kind::Cell | Kind::Cells | Kind::Item | Kind::ItemRef | Kind::FnOf(_) | Kind::Walked)
    }

    /// How the parameter takes an int past 64 bits: as it is where it holds
    /// or shows any value (a list's item, `str`'s, `log`'s), as its nearest
    /// float where it takes a number as a float, and refused where it takes
    /// 64 bits — a count, a position, a size, a key, a cell (spec, Numbers
    /// That Keep Every Digit). The width pass converts by it (int_width),
    /// and the interpreter's rows refuse by it (`groups_in_place`).
    pub fn takes_big(self) -> TakesBig {
        match self.each() {
            Kind::Shown | Kind::Encoded | Kind::ShownDyn | Kind::Item | Kind::ItemRef | Kind::List | Kind::Seq | Kind::MapOf | Kind::MapRef | Kind::Big | Kind::Bigs | Kind::Decimal | Kind::Fraction | Kind::FnOf(_) | Kind::Walked => TakesBig::AsIs,
            Kind::Float | Kind::Operand | Kind::Tensor | Kind::Floats => TakesBig::AsFloat,
            _ => TakesBig::Refused,
        }
    }

    /// Whether the parameter takes the rest of the arguments.
    pub fn is_rest(self) -> bool {
        matches!(self, Kind::Sizes | Kind::Groups | Kind::Lists | Kind::Shows | Kind::Operands | Kind::Scalars)
    }

    /// The kind of each argument a parameter takes: a size of the sizes, a
    /// tensor of the groups.
    pub fn each(self) -> Kind {
        match self {
            Kind::Sizes => Kind::Int,
            Kind::Groups => Kind::Tensor,
            Kind::Lists => Kind::Strs,
            Kind::Shows => Kind::ShownDyn,
            Kind::Operands => Kind::Operand,
            Kind::Scalars => Kind::Scalar,
            k => k,
        }
    }

    /// The value the compiled program holds for a failed call of this kind.
    pub fn default_rust(self) -> &'static str {
        match self {
            Kind::Int | Kind::Whole => "0_i64",
            Kind::Big => "crow_runtime::int::Int::default()",
            Kind::Decimal => "crow_runtime::decimal::Dec::default()",
            Kind::Fraction => "crow_runtime::fraction::Frac::default()",
            Kind::Float => "0.0_f64",
            Kind::Moment => "crow_runtime::moments::Moment::default()",
            Kind::Bool => "false",
            Kind::Str | Kind::Key => "String::new()",
            Kind::Keys => "Vec::<String>::new()",
            Kind::Tensor => "crow_runtime::tensor::CrowTensor::from_vec(vec![], vec![0])",
            Kind::FloatMap => "std::collections::BTreeMap::new()",
            Kind::Sizes => "Vec::<i64>::new()",
            Kind::Tensors | Kind::Groups => "Vec::<crow_runtime::tensor::CrowTensor>::new()",
            Kind::Unit => "()",
            Kind::Sparse => "crow_runtime::sparse::CrowSparse::from_dense(&crow_runtime::tensor::CrowTensor::zeros(vec![0, 0]))",
            Kind::Frame => "crow_runtime::frame::CrowFrame::new(vec![], vec![], vec![])",
            Kind::Strs => "Vec::<String>::new()",
            Kind::Chunks => "Vec::<crow_runtime::sys::Bytes>::new()",
            Kind::Bools => "Vec::<bool>::new()",
            Kind::Cell => "crow_runtime::cell::Cell::Int(0)",
            Kind::Cells => "crow_runtime::frames::Cells::One(crow_runtime::cell::Cell::Int(0))",
            Kind::Lists => "Vec::<Vec<String>>::new()",
            Kind::Words | Kind::Texts => "Vec::<String>::new()",
            Kind::Ints => "Vec::<i64>::new()",
            Kind::Bigs => "Vec::<crow_runtime::int::Int>::new()",
            Kind::Floats => "Vec::<f64>::new()",
            Kind::Json => "serde_json::Value::Null",
            Kind::Bytes => "crow_runtime::sys::Bytes::default()",
            Kind::Stream => STREAM.default,
            // taken, never given
            Kind::Column => "unreachable!()",
            Kind::Markup => "String::new()",
            Kind::TextRows => "Vec::<std::collections::BTreeMap<String, String>>::new()",
            Kind::Shell => "crow_runtime::core::Shell { status: -1, stdout: String::new(), stderr: String::new() }",
            Kind::Reply => "crow_runtime::sources::Reply::default()",
            Kind::Answer => "crow_runtime::sys::Answer::default()",
            Kind::Response => RESPONSE.default,
            Kind::Socket => SOCKET.default,
            Kind::Mailer => MAILER.default,
            // taken, never given; or answered by the backends' own arms
            Kind::FnOf(_) | Kind::Walked | Kind::Request | Kind::Db | Kind::Tx | Kind::Found => "unreachable!()",
            Kind::Shown | Kind::Encoded | Kind::Shows | Kind::ShownDyn | Kind::Operands | Kind::Operand | Kind::Scalars | Kind::Scalar => "unreachable!()",
            Kind::JsonList => "Vec::<serde_json::Value>::new()",
            // a generic answer never fails as a value (its rows answer or stop)
            Kind::List | Kind::Seq | Kind::Item | Kind::ItemRef => "Vec::new()",
            Kind::MapOf | Kind::MapRef | Kind::TextMap => "std::collections::BTreeMap::new()",
        }
    }
}

/// The arguments grouped by parameter: one each, and a rest parameter the
/// rest (the same grouping both modes use: `groups` for the interpreter's
/// values, [`print`] for the compiled program's text).
pub fn spans(params: &[Kind], argc: usize, name: &str) -> Result<Vec<(usize, usize)>, String> {
    let fixed = params.iter().filter(|k| !k.is_rest()).count();
    let rest = params.iter().any(|k| k.is_rest());
    if (!rest && argc != fixed) || (rest && argc < fixed) {
        let wanted = if rest { format!("at least {}", fixed) } else { fixed.to_string() };
        return Err(miscount(name, &wanted, argc));
    }
    let mut at = 0;
    let mut out = Vec::with_capacity(params.len());
    for k in params {
        let n = if k.is_rest() { argc - fixed } else { 1 };
        out.push((at, at + n));
        at += n;
    }
    Ok(out)
}

/// A text literal as the emitter prints it, `"…".to_string()`, as the
/// literal itself: a parameter that reads text takes `&str`, and the
/// String allocated for it on every call (`split[row, '\t']` a million
/// times) was never kept.
pub fn literal_text(arg: &str) -> Option<String> {
    let inner = arg.strip_prefix('"')?.strip_suffix("\".to_string()")?;
    // one literal, not an expression that begins and ends with one
    let mut escaped = false;
    for c in inner.chars() {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return None,
            _ => escaped = false,
        }
    }
    Some(format!("\"{}\"", inner))
}

/// An indexed read of a list as the emitter prints it — `{ let _i = …;
/// let _v = &xs; _v[…].clone() }` — with the item referenced instead of
/// cloned: a parameter that only reads it needs no copy of it. Only of a
/// list a name holds (a local, a field of one, a lent parameter, the
/// payload of a value that can fail — `place_text`): a list an expression
/// makes dies with the block, and a reference into it would not build.
pub fn borrowed_index(arg: &str) -> Option<String> {
    if !arg.starts_with("{ let _i = ") {
        return None;
    }
    let body = arg.strip_suffix(".clone() }")?;
    let at = body.rfind("; _v[")?;
    let held = body[..at].rfind("; let _v = &").map(|k| &body[k + 12..at])?;
    if !place_text(held) {
        return None;
    }
    Some(format!("{}; &_v[{} }}", &body[..at], &body[at + 5..]))
}

/// Whether printed Rust names a place a name holds: a local, a field of
/// one, a parameter the caller lends (`(*rows)`), the payload of a value
/// that can fail (`(rows).value`) — no call, so no temporary, and no name
/// of the emitter's own (`_t4`: a temporary that can end before the read).
fn place_text(s: &str) -> bool {
    let b = s.as_bytes();
    let base = s.trim_start_matches(['(', '*']);
    !s.is_empty()
        && !base.starts_with('_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '(' | ')' | '*'))
        && b.iter().enumerate().all(|(i, &c)| match c {
            b'(' => i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'),
            b'*' => i > 0 && b[i - 1] == b'(',
            _ => true,
        })
        && s.matches('(').count() == s.matches(')').count()
}

/// The kind each of `argc` arguments is read as by these parameters, or
/// none when the count does not fit them (the lowering converts an
/// argument to its kind: a list of numbers or a column to a tensor).
pub fn kinds_of(params: &[Kind], argc: usize) -> Option<Vec<Kind>> {
    let spans = spans(params, argc, "").ok()?;
    Some(params.iter().zip(spans).flat_map(|(k, (a, b))| std::iter::repeat_n(k.each(), b - a)).collect())
}


/// More parameters than any row has (a unit test holds every row to it —
/// `core.request` has nine; the first bound, eight, had been read off the
/// `row!` lines and cut its last argument off): the
/// interpreter's bridges group a call's arguments in a fixed array, so no
/// call allocates for it — two vectors a call had been, on every builtin
/// call, and the system allocator's free reads the clock.
pub const MOST_PARAMS: usize = 12;

/// `spans`, in a fixed array: the groups' bounds and how many there are.
#[inline]
fn spans_fixed(params: &[Kind], argc: usize, name: &str) -> Result<([(usize, usize); MOST_PARAMS], usize), String> {
    let fixed = params.iter().filter(|k| !k.is_rest()).count();
    let rest = params.iter().any(|k| k.is_rest());
    if (!rest && argc != fixed) || (rest && argc < fixed) {
        let wanted = if rest { format!("at least {}", fixed) } else { fixed.to_string() };
        return Err(miscount(name, &wanted, argc));
    }
    let mut out = [(0, 0); MOST_PARAMS];
    let mut at = 0;
    for (slot, k) in out.iter_mut().zip(params) {
        let n = if k.is_rest() { argc - fixed } else { 1 };
        *slot = (at, at + n);
        at += n;
    }
    Ok((out, params.len().min(MOST_PARAMS)))
}

/// An int past 64 bits handed where a row takes 64 bits, refused where it is
/// handed in the words the compiled program's conversion says
/// (`ir::Who::Row`): the interpreter's ints grow anywhere, so its rows meet
/// what the compiled program converts before the call. A list is looked
/// into only where its items are ints the row reads as 64 bits.
#[inline]
fn no_big_taken(params: &[Kind], spans: &[(usize, usize)], args: &[CrowValue], name: &str) -> Result<(), String> {
    for (k, (a, b)) in params.iter().zip(spans) {
        let each = k.each();
        if each.takes_big() != TakesBig::Refused {
            continue;
        }
        for v in &args[*a..*b] {
            let big = match &v.value {
                Value::Big(n) => Some(crow_runtime::int::Int::Big(n.clone())),
                Value::Array(items) if matches!(each, Kind::Ints | Kind::Cells | Kind::Keys) => items.iter().find_map(|i| if let Value::Big(n) = &i.value { Some(crow_runtime::int::Int::Big(n.clone())) } else { None }),
                _ => None,
            };
            if let Some(n) = big {
                return Err(crow_runtime::int::past_64_bits(&crate::ir::Who::row_text(name), &n.brief()));
            }
        }
    }
    Ok(())
}

/// `groups`, without allocating: the groups in a fixed array (empty past
/// the row's parameters).
#[inline]
pub fn groups_in_place<'a>(params: &[Kind], args: &'a [CrowValue], name: &str) -> Result<[&'a [CrowValue]; MOST_PARAMS], String> {
    let (spans, n) = spans_fixed(params, args.len(), name)?;
    no_big_taken(params, &spans[..n], args, name)?;
    let mut out: [&'a [CrowValue]; MOST_PARAMS] = [&[]; MOST_PARAMS];
    for (slot, (a, b)) in out.iter_mut().zip(spans).take(n) {
        *slot = &args[a..b];
    }
    Ok(out)
}

/// `groups_mut`, without allocating.
#[inline]
pub fn groups_mut_in_place<'a>(params: &[Kind], args: &'a mut [CrowValue], name: &str) -> Result<[&'a mut [CrowValue]; MOST_PARAMS], String> {
    let (spans, n) = spans_fixed(params, args.len(), name)?;
    no_big_taken(params, &spans[..n], args, name)?;
    let mut out: [&'a mut [CrowValue]; MOST_PARAMS] = Default::default();
    let mut rest = args;
    for (slot, (a, b)) in out.iter_mut().zip(spans).take(n) {
        let (group, tail) = std::mem::take(&mut rest).split_at_mut(b - a);
        *slot = group;
        rest = tail;
    }
    Ok(out)
}



/// A row's call as the compiled program prints it: each argument as its
/// kind takes it, the failure as the row says. `line` is the stop's line
/// (a module's code carries `modules::STOP_LINE` for the emitter to fill),
/// `trace` where an error value is made (none for a module's function,
/// whose error values carry no trace in either mode).
pub fn print(row: &Row, args: &[String], line: &str, trace: Option<&str>) -> Result<String, String> {
    let mut printed = Vec::with_capacity(row.params.len());
    for (k, (a, b)) in row.params.iter().zip(spans(row.params, args.len(), row.name)?) {
        if k.is_rest() {
            printed.push(format!("&[{}]", args[a..b].iter().map(|x| k.print(x, line)).collect::<Vec<_>>().join(", ")));
        } else {
            printed.push(k.print(&args[a], line));
        }
    }
    if row.is_crow() {
        return Err(format!("a bug in Crow: `{}` is written in Crow, and its call was left to the runtime (an argument's kind not known before the program runs)", row.name));
    }
    let call = format!("{}({})", row.rust_path(), printed.join(", "));
    let captured = if stamps(row.name) { "crow_runtime::moments::now()" } else { "0.0" };
    Ok(match row.fails {
        Fails::Never => call,
        Fails::Stops => format!("{}.unwrap_or_else(|e| crow_stop({}, e))", call, line),
        Fails::Value(kind) => format!(
            "match {} {{ Ok(_v) => CrowResult {{ ok: true, value: _v, reason: String::new(), kind: String::new(), trace: vec![], captured: {} }}, Err(_e) => CrowResult {{ ok: false, value: {}, reason: _e, kind: \"{}\".to_string(), trace: vec![{}], captured: 0.0 }} }}",
            call,
            captured,
            row.ret.default_rust(),
            kind,
            trace.unwrap_or("")
        ),
    })
}

// ── the interpreter's reading and writing, one per kind ─────────────

fn named(v: &CrowValue) -> &'static str {
    v.value.type_name()
}

pub fn read_int(v: &CrowValue) -> Result<i64, String> {
    match &v.value {
        Value::Int(n) => Ok(*n),
        Value::Float(f) => Ok(*f as i64),
        _ => Err(format!("expected an int, got {}", named(v))),
    }
}

pub fn read_float(v: &CrowValue) -> Result<f64, String> {
    match &v.value {
        Value::Float(f) => Ok(*f),
        Value::Int(n) => Ok(*n as f64),
        Value::Big(n) => Ok(crow_runtime::int::Int::Big(n.clone()).to_f64()),
        _ => Err(format!("expected a number, got {}", named(v))),
    }
}

pub fn read_moment(v: &CrowValue) -> Result<crow_runtime::moments::Moment, String> {
    match &v.value {
        Value::Moment(m) => Ok(*m),
        _ => Err(format!("expected a moment, got {}", named(v))),
    }
}

pub fn write_moment(m: crow_runtime::moments::Moment) -> CrowValue {
    CrowValue::ok(Value::Moment(m))
}

pub fn read_bool(v: &CrowValue) -> Result<bool, String> {
    match &v.value {
        Value::Bool(b) => Ok(*b),
        _ => Err(format!("expected a bool, got {}", named(v))),
    }
}

pub fn read_str(v: &CrowValue) -> Result<&str, String> {
    match &v.value {
        Value::Str(s) => Ok(s.as_str()),
        _ => Err(format!("expected a string, got {}", named(v))),
    }
}

pub fn read_tensor(v: &CrowValue) -> Result<Cow<'_, CrowTensor>, String> {
    match &v.value {
        Value::NumArray(t) => Ok(Cow::Borrowed(t.as_ref())),
        // a list of rows is a matrix, as a list of numbers is a vector
        Value::Array(items) if !items.is_empty() && items.iter().all(|i| i.is_ok() && matches!(i.value, Value::NumArray(_))) => {
            let rows: Vec<CrowTensor> = items.iter().filter_map(|i| if let Value::NumArray(t) = &i.value { Some(t.as_ref().clone()) } else { None }).collect();
            CrowTensor::from_elements(&rows).map(Cow::Owned)
        }
        Value::Array(items) => {
            let mut data = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                // a failed item holds a placeholder, never a number to read
                if item.is_error() {
                    return Err(if item.kind() == "missing" { crow_runtime::stops::missing_cell(i) } else { format!("item {} of the list failed: {}", i, item.reason()) });
                }
                data.push(read_float(item)?);
            }
            let n = data.len();
            Ok(Cow::Owned(CrowTensor::from_vec(data, vec![n])))
        }
        _ => Err(format!("expected numbers, got {}", named(v))),
    }
}

pub fn write_int(n: i64) -> CrowValue {
    CrowValue::ok(Value::Int(n))
}

/// An exact decimal (an int is the decimal it is).
pub fn read_decimal(v: &CrowValue) -> Result<crow_runtime::decimal::Dec, String> {
    match &v.value {
        Value::Decimal(d) => Ok((**d).clone()),
        other => match Value::int_of(other) {
            Some(n) => Ok(crow_runtime::decimal::Dec::of_int(&n)),
            None => Err(format!("expected a decimal, got {}", named(v))),
        },
    }
}

pub fn write_decimal(d: crow_runtime::decimal::Dec) -> CrowValue {
    CrowValue::ok(Value::Decimal(std::sync::Arc::new(d)))
}

/// An exact ratio (an int or a decimal is the fraction it is).
pub fn read_fraction(v: &CrowValue) -> Result<crow_runtime::fraction::Frac, String> {
    match &v.value {
        Value::Fraction(q) => Ok((**q).clone()),
        Value::Decimal(d) => Ok(crow_runtime::fraction::Frac::of_dec(d)),
        other => match Value::int_of(other) {
            Some(n) => Ok(crow_runtime::fraction::Frac::of_int(&n)),
            None => Err(format!("expected a fraction, got {}", named(v))),
        },
    }
}

pub fn write_fraction(q: crow_runtime::fraction::Frac) -> CrowValue {
    CrowValue::ok(Value::Fraction(std::sync::Arc::new(q)))
}

/// A whole number of any size.
pub fn read_big(v: &CrowValue) -> Result<crow_runtime::int::Int, String> {
    Value::int_of(&v.value).ok_or_else(|| format!("expected an int, got {}", named(v)))
}

pub fn write_float(f: f64) -> CrowValue {
    CrowValue::ok(Value::Float(f))
}

pub fn write_bool(b: bool) -> CrowValue {
    CrowValue::ok(Value::Bool(b))
}

pub fn write_str(s: String) -> CrowValue {
    CrowValue::ok(Value::Str(s))
}

pub fn write_tensor(t: CrowTensor) -> CrowValue {
    CrowValue::ok(Value::NumArray(Box::new(t)))
}

pub fn write_frame(f: crow_runtime::frame::CrowFrame) -> CrowValue {
    CrowValue::ok(Value::Frame(Box::new(f)))
}

pub fn write_tensors(ts: Vec<CrowTensor>) -> CrowValue {
    CrowValue::ok(Value::Array(std::sync::Arc::new(ts.into_iter().map(write_tensor).collect())))
}

pub fn write_unit(_: ()) -> CrowValue {
    CrowValue::done()
}

pub fn write_sparse(m: crow_runtime::sparse::CrowSparse) -> CrowValue {
    CrowValue::ok(Value::Sparse(Box::new(m)))
}

pub fn read_sparse(v: &CrowValue) -> Result<&crow_runtime::sparse::CrowSparse, String> {
    match &v.value {
        Value::Sparse(m) => Ok(m.as_ref()),
        _ => Err(format!("expected a sparse matrix, got {}", named(v))),
    }
}

pub fn read_frame(v: &CrowValue) -> Result<&crow_runtime::frame::CrowFrame, String> {
    match &v.value {
        Value::Frame(f) => Ok(f.as_ref()),
        _ => Err(format!("expected a frame, got {}", named(v))),
    }
}

pub fn read_strs(v: &CrowValue) -> Result<Vec<String>, String> {
    match &v.value {
        Value::Array(items) => items.iter().map(|i| read_str(i).map(|s| s.to_string())).collect(),
        _ => Err(format!("expected a list of text, got {}", named(v))),
    }
}

/// A frame's column as the interpreter holds one: its cells as a list, a
/// missing one an error of reason `missing` holding the column's
/// placeholder, as `df.col` reads them (ops::frame_cell).
pub fn write_column(c: crow_runtime::frame::CrowColumn) -> CrowValue {
    CrowValue::ok(Value::Array(std::sync::Arc::new((0..c.len()).map(|i| crate::ops::column_cell(&c.data, c.valid[i], i)).collect())))
}

/// A mask answered: a list of bools.
pub fn write_bools(bs: Vec<bool>) -> CrowValue {
    CrowValue::ok(Value::Array(std::sync::Arc::new(bs.into_iter().map(|b| CrowValue::ok(Value::Bool(b))).collect())))
}

pub fn write_strs(ss: Vec<String>) -> CrowValue {
    CrowValue::ok(Value::Array(std::sync::Arc::new(ss.into_iter().map(write_str).collect())))
}

/// A mask: a list of bools.
pub fn read_bools(v: &CrowValue) -> Result<Vec<bool>, String> {
    match &v.value {
        Value::Array(items) => items.iter().map(read_bool).collect(),
        _ => Err(format!("expected a list of bools, got {}", named(v))),
    }
}

/// A scalar, as the compiled program's `Cell::from` reads its typed value:
/// a number, text or a bool; a parsed JSON value's `null`, list or map is
/// named for the builtin to refuse.
pub fn read_cell(v: &CrowValue) -> Result<crow_runtime::cell::Cell, String> {
    use crow_runtime::cell::Cell;
    match &v.value {
        Value::Moment(m) => Ok(crow_runtime::cell::Cell::Moment(*m)),
        Value::Int(n) => Ok(Cell::Int(*n)),
        Value::Float(x) => Ok(Cell::Float(*x)),
        Value::Bool(b) => Ok(Cell::Bool(*b)),
        Value::Str(s) => Ok(Cell::Text(s.clone())),
        Value::Done => Ok(Cell::Null),
        Value::Array(_) => Ok(Cell::Other("a list")),
        Value::Map(_) => Ok(Cell::Other("a map")),
        // a parsed value, as the compiled program reads its JSON
        Value::Json(j) => Ok(match &**j {
            serde_json::Value::Array(_) => Cell::Other("a list"),
            serde_json::Value::Object(_) => Cell::Other("a map"),
            scalar => Cell::from(scalar.clone()),
        }),
        _ => Err(format!("expected a number, text or a bool, got {}", named(v))),
    }
}

/// A list's items as the text `join` writes them, as the compiled
/// program's `Words` reads its typed list.
pub fn read_words(v: &CrowValue) -> Result<Vec<String>, String> {
    match &v.value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(n, i)| match &i.value {
                // a failed item holds a placeholder, never a word
                _ if i.is_error() => Err(if i.kind() == "missing" { crow_runtime::stops::missing_cell(n) } else { format!("item {} of the list failed: {}", n, i.reason()) }),
                Value::Str(s) => Ok(s.clone()),
                Value::Int(n) => Ok(n.to_string()),
                Value::Float(x) => Ok(x.to_string()),
                Value::Bool(b) => Ok(b.to_string()),
                Value::Moment(m) => Ok(m.to_string()),
                _ => Err(format!("expected a list of numbers, text, bools or moments, got a list of {}", named(i))),
            })
            .collect(),
        Value::NumArray(t) => Ok(t.to_vec().iter().map(|x| x.to_string()).collect()),
        _ => Err(format!("expected a list, got {}", named(v))),
    }
}

/// An int only.
pub fn read_whole(v: &CrowValue) -> Result<i64, String> {
    match &v.value {
        Value::Int(n) => Ok(*n),
        _ => Err(format!("expected an int, got {}", named(v))),
    }
}

/// A column's values, as the compiled program's `Cells::from` reads its
/// typed value: a list of one kind (ints beside floats make floats), a
/// tensor's numbers, a frame's column — whose missing cells, errors of
/// reason `missing` here, stay missing — or a cell for every row.
pub fn read_cells(v: &CrowValue) -> Result<crow_runtime::frames::Cells, String> {
    use crow_runtime::frame::Column;
    use crow_runtime::frames::Cells;
    let items = match &v.value {
        Value::Array(items) => items,
        Value::NumArray(t) => return Ok(Cells::from(t.as_ref().clone())),
        _ => return read_cell(v).map(Cells::One),
    };
    // every cell says the column's kind, a missing one by the placeholder
    // it holds (ops::frame_cell), so a column all of whose cells are missing
    // keeps its kind, as the compiled program's column does
    let valid: Vec<bool> = items.iter().map(|i| !i.is_error()).collect();
    let all = |f: fn(&Value) -> bool| !items.is_empty() && items.iter().all(|i| f(&i.value));
    let column = if all(|v| matches!(v, Value::Moment(_))) {
        let moments: Vec<crow_runtime::moments::Moment> = items.iter().map(|i| if let Value::Moment(m) = i.value { m } else { crow_runtime::moments::Moment::default() }).collect();
        Column::DateTime(moments.iter().map(|m| m.ms).collect(), moments.iter().map(|m| m.offset).collect())
    } else if all(|v| matches!(v, Value::Int(_))) {
        Column::Int(items.iter().map(|i| if let Value::Int(n) = i.value { n } else { 0 }).collect())
    } else if all(|v| matches!(v, Value::Int(_) | Value::Float(_))) {
        Column::Float(items.iter().map(|i| match i.value { Value::Int(n) => n as f64, Value::Float(x) => x, _ => f64::NAN }).collect())
    } else if all(|v| matches!(v, Value::Bool(_))) {
        Column::Bool(items.iter().map(|i| matches!(i.value, Value::Bool(true))).collect())
    } else if items.iter().all(|i| matches!(i.value, Value::Str(_))) {
        Column::Str(items.iter().map(|i| if let Value::Str(s) = &i.value { s.clone() } else { String::new() }).collect())
    } else {
        let other = items.iter().find(|i| !matches!(i.value, Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Str(_) | Value::Moment(_)));
        return Err(format!("expected a column's values, got a list of {}", other.map(named).unwrap_or("mixed values")));
    };
    Ok(Cells::Column(column, crow_runtime::frame::Validity::from(valid)))
}

/// The lists of text of a group, each a list.
pub fn read_lists(group: &[CrowValue]) -> Result<Vec<Vec<String>>, String> {
    group.iter().map(read_strs).collect()
}

/// A list of tensors: a decomposition's parts, a complex number.
pub fn read_tensors(v: &CrowValue) -> Result<Vec<CrowTensor>, String> {
    match &v.value {
        Value::Array(items) => items.iter().map(|i| read_tensor(i).map(|t| t.into_owned())).collect(),
        _ => Err(format!("expected a list of tensors, got {}", named(v))),
    }
}

/// A list's items, the interpreter's values (a failed item is kept as the
/// item it is, as the compiled program keeps a list's results).
pub fn read_seq(v: &CrowValue) -> Result<&[CrowValue], String> {
    match &v.value {
        Value::Array(items) => Ok(items.as_slice()),
        _ => Err(format!("expected a list, got {}", named(v))),
    }
}

/// A list the function makes its answer of.
pub fn read_list(v: &CrowValue) -> Result<Vec<CrowValue>, String> {
    read_seq(v).map(|items| items.to_vec())
}

/// A map's entries, the interpreter's values.
pub fn read_map_ref(v: &CrowValue) -> Result<&BTreeMap<String, CrowValue>, String> {
    match &v.value {
        Value::Map(m) => Ok(m.as_ref()),
        _ => Err(format!("expected a map, got {}", named(v))),
    }
}

/// A map's values as text, each as it shows.
pub fn read_text_map(v: &CrowValue) -> Result<BTreeMap<String, String>, String> {
    use crow_runtime::display::CrowDisplay;
    Ok(read_map_ref(v)?.iter().map(|(k, v)| (k.clone(), v.crow_fmt())).collect())
}

/// A map the function makes its answer of.
pub fn read_map_of(v: &CrowValue) -> Result<BTreeMap<String, CrowValue>, String> {
    read_map_ref(v).cloned()
}

/// A list's items, each read by `read`: a failed item is the reason the
/// list has no such items (a missing cell's, as a column's).
fn read_items<T>(v: &CrowValue, what: &str, read: fn(&CrowValue) -> Result<T, String>) -> Result<Vec<T>, String> {
    let items = read_seq(v).map_err(|_| format!("expected {}, got {}", what, named(v)))?;
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        if item.is_error() {
            return Err(if item.kind() == "missing" { crow_runtime::stops::missing_cell(i) } else { format!("item {} of the list failed: {}", i, item.reason()) });
        }
        out.push(read(item)?);
    }
    Ok(out)
}

/// A list of ints.
pub fn read_ints(v: &CrowValue) -> Result<Vec<i64>, String> {
    read_items(v, "a list of ints", read_whole)
}

/// A list of floats.
pub fn read_floats(v: &CrowValue) -> Result<Vec<f64>, String> {
    read_items(v, "a list of numbers", read_float)
}

/// A parsed JSON value: the interpreter holds what it parsed to, whose
/// own row it picks, so it reads one only for a row it cannot pick.
/// A JSON argument: a parsed value as it is, borrowed; a value the program
/// built (a list, a map) as its JSON.
pub fn read_json(v: &CrowValue) -> Result<std::borrow::Cow<'_, serde_json::Value>, String> {
    match &v.value {
        Value::Json(j) if v.is_ok() => Ok(std::borrow::Cow::Borrowed(&**j)),
        _ => crate::builtins::crow_to_json("", v).map(std::borrow::Cow::Owned),
    }
}

/// A frame's column: the interpreter holds its cells as a list, whose own
/// row it picks.
/// A frame's column as the interpreter holds it — its cells, a missing one
/// an error — read as the column the runtime's rows take (`len` of a
/// column; it had been refused: "expected a frame's column, got array").
pub fn read_column(v: &CrowValue) -> Result<crow_runtime::frame::CrowColumn, String> {
    match read_cells(v)? {
        crow_runtime::frames::Cells::Column(data, valid) => Ok(crow_runtime::frame::CrowColumn { data: std::sync::Arc::new(data), valid: std::sync::Arc::new(valid) }),
        crow_runtime::frames::Cells::One(_) => Err(format!("expected a frame's column, got {}", named(v))),
    }
}

/// Bytes, as `sys` reads and writes them.
pub fn read_bytes(v: &CrowValue) -> Result<crow_runtime::sys::Bytes, String> {
    match &v.value {
        Value::Bytes(b) => Ok(b.clone()),
        _ => Err(format!("expected bytes, got {}", named(v))),
    }
}

/// A list of bytes values, as a connection's chunks are gathered.
pub fn read_chunks(v: &CrowValue) -> Result<Vec<crow_runtime::sys::Bytes>, String> {
    match &v.value {
        Value::Array(items) => items.iter().map(read_bytes).collect(),
        _ => Err(format!("expected a list of bytes, got {}", named(v))),
    }
}

pub fn write_bytes(b: crow_runtime::sys::Bytes) -> CrowValue {
    CrowValue::ok(Value::Bytes(b))
}

/// A connection `sys.open` made.
pub fn read_stream(v: &CrowValue) -> Result<crow_runtime::sys::CrowStream, String> {
    match &v.value {
        Value::Stream(s) => Ok(s.clone()),
        _ => Err(format!("expected a stream, got {}", named(v))),
    }
}

pub fn write_stream(s: crow_runtime::sys::CrowStream) -> CrowValue {
    CrowValue::ok(Value::Stream(s))
}

pub fn read_socket(v: &CrowValue) -> Result<crow_runtime::sys::Socket, String> {
    match &v.value {
        Value::Socket(s) => Ok(s.clone()),
        _ => Err(format!("expected a socket, got {}", named(v))),
    }
}

pub fn write_socket(s: crow_runtime::sys::Socket) -> CrowValue {
    CrowValue::ok(Value::Socket(s))
}

pub fn read_mailer(v: &CrowValue) -> Result<crow_runtime::sys::Mailer, String> {
    match &v.value {
        Value::Mailer(m) => Ok(m.clone()),
        _ => Err(format!("expected a mailer, got {}", named(v))),
    }
}

pub fn write_mailer(m: crow_runtime::sys::Mailer) -> CrowValue {
    CrowValue::ok(Value::Mailer(m))
}

pub fn write_list(items: Vec<CrowValue>) -> CrowValue {
    CrowValue::ok(Value::Array(std::sync::Arc::new(items)))
}

pub fn write_map_of(m: BTreeMap<String, CrowValue>) -> CrowValue {
    CrowValue::ok(Value::map(m))
}

/// A list of whole numbers of any size.
pub fn read_bigs(v: &CrowValue) -> Result<Vec<crow_runtime::int::Int>, String> {
    read_items(v, "a list of ints", read_big)
}

pub fn write_bigs(ns: Vec<crow_runtime::int::Int>) -> CrowValue {
    CrowValue::ok(Value::Array(std::sync::Arc::new(ns.into_iter().map(|n| CrowValue::ok(Value::of_int(n))).collect())))
}

pub fn write_ints(ns: Vec<i64>) -> CrowValue {
    CrowValue::ok(Value::Array(std::sync::Arc::new(ns.into_iter().map(write_int).collect())))
}

pub fn write_floats(xs: Vec<f64>) -> CrowValue {
    CrowValue::ok(Value::Array(std::sync::Arc::new(xs.into_iter().map(write_float).collect())))
}

/// A request's answer as the interpreter's record: its status, body and
/// headers, as the compiled program's `CrowReply` holds them.
pub fn write_reply(r: crow_runtime::sources::Reply) -> CrowValue {
    let mut m = BTreeMap::new();
    m.insert("status".to_string(), write_int(r.status));
    m.insert("body".to_string(), write_str(r.body));
    let headers: BTreeMap<String, CrowValue> = r.headers.into_iter().map(|(k, v)| (k, write_str(v))).collect();
    m.insert("headers".to_string(), CrowValue::ok(Value::map(headers)));
    CrowValue::ok(Value::map(m))
}

/// A connection's answer as the interpreter's record: `status`, `phrase`,
/// `headers`, `body` (bytes).
pub fn write_answer(a: crow_runtime::sys::Answer) -> CrowValue {
    let mut m = BTreeMap::new();
    m.insert("status".to_string(), write_int(a.status));
    m.insert("phrase".to_string(), write_str(a.phrase));
    let headers: BTreeMap<String, CrowValue> = a.headers.into_iter().map(|(k, v)| (k, write_str(v))).collect();
    m.insert("headers".to_string(), CrowValue::ok(Value::map(headers)));
    m.insert("body".to_string(), write_bytes(a.body));
    CrowValue::ok(Value::map(m))
}

/// A command's answer as the interpreter's record: `status`, `stdout`,
/// `stderr`.
pub fn write_response(r: crow_web::core::CrowResponse) -> CrowValue {
    CrowValue::ok(Value::Response(Box::new(r)))
}

pub fn write_shell(sh: crow_runtime::core::Shell) -> CrowValue {
    let mut m = BTreeMap::new();
    m.insert("status".to_string(), write_int(sh.status));
    m.insert("stdout".to_string(), write_str(sh.stdout));
    m.insert("stderr".to_string(), write_str(sh.stderr));
    CrowValue::ok(Value::map(m))
}

/// A parsed JSON value as the interpreter holds it: arrays, maps and
/// scalars, a `null` inside an error value of reason `null`.
/// A JSON answer, kept as the compiled program keeps it.
pub fn write_json(v: serde_json::Value) -> CrowValue {
    CrowValue::ok(Value::Json(std::sync::Arc::new(v)))
}

pub fn write_json_list(vs: Vec<serde_json::Value>) -> CrowValue {
    CrowValue::ok(Value::Array(std::sync::Arc::new(vs.into_iter().map(write_json).collect())))
}

pub fn write_markup(s: String) -> CrowValue {
    CrowValue::markup(s)
}

pub fn write_text_rows(rows: Vec<BTreeMap<String, String>>) -> CrowValue {
    CrowValue::ok(Value::Array(std::sync::Arc::new(rows.into_iter().map(|r| CrowValue::ok(Value::map(r.into_iter().map(|(k, v)| (k, write_str(v))).collect()))).collect())))
}

/// One argument of a group, the group of a single parameter.
pub fn one(group: &[CrowValue]) -> Result<&CrowValue, String> {
    group.first().ok_or_else(|| "an argument is missing".to_string())
}

/// One argument of a group, to keep: taken out of the arguments.
pub fn one_taken(group: &mut [CrowValue]) -> Result<CrowValue, String> {
    group.first_mut().map(|v| std::mem::replace(v, CrowValue::done())).ok_or_else(|| "an argument is missing".to_string())
}

/// A list the function makes its answer of, moved.
pub fn take_list(v: CrowValue) -> Result<Vec<CrowValue>, String> {
    match v.value {
        Value::Array(items) => Ok(std::sync::Arc::unwrap_or_clone(items)),
        other => Err(format!("expected a list, got {}", other.type_name())),
    }
}

/// A map the function makes its answer of, moved.
pub fn take_map(v: CrowValue) -> Result<BTreeMap<String, CrowValue>, String> {
    match v.value {
        // (the entries moved out where nothing else holds them)
        Value::Map(m) => Ok(std::sync::Arc::unwrap_or_clone(m)),
        other => Err(format!("expected a map, got {}", other.type_name())),
    }
}

/// The sizes of a group, each an int.
pub fn read_sizes(group: &[CrowValue]) -> Result<Vec<i64>, String> {
    group.iter().map(read_int).collect()
}

/// A rest parameter's tensors, each read where it lies.
pub fn read_groups(group: &[CrowValue]) -> Result<Vec<Cow<'_, CrowTensor>>, String> {
    group.iter().map(read_tensor).collect()
}

/// A fused chain's operands: each a tensor, or a number.
pub fn read_operands(group: &[CrowValue]) -> Result<Vec<crow_runtime::fused::Operand<'_>>, String> {
    group
        .iter()
        .map(|v| match &v.value {
            Value::NumArray(t) => Ok(crow_runtime::fused::Operand::Tensor(t.as_ref())),
            Value::Int(n) => Ok(crow_runtime::fused::Operand::Number(*n as f64)),
            Value::Big(n) => Ok(crow_runtime::fused::Operand::Number(crow_runtime::int::Int::Big(n.clone()).to_f64())),
            Value::Float(x) => Ok(crow_runtime::fused::Operand::Number(*x)),
            _ => Err(format!("expected a tensor or a number, got {}", named(v))),
        })
        .collect()
}

/// A fused mask's values: each a number, text or a bool.
pub fn read_scalars(group: &[CrowValue]) -> Result<Vec<crow_runtime::mask::Cell<'_>>, String> {
    use crow_runtime::mask::Cell;
    group
        .iter()
        .map(|v| match &v.value {
            Value::Int(n) => Ok(Cell::Number(*n as f64)),
            Value::Float(x) => Ok(Cell::Number(*x)),
            Value::Str(s) => Ok(Cell::Text(s)),
            Value::Bool(b) => Ok(Cell::Bool(*b)),
            _ => Err(format!("expected a number, text or a bool, got {}", named(v))),
        })
        .collect()
}

/// The reading of a parameter's group by its kind, as the function takes it.
macro_rules! read {
    (Int, $g:expr) => { read_int(one($g)?)? };
    (Whole, $g:expr) => { read_whole(one($g)?)? };
    (Big, $g:expr) => { &read_big(one($g)?)? };
    (Decimal, $g:expr) => { &read_decimal(one($g)?)? };
    (Fraction, $g:expr) => { &read_fraction(one($g)?)? };
    (Words, $g:expr) => { &read_words(one($g)?)? };
    (Float, $g:expr) => { read_float(one($g)?)? };
    (Moment, $g:expr) => { read_moment(one($g)?)? };
    (Bool, $g:expr) => { read_bool(one($g)?)? };
    (Str, $g:expr) => { read_str(one($g)?)? };
    (Tensor, $g:expr) => { &*read_tensor(one($g)?)? };
    (Tensors, $g:expr) => { &read_tensors(one($g)?)? };
    (Sparse, $g:expr) => { read_sparse(one($g)?)? };
    (Frame, $g:expr) => { read_frame(one($g)?)? };
    (Strs, $g:expr) => { &read_strs(one($g)?)? };
    (Chunks, $g:expr) => { &read_chunks(one($g)?)? };
    (Sizes, $g:expr) => { &read_sizes($g)? };
    (Groups, $g:expr) => { &read_groups($g)?.iter().map(|t| t.as_ref()).collect::<Vec<&CrowTensor>>() };
    (Bools, $g:expr) => { &read_bools(one($g)?)? };
    (Cell, $g:expr) => { &read_cell(one($g)?)? };
    (Cells, $g:expr) => { read_cells(one($g)?)? };
    (Lists, $g:expr) => { &read_lists($g)?.iter().map(|l| l.as_slice()).collect::<Vec<&[String]>>() };
    (Operands, $g:expr) => { &read_operands($g)? };
    (Scalars, $g:expr) => { &read_scalars($g)? };
    (Shows, $g:expr) => { &$g.iter().map(|v| v as &dyn crow_runtime::display::CrowDisplay).collect::<Vec<&dyn crow_runtime::display::CrowDisplay>>() };
    (List, $g:expr) => { read_list(one($g)?)? };
    (Seq, $g:expr) => { read_seq(one($g)?)? };
    (Item, $g:expr) => { one($g)?.clone() };
    (ItemRef, $g:expr) => { one($g)? };
    (MapOf, $g:expr) => { read_map_of(one($g)?)? };
    (MapRef, $g:expr) => { read_map_ref(one($g)?)? };
    (TextMap, $g:expr) => { &read_text_map(one($g)?)? };
    (Ints, $g:expr) => { read_ints(one($g)?)? };
    (Bigs, $g:expr) => { read_bigs(one($g)?)? };
    (Floats, $g:expr) => { read_floats(one($g)?)? };
    (Texts, $g:expr) => { read_strs(one($g)?)? };
    (Json, $g:expr) => { &*read_json(one($g)?)? };
    (Column, $g:expr) => { &read_column(one($g)?)? };
    (Bytes, $g:expr) => { &read_bytes(one($g)?)? };
    (Stream, $g:expr) => { &read_stream(one($g)?)? };
    (Socket, $g:expr) => { &read_socket(one($g)?)? };
    (Mailer, $g:expr) => { &read_mailer(one($g)?)? };
    (Shown, $g:expr) => { one($g)? };
    (Encoded, $g:expr) => { one($g)? };
}

/// The value of an answer by its kind.
macro_rules! answer {
    (Int, $x:expr) => { write_int($x) };
    (Big, $x:expr) => { CrowValue::ok(Value::of_int($x)) };
    (Decimal, $x:expr) => { write_decimal($x) };
    (Fraction, $x:expr) => { write_fraction($x) };
    (Float, $x:expr) => { write_float($x) };
    (Moment, $x:expr) => { write_moment($x) };
    (Bool, $x:expr) => { write_bool($x) };
    (Str, $x:expr) => { write_str($x) };
    (Tensor, $x:expr) => { write_tensor($x) };
    (Frame, $x:expr) => { write_frame($x) };
    (Column, $x:expr) => { write_column($x) };
    (Bytes, $x:expr) => { write_bytes($x) };
    (Stream, $x:expr) => { write_stream($x) };
    (Strs, $x:expr) => { write_strs($x) };
    (Bools, $x:expr) => { write_bools($x) };
    (Tensors, $x:expr) => { write_tensors($x) };
    (Unit, $x:expr) => { write_unit($x) };
    (Sparse, $x:expr) => { write_sparse($x) };
    (List, $x:expr) => { write_list($x) };
    (MapOf, $x:expr) => { write_map_of($x) };
    (TextMap, $x:expr) => { CrowValue::ok(Value::map($x.into_iter().map(|(k, t)| (k, CrowValue::ok(Value::Str(t)))).collect())) };
    (Ints, $x:expr) => { write_ints($x) };
    (Bigs, $x:expr) => { write_bigs($x) };
    (Floats, $x:expr) => { write_floats($x) };
    (Texts, $x:expr) => { write_strs($x) };
    (Markup, $x:expr) => { write_markup($x) };
    (TextRows, $x:expr) => { write_text_rows($x) };
    (Shell, $x:expr) => { write_shell($x) };
    (Reply, $x:expr) => { write_reply($x) };
    (Answer, $x:expr) => { write_answer($x) };
    (Response, $x:expr) => { write_response($x) };
    (Socket, $x:expr) => { write_socket($x) };
    (Mailer, $x:expr) => { write_mailer($x) };
    (Json, $x:expr) => { write_json($x) };
    (JsonList, $x:expr) => { write_json_list($x) };
}

/// The taking of a parameter's group by its kind: a collection the
/// function makes its answer of is moved out of the arguments; any other
/// kind is read as [`read!`] reads it.
macro_rules! take {
    (List, $g:expr) => { take_list(one_taken($g)?)? };
    (MapOf, $g:expr) => { take_map(one_taken($g)?)? };
    (Item, $g:expr) => { one_taken($g)? };
    ($k:ident, $g:expr) => { read!($k, $g) };
}

/// What a failure is, as the row macro writes it.
macro_rules! fails {
    (never) => { Fails::Never };
    (stops) => { Fails::Stops };
    (value $kind:literal) => { Fails::Value($kind) };
}

/// The function's answer as the row says: its value, its reason as a stop
/// (the call's `Err`), or an error value of the row's kind.
macro_rules! respond {
    ($r:ident, $call:expr, $trace:expr, never) => { Ok(answer!($r, $call)) };
    ($r:ident, $call:expr, $trace:expr, stops) => {{
        let got = $call?;
        Ok(answer!($r, got))
    }};
    ($r:ident, $call:expr, $trace:expr, value $kind:literal) => {
        Ok(match $call {
            Ok(got) => answer!($r, got),
            Err(e) => CrowValue::error_with_kind(&e, $kind, $trace),
        })
    };
}

/// A row: its id, Crow name and module, the runtime function, the kinds of
/// its parameters and answer, and what a failure is (`never`, `stops`, or
/// `value "kind"`). The interpreter's call is made from the same tokens the
/// emitter prints, so the two cannot name different functions.
/// The path of a row whose body is Crow (`crow!`).
pub const IN_CROW: &str = "crow";

/// The name of the program's function a Crow row's body is inlined as:
/// the module's path and the name, behind an underscore no program
/// spells (`_math_nn_relu`); `crate::modules::inline` makes them.
pub fn crow_fn_name(row: &Row) -> String {
    format!("_{}_{}", crow_prefix_of(row.module), crow_fn_base(row))
}

/// The prefix a module's functions carry when inlined: the module's path,
/// the core's being `core`.
pub fn crow_prefix_of(module: &str) -> String {
    let module = if module.is_empty() { "core" } else { module };
    module.replace('.', "_")
}

/// The name of the function a Crow row's body is, in the module's source:
/// the row's id after its dot (`read` and `read_asked` share the name
/// `read`, and are two functions).
pub fn crow_fn_base(row: &Row) -> &str {
    row.id.rsplit('.').next().unwrap_or(row.name)
}

/// The Crow row of a module's name that takes these kinds, when it is one.
pub fn crow_row(module: &str, name: &str, args: &[Arg]) -> Option<&'static Row> {
    self::module(module, name, args).filter(|r| r.is_crow() && takes(r, args)).or_else(|| {
        // a core row a module's name reaches too (`io.read` is the core's
        // `read`): the row whose id the module prefixes
        let prefix = format!("{}.", module);
        (!module.is_empty()).then(|| ROWS.iter().find(|r| r.module.is_empty() && r.name == name && r.id.starts_with(&prefix) && r.is_crow() && takes(r, args))).flatten()
    })
}

fn written_in_crow(name: &str) -> String {
    format!("a bug in Crow: `{}` is written in Crow, and was called as a runtime function", name)
}

/// A value as a Crow row's parameter takes it, for the interpreter to hand
/// the row's function a value nothing typed before the program ran: the
/// kinds the standard library's Crow is written over.
pub fn value_as(kind: Kind, v: &CrowValue) -> Result<CrowValue, String> {
    Ok(match kind {
        Kind::Tensor => CrowValue::ok(Value::NumArray(Box::new(read_tensor(v)?.into_owned()))),
        Kind::Float => CrowValue::ok(Value::Float(read_float(v)?)),
        Kind::Moment => CrowValue::ok(Value::Moment(read_moment(v)?)),
        Kind::Int | Kind::Whole => CrowValue::ok(Value::Int(read_int(v)?)),
        Kind::Big => CrowValue::ok(Value::of_int(read_big(v)?)),
        Kind::Decimal => write_decimal(read_decimal(v)?),
        Kind::Fraction => write_fraction(read_fraction(v)?),
        Kind::Str => CrowValue::ok(Value::Str(read_str(v)?.to_string())),
        Kind::Bool => CrowValue::ok(Value::Bool(read_bool(v)?)),
        Kind::Bytes => CrowValue::ok(Value::Bytes(read_bytes(v)?)),
        Kind::Chunks => CrowValue::ok(Value::Array(std::sync::Arc::new(read_chunks(v)?.into_iter().map(write_bytes).collect()))),
        Kind::TextMap => CrowValue::ok(Value::map(read_text_map(v)?.into_iter().map(|(k, t)| (k, CrowValue::ok(Value::Str(t)))).collect())),
        Kind::Frame => CrowValue::ok(Value::Frame(Box::new(read_frame(v)?.clone()))),
        Kind::Strs => CrowValue::ok(Value::Array(std::sync::Arc::new(read_strs(v)?.into_iter().map(|t| CrowValue::ok(Value::Str(t))).collect()))),
        // a value JSON can hold reaches the function as its JSON text
        // (`crow_param_type`): the one form a map, a record or a list of
        // either takes in Crow
        Kind::Encoded => CrowValue::ok(Value::Str(crate::builtins::crow_to_encoded(v).to_string())),
        Kind::Socket => CrowValue::ok(Value::Socket(read_socket(v)?)),
        Kind::Mailer => CrowValue::ok(Value::Mailer(read_mailer(v)?)),
        other => return Err(format!("a bug in Crow: a builtin written in Crow takes {} here, which the runtime cannot hand it", other.words())),
    })
}

/// A Crow row's arguments as its function takes them, for the interpreter:
/// each read as the row's parameter takes it (`value_as`), and a parameter
/// that takes the rest of the arguments (`anova[a, b, c]`) handed them as
/// one list.
pub fn crow_arguments(row: &Row, vals: &[CrowValue]) -> Result<Vec<CrowValue>, String> {
    let spans = spans(row.params, vals.len(), row.name)?;
    let mut handed = Vec::with_capacity(row.params.len());
    for (k, (a, b)) in row.params.iter().zip(spans) {
        if k.is_rest() {
            let items: Vec<CrowValue> = vals[a..b].iter().map(|v| value_as(k.each(), v)).collect::<Result<_, _>>()?;
            handed.push(CrowValue::ok(Value::Array(std::sync::Arc::new(items))));
        } else {
            handed.push(value_as(*k, &vals[a])?);
        }
    }
    Ok(handed)
}

/// The type a parameter of a row written in Crow has, from the row's kind:
/// the kind's own, except that a value JSON can hold (`Encoded`: a map, a
/// record, a list of either) reaches the function as its JSON text — the
/// one form every such value takes in Crow, read back with `json[…]`. The
/// lowering hands the call `encode[value]` (`crow_call_args`) and the
/// interpreter the same (`value_as`).
pub fn crow_param_type(k: Kind) -> Ty {
    match k {
        Kind::Encoded => Ty::Str,
        other => other.ty(),
    }
}

/// A row whose body is a function of the standard library written in Crow
/// (src/modules/*.co; design rule 20): the table states its contract — what
/// it takes, what it answers, how it fails — and both backends call the
/// function the module's source is inlined as (`crow_fn_name`). The
/// interpreter's closures here are never reached: a call whose kinds the
/// lowering did not know picks the row as the program runs and hands the
/// function its values through `value_as`.
macro_rules! crow {
    ($id:literal, $module:literal, $name:literal, ($($k:ident),*) -> $r:ident, $($fails:tt)+) => {
        Row {
            id: $id, name: $name, module: $module, path: IN_CROW,
            params: &[$(Kind::$k),*], ret: Kind::$r, fails: fails!($($fails)+), options: &[], written: "", form: "", props: &[], taken: None, big: None, range: IntRange::I64, shape: Shape::Open, columns: &[], provider: None, feature: None,
            call: |_args: &[CrowValue], _trace: &str| Err(written_in_crow($name)),
            take: |_args: Vec<CrowValue>, _trace: &str| Err(written_in_crow($name)),
        }
    };
}

/// A core row over a map, keyed by text or by ints: the interpreter's
/// call and take are written by hand (`maps`), one branch per key kind,
/// since one closure cannot name two instantiations of the generic
/// function; the compiled program instantiates it by the map's type.
macro_rules! maprow {
    ($id:literal, $name:literal, $f:path, ($($k:ident),*) -> $r:ident, $fails:tt, $call:path, $take:path) => {
        Row {
            id: $id, name: $name, module: "", path: stringify!($f),
            params: &[$(Kind::$k),*], ret: Kind::$r, fails: fails!($fails), options: &[], written: "", form: "", props: &[], taken: None, big: None, range: IntRange::I64, shape: Shape::Open, columns: &[], provider: None, feature: None,
            call: $call,
            take: $take,
        }
    };
}

macro_rules! row {
    ($id:literal, $module:literal, $name:literal, $f:path, ($($k:ident),*) -> $r:ident, $($fails:tt)+) => {
        Row {
            id: $id, name: $name, module: $module, path: stringify!($f),
            params: &[$(Kind::$k),*], ret: Kind::$r, fails: fails!($($fails)+), options: &[], written: "", form: "", props: &[], taken: None, big: None, range: IntRange::I64, shape: Shape::Open, columns: &[], provider: None, feature: None,
            call: |args: &[CrowValue], trace: &str| {
                let _ = trace;
                let groups = groups_in_place(&[$(Kind::$k),*], args, $name)?;
                let mut g = groups.into_iter();
                let mut next = || g.next().unwrap_or(&[]);
                let _ = &mut next;
                respond!($r, $f($(read!($k, next())),*), trace, $($fails)+)
            },
            take: |mut args: Vec<CrowValue>, trace: &str| {
                let _ = trace;
                let groups = groups_mut_in_place(&[$(Kind::$k),*], &mut args, $name)?;
                let mut g = groups.into_iter();
                let mut next = || g.next().unwrap_or(&mut []);
                let _ = &mut next;
                respond!($r, $f($(take!($k, next())),*), trace, $($fails)+)
            },
        }
    };
}

/// The rows. A builtin here is written once, in the runtime; its old arms
/// in `builtins.rs` and `emit_rs/builtins.rs` are gone.
pub static ROWS: &[Row] = &[
    // text: one function each, as both modes already called; a count or a
    // kind no row takes is refused before the program runs
    row!("text.split", "", "split", crow_runtime::text::split, (Str, Str) -> Strs, never),
    row!("text.trim", "", "trim", crow_runtime::text::trim, (Str) -> Str, never),
    // how many times a part occurs (not overlapping: Python's str.count),
    // and each character of one set mapped to the one at its place in
    // another, in one pass (DNA's complement: tr, Python's translate)
    Row { range: IntRange::Count, ..row!("text.occurs", "", "occurs", crow_runtime::text::count, (Str, Str) -> Int, stops) },
    Row { taken: Some("crow_runtime::text::translate_taken"), take: texts::translate_taken, ..row!("text.translate", "", "translate", crow_runtime::text::translate, (Str, Str, Str) -> Str, stops) },
    Row { taken: Some("crow_runtime::text::replace_taken"), props: &[Prop::InPlace], take: texts::replace_taken, ..row!("text.replace", "", "replace", crow_runtime::text::replace, (Str, Str, Str) -> Str, never) },
    Row { taken: Some("crow_runtime::text::upper_taken"), props: &[Prop::InPlace], take: texts::upper_taken, ..row!("text.upper", "", "upper", crow_runtime::text::upper, (Str) -> Str, never) },
    Row { taken: Some("crow_runtime::text::lower_taken"), props: &[Prop::InPlace], take: texts::lower_taken, ..row!("text.lower", "", "lower", crow_runtime::text::lower, (Str) -> Str, never) },
    row!("text.starts", "", "starts", crow_runtime::text::starts, (Str, Str) -> Bool, never),
    row!("text.ends", "", "ends", crow_runtime::text::ends, (Str, Str) -> Bool, never),
    row!("core.join", "", "join", crow_runtime::core::join_words, (Words, Str) -> Str, never),
    // collections: one generic function each over a list's or a map's
    // items, then text's and a tensor's own, a name's rows in the order
    // they are tried (a list reaches its own row before a tensor's). A kind
    // no row takes is refused before the program runs: compiled, `has` of a
    // list and `len` of a number did not build, and `contains` of a record
    // did not compare.
    Row { props: &[Prop::Differentiable], range: IntRange::Count, ..row!("core.len", "", "len", crow_runtime::core::len, (Seq) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Count, ..row!("core.len_text", "", "len", crow_runtime::core::len_text, (Str) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Count, ..maprow!("core.len_map", "len", crow_runtime::core::len_map, (MapRef) -> Int, never, maps::len, maps::len_taken) },
    Row { props: &[Prop::Differentiable], range: IntRange::Count, ..row!("core.len_column", "", "len", crow_runtime::core::len_column, (Column) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Count, ..row!("core.len_tensor", "", "len", crow_runtime::core::len_tensor, (Tensor) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Count, ..row!("core.len_frame", "", "len", crow_runtime::core::len_frame, (Frame) -> Int, never) },
    // a JSON value's kind shows as the program runs: of a number or a bool,
    // the refusal the interpreter gives the value it parsed to (compiled,
    // its length was 0)
    Row { props: &[Prop::Differentiable], ..row!("core.len_json", "", "len", crow_runtime::core::len_json, (Json) -> Int, stops) },
    Row { props: &[Prop::Differentiable], range: IntRange::Count, ..row!("core.len_bytes", "", "len", crow_runtime::sys::len_bytes, (Bytes) -> Int, never) },
    // an item compared as `eq` compares it: a record field by field, a map
    // entry by entry
    row!("core.contains", "", "contains", crow_runtime::core::contains, (Seq, ItemRef) -> Bool, never),
    row!("core.contains_text", "", "contains", crow_runtime::core::contains_text, (Str, Str) -> Bool, never),
    row!("core.contains_number", "", "contains", crow_runtime::core::contains_number, (Tensor, Float) -> Bool, stops),
    // `xs[mask]` and `t[mask]`: the items, the rows, where it is true — the
    // index's own rows, reached by the lowering, never by name
    row!("core.__pick_items", "", "__pick_items", crow_runtime::core::pick_items, (Seq, Bools) -> List, stops),
    row!("core.__pick_rows", "", "__pick_rows", crow_runtime::math::pick_rows, (Tensor, Bools) -> Tensor, stops),
    // (a tensor of two axes or more by a mask of its shape: its numbers)
    row!("core.__pick_numbers", "", "__pick_numbers", crow_runtime::math::pick_numbers, (Tensor, Tensor) -> Tensor, stops),
    row!("core.__pick_cells", "", "__pick_cells", crow_runtime::frames::pick_cells, (Column, Bools) -> Column, stops),
    // the rest of a call's arguments gathered for a function written in
    // Crow, where one of them can fail — reached by the lowering
    row!("core.__sizes", "", "__sizes", crow_runtime::core::sizes, (Sizes) -> Ints, never),
    row!("core.__truncated", "", "__truncated", crow_runtime::core::truncated, (Float) -> Int, never),
    row!("core.__text_map", "", "__text_map", crow_runtime::sources::texts, (TextMap) -> TextMap, never),
    row!("core.__groups", "", "__groups", crow_runtime::core::groups, (Groups) -> Tensors, never),
    // `map[keys, values]`: the literal's other form, reached by the lowering
    maprow!("core.__map_of", "__map_of", crow_runtime::core::map_of, (Keys, Seq) -> MapOf, stops, maps::map_of, maps::map_of_taken),
    Row { taken: Some("crow_runtime::core::slice_taken"), take: lists::slice_taken, ..row!("core.slice", "", "slice", crow_runtime::core::slice, (Seq, Int, Int) -> List, never) },
    row!("core.slice_str", "", "slice", crow_runtime::core::slice_str, (Str, Int, Int) -> Str, never),
    Row { shape: Shape::Like(0, Dim::Open), ..row!("core.slice_tensor", "", "slice", crow_runtime::core::slice_tensor, (Tensor, Int, Int) -> Tensor, never) },
    Row { props: &[Prop::InPlace], ..row!("core.push", "", "push", crow_runtime::core::push, (List, Item) -> List, never) },
    Row { shape: Shape::Like(0, Dim::Open), props: &[Prop::InPlace], ..row!("core.push_row", "", "push", crow_runtime::core::push_row, (Tensor, Tensor) -> Tensor, stops) },
    // (given the tensor — `x = push[move x, n]` — it grows where it lies)
    Row { shape: Shape::Dims(&[Dim::Open]), taken: Some("crow_runtime::core::push_number_taken"), props: &[Prop::InPlace], take: tensors::push_number_taken, ..row!("core.push_number", "", "push", crow_runtime::core::push_number, (Tensor, Float) -> Tensor, stops) },
    // `m[i][j] = x`, as the lowering writes it (`m = __set_at[m, [i, j], x]`):
    // a number of a tensor at one position per axis, written in place
    Row { taken: Some("crow_runtime::core::set_at_taken"), props: &[Prop::InPlace], take: tensors::set_at_taken, ..row!("core.__set_at", "", "__set_at", crow_runtime::core::set_at, (Tensor, Ints, Float) -> Tensor, stops) },
    // `m[i] = row` (`m = __set_part[m, [i], row]`): a part of a tensor — a
    // matrix's row — replaced by one of its own shape, in place
    Row { taken: Some("crow_runtime::core::set_part_taken"), props: &[Prop::InPlace], take: tensors::set_part_taken, ..row!("core.__set_part", "", "__set_part", crow_runtime::core::set_part, (Tensor, Ints, Tensor) -> Tensor, stops) },
    Row { props: &[Prop::InPlace], ..row!("core.pop", "", "pop", crow_runtime::core::pop, (List) -> List, never) },
    Row { shape: Shape::Like(0, Dim::Open), props: &[Prop::InPlace], ..row!("core.pop_tensor", "", "pop", crow_runtime::core::pop_tensor, (Tensor) -> Tensor, never) },
    // numbers ascending with NaN last, text by its characters; a list of
    // anything else has no order (a record's was the interpreter's
    // unchanged list, and the compiled program's missing Ord)
    Row { big: Some("core.__sort_big"), props: &[Prop::InPlace], ..row!("core.sort_ints", "", "sort", crow_runtime::core::sort_ints, (Ints) -> Ints, never) },
    Row { props: &[Prop::InPlace], ..row!("core.sort_floats", "", "sort", crow_runtime::core::sort_floats, (Floats) -> Floats, never) },
    Row { props: &[Prop::InPlace], ..row!("core.sort_strs", "", "sort", crow_runtime::core::sort_strs, (Texts) -> Texts, never) },
    Row { shape: Shape::Same(0), props: &[Prop::InPlace], ..row!("core.sort_tensor", "", "sort", crow_runtime::core::sort_tensor, (Tensor) -> Tensor, stops) },
    Row { props: &[Prop::InPlace], ..row!("core.reverse", "", "reverse", crow_runtime::core::reverse, (List) -> List, never) },
    Row { taken: Some("crow_runtime::core::reverse_text_taken"), props: &[Prop::InPlace], take: texts::reverse_taken, ..row!("core.reverse_text", "", "reverse", crow_runtime::core::reverse_text, (Str) -> Str, never) },
    Row { shape: Shape::Same(0), props: &[Prop::InPlace], ..row!("core.reverse_tensor", "", "reverse", crow_runtime::core::reverse_tensor, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::InPlace], ..row!("core.concat", "", "concat", crow_runtime::core::concat, (List, Seq) -> List, never) },
    Row { props: &[Prop::InPlace], ..row!("core.concat_text", "", "concat", crow_runtime::core::concat_text, (Str, Str) -> Str, never) },
    Row { shape: Shape::Along(0, None), props: &[Prop::InPlace], ..row!("core.concat_tensors", "", "concat", crow_runtime::core::concat_tensors, (Tensor, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Along(0, Some(2)), props: &[Prop::InPlace], options: &[("axis", Preset::Asked)], ..row!("core.concat_axis", "", "concat", crow_runtime::core::concat_axis, (Tensor, Tensor, Int) -> Tensor, stops) },
    // ints only (compiled, a float was cut to an int where the interpreter
    // refused it)
    Row { props: &[Prop::Constructs, Prop::Range], ..row!("core.range_to", "", "range", crow_runtime::core::range_to, (Whole) -> Ints, stops) },
    Row { props: &[Prop::Constructs, Prop::Range], ..row!("core.range_from", "", "range", crow_runtime::core::range_from, (Whole, Whole) -> Ints, stops) },
    Row { props: &[Prop::Constructs, Prop::Range], ..row!("core.range", "", "range", crow_runtime::core::range, (Whole, Whole, Whole) -> Ints, stops) },
    maprow!("core.keys", "keys", crow_runtime::core::keys, (MapRef) -> Keys, never, maps::keys, maps::keys_taken),
    maprow!("core.values", "values", crow_runtime::core::values, (MapRef) -> List, never, maps::values, maps::values_taken),
    maprow!("core.has", "has", crow_runtime::core::has, (MapRef, Key) -> Bool, never, maps::has, maps::has_taken),
    // `key in m`: the map's membership, the same function (the name `has`
    // is the lowering's own; a program writes the operator)
    maprow!("core.contains_key", "contains", crow_runtime::core::has, (MapRef, Key) -> Bool, never, maps::has, maps::has_taken),
    Row { props: &[Prop::InPlace], ..maprow!("core.set", "set", crow_runtime::core::set, (MapOf, Key, Item) -> MapOf, never, maps::set, maps::set_taken) },
    Row { props: &[Prop::InPlace], ..maprow!("core.del", "del", crow_runtime::core::map_del, (MapOf, Key) -> MapOf, never, maps::del, maps::del_taken) },
    // a parsed JSON object's keys, values and entries: the compiled program
    // holds the parsed value, the interpreter what it parsed to, whose map
    // rows it picks (compiled, these did not build)
    row!("json.keys", "", "keys", crow_runtime::json::keys, (Json) -> Strs, stops),
    row!("json.values", "", "values", crow_runtime::json::values, (Json) -> JsonList, stops),
    row!("json.has", "", "has", crow_runtime::json::has, (Json, Str) -> Bool, stops),
    row!("json.contains_key", "", "contains", crow_runtime::json::has, (Json, Str) -> Bool, stops),
    // a value's display and its JSON: one display (crow_runtime::display)
    // and one encoding (crow_runtime::json::ToJson) that both modes' values
    // follow — the compiled program had shown a record holding a list, or a
    // list of records, in Rust's debug form, and `str` of a JSON list as its
    // JSON text; the interpreter had refused to encode a record
    // (a column's cells as text, before the form that shows any value as text)
    row!("frame.str_column", "", "str", crow_runtime::frames::str_column, (Column) -> Column, stops),
    row!("display.str", "", "str", crow_runtime::display::shown, (Shown) -> Str, never),
    row!("json.parse", "", "json", crow_runtime::json::parse, (Str) -> Json, value "parse"),
    row!("json.encode", "", "encode", crow_runtime::json::encode, (Encoded) -> Str, never),
    // an element-wise chain of tensors in one pass, which the lowering writes
    // in place of the chain's operators (lower_ir/fuse.rs): a shape that does
    // not meet stops with the operator's words
    row!("tensor.fused", "", "__fused", crow_runtime::fused::run, (Str, Operands) -> Tensor, stops),
    // text written so a header or a URL can carry it, and read back: a
    // scheme encode does not know is a misuse; text decode cannot read is
    // an error value
    row!("text.encode", "", "encode", crow_runtime::text::encode_as, (Str, Str) -> Str, stops),
    row!("text.decode", "", "decode", crow_runtime::text::decode_as, (Str, Str) -> Str, value "parse"),
    // log: the values as they show, one display for both modes (compiled,
    // an error without a value among several did not build)
    Row { props: &[Prop::Differentiable, Prop::Does(crate::effects::Effect::Console)], ..row!("display.log", "", "log", crow_runtime::display::log, (Shows) -> Unit, never) },
    maprow!("core.map_new", "map", crow_runtime::core::map_new, () -> MapOf, never, maps::new, maps::new_taken),
    // a plain number's math (on the dual numbers a function grad
    // differentiates, each keeps the derivative's own form)
    Row { props: &[Prop::Differentiable], range: IntRange::Abs, big: Some("core.__abs_big"), ..row!("core.abs_int", "", "abs", crow_runtime::core::abs_int, (Whole) -> Int, stops) },
    Row { props: &[Prop::Differentiable], ..row!("core.abs", "", "abs", crow_runtime::core::abs_float, (Float) -> Float, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.sqrt", "", "sqrt", crow_runtime::core::sqrt, (Float) -> Float, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Keeps, big: Some("core.__whole_big"), ..row!("core.floor_int", "", "floor", crow_runtime::core::whole, (Whole) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, big: Some("core.__floor_big"), ..row!("core.floor", "", "floor", crow_runtime::core::floor, (Float) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Keeps, big: Some("core.__whole_big"), ..row!("core.ceil_int", "", "ceil", crow_runtime::core::whole, (Whole) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, big: Some("core.__ceil_big"), ..row!("core.ceil", "", "ceil", crow_runtime::core::ceil, (Float) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Keeps, big: Some("core.__whole_big"), ..row!("core.round_int", "", "round", crow_runtime::core::whole, (Whole) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, big: Some("core.__round_big"), ..row!("core.round", "", "round", crow_runtime::core::round, (Float) -> Int, never) },
    // the program and its system: effects, which the session layer
    // performs, and whose call inside is the row (compiled, env's default
    // had been text of any value, and a negative sleep panicked or slept
    // forever in both modes)
    Row { props: &[Prop::Does(crate::effects::Effect::Process)], ..row!("core.args", "", "args", crow_runtime::core::args, () -> Strs, never) },
    Row { props: &[Prop::Does(crate::effects::Effect::Process)], ..row!("core.env", "", "env", crow_runtime::core::env, (Str) -> Str, value "env") },
    Row { props: &[Prop::Does(crate::effects::Effect::Process)], ..row!("core.env_or", "", "env", crow_runtime::core::env_or, (Str, Cell) -> Str, value "env") },
    Row { props: &[Prop::Does(crate::effects::Effect::Input), Prop::Does(crate::effects::Effect::Waits)], ..row!("core.input", "", "input", crow_runtime::core::input, () -> Str, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::Input), Prop::Does(crate::effects::Effect::Waits)], ..row!("core.input_prompt", "", "input", crow_runtime::core::input_prompt, (Str) -> Str, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::Exit)], ..row!("core.exit", "", "exit", crow_runtime::core::exit, () -> Unit, never) },
    Row { props: &[Prop::Does(crate::effects::Effect::Exit)], ..row!("core.exit_code", "", "exit", crow_runtime::core::exit_code, (Whole) -> Unit, never) },
    row!("core.halt", "", "halt", crow_runtime::core::halt, (Str) -> Unit, stops),
    Row { props: &[Prop::Does(crate::effects::Effect::Waits)], ..row!("core.sleep_ms", "", "sleep", crow_runtime::core::sleep_ms, (Whole) -> Unit, stops) },
    Row { props: &[Prop::Does(crate::effects::Effect::Waits)], ..row!("core.sleep", "", "sleep", crow_runtime::core::sleep_secs, (Float) -> Unit, stops) },
    // (the lowering's: a loaded frame held to the columns the program's
    // reads say it has, where it is bound — lower_ir/uses.rs)
    row!("core.__expect_frame", "", "__expect_frame", crow_runtime::frames::expect, (Frame, Strs, Strs, Strs) -> Frame, stops),
    // a parsed value read as the kind its use implies (the lowering's
    // `read_json`; spec, JSON): the read that asked is the second argument
    row!("core.__read_int", "", "__read_int", crow_runtime::json::read_int, (Json, Str) -> Int, stops),
    row!("core.__read_float", "", "__read_float", crow_runtime::json::read_float, (Json, Str) -> Float, stops),
    row!("core.__read_str", "", "__read_str", crow_runtime::json::read_str, (Json, Str) -> Str, stops),
    row!("core.__read_bool", "", "__read_bool", crow_runtime::json::read_bool, (Json, Str) -> Bool, stops),
    row!("core.__read_moment", "", "__read_moment", crow_runtime::json::read_moment, (Json, Str) -> Moment, stops),
    // a call by name (`apply`, lowered to a dispatch the compiler writes):
    // each argument read from the parsed value by the parameter's name
    // and kind, an error value of kind `call` where it is missing or of
    // another kind; the answer's text made one that can fail
    row!("core.__arg_int", "", "__arg_int", crow_runtime::json::arg_int, (Json, Str, Str) -> Int, value "call"),
    row!("core.__arg_float", "", "__arg_float", crow_runtime::json::arg_float, (Json, Str, Str) -> Float, value "call"),
    row!("core.__arg_str", "", "__arg_str", crow_runtime::json::arg_str, (Json, Str, Str) -> Str, value "call"),
    row!("core.__arg_bool", "", "__arg_bool", crow_runtime::json::arg_bool, (Json, Str, Str) -> Bool, value "call"),
    row!("core.__arg_json", "", "__arg_json", crow_runtime::json::arg_json, (Json, Str, Str) -> Json, value "call"),
    row!("core.__ok_text", "", "__ok_text", crow_runtime::json::ok_text, (Str) -> Str, value "call"),
    Row { props: &[Prop::Does(crate::effects::Effect::Clock)], ..row!("moments.now", "", "now", crow_runtime::moments::now_moment, () -> Moment, never) },
    // the file system: an effect, which the session layer performs (held to
    // a handler's commit, recorded in a rehearsal) and whose call inside is
    // the row; a failure is an error value of kind 'io', a success of a
    // write empty text, as its type says (it had been `done` interpreted)
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], ..crow!("io.read", "", "read", (Str) -> Str, value "io") },
    // a URL read with options: `read[url, headers = map[…], query = map[…],
    // timeout = s]` (the lowering fills in what the call leaves out)
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], options: &[("headers", Preset::Map), ("query", Preset::Map), ("timeout", Preset::Float(0.0))], ..crow!("io.read_asked", "", "read", (Str, TextMap, TextMap, Float) -> Str, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::FilesWrite)], ..crow!("io.write", "", "write", (Str, Str) -> Str, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], ..row!("io.exists", "", "exists", crow_runtime::io::exists, (Str) -> Bool, never) },
    Row { props: &[Prop::Does(crate::effects::Effect::FilesWrite)], ..row!("io.mkdir", "", "mkdir", crow_runtime::io::mkdir, (Str) -> Str, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], ..row!("io.ls", "", "ls", crow_runtime::io::ls, (Str) -> Strs, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::FilesWrite)], ..row!("io.rm", "", "rm", crow_runtime::io::rm, (Str) -> Str, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::FilesWrite)], ..row!("io.mv", "", "mv", crow_runtime::io::mv, (Str, Str) -> Str, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::FilesWrite)], ..row!("io.cp", "", "cp", crow_runtime::io::cp, (Str, Str) -> Str, value "io") },
    // a command that cannot start is an error value of kind 'shell'; a
    // non-zero exit is an answer, read from `.status`
    Row { props: &[Prop::Does(crate::effects::Effect::Shell), Prop::Does(crate::effects::Effect::Waits)], ..row!("core.shell", "", "shell", crow_runtime::core::shell, (Str) -> Shell, value "shell") },
    // an HTTP request, answered whatever its status: its URL, method and
    // body, then its headers, query, form, files, save and timeout (the
    // lowering fills in what the call leaves out: `request[url]` is a GET)
    // written in Crow (src/modules/core.co) on the sys layer
    Row { props: &[Prop::Does(crate::effects::Effect::Network)], options: &[("method", Preset::Str("GET")), ("body", Preset::Str("")), ("headers", Preset::Map), ("query", Preset::Map), ("form", Preset::Map), ("files", Preset::Map), ("save", Preset::Str("")), ("timeout", Preset::Float(0.0))], ..crow!("core.request", "", "request", (Str, Str, Str, TextMap, TextMap, TextMap, TextMap, Str, Float) -> Reply, value "http") },
    // min and max of numbers (text was compared compiled and refused
    // interpreted: min['a', 'b'] is refused before the program runs)
    Row { props: &[Prop::Differentiable], range: IntRange::Min, big: Some("core.__min_big"), ..row!("core.min_ints", "", "min", crow_runtime::core::min_ints, (Whole, Whole) -> Int, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.min", "", "min", crow_runtime::core::min_floats, (Float, Float) -> Float, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Max, big: Some("core.__max_big"), ..row!("core.max_ints", "", "max", crow_runtime::core::max_ints, (Whole, Whole) -> Int, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.max", "", "max", crow_runtime::core::max_floats, (Float, Float) -> Float, never) },
    row!("hash.hash", "", "hash", crow_runtime::hash::hash, (Str, Str) -> Str, value "hash"),
    // fmt: a number's or a text's options, or a moment's time style, which
    // can fail (kind 'time'; compiled, the failure was the text "[fmt: …]")
    Row { options: &[("decimals", Preset::Int(-1)), ("commas", Preset::Bool(false)), ("width", Preset::Int(0)), ("align", Preset::Str("")), ("fill", Preset::Str(" ")), ("percent", Preset::Bool(false))], ..row!("display.fmt", "", "fmt", crow_runtime::display::fmt_value, (Cell, Int, Bool, Int, Str, Str, Bool) -> Str, stops) },
    Row { written: "fmt", options: &[("style", Preset::Asked), ("tz", Preset::Str(""))], ..row!("display.fmt_time", "", "__fmt_time", crow_runtime::moments::fmt_time_of, (Moment, Str, Str) -> Str, value "time") },
    // conversions: of a number or a bool, plain; of text or a JSON value,
    // fallible (kind 'parse'), read by one rule (compiled, JSON text was
    // trimmed and a JSON bool refused)
    Row { props: &[Prop::Differentiable], range: IntRange::Same, ..row!("core.int", "", "int", crow_runtime::cell::int_of_int, (Whole) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, big: Some("core.__int_of_float_big"), ..row!("core.int_of_float", "", "int", crow_runtime::cell::int_of_float, (Float) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, ..row!("core.int_of_decimal", "", "int", crow_runtime::decimal::trunc, (Decimal) -> Big, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, ..row!("core.int_of_fraction", "", "int", crow_runtime::fraction::trunc, (Fraction) -> Big, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Bool, ..row!("core.int_of_bool", "", "int", crow_runtime::cell::int_of_bool, (Bool) -> Int, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.int_of_moment", "", "int", crow_runtime::cell::int_of_moment, (Moment) -> Int, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, big: Some("core.__int_big"), ..row!("core.int_of", "", "int", crow_runtime::cell::int_of, (Cell) -> Int, value "parse") },
    // every digit kept: the rows the width pass calls in place of `int`,
    // `floor`, `ceil`, `round`, `abs`, `min` and `max` where an answer may
    // grow past 64 bits or a whole number past them is handed (int_width,
    // `big_form`); no program names them
    // a literal past 64 bits, reached by the parser
    row!("core.__whole", "", "__whole", crow_runtime::int::whole, (Str) -> Big, never),
    Row { range: IntRange::Any, ..row!("core.__int_big", "", "__int_big", crow_runtime::int::int_of, (Cell) -> Big, value "parse") },
    // an int where a record or none is held: 0 is none, another int stops
    // (the lowering's `none_of`, in both modes)
    row!("core.__none", "", "__none", crow_runtime::core::none_of, (Int) -> Int, stops),
    Row { range: IntRange::Any, ..row!("core.__int_of_float_big", "", "__int_of_float_big", crow_runtime::int::int_of_float, (Float) -> Big, stops) },
    Row { range: IntRange::Any, ..row!("core.__floor_big", "", "__floor_big", crow_runtime::int::floor, (Float) -> Big, stops) },
    Row { range: IntRange::Any, ..row!("core.__ceil_big", "", "__ceil_big", crow_runtime::int::ceil, (Float) -> Big, stops) },
    Row { range: IntRange::Any, ..row!("core.__round_big", "", "__round_big", crow_runtime::int::round, (Float) -> Big, stops) },
    Row { range: IntRange::Abs, ..row!("core.__abs_big", "", "__abs_big", crow_runtime::int::abs, (Big) -> Big, never) },
    Row { range: IntRange::Keeps, ..row!("core.__whole_big", "", "__whole_big", crow_runtime::int::itself, (Big) -> Big, never) },
    Row { range: IntRange::Min, ..row!("core.__min_big", "", "__min_big", crow_runtime::int::min, (Big, Big) -> Big, never) },
    Row { range: IntRange::And, ..row!("core.__bitand_big", "", "__bitand_big", crow_runtime::bits::and_big, (Big, Big) -> Big, never) },
    Row { range: IntRange::OrXor, ..row!("core.__bitor_big", "", "__bitor_big", crow_runtime::bits::or_big, (Big, Big) -> Big, never) },
    Row { range: IntRange::OrXor, ..row!("core.__bitxor_big", "", "__bitxor_big", crow_runtime::bits::xor_big, (Big, Big) -> Big, never) },
    Row { range: IntRange::Count, ..row!("core.__bitcount_big", "", "__bitcount_big", crow_runtime::bits::count_big, (Big) -> Int, never) },
    Row { range: IntRange::Shift, ..row!("core.__shift_big", "", "__shift_big", crow_runtime::bits::shift_big, (Big, Whole) -> Big, stops) },
    Row { range: IntRange::Max, ..row!("core.__max_big", "", "__max_big", crow_runtime::int::max, (Big, Big) -> Big, never) },
    row!("core.__sort_big", "", "__sort_big", crow_runtime::int::sort, (Bigs) -> Bigs, never),
    row!("core.__float_of_big", "", "__float_of_big", crow_runtime::int::float_of, (Big) -> Float, never),
    // exact numbers (spec, Numbers That Keep Every Digit): a decimal of text,
    // an int, a float (the text it shows) or itself; with digits, every
    // answer it takes part in rounds to them; a fraction of two ints in
    // lowest terms, of an int or a decimal exactly; each read as an int or a
    // float, rounded, and the math of a decimal to its digits (kind 'math'
    // where it has no value)
    row!("core.decimal_of_text", "", "decimal", crow_runtime::decimal::of_text, (Str) -> Decimal, value "parse"),
    row!("core.decimal_of_int", "", "decimal", crow_runtime::decimal::of_int, (Big) -> Decimal, never),
    row!("core.decimal_of_float", "", "decimal", crow_runtime::decimal::of_float, (Float) -> Decimal, stops),
    row!("core.decimal", "", "decimal", crow_runtime::decimal::same, (Decimal) -> Decimal, never),
    row!("core.__decimal_digits", "", "__decimal_digits", crow_runtime::decimal::with_digits, (Decimal, Whole) -> Decimal, stops),
    row!("core.fraction", "", "fraction", crow_runtime::fraction::of_parts, (Big, Big) -> Fraction, value "math"),
    row!("core.fraction_of_int", "", "fraction", crow_runtime::fraction::of_int, (Big) -> Fraction, never),
    row!("core.fraction_of_decimal", "", "fraction", crow_runtime::fraction::of_dec, (Decimal) -> Fraction, never),
    row!("core.fraction_same", "", "fraction", crow_runtime::fraction::same, (Fraction) -> Fraction, never),
    Row { props: &[Prop::Differentiable], range: IntRange::Any, ..row!("core.round_decimal", "", "round", crow_runtime::decimal::round, (Decimal) -> Big, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.round_decimal_places", "", "round", crow_runtime::decimal::round_places, (Decimal, Whole) -> Decimal, stops) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, ..row!("core.floor_decimal", "", "floor", crow_runtime::decimal::floor, (Decimal) -> Big, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, ..row!("core.ceil_decimal", "", "ceil", crow_runtime::decimal::ceil, (Decimal) -> Big, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.abs_decimal", "", "abs", crow_runtime::decimal::abs, (Decimal) -> Decimal, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.min_decimals", "", "min", crow_runtime::decimal::min, (Decimal, Decimal) -> Decimal, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.max_decimals", "", "max", crow_runtime::decimal::max, (Decimal, Decimal) -> Decimal, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.sqrt_decimal", "", "sqrt", crow_runtime::decimal::sqrt, (Decimal) -> Decimal, value "math") },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, ..row!("core.round_fraction", "", "round", crow_runtime::fraction::round, (Fraction) -> Big, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, ..row!("core.floor_fraction", "", "floor", crow_runtime::fraction::floor, (Fraction) -> Big, never) },
    Row { props: &[Prop::Differentiable], range: IntRange::Any, ..row!("core.ceil_fraction", "", "ceil", crow_runtime::fraction::ceil, (Fraction) -> Big, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.abs_fraction", "", "abs", crow_runtime::fraction::abs, (Fraction) -> Fraction, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.min_fractions", "", "min", crow_runtime::fraction::min, (Fraction, Fraction) -> Fraction, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.max_fractions", "", "max", crow_runtime::fraction::max, (Fraction, Fraction) -> Fraction, never) },
    Row { props: &[Prop::Differentiable], big: Some("core.__float_of_big"), ..row!("core.float_of_int", "", "float", crow_runtime::cell::float_of_int, (Whole) -> Float, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.float_of_decimal", "", "float", crow_runtime::decimal::to_float, (Decimal) -> Float, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.float_of_fraction", "", "float", crow_runtime::fraction::to_float, (Fraction) -> Float, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.float", "", "float", crow_runtime::cell::float_of_float, (Float) -> Float, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.float_of_bool", "", "float", crow_runtime::cell::float_of_bool, (Bool) -> Float, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.float_of_moment", "", "float", crow_runtime::cell::float_of_moment, (Moment) -> Float, never) },
    Row { props: &[Prop::Differentiable], ..row!("core.float_of", "", "float", crow_runtime::cell::float_of, (Cell) -> Float, value "parse") },
    // draws from the program's generator
    Row { props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("core.rand", "", "rand", crow_runtime::core::rand_float, () -> Float, never) },
    Row { props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("core.rand_int", "", "rand", crow_runtime::core::rand_int, (Int) -> Int, stops) },
    Row { props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("core.rand_between", "", "rand", crow_runtime::core::rand_between, (Int, Int) -> Int, stops) },
    row!("moments.moment_text", "", "moment", crow_runtime::moments::moment_text, (Str) -> Moment, value "time"),
    Row { options: &[("tz", Preset::Asked)], ..row!("moments.moment_in", "", "moment", crow_runtime::moments::moment_in, (Moment, Str) -> Moment, value "time") },
    Row { options: &[("tz", Preset::Asked)], ..row!("moments.moment_text_in", "", "moment", crow_runtime::moments::moment_text_in, (Str, Str) -> Moment, value "time") },
    row!("moments.moment_of_seconds", "", "moment", crow_runtime::moments::moment_of_seconds, (Float) -> Moment, stops),
    row!("moments.moment_self", "", "moment", crow_runtime::moments::moment_self, (Moment) -> Moment, never),
    row!("moments.bucket", "", "bucket", crow_runtime::moments::bucket, (Moment, Float) -> Moment, stops),
    Row { props: &[Prop::Differentiable], ..row!("moments.min", "", "min", crow_runtime::moments::min_of, (Moment, Moment) -> Moment, never) },
    Row { props: &[Prop::Differentiable], ..row!("moments.max", "", "max", crow_runtime::moments::max_of, (Moment, Moment) -> Moment, never) },
    Row { options: &[("hour", Preset::Int(0)), ("minute", Preset::Int(0)), ("second", Preset::Int(0)), ("tz", Preset::Str("UTC"))], ..row!("moments.moment", "", "moment", crow_runtime::moments::moment_parts, (Int, Int, Int, Int, Int, Float, Str) -> Moment, value "time") },
    row!("text.repeat", "", "repeat", crow_runtime::text::repeat, (Str, Int) -> Str, stops),
    // an invalid pattern is an error value of kind 'regex', as find[]'s is
    // (the interpreter answered one while the compiled program stopped)
    row!("text.matches", "", "matches", crow_runtime::text::matches, (Str, Str) -> Bool, value "regex"),
    row!("text.sub", "", "sub", crow_runtime::text::sub, (Str, Str, Str) -> Str, value "regex"),
    // `len[sub[…]]` of text, counted without the text (lower_ir: sub_len_of)
    row!("text.__sub_len", "", "__sub_len", crow_runtime::text::sub_len, (Str, Str, Str) -> Int, value "regex"),
    // math: the constructors and the distributions (a size, a parameter, a
    // rank the interpreter refused and the compiled program cast or
    // panicked on is one check)
    Row { shape: Shape::Sizes(0, 0), props: &[Prop::Constructs], ..row!("math.zeros", "math", "zeros", crow_runtime::math::zeros, (Sizes) -> Tensor, stops) },
    Row { shape: Shape::Sizes(0, 0), props: &[Prop::Constructs], ..row!("math.ones", "math", "ones", crow_runtime::math::ones, (Sizes) -> Tensor, stops) },
    Row { shape: Shape::Sizes(0, 0), props: &[Prop::Constructs, Prop::Does(crate::effects::Effect::Random)], ..crow!("math.randn", "math", "randn", (Sizes) -> Tensor, stops) },
    Row { shape: Shape::Sizes(0, 1), props: &[Prop::Constructs], ..row!("math.full", "math", "full", crow_runtime::math::full, (Sizes, Float) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Size(0), Dim::Size(0)]), props: &[Prop::Constructs], ..row!("math.eye", "math", "eye", crow_runtime::math::eye, (Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Size(1)]), props: &[Prop::Differentiable], ..row!("math.onehot", "math", "onehot", crow_runtime::math::onehot, (Tensor, Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Size(2)]), props: &[Prop::Constructs], ..row!("math.linspace", "math", "linspace", crow_runtime::math::linspace, (Float, Float, Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("math.arange", "math", "arange", crow_runtime::math::arange, (Float, Float, Float) -> Tensor, stops) },
    Row { shape: Shape::Diag(0), ..row!("math.diag", "math", "diag", crow_runtime::math::diag, (Tensor) -> Tensor, stops) },
    Row { shape: Shape::Sizes(2, 0), props: &[Prop::Does(crate::effects::Effect::Random)], ..crow!("math.normal", "math", "normal", (Float, Float, Sizes) -> Tensor, stops) },
    Row { shape: Shape::Sizes(2, 0), props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("math.uniform", "math", "uniform", crow_runtime::math::uniform, (Float, Float, Sizes) -> Tensor, stops) },
    Row { shape: Shape::Sizes(1, 0), props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("math.poisson", "math", "poisson", crow_runtime::math::poisson, (Float, Sizes) -> Tensor, stops) },
    Row { shape: Shape::Sizes(1, 0), props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("math.bernoulli", "math", "bernoulli", crow_runtime::math::bernoulli, (Float, Sizes) -> Tensor, stops) },
    Row { shape: Shape::Sizes(2, 0), props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("math.gamma_dist", "math", "gamma_dist", crow_runtime::math::gamma_dist, (Float, Float, Sizes) -> Tensor, stops) },
    Row { shape: Shape::Sizes(2, 0), props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("math.beta_dist", "math", "beta_dist", crow_runtime::math::beta_dist, (Float, Float, Sizes) -> Tensor, stops) },
    Row { shape: Shape::Sizes(1, 0), props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("math.exponential", "math", "exponential", crow_runtime::math::exponential, (Float, Sizes) -> Tensor, stops) },
    // math: linear algebra (a vector, a rank-3 tensor or a rectangle where a
    // square is needed, a matrix that is not positive definite: the reason,
    // where a kernel panicked)
    row!("math.det", "math", "det", crow_runtime::math::det, (Tensor) -> Float, stops),
    Row { shape: Shape::Same(0), ..row!("math.inv", "math", "inv", crow_runtime::math::inv, (Tensor) -> Tensor, stops) },
    Row { shape: Shape::Pieces(&[&[Dim::Of(0, 0), Dim::Open], &[Dim::Open], &[Dim::Open, Dim::Of(0, 1)]]), ..row!("math.svd", "math", "svd", crow_runtime::math::svd, (Tensor) -> Tensors, stops) },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), ..row!("math.qr", "math", "qr", crow_runtime::math::qr, (Tensor) -> Tensors, stops) },
    Row { shape: Shape::Same(0), ..row!("math.cholesky", "math", "cholesky", crow_runtime::math::cholesky, (Tensor) -> Tensor, stops) },
    Row { shape: Shape::Pieces(&[&[Dim::Of(0, 0)], &[Dim::Of(0, 0), Dim::Of(0, 0)]]), ..row!("math.eigen", "math", "eigen", crow_runtime::math::eigen, (Tensor) -> Tensors, stops) },
    row!("math.norm", "math", "norm", crow_runtime::math::norm, (Tensor) -> Float, never),
    row!("math.norm_p", "math", "norm", crow_runtime::math::norm_p, (Tensor, Float) -> Float, never),
    row!("math.dot", "math", "dot", crow_runtime::math::dot, (Tensor, Tensor) -> Float, stops),
    Row { shape: Shape::Same(0), ..row!("math.pinv", "math", "pinv", crow_runtime::math::pinv, (Tensor) -> Tensor, stops) },
    Row { shape: Shape::Like(1, Dim::Of(0, 1)), ..row!("math.lstsq", "math", "lstsq", crow_runtime::math::lstsq, (Tensor, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Same(0), ..row!("math.triu", "math", "triu", crow_runtime::math::triu, (Tensor) -> Tensor, stops) },
    Row { shape: Shape::Same(0), ..row!("math.tril", "math", "tril", crow_runtime::math::tril, (Tensor) -> Tensor, stops) },
    row!("math.rank", "math", "rank", crow_runtime::math::rank, (Tensor) -> Int, stops),
    row!("math.cond", "math", "cond", crow_runtime::math::cond, (Tensor) -> Float, stops),
    // math: reductions (an axis out of range is the reason), sorting and
    // selection, shapes (a reshape that does not fill, a transpose of a
    // vector: the reason, where a kernel asserted)
    Row { props: &[Prop::Differentiable], ..row!("math.sum", "math", "sum", crow_runtime::math::sum, (Tensor) -> Float, never) },
    Row { shape: Shape::Reduce(0, 1), props: &[Prop::Differentiable], options: &[("axis", Preset::Asked)], ..row!("math.sum_axis", "math", "sum", crow_runtime::math::sum_axis, (Tensor, Int) -> Tensor, stops) },
    Row { props: &[Prop::Differentiable], ..row!("math.mean", "math", "mean", crow_runtime::math::mean, (Tensor) -> Float, never) },
    Row { shape: Shape::Reduce(0, 1), props: &[Prop::Differentiable], options: &[("axis", Preset::Asked)], ..row!("math.mean_axis", "math", "mean", crow_runtime::math::mean_axis, (Tensor, Int) -> Tensor, stops) },
    Row { props: &[Prop::Differentiable], ..row!("math.min", "math", "min", crow_runtime::math::min, (Tensor) -> Float, stops) },
    Row { shape: Shape::Reduce(0, 1), props: &[Prop::Differentiable], options: &[("axis", Preset::Asked)], ..row!("math.min_axis", "math", "min", crow_runtime::math::min_axis, (Tensor, Int) -> Tensor, stops) },
    Row { props: &[Prop::Differentiable], ..row!("math.max", "math", "max", crow_runtime::math::max, (Tensor) -> Float, stops) },
    Row { shape: Shape::Reduce(0, 1), props: &[Prop::Differentiable], options: &[("axis", Preset::Asked)], ..row!("math.max_axis", "math", "max", crow_runtime::math::max_axis, (Tensor, Int) -> Tensor, stops) },
    row!("math.prod", "math", "prod", crow_runtime::math::prod, (Tensor) -> Float, never),
    Row { shape: Shape::Reduce(0, 1), options: &[("axis", Preset::Asked)], ..row!("math.prod_axis", "math", "prod", crow_runtime::math::prod_axis, (Tensor, Int) -> Tensor, stops) },
    row!("math.argmax", "math", "argmax", crow_runtime::math::argmax, (Tensor) -> Int, never),
    row!("math.argmin", "math", "argmin", crow_runtime::math::argmin, (Tensor) -> Int, never),
    Row { shape: Shape::Same(0), ..row!("math.cumsum", "math", "cumsum", crow_runtime::math::cumsum, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Same(0), ..row!("math.cumprod", "math", "cumprod", crow_runtime::math::cumprod, (Tensor) -> Tensor, never) },
    row!("math.argsort", "math", "argsort", crow_runtime::math::argsort, (Tensor) -> Ints, never),
    Row { shape: Shape::Dims(&[Dim::Size(1)]), ..row!("math.topk", "math", "topk", crow_runtime::math::topk, (Tensor, Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("math.unique", "math", "unique", crow_runtime::math::unique, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Broadcast(&[0, 1, 2]), props: &[Prop::Mask(0)], ..row!("math.where", "math", "where", crow_runtime::math::where_, (Tensor, Tensor, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Sizes(1, 0), props: &[Prop::Differentiable], ..row!("math.reshape", "math", "reshape", crow_runtime::math::reshape, (Tensor, Sizes) -> Tensor, stops) },
    Row { shape: Shape::Flat(0), props: &[Prop::Differentiable], ..row!("math.flatten", "math", "flatten", crow_runtime::math::flatten, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Transposed(0), props: &[Prop::Differentiable], ..row!("math.transpose", "math", "transpose", crow_runtime::math::transpose, (Tensor) -> Tensor, stops) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable], ..row!("math.clip", "math", "clip", crow_runtime::math::clip, (Tensor, Float, Float) -> Tensor, never) },
    Row { shape: Shape::Broadcast(&[0, 1]), ..row!("math.atan2", "math", "atan2", crow_runtime::math::atan2, (Tensor, Tensor) -> Tensor, stops) },
    // element-wise: a number and a tensor share one rule (the number's row
    // first, so a number never reaches the tensor's)
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.exp_decimal", "math", "exp", crow_runtime::decimal::exp, (Decimal) -> Decimal, value "math") },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.ln_decimal", "math", "ln", crow_runtime::decimal::ln, (Decimal) -> Decimal, value "math") },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.sin_decimal", "math", "sin", crow_runtime::decimal::sin, (Decimal) -> Decimal, value "math") },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.cos_decimal", "math", "cos", crow_runtime::decimal::cos, (Decimal) -> Decimal, value "math") },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.tan_decimal", "math", "tan", crow_runtime::decimal::tan, (Decimal) -> Decimal, value "math") },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.atan_decimal", "math", "atan", crow_runtime::decimal::atan, (Decimal) -> Decimal, value "math") },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.exp_n", "math", "exp", crow_runtime::math::exp_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.exp_t", "math", "exp", crow_runtime::math::exp_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.ln_n", "math", "ln", crow_runtime::math::ln_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.ln_t", "math", "ln", crow_runtime::math::ln_t, (Tensor) -> Tensor, never) },
    // (a × b + c rounded once: what a compiled loop accumulates with, and
    // a sum held to one must too — scikit-learn's row norms and scipy's
    // sparse products differed in the last digit from a plain sum)
    row!("math.fma", "math", "fma", crow_runtime::math::fma, (Float, Float, Float) -> Float, never),
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.log2_n", "math", "log2", crow_runtime::math::log2_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.log2_t", "math", "log2", crow_runtime::math::log2_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.log10_n", "math", "log10", crow_runtime::math::log10_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.log10_t", "math", "log10", crow_runtime::math::log10_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.sin_n", "math", "sin", crow_runtime::math::sin_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.sin_t", "math", "sin", crow_runtime::math::sin_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.cos_n", "math", "cos", crow_runtime::math::cos_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.cos_t", "math", "cos", crow_runtime::math::cos_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.tan_n", "math", "tan", crow_runtime::math::tan_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.tan_t", "math", "tan", crow_runtime::math::tan_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.asin_n", "math", "asin", crow_runtime::math::asin_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.asin_t", "math", "asin", crow_runtime::math::asin_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.acos_n", "math", "acos", crow_runtime::math::acos_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.acos_t", "math", "acos", crow_runtime::math::acos_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.atan_n", "math", "atan", crow_runtime::math::atan_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.atan_t", "math", "atan", crow_runtime::math::atan_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.sinh_n", "math", "sinh", crow_runtime::math::sinh_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.sinh_t", "math", "sinh", crow_runtime::math::sinh_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.cosh_n", "math", "cosh", crow_runtime::math::cosh_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.cosh_t", "math", "cosh", crow_runtime::math::cosh_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.tanh_n", "math", "tanh", crow_runtime::math::tanh_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.tanh_t", "math", "tanh", crow_runtime::math::tanh_t, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.sign_n", "math", "sign", crow_runtime::math::sign_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.sign_t", "math", "sign", crow_runtime::math::sign_t, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable], ..row!("math.abs_t", "math", "abs", crow_runtime::math::abs_t, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable], ..row!("math.sqrt_t", "math", "sqrt", crow_runtime::math::sqrt_t, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable], ..row!("math.floor_t", "math", "floor", crow_runtime::math::floor_t, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable], ..row!("math.ceil_t", "math", "ceil", crow_runtime::math::ceil_t, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable], ..row!("math.round_t", "math", "round", crow_runtime::math::round_t, (Tensor) -> Tensor, never) },
    // math: signal, special functions, complex numbers, draws (a rank, a
    // window or a count the kernels asserted is the reason)
    Row { shape: Shape::Same(0), ..row!("math.fft", "math", "fft", crow_runtime::math::fft, (Tensor) -> Tensors, stops) },
    Row { shape: Shape::Same(0), ..row!("math.ifft", "math", "ifft", crow_runtime::math::ifft, (Tensor, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), ..row!("math.fft2", "math", "fft2", crow_runtime::math::fft2, (Tensor) -> Tensors, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("math.conv", "math", "conv", crow_runtime::math::conv, (Tensor, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), ..row!("math.conv2d", "math", "conv2d", crow_runtime::math::conv2d, (Tensor, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Size(0)]), ..row!("math.hamming", "math", "hamming", crow_runtime::math::hamming, (Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Size(0)]), ..row!("math.hanning", "math", "hanning", crow_runtime::math::hanning, (Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), ..row!("math.stft", "math", "stft", crow_runtime::math::stft, (Tensor, Int, Int) -> Tensors, stops) },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), ..row!("math.spectrogram", "math", "spectrogram", crow_runtime::math::spectrogram, (Tensor, Int, Int) -> Tensor, stops) },
    Row { shape: Shape::Same(0), ..row!("math.lowpass", "math", "lowpass", crow_runtime::math::lowpass, (Tensor, Float, Float) -> Tensor, stops) },
    Row { shape: Shape::Same(0), ..row!("math.highpass", "math", "highpass", crow_runtime::math::highpass, (Tensor, Float, Float) -> Tensor, stops) },
    Row { shape: Shape::Same(0), ..row!("math.bandpass", "math", "bandpass", crow_runtime::math::bandpass, (Tensor, Float, Float, Float) -> Tensor, stops) },
    Row { shape: Shape::Same(0), ..row!("math.gamma", "math", "gamma", crow_runtime::math::gamma, (Tensor) -> Tensor, never) },
    Row { props: &[Prop::Differentiable, Prop::Dual], ..row!("math.erf_n", "math", "erf", crow_runtime::math::erf_n, (Float) -> Float, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Differentiable, Prop::Dual], ..row!("math.erf", "math", "erf", crow_runtime::math::erf, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Same(0), ..row!("math.factorial", "math", "factorial", crow_runtime::math::factorial, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Same(0), ..row!("math.bessel", "math", "bessel", crow_runtime::math::bessel, (Tensor, Int) -> Tensor, never) },
    Row { shape: Shape::Broadcast(&[0, 1]), ..row!("math.choose", "math", "choose", crow_runtime::math::choose, (Tensor, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("math.complex", "math", "complex", crow_runtime::math::complex, (Float, Float) -> Tensors, never) },
    Row { shape: Shape::Items(0), ..row!("math.real", "math", "real", crow_runtime::math::real, (Tensors) -> Tensor, stops) },
    Row { shape: Shape::Items(0), ..row!("math.imag", "math", "imag", crow_runtime::math::imag, (Tensors) -> Tensor, stops) },
    Row { shape: Shape::Items(0), ..row!("math.magnitude", "math", "magnitude", crow_runtime::math::magnitude, (Tensors) -> Tensor, stops) },
    Row { shape: Shape::Items(0), ..row!("math.phase", "math", "phase", crow_runtime::math::phase, (Tensors) -> Tensor, stops) },
    Row { shape: Shape::Items(0), ..row!("math.conjugate", "math", "conjugate", crow_runtime::math::conjugate, (Tensors) -> Tensors, stops) },
    Row { props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("math.seed", "math", "seed", crow_runtime::math::seed, (Int) -> Unit, never) },
    Row { shape: Shape::Same(0), props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("math.shuffle", "math", "shuffle", crow_runtime::math::shuffle, (Tensor) -> Tensor, never) },
    Row { shape: Shape::Dims(&[Dim::Size(1)]), props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("math.choice", "math", "choice", crow_runtime::math::choice, (Tensor, Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Size(0)]), props: &[Prop::Does(crate::effects::Effect::Random)], ..row!("math.permutation", "math", "permutation", crow_runtime::math::permutation, (Int) -> Tensor, stops) },
    Row { shape: Shape::Broadcast(&[0, 1]), ..row!("math.mod", "math", "mod", crow_runtime::math::modulo, (Tensor, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Same(0), ..row!("math.mod_by", "math", "mod", crow_runtime::math::modulo_by, (Tensor, Float) -> Tensor, never) },
    // math: sparse matrices, solving, stacking, conversion (an entry outside
    // its matrix, a system that does not meet, tensors of two shapes: the
    // reason; a frame that does not convert stopped the interpreter and
    // unwrapped into a panic compiled)
    row!("math.sparse", "math", "sparse", crow_runtime::math::sparse, (Tensor) -> Sparse, stops),
    row!("math.sparse_coo", "math", "sparse", crow_runtime::math::sparse_coo, (Tensor, Tensor, Tensor, Tensor) -> Sparse, stops),
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), ..row!("math.dense", "math", "dense", crow_runtime::math::dense, (Sparse) -> Tensor, never) },
    Row { shape: Shape::Same(0), ..row!("math.dense_of", "math", "dense", crow_runtime::math::dense_of, (Tensor) -> Tensor, never) },
    row!("math.nnz", "math", "nnz", crow_runtime::math::nnz, (Sparse) -> Int, never),
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("math.spmv", "math", "spmv", crow_runtime::math::spmv, (Sparse, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Same(1), ..row!("math.solve_sparse", "math", "solve", crow_runtime::math::solve_sparse, (Sparse, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Same(1), ..row!("math.solve", "math", "solve", crow_runtime::math::solve, (Tensor, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Stacked(0), ..row!("math.stack", "math", "stack", crow_runtime::math::stack, (Tensors) -> Tensor, stops) },
    Row { shape: Shape::Restacked(0), options: &[("axis", Preset::Asked)], ..row!("math.stack_axis", "math", "stack", crow_runtime::math::stack_axis, (Tensors, Int) -> Tensor, stops) },
    Row { shape: Shape::Stacked(0), ..row!("math.stack_rows", "math", "stack", crow_runtime::math::stack_rows, (Tensor) -> Tensor, stops) },
    Row { shape: Shape::Restacked(0), ..row!("math.stack_rows_axis", "math", "stack", crow_runtime::math::stack_rows_axis, (Tensor, Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), props: &[Prop::Constructs], ..row!("math.tensor_of_frame", "math", "tensor", crow_runtime::math::tensor_of_frame, (Frame) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), props: &[Prop::Constructs], ..row!("math.tensor_of_columns", "math", "tensor", crow_runtime::math::tensor_of_columns, (Frame, Strs) -> Tensor, stops) },
    Row { shape: Shape::Same(0), props: &[Prop::Constructs], ..row!("math.tensor", "math", "tensor", crow_runtime::math::tensor, (Tensor) -> Tensor, never) },
    // math.stats (series of two lengths, a percent past 100, a distribution
    // no test knows: the reason, where they were answered NaN, 0.5 or an
    // empty map; anova of one group was NaN compiled and refused
    // interpreted)
    // math.stats and math.nn are written in Crow (src/modules/stats.co,
    // nn.co): their functions are their declaration, typed by their use
    // (docs/extending-crow.md, phase 2); the one row left is the variadic
    // `anova[a, b, …]`, whose rest parameter no function of the language
    // takes yet (phase 4)
    crow!("stats.anova", "math.stats", "anova", (Groups) -> FloatMap, stops),
    // chart (a window of no rows, series of two lengths: the reason, where
    // a negative window was a huge one compiled and one row interpreted,
    // and a shorter series cut the others)
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("chart.sma", "chart", "sma", crow_runtime::indicators::sma, (Tensor, Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("chart.ema", "chart", "ema", crow_runtime::indicators::ema, (Tensor, Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("chart.rsi", "chart", "rsi", crow_runtime::indicators::rsi, (Tensor, Int) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("chart.rsi_buckets", "chart", "rsi", crow_runtime::indicators::rsi_buckets, (Tensor, Tensor, Int, Float) -> Tensor, stops) },
    Row { columns: &[("line", Kind::Float), ("signal", Kind::Float), ("hist", Kind::Float)], ..row!("chart.macd", "chart", "macd", crow_runtime::indicators::macd, (Tensor, Int, Int, Int) -> Frame, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("chart.vwap", "chart", "vwap", crow_runtime::indicators::vwap, (Tensor, Tensor) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("chart.vwap_trailing", "chart", "vwap", crow_runtime::indicators::vwap_trailing, (Tensor, Tensor, Tensor, Float) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("chart.rolling_sum", "chart", "rolling_sum", crow_runtime::indicators::rolling_sum, (Tensor, Tensor, Float) -> Tensor, stops) },
    Row { shape: Shape::Dims(&[Dim::Open]), ..row!("chart.cvd", "chart", "cvd", crow_runtime::indicators::cvd, (Tensor, Tensor) -> Tensor, stops) },
    // `use sys`: the standard library's own layer (crow_runtime::sys) — a
    // program may not import it (spec, The standard library's layer)
    row!("sys.open", "sys", "open", crow_runtime::sys::open, (Str, Int, Bool, Float) -> Stream, value "sys"),
    Row { props: &[Prop::Does(crate::effects::Effect::Network)], ..row!("sys.listen", "sys", "listen", crow_runtime::sys::listen, (Str, Int) -> Stream, value "sys") },
    Row { props: &[Prop::Does(crate::effects::Effect::Network), Prop::Does(crate::effects::Effect::Waits)], ..row!("sys.accept", "sys", "accept", crow_runtime::sys::accept, (Stream, Float) -> Stream, value "sys") },
    row!("sys.address", "sys", "address", crow_runtime::sys::address, (Stream) -> Str, never),
    // a process the program talks to: started as `shell` runs a command,
    // its input and output the socket's stream, what a read took past a
    // message kept by the socket (the MCP stdio transport stands on it);
    // as `open`, the handle made is no effect — its `send` and `recv` are
    row!("sys.start", "sys", "start", crow_runtime::sys::start, (Str) -> Socket, value "sys"),
    Row { props: &[Prop::Does(crate::effects::Effect::Network)], ..row!("sys.send", "sys", "send", crow_runtime::sys::send, (Stream, Bytes) -> Int, value "sys") },
    Row { props: &[Prop::Does(crate::effects::Effect::Network), Prop::Does(crate::effects::Effect::Waits)], ..row!("sys.recv", "sys", "recv", crow_runtime::sys::recv, (Stream, Int, Float) -> Bytes, value "sys") },
    row!("sys.close", "sys", "close", crow_runtime::sys::close, (Stream) -> Unit, never),
    row!("sys.bytes", "sys", "bytes", crow_runtime::sys::bytes_of_text, (Str) -> Bytes, never),
    row!("sys.text", "sys", "text", crow_runtime::sys::text_of_bytes, (Bytes) -> Str, value "parse"),
    row!("sys.slice", "sys", "slice", crow_runtime::sys::slice, (Bytes, Int, Int) -> Bytes, stops),
    Row { range: IntRange::Position, ..row!("sys.find", "sys", "find", crow_runtime::sys::find, (Bytes, Bytes) -> Int, never) },
    Row { props: &[Prop::InPlace], ..row!("sys.concat", "sys", "concat", crow_runtime::sys::concat, (Bytes, Bytes) -> Bytes, never) },
    row!("sys.byte", "sys", "byte", crow_runtime::sys::byte, (Bytes, Int) -> Int, stops),
    Row { props: &[Prop::InPlace], ..row!("sys.concat_all", "sys", "concat", crow_runtime::sys::gather, (Chunks) -> Bytes, never) },
    row!("sys.file", "sys", "file", crow_runtime::sys::file, (Str) -> Bytes, value "sys"),
    row!("sys.file_lines", "sys", "file_lines", crow_runtime::sys::file_lines, (Str) -> Strs, value "sys"),
    row!("sys.store", "sys", "store", crow_runtime::sys::store, (Str, Bytes) -> Unit, value "sys"),
    row!("sys.random", "sys", "random", crow_runtime::sys::random, (Int) -> Bytes, stops),
    row!("sys.hex", "sys", "hex", crow_runtime::sys::hex, (Bytes) -> Str, never),
    row!("sys.text_with", "sys", "text", crow_runtime::sys::text_with, (Bytes, Str) -> Str, never),
    row!("sys.hash", "sys", "hash", crow_runtime::sys::hash, (Bytes, Str) -> Str, stops),
    row!("sys.hmac", "sys", "hmac", crow_runtime::sys::hmac, (Bytes, Bytes, Str) -> Bytes, stops),
    row!("sys.reply", "sys", "reply", crow_runtime::sources::reply, (Int, Str, TextMap) -> Reply, never),
    // a request's fixture: what a recording kept, what a replay answers
    // (crow_runtime::fixtures; `request`'s body asks before it sends)
    row!("sys.replaying", "sys", "replaying", crow_runtime::fixtures::replaying_now, () -> Bool, never),
    row!("sys.recording", "sys", "recording", crow_runtime::fixtures::recording_now, () -> Bool, never),
    row!("sys.rehearsing", "sys", "rehearsing", crow_runtime::fixtures::rehearsing_now, () -> Bool, never),
    row!("sys.replayed", "sys", "replayed", crow_runtime::fixtures::replayed, (Str, Str, Str) -> Reply, value "http"),
    row!("sys.recorded", "sys", "recorded", crow_runtime::fixtures::recorded, (Str, Str, Str, Int, Str, TextMap) -> Unit, value "sys"),
    // bytes as text and back, a signature compared, a number's bytes, and a
    // response of bytes: what a token, a frame's header and a packed value
    // are written over
    row!("sys.base64", "sys", "base64", crow_runtime::sys::base64, (Bytes) -> Str, never),
    row!("sys.base64url", "sys", "base64url", crow_runtime::sys::base64url, (Bytes) -> Str, never),
    row!("sys.unbase64", "sys", "unbase64", crow_runtime::sys::unbase64, (Str) -> Bytes, value "parse"),
    row!("sys.same", "sys", "same", crow_runtime::sys::same, (Bytes, Bytes) -> Bool, never),
    row!("sys.pack", "sys", "pack", crow_runtime::sys::pack, (Int, Int) -> Bytes, stops),
    row!("sys.unpack", "sys", "unpack", crow_runtime::sys::unpack, (Bytes, Int, Int) -> Int, stops),
    row!("sys.records", "sys", "records", crow_runtime::sys::records, (Bytes) -> Frame, value "kafka"),
    row!("sys.records_of", "sys", "records_of", crow_runtime::sys::records_of, (Frame) -> Bytes, value "kafka"),
    row!("sys.ieee", "sys", "ieee", crow_runtime::sys::ieee, (Float) -> Bytes, never),
    row!("sys.binary", "sys", "binary", crow_web::core::response::binary_of, (Bytes, Str) -> Response, never),
    // a socket: the stream it talks over, what it holds unread, a frame's
    // mask, a digest as bytes (the handshake's accept key)
    row!("sys.socket_of", "sys", "socket_of", crow_runtime::sys::socket_of, (Stream, Str, Bytes) -> Socket, never),
    row!("sys.stream_of", "sys", "stream_of", crow_runtime::sys::stream_of, (Socket) -> Stream, never),
    row!("sys.leftover", "sys", "leftover", crow_runtime::sys::leftover, (Socket) -> Bytes, never),
    row!("sys.unread", "sys", "unread", crow_runtime::sys::unread, (Socket, Bytes) -> Unit, never),
    row!("sys.mask", "sys", "mask", crow_runtime::sys::mask, (Bytes, Bytes) -> Bytes, stops),
    row!("sys.digest", "sys", "digest", crow_runtime::sys::digest, (Bytes, Str) -> Bytes, stops),
    // the special functions the statistics are written over (crow_runtime::stats)
    row!("sys.betainc", "sys", "betainc", crow_runtime::stats::betainc, (Float, Float, Float) -> Float, never),
    row!("sys.gammainc", "sys", "gammainc", crow_runtime::stats::gammainc, (Float, Float) -> Float, never),
    row!("sys.lgamma", "sys", "lgamma", crow_runtime::stats::lgamma, (Float) -> Float, never),
    row!("sys.shaped", "sys", "shaped", crow_runtime::math::shaped, (Tensor, Ints) -> Tensor, stops),
    row!("sys.note", "sys", "note", crow_runtime::sys::note, (Stream, Str, Str) -> Unit, never),
    row!("sys.noted", "sys", "noted", crow_runtime::sys::noted, (Stream, Str) -> Str, never),
    row!("sys.machine", "sys", "machine", crow_runtime::sys::machine, () -> Str, never),
    row!("sys.process", "sys", "process", crow_runtime::sys::process, () -> Int, never),
    row!("sys.alive", "sys", "alive", crow_runtime::sys::alive, (Int) -> Bool, never),
    row!("sys.rowbinary_of_frame", "sys", "rowbinary_of_frame", crow_runtime::sys::rowbinary_of_frame, (Frame, Texts) -> Bytes, value "frame"),
    row!("sys.xor", "sys", "xor", crow_runtime::sys::xor, (Bytes, Bytes) -> Bytes, stops),
    row!("sys.copy_of_frame", "sys", "copy_of_frame", crow_runtime::sys::copy_of_frame, (Frame) -> Bytes, never),
    // the Postgres driver written in Crow (src/modules/postgres.co)
    crow!("db.postgres.pg_open", "db.postgres", "pg_open", (Encoded) -> Stream, value "db"),
    crow!("db.postgres.pg_check", "db.postgres", "pg_check", (Encoded, Stream) -> Int, value "db"),
    crow!("db.postgres.pg_query", "db.postgres", "pg_query", (Encoded, Stream, Str, Encoded) -> Frame, value "db"),
    crow!("db.postgres.pg_exec", "db.postgres", "pg_exec", (Encoded, Stream, Str, Encoded) -> Int, value "db"),
    crow!("db.postgres.pg_insert", "db.postgres", "pg_insert", (Encoded, Stream, Str, Frame) -> Int, value "db"),
    crow!("db.postgres.pg_describe", "db.postgres", "pg_describe", (Encoded, Stream, Str) -> Frame, value "db"),
    crow!("db.postgres.pg_cancel", "db.postgres", "pg_cancel", (Encoded, Stream) -> Int, value "db"),
    // the ClickHouse driver written in Crow (src/modules/clickhouse.co),
    // as crow_db::driven calls it (docs/db-in-crow-design.md)
    crow!("db.clickhouse.ch_open", "db.clickhouse", "ch_open", (Encoded) -> Stream, value "db"),
    crow!("db.clickhouse.ch_check", "db.clickhouse", "ch_check", (Encoded, Stream) -> Int, value "db"),
    crow!("db.clickhouse.ch_query", "db.clickhouse", "ch_query", (Encoded, Stream, Str, Encoded) -> Frame, value "db"),
    crow!("db.clickhouse.ch_exec", "db.clickhouse", "ch_exec", (Encoded, Stream, Str, Encoded) -> Int, value "db"),
    crow!("db.clickhouse.ch_insert", "db.clickhouse", "ch_insert", (Encoded, Stream, Str, Frame) -> Int, value "db"),
    crow!("db.clickhouse.ch_describe", "db.clickhouse", "ch_describe", (Encoded, Stream, Str) -> Frame, value "db"),
    crow!("db.clickhouse.ch_cancel", "db.clickhouse", "ch_cancel", (Encoded, Stream) -> Int, value "db"),
    // a mailer's fields, and a connection secured (SMTP's STARTTLS)
    row!("sys.mailer_field", "sys", "mailer_field", crow_runtime::sys::mailer_field, (Mailer, Str) -> Str, stops),
    row!("sys.secure", "sys", "secure", crow_runtime::sys::secure, (Stream, Str) -> Stream, value "sys"),
    // the mailer `use web.mail` makes: a handle, its password in no value
    row!("web.mail.smtp", "web.mail", "smtp", crow_runtime::sys::mailer, (Str, Int, Str, Str) -> Mailer, never),
    row!("sys.answer", "sys", "answer", crow_runtime::sys::answer, (Int, Str, TextMap, Bytes) -> Answer, never),
    // the parse kernels the readers written in Crow stand on
    row!("sys.csv_rows", "sys", "csv_rows", crow_runtime::sys::csv_rows, (Str) -> TextRows, value "io"),
    row!("sys.frame_of_csv", "sys", "frame_of_csv", crow_runtime::sys::frame_of_csv, (Str) -> Frame, value "frame"),
    row!("sys.frame_of_csv_bytes", "sys", "frame_of_csv_bytes", crow_runtime::sys::frame_of_csv_bytes, (Bytes) -> Frame, value "frame"),
    row!("sys.frame_of_json", "sys", "frame_of_json", crow_runtime::sys::frame_of_json, (Str) -> Frame, value "frame"),
    Row { feature: Some(("crow_runtime", "parquet")), ..row!("sys.frame_of_parquet", "sys", "frame_of_parquet", crow_runtime::sys::frame_of_parquet, (Bytes) -> Frame, value "frame") },
    row!("sys.frame_of_rowbinary", "sys", "frame_of_rowbinary", crow_runtime::sys::frame_of_rowbinary, (Bytes) -> Frame, value "frame"),
    // the log's batch files (crow_runtime::batch; the lake module stands on them)
    Row { props: &[Prop::Does(crate::effects::Effect::FilesWrite)], feature: Some(("crow_runtime", "parquet")), ..row!("sys.batch_write", "sys", "batch_write", crow_runtime::sys::batch_write, (Str, Int, Int, Frame, Texts, Texts) -> Str, value "lake") },
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], feature: Some(("crow_runtime", "parquet")), ..row!("sys.batch_read", "sys", "batch_read", crow_runtime::sys::batch_read, (Str) -> Frame, value "lake") },
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], feature: Some(("crow_runtime", "parquet")), ..row!("sys.batch_stats", "sys", "batch_stats", crow_runtime::sys::batch_stats, (Str) -> Str, value "lake") },
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], feature: Some(("crow_runtime", "parquet")), ..row!("sys.batch_admits", "sys", "batch_admits", crow_runtime::sys::batch_admits, (Str, Str, Str, Bool) -> Bool, value "lake") },
    Row { props: &[Prop::Does(crate::effects::Effect::FilesWrite)], feature: Some(("crow_runtime", "parquet")), ..row!("sys.batch_merge", "sys", "batch_merge", crow_runtime::sys::batch_merge, (Str, Texts, Texts, Bool) -> Str, value "lake") },
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], feature: Some(("crow_runtime", "parquet")), ..row!("sys.batch_find", "sys", "batch_find", crow_runtime::sys::batch_find, (Str, Str, Str) -> Frame, value "lake") },
    row!("sys.frame_of_postgres", "sys", "frame_of_postgres", crow_runtime::sys::frame_of_postgres, (Bytes) -> Frame, value "frame"),
    row!("sys.frame_csv", "sys", "frame_csv", crow_runtime::sys::frame_csv, (Frame) -> Str, value "frame"),
    row!("sys.frame_json", "sys", "frame_json", crow_runtime::sys::frame_json, (Frame) -> Str, never),
    Row { feature: Some(("crow_runtime", "parquet")), ..row!("sys.frame_parquet", "sys", "frame_parquet", crow_runtime::sys::frame_parquet, (Frame) -> Bytes, value "frame") },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), ..row!("sys.tensor_of_frame", "sys", "tensor_of_frame", crow_runtime::sys::tensor_of_frame, (Frame) -> Tensor, value "load") },
    // the protocol, written in Crow (src/modules/sys.co): its functions
    // are its declaration (`exchange`, `put`, `upgrade`, the formats; phase
    // 2 of docs/extending-crow.md), but a remote source read with a
    // reader's options is an effect the session layer holds and a page's
    // source, and keeps its row until effects are declared on the layer's
    // own rows (phase 3)
    Row { props: &[Prop::Does(crate::effects::Effect::Network), Prop::Source], ..crow!("sys.fetch", "sys", "fetch", (Str, TextMap, TextMap, Float) -> Answer, value "sys") },
    // a value packed as MessagePack (src/modules/web.co): a value of any
    // kind, kept as it is — an int beside a float, which the number rule
    // would join into one function of floats — a parameter of no kind the
    // language can declare yet (docs/extending-crow.md, phase 4), so the
    // row stays, handing the function the value's JSON text
    crow!("web.msgpack", "web", "msgpack", (Encoded) -> Response, never),
    // the WebSocket client written in Crow (src/modules/ws.co): `socket`
    // and `close` are its functions; a message sent or received is an
    // effect, and keeps its row (phase 3), named as the program calls it,
    // its function by the row's id
    Row { props: &[Prop::Does(crate::effects::Effect::Network)], ..crow!("web.ws.send_text", "web.ws", "send", (Socket, Str) -> Str, value "socket") },
    Row { props: &[Prop::Does(crate::effects::Effect::Network), Prop::Does(crate::effects::Effect::Waits)], ..crow!("web.ws.recv_text", "web.ws", "recv", (Socket) -> Str, value "socket") },
    Row { props: &[Prop::Does(crate::effects::Effect::Network), Prop::Does(crate::effects::Effect::Waits)], ..crow!("web.ws.recv_for", "web.ws", "recv", (Socket, Float) -> Str, value "socket") },
    // mail written in Crow (src/modules/mail.co): SMTP over the sys layer
    Row { props: &[Prop::Does(crate::effects::Effect::Mail)], ..crow!("web.mail.mail", "web.mail", "mail", (Mailer, Str, Str, Str) -> Bool, value "mail") },
    // frame (a column the frame lacks, an aggregation or a join it does not
    // know, a mask or a count that does not fit: the reason the program
    // stops with, as df.col of a missing column is — the interpreter had
    // answered error values its type said could not happen, the compiled
    // program stopped or panicked; values with cannot fit are an error
    // value, as text that is no time is)
    row!("frame.new", "frame", "new", crow_runtime::frames::new, () -> Frame, never),
    // a column written where the frame lies — `df.name = values`: a new
    // column, or one replaced; a scalar repeated down the rows; the
    // literal's columns too (`frame.new[a = xs]` is `new[]` written at each
    // name) — and a column read by a name the program computes
    // (`df.`name``): the language's own forms, found by their property and
    // named in a refusal as the program writes them (spec, Frames). A
    // write whose values are not as long as the rows stops, as a write at
    // a position that is not there does (`with` had answered an error
    // value, which a write statement would hold unseen).
    Row { props: &[Prop::ColumnWrite], form: "`df.name = values`", ..row!("core.__set_col", "", "__set_col", crow_runtime::frames::with, (Frame, Str, Cells) -> Frame, stops) },
    Row { props: &[Prop::ColumnRead], form: "`df.`name``", ..row!("core.__col", "", "__col", crow_runtime::frames::column, (Frame, Str) -> Column, stops) },
    Row { props: &[Prop::Binds(1)], ..row!("frame.select", "frame", "select", crow_runtime::frames::select, (Frame, Strs) -> Frame, stops) },
    // `f[mask]`, a frame's rows picked by a mask (the lowering's, at a
    // frame's index), and its fused form where the mask compares the
    // frame's own columns, computed in one pass (lower_ir/fuse.rs)
    row!("core.__pick_frame", "", "__pick_frame", crow_runtime::frames::pick_rows, (Frame, Bools) -> Frame, stops),
    row!("core.__pick_where", "", "__pick_where", crow_runtime::mask::pick_where, (Frame, Str, Strs, Scalars) -> Frame, stops),
    Row { props: &[Prop::InPlace], ..row!("frame.sort", "frame", "sort", crow_runtime::frames::sort, (Frame, Str) -> Frame, stops) },
    Row { props: &[Prop::InPlace], options: &[("desc", Preset::Asked)], ..row!("frame.sort_by", "frame", "sort", crow_runtime::frames::sort_by, (Frame, Str, Bool) -> Frame, stops) },
    Row { props: &[Prop::InPlace], ..row!("frame.sort_keys", "frame", "sort", crow_runtime::frames::sort_keys, (Frame, Strs) -> Frame, stops) },
    Row { props: &[Prop::InPlace], options: &[("desc", Preset::Asked)], ..row!("frame.sort_keys_by", "frame", "sort", crow_runtime::frames::sort_keys_by, (Frame, Strs, Bool) -> Frame, stops) },
    Row { props: &[Prop::InPlace], ..row!("frame.sort_keys_each", "frame", "sort", crow_runtime::frames::sort_keys_each, (Frame, Strs, Bools) -> Frame, stops) },
    row!("frame.group", "frame", "group", crow_runtime::frames::group, (Frame, Str, Lists) -> Frame, stops),
    row!("frame.group_keys", "frame", "group", crow_runtime::frames::group_keys, (Frame, Strs, Lists) -> Frame, stops),
    row!("frame.pivot", "frame", "pivot", crow_runtime::frames::pivot, (Frame, Str, Str, Str) -> Frame, stops),
    Row { options: &[("n", Preset::Int(0)), ("by", Preset::List), ("span", Preset::Float(0.0)), ("on", Preset::Str(""))], ..row!("frame.window", "frame", "window", crow_runtime::frames::window, (Frame, Str, Str, Int, Strs, Float, Str) -> Column, stops) },
    Row { options: &[("n", Preset::Int(0)), ("by", Preset::List), ("span", Preset::Float(0.0)), ("on", Preset::Str(""))], ..row!("frame.window_one", "frame", "window", crow_runtime::frames::window_one, (Frame, Str, Str, Int, Str, Float, Str) -> Column, stops) },
    row!("frame.join", "frame", "join", crow_runtime::frames::join, (Frame, Frame, Str) -> Frame, stops),
    row!("frame.join_how", "frame", "join", crow_runtime::frames::join_how, (Frame, Frame, Str, Str) -> Frame, stops),
    row!("frame.join_keys", "frame", "join", crow_runtime::frames::join_keys, (Frame, Frame, Strs) -> Frame, stops),
    row!("frame.join_keys_how", "frame", "join", crow_runtime::frames::join_keys_how, (Frame, Frame, Strs, Str) -> Frame, stops),
    row!("frame.unpivot", "frame", "unpivot", crow_runtime::frames::unpivot, (Frame, Strs, Strs) -> Frame, stops),
    row!("frame.unpivot_one", "frame", "unpivot", crow_runtime::frames::unpivot_one, (Frame, Str, Strs) -> Frame, stops),
    row!("frame.complete", "frame", "complete", crow_runtime::frames::complete, (Frame) -> Frame, stops),
    row!("frame.complete_one", "frame", "complete", crow_runtime::frames::complete_one, (Frame, Str) -> Frame, stops),
    row!("frame.complete_in", "frame", "complete", crow_runtime::frames::complete_in, (Frame, Strs) -> Frame, stops),
    row!("frame.sample", "frame", "sample", crow_runtime::frames::sample, (Frame, Int) -> Frame, stops),
    row!("frame.bucket", "frame", "bucket", crow_runtime::frames::bucket, (Column, Float) -> Column, stops),
    Row { props: &[Prop::Mask(0)], ..row!("frame.where", "frame", "where", crow_runtime::frames::choose, (Bools, Cells, Cells) -> Column, stops) },
    Row { props: &[Prop::Differentiable], ..row!("frame.int_column", "", "int", crow_runtime::frames::int_column, (Column) -> Column, stops) },
    Row { props: &[Prop::Differentiable], ..row!("frame.float_column", "", "float", crow_runtime::frames::float_column, (Column) -> Column, stops) },
    row!("frame.moment_column", "", "moment", crow_runtime::frames::moment_column, (Column) -> Column, stops),
    Row { props: &[Prop::InPlace], ..row!("frame.lower_column", "", "lower", crow_runtime::verbs::lower_column, (Column) -> Column, stops) },
    Row { props: &[Prop::InPlace], ..row!("frame.upper_column", "", "upper", crow_runtime::verbs::upper_column, (Column) -> Column, stops) },
    row!("frame.trim_column", "", "trim", crow_runtime::verbs::trim_column, (Column) -> Column, stops),
    Row { props: &[Prop::InPlace], ..row!("frame.replace_column", "", "replace", crow_runtime::verbs::replace_column, (Column, Str, Str) -> Column, stops) },
    row!("frame.sub_column", "", "sub", crow_runtime::verbs::sub_column, (Column, Str, Str) -> Column, stops),
    row!("frame.contains_column", "", "contains", crow_runtime::verbs::contains_column, (Column, Str) -> Bools, stops),
    row!("frame.starts_column", "", "starts", crow_runtime::verbs::starts_column, (Column, Str) -> Bools, stops),
    row!("frame.ends_column", "", "ends", crow_runtime::verbs::ends_column, (Column, Str) -> Bools, stops),
    row!("frame.stack", "frame", "stack", crow_runtime::frames::stack, (Frame, Frame) -> Frame, stops),
    row!("frame.head", "frame", "head", crow_runtime::frames::head, (Frame, Int) -> Frame, stops),
    row!("frame.tail", "frame", "tail", crow_runtime::frames::tail, (Frame, Int) -> Frame, stops),
    row!("frame.fill", "frame", "fill", crow_runtime::frames::fill, (Frame, Str, Cell, Str) -> Frame, stops),
    row!("frame.unique", "frame", "unique", crow_runtime::frames::unique, (Frame, Str) -> Column, stops),
    row!("frame.unique_rows", "frame", "unique", crow_runtime::frames::unique_rows, (Frame, Strs) -> Frame, stops),
    row!("frame.rename", "frame", "rename", crow_runtime::frames::rename, (Frame, Str, Str) -> Frame, stops),
    row!("frame.stats", "frame", "stats", crow_runtime::frames::stats, (Frame) -> Str, never),
    // the modules' files: effects, as the file system's are. `save`'s
    // failure is an error value of kind 'frame', as `load`'s is (compiled,
    // it stopped the program)
    // the readers, written in Crow on the sys layer (src/modules/frame.co,
    // io.co, math.co): a file, a URL or an S3 object
    Row { props: &[Prop::Table(0), Prop::Does(crate::effects::Effect::Files)], ..crow!("frame.load", "frame", "load", (Str) -> Frame, value "frame") },
    Row { props: &[Prop::Table(0), Prop::Does(crate::effects::Effect::Files)], options: &[("headers", Preset::Map), ("query", Preset::Map), ("timeout", Preset::Float(0.0))], ..crow!("frame.load_asked", "frame", "load", (Str, TextMap, TextMap, Float) -> Frame, value "frame") },
    Row { props: &[Prop::Table(1), Prop::Does(crate::effects::Effect::FilesWrite)], ..crow!("frame.save", "frame", "save", (Frame, Str) -> Str, value "frame") },
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], ..crow!("io.csv", "io", "csv", (Str) -> TextRows, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], options: &[("headers", Preset::Map), ("query", Preset::Map), ("timeout", Preset::Float(0.0))], ..crow!("io.csv_asked", "io", "csv", (Str, TextMap, TextMap, Float) -> TextRows, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], ..crow!("io.lines", "io", "lines", (Str) -> Strs, value "io") },
    Row { props: &[Prop::Does(crate::effects::Effect::Files)], options: &[("headers", Preset::Map), ("query", Preset::Map), ("timeout", Preset::Float(0.0))], ..crow!("io.lines_asked", "io", "lines", (Str, TextMap, TextMap, Float) -> Strs, value "io") },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), props: &[Prop::Table(0), Prop::Does(crate::effects::Effect::Files)], ..crow!("math.load", "math", "load", (Str) -> Tensor, value "load") },
    Row { shape: Shape::Dims(&[Dim::Open, Dim::Open]), props: &[Prop::Table(0), Prop::Does(crate::effects::Effect::Files)], options: &[("headers", Preset::Map), ("query", Preset::Map), ("timeout", Preset::Float(0.0))], ..crow!("math.load_asked", "math", "load", (Str, TextMap, TextMap, Float) -> Tensor, value "load") },
    // doc: markdown rendered by the kernel
    row!("doc.markdown", "doc", "markdown", crow_runtime::markdown::to_html, (Str) -> Markup, never),
    Row { columns: &[("title", Kind::Str), ("path", Kind::Str), ("depth", Kind::Int), ("part", Kind::Str)], ..row!("doc.chapters", "doc", "chapters", crow_runtime::markdown::chapters_frame, (Str) -> Frame, never) },
    row!("doc.menu", "doc", "menu", crow_runtime::markdown::menu_from_frame, (Frame, Str) -> Markup, never),
    row!("doc.pager", "doc", "pager", crow_runtime::markdown::pager_from_frame, (Frame, Str) -> Markup, never),
    row!("doc.plain", "doc", "plain", crow_runtime::markdown::plain_text, (Str) -> Str, never),
    row!("doc.document", "doc", "document", crow_runtime::markdown::document, (Str, Str) -> Str, never),
    // `use bits`: an int's bits as two's complement of any width, and a
    // character's code (crow_runtime::bits); the width pass hands a number
    // past 64 bits, or a shift that may pass them, to the `__…_big` forms
    Row { range: IntRange::And, big: Some("core.__bitand_big"), ..row!("bits.bitand", "bits", "bitand", crow_runtime::bits::and, (Whole, Whole) -> Int, never) },
    Row { range: IntRange::OrXor, big: Some("core.__bitor_big"), ..row!("bits.bitor", "bits", "bitor", crow_runtime::bits::or, (Whole, Whole) -> Int, never) },
    Row { range: IntRange::OrXor, big: Some("core.__bitxor_big"), ..row!("bits.bitxor", "bits", "bitxor", crow_runtime::bits::xor, (Whole, Whole) -> Int, never) },
    Row { range: IntRange::Count, big: Some("core.__bitcount_big"), ..row!("bits.bitcount", "bits", "bitcount", crow_runtime::bits::count, (Whole) -> Int, never) },
    Row { range: IntRange::Shift, big: Some("core.__shift_big"), ..row!("bits.shift", "bits", "shift", crow_runtime::bits::shift, (Whole, Whole) -> Int, stops) },
    Row { range: IntRange::Code, ..row!("bits.code", "bits", "code", crow_runtime::bits::code, (Str) -> Int, stops) },
    row!("bits.char", "bits", "char", crow_runtime::bits::char, (Whole) -> Str, stops),
];

/// A form: a name a program calls that no runtime row answers — the
/// lowering and the backends give it its meaning (`route`, `page`, `serve`,
/// the ui controls, the db verbs, the test harness, the core's own forms).
/// Listed with the rows for what the table says of every name a program
/// can call: its counts of arguments and its options, and nothing else. A
/// form's kinds are not its contract (`Shown` where any value is taken;
/// docs/extending-crow.md gives each its kind in a later phase): no
/// lookup by kind reads FORMS, and the self-check writes no case from them.
pub const IN_FORM: &str = "form";

fn a_form(name: &str) -> String {
    format!("{name} is a form of its own, called through no row: a bug in Crow")
}

macro_rules! form {
    ($id:literal, $module:literal, $name:literal, ($($k:ident),*)) => {
        Row {
            id: $id, name: $name, module: $module, path: IN_FORM,
            params: &[$(Kind::$k),*], ret: Kind::Unit, fails: Fails::Never, options: &[], written: "", form: "", props: &[], taken: None, big: None, range: IntRange::I64, shape: Shape::Open, columns: &[], provider: None, feature: None,
            call: |_args: &[CrowValue], _trace: &str| Err(a_form($name)),
            take: |_args: Vec<CrowValue>, _trace: &str| Err(a_form($name)),
        }
    };
}

pub static FORMS: &[Row] = &[
    // the core's own forms
    form!("core.type", "", "type", (Shown)),
    form!("core.error", "", "error", (Str)),
    form!("core.error_kind", "", "error", (Str, Str)),
    Row { props: &[Prop::Ordered], ..form!("core.assert", "", "assert", (Shown, Shows)) },
    Row { props: &[Prop::Varies], ..form!("core.captured", "", "captured", (Shown)) },
    form!("core.__known", "", "__known", (Shown)),
    // a function value compared with a function written (the dispatch
    // `describe` and `apply` lower to): by name interpreted, by the
    // function's address or its name compiled
    Row { params: &[Kind::Shown, Kind::Shown], ret: Kind::Bool, props: &[Prop::Answers], ..form!("core.__same_fn", "", "__same_fn", ()) },
    form!("core.cpu", "", "cpu", (Int)),
    Row { feature: Some(("crow_runtime", "gpu")), ..form!("core.gpu", "", "gpu", ()) },
    Row { feature: Some(("crow_runtime", "gpu")), ..form!("core.gpu_count", "", "gpu", (Int)) },
    form!("core.grad", "", "grad", (Shown)),
    // a predicate over what a loop walks (the regex form of `find`, over
    // text, beside it): each declares the function it takes
    Row { params: &[Kind::Walked, Kind::FnOf(&PRED)], ret: Kind::Item, fails: Fails::Value("find"), props: &[Prop::Answers], ..form!("core.find_by", "", "find", ()) },
    Row { params: &[Kind::Str, Kind::Str], ret: Kind::Found, fails: Fails::Value("find"), props: &[Prop::Answers], ..form!("core.find", "", "find", ()) },
    Row { params: &[Kind::Walked, Kind::FnOf(&PRED)], ret: Kind::Bool, props: &[Prop::Answers], ..form!("core.any", "", "any", ()) },
    Row { params: &[Kind::Walked, Kind::FnOf(&PRED)], ret: Kind::Bool, props: &[Prop::Answers], ..form!("core.all", "", "all", ()) },
    Row { props: &[Prop::Does(crate::effects::Effect::Session)], ..form!("core.notice", "", "notice", (Str)) },
    form!("core.__notice_text", "", "__notice_text", ()),
    form!("core.__notice_at", "", "__notice_at", ()),
    Row { params: &[Kind::FnOf(&DEFERRED)], ret: Kind::Bool, props: &[Prop::Ordered, Prop::Varies, Prop::Answers], ..form!("core.running", "", "running", ()) },
    // chart[frame, x = 'ts', candles = [], lines = [], bars_y = '', marker = 0, tz = 'UTC', height = 320, panes = [], levels = 0, points = 0, seek = 0]
    Row {
        options: &[
            ("x", Preset::Str("ts")), ("candles", Preset::List), ("lines", Preset::List), ("bars_y", Preset::Str("")), ("marker", Preset::Int(0)),
            ("tz", Preset::Str("UTC")), ("height", Preset::Int(320)), ("panes", Preset::List), ("levels", Preset::Int(0)), ("points", Preset::Int(0)), ("seek", Preset::Event(&SEEK)),
        ],
        ..form!("core.chart", "", "chart", (Frame, Str, Strs, Strs, Str, Int, Str, Int, Shown, Shown, Shown, Shown))
    },
    // web: routes, the server, the responses, pages, the signed-in reader
    Row { params: &[Kind::Str, Kind::FnOf(&HANDLER)], props: &[Prop::Does(crate::effects::Effect::Serving)], ..form!("web.route", "web", "route", ()) },
    Row { params: &[Kind::Str, Kind::Str, Kind::FnOf(&HANDLER)], props: &[Prop::Does(crate::effects::Effect::Serving)], ..form!("web.route_method", "web", "route", ()) },
    Row { params: &[Kind::Str, Kind::Str, Kind::FnOf(&HANDLER), Kind::Int], props: &[Prop::Does(crate::effects::Effect::Serving)], ..form!("web.route_limited", "web", "route", ()) },
    // serve[port, refresh = 0, budget = 0, theme = '', density = '', accent = '', wide = false, keep = false, uploads = 10, cors = [], files = '']
    Row {
        props: &[Prop::Does(crate::effects::Effect::Serving)],
        options: &[
            ("refresh", Preset::Int(0)), ("budget", Preset::Int(0)), ("theme", Preset::Str("")), ("density", Preset::Str("")), ("accent", Preset::Str("")),
            ("wide", Preset::Bool(false)), ("keep", Preset::Bool(false)), ("uploads", Preset::Int(10)), ("cors", Preset::List), ("files", Preset::Str("")),
        ],
        ..form!("web.serve", "web", "serve", (Int, Int, Int, Str, Str, Str, Bool, Bool, Int, Strs, Str))
    },
    // the standard modules' two operations over functions (section 7 of
    // docs/extending-crow.md): a function's description, and a call by
    // name — each lowered to what inference knows, so both modes agree
    Row { params: &[Kind::Shown], ret: Kind::Json, fails: Fails::Value("describe"), props: &[Prop::Answers], ..form!("sys.describe", "sys", "describe", ()) },
    Row { params: &[Kind::Shown, Kind::Json], ret: Kind::Str, fails: Fails::Value("call"), props: &[Prop::Answers], ..form!("sys.apply", "sys", "apply", ()) },
    form!("web.respond", "web", "respond", (Int, Shown)),
    form!("web.redirect", "web", "redirect", (Str)),
    form!("web.html", "web", "html", (Shown)),
    form!("web.text", "web", "text", (Shown)),
    Row { feature: Some(("crow_web", "arrow_ipc")), ..form!("web.arrow", "web", "arrow", (Frame)) },
    form!("web.bytes", "web", "bytes", (Shown)),
    Row { params: &[Kind::FnOf(&FEED), Kind::Shows], ret: Kind::Response, props: &[Prop::Answers], ..form!("web.stream", "web", "stream", ()) },
    Row { props: &[Prop::Does(crate::effects::Effect::Serving)], ..form!("web.page", "web", "page", (Str, Shown)) },
    Row { props: &[Prop::Does(crate::effects::Effect::Serving)], ..form!("web.page_with", "web", "page", (Str, Shown, Shown)) },
    Row { props: &[Prop::Does(crate::effects::Effect::Session)], ..form!("web.sign_in", "web", "sign_in", (Shown)) },
    Row { props: &[Prop::Does(crate::effects::Effect::Session)], ..form!("web.sign_out", "web", "sign_out", ()) },
    Row { props: &[Prop::Does(crate::effects::Effect::Session)], ..form!("web.download", "web", "download", (Str, Shown)) },
    form!("web.__reader", "web", "__reader", ()),
    // (the page's signed-in reader, written bare: the lowering's `__reader[]`)
    form!("web.reader", "web", "reader", ()),
    // the test harness: app[view, width = 1280, wide = false]
    Row { props: &[Prop::TestOnly], options: &[("width", Preset::Int(1280)), ("wide", Preset::Bool(false))], ..form!("web.app", "web", "app", (Shown, Int, Bool)) },
    // ui: a page's data sources and its controls
    Row { props: &[Prop::Does(crate::effects::Effect::Network), Prop::Source], ..form!("ui.fetch", "ui", "fetch", (Str)) },
    Row { props: &[Prop::Does(crate::effects::Effect::Network), Prop::Source], ..form!("ui.poll", "ui", "poll", (Str, Float)) },
    Row { props: &[Prop::Binds(1)], options: &[("type", Preset::Str("")), ("lines", Preset::Int(0))], ..form!("ui.field", "ui", "field", (Str, Shown, Str, Int)) },
    Row { props: &[Prop::Binds(1)], options: &[("min", Preset::Int(0)), ("max", Preset::Int(100)), ("step", Preset::Int(1))], ..form!("ui.slider", "ui", "slider", (Str, Shown, Int, Int, Int)) },
    Row { props: &[Prop::Binds(1)], ..form!("ui.checkbox", "ui", "checkbox", (Str, Shown)) },
    Row { props: &[Prop::Binds(1)], ..form!("ui.toggle", "ui", "toggle", (Str, Shown)) },
    Row { props: &[Prop::Binds(1)], ..form!("ui.radio", "ui", "radio", (Strs, Shown)) },
    Row { props: &[Prop::Binds(1)], ..form!("ui.tabs", "ui", "tabs", (Strs, Shown)) },
    Row { options: &[("max", Preset::Float(1.0))], ..form!("ui.progress", "ui", "progress", (Float, Float)) },
    Row { options: &[("size", Preset::Str("m")), ("color", Preset::Str(""))], ..form!("ui.icon", "ui", "icon", (Str, Str, Str)) },
    // db: its verbs that are forms of their own
    Row { props: &[Prop::Does(crate::effects::Effect::DbRead), Prop::Connects], ..form!("db.connect", "db", "connect", (Str)) },
    // `query` runs every statement and answers a frame: a read, or — by what
    // its provider says of the literal, a write for a statement built as
    // the program runs — its writing twin, hidden, written as `query`
    Row { provider: Some(&Provider { connection: 0, statement: 1, columns: crow_db::describe::columns, writes: crow_db::describe::writes }), props: &[Prop::Does(crate::effects::Effect::DbRead)], ..form!("db.query", "db", "query", (Shown, Str, Shows)) },
    Row { written: "query", provider: Some(&Provider { connection: 0, statement: 1, columns: crow_db::describe::columns, writes: crow_db::describe::writes }), props: &[Prop::Does(crate::effects::Effect::DbWrite)], ..form!("db.__query_write", "db", "__query_write", (Shown, Str, Shows)) },
    // (a bulk insert the database refuses answers why, as a statement's
    // refusal does: an error value of kind 'db')
    Row { fails: Fails::Value("db"), props: &[Prop::Does(crate::effects::Effect::DbWrite)], ..form!("db.insert", "db", "insert", (Shows)) },
    Row { params: &[Kind::Db, Kind::FnOf(&TX_BODY)], props: &[Prop::Does(crate::effects::Effect::DbWrite)], ..form!("db.transact", "db", "transact", ()) },
    form!("db.describe", "db", "describe", (Shows)),
    // (counted by nothing, as before: `later`, a control, the harness verbs)
    Row { params: &[Kind::FnOf(&DEFERRED), Kind::Shows], props: &[Prop::Does(crate::effects::Effect::Session), Prop::Answers], ..form!("core.later", "", "later", ()) },
    Row { props: &[Prop::Binds(1), Prop::Unreserved], ..form!("ui.select", "ui", "select", (Shows)) },
    Row { props: &[Prop::TestOnly], ..form!("web.click", "web", "click", (Shows)) },
    Row { props: &[Prop::TestOnly], ..form!("web.elements", "web", "elements", (Shows)) },
    Row { props: &[Prop::TestOnly], ..form!("web.change", "web", "change", (Shows)) },
    Row { props: &[Prop::TestOnly], ..form!("web.box", "web", "box", (Shows)) },
    Row { props: &[Prop::TestOnly], ..form!("web.fits", "web", "fits", (Shows)) },
    Row { props: &[Prop::TestOnly], ..form!("web.style", "web", "style", (Shows)) },
    Row { props: &[Prop::TestOnly], ..form!("web.downloads", "web", "downloads", (Shows)) },
    Row { props: &[Prop::TestOnly], ..form!("web.mails", "web", "mails", (Shows)) },
];

/// Every row of a name, the forms included: what the table says of a name
/// a program can call — by its name, or by the name a hidden form is
/// written as, in the table's order. Read from an index built once: the
/// interpreter asks after every builtin call (whether its answer is
/// stamped, whether it grows in place, whether a rehearsal records it),
/// and a scan of the table per question had made it two to four times
/// slower when the hand-kept lists became the table's (e967633: n-body
/// interpreted 2.0 s to 8.0 s).
pub fn rows_named(name: &str) -> impl Iterator<Item = &'static Row> {
    static BY_NAME: std::sync::OnceLock<std::collections::HashMap<&'static str, Vec<&'static Row>>> = std::sync::OnceLock::new();
    let index = BY_NAME.get_or_init(|| {
        let mut index: std::collections::HashMap<&'static str, Vec<&'static Row>> = std::collections::HashMap::new();
        for r in ROWS.iter().chain(FORMS.iter()) {
            index.entry(r.name).or_default().push(r);
            if !r.written.is_empty() && r.written != r.name {
                index.entry(r.written).or_default().push(r);
            }
        }
        index
    });
    index.get(name).map_or(&[][..], |rows| rows.as_slice()).iter().copied()
}

/// A module's rows of a name, the forms included.
pub fn rows_named_in(module: &str, name: &str) -> impl Iterator<Item = &'static Row> {
    rows_named(name).filter(move |r| r.module == module)
}

/// Whether a row of the name has the property (`Prop`): what a builtin
/// does is said of its name, as the lists it replaces were read.
pub fn has(name: &str, prop: Prop) -> bool {
    rows_named(name).any(|r| r.props.contains(&prop))
}

/// The kinds of effect a builtin of the name has, over every row of the
/// name (`Prop::Does`); a name written with its module (`frame.load`) is
/// read by its last part, as the rows are named. Empty for a pure builtin.
pub fn effects_of(name: &str) -> Vec<crate::effects::Effect> {
    let bare = name.rsplit('.').next().unwrap_or(name);
    // (a hidden form written as the name — `query`'s writing twin — has
    // kinds of its own, read by its own name)
    let mut out: Vec<crate::effects::Effect> = rows_named(bare)
        .filter(|r| r.name == bare)
        .flat_map(|r| r.props.iter())
        .filter_map(|p| match p {
            Prop::Does(e) => Some(*e),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Whether a builtin of the name reaches outside the program's values
/// (`Effect::outside`: any kind but a draw): its call lowers to
/// `Expr::Effect`, which the session layer performs.
pub fn is_effect(name: &str) -> bool {
    effect_is(name, crate::effects::Effect::outside)
}

/// The kinds of effect the module's own rows of the name have: a call
/// written with its module (`ui.input`, `db.query`) is the module's row,
/// never a namesake's elsewhere (the core's `input` waits on the reader;
/// ui's is an element).
pub fn effects_of_in(module: &str, name: &str) -> Vec<crate::effects::Effect> {
    let mut out: Vec<crate::effects::Effect> = rows_named_in(module, name)
        .filter(|r| r.name == name)
        .flat_map(|r| r.props.iter())
        .filter_map(|p| match p {
            Prop::Does(e) => Some(*e),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Whether the module's own row of the name reaches outside the program's
/// values (`is_effect`, for a call written with its module).
pub fn is_effect_in(module: &str, name: &str) -> bool {
    effects_of_in(module, name).into_iter().any(crate::effects::Effect::outside)
}

/// Whether the name's kinds include one the predicate holds: `held`,
/// `stamps`, `recorded`, `acts_for_session` (crate::effects::Effect).
pub fn effect_is(name: &str, which: fn(crate::effects::Effect) -> bool) -> bool {
    // (read as effects_of reads the kinds, with nothing gathered: asked
    // after every builtin call, whether its answer is stamped)
    let bare = name.rsplit('.').next().unwrap_or(name);
    rows_named(bare)
        .filter(|r| r.name == bare)
        .flat_map(|r| r.props.iter())
        .any(|p| matches!(p, Prop::Does(e) if which(*e)))
}

/// Held to the handler's commit (its kind says: a session's, a database
/// write, mail).
pub fn held(name: &str) -> bool {
    effect_is(name, crate::effects::Effect::held)
}

/// Its answer carries the moment it was read.
pub fn stamps(name: &str) -> bool {
    effect_is(name, crate::effects::Effect::stamps)
}

/// Whether a row's answer is stamped, as its name's is (`stamps`): worked
/// out once for every row of the table and read by the row's place in it,
/// since the interpreter asks after every builtin call it makes and a
/// lookup by name there cost n-body a twelfth of its time.
pub fn row_stamps(row: &'static Row) -> bool {
    static FLAGS: std::sync::OnceLock<(Vec<bool>, Vec<bool>)> = std::sync::OnceLock::new();
    let (rows, forms) = FLAGS.get_or_init(|| (ROWS.iter().map(|r| stamps(r.name)).collect(), FORMS.iter().map(|r| stamps(r.name)).collect()));
    let size = std::mem::size_of::<Row>();
    let at = row as *const Row as usize;
    for (table, flags) in [(ROWS, rows), (FORMS, forms)] {
        let start = table.as_ptr() as usize;
        if size > 0 && at >= start && at < start + table.len() * size {
            return flags[(at - start) / size];
        }
    }
    stamps(row.name)
}

/// Waits: a view or a handler reaching it freezes the live loop.
pub fn waits(name: &str) -> bool {
    effects_of(name).contains(&crate::effects::Effect::Waits)
}

/// Whether a loop over a call of the name keeps its order: an effect of
/// any kind (a draw from the generator among them), or a builtin whose
/// order shows (`Prop::Ordered`).
pub fn ordered(name: &str) -> bool {
    !effects_of(name).is_empty() || has(name, Prop::Ordered)
}

/// Whether a function `crow check` probes as pure may not call the name:
/// an effect of any kind, or a builtin answering by the run (`Prop::Varies`).
pub fn impure(name: &str) -> bool {
    !effects_of(name).is_empty() || has(name, Prop::Varies)
}

/// The names with the kind, each once.
pub fn with_effect(e: crate::effects::Effect) -> Vec<&'static str> {
    with(Prop::Does(e))
}

/// The names with the property, each once.
pub fn with(prop: Prop) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for r in ROWS.iter().chain(FORMS.iter()) {
        if r.props.contains(&prop) && !names.contains(&r.name) {
            names.push(r.name);
        }
    }
    names
}

/// Whether a call of the name with this many arguments is a page's data
/// source (`fetch[url]`, `poll[url, seconds]`).
pub fn source(name: &str, argc: usize) -> bool {
    rows_named(name).any(|r| r.props.contains(&Prop::Source) && r.params.len() == argc)
}

/// The argument a control of the name binds to a fact, where it is one.
pub fn binds(name: &str) -> Option<usize> {
    rows_named(name).flat_map(|r| r.props.iter()).find_map(|p| match p {
        Prop::Binds(i) => Some(*i),
        _ => None,
    })
}

/// Whether any row of the name declares options.
pub fn takes_options(name: &str) -> bool {
    rows_named(name).any(|r| !r.options.is_empty())
}

/// Whether a form of the name takes this many arguments: all of its
/// parameters, those before its options (a call not filled in yet), or at
/// least those before a rest. A name no form has is counted by its rows
/// (`has_core_rows`), or by nothing.
pub fn arity_ok(name: &str, argc: usize) -> bool {
    let mut forms = FORMS.iter().filter(|r| r.name == name).peekable();
    if forms.peek().is_none() {
        return true;
    }
    forms.any(|r| {
        let all = r.params.len();
        if r.params.last().is_some_and(|k| k.is_rest()) {
            return argc + 1 >= all;
        }
        argc == all || argc == all - r.options.len()
    })
}


/// The row of an id (the tests name rows so).
#[cfg(test)]
pub fn by_id(id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.id == id)
}

/// The row or form of a name that declares its frame's columns or a
/// provider of them, in the module or in any.
pub fn frame_declared(module: Option<&str>, name: &str) -> Option<&'static Row> {
    ROWS.iter().chain(FORMS).find(|r| r.name == name && module.is_none_or(|m| r.module == m) && (!r.columns.is_empty() || r.provider.is_some()))
}

/// The form of a name that takes a statement — one with a provider — in
/// the module or in any; the hidden writing twin is never found by name.
pub fn statement_form(module: Option<&str>, name: &str) -> Option<&'static Row> {
    FORMS.iter().find(|r| r.name == name && r.written.is_empty() && module.is_none_or(|m| r.module == m) && r.provider.is_some())
}

/// The hidden form written as this row's name, in its module: a statement
/// form's writing twin.
pub fn writing_twin(row: &'static Row) -> Option<&'static Row> {
    FORMS.iter().find(|r| r.module == row.module && r.written == row.name && r.id != row.id)
}

/// Whether an effect's name (bare, or with its module) is a statement
/// form's: `query`, or its writing twin.
pub fn is_statement_effect(name: &str) -> bool {
    FORMS.iter().any(|f| f.provider.is_some() && (name == f.name || name == f.id))
}

/// Whether a name is the form that opens a connection (`Prop::Connects`).
pub fn opens_connection(module: Option<&str>, name: &str) -> bool {
    ROWS.iter().chain(FORMS).any(|r| r.name == name && module.is_none_or(|m| r.module == m) && r.props.contains(&Prop::Connects))
}

/// The row of an id: what a pass of the lowering writes itself (a fused
/// chain's `tensor.fused`).
pub fn row(id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.id == id)
}

/// The core builtin's row by its Crow name.
pub fn core(name: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.module.is_empty() && r.name == name)
}

/// A core builtin's row for a call with these arguments: the first whose
/// count and kinds take them (`min` of two ints, or of numbers). The
/// lowering reads the arguments from their types and the IR names the row
/// it picks; a call it leaves by name (an argument of unknown kind, a
/// parsed value) has its row picked from the values as the program runs.
pub fn core_row(name: &str, args: &[Arg]) -> Option<&'static Row> {
    ROWS.iter().filter(|r| r.module.is_empty() && r.name == name).find(|r| takes(r, args))
}

/// Whether a form of the name says it answers its failure as a value
/// (`fails`), as a row that can fail does: what the Rust backend reads of a
/// form that prints itself (db's `insert`).
pub fn form_fails(name: &str) -> bool {
    FORMS.iter().any(|r| r.name == name && matches!(r.fails, Fails::Value(_)))
}

/// A form that says what it answers (`Prop::Answers`), for a call with
/// these arguments: typed by the row as a runtime row's call is.
pub fn form_answer(name: &str, args: &[Arg]) -> Option<&'static Row> {
    FORMS.iter().filter(|r| r.name == name && r.props.contains(&Prop::Answers)).find(|r| takes(r, args))
}

/// The function parameters of a row of the name taking this many
/// arguments, each with its position and signature — the one declaration
/// of what a function handed to a builtin takes and answers.
pub fn fn_params(name: &str, argc: usize) -> Vec<(usize, &'static Sig)> {
    let bare = name.rsplit('.').next().unwrap_or(name);
    for r in rows_named(bare) {
        if spans(r.params, argc, r.name).is_err() {
            continue;
        }
        let found: Vec<(usize, &'static Sig)> = r
            .params
            .iter()
            .enumerate()
            .filter_map(|(i, k)| match k {
                Kind::FnOf(sig) => Some((i, *sig)),
                _ => None,
            })
            .collect();
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// The position of the row's walked argument, whose items a function
/// parameter's `Item` names.
pub fn walked_param(name: &str, argc: usize) -> Option<usize> {
    let bare = name.rsplit('.').next().unwrap_or(name);
    rows_named(bare).filter(|r| spans(r.params, argc, r.name).is_ok()).find_map(|r| r.params.iter().position(|k| matches!(k, Kind::Walked)))
}

/// The names a module's rows answer, each once, in order.
/// A module's forms by name (`db`'s verbs), each once.
pub fn module_forms(module: &str) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = FORMS.iter().filter(|r| r.module == module).map(|r| r.name).collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The names a module answers: its rows', and the functions its source
/// exports where it is written in Crow (`crate::modules::exports`).
pub fn module_names(module: &str) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = ROWS.iter().filter(|r| r.module == module).map(|r| r.name).collect();
    names.extend(crate::modules::exports(module).iter().copied());
    names.sort_unstable();
    names.dedup();
    names
}

/// Whether a module (`""` the core) answers the name with rows.
/// Where a call takes a mask — a list of bools, one per row: `where`'s
/// first argument (spec, Frames; an index takes one as well, `f[mask]`,
/// read where the index is lowered). A frame's column of bools there is
/// read as the mask it makes (`c eq true`). (Not every `Bools` parameter
/// is a mask: `sort`'s are a key's descending flags.)
pub fn takes_mask_at(name: &str, i: usize, argc: usize) -> bool {
    rows_named(name).any(|r| r.params.len() == argc && r.props.contains(&Prop::Mask(i)))
}

pub fn has_rows(module: &str, name: &str) -> bool {
    ROWS.iter().any(|r| r.module == module && r.name == name)
}

/// Whether the core answers the name with rows.
pub fn has_core_rows(name: &str) -> bool {
    core(name).is_some()
}

/// A form that declares its kinds, checked as a runtime row is: one taking
/// a function (`Kind::FnOf`), or one that says what it answers
/// (`Prop::Answers`).
fn checked_form(r: &Row) -> bool {
    r.params.iter().any(|k| matches!(k, Kind::FnOf(_))) || r.props.contains(&Prop::Answers)
}

/// Whether a form of the name declares its kinds (`checked_form`).
pub fn has_checked_form(name: &str) -> bool {
    FORMS.iter().any(|r| r.name == name && r.module.is_empty() && checked_form(r))
}

/// How a parsed JSON value is used where a builtin computes with it: read as
/// one type first — the words arithmetic on one says, and a list's items
/// one by one (the builtins that navigate it take it as it is: `len`,
/// `keys`, `values`, `has`, `str`, `int`, `float`, `encode`).
pub const READ_FIRST: &str = " — a value whose type the program does not know is read as one first: int[…], float[…] or str[…], a list item by item (for x in …)";

/// The refusal of a call that no row of its name — in the core (`""`) and
/// in the modules given — takes, by count or by kind; none when one takes
/// it, or when an argument's kind is not known before the program runs
/// (a JSON value's is known only then).
pub fn refusal(modules: &[&str], name: &str, args: &[Arg]) -> Option<String> {
    // (a form that declares its kinds — a function parameter, its answer —
    // is checked as a runtime row is)
    let rows: Vec<&Row> = ROWS.iter().chain(FORMS.iter().filter(|r| checked_form(r))).filter(|r| r.name == name && modules.contains(&r.module)).collect();
    if rows.is_empty() || rows.iter().any(|r| takes(r, args)) || args.contains(&Arg::Unknown) {
        return None;
    }
    // a parsed JSON value is read as one type before a builtin computes
    // with it (spec, json[]): the words say how
    let read = if args.contains(&Arg::Json) { READ_FIRST } else { "" };
    if !rows.iter().any(|r| spans(r.params, args.len(), name).is_ok()) {
        let mut counts: Vec<String> = Vec::new();
        for r in &rows {
            let fixed = r.params.iter().filter(|k| !k.is_rest()).count();
            let count = if r.params.iter().any(|k| k.is_rest()) { format!("at least {}", fixed) } else { fixed.to_string() };
            if !counts.contains(&count) {
                counts.push(count);
            }
        }
        return Some(miscount(name, &listed(&counts, "or"), args.len()));
    }
    let mut forms: Vec<String> = Vec::new();
    // a JSON value's and a column's own rows are the compiled program's
    // holding of what the interpreter holds as a list, a map or text
    for r in rows.iter().filter(|r| spans(r.params, args.len(), name).is_ok() && !r.params.iter().any(|k| matches!(k, Kind::Json | Kind::Column))) {
        let form = listed(&r.params.iter().map(|k| k.words().to_string()).collect::<Vec<_>>(), "and");
        if !forms.contains(&form) {
            forms.push(form);
        }
    }
    let got = listed(&args.iter().map(|a| a.words().to_string()).collect::<Vec<_>>(), "and");
    Some(format!("{} takes {}, got {}{}", form(name), listed(&forms, "or"), got, read))
}

/// `a`, `a and b`, `a, b or c`.
fn listed(items: &[String], last: &str) -> String {
    match items.split_last() {
        Some((tail, rest)) if !rest.is_empty() => format!("{} {} {}", rest.join(", "), last, tail),
        _ => items.join(""),
    }
}

/// What an argument is, as the choice among a name's rows reads it: from
/// the IR's type, where the lowering picks the row the IR names; or, for a
/// call the IR leaves by name, from the interpreter's value and the
/// emitter's type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arg {
    Int,
    Float,
    /// An exact decimal, and an exact ratio.
    Decimal,
    Fraction,
    /// A moment, and a list of them.
    Moment,
    Moments,
    Bytes,
    /// A list of bytes (a connection's chunks).
    Chunks,
    /// A parsed JSON value, whose kind shows only as it runs.
    Json,
    /// A tensor.
    Tensor,
    /// A list of ints.
    Ints,
    /// A list of floats.
    Floats,
    /// A list of tensors.
    Tensors,
    /// A list of lists of numbers: a matrix where a tensor is taken, each
    /// list a row (no parameter takes it as a list of tensors: `stack` of
    /// it is the matrix's rows restacked, as `stack` of a matrix is).
    Rows,
    Text,
    /// A list of text.
    Texts,
    Truth,
    /// A list of bools.
    Truths,
    /// A list of records.
    Records,
    /// A list of maps.
    Maps,
    /// A list of anything else: lists of text, functions.
    Items,
    /// A list of numbers whose items can fail: its numbers where numbers
    /// are taken, a failed item stopping the program, as a column's
    /// missing cell does.
    FallibleNumbers,
    /// A list of anything else whose items can fail.
    Fallible,
    /// A frame's column: numbers where numbers are taken, the compiled
    /// program's `CrowColumn` (the interpreter holds its cells as a list).
    Column,
    Map,
    Record,
    Sparse,
    Frame,
    /// A handle a module's rows make and take (`table::Handle`): a
    /// connection, a socket, a stream, a mailer, a response, an app.
    Handle(&'static Handle),
    /// A function.
    Fn,
    /// A function of a list literal's, called by name (`Ty::Fns`).
    Fns,
    /// A list whose items are not known here — an empty one, or one of
    /// parsed values: any parameter that takes a list takes it, and no
    /// other (it is a list, whatever it holds).
    AnyList,
    /// Not known here: any parameter takes it.
    Unknown,
}

impl Arg {
    /// What `json[…]` answers as a response (use web) rather than parses:
    /// a map or a record (an object), a list or a tensor (an array, a failed
    /// item `null` as `encode` writes it), a frame (its records). Text is
    /// parsed.
    pub fn json_response(&self) -> bool {
        matches!(
            self,
            // (a parsed value answers as it is: `json[json[text]]` serves
            // JSON a program holds as text)
            Arg::Json
                | Arg::Map
                | Arg::Record
                | Arg::Frame
                | Arg::Ints
                | Arg::Floats
                | Arg::Texts
                | Arg::Truths
                | Arg::Records
                | Arg::Maps
                | Arg::Items
                | Arg::Moments
                | Arg::Tensors
                | Arg::Rows
                | Arg::Tensor
                | Arg::Fallible
                | Arg::FallibleNumbers
                | Arg::AnyList
        )
    }

    /// What the argument is, as a refusal names it.
    pub fn words(self) -> &'static str {
        match self {
            Arg::Int => "an int",
            Arg::Float => "a number",
            Arg::Decimal => "a decimal",
            Arg::Fraction => "a fraction",
            Arg::Moment => "a moment",
            Arg::Moments => "a list of moments",
            Arg::Json => "a JSON value",
            Arg::Tensor => "numbers",
            Arg::Ints => "a list of ints",
            Arg::Floats => "a list of numbers",
            Arg::Tensors => "a list of tensors",
            Arg::Rows => "a list of lists of numbers",
            Arg::Text => "text",
            Arg::Texts => "a list of text",
            Arg::Chunks => "a list of bytes",
            Arg::Truth => "a bool",
            Arg::Truths => "a list of bools",
            Arg::Records => "a list of records",
            Arg::Maps => "a list of maps",
            Arg::Items => "a list",
            Arg::FallibleNumbers => "a list of numbers that can fail",
            Arg::Fallible => "a list whose items can fail",
            Arg::Column => "a frame's column",
            Arg::Map => "a map",
            Arg::Record => "a record",
            Arg::Sparse => "a sparse matrix",
            Arg::Bytes => "bytes",
            Arg::Handle(h) => h.word,
            Arg::Frame => "a frame",
            Arg::Fn => "a function",
            Arg::Fns => "a function called by name",
            Arg::AnyList => "a list",
            Arg::Unknown => "a value",
        }
    }

    pub fn of_type(t: &Ty) -> Arg {
        match t {
            // a value that can fail is handed on as its error before the
            // call: the call takes what it holds
            Ty::Result(inner) => Arg::of_type(inner),
            Ty::Int | Ty::Big => Arg::Int,
            Ty::Float => Arg::Float,
            Ty::Decimal => Arg::Decimal,
            Ty::Fraction => Arg::Fraction,
            Ty::Moment => Arg::Moment,
            Ty::Json => Arg::Json,
            // a mask of a matrix's shape (its rows of bools, deeper for more
            // axes) is numbers where numbers are taken, as a list of bools
            // is: a matrix of 1 and 0
            Ty::List(_) if t.truths_depth() >= 2 => Arg::Rows,
            Ty::List(inner) => match inner.as_ref() {
                Ty::Unknown => Arg::AnyList,
                Ty::Result(item) if matches!(item.as_ref(), Ty::Int | Ty::Float) => Arg::FallibleNumbers,
                Ty::Result(_) => Arg::Fallible,
                i => Arg::of_items(Arg::of_type(i)),
            },
            Ty::Tensor(..) => Arg::Tensor,
            Ty::Column(_) => Arg::Column,
            Ty::Str | Ty::Markup => Arg::Text,
            Ty::Bool => Arg::Truth,
            Ty::Map(..) => Arg::Map,
            Ty::Record(_) | Ty::OptRecord(_) => Arg::Record,
            Ty::Sparse => Arg::Sparse,
            Ty::Bytes => Arg::Bytes,
            Ty::Handle(h) => Arg::Handle(h),
            Ty::Frame(_) => Arg::Frame,
            Ty::Fn(..) => Arg::Fn,
            Ty::Fns(_) => Arg::Fns,
            // (no row takes a handle a program cannot hand on, a view's
            // node, nothing, or what is not known: each is read by the
            // value as the program runs)
            Ty::WebRequest | Ty::Node | Ty::Unit | Ty::Unknown => Arg::Unknown,
        }
    }

    /// A list, by what its items are.
    fn of_items(item: Arg) -> Arg {
        match item {
            Arg::Int => Arg::Ints,
            Arg::Float => Arg::Floats,
            Arg::Moment => Arg::Moments,
            Arg::Tensor => Arg::Tensors,
            Arg::Ints | Arg::Floats => Arg::Rows,
            Arg::Text => Arg::Texts,
            Arg::Bytes => Arg::Chunks,
            Arg::Truth => Arg::Truths,
            Arg::Record => Arg::Records,
            Arg::Map => Arg::Maps,
            // a list of parsed values (or of cells no description typed yet)
            // is one whose items' kind shows as the program runs
            Arg::Unknown | Arg::Json | Arg::AnyList => Arg::AnyList,
            _ => Arg::Items,
        }
    }

    /// What a value is. A list is what its first item is (an empty one
    /// could be any list); the interpreter's JSON value and frame's column
    /// are what they hold.
    pub fn of_value(v: &CrowValue) -> Arg {
        match &v.value {
            Value::Int(_) | Value::Big(_) => Arg::Int,
            Value::Float(_) => Arg::Float,
            Value::Decimal(_) => Arg::Decimal,
            Value::Fraction(_) => Arg::Fraction,
            Value::NumArray(_) => Arg::Tensor,
            Value::Array(items) => match items.first() {
                Some(first) => Arg::of_items(Arg::of_value(first)),
                None => Arg::AnyList,
            },
            Value::Str(_) => Arg::Text,
            Value::Bool(_) => Arg::Truth,
            Value::Map(_) => Arg::Map,
            Value::StructInstance(_) => Arg::Record,
            Value::Sparse(_) => Arg::Sparse,
            Value::Bytes(_) => Arg::Bytes,
            Value::Stream(_) => Arg::Handle(&STREAM),
            Value::Socket(_) => Arg::Handle(&SOCKET),
            Value::Mailer(_) => Arg::Handle(&MAILER),
            Value::DbConn(_) => Arg::Handle(&DB),
            Value::Response(_) => Arg::Handle(&RESPONSE),
            Value::Frame(_) => Arg::Frame,
            Value::Fn(_) | Value::GradFn(_) => Arg::Fn,
            Value::Json(_) => Arg::Json,
            _ => Arg::Unknown,
        }
    }
}

impl Kind {
    /// The kind as a refusal names what a parameter takes.
    pub fn words(self) -> &'static str {
        match self {
            Kind::Int | Kind::Whole | Kind::Big => "an int",
            Kind::Decimal => "a decimal",
            Kind::Fraction => "a fraction",
            Kind::Float => "a number",
            Kind::Moment => "a moment",
            Kind::Bool => "a bool",
            Kind::Str => "text",
            Kind::Key => "a key (text or an int)",
            Kind::Keys => "keys (text or ints)",
            Kind::Tensor => "numbers",
            Kind::Tensors => "a list of tensors",
            Kind::Bytes => "bytes",
            Kind::Stream => STREAM.word,
            Kind::Strs => "a list of text",
            Kind::Chunks => "a list of bytes",
            Kind::Bools => "a list of bools",
            Kind::Sparse => "a sparse matrix",
            Kind::Frame => "a frame",
            Kind::Cell => "a number, text or a bool",
            Kind::Cells => "a column's values",
            Kind::Words => "a list",
            Kind::Sizes => "sizes",
            Kind::Groups => "tensors",
            Kind::Lists => "lists of text",
            Kind::FloatMap | Kind::MapOf | Kind::MapRef | Kind::TextMap => "a map",
            Kind::Unit => "nothing",
            Kind::List | Kind::Seq => "a list",
            Kind::Item | Kind::ItemRef => "a value",
            Kind::Ints | Kind::Bigs => "a list of ints",
            Kind::Floats => "a list of numbers",
            Kind::Texts => "a list of text",
            Kind::Json => "a JSON value",
            Kind::Column => "a frame's column",
            Kind::Markup => "markup",
            Kind::TextRows => "rows of text",
            Kind::Shell => "a command's answer",
            Kind::Reply => "a request's answer",
            Kind::Answer => "a connection's answer",
            Kind::Response => RESPONSE.word,
            Kind::FnOf(_) => "a function",
            Kind::Walked => "a list",
            Kind::Request => "a request",
            Kind::Db => DB.word,
            Kind::Tx => TX.word,
            Kind::Found => "a match",
            Kind::Socket => SOCKET.word,
            Kind::Mailer => MAILER.word,
            Kind::Shown | Kind::Shows | Kind::ShownDyn => "a value",
            Kind::Encoded => "a value JSON can hold",
            Kind::JsonList => "a list of JSON values",
            Kind::Operands | Kind::Operand => "a tensor or a number",
            Kind::Scalars | Kind::Scalar => "a number, text or a bool",
        }
    }

    /// Whether a parameter of this kind takes an argument of that kind.
    fn takes(self, a: Arg) -> bool {
        // a list whose items are not known could be any list
        if a == Arg::AnyList {
            return [Arg::Ints, Arg::Floats, Arg::Texts, Arg::Truths, Arg::Tensors, Arg::Rows, Arg::Records, Arg::Maps, Arg::Items].iter().any(|l| self.takes(*l));
        }
        a == Arg::Unknown
            || match self.each() {
                Kind::Int | Kind::Float => matches!(a, Arg::Int | Arg::Float),
                Kind::Moment => a == Arg::Moment,
                Kind::Whole | Kind::Big => a == Arg::Int,
                Kind::Decimal => a == Arg::Decimal,
                Kind::Fraction => a == Arg::Fraction,
                // numbers held as a list or a column are converted where
                // they meet the parameter (the lowering's row arguments)
                // (a mask where numbers are asked is numbers: true 1, false 0)
                Kind::Tensor => matches!(a, Arg::Tensor | Arg::Ints | Arg::Floats | Arg::Column | Arg::FallibleNumbers | Arg::Tensors | Arg::Rows | Arg::Truths),
                Kind::Tensors => a == Arg::Tensors,
                Kind::Str => a == Arg::Text,
                Kind::Key => matches!(a, Arg::Text | Arg::Int),
                Kind::Keys => matches!(a, Arg::Texts | Arg::Ints | Arg::Tensor),
                Kind::Strs | Kind::Texts => a == Arg::Texts,
                Kind::Chunks => a == Arg::Chunks,
                Kind::Bool => a == Arg::Truth,
                Kind::Bools => a == Arg::Truths,
                Kind::Sparse => a == Arg::Sparse,
                Kind::Bytes => a == Arg::Bytes,
                Kind::Stream => matches!(a, Arg::Handle(h) if h.name == STREAM.name),
                Kind::Socket => matches!(a, Arg::Handle(h) if h.name == SOCKET.name),
                Kind::Mailer => matches!(a, Arg::Handle(h) if h.name == MAILER.name),
                Kind::Response => matches!(a, Arg::Handle(h) if h.name == RESPONSE.name),
                Kind::FnOf(_) => a == Arg::Fn,
                Kind::Walked => matches!(a, Arg::Tensor | Arg::Column | Arg::Ints | Arg::Floats | Arg::Texts | Arg::Truths | Arg::Tensors | Arg::Rows | Arg::Records | Arg::Maps | Arg::Items | Arg::Chunks | Arg::Fallible | Arg::FallibleNumbers | Arg::Moments | Arg::AnyList),
                // (a transaction's connection is a connection)
                Kind::Db | Kind::Tx => matches!(a, Arg::Handle(h) if h.name == DB.name || h.name == TX.name),
                Kind::Request | Kind::Found => false,
                Kind::Frame => a == Arg::Frame,
                Kind::Cell => matches!(a, Arg::Int | Arg::Float | Arg::Text | Arg::Truth | Arg::Json | Arg::Moment),
                Kind::Cells => matches!(a, Arg::Int | Arg::Float | Arg::Text | Arg::Truth | Arg::Tensor | Arg::Ints | Arg::Floats | Arg::Texts | Arg::Truths | Arg::Column | Arg::Moment | Arg::Moments),
                // (a list whose items can fail: its items, a failed one
                // stopping the program, as a sum's numbers do)
                Kind::Words => matches!(a, Arg::Tensor | Arg::Ints | Arg::Floats | Arg::Texts | Arg::Truths | Arg::Column | Arg::Moments | Arg::Fallible | Arg::FallibleNumbers),
                Kind::List | Kind::Seq => matches!(a, Arg::Ints | Arg::Floats | Arg::Texts | Arg::Truths | Arg::Tensors | Arg::Rows | Arg::Records | Arg::Maps | Arg::Items | Arg::Chunks | Arg::Fallible | Arg::FallibleNumbers | Arg::Moments),
                Kind::Item | Kind::ItemRef => true,
                Kind::MapOf | Kind::MapRef | Kind::TextMap => a == Arg::Map,
                Kind::Ints | Kind::Bigs => a == Arg::Ints,
                Kind::Floats => a == Arg::Floats,
                Kind::Json => a == Arg::Json,
                Kind::Column => a == Arg::Column,
                // a function shows as nothing, and JSON holds no function, frame or matrix
                Kind::Shown | Kind::ShownDyn => !matches!(a, Arg::Fn | Arg::Fns),
                // (nor a connection, a response, a socket, a mailer, a stream,
                // or bytes — which `encode` had written as JSON's null, and
                // the compiled program had not built: base64[b] is text)
                Kind::Encoded => !matches!(a, Arg::Fn | Arg::Fns | Arg::Frame | Arg::Sparse | Arg::Column | Arg::Handle(_) | Arg::Bytes | Arg::Chunks),
                Kind::Operand => matches!(a, Arg::Tensor | Arg::Int | Arg::Float),
                Kind::Scalar => matches!(a, Arg::Int | Arg::Float | Arg::Text | Arg::Truth),
                _ => true,
            }
    }
}

/// Whether a row takes these arguments: their count, and each one's kind.
/// A frame's column is numbers where a module takes numbers; the core
/// takes one where it says so (`len`), the interpreter's cells being a list.
fn takes(row: &Row, args: &[Arg]) -> bool {
    match spans(row.params, args.len(), row.name) {
        Ok(spans) => row.params.iter().zip(spans).all(|(k, (a, b))| args[a..b].iter().all(|x| accepts(row, *k, *x))),
        Err(_) => false,
    }
}

/// Whether a parameter of the row takes an argument of the class: what its
/// kind takes, except that a core row's tensor parameter does not take a
/// frame's column (the column form is the frame module's).
fn accepts(row: &Row, k: Kind, x: Arg) -> bool {
    k.takes(x) && !(row.module.is_empty() && x == Arg::Column && k.each() == Kind::Tensor)
}

/// A module function's row for a call with these arguments: of the rows of
/// its name, the first that takes their count and kinds (`exp` of a number
/// or of a tensor, `norm[t]` or `norm[t, p]`); then the first that takes
/// their count; then the first, whose own check refuses the call.
/// A module's row of a name that takes these arguments, or none: what
/// tells two modules' rows of one name apart (sys's `send` of a stream,
/// a socket's of a socket).
/// The module a bare call reaches, decided once for every pipeline: type
/// inference typing the call and the lowering naming its row read this
/// one answer. (Each had picked by a rule of its own — inference by the
/// first argument's type, the lowering by every argument's kind — and
/// `where` of a mask was math's to the one and the frame's to the other,
/// so the variable holding the column was declared a tensor and the
/// compiled program did not build.) The core's row the kinds pick comes
/// first; else, among the modules in scope that answer the name, the one
/// whose row takes the kinds, where exactly one does; else, where the
/// kinds do not settle it, the first argument's class — a frame or its
/// column the frame's, a tensor or a list math's; else none.
pub fn module_of_bare(name: &str, providers: &[&str], kinds: &[Arg]) -> Option<String> {
    if providers.is_empty() {
        return None;
    }
    let unknown = kinds.iter().any(|k| matches!(k, Arg::Unknown | Arg::AnyList));
    // (a core form that declares its kinds — `find` of text, `any` of a
    // list and a function — is the core's as a row is: `find['a', 'b']`
    // under `use sys` had reached the layer's row of bytes, and the
    // lowered program disagreed with itself — crow check --self's pairs)
    if !unknown && (core_row(name, kinds).is_some() || form_answer(name, kinds).is_some_and(|r| r.module.is_empty())) {
        return None;
    }
    if providers.len() == 1 {
        // (unless its rows take none of these and the core's do: `slice[[],
        // 1, 3]` under `use sys` had reached the layer's row of bytes in
        // inference and the core's of lists in the lowering, and `type` of
        // it was `unknown` compiled — crow check --self's pairs)
        if unknown && row_taking(providers[0], name, kinds).is_none() && (core_row(name, kinds).is_some() || form_answer(name, kinds).is_some_and(|r| r.module.is_empty())) {
            return None;
        }
        return Some(providers[0].to_string());
    }
    let taking: Vec<&&str> = providers.iter().filter(|m| row_taking(m, name, kinds).is_some()).collect();
    if taking.len() == 1 {
        return Some(taking[0].to_string());
    }
    // (two modules' rows take: a frame's column anywhere makes the call the
    // frame's — math's rows take a column only as its numbers, and a mask
    // beside one is the frame's `where`, not math's)
    if taking.len() > 1 && kinds.iter().any(|k| matches!(k, Arg::Column)) && taking.iter().any(|m| ***m == *"frame") {
        return Some("frame".to_string());
    }
    let by_first = match kinds.first() {
        Some(Arg::Frame | Arg::Column) => "frame",
        Some(Arg::Tensor | Arg::Ints | Arg::Floats | Arg::Tensors | Arg::Rows | Arg::Texts | Arg::Truths | Arg::Records | Arg::Maps | Arg::Items | Arg::AnyList | Arg::FallibleNumbers | Arg::Fallible) => "math",
        _ => return None,
    };
    providers.contains(&by_first).then(|| by_first.to_string())
}

pub fn row_taking(module: &str, name: &str, args: &[Arg]) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.module == module && r.name == name && takes(r, args))
}

pub fn module(module: &str, name: &str, args: &[Arg]) -> Option<&'static Row> {
    let named = || ROWS.iter().filter(move |r| r.module == module && r.name == name);
    named()
        .find(|r| takes(r, args))
        .or_else(|| named().find(|r| spans(r.params, args.len(), r.name).is_ok()))
        .or_else(|| named().next())
}

/// The refusal of a call whose count of arguments no row of its name
/// takes, naming every count the name takes (`rsi[] takes 2 or 4
/// arguments, got 3`); none when a row takes it.
pub fn miscounted(module: &str, name: &str, argc: usize) -> Option<String> {
    let rows: Vec<&Row> = ROWS.iter().filter(|r| r.module == module && r.name == name).collect();
    if rows.is_empty() || rows.iter().any(|r| spans(r.params, argc, name).is_ok()) {
        return None;
    }
    let mut counts: Vec<String> = Vec::new();
    for r in rows {
        let fixed = r.params.iter().filter(|k| !k.is_rest()).count();
        let count = if r.params.iter().any(|k| k.is_rest()) { format!("at least {}", fixed) } else { fixed.to_string() };
        if !counts.contains(&count) {
            counts.push(count);
        }
    }
    let counts = match counts.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {}", rest.join(", "), last),
        _ => counts.join(""),
    };
    Some(miscount(name, &counts, argc))
}


/// A key of the other kind than the map's: the refusal, naming both.
pub fn wrong_key(key: crate::ir::Key, given: &Ty) -> String {
    let given = Arg::of_type(given).words();
    match key {
        crate::ir::Key::Int => format!("the map's keys are ints, not {}: `m[2]` reads one, `'2'` is text", given),
        _ => format!("the map's keys are text, not {}: `m['k']` reads one — a map keyed by ints is made by `map[[1, 2], …]` or `set[map[], 2, v]`", given),
    }
}

/// A row's call as the program writes it, for a refusal: the operator or
/// the index behind a hidden row, else `name[]`.
pub fn form(name: &str) -> String {
    // (a row that declares how it is written: the language's forms)
    if let Some(r) = ROWS.iter().find(|r| r.name == name && !r.form.is_empty()) {
        return r.form.to_string();
    }
    match name {
        "contains" => "`x in xs`".to_string(),
        "__map_of" => "map[keys, values]".to_string(),
        "__pick_items" | "__pick_rows" | "__pick_cells" => "`xs[mask]`".to_string(),
        "__pick_frame" | "__pick_where" => "`f[mask]`".to_string(),
        "__set_at" => "`m[i][j] = x`".to_string(),
        "__set_part" => "`m[i] = row`".to_string(),
        "__sub_len" => "len[sub[…]]".to_string(),
        "__none" => "a record or 0 (none)".to_string(),
        "__sizes" => "a call's sizes (`randn[2, 3]`)".to_string(),
        "__whole" => "a whole number past 64 bits".to_string(),
        "__truncated" => "a number where an int is taken".to_string(),
        "__text_map" => "a map of text".to_string(),
        "__groups" => "a call's tensors (`anova[a, b]`)".to_string(),
        // a hidden row reached by an option: as the program writes it
        _ => format!("{}[]", ROWS.iter().find(|r| r.name == name && !r.written.is_empty()).map_or(name, |r| r.written)),
    }
}

/// `sma[] takes 2 arguments, got 3`, `median[] takes 1 argument, got 2`.
fn miscount(name: &str, counts: &str, argc: usize) -> String {
    // one list of pieces: joined when written down, or held as bytes
    if name == "concat" && argc == 1 {
        return "concat[] takes two pieces, or one list of them written down — `concat[[a, b, c]]` — or a list of bytes held in a variable".to_string();
    }
    let noun = if counts == "1" || counts == "at least 1" { "argument" } else { "arguments" };
    format!("{} takes {} {}, got {}", form(name), counts, noun, argc)
}

/// The core names with syntax or state of their own, which no row says: a
/// value's own status (`error` makes one, `captured` reads its stamp) and
/// a static type's word compiled beside a value's interpreted (`type`); a
/// statement reporting its operands (`assert`, whose words are the
/// runtime's, `display::assert_failed`); what the session holds (`later`,
/// `running`, `notice`); a function argument the walker calls (`any`,
/// `all`, `find`, `grad`); a resource (`cpu`, `gpu`, `nodes`); a test
/// block; a chart's tree.
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test

/// The scalar names the core and math both answer — a number's row the
/// core's, a tensor's math's. One list, read wherever a call is resolved
/// by name (it had been written in the interpreter and twice in the
/// emitter); the test below holds it to the table.
pub const CORE_AND_MATH: &[&str] = &["abs", "sqrt", "floor", "ceil", "round", "min", "max"];

/// The core builtins still written twice, each to become a row (docs/
/// bug-classes.md §4). The list only shrinks: a builtin added outside the
/// table fails `every_core_builtin_is_a_row_or_says_why_not`.
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test
pub const TO_MOVE: &[&str] = &[];

/// The module functions with a form of their own: the database's effects,
/// which the session layer performs (a connection, a transaction, a write
/// held to the handler's commit).
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test

/// The module functions still written twice, each to become a row (docs/
/// bug-classes.md §4); like TO_MOVE, it only shrinks.
#[cfg_attr(not(test), allow(dead_code))] // read by the drift test
pub const MODULE_TO_MOVE: &[(&str, &str)] = &[];

#[cfg(test)]
mod tests {
    /// The interpreter's bridges group a call's arguments in a fixed array
    /// (`groups_in_place`): no row may have more parameters than it holds.
    #[test]
    fn every_row_fits_the_bridges_fixed_grouping() {
        for row in ROWS {
            assert!(row.params.len() <= MOST_PARAMS, "{} has {} parameters", row.id, row.params.len());
        }
    }

    use super::*;

    /// A row declares the columns of its frame or a provider of them, and
    /// the form that opens a connection says so: the compiler asks the
    /// table and names nothing.
    #[test]
    fn frames_and_connections_are_declared_on_rows() {
        let query = frame_declared(Some("db"), "query").expect("db.query declares a provider");
        let p = query.provider.expect("a provider");
        assert_eq!((p.connection, p.statement), (0, 1));
        assert!(frame_declared(None, "query").is_some());
        assert_eq!(frame_declared(Some("chart"), "macd").map(|r| r.columns.len()), Some(3));
        assert_eq!(frame_declared(None, "chapters").map(|r| r.columns.iter().map(|(n, _)| *n).collect::<Vec<_>>()), Some(vec!["title", "path", "depth", "part"]));
        assert!(frame_declared(Some("frame"), "sort").is_none());
        assert!(opens_connection(None, "connect") && opens_connection(Some("db"), "connect"));
        assert!(!opens_connection(Some("web"), "connect") && !opens_connection(None, "query"));
    }

    fn int(n: i64) -> CrowValue {
        CrowValue::ok(Value::Int(n))
    }
    fn text(s: &str) -> CrowValue {
        CrowValue::ok(Value::Str(s.to_string()))
    }

    #[test]
    fn a_rows_path_is_the_function_it_calls() {
        let r = by_id("text.repeat").expect("the row");
        assert_eq!(r.rust_path(), "crow_runtime::text::repeat");
        assert_eq!(r.params, &[Kind::Str, Kind::Int]);
        assert_eq!(r.ty(), Ty::Str);
        assert_eq!(by_id("moments.moment_text").expect("the row").ty(), Ty::Result(Box::new(Ty::Moment)));
        assert_eq!(by_id("moments.now").expect("the row").ty(), Ty::Moment);
    }

    #[test]
    fn a_failure_is_what_the_row_says() {
        let repeat = by_id("text.repeat").expect("the row");
        assert_eq!((repeat.call)(&[text("ab"), int(2)], "t:1").map(|v| format!("{}", v.value)), Ok("abab".to_string()));
        assert_eq!((repeat.call)(&[text("ab"), int(-1)], "t:1").map(|_| ()), Err("repeat[] count cannot be negative (got -1)".to_string()));
        let parse = by_id("moments.moment_text_in").expect("the row");
        let bad = (parse.call)(&[text("not a time"), text("UTC")], "t:3").expect("an error value, not a stop");
        assert!(bad.is_error());
        assert_eq!(bad.kind(), "time");
        assert_eq!(bad.trace(), &["t:3".to_string()]);
    }

    /// The one row choice for a bare name, as inference and the lowering
    /// both read it: the core's row the kinds fit, else the one module whose
    /// row takes them, else the first argument's class, else none.
    #[test]
    fn a_bare_name_reaches_one_module_by_its_kinds() {
        let both = ["frame", "math"];
        // where of a mask, a column and an int is the frame's; of three tensors math's
        assert_eq!(module_of_bare("where", &both, &[Arg::Truths, Arg::Column, Arg::Int]), Some("frame".to_string()));
        assert_eq!(module_of_bare("where", &both, &[Arg::Tensor, Arg::Tensor, Arg::Tensor]), Some("math".to_string()));
        // sort of a list is the core's (none), of a frame the frame's
        assert_eq!(module_of_bare("sort", &["frame"], &[Arg::Ints]), None);
        assert_eq!(module_of_bare("sort", &["frame"], &[Arg::Frame, Arg::Text]), Some("frame".to_string()));
        // a kind not known yet is taken by any row, so the count decides:
        // unique of a frame and a name fits the frame's rows, of one value
        // math's; of a list the kinds pick math's (the core's unique is a
        // tensor's)
        assert_eq!(module_of_bare("unique", &both, &[Arg::Frame, Arg::Unknown]), Some("frame".to_string()));
        assert_eq!(module_of_bare("unique", &both, &[Arg::Unknown]), Some("math".to_string()));
        assert_eq!(module_of_bare("stack", &both, &[Arg::Unknown, Arg::Unknown]), None, "two rows take two unknowns and the class of nothing is nothing");
        // one provider is reached whatever the kinds say
        assert_eq!(module_of_bare("rank", &["math"], &[Arg::Unknown]), Some("math".to_string()));
        assert_eq!(module_of_bare("nope", &[], &[Arg::Int]), None);
    }

    /// A core builtin is a row, has a form of its own, or is still to move,
    /// and exactly one of these: a builtin added outside the table, or a
    /// moved one left on the list, fails here.
    #[test]
    fn every_core_builtin_is_a_row_or_says_why_not() {
        for name in crate::lower_ir::CORE_RESERVED.iter() {
            let form = FORMS.iter().any(|f| f.module.is_empty() && f.name == *name);
            let places = [core(name).is_some(), form, TO_MOVE.contains(name), *name == "test"];
            assert_eq!(places.iter().filter(|p| **p).count(), 1, "'{}' is in {} of: the table, FORMS, TO_MOVE", name, places.iter().filter(|p| **p).count());
        }
        for r in ROWS.iter().filter(|r| r.module.is_empty()) {
            // (an internal name is a core builtin's form: `__fmt_time`)
            assert!(crate::lower_ir::CORE_RESERVED.contains(&r.name) || r.name.starts_with("__"), "row '{}' names no core builtin", r.name);
        }
    }

    /// A module function is a row, has a form of its own, is a function its
    /// source exports (a module written in Crow), or is still to move, and
    /// exactly one of these.
    #[test]
    fn every_module_function_is_a_row_or_says_why_not() {
        // (every module the manifest gives a registry object)
        let modules: Vec<(&str, Vec<String>)> = crate::modules::MODULES
            .iter()
            .filter_map(|m| m.module.map(|make| (m.path, make().functions().iter().map(|f| f.to_string()).collect())))
            .collect();
        for (m, names) in modules {
            for name in names.iter().map(String::as_str) {
                let row = ROWS.iter().any(|r| r.module == m && r.name == name);
                let form = FORMS.iter().any(|f| f.module == m && f.name == name);
                let export = crate::modules::exports(m).contains(&name);
                let places = [row, form, export, MODULE_TO_MOVE.contains(&(m, name))];
                assert_eq!(places.iter().filter(|p| **p).count(), 1, "{}.{} is in {} of: the table, FORMS, its source's exports, MODULE_TO_MOVE", m, name, places.iter().filter(|p| **p).count());
            }
        }
    }

    /// A rest parameter takes each of the rest as its kind, and a count no
    /// row of a name takes names every count the name takes.
    #[test]
    fn a_rest_takes_the_rest_and_a_miscount_names_the_counts() {
        let anova = module("math.stats", "anova", &[Arg::Tensor; 3]).expect("the row");
        assert_eq!(kinds_of(anova.params, 3), Some(vec![Kind::Tensor; 3]));
        assert_eq!(kinds_of(by_id("math.full").expect("the row").params, 3), Some(vec![Kind::Int, Kind::Int, Kind::Float]));
        assert_eq!(kinds_of(by_id("chart.sma").expect("the row").params, 3), None);
        assert_eq!(miscounted("chart", "rsi", 3), Some("rsi[] takes 2 or 4 arguments, got 3".to_string()));
        assert_eq!(miscounted("chart", "rsi", 4), None);
        assert_eq!(miscounted("math", "zeros", 0), None);
        assert_eq!(miscounted("web.ws", "recv", 3), Some("recv[] takes 1 or 2 arguments, got 3".to_string()));
        let zeros = by_id("math.zeros").expect("the row");
        let printed = print(zeros, &["a".to_string(), "b".to_string()], "7", None).expect("prints");
        assert!(printed.starts_with("crow_runtime::math::zeros(&[") && printed.contains("(a)") && printed.contains("(b)"), "{printed}");
        assert!(printed.ends_with("]).unwrap_or_else(|e| crow_stop(7, e))"), "{printed}");
        // a row written in Crow is never printed as a runtime call
        let refused = print(anova, &["a".to_string(), "b".to_string()], "7", None).expect_err("refused");
        assert!(refused.starts_with("a bug in Crow: `anova` is written in Crow"), "{refused}");
    }

    /// A Rust row's text parameter takes a literal as the literal and an
    /// indexed read as a reference: neither allocates or copies per call
    #[test]
    fn a_text_parameter_takes_a_literal_and_an_indexed_read_without_a_copy() {
        assert_eq!(literal_text("\"\\t\".to_string()"), Some("\"\\t\"".to_string()));
        assert_eq!(literal_text("\"a\".to_string() + \"b\".to_string()"), None);
        assert_eq!(literal_text("name.to_string()"), None);
        let indexed = "{ let _i = 2_i64; let _v = &rows; _v[crow_runtime::stops::at(_i, _v.len(), 5)].clone() }";
        let borrowed = "{ let _i = 2_i64; let _v = &rows; &_v[crow_runtime::stops::at(_i, _v.len(), 5)] }";
        assert_eq!(borrowed_index(indexed).as_deref(), Some(borrowed));
        // an item that is Copy is not cloned, and is not a borrow either
        assert_eq!(borrowed_index("{ let _i = 0_i64; let _v = &xs; _v[crow_runtime::stops::at(_i, _v.len(), 5)] }"), None);
        // a list an expression makes dies with the block: the item is cloned
        assert_eq!(borrowed_index("{ let _i = 0_i64; let _v = &crow_runtime::text::split(&*(s), \" \"); _v[crow_runtime::stops::at(_i, _v.len(), 5)].clone() }"), None);
        assert_eq!(borrowed_index("{ let _i = 0_i64; let _v = &(_t4).value.names; _v[crow_runtime::stops::at(_i, _v.len(), 5)].clone() }"), None);
        assert!(borrowed_index("{ let _i = 0_i64; let _v = &r.names; _v[crow_runtime::stops::at(_i, _v.len(), 5)].clone() }").is_some());
        // a parameter the caller lends and the payload of a held value that can
        // fail are places a name holds; a call's answer is not
        assert!(borrowed_index("{ let _i = i; let _v = &(*rows); _v[crow_runtime::stops::at(_i, _v.len(), 6)].clone() }").is_some());
        assert!(borrowed_index("{ let _i = i; let _v = &(rows).value; _v[crow_runtime::stops::at(_i, _v.len(), 6)].clone() }").is_some());
        assert_eq!(borrowed_index("{ let _i = i; let _v = &(rows.clone()); _v[crow_runtime::stops::at(_i, _v.len(), 6)].clone() }"), None);
        assert_eq!(borrowed_index("{ let _i = i; let _v = &f(x); _v[crow_runtime::stops::at(_i, _v.len(), 6)].clone() }"), None);
        assert_eq!(Kind::Str.print("\"\\t\".to_string()", "3"), "\"\\t\"");
        assert_eq!(Kind::Str.print(indexed, "3"), format!("&*({})", borrowed));
        assert_eq!(Kind::Str.print("name", "3"), "&*(name)");
    }

    #[test]
    fn every_id_is_unique() {
        for (i, r) in ROWS.iter().enumerate() {
            assert!(ROWS.iter().skip(i + 1).all(|o| o.id != r.id), "{} twice", r.id);
        }
    }

    /// A JSON number's length stops the compiled program with the words the
    /// interpreter's refusal gives the number it parsed to.
    #[test]
    fn a_json_value_without_a_length_is_refused_in_the_tables_words() {
        assert_eq!(crow_runtime::core::len_json(&serde_json::json!(5)), Err(refusal(&[""], "len", &[Arg::Int]).expect("a refusal")));
        assert_eq!(crow_runtime::core::len_json(&serde_json::json!(2.5)), Err(refusal(&[""], "len", &[Arg::Float]).expect("a refusal")));
        assert_eq!(crow_runtime::core::len_json(&serde_json::json!(true)), Err(refusal(&[""], "len", &[Arg::Truth]).expect("a refusal")));
        assert_eq!(crow_runtime::core::len_json(&serde_json::json!([1, 2])), Ok(2));
    }

    /// `log` is its row over any values that show, each handed as the one
    /// display takes it; env's default is text, as the variable is; the math
    /// of a plain number is its row, an int's int and a float's float.
    #[test]
    fn a_builtin_once_per_mode_is_a_row() {
        let log = core("log").expect("log's row");
        let printed = print(log, &["a".to_string(), "b".to_string()], "1", None).expect("printed");
        assert!(printed.contains("&(a) as &dyn crow_runtime::display::CrowDisplay"), "{}", printed);
        assert_eq!(crow_runtime::core::env_or("CROW_TABLE_TEST_NO_SUCH_VARIABLE", &crow_runtime::cell::Cell::Int(3000)), Ok("3000".to_string()));
        assert_eq!(core_row("abs", &[Arg::Int]).map(|r| r.id), Some("core.abs_int"));
        assert_eq!(core_row("abs", &[Arg::Float]).map(|r| r.id), Some("core.abs"));
        assert_eq!(crow_runtime::core::round(2.5), 3);
        assert_eq!(crow_runtime::core::round(-2.5), -3);
        assert!(crow_runtime::core::sleep_ms(-1).is_err());
        assert!(!FORMS.iter().any(|f| f.name == "log") && FORMS.iter().any(|f| f.name == "assert"));
    }

    /// A list of numbers that can fail is numbers where numbers are taken
    /// (a failed one stops as it is read); a list of parsed values is one
    /// whose items' kind shows as the program runs.
    #[test]
    fn a_list_is_what_its_items_are() {
        let fallible = Ty::List(Box::new(Ty::Result(Box::new(Ty::Float))));
        assert_eq!(Arg::of_type(&fallible), Arg::FallibleNumbers);
        assert!(core_row("sum", &[Arg::FallibleNumbers]).is_none(), "sum is math's");
        assert!(module("math", "sum", &[Arg::FallibleNumbers]).is_some_and(|r| takes(r, &[Arg::FallibleNumbers])));
        // a list of parsed values is a list, whatever its items turn out to
        // be: a list's parameter takes it, a text's does not
        assert_eq!(Arg::of_type(&Ty::List(Box::new(Ty::Json))), Arg::AnyList);
        assert_eq!(Arg::of_type(&Ty::List(Box::new(Ty::Unknown))), Arg::AnyList);
        assert_eq!(core_row("len", &[Arg::AnyList]).map(|r| r.id), Some("core.len"));
        assert!(core_row("upper", &[Arg::AnyList]).is_none());
        assert!(refusal(&[""], "upper", &[Arg::AnyList]).is_some());
        assert!(refusal(&[""], "upper", &[Arg::Json]).is_some_and(|e| e.ends_with(READ_FIRST)));
    }

    /// A collection is taken, not copied, by the call that keeps it: the
    /// interpreter's `take` moves the list into the runtime's push.
    #[test]
    fn a_collection_is_moved_into_its_row() {
        let push = core_row("push", &[Arg::Ints, Arg::Int]).expect("the generic push");
        assert_eq!(push.id, "core.push");
        let grown = (push.take)(vec![CrowValue::ok(Value::Array(std::sync::Arc::new(vec![int(1)]))), int(2)], "t:1").expect("a list");
        assert_eq!(format!("{}", grown.value), "[1, 2]");
        assert_eq!(core_row("sort", &[Arg::Floats]).map(|r| r.id), Some("core.sort_floats"));
        assert!(core_row("sort", &[Arg::Records]).is_none());
        // a list of lists of numbers is rows, which a list of tensors'
        // parameter does not take and a tensor's does (as a matrix)
        let rows = Arg::of_type(&Ty::List(Box::new(Ty::List(Box::new(Ty::Float)))));
        assert_eq!(rows, Arg::Rows);
        assert_eq!(Arg::of_type(&Ty::List(Box::new(Ty::tensor(Ty::Float)))), Arg::Tensors);
        assert!(!Kind::Tensors.takes(Arg::Rows) && Kind::Tensor.takes(Arg::Rows) && Kind::List.takes(Arg::Rows));
        assert!(takes_mask_at("where", 0, 3) && !takes_mask_at("sort", 2, 3));
        assert_eq!(core_row("len", &[Arg::Column]).map(|r| r.id), Some("core.len_column"));
        assert!(core_row("contains", &[Arg::Column, Arg::Int]).is_none());
    }

    /// A call whose list's items are not known stays the core's where the
    /// one module that names it takes no list (`slice[[], 1, 3]` under `use
    /// sys` had been typed as the layer's bytes), and JSON holds no bytes
    /// (`encode` of them had answered null) — crow check --self.
    #[test]
    fn a_list_of_nothing_known_stays_the_cores_and_bytes_are_no_json() {
        assert_eq!(module_of_bare("slice", &["sys"], &[Arg::AnyList, Arg::Int, Arg::Int]), None);
        assert_eq!(module_of_bare("slice", &["sys"], &[Arg::Bytes, Arg::Int, Arg::Int]), Some("sys".to_string()));
        assert!(core_row("encode", &[Arg::Bytes]).is_none());
        assert!(core_row("encode", &[Arg::Chunks]).is_none());
        assert!(core_row("encode", &[Arg::Map]).is_some());
    }

    /// The index answers every name as the scan of the table did: the same
    /// rows, in the same order, a hidden form found by the name it is
    /// written as (`query`'s writing twin) and a name no row has answering
    /// none.
    #[test]
    fn a_name_is_answered_from_the_index_as_the_scan_answered_it() {
        let scan = |name: &str| -> Vec<&'static str> { ROWS.iter().chain(FORMS.iter()).filter(|r| r.name == name || r.written == name).map(|r| r.id).collect() };
        let mut names: Vec<&str> = ROWS.iter().chain(FORMS.iter()).flat_map(|r| [r.name, r.written]).filter(|n| !n.is_empty()).collect();
        names.extend(["no_such_builtin", "x"]);
        for n in names {
            assert_eq!(rows_named(n).map(|r| r.id).collect::<Vec<_>>(), scan(n), "rows named {}", n);
        }
        // (a row's stamp, read by its place in the table, is its name's)
        for r in ROWS.iter().chain(FORMS.iter()) {
            assert_eq!(row_stamps(r), stamps(r.name), "the stamp of {}", r.id);
        }
    }

    /// The questions the interpreter asks of a name after every builtin
    /// call are answered without a scan of the table: measured in the same
    /// run, the index answers a thousand names at least ten times faster
    /// than a scan does (e967633 had put a scan behind every call, and the
    /// interpreter ran n-body four times slower).
    #[test]
    fn a_name_is_answered_without_a_scan_of_the_table() {
        let names: Vec<&str> = ROWS.iter().map(|r| r.name).take(200).collect();
        let rounds = 50;
        let began = std::time::Instant::now();
        let mut indexed = 0usize;
        for _ in 0..rounds {
            for n in &names {
                indexed += rows_named(n).count();
            }
        }
        let by_index = began.elapsed();
        let began = std::time::Instant::now();
        let mut scanned = 0usize;
        for _ in 0..rounds {
            for n in &names {
                scanned += ROWS.iter().chain(FORMS.iter()).filter(|r| r.name == *n || r.written == *n).count();
            }
        }
        let by_scan = began.elapsed();
        assert_eq!(indexed, scanned);
        assert!(by_index * 10 < by_scan, "the index took {:?} where a scan took {:?}", by_index, by_scan);
        assert!(stamps("read") && !stamps("sqrt") && has("push", Prop::InPlace) && !has("sqrt", Prop::InPlace));
    }

    /// Every row is reachable: no earlier row of its name and module takes
    /// every argument it takes, or the first-fit lookup would never reach
    /// it (`str[column]` had been listed after the form that shows any
    /// value, and every column was shown).
    #[test]
    fn no_row_is_shadowed_by_an_earlier_row() {
        fn positional(r: &Row) -> bool {
            matches!(spans(r.params, r.params.len(), r.name), Ok(sp) if sp.iter().all(|(a, b)| b - a == 1))
        }
        let classes = crate::totality::every_arg();
        let mut shadowed = Vec::new();
        for (j, later) in ROWS.iter().enumerate() {
            if !positional(later) {
                continue;
            }
            for earlier in &ROWS[..j] {
                if earlier.name != later.name || earlier.module != later.module || earlier.params.len() != later.params.len() || !positional(earlier) {
                    continue;
                }
                let covered = (0..later.params.len())
                    .all(|i| classes.iter().all(|&x| !accepts(later, later.params[i], x) || accepts(earlier, earlier.params[i], x)));
                if covered {
                    shadowed.push(format!("{} is never reached: {} before it takes everything it takes", later.id, earlier.id));
                }
            }
        }
        assert!(shadowed.is_empty(), "{}", shadowed.join("\n"));
    }

    /// The one list of the scalar names core and math both answer is what
    /// the table says it is.
    #[test]
    fn the_core_and_math_list_is_the_tables() {
        use std::collections::BTreeSet;
        let core: BTreeSet<&str> = ROWS.iter().filter(|r| r.module.is_empty()).map(|r| r.name).collect();
        let math: BTreeSet<&str> = ROWS.iter().filter(|r| r.module == "math").map(|r| r.name).collect();
        let both: BTreeSet<&str> = core.intersection(&math).copied().collect();
        let listed: BTreeSet<&str> = CORE_AND_MATH.iter().copied().collect();
        assert_eq!(both, listed, "the names core and math both answer");
    }
}


/// The map rows' bridges: a map is keyed by text or by ints (`Value::Map`,
/// `Value::IntMap`), an open map — empty — takes its first key's kind.
/// A tensor row's own bridge, given its arguments to keep.
/// The runtime function a row's call is printed as when its first argument
/// is a text nothing else holds (`s = replace[s, …]`, or another call's
/// answer): the text is changed where it lies, as the interpreter's `take`
/// bridges change it (`texts`) — and a list cut, a tensor pushed onto, the
/// same way. None for a row without one.
pub fn taken_path(id: &str) -> Option<&'static str> {
    ROWS.iter().find(|r| r.id == id).and_then(|r| r.taken)
}

/// The row a frame's column is written through — `df.name = values`, and
/// each column of `frame.new[name = values]` — the one row carrying
/// `Prop::ColumnWrite`: the lowering reaches it by the property, never by
/// its name.
pub fn column_write() -> &'static Row {
    ROWS.iter().find(|r| r.props.contains(&Prop::ColumnWrite)).expect("the column write's row is declared")
}

/// The row a frame's column is read through by a name the program
/// computes (`df.`name``): the one row carrying `Prop::ColumnRead`.
pub fn column_read() -> &'static Row {
    ROWS.iter().find(|r| r.props.contains(&Prop::ColumnRead)).expect("the column read's row is declared")
}

/// The row that keeps every digit in place of one that answers 64 bits
/// (int_width): `int` of text or a float, `floor`, `ceil` and `round` answer
/// a whole number of any size; `abs`, `min` and `max` keep one handed to
/// them. None for a row with no such form.
pub fn big_form(id: &str) -> Option<&'static Row> {
    let big = ROWS.iter().find(|r| r.id == id)?.big?;
    ROWS.iter().find(|r| r.id == big)
}

/// The name a program wrote for a row the width pass put in another's
/// place (`__floor_big` is `floor`'s), by `big_form`'s own pairs; a row's
/// own name otherwise. A dual number a function `grad` differentiates is
/// answered by that name: the width pass reads its parameter as a float
/// of any size, and `floor[x]` of one became the growing form, which no
/// dual reaches (the interpreter did not run it, the compiled program cast
/// the dual to a float).
pub fn written_name(row: &'static Row) -> &'static str {
    if !row.written.is_empty() {
        return row.written;
    }
    ROWS.iter().find(|r| big_form(r.id).is_some_and(|b| std::ptr::eq(b, row))).map_or(row.name, |r| r.name)
}

/// The name a program writes for an effect's name (bare, or with its
/// module): a hidden form's `written`, else the bare name.
pub fn shown_effect(name: &str) -> &str {
    let bare = name.rsplit('.').next().unwrap_or(name);
    FORMS.iter().chain(ROWS.iter()).find(|r| r.name == bare && !r.written.is_empty()).map_or(bare, |r| r.written)
}

/// Whether a row's take path makes its answer of the value handed at
/// position `i` of `argc` (a list it pushes onto, a map it sets, a text it
/// changes where it lies): a value read for the last time there is moved
/// into the call; anywhere else the row only reads it, and it is lent.
pub fn keeps_storage(row: &Row, i: usize, argc: usize) -> bool {
    if i == 0 && row.taken.is_some() {
        return true;
    }
    kinds_of(row.params, argc).and_then(|k| k.get(i).copied()).is_some_and(|k| k.owned())
}

pub mod texts {
    use super::*;

    /// The text of the first argument, taken out of it, when every argument
    /// is a plain text (no failure riding on it); None for anything else.
    fn texts_taken(args: &mut [CrowValue]) -> Option<(String, Vec<&str>)> {
        if !args.iter().all(|a| a.is_ok() && matches!(a.value, Value::Str(_))) {
            return None;
        }
        let (first, rest) = args.split_first_mut()?;
        let Value::Str(s) = &mut first.value else { return None };
        let taken = std::mem::take(s);
        let rest: Vec<&str> = rest.iter().filter_map(|a| if let Value::Str(x) = &a.value { Some(x.as_str()) } else { None }).collect();
        Some((taken, rest))
    }

    fn by_row(id: &str, args: &[CrowValue], trace: &str) -> Result<CrowValue, String> {
        match ROWS.iter().find(|r| r.id == id) {
            Some(row) => (row.call)(args, trace),
            None => Err(format!("a bug in Crow: the row {} is gone", id)),
        }
    }

    /// `replace[s, old, new]` given the text: one ASCII character for
    /// another is written where it lies (crow_runtime::text::replace_taken);
    /// anything else is the row's own call.
    pub fn replace_taken(mut args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        if args.len() == 3 {
            if let Some((s, rest)) = texts_taken(&mut args) {
                return Ok(write_str(crow_runtime::text::replace_taken(s, rest[0], rest[1])));
            }
        }
        by_row("text.replace", &args, trace)
    }

    /// `translate[s, from, to]` given the text: mapped where it lies.
    pub fn translate_taken(mut args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        if args.len() == 3 {
            if let Some((s, rest)) = texts_taken(&mut args) {
                return crow_runtime::text::translate_taken(s, rest[0], rest[1]).map(write_str);
            }
        }
        by_row("text.translate", &args, trace)
    }

    /// `upper[s]` given the text: ASCII raised where it lies.
    pub fn upper_taken(mut args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        if args.len() == 1 {
            if let Some((s, _)) = texts_taken(&mut args) {
                return Ok(write_str(crow_runtime::text::upper_taken(s)));
            }
        }
        by_row("text.upper", &args, trace)
    }

    /// `reverse[s]` given the text: ASCII reversed where it lies.
    pub fn reverse_taken(mut args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        if args.len() == 1 {
            if let Some((s, _)) = texts_taken(&mut args) {
                return Ok(write_str(crow_runtime::core::reverse_text_taken(s)));
            }
        }
        by_row("core.reverse_text", &args, trace)
    }

    /// `lower[s]` given the text: ASCII lowered where it lies.
    pub fn lower_taken(mut args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        if args.len() == 1 {
            if let Some((s, _)) = texts_taken(&mut args) {
                return Ok(write_str(crow_runtime::text::lower_taken(s)));
            }
        }
        by_row("text.lower", &args, trace)
    }
}

pub mod tensors {
    use super::*;

    /// `push[t, x]` given the tensor: the number appended where the tensor
    /// lies when nothing else holds it (crow_runtime::core::push_number_taken);
    /// anything else, the row's own call, with its conversions and refusals.
    pub fn push_number_taken(mut args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        if let [t, x] = args.as_mut_slice()
            && t.is_ok()
            && x.is_ok()
            && let (Value::NumArray(tensor), Some(n)) = (&mut t.value, x.plain_float().or_else(|| x.plain_int().map(|i| i as f64)))
        {
            tensor.push_scalar_in_place(n)?;
            return Ok(std::mem::replace(t, CrowValue::done()));
        }
        match ROWS.iter().find(|r| r.id == "core.push_number") {
            Some(row) => (row.call)(&args, trace),
            None => Err("a bug in Crow: the row core.push_number is gone".to_string()),
        }
    }

    /// `m[i][j] = x`'s row given the tensor: the number written where the
    /// tensor lies when nothing else holds it
    /// (crow_runtime::core::set_at_in_place); anything else, the row's own
    /// call, with its conversions and refusals.
    pub fn set_at_taken(mut args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        if let [t, at, x] = args.as_mut_slice()
            && t.is_ok()
            && at.is_ok()
            && x.is_ok()
            && let (Value::NumArray(tensor), Ok(positions), Some(n)) = (&mut t.value, super::read_ints(at), x.plain_float().or_else(|| x.plain_int().map(|i| i as f64)))
        {
            crow_runtime::core::set_at_in_place(tensor, &positions, n)?;
            return Ok(std::mem::replace(t, CrowValue::done()));
        }
        match ROWS.iter().find(|r| r.id == "core.__set_at") {
            Some(row) => (row.call)(&args, trace),
            None => Err("a bug in Crow: the row core.__set_at is gone".to_string()),
        }
    }

    /// `m[i] = row`'s row given the tensor: the part written where the
    /// tensor lies when nothing else holds it
    /// (crow_runtime::core::set_part_in_place); anything else, the row's
    /// own call.
    pub fn set_part_taken(mut args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        if let [t, at, part] = args.as_mut_slice()
            && t.is_ok()
            && at.is_ok()
            && part.is_ok()
            && let (Value::NumArray(tensor), Ok(positions), Value::NumArray(p)) = (&mut t.value, super::read_ints(at), &part.value)
        {
            crow_runtime::core::set_part_in_place(tensor, &positions, p)?;
            return Ok(std::mem::replace(t, CrowValue::done()));
        }
        match ROWS.iter().find(|r| r.id == "core.__set_part") {
            Some(row) => (row.call)(&args, trace),
            None => Err("a bug in Crow: the row core.__set_part is gone".to_string()),
        }
    }
}

pub mod lists {
    use super::*;

    /// `slice[xs, a, b]` given the list: cut where it lies when nothing
    /// else holds its items (a shared list is the row's own call, a copy).
    pub fn slice_taken(mut args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        if let [xs, a, b] = args.as_mut_slice()
            && xs.is_ok()
            && let (Value::Array(items), Some(a), Some(b)) = (&mut xs.value, a.plain_int(), b.plain_int())
            && let Some(v) = std::sync::Arc::get_mut(items)
        {
            let v = crow_runtime::core::slice_taken(std::mem::take(v), a, b);
            return Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(v))));
        }
        match ROWS.iter().find(|r| r.id == "core.slice") {
            Some(row) => (row.call)(&args, trace),
            None => Err("a bug in Crow: the row core.slice is gone".to_string()),
        }
    }
}

pub mod maps {
    use super::*;
    use std::collections::BTreeMap;

    enum Keyed<'a> {
        Text(&'a BTreeMap<String, CrowValue>),
        Int(&'a BTreeMap<i64, CrowValue>),
    }
    // (a map a row takes is written where it lies when nothing else
    // holds its entries: `Arc::make_mut`)
    enum Owned {
        Text(std::sync::Arc<BTreeMap<String, CrowValue>>),
        Int(std::sync::Arc<BTreeMap<i64, CrowValue>>),
    }
    enum K {
        Text(String),
        Int(i64),
    }

    fn map_of_value(v: &CrowValue) -> Result<Keyed<'_>, String> {
        match &v.value {
            Value::Map(m) => Ok(Keyed::Text(m)),
            Value::IntMap(m) => Ok(Keyed::Int(m)),
            _ => Err(format!("expected a map, got {}", named(v))),
        }
    }
    fn map_taken(v: CrowValue) -> Result<Owned, String> {
        match v.value {
            Value::Map(m) => Ok(Owned::Text(m)),
            Value::IntMap(m) => Ok(Owned::Int(m)),
            other => Err(format!("expected a map, got {}", other.type_name())),
        }
    }
    fn key_of(v: &CrowValue) -> Result<K, String> {
        match &v.value {
            Value::Str(s) => Ok(K::Text(s.clone())),
            Value::Int(n) => Ok(K::Int(*n)),
            _ => Err(format!("a map's key is text or an int, got {}", named(v))),
        }
    }
    fn wrong(key: &K, keyed_by_ints: bool) -> String {
        match key {
            K::Int(n) if !keyed_by_ints => format!("the map's keys are text, not ints: {}", n),
            K::Text(t) => format!("the map's keys are ints, not text: '{}'", t),
            K::Int(n) => format!("the map's keys are text, not ints: {}", n),
        }
    }
    fn text(m: BTreeMap<String, CrowValue>) -> CrowValue {
        CrowValue::ok(Value::map(m))
    }
    fn ints(m: BTreeMap<i64, CrowValue>) -> CrowValue {
        CrowValue::ok(Value::int_map(m))
    }
    fn text_kept(m: std::sync::Arc<BTreeMap<String, CrowValue>>) -> CrowValue {
        CrowValue::ok(Value::Map(m))
    }
    fn ints_kept(m: std::sync::Arc<BTreeMap<i64, CrowValue>>) -> CrowValue {
        CrowValue::ok(Value::IntMap(m))
    }
    fn arg<'a>(args: &'a [CrowValue], i: usize, name: &str) -> Result<&'a CrowValue, String> {
        args.get(i).ok_or_else(|| format!("{}[] is missing its argument {}", name, i + 1))
    }

    pub fn new(_args: &[CrowValue], _trace: &str) -> Result<CrowValue, String> {
        Ok(text(BTreeMap::new()))
    }
    pub fn new_taken(_args: Vec<CrowValue>, _trace: &str) -> Result<CrowValue, String> {
        Ok(text(BTreeMap::new()))
    }

    pub fn set(args: &[CrowValue], _trace: &str) -> Result<CrowValue, String> {
        let (m, k, v) = (arg(args, 0, "set")?, arg(args, 1, "set")?, arg(args, 2, "set")?);
        match (map_of_value(m)?, key_of(k)?) {
            (Keyed::Text(m), K::Text(k)) => Ok(text(crow_runtime::core::set(m.clone(), &k, v.clone()))),
            (Keyed::Int(m), K::Int(k)) => Ok(ints(crow_runtime::core::set(m.clone(), &k, v.clone()))),
            // an open map takes its first key's kind
            (Keyed::Text(m), K::Int(k)) if m.is_empty() => Ok(ints(crow_runtime::core::set(BTreeMap::new(), &k, v.clone()))),
            (Keyed::Int(_), key) => Err(wrong(&key, true)),
            (Keyed::Text(_), key) => Err(wrong(&key, false)),
        }
    }
    pub fn set_taken(args: Vec<CrowValue>, _trace: &str) -> Result<CrowValue, String> {
        let mut it = args.into_iter();
        let (m, k, v) = (it.next().ok_or("set[] is missing its map")?, it.next().ok_or("set[] is missing its key")?, it.next().ok_or("set[] is missing its value")?);
        match (map_taken(m)?, key_of(&k)?) {
            (Owned::Text(mut m), K::Text(k)) => {
                crow_runtime::core::set_in(std::sync::Arc::make_mut(&mut m), &k, v);
                Ok(text_kept(m))
            }
            (Owned::Int(mut m), K::Int(k)) => {
                crow_runtime::core::set_in(std::sync::Arc::make_mut(&mut m), &k, v);
                Ok(ints_kept(m))
            }
            (Owned::Text(m), K::Int(k)) if m.is_empty() => Ok(ints(crow_runtime::core::set(BTreeMap::new(), &k, v))),
            (Owned::Int(_), key) => Err(wrong(&key, true)),
            (Owned::Text(_), key) => Err(wrong(&key, false)),
        }
    }

    pub fn del(args: &[CrowValue], _trace: &str) -> Result<CrowValue, String> {
        let (m, k) = (arg(args, 0, "del")?, arg(args, 1, "del")?);
        match (map_of_value(m)?, key_of(k)?) {
            (Keyed::Text(m), K::Text(k)) => Ok(text(crow_runtime::core::map_del(m.clone(), &k))),
            (Keyed::Int(m), K::Int(k)) => Ok(ints(crow_runtime::core::map_del(m.clone(), &k))),
            (Keyed::Int(_), key) => Err(wrong(&key, true)),
            (Keyed::Text(_), key) => Err(wrong(&key, false)),
        }
    }
    pub fn del_taken(args: Vec<CrowValue>, _trace: &str) -> Result<CrowValue, String> {
        let mut it = args.into_iter();
        let (m, k) = (it.next().ok_or("del[] is missing its map")?, it.next().ok_or("del[] is missing its key")?);
        match (map_taken(m)?, key_of(&k)?) {
            (Owned::Text(mut m), K::Text(k)) => {
                std::sync::Arc::make_mut(&mut m).remove(&k);
                Ok(text_kept(m))
            }
            (Owned::Int(mut m), K::Int(k)) => {
                std::sync::Arc::make_mut(&mut m).remove(&k);
                Ok(ints_kept(m))
            }
            (Owned::Int(_), key) => Err(wrong(&key, true)),
            (Owned::Text(_), key) => Err(wrong(&key, false)),
        }
    }

    pub fn has(args: &[CrowValue], _trace: &str) -> Result<CrowValue, String> {
        let (m, k) = (arg(args, 0, "in")?, arg(args, 1, "in")?);
        let found = match (map_of_value(m)?, key_of(k)?) {
            (Keyed::Text(m), K::Text(k)) => crow_runtime::core::has(m, &k),
            (Keyed::Int(m), K::Int(k)) => crow_runtime::core::has(m, &k),
            // (a key of the other kind is in no map of this one — an open
            // map holds no key yet)
            _ => false,
        };
        Ok(CrowValue::ok(Value::Bool(found)))
    }
    pub fn has_taken(args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        has(&args, trace)
    }

    pub fn keys(args: &[CrowValue], _trace: &str) -> Result<CrowValue, String> {
        let m = arg(args, 0, "keys")?;
        let out: Vec<CrowValue> = match map_of_value(m)? {
            Keyed::Text(m) => crow_runtime::core::keys(m).into_iter().map(|k| CrowValue::ok(Value::Str(k))).collect(),
            Keyed::Int(m) => crow_runtime::core::keys(m).into_iter().map(|k| CrowValue::ok(Value::Int(k))).collect(),
        };
        Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(out))))
    }
    pub fn keys_taken(args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        keys(&args, trace)
    }

    pub fn values(args: &[CrowValue], _trace: &str) -> Result<CrowValue, String> {
        let m = arg(args, 0, "values")?;
        let out = match map_of_value(m)? {
            Keyed::Text(m) => crow_runtime::core::values(m),
            Keyed::Int(m) => crow_runtime::core::values(m),
        };
        Ok(CrowValue::ok(Value::Array(std::sync::Arc::new(out))))
    }
    pub fn values_taken(args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        values(&args, trace)
    }

    pub fn len(args: &[CrowValue], _trace: &str) -> Result<CrowValue, String> {
        let m = arg(args, 0, "len")?;
        let n = match map_of_value(m)? {
            Keyed::Text(m) => crow_runtime::core::len_map(m),
            Keyed::Int(m) => crow_runtime::core::len_map(m),
        };
        Ok(CrowValue::ok(Value::Int(n)))
    }
    pub fn len_taken(args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        len(&args, trace)
    }

    /// `map[keys, values]`: text keys make a map keyed by text, ints one
    /// keyed by ints (a tensor of ints too); mixed keys are refused.
    pub fn map_of(args: &[CrowValue], _trace: &str) -> Result<CrowValue, String> {
        let (ks, vs) = (arg(args, 0, "map")?, arg(args, 1, "map")?);
        let values = read_seq(vs)?;
        let keys: Vec<CrowValue> = match &ks.value {
            Value::Array(items) => items.as_ref().clone(),
            Value::NumArray(t) => t.to_vec().into_iter().map(|x| CrowValue::ok(Value::Int(x as i64))).collect(),
            _ => return Err(format!("map[keys, values] takes a list of keys, got {}", named(ks))),
        };
        if keys.iter().all(|k| matches!(k.value, Value::Int(_))) {
            let ks: Vec<i64> = keys.iter().filter_map(|k| if let Value::Int(n) = k.value { Some(n) } else { None }).collect();
            return Ok(ints(crow_runtime::core::map_of(&ks, values)?));
        }
        if keys.iter().all(|k| matches!(k.value, Value::Str(_))) {
            let ks: Vec<String> = keys.iter().filter_map(|k| if let Value::Str(s) = &k.value { Some(s.clone()) } else { None }).collect();
            return Ok(text(crow_runtime::core::map_of(&ks, values)?));
        }
        Err("map[keys, values] takes keys of one kind: text or ints".to_string())
    }
    pub fn map_of_taken(args: Vec<CrowValue>, trace: &str) -> Result<CrowValue, String> {
        map_of(&args, trace)
    }
}
