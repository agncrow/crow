pub mod db;
pub mod math;

use std::collections::HashMap;
use std::sync::Arc;
use crate::value::CrowValue;
use crate::type_infer::Ty;

/// A module of the standard library, declared once (design rule 24): what
/// every place that had listed modules by name reads — the registry, the
/// type checker, the lowering's checks, the emitter, the inliner, the docs
/// check, the database drivers. Several flags transcribe lists the
/// compiler had kept apart, each named for what its readers make of it;
/// unifying them is a later phase's (docs/extending-crow.md).
pub struct Module {
    /// The import path (`math.stats`); "" the core, imported by no `use`.
    pub path: &'static str,
    /// The prefix its inlined functions carry (`_math_stats`).
    pub prefix: &'static str,
    /// Its source, where it is written in Crow (design rule 20): inlined
    /// into the program that imports it.
    pub source: Option<&'static str>,
    /// The modules its import brings beside its parent: `db` its drivers.
    pub loads: &'static [&'static str],
    /// The runtime crates its import needs built in, each with the
    /// crate's features.
    pub needs: &'static [(&'static str, &'static [&'static str])],
    /// Only the standard library may import it (`sys`), or nothing does (a
    /// driver, carried by `use db`).
    pub internal: bool,
    /// Its functions and constants are names a bare `use` brings into
    /// scope through the registry.
    pub registry: bool,
    /// Every function of it is a row of the table, which the registry
    /// answers by name; a module of own forms is answered by the compiler.
    pub rows: bool,
    /// Loaded into the tooling's default registry (`with_stdlib`): its
    /// functions are known names without an import.
    pub known: bool,
    /// A qualified call to it is its bare call, pinned to it (the emitter).
    pub pinned: bool,
    /// A qualified call to it is typed through the registry.
    pub typed: bool,
    /// Its import makes a page program: views, facts, the live runtime.
    pub ui: bool,
    /// The sentence its page of the book opens with; none for a module
    /// documented under another's section (`under`) or under none.
    pub docs: Option<&'static str>,
    /// The module whose section of the spec documents it.
    pub under: Option<&'static str>,
    /// A database driver written in Crow, carried by `use db`.
    pub driver: Option<Driver>,
    /// The registry's object for it, where it has one.
    pub module: Option<fn() -> Arc<dyn CrowModule>>,
}

/// A database driver: the backend's name as crow_db knows it, the URL
/// schemes that name it, and the prefix its rows carry (`ch_query`).
pub struct Driver {
    pub backend: &'static str,
    pub schemes: &'static [&'static str],
    pub rows: &'static str,
}

impl Module {
    /// A module with nothing declared but its path and prefix.
    const fn plain(path: &'static str, prefix: &'static str) -> Module {
        Module { path, prefix, source: None, loads: &[], needs: &[], internal: false, registry: false, rows: true, known: false, pinned: false, typed: false, ui: false, docs: None, under: None, driver: None, module: None }
    }
}

const RUNTIME: (&str, &[&str]) = ("crow_runtime", &[]);

/// The standard library, module by module.
pub static MODULES: &[Module] = &[
    // the core's own functions written in Crow (`request`): no import
    // brings them, the source is inlined into a program that calls one
    Module { source: Some(CORE_SOURCE), ..Module::plain("", "_core") },
    Module {
        source: Some(MATH_SOURCE), needs: &[RUNTIME], registry: true, known: true, pinned: true, typed: true,
        docs: Some("Tensors, element-wise math, reductions, linear algebra, random numbers, signal processing, sparse arrays, and numeric loading."),
        module: Some(|| Arc::new(math::MathModule)),
        ..Module::plain("math", "_math")
    },
    Module {
        source: Some(STATS_SOURCE), loads: &["math"], needs: &[RUNTIME], registry: true, known: true, pinned: true, typed: true,
        docs: Some("Descriptive statistics, correlation, tests, and fitting over tensors and frame columns."),
        module: Some(|| Arc::new(Named::new("math.stats"))),
        ..Module::plain("math.stats", "_math_stats")
    },
    Module {
        source: Some(NN_SOURCE), loads: &["math"], needs: &[RUNTIME], registry: true, known: true, pinned: true, typed: true,
        docs: Some("Activation functions, losses, and a linear layer for small networks with autograd."),
        module: Some(|| Arc::new(Named::new("math.nn"))),
        ..Module::plain("math.nn", "_math_nn")
    },
    Module {
        source: Some(IO_SOURCE), needs: &[RUNTIME], registry: true, known: true, pinned: true, typed: true,
        docs: Some("Structured file parsing beyond the built-in read and write."),
        module: Some(|| Arc::new(Named::new("io"))),
        ..Module::plain("io", "_io")
    },
    Module {
        source: Some(SYS_SOURCE), internal: true, registry: true, typed: true,
        docs: Some("The standard library's own layer: bytes, files, and a connection to a host, for the modules written in Crow and the packages that need a kernel's speed — meant for library code, which any program or package may import."),
        module: Some(|| Arc::new(Named::new("sys"))),
        ..Module::plain("sys", "_sys")
    },
    Module {
        source: Some(FRAME_SOURCE), needs: &[RUNTIME], registry: true, known: true, pinned: true, typed: true,
        docs: Some("Column-oriented data frames: load, select, masks, sort, group, join, and time handling."),
        module: Some(|| Arc::new(Named::new("frame"))),
        ..Module::plain("frame", "_frame")
    },
    Module {
        needs: &[RUNTIME], registry: true, known: true, pinned: true, typed: true,
        docs: Some("Documentation from Markdown, rendered by the kernel so both execution modes write the same pages."),
        module: Some(|| Arc::new(Named::new("doc"))),
        ..Module::plain("doc", "_doc")
    },
    Module {
        needs: &[RUNTIME], registry: true, known: true, pinned: true, typed: true,
        docs: Some("Technical indicators for the chart element, computed by the kernel so both execution modes draw the same numbers."),
        module: Some(|| Arc::new(Named::new("chart"))),
        ..Module::plain("chart", "_chart")
    },
    Module {
        needs: &[RUNTIME], registry: true, known: true, pinned: true,
        docs: Some("An int's bits — read as two's complement of any width — and a character's code."),
        module: Some(|| Arc::new(Named::new("bits"))),
        ..Module::plain("bits", "_bits")
    },
    // process memory: a record held for the process under a name, shared
    // by its threads — rows of the table, the store the runtime's; meant
    // for library code, as `sys` is (Vijay, 2026-10-29: a suggestion, not
    // a rule — any program or package may import it)
    Module {
        needs: &[RUNTIME], internal: true, registry: true, pinned: true,
        docs: Some("Process memory: a record the standard library holds for the process under a name, shared by its threads — remembered, recalled, revised under the name's lock, forgotten; what a module keeps between calls. Meant for library code — the standard library and packages that need it — which any program or package may import."),
        module: Some(|| Arc::new(Named::new("memory"))),
        ..Module::plain("memory", "_memory")
    },
    Module {
        source: Some(LLM_SOURCE), needs: &[RUNTIME], registry: true,
        docs: Some("A language model as a connection: conversations, replies, tools as the functions themselves, a reply followed piece by piece, and embeddings, over Anthropic's and the OpenAI-compatible wire formats."),
        module: Some(|| Arc::new(Named::new("llm"))),
        ..Module::plain("llm", "_llm")
    },
    // the log as a directory of batch files: one commit, one file, written
    // once by the kernel; the sequencer, the reads and the catalog in Crow
    Module {
        source: Some(MAGPIE_SOURCE), needs: &[], registry: true,
        docs: Some("The log as a directory of batch files: each commit one file written once, holding its rows' positions, sorted by key, with the statistics a reader prunes by; appends, reads from a position, the latest version of each key, lookups, and a sequencer that refuses a stale read."),
        module: Some(|| Arc::new(Named::new("magpie"))),
        ..Module::plain("magpie", "_magpie")
    },
    // the Kafka door of the magpie: the 0.10.1 protocol in Crow over the sys
    // layer's listener, the message sets the kernel's
    Module {
        source: Some(KAFKA_SOURCE), needs: &[], registry: true,
        docs: Some("The Kafka door of the magpie: a topic a log, a producer's records appended as a batch, a consumer's fetch a read from its offset, groups and their commits two logs of their own — the protocol a 0.10.1 broker speaks, served from one loop."),
        module: Some(|| Arc::new(Named::new("kafka"))),
        ..Module::plain("kafka", "_kafka")
    },
    // the RESP door of Magpie: Redis's protocol in Crow over the sys
    // layer's listener, the keys and the streams logs of Magpie's
    Module {
        source: Some(REDIS_SOURCE), needs: &[], registry: true,
        docs: Some("The RESP door of Magpie: a Redis client's SET and GET a put and a read of the image, INCR and MGET and DEL over a log keyed by the key, a stream a log of its own with XADD and XREAD by position — RESP2 as redis-cli and every client write it, served from one loop."),
        module: Some(|| Arc::new(Named::new("redis"))),
        ..Module::plain("redis", "_redis")
    },
    // Magpie's kernels in Crow (docs/magpie-in-crow.md), each a module
    // under magpie, imported by the verbs: the codec first
    Module {
        source: Some(CODEC_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("Magpie's codec, in Crow: the LZ77 byte codec a chunk's payload goes through where it shrinks it, the Rust kernel's stream byte for byte — compress and decompress over the sys layer's bytes."),
        module: Some(|| Arc::new(Named::new("magpie.codec"))),
        ..Module::plain("magpie.codec", "_magpie_codec")
    },
    Module {
        source: Some(PACKING_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("Bits and varints, in Crow: the two ways a number is written into a Magpie file — values at a width of bits each, little-endian, and LEB128 counts — the Rust kernel's bytes byte for byte."),
        module: Some(|| Arc::new(Named::new("magpie.packing"))),
        ..Module::plain("magpie.packing", "_magpie_packing")
    },
    Module {
        source: Some(ENCODE_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("A column chunk, in Crow: the cells of one column over one row group as a Magpie file writes them — ints by frame of reference or delta, floats as their bits, text plain or as a dictionary, bools as a bitmap, moments, vectors — through the codec where it shrinks them, the kernel's bytes byte for byte."),
        module: Some(|| Arc::new(Named::new("magpie.encode"))),
        ..Module::plain("magpie.encode", "_magpie_encode")
    },
    Module {
        source: Some(BLOOM_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("A bloom filter over byte strings, in Crow: k hashes into m bits, FNV-1a and a mix of it, the kernel's bits bit for bit — what a file keeps per column chunk, and per indexed text column over its tokens."),
        module: Some(|| Arc::new(Named::new("magpie.bloom"))),
        ..Module::plain("magpie.bloom", "_magpie_bloom")
    },
    Module {
        source: Some(STEM_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("The Porter stemmer, in Crow: a word's stem as the paper gives it, for a log that declares `stem` — running, runs and runner all run — the kernel's answer word for word."),
        module: Some(|| Arc::new(Named::new("magpie.stem"))),
        ..Module::plain("magpie.stem", "_magpie_stem")
    },
    Module {
        source: Some(POSTINGS_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("A text column's index over one row group, in Crow: the documents' lengths, a sorted dictionary of terms, and each term's postings in blocks with their bounds — the kernel's bytes byte for byte; terms found by word, by prefix and within an edit distance."),
        module: Some(|| Arc::new(Named::new("magpie.postings"))),
        ..Module::plain("magpie.postings", "_magpie_postings")
    },
    Module {
        source: Some(FILE_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("A Magpie file, in Crow: a frame written as row groups of column chunks with their statistics and filters, column blocks and a footer, and read back by region — a footer, one column's block, one group's chunk — the kernel's bytes byte for byte."),
        module: Some(|| Arc::new(Named::new("magpie.file"))),
        ..Module::plain("magpie.file", "_magpie_file")
    },
    Module {
        source: Some(MANIFEST_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("A log's manifest, in Crow: its live segments and what the log declares of itself — its keys, its time column, its narrow columns, whether it stems — a Magpie file of one group replaced whole, the kernel's bytes byte for byte; and the names a log's files carry."),
        module: Some(|| Arc::new(Named::new("magpie.manifest"))),
        ..Module::plain("magpie.manifest", "_magpie_manifest")
    },
    Module {
        source: Some(MERGE_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("The streaming merge, in Crow: batches and segments folded into segments of bounded size, the rows in the log's keys' order then position, the newest version of each key kept — one group per input held — and the manifest switched in one step; the kernel's segments byte for byte."),
        module: Some(|| Arc::new(Named::new("magpie.merge"))),
        ..Module::plain("magpie.merge", "_magpie_merge")
    },
    Module {
        source: Some(TAIL_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("The sequencer, in Crow: a log's rows put since the last flush, numbered as they arrive, held for the process in memory with the key index and the newest row of each key, flushed as one batch file by work a put asks for — what a reader of the process sees at once, and another once flushed."),
        module: Some(|| Arc::new(Named::new("magpie.tail"))),
        ..Module::plain("magpie.tail", "_magpie_tail")
    },
    Module {
        source: Some(EXEC_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("The executor, in Crow: aggregates over a log's files pushed below the reader — each row group pruned by its statistics and filters, its rows masked and grouped into partials merged in order — and the as-of join, each label's newest row at or before its moment; the kernel's answers exactly."),
        module: Some(|| Arc::new(Named::new("magpie.exec"))),
        ..Module::plain("magpie.exec", "_magpie_exec")
    },
    Module {
        source: Some(NEAREST_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("The nearest rows by a vector, in Crow: every present row of a log's vector column scored by the cosine with the query, a group at a time, the best k kept — a greater score first, an earlier position between equals — the kernel's answer row for row."),
        module: Some(|| Arc::new(Named::new("magpie.nearest"))),
        ..Module::plain("magpie.nearest", "_magpie_nearest")
    },
    Module {
        source: Some(SEARCH_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("Search over a log, in Crow: BM25 as Lucene scores it over the whole log as one corpus — a word, a phrase, a prefix, a fuzzy word — from a group's index where it has one and its text where it has none, the blocks that cannot reach the k-th score skipped, a snippet around the first hit; the kernel's ranking and scores to the last digit."),
        module: Some(|| Arc::new(Named::new("magpie.search"))),
        ..Module::plain("magpie.search", "_magpie_search")
    },
    Module {
        source: Some(INFLATE_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("Inflate and gzip, in Crow: a raw deflate stream's stored, fixed and dynamic Huffman blocks decoded, a gzip member's header read and its CRC-32 and length checked — the decoder the Kafka door reads a producer's compressed records with."),
        module: Some(|| Arc::new(Named::new("magpie.inflate"))),
        ..Module::plain("magpie.inflate", "_magpie_inflate")
    },
    Module {
        source: Some(SNAPPY_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("Snappy's raw block format decoded, in Crow: literal runs and copies of three widths — the decoder the Kafka door reads a producer's snappy-compressed records with."),
        module: Some(|| Arc::new(Named::new("magpie.snappy"))),
        ..Module::plain("magpie.snappy", "_magpie_snappy")
    },
    Module {
        source: Some(RECORDS_SOURCE), needs: &[], registry: true, under: Some("magpie"),
        docs: Some("The Kafka wire's record sets, in Crow: a legacy message set decoded into a frame and encoded from one, each message's CRC-32 checked and written, a compressed wrapper read as the records it holds — the kernel's bytes byte for byte."),
        module: Some(|| Arc::new(Named::new("magpie.records"))),
        ..Module::plain("magpie.records", "_magpie_records")
    },
    // tools from other programs, and a program as one: the client on
    // request, the server a function a route calls (it answers responses,
    // so crow_web comes with the import)
    Module {
        source: Some(MCP_SOURCE), needs: &[("crow_web", &[]), RUNTIME], registry: true,
        docs: Some("Tools from other programs, and a Crow program as one: the Model Context Protocol over Streamable HTTP — a server named, its tools listed and called, and a route answering as a server over the functions themselves."),
        module: Some(|| Arc::new(Named::new("mcp"))),
        ..Module::plain("mcp", "_mcp")
    },
    // the database: its verbs are forms of their own, its drivers written
    // in Crow are carried by its import
    Module {
        loads: &["db.clickhouse", "db.postgres"], needs: &[("crow_db", &["sqlite"]), RUNTIME], rows: false, known: true, typed: true,
        docs: Some("Databases as connections: every statement through query, bulk insert, transactions, the tables a program declares, and pooling (migrations are the tool's: crow db apply)."),
        module: Some(|| Arc::new(db::DbModule)),
        ..Module::plain("db", "_db")
    },
    Module {
        source: Some(CLICKHOUSE_SOURCE), internal: true, driver: Some(Driver { backend: "clickhouse", schemes: &["clickhouse://"], rows: "ch" }),
        module: Some(|| Arc::new(Named::new("db.clickhouse"))),
        ..Module::plain("db.clickhouse", "_db_clickhouse")
    },
    Module {
        source: Some(POSTGRES_SOURCE), internal: true, driver: Some(Driver { backend: "postgres", schemes: &["postgres://", "postgresql://"], rows: "pg" }),
        module: Some(|| Arc::new(Named::new("db.postgres"))),
        ..Module::plain("db.postgres", "_db_postgres")
    },
    // the server and its modules: its names written in Crow are rows, the
    // routes, pages and responses forms of their own
    Module {
        source: Some(WEB_SOURCE), needs: &[("crow_web", &["http"]), RUNTIME],
        docs: Some("The server: routes and responses, auth, CORS, sessions, templates, cache, rate limits, uploads, SSE, sockets, pages, static files, mail, queues, API docs, and the HTTP client."),
        module: Some(|| Arc::new(Named::new("web"))),
        ..Module::plain("web", "_web")
    },
    Module { source: Some(AUTH_SOURCE), needs: &[("crow_web", &[]), RUNTIME], under: Some("web"), module: Some(|| Arc::new(Named::new("web.auth"))), ..Module::plain("web.auth", "_web_auth") },
    Module { source: Some(WS_SOURCE), needs: &[("crow_web", &[]), RUNTIME], under: Some("web"), module: Some(|| Arc::new(Named::new("web.ws"))), ..Module::plain("web.ws", "_web_ws") },
    Module { source: Some(MAIL_SOURCE), needs: &[("crow_web", &[]), RUNTIME], under: Some("web"), module: Some(|| Arc::new(Named::new("web.mail"))), ..Module::plain("web.mail", "_web_mail") },
    // the page: views, facts and the live runtime, by either import
    Module {
        needs: &[("crow_web", &["live"])], rows: false, ui: true,
        docs: Some("Views and the element vocabulary, reactive state and inputs, the layout vocabulary, charts, and styling."),
        ..Module::plain("ui", "_ui")
    },
    Module { needs: &[("crow_web", &["live"])], rows: false, ui: true, under: Some("web"), ..Module::plain("web.ui", "_web_ui") },
];

/// A module by its import path.
pub fn by_path(path: &str) -> Option<&'static Module> {
    MODULES.iter().find(|m| m.path == path)
}

/// The module a path is a module of (`math` of `math.stats`), where that
/// is a module.
pub fn parent_of(path: &str) -> Option<&'static str> {
    let (parent, _) = path.rsplit_once('.')?;
    by_path(parent).map(|m| m.path)
}

/// What importing a module brings with it: its parent, and what it loads.
pub fn brought_by(path: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = parent_of(path).into_iter().collect();
    if let Some(m) = by_path(path) {
        out.extend(m.loads.iter().copied());
    }
    out
}

/// The modules written in Crow, each with the prefix its functions carry.
pub fn crow_modules() -> impl Iterator<Item = (&'static str, &'static str)> {
    MODULES.iter().filter(|m| m.source.is_some()).map(|m| (m.path, m.prefix))
}

/// The source of a standard module written in Crow (design rule 20), by
/// its import path. Each is inlined into the program that imports it, its
/// functions named by `crate::table::crow_fn_name` (`_math_nn_relu`).
pub fn crow_source(path: &str) -> Option<&'static str> {
    by_path(path)?.source
}

/// A module every function of which is a row of the table: the registry
/// answers it by its rows' names.
pub struct Named {
    path: &'static str,
    names: Vec<&'static str>,
}

impl Named {
    pub fn new(path: &'static str) -> Self {
        Self { path, names: crate::table::module_names(path) }
    }
}

impl CrowModule for Named {
    fn name(&self) -> &str {
        self.path
    }

    fn functions(&self) -> &[&str] {
        &self.names
    }
}

/// A token signed and read, written in Crow (`use web.auth`).
pub(crate) const AUTH_SOURCE: &str = include_str!("auth.co");

/// A value packed as MessagePack, written in Crow (`use web`).
pub(crate) const WEB_SOURCE: &str = include_str!("web.co");

/// The WebSocket client, written in Crow (`use web.ws`).
pub(crate) const WS_SOURCE: &str = include_str!("ws.co");

/// Mail over SMTP, written in Crow (`use web.mail`).
pub(crate) const MAIL_SOURCE: &str = include_str!("mail.co");

/// The statistics, written in Crow (`use math.stats`).
pub(crate) const STATS_SOURCE: &str = include_str!("stats.co");

/// The neural networks, written in Crow over the math builtins (`use math.nn`).
pub(crate) const NN_SOURCE: &str = include_str!("nn.co");

/// The ClickHouse driver, written in Crow (carried by `use db`).
pub(crate) const CLICKHOUSE_SOURCE: &str = include_str!("clickhouse.co");

/// The Postgres driver, written in Crow (carried by `use db`).
pub(crate) const POSTGRES_SOURCE: &str = include_str!("postgres.co");

/// The protocol the HTTP client and the readers stand on, written in Crow
/// over the sys primitives: `sys.exchange`, `sys.fetch`, `sys.put`, the
/// formats; a module that imports `sys` inlines it.
pub(crate) const SYS_SOURCE: &str = include_str!("sys.co");
/// `use llm`: a language model as a connection, written in Crow.
pub(crate) const LLM_SOURCE: &str = include_str!("llm.co");
pub(crate) const MCP_SOURCE: &str = include_str!("mcp.co");
/// The log over batch files, written in Crow on the sys layer.
pub(crate) const MAGPIE_SOURCE: &str = include_str!("magpie.co");
/// The Kafka door, written in Crow over the magpie and the sys layer.
pub(crate) const KAFKA_SOURCE: &str = include_str!("kafka.co");
/// The RESP door, written in Crow over Magpie and the sys layer.
pub(crate) const REDIS_SOURCE: &str = include_str!("redis.co");
pub(crate) const CODEC_SOURCE: &str = include_str!("codec.co");
pub(crate) const PACKING_SOURCE: &str = include_str!("packing.co");
pub(crate) const ENCODE_SOURCE: &str = include_str!("encode.co");
pub(crate) const BLOOM_SOURCE: &str = include_str!("bloom.co");
pub(crate) const STEM_SOURCE: &str = include_str!("stem.co");
pub(crate) const POSTINGS_SOURCE: &str = include_str!("postings.co");
pub(crate) const FILE_SOURCE: &str = include_str!("file.co");
pub(crate) const MANIFEST_SOURCE: &str = include_str!("manifest.co");
pub(crate) const MERGE_SOURCE: &str = include_str!("merge.co");
pub(crate) const TAIL_SOURCE: &str = include_str!("tail.co");
pub(crate) const EXEC_SOURCE: &str = include_str!("exec.co");
pub(crate) const NEAREST_SOURCE: &str = include_str!("nearest.co");
pub(crate) const SEARCH_SOURCE: &str = include_str!("search.co");
pub(crate) const INFLATE_SOURCE: &str = include_str!("inflate.co");
pub(crate) const SNAPPY_SOURCE: &str = include_str!("snappy.co");
pub(crate) const RECORDS_SOURCE: &str = include_str!("records.co");
/// `io.lines`, `io.csv`: readers written in Crow.
pub(crate) const IO_SOURCE: &str = include_str!("io.co");
/// `frame.load`, `frame.save`: written in Crow.
pub(crate) const FRAME_SOURCE: &str = include_str!("frame.co");
/// `math.load`: written in Crow.
pub(crate) const MATH_SOURCE: &str = include_str!("math.co");

/// The core builtins written in Crow (`request`): no import brings them,
/// so the source is inlined into a program that calls one of them.
pub(crate) const CORE_SOURCE: &str = include_str!("core.co");

thread_local! {
    /// What each inlined module imports (its own `use` lines), by its
    /// prefix: the registry loads them into the module's scope.
    static CROW_IMPORTS: std::cell::RefCell<Vec<(String, Vec<String>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A standard module by its import path, for a registry's scope.
pub fn module_by_path(path: &str) -> Option<Arc<dyn CrowModule>> {
    Some((by_path(path)?.module?)())
}

/// Every function of every module a program can import, by name: what the
/// checker's undefined-variable rule counts as defined (`with_stdlib` loads
/// the core set; the modules a program imports by path — llm, web.ws, … —
/// are read here, or a part's `reach` and `chat` were "undefined").
pub fn every_module_function() -> Vec<String> {
    MODULES.iter().filter(|m| !m.internal).filter_map(|m| m.module).flat_map(|make| make().functions().iter().map(|f| f.to_string()).collect::<Vec<_>>()).collect()
}

/// The standard modules written in Crow that a program carries, by the
/// functions inlined behind their prefixes, each with its own imports
/// (its source's `use` lines) — read from the program itself, since a
/// harness lowers several programs in turn (a part's tests, the app).
pub fn crow_modules_carried<'a>(fn_names: impl Iterator<Item = &'a str>) -> Vec<(String, Vec<String>)> {
    let prefixes: std::collections::BTreeSet<String> = fn_names.filter_map(crow_scope_of).collect();
    prefixes
        .into_iter()
        .filter_map(|prefix| {
            let (path, _) = crow_modules().find(|(_, p)| *p == prefix)?;
            let imports: Vec<String> = crow_source(path)?
                .lines()
                .filter_map(|l| l.strip_prefix("use "))
                .flat_map(|rest| rest.split(',').map(|m| m.trim().split(' ').next().unwrap_or("").to_string()))
                .filter(|m| !m.is_empty())
                .collect();
            Some((prefix, imports))
        })
        .collect()
}

/// Every module a program carries, its imports loaded into its own scope
/// of a registry (a module's `use math.nn` loads math too, as a program's
/// does), given the program's function names.
pub fn load_crow_scopes<'a>(registry: &mut ModuleRegistry, fn_names: impl Iterator<Item = &'a str>) {
    for (prefix, imports) in crow_modules_carried(fn_names) {
        // (a module's own rows are in its scope: the protocol written in
        // sys.co calls sys's primitives)
        if let Some((own, _)) = crow_modules().find(|(_, p)| *p == prefix) {
            if let Some(m) = module_by_path(own) {
                registry.load_for(&prefix, m);
            }
        }
        for path in imports {
            for brought in brought_by(&path) {
                if let Some(m) = module_by_path(brought) {
                    registry.load_for(&prefix, m);
                }
            }
            if let Some(m) = module_by_path(&path) {
                registry.load_for(&prefix, m);
            }
        }
    }
}

/// The imports of every standard module the program inlined last, by
/// prefix (what the inliner recorded; the scopes are read from the
/// program itself, `crow_modules_carried`).
#[cfg(test)]
pub fn crow_imports() -> Vec<(String, Vec<String>)> {
    CROW_IMPORTS.with(|c| c.borrow().clone())
}

/// The scope a function's names are resolved in: a standard module's
/// prefix for one of its functions (inlined, or its traced copy), none for
/// the program's own.
/// `crow_scope_of`, borrowed: the module's prefix as the table holds it.
pub fn crow_scope_str(name: &str) -> Option<&'static str> {
    let name = name.strip_prefix("_traced_").unwrap_or(name);
    crow_modules()
        .filter(|(_, prefix)| name.starts_with(prefix) && name.len() > prefix.len() && name.as_bytes()[prefix.len()] == b'_')
        .max_by_key(|(_, prefix)| prefix.len())
        .map(|(_, p)| p)
}

/// The scope a call of the named function runs in: its standard module's,
/// or the program's.
pub fn scope_of_call(name: &str) -> &'static str {
    crow_scope_str(name).unwrap_or(PROGRAM)
}

pub fn crow_scope_of(name: &str) -> Option<String> {
    let name = name.strip_prefix("_traced_").unwrap_or(name);
    // (the longest prefix that fits: `_web_auth_sign` is web.auth's, not web's)
    crow_modules()
        .filter(|(_, prefix)| name.starts_with(prefix) && name.len() > prefix.len() && name.as_bytes()[prefix.len()] == b'_')
        .max_by_key(|(_, prefix)| prefix.len())
        .map(|(_, p)| p.to_string())
}

/// Whether a function of the program is a standard module's, inlined: its
/// stops name the line it was called from, not a line of the module's.
pub fn is_crow_fn(name: &str) -> bool {
    let name = name.strip_prefix("_traced_").unwrap_or(name);
    crow_modules().any(|(_, prefix)| name.starts_with(prefix) && name.len() > prefix.len() && name.as_bytes()[prefix.len()] == b'_')
}

/// The functions a module written in Crow declares for programs: its
/// top-level functions whose names begin with no underscore — a helper's
/// does (`_talk_of`), and is the module's own — apart from the functions a
/// row of the table still names (an effect's, a variadic's, a driver's;
/// docs/extending-crow.md, phase 2), which are reached through the row. A
/// record a module declares is its own until records are declared for
/// programs (phase 5). Parsed from the source once, for every module at
/// once; a source that does not parse declares nothing here, and is
/// reported where it is inlined.
pub fn exports(path: &str) -> &'static [&'static str] {
    static EXPORTS: std::sync::LazyLock<HashMap<&'static str, Vec<&'static str>>> = std::sync::LazyLock::new(|| {
        MODULES
            .iter()
            .filter_map(|m| {
                let stmts = parse_source(m.source?).ok()?;
                let rows: Vec<&str> = crate::table::ROWS.iter().filter(|r| r.module == m.path && r.is_crow()).flat_map(|r| [r.name, crate::table::crow_fn_base(r)]).collect();
                let names: Vec<&'static str> = stmts
                    .iter()
                    .filter_map(|s| match s {
                        crate::ast::Stmt::FnDecl { name, .. } => Some(name.as_str()),
                        _ => None,
                    })
                    .filter(|n| !n.starts_with('_') && !rows.contains(n))
                    .map(|n| &*Box::leak(n.to_string().into_boxed_str()))
                    .collect();
                Some((m.path, names))
            })
            .collect()
    });
    EXPORTS.get(path).map_or(&[], |v| v.as_slice())
}

/// A module's source, parsed.
fn parse_source(source: &str) -> Result<Vec<crate::ast::Stmt>, String> {
    let mut lexer = crate::lexer::Lexer::new(source);
    let tokens = lexer.tokenize()?;
    crate::parser::Parser::new(tokens).parse()
}

/// A message with every inlined function named as the program writes it:
/// `_llm_chat` is `chat`, `_math_stats_median` is `median` — the prefix a
/// module's functions carry is no program's spelling.
pub fn unprefixed(msg: &str) -> String {
    let mut prefixes: Vec<&'static str> = crow_modules().map(|(_, p)| p).collect();
    prefixes.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let chars: Vec<char> = msg.chars().collect();
    let mut out = String::with_capacity(msg.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '_' && (i == 0 || !is_word(chars[i - 1])) {
            let rest: String = chars[i..].iter().collect();
            if let Some(p) = prefixes.iter().find(|p| rest.starts_with(&format!("{}_", p)) && rest[p.len() + 1..].starts_with(|c: char| c.is_alphabetic())) {
                i += p.len() + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// An inlined function's name as the program writes it: the prefix gone
/// (`unprefixed`), and a copy's or a traced version's suffix (`relu__2`,
/// `_traced_…`) with it.
pub fn written_as(name: &str) -> String {
    let name = name.strip_prefix("_traced_").unwrap_or(name);
    let name = unprefixed(name);
    match name.rsplit_once("__") {
        Some((base, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => base.to_string(),
        _ => name,
    }
}

/// The names a body binds — its functions, records, derivations, tests,
/// variables, parameters and loop variables — at every depth.
fn bound_in_stmt(st: &crate::ast::Stmt, out: &mut std::collections::BTreeSet<String>) {
    use crate::ast::Stmt;
    match st {
        Stmt::Assign { name, value } => {
            out.insert(name.clone());
            bound_in_expr(value, out);
        }
        Stmt::MultiAssign { names, values } => {
            out.extend(names.iter().cloned());
            values.iter().for_each(|v| bound_in_expr(v, out));
        }
        Stmt::FnDecl { name, params, body, .. } => {
            out.insert(name.clone());
            out.extend(params.iter().cloned());
            body.iter().for_each(|s| bound_in_stmt(s, out));
        }
        Stmt::TestBlock { params, body, .. } => {
            out.extend(params.iter().cloned());
            body.iter().for_each(|s| bound_in_stmt(s, out));
        }
        Stmt::StructDecl { name, .. } => {
            out.insert(name.clone());
        }
        Stmt::Derive { name, body, .. } => {
            out.insert(name.clone());
            body.iter().for_each(|s| bound_in_stmt(s, out));
        }
        Stmt::For { var, iter, body } => {
            out.insert(var.clone());
            bound_in_expr(iter, out);
            body.iter().for_each(|s| bound_in_stmt(s, out));
        }
        Stmt::While { condition, body } => {
            bound_in_expr(condition, out);
            body.iter().for_each(|s| bound_in_stmt(s, out));
        }
        Stmt::If { condition, then_body, else_body } => {
            bound_in_expr(condition, out);
            then_body.iter().for_each(|s| bound_in_stmt(s, out));
            if let Some(eb) = else_body {
                eb.iter().for_each(|s| bound_in_stmt(s, out));
            }
        }
        Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => bound_in_expr(e, out),
        Stmt::FieldAssign { object, value, .. } => {
            bound_in_expr(object, out);
            bound_in_expr(value, out);
        }
        Stmt::IndexAssign { object, index, value } => {
            bound_in_expr(object, out);
            bound_in_expr(index, out);
            bound_in_expr(value, out);
        }
        Stmt::Return(None) | Stmt::Use { .. } | Stmt::Break | Stmt::Continue | Stmt::Line(_) => {}
    }
}

fn bound_in_expr(e: &crate::ast::Expr, out: &mut std::collections::BTreeSet<String>) {
    use crate::ast::Expr;
    match e {
        Expr::ForExpr { var, iter, body } => {
            out.insert(var.clone());
            bound_in_expr(iter, out);
            body.iter().for_each(|s| bound_in_stmt(s, out));
        }
        Expr::BinOp { left, right, .. } => {
            bound_in_expr(left, out);
            bound_in_expr(right, out);
        }
        Expr::UnaryOp { expr, .. } | Expr::Named { value: expr, .. } | Expr::FieldAccess { object: expr, .. } => bound_in_expr(expr, out),
        Expr::Call { callee, args } => {
            bound_in_expr(callee, out);
            args.iter().for_each(|a| bound_in_expr(a, out));
        }
        Expr::Array(items) => items.iter().for_each(|i| bound_in_expr(i, out)),
        Expr::Map(pairs) => pairs.iter().for_each(|(k, v)| {
            bound_in_expr(k, out);
            bound_in_expr(v, out);
        }),
        Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Ident(_) => {}
    }
}

/// The spelling of a module an access names (`math.nn` of `math.nn.relu`),
/// or none where the object is no chain of names.
fn spelled(e: &crate::ast::Expr) -> Option<String> {
    use crate::ast::Expr;
    match e {
        Expr::Ident(n) => Some(n.clone()),
        Expr::FieldAccess { object, field } => Some(format!("{}.{}", spelled(object)?, field)),
        _ => None,
    }
}

/// Names rewritten through a body, at every depth and inside the
/// interpolations of its strings: a bare name to the function it reaches
/// (an export's inlined name, a helper's), unless a parameter or a loop
/// variable shadows it; a module's spelling and a name — `math.nn.relu`,
/// `s.median` under `use math.stats as s` — to the same. A field after a
/// dot is a field, and an assignment's target is never rewritten.
struct Rewrite {
    bare: std::collections::BTreeMap<String, String>,
    qualified: Vec<(String, std::collections::BTreeMap<String, String>)>,
}

impl Rewrite {
    fn qualified_name(&self, spelling: &str, name: &str) -> Option<&String> {
        self.qualified.iter().find(|(s, _)| s == spelling).and_then(|(_, m)| m.get(name))
    }

    fn stmt(&self, st: &mut crate::ast::Stmt, shadowed: &std::collections::HashSet<String>) {
        use crate::ast::Stmt;
        match st {
            Stmt::Assign { value, .. } => self.expr(value, shadowed),
            Stmt::MultiAssign { values, .. } => values.iter_mut().for_each(|v| self.expr(v, shadowed)),
            Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => self.expr(e, shadowed),
            Stmt::If { condition, then_body, else_body } => {
                self.expr(condition, shadowed);
                then_body.iter_mut().for_each(|s| self.stmt(s, shadowed));
                if let Some(eb) = else_body {
                    eb.iter_mut().for_each(|s| self.stmt(s, shadowed));
                }
            }
            Stmt::For { var, iter, body } => {
                self.expr(iter, shadowed);
                let mut inner = shadowed.clone();
                inner.insert(var.clone());
                body.iter_mut().for_each(|s| self.stmt(s, &inner));
            }
            Stmt::While { condition, body } => {
                self.expr(condition, shadowed);
                body.iter_mut().for_each(|s| self.stmt(s, shadowed));
            }
            // a function sees its parameters and its own locals, never the
            // program's top-level names (spec, No Ambient Mutability): a
            // name it binds is its own wherever in its body it is read, and
            // nowhere else (a local `head = …` in one function had kept
            // every other function's `head[d]` from reaching the export:
            // "module 'magpie' has no function 'head'", in both modes); a
            // test block sees the program's values as a caller does
            Stmt::FnDecl { params, body, .. } => {
                let mut inner: std::collections::HashSet<String> = params.iter().cloned().collect();
                let mut locals = std::collections::BTreeSet::new();
                body.iter().for_each(|s| bound_in_stmt(s, &mut locals));
                inner.extend(locals);
                body.iter_mut().for_each(|s| self.stmt(s, &inner));
            }
            Stmt::TestBlock { params, body, .. } => {
                let mut inner = shadowed.clone();
                inner.extend(params.iter().cloned());
                let mut locals = std::collections::BTreeSet::new();
                body.iter().for_each(|s| bound_in_stmt(s, &mut locals));
                inner.extend(locals);
                body.iter_mut().for_each(|s| self.stmt(s, &inner));
            }
            Stmt::Derive { body, .. } => body.iter_mut().for_each(|s| self.stmt(s, shadowed)),
            Stmt::FieldAssign { object, value, .. } => {
                self.expr(object, shadowed);
                self.expr(value, shadowed);
            }
            Stmt::IndexAssign { object, index, value } => {
                self.expr(object, shadowed);
                self.expr(index, shadowed);
                self.expr(value, shadowed);
            }
            Stmt::Return(None) | Stmt::StructDecl { .. } | Stmt::Use { .. } | Stmt::Break | Stmt::Continue | Stmt::Line(_) => {}
        }
    }

    fn expr(&self, e: &mut crate::ast::Expr, shadowed: &std::collections::HashSet<String>) {
        use crate::ast::Expr;
        if let Expr::FieldAccess { object, field } = e {
            if let Some(prefixed) = spelled(object).and_then(|s| self.qualified_name(&s, field)) {
                *e = Expr::Ident(prefixed.clone());
                return;
            }
        }
        match e {
            Expr::Ident(n) => {
                if !shadowed.contains(n) {
                    if let Some(p) = self.bare.get(n) {
                        *n = p.clone();
                    }
                }
            }
            Expr::BinOp { left, right, .. } => {
                self.expr(left, shadowed);
                self.expr(right, shadowed);
            }
            Expr::UnaryOp { expr, .. } | Expr::Named { value: expr, .. } | Expr::FieldAccess { object: expr, .. } => self.expr(expr, shadowed),
            Expr::Call { callee, args } => {
                self.expr(callee, shadowed);
                args.iter_mut().for_each(|a| self.expr(a, shadowed));
            }
            Expr::Array(items) => items.iter_mut().for_each(|i| self.expr(i, shadowed)),
            Expr::Map(pairs) => pairs.iter_mut().for_each(|(k, v)| {
                self.expr(k, shadowed);
                self.expr(v, shadowed);
            }),
            Expr::Str(s) => {
                if s.contains('`') {
                    *s = self.interpolated(s, shadowed);
                }
            }
            Expr::ForExpr { var, iter, body } => {
                self.expr(iter, shadowed);
                let mut inner = shadowed.clone();
                inner.insert(var.clone());
                body.iter_mut().for_each(|s| self.stmt(s, &inner));
            }
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) => {}
        }
    }

    /// The names between the backticks of a string: a chain of names
    /// (`math.nn.relu`) as a module's spelling and a name, a bare name as a
    /// bare name; the fields after a chain's head are fields.
    fn interpolated(&self, s: &str, shadowed: &std::collections::HashSet<String>) -> String {
        let chars: Vec<char> = s.chars().collect();
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let mut out = String::with_capacity(s.len());
        let mut in_span = false;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '`' {
                in_span = !in_span;
                out.push(c);
                i += 1;
                continue;
            }
            if in_span && (c.is_alphabetic() || c == '_') && (i == 0 || (!is_word(chars[i - 1]) && chars[i - 1] != '.')) {
                // the chain of names at i: words joined by dots
                let mut j = i;
                let mut words: Vec<String> = Vec::new();
                loop {
                    let start = j;
                    while j < chars.len() && is_word(chars[j]) {
                        j += 1;
                    }
                    words.push(chars[start..j].iter().collect());
                    if j + 1 < chars.len() && chars[j] == '.' && (chars[j + 1].is_alphabetic() || chars[j + 1] == '_') {
                        j += 1;
                        continue;
                    }
                    break;
                }
                let head = &words[0];
                let replaced = if words.len() > 1 && !shadowed.contains(head) {
                    let spelling = words[..words.len() - 1].join(".");
                    self.qualified_name(&spelling, &words[words.len() - 1]).cloned()
                } else {
                    None
                };
                match replaced {
                    Some(name) => out.push_str(&name),
                    None => {
                        match self.bare.get(head).filter(|_| !shadowed.contains(head)) {
                            Some(p) => out.push_str(p),
                            None => out.push_str(head),
                        }
                        for w in &words[1..] {
                            out.push('.');
                            out.push_str(w);
                        }
                    }
                }
                i = j;
                continue;
            }
            out.push(c);
            i += 1;
        }
        out
    }
}

/// The program with every standard module it imports that is written in
/// Crow inlined after the import: the module's functions the program
/// reaches — an exported function (`exports`) called by a bare name, by
/// the module's path or its alias, or handed as a value, by the program or
/// by a function kept so far; a row's function, reached the same way by
/// the row's name — and the helpers they reach, each renamed behind the
/// module's prefix. A module's own imports and the core are reached the
/// same way (io's readers are written over sys's protocol and the core's
/// `read`), to a fixpoint, so a program that uses math without `load`
/// carries none of the protocol. A call of an exported function is
/// rewritten to the function's new name wherever it is made — in the
/// program, where the program binds no such name itself (a clash with an
/// import is refused where names are checked) and one module in its bare
/// scope exports it (two is an ambiguity, refused there too), and in the
/// modules that import it; a call of a row's name stays bare, so it is
/// the row's call wherever it is made — read as the row's parameters take
/// it, one signature. The module's tests, its own imports and its line
/// marks are left out (the import loads what the module needs; a stop
/// inside one of its functions names the line of the call in the
/// program).
pub fn inline_crow_modules(stmts: &[crate::ast::Stmt]) -> Result<Vec<crate::ast::Stmt>, String> {
    use crate::ast::Stmt;
    use std::collections::{BTreeMap, BTreeSet, HashSet};
    CROW_IMPORTS.with(|c| c.borrow_mut().clear());
    // the modules the program may reach — what it imports, each one's own
    // imports, and the core — parsed once each
    struct Module {
        path: String,
        prefix: &'static str,
        imports: Vec<String>,
        stmts: Vec<Stmt>,
    }
    let mut modules: Vec<Module> = Vec::new();
    let mut todo: Vec<String> = vec![String::new()];
    for s in stmts {
        if let Stmt::Use { path, .. } = s {
            todo.push(path.clone());
            // (`use math.nn` is math's too, as the registry reads it)
            if let Some(parent) = parent_of(path) {
                todo.push(parent.to_string());
            }
        }
    }
    // a program that imports db carries the drivers written in Crow its
    // connection URLs name, every row of each reached: crow_db calls them
    // from Rust, not the program (docs/db-in-crow-design.md)
    let mut carried: BTreeSet<String> = BTreeSet::new();
    if todo.iter().any(|p| p == "db") {
        let mut urls: Vec<String> = Vec::new();
        stmts.iter().for_each(|s| connect_urls_in_stmt(s, &mut urls));
        for driver in db::drivers_for(&urls) {
            todo.push(driver.to_string());
            carried.insert(driver.to_string());
        }
    }
    // the modules whose functions the program itself may call: what it
    // imports (the core's need no import)
    let imported: BTreeSet<String> = todo.iter().cloned().collect();
    while let Some(path) = todo.pop() {
        if modules.iter().any(|m| m.path == path) {
            continue;
        }
        let Some(source) = crow_source(&path) else { continue };
        let prefix = by_path(&path).map(|m| m.prefix).unwrap_or("_");
        let mod_stmts = parse_source(source).map_err(|e| format!("a bug in Crow: the module {} does not parse: {}", path, e))?;
        let imports: Vec<String> = mod_stmts.iter().filter_map(|m| if let Stmt::Use { path, .. } = m { Some(path.clone()) } else { None }).collect();
        todo.extend(imports.iter().cloned());
        modules.push(Module { path, prefix, imports, stmts: mod_stmts });
    }
    // what the program names: a call by a bare name or a module's, a
    // function handed as a value
    let mut named: BTreeSet<String> = BTreeSet::new();
    stmts.iter().for_each(|s| called_in_stmt(s, &mut named));
    // each module's functions the program reaches: a row's, named by what
    // may call it — the program, where it imports the module, and the
    // functions kept so far of the modules that import it (the core's
    // from anywhere) — an export named the same way, and what either
    // reaches among the module's own; to a fixpoint, since a module's
    // function calls other modules' functions (`read` from io's readers,
    // `fetch` from `read`). A name a module never imports reaches nothing
    // of it: sys's `lines[0]` is its local, not io's reader, and a feed's
    // `put` is the program's parameter
    let mut kept: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    loop {
        let mut changed = false;
        for m in &modules {
            let mut called: BTreeSet<String> = BTreeSet::new();
            if m.path.is_empty() || imported.contains(&m.path) {
                called.extend(named.iter().cloned());
            }
            if carried.contains(&m.path) {
                called.extend(crate::table::ROWS.iter().filter(|r| r.module == m.path && r.is_crow()).map(|r| r.name.to_string()));
            }
            for k in &modules {
                if !(m.path.is_empty() || k.imports.contains(&m.path)) {
                    continue;
                }
                let Some(fns) = kept.get(&k.path) else { continue };
                for s in &k.stmts {
                    if let Stmt::FnDecl { name, body, .. } = s {
                        if fns.contains(name) {
                            body.iter().for_each(|st| called_in_stmt(st, &mut called));
                        }
                    }
                }
            }
            let bodies: BTreeMap<&str, &Vec<Stmt>> = m.stmts.iter().filter_map(|s| if let Stmt::FnDecl { name, body, .. } = s { Some((name.as_str(), body)) } else { None }).collect();
            // the functions of every row of a name called (named by the
            // row's id: `read` and `read_asked` — the row a call runs is
            // decided later, by its arguments), every export called, and
            // what they reach among the module's own
            let mut reach: Vec<String> = crate::table::ROWS
                .iter()
                .filter(|r| r.module == m.path && r.is_crow() && called.contains(r.name))
                .map(|r| crate::table::crow_fn_base(r).to_string())
                .collect();
            reach.extend(exports(&m.path).iter().filter(|e| called.contains(**e)).map(|e| e.to_string()));
            let mut set: BTreeSet<String> = BTreeSet::new();
            while let Some(n) = reach.pop() {
                if !set.insert(n.clone()) {
                    continue;
                }
                if let Some(body) = bodies.get(n.as_str()) {
                    let mut inner = BTreeSet::new();
                    body.iter().for_each(|st| called_in_stmt(st, &mut inner));
                    reach.extend(inner.into_iter().filter(|c| bodies.contains_key(c.as_str())));
                }
            }
            if kept.get(&m.path) != Some(&set) {
                kept.insert(m.path.clone(), set);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // the program's calls of the exports in its scope: a bare name, where
    // one module imported bare (or brought by one, or the core) exports it
    // and the program declares no such function, record or derivation
    // (those are everywhere its own); a name a scope binds is that scope's
    // alone — the top level's at the top level, a function's in its body
    // (`Rewrite`); a module's path or alias and the name, for the module
    // it names
    let mut own: BTreeSet<String> = BTreeSet::new();
    let mut top: HashSet<String> = HashSet::new();
    for st in stmts {
        match st {
            Stmt::FnDecl { name, .. } | Stmt::StructDecl { name, .. } | Stmt::Derive { name, .. } => {
                own.insert(name.clone());
            }
            Stmt::TestBlock { .. } => {}
            other => {
                let mut bound = BTreeSet::new();
                bound_in_stmt(other, &mut bound);
                top.extend(bound);
            }
        }
    }
    let mut bare: BTreeSet<String> = BTreeSet::new();
    bare.insert(String::new());
    for s in stmts {
        if let Stmt::Use { path, alias: None } = s {
            bare.insert(path.clone());
            bare.extend(brought_by(path).into_iter().map(String::from));
        }
    }
    let mut providers: BTreeMap<&str, Vec<&Module>> = BTreeMap::new();
    for m in modules.iter().filter(|m| bare.contains(&m.path)) {
        for e in exports(&m.path) {
            providers.entry(e).or_default().push(m);
        }
    }
    let program_bare: BTreeMap<String, String> = providers
        .iter()
        .filter(|(e, ms)| ms.len() == 1 && !own.contains(**e))
        .map(|(e, ms)| (e.to_string(), format!("{}_{}", ms[0].prefix, e)))
        .collect();
    let exported_as = |m: &Module| -> BTreeMap<String, String> { exports(&m.path).iter().map(|e| (e.to_string(), format!("{}_{}", m.prefix, e))).collect() };
    let program_qualified: Vec<(String, BTreeMap<String, String>)> = stmts
        .iter()
        .filter_map(|s| if let Stmt::Use { path, alias } = s { Some((alias.clone().unwrap_or_else(|| path.clone()), path.clone())) } else { None })
        .filter_map(|(spelling, path)| modules.iter().find(|k| k.path == path).map(|k| (spelling, exported_as(k))))
        .collect();
    let mut out: Vec<Stmt> = stmts.to_vec();
    let program = Rewrite { bare: program_bare, qualified: program_qualified };
    for st in &mut out {
        program.stmt(st, &top);
    }
    for m in &modules {
        let Some(fns) = kept.get(&m.path).filter(|fns| !fns.is_empty()) else { continue };
        CROW_IMPORTS.with(|c| c.borrow_mut().push((m.prefix.to_string(), m.imports.clone())));
        let row_names: BTreeSet<&str> = crate::table::ROWS.iter().filter(|r| r.module == m.path && r.is_crow()).map(|r| r.name).collect();
        let defined: BTreeSet<&str> = m.stmts.iter().filter_map(|s| match s {
            Stmt::FnDecl { name, .. } | Stmt::StructDecl { name, .. } => Some(name.as_str()),
            _ => None,
        }).collect();
        // the module's own: a helper's references are rewritten, an
        // export's, and a row function's whose name is not the row's
        // (`read_asked`); a call of a row's name is the row's call wherever
        // it is made
        let mut names: BTreeMap<String, String> = BTreeMap::new();
        for s in &m.stmts {
            match s {
                Stmt::FnDecl { name, .. } if fns.contains(name) && !row_names.contains(name.as_str()) => {
                    names.insert(name.clone(), format!("{}_{}", m.prefix, name));
                }
                Stmt::StructDecl { name, .. } => {
                    names.insert(name.clone(), format!("{}_{}", m.prefix, name));
                }
                _ => {}
            }
        }
        // what it imports: each one's exports behind that module's prefix,
        // bare where the module defines no function of the name itself,
        // and by the import's path
        let mut qualified: Vec<(String, BTreeMap<String, String>)> = Vec::new();
        for path in &m.imports {
            for p in std::iter::once(path.clone()).chain(brought_by(path).into_iter().map(String::from)) {
                let Some(k) = modules.iter().find(|k| k.path == p) else { continue };
                let theirs = exported_as(k);
                for (e, prefixed) in &theirs {
                    if !defined.contains(e.as_str()) && !row_names.contains(e.as_str()) {
                        names.entry(e.clone()).or_insert_with(|| prefixed.clone());
                    }
                }
                if p == *path {
                    qualified.push((path.clone(), theirs));
                }
            }
        }
        let rewrite = Rewrite { bare: names, qualified };
        for s in &m.stmts {
            match s {
                Stmt::FnDecl { name, params, defaults, body, is_view } if fns.contains(name) => {
                    let mut body = body.clone();
                    // (its parameters and locals shadow the module's names)
                    let mut bound: BTreeSet<String> = params.iter().cloned().collect();
                    body.iter().for_each(|st| bound_in_stmt(st, &mut bound));
                    let shadowed: HashSet<String> = bound.into_iter().collect();
                    for st in &mut body {
                        rewrite.stmt(st, &shadowed);
                    }
                    drop_lines(&mut body);
                    out.push(Stmt::FnDecl { name: format!("{}_{}", m.prefix, name), params: params.clone(), defaults: defaults.clone(), body, is_view: *is_view });
                }
                Stmt::StructDecl { name, fields } => {
                    out.push(Stmt::StructDecl { name: format!("{}_{}", m.prefix, name), fields: fields.clone() });
                }
                // the module's other functions (its tests' helpers, the
                // functions the program never reaches), its imports, its
                // tests, and its line marks
                _ => {}
            }
        }
    }
    Ok(out)
}

/// The names a statement calls, by a bare name, through every nested
/// block and expression.
/// The URLs a program's `connect[…]` calls spell as literals, in its
/// statements (a call whose URL is not a literal counts as one spelled by
/// nothing: `drivers_for` then carries every driver).
fn connect_urls_in_stmt(st: &crate::ast::Stmt, out: &mut Vec<String>) {
    use crate::ast::Stmt;
    match st {
        Stmt::Assign { value, .. } => connect_urls_in_expr(value, out),
        Stmt::MultiAssign { values, .. } => values.iter().for_each(|v| connect_urls_in_expr(v, out)),
        Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => connect_urls_in_expr(e, out),
        Stmt::If { condition, then_body, else_body } => {
            connect_urls_in_expr(condition, out);
            then_body.iter().for_each(|s| connect_urls_in_stmt(s, out));
            if let Some(e) = else_body {
                e.iter().for_each(|s| connect_urls_in_stmt(s, out));
            }
        }
        Stmt::For { iter, body, .. } => {
            connect_urls_in_expr(iter, out);
            body.iter().for_each(|s| connect_urls_in_stmt(s, out));
        }
        Stmt::While { condition, body } => {
            connect_urls_in_expr(condition, out);
            body.iter().for_each(|s| connect_urls_in_stmt(s, out));
        }
        Stmt::FnDecl { body, .. } => body.iter().for_each(|s| connect_urls_in_stmt(s, out)),
        Stmt::Derive { body, .. } => body.iter().for_each(|s| connect_urls_in_stmt(s, out)),
        _ => {}
    }
}

fn connect_urls_in_expr(e: &crate::ast::Expr, out: &mut Vec<String>) {
    use crate::ast::Expr;
    if let Expr::Call { callee, args } = e {
        let is_connect = match callee.as_ref() {
            Expr::Ident(n) => n == "connect",
            Expr::FieldAccess { object, field } => field == "connect" && matches!(object.as_ref(), Expr::Ident(m) if m == "db"),
            _ => false,
        };
        if is_connect {
            match args.first() {
                Some(Expr::Str(url)) if !url.contains('`') => out.push(url.clone()),
                _ => out.push(String::new()),
            }
        }
        args.iter().for_each(|a| connect_urls_in_expr(a, out));
        connect_urls_in_expr(callee, out);
        return;
    }
    match e {
        Expr::BinOp { left, right, .. } => {
            connect_urls_in_expr(left, out);
            connect_urls_in_expr(right, out);
        }
        Expr::UnaryOp { expr, .. } | Expr::Named { value: expr, .. } | Expr::FieldAccess { object: expr, .. } => connect_urls_in_expr(expr, out),
        Expr::Array(items) => items.iter().for_each(|i| connect_urls_in_expr(i, out)),
        Expr::Map(pairs) => pairs.iter().for_each(|(k, v)| {
            connect_urls_in_expr(k, out);
            connect_urls_in_expr(v, out);
        }),
        Expr::ForExpr { iter, body, .. } => {
            connect_urls_in_expr(iter, out);
            body.iter().for_each(|s| connect_urls_in_stmt(s, out));
        }
        _ => {}
    }
}

fn called_in_stmt(st: &crate::ast::Stmt, out: &mut std::collections::BTreeSet<String>) {
    use crate::ast::Stmt;
    match st {
        Stmt::Assign { value, .. } => called_in_expr(value, out),
        Stmt::MultiAssign { values, .. } => values.iter().for_each(|v| called_in_expr(v, out)),
        Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => called_in_expr(e, out),
        Stmt::If { condition, then_body, else_body } => {
            called_in_expr(condition, out);
            then_body.iter().for_each(|s| called_in_stmt(s, out));
            if let Some(e) = else_body {
                e.iter().for_each(|s| called_in_stmt(s, out));
            }
        }
        Stmt::For { iter, body, .. } => {
            called_in_expr(iter, out);
            body.iter().for_each(|s| called_in_stmt(s, out));
        }
        Stmt::While { condition, body } => {
            called_in_expr(condition, out);
            body.iter().for_each(|s| called_in_stmt(s, out));
        }
        Stmt::FieldAssign { object, value, .. } => {
            called_in_expr(object, out);
            called_in_expr(value, out);
        }
        Stmt::IndexAssign { object, index, value } => {
            called_in_expr(object, out);
            called_in_expr(index, out);
            called_in_expr(value, out);
        }
        Stmt::FnDecl { body, .. } | Stmt::Derive { body, .. } | Stmt::TestBlock { body, .. } => body.iter().for_each(|s| called_in_stmt(s, out)),
        _ => {}
    }
}

fn called_in_expr(e: &crate::ast::Expr, out: &mut std::collections::BTreeSet<String>) {
    use crate::ast::Expr;
    match e {
        Expr::Call { callee, args } => {
            match callee.as_ref() {
                Expr::Ident(n) => {
                    out.insert(n.clone());
                }
                // a module's function called by the module's name
                // (`io.csv[…]`, `math.nn.relu[…]`)
                Expr::FieldAccess { object, field } => {
                    out.insert(field.clone());
                    called_in_expr(object, out);
                }
                other => called_in_expr(other, out),
            }
            args.iter().for_each(|a| called_in_expr(a, out));
        }
        // a function handed as a value (`stream[lines, 2]`, `f = relu`)
        Expr::Ident(n) => {
            out.insert(n.clone());
        }
        // a call inside an interpolation (`'`relu[x]`'`), read when the
        // string is lowered: every name between backticks counts
        Expr::Str(s) => {
            let mut inside = false;
            let mut word = String::new();
            for c in s.chars().chain(std::iter::once('`')) {
                if inside && (c.is_alphanumeric() || c == '_') {
                    word.push(c);
                    continue;
                }
                if !word.is_empty() && !word.starts_with(|d: char| d.is_ascii_digit()) {
                    out.insert(std::mem::take(&mut word));
                }
                word.clear();
                if c == '`' {
                    inside = !inside;
                }
            }
        }
        Expr::BinOp { left, right, .. } => {
            called_in_expr(left, out);
            called_in_expr(right, out);
        }
        Expr::UnaryOp { expr, .. } | Expr::Named { value: expr, .. } | Expr::FieldAccess { object: expr, .. } => called_in_expr(expr, out),
        Expr::Array(items) => items.iter().for_each(|i| called_in_expr(i, out)),
        Expr::Map(pairs) => pairs.iter().for_each(|(k, v)| {
            called_in_expr(k, out);
            called_in_expr(v, out);
        }),
        Expr::ForExpr { iter, body, .. } => {
            called_in_expr(iter, out);
            body.iter().for_each(|s| called_in_stmt(s, out));
        }
        _ => {}
    }
}

/// A block without its line marks, through every nested block.
fn drop_lines(body: &mut Vec<crate::ast::Stmt>) {
    use crate::ast::Stmt;
    body.retain(|s| !matches!(s, Stmt::Line(_)));
    for s in body.iter_mut() {
        match s {
            Stmt::If { then_body, else_body, .. } => {
                drop_lines(then_body);
                if let Some(e) = else_body {
                    drop_lines(e);
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } | Stmt::FnDecl { body, .. } | Stmt::Derive { body, .. } | Stmt::TestBlock { body, .. } => drop_lines(body),
            _ => {}
        }
    }
}

/// Where a module's printed code stops the program, the line it stops on:
/// the emitter puts the call's line in its place, so a module's refusal
/// names its line as the interpreter's does (`crow_stop(__crow_line__, e)`).
pub const STOP_LINE: &str = "__crow_line__";

/// Trait that every Crow module implements.
/// Each module is self-contained: it declares its functions, constants,
/// and knows how to interpret calls, emit Rust code, and infer types.
pub trait CrowModule: Send + Sync {
    /// Module name (e.g. "math", "math.stats")
    fn name(&self) -> &str;

    /// List of function names this module provides.
    fn functions(&self) -> &[&str];

    /// Constants this module provides (e.g. pi, e, inf, nan).
    /// Returns (name, value) pairs.
    fn constants(&self) -> Vec<(&str, CrowValue)> {
        vec![]
    }

    // A function with a row in the builtin table (crate::table) is answered
    // by its row, for the interpreter, the emitter and inference alike; a
    // module's own arms answer the rest, and a module whose functions are
    // all rows has none.

    /// Interpret a function call. Called by the interpreter.
    fn call(&self, name: &str, _args: &[CrowValue]) -> Result<CrowValue, String> {
        Err(format!("module '{}' has no function '{}'", self.name(), name))
    }

    /// Emit Rust code for a function call. Called by the Rust backend.
    /// `arg_strs` are the argument expressions, already printed.
    /// `arg_types` are the inferred types of each argument.
    fn emit_rust(&self, name: &str, _arg_strs: &[String], _arg_types: &[Ty]) -> Result<String, String> {
        Err(format!("module '{}' has no function '{}'", self.name(), name))
    }

    /// Infer the return type of a function call. Called by the type inference engine.
    fn type_of(&self, _name: &str, _arg_count: usize) -> Ty {
        Ty::Unknown
    }
}

/// Registry of loaded modules. Provides generic dispatch.
#[derive(Clone)]
pub struct ModuleRegistry {
    modules: Vec<Arc<dyn CrowModule>>,
    /// Maps function name → index into `modules` vec.
    /// Built when modules are loaded.
    fn_map: HashMap<String, Vec<usize>>,
    /// Maps constant name → index into `modules` vec.
    const_map: HashMap<String, usize>,
    /// Which modules each scope sees: the program's (`PROGRAM`, what its
    /// `use` lines import) and each standard module's written in Crow (its
    /// prefix, what its own `use` lines import — `sys`, which a program
    /// never sees). A name is resolved in the scope of the function it is
    /// read in (`enter`), so a program's imports never reach into a
    /// module's formulas, and a module's layer never reaches the program.
    scopes: HashMap<String, Vec<usize>>,
}

/// The program's own scope.
pub const PROGRAM: &str = "program";

thread_local! {
    /// The scope names are resolved in on this thread (a registry is shared
    /// by the threads a served program runs on; each resolves in the scope
    /// of the function it is running).
    static CURRENT_SCOPE: std::cell::Cell<&'static str> = const { std::cell::Cell::new(PROGRAM) };
}

impl std::fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.modules.iter().map(|m| m.name()).collect();
        f.debug_struct("ModuleRegistry")
            .field("modules", &names)
            .finish()
    }
}

impl ModuleRegistry {
    pub fn new() -> Self {
        ModuleRegistry {
            modules: Vec::new(),
            fn_map: HashMap::new(),
            const_map: HashMap::new(),
            scopes: HashMap::new(),
        }
    }

    /// Load a module into the program's scope, registering all its
    /// functions and constants.
    pub fn load(&mut self, module: Arc<dyn CrowModule>) {
        self.load_for(PROGRAM, module);
    }

    /// Load a module into a scope: a module already loaded (by another
    /// scope) is shared, and becomes visible in this one.
    pub fn load_for(&mut self, scope: &str, module: Arc<dyn CrowModule>) {
        let idx = match self.modules.iter().position(|m| m.name() == module.name()) {
            Some(i) => i,
            None => {
                let idx = self.modules.len();
                for &fname in module.functions() {
                    self.fn_map.entry(fname.to_string()).or_default().push(idx);
                }
                for (cname, _) in module.constants() {
                    self.const_map.entry(cname.to_string()).or_insert(idx);
                }
                self.modules.push(module);
                idx
            }
        };
        let seen = self.scopes.entry(scope.to_string()).or_default();
        if !seen.contains(&idx) {
            seen.push(idx);
        }
    }

    /// Resolve names in the scope of a function from here on: a standard
    /// module's written in Crow sees its own imports, any other function
    /// the program's.
    pub fn enter(&self, fn_name: &str) -> &'static str {
        CURRENT_SCOPE.with(|c| c.replace(scope_of_call(fn_name)))
    }

    /// Back to a scope `enter` answered.
    pub fn resume(&self, scope: &'static str) {
        CURRENT_SCOPE.with(|c| c.set(scope));
    }

    /// `enter` for a call: the scope swapped only when the function's
    /// differs from the current one; `None` when unchanged.
    pub fn enter_call(&self, fn_name: &str) -> Option<&'static str> {
        self.enter_scope(scope_of_call(fn_name))
    }

    /// Into a scope `scope_of_call` named (a prepared function keeps its
    /// own), answering the one it left if it left one. A scope is one of
    /// the static names (`PROGRAM`, a module's prefix): the current one is
    /// a static name too, so a call compares two of them, and a call into
    /// a module's function and back had built and dropped two strings.
    pub fn enter_scope(&self, scope: &'static str) -> Option<&'static str> {
        CURRENT_SCOPE.with(|c| {
            let current = c.get();
            if std::ptr::eq(current, scope) || current == scope {
                None
            } else {
                c.set(scope);
                Some(current)
            }
        })
    }

    /// Back to the scope `enter_call` left, if it left one.
    pub fn resume_call(&self, saved: Option<&'static str>) {
        if let Some(scope) = saved {
            CURRENT_SCOPE.with(|c| c.set(scope));
        }
    }

    /// The program's scope, where a pass over the program begins: a pass
    /// before it may have left a module's (the last function it typed).
    pub fn reset_scope() {
        CURRENT_SCOPE.with(|c| c.set(PROGRAM));
    }

    /// Whether the module at `idx` is visible in the current scope.
    fn visible(&self, idx: usize) -> bool {
        CURRENT_SCOPE.with(|c| self.scopes.get(c.get()).is_some_and(|v| v.contains(&idx)))
    }

    /// The modules providing a name, in the current scope.
    fn providers(&self, name: &str) -> Vec<usize> {
        self.fn_map.get(name).map(|idxs| idxs.iter().copied().filter(|&i| self.visible(i)).collect()).unwrap_or_default()
    }

    /// A registry with every stdlib module loaded — for tooling (checker,
    /// LSP) that needs the full export vocabulary without a `use` context.
    pub fn with_stdlib() -> Self {
        let mut r = Self::new();
        for m in MODULES.iter().filter(|m| m.known) {
            if let Some(make) = m.module {
                r.load(make());
            }
        }
        r
    }

    /// Every function name provided by any loaded module.
    pub fn all_function_names(&self) -> Vec<&str> {
        self.modules
            .iter()
            .enumerate()
            .filter(|(i, _)| self.visible(*i))
            .flat_map(|(_, m)| m.functions().iter().copied())
            .collect()
    }

    /// Check if a function name belongs to any loaded module.
    pub fn has_function(&self, name: &str) -> bool {
        !self.providers(name).is_empty()
    }

    /// All loaded modules providing this name — >1 means a bare call is
    /// ambiguous (spec: Conflict Resolution).
    pub fn providers_of(&self, name: &str) -> Vec<&str> {
        self.providers(name).into_iter().map(|i| self.modules[i].name()).collect()
    }

    /// The functions a loaded module provides, by its name (`math.stats`).
    pub fn functions_of(&self, module: &str) -> Vec<&str> {
        self.modules.iter().filter(|m| m.name() == module).flat_map(|m| m.functions().iter().copied()).collect()
    }

    /// Which loaded module provides this function (for reservation messages).
    /// Returns the first provider when several do.
    pub fn module_name_of(&self, name: &str) -> Option<&str> {
        let idx = self.providers(name).into_iter().next()?;
        Some(self.modules[idx].name())
    }

    /// Resolve a function within one specific module (prefixed calls).
    fn idx_in(&self, module_name: &str, fname: &str) -> Option<usize> {
        self.providers(fname).into_iter().find(|&i| self.modules[i].name() == module_name)
    }

    /// Call a module function through its module prefix.
    pub fn call_in(&self, module_name: &str, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        let idx = self.idx_in(module_name, name).ok_or_else(|| {
            format!("module '{}' has no function '{}'", module_name, name)
        })?;
        self.call_at(idx, name, args)
    }

    /// Emit Rust for a function through its module prefix.
    pub fn emit_rust_in(&self, module_name: &str, name: &str, arg_strs: &[String], arg_types: &[Ty]) -> Result<String, String> {
        let idx = self.idx_in(module_name, name).ok_or_else(|| {
            format!("module '{}' has no function '{}'", module_name, name)
        })?;
        self.emit_at(idx, name, arg_strs, arg_types)
    }

    /// Infer return type through a module prefix, from the arguments' types.
    pub fn type_of_in(&self, module_name: &str, name: &str, arg_types: &[Ty]) -> Option<Ty> {
        let idx = self.idx_in(module_name, name)?;
        Some(self.type_at(idx, name, arg_types))
    }

    // A module function with a row in the builtin table is its runtime
    // function, for the interpreter, the emitter and inference alike
    // (crate::table); the module's own arms answer the rest.

    fn call_at(&self, idx: usize, name: &str, args: &[CrowValue]) -> Result<CrowValue, String> {
        if let Some(e) = crate::table::miscounted(self.modules[idx].name(), name, args.len()) {
            return Err(e);
        }
        let kinds: Vec<crate::table::Arg> = args.iter().map(crate::table::Arg::of_value).collect();
        match crate::table::module(self.modules[idx].name(), name, &kinds) {
            // a module's error value carries no trace, in either mode
            Some(row) => (row.call)(args, ""),
            None => self.modules[idx].call(name, args),
        }
    }

    fn emit_at(&self, idx: usize, name: &str, arg_strs: &[String], arg_types: &[Ty]) -> Result<String, String> {
        if let Some(e) = crate::table::miscounted(self.modules[idx].name(), name, arg_strs.len()) {
            return Err(e);
        }
        let kinds: Vec<crate::table::Arg> = if arg_types.len() == arg_strs.len() {
            arg_types.iter().map(crate::table::Arg::of_type).collect()
        } else {
            vec![crate::table::Arg::Unknown; arg_strs.len()]
        };
        match crate::table::module(self.modules[idx].name(), name, &kinds) {
            Some(row) => crate::table::print(row, arg_strs, STOP_LINE, None),
            None => self.modules[idx].emit_rust(name, arg_strs, arg_types),
        }
    }

    fn type_at(&self, idx: usize, name: &str, arg_types: &[Ty]) -> Ty {
        let kinds: Vec<crate::table::Arg> = arg_types.iter().map(crate::table::Arg::of_type).collect();
        let module = self.modules[idx].name();
        // an argument whose kind is not known yet picks the row as the
        // program runs: the call's type is what every row of the name
        // answers, or not known — as a core builtin's is (the first row's
        // answer had been taken: `exp` of a parameter no call had typed was
        // a number, and stuck as the function's return)
        if kinds.iter().any(|k| matches!(k, crate::table::Arg::Unknown | crate::table::Arg::AnyList)) {
            // (the core's rows of the name too, where it has them: `slice`
            // of text or of bytes is picked as the program runs)
            let answers: Vec<Ty> = crate::table::ROWS
                .iter()
                .filter(|r| (r.module == module || r.module.is_empty()) && r.name == name && crate::table::kinds_of(r.params, arg_types.len()).is_some())
                .map(|r| r.ty())
                .collect();
            if answers.iter().any(|t| *t != answers[0]) {
                return Ty::Unknown;
            }
        }
        match crate::table::module(module, name, &kinds) {
            Some(row) => crate::table::exact_form(row, &kinds).ty_given(arg_types),
            None => self.modules[idx].type_of(name, arg_types.len()),
        }
    }

    /// Check if a constant name belongs to any loaded module.
    pub fn has_constant(&self, name: &str) -> bool {
        self.const_map.get(name).is_some_and(|&i| self.visible(i))
    }

    /// Get the constant value for a name.
    pub fn get_constant(&self, name: &str) -> Option<CrowValue> {
        let idx = self.const_map.get(name).filter(|&&i| self.visible(i))?;
        let module = &self.modules[*idx];
        module.constants().into_iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    /// Bare-call ambiguity message, or None when the name is unique.
    fn ambiguity(&self, name: &str) -> Option<String> {
        let providers = self.providers_of(name);
        if providers.len() > 1 {
            let forms: Vec<String> = providers.iter().map(|m| format!("{}.{}[...]", m, name)).collect();
            Some(format!(
                "'{}' is ambiguous — provided by {}; use {}",
                name,
                providers.join(" and "),
                forms.join(" or ")
            ))
        } else {
            None
        }
    }


    /// Emit Rust for a module function (the Rust backend, bare call).
    pub fn emit_rust(&self, name: &str, arg_strs: &[String], arg_types: &[Ty]) -> Result<String, String> {
        if let Some(msg) = self.ambiguity(name) {
            return Err(msg);
        }
        let idx = self.providers(name).into_iter().next()
            .ok_or_else(|| format!("unknown module function: {}", name))?;
        self.emit_at(idx, name, arg_strs, arg_types)
    }

    /// Infer return type of a module function (bare call; None when
    /// ambiguous), from the arguments' types.
    pub fn type_of(&self, name: &str, arg_types: &[Ty]) -> Option<Ty> {
        let idxs = self.providers(name);
        if idxs.len() != 1 {
            return None;
        }
        Some(self.type_at(idxs[0], name, arg_types))
    }


    /// Check if a specific module (by name) has been loaded.
    pub fn is_loaded(&self, module_name: &str) -> bool {
        self.modules.iter().enumerate().any(|(i, m)| m.name() == module_name && self.visible(i))
    }

    /// All registered function names, in the current scope.
    pub fn fn_names(&self) -> Vec<&str> {
        self.fn_map.iter().filter(|(_, idxs)| idxs.iter().any(|&i| self.visible(i))).map(|(s, _)| s.as_str()).collect()
    }

    /// All registered constant names, in the current scope.
    pub fn const_names(&self) -> Vec<&str> {
        self.const_map.iter().filter(|(_, i)| self.visible(**i)).map(|(s, _)| s.as_str()).collect()
    }

}

#[cfg(test)]
mod tests {
    /// A module written in Crow declares, in its source, every function
    /// its rows name, each named behind the module's prefix; the prefix is
    /// the path's own.
    #[test]
    fn a_crow_written_modules_rows_are_its_sources_functions() {
        for m in super::MODULES.iter().filter(|m| !m.path.is_empty()) {
            assert_eq!(m.prefix, format!("_{}", m.path.replace('.', "_")), "{}'s prefix", m.path);
            let Some(source) = m.source else { continue };
            let declared: Vec<&str> = source.lines().filter_map(|l| l.strip_prefix("fn ")).map(|l| l.split('[').next().unwrap_or("")).collect();
            for row in crate::table::ROWS.iter().filter(|r| r.module == m.path && r.is_crow()) {
                // (the function is named by the row's id, which tells apart
                // the rows of one name: `load_asked` beside `load`)
                let function = row.id.rsplit('.').next().unwrap_or(row.id);
                assert!(declared.contains(&function), "{} declares {}", m.path, function);
                assert_eq!(crate::table::crow_fn_name(row), format!("{}_{}", m.prefix, function));
            }
        }
    }

    use super::*;
    use crate::ast::Stmt;

    fn names_called(body: &[Stmt]) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        body.iter().for_each(|s| called_in_stmt(s, &mut out));
        out
    }

    fn has_line(body: &[Stmt]) -> bool {
        body.iter().any(|s| match s {
            Stmt::Line(_) => true,
            Stmt::If { then_body, else_body, .. } => has_line(then_body) || else_body.as_ref().is_some_and(|e| has_line(e)),
            Stmt::For { body, .. } | Stmt::While { body, .. } => has_line(body),
            _ => false,
        })
    }

    /// A module's exports are the functions and records its source
    /// declares with no leading underscore, apart from those a row still
    /// names; a helper is never one, and the core exports nothing (its
    /// builtins written in Crow keep their rows for now).
    #[test]
    fn a_modules_exports_are_its_sources_functions_without_an_underscore() {
        for (path, _) in crow_modules() {
            let source = crow_source(path).expect("source");
            let declared: Vec<&str> = source.lines().filter_map(|l| l.strip_prefix("fn ")).map(|l| l.split('[').next().unwrap_or("")).collect();
            for e in exports(path) {
                assert!(declared.contains(e), "{} declares {}", path, e);
                assert!(!e.starts_with('_'), "{}'s {} is a helper", path, e);
                assert!(!crate::table::ROWS.iter().any(|r| r.module == path && r.is_crow() && (r.name == *e || crate::table::crow_fn_base(r) == *e)), "{}'s {} is a row's", path, e);
            }
        }
        assert!(exports("").is_empty(), "{:?}", exports(""));
        let llm = exports("llm");
        for f in ["reach", "ask", "asked", "said", "told", "chat", "follow", "next", "reply", "embed"] {
            assert!(llm.contains(&f), "llm exports {}: {:?}", f, llm);
        }
        assert!(!llm.contains(&"_talk_of") && !llm.contains(&"talk_of"), "{:?}", llm);
        assert!(exports("math.stats").contains(&"median") && !exports("math.stats").contains(&"anova"), "{:?}", exports("math.stats"));
        assert!(exports("web.ws").contains(&"socket") && exports("web.ws").contains(&"close") && !exports("web.ws").contains(&"send"), "{:?}", exports("web.ws"));
        // a message names the function as the program writes it
        assert_eq!(unprefixed("'_llm_chat' has no parameter named 'max' (in _math_stats_median, _core_read_asked)"), "'chat' has no parameter named 'max' (in median, read_asked)");
        assert_eq!(unprefixed("_mathx_relu and x_llm_chat stay"), "_mathx_relu and x_llm_chat stay");
    }

    /// `use math.nn` inlines the exported functions the program calls and
    /// the helpers they reach, renamed behind the prefix, their line marks
    /// dropped; the program's calls name the inlined functions; a
    /// test-only helper is left out, and a program without the import gets
    /// nothing.
    #[test]
    fn a_crow_module_is_inlined_as_its_rows_reach_it() {
        let stmts = crate::build("use math, math.nn\nlog[relu[[1.0]]]\nlog[crossentropy[[[1.0, 0.0]], [0.0]]]\n").expect("parses");
        let out = inline_crow_modules(&stmts).expect("inlines");
        let fns: std::collections::BTreeMap<String, Vec<Stmt>> = out.iter().filter_map(|s| if let Stmt::FnDecl { name, body, .. } = s { Some((name.clone(), body.clone())) } else { None }).collect();
        for f in ["relu", "crossentropy"] {
            assert!(fns.contains_key(&format!("_math_nn_{}", f)), "{} is inlined: {:?}", f, fns.keys());
        }
        for f in ["linear", "mse", "gelu"] {
            assert!(!fns.contains_key(&format!("_math_nn_{}", f)), "a row the program never reaches stays out: {:?}", fns.keys());
        }
        assert!(!fns.keys().any(|n| n.contains("through_all")), "a test's helper is not inlined: {:?}", fns.keys());
        for (n, body) in &fns {
            assert!(!has_line(body), "{} keeps no line mark", n);
        }
        let ce = names_called(&fns["_math_nn_crossentropy"]);
        assert!(ce.contains("onehot") && ce.contains("halt") && !ce.iter().any(|c| c.starts_with("_math_nn_")), "{:?}", ce);
        assert!(is_crow_fn("_math_nn_relu") && is_crow_fn("_traced__math_nn_relu") && !is_crow_fn("relu") && !is_crow_fn("_mathx_relu"));
        let plain = crate::build("use math\nlog[1]\n").expect("parses");
        assert_eq!(inline_crow_modules(&plain).expect("inlines").len(), plain.len());
        // a row reached by the module's name, handed as a value, or called
        // inside an interpolation
        let names = |src: &str| -> Vec<String> {
            let stmts = crate::build(src).expect("parses");
            inline_crow_modules(&stmts).expect("inlines").iter().filter_map(|s| if let Stmt::FnDecl { name, .. } = s { Some(name.clone()) } else { None }).collect()
        };
        let qualified = names("use math, math.nn\nlog[math.nn.sigmoid[[1.0]]]\n");
        assert!(qualified.contains(&"_math_nn_sigmoid".to_string()) && !qualified.contains(&"_math_nn_relu".to_string()), "{:?}", qualified);
        let handed = names("use math, math.nn\nf = gelu\nlog[f[[1.0]]]\n");
        assert!(handed.contains(&"_math_nn_gelu".to_string()), "{:?}", handed);
        let interpolated = names("use math, math.nn\nlog['`mse[[1.0], [2.0]]`']\n");
        assert!(interpolated.contains(&"_math_nn_mse".to_string()) && !interpolated.contains(&"_math_nn_relu".to_string()), "{:?}", interpolated);
    }

    /// A module's own imports are inlined after it (io's readers are
    /// written over sys's protocol), and its call of a core row written in
    /// Crow inlines the core — each module once, behind its own prefix,
    /// the row's function named by the row's id (`_core_read_asked`), and
    /// only what the program reaches: a reader it never calls stays out,
    /// as does every row of a module it never names.
    #[test]
    fn a_module_reaches_what_it_calls() {
        let names = |src: &str| -> Vec<String> {
            let stmts = crate::build(src).expect("parses");
            inline_crow_modules(&stmts).expect("inlines").iter().filter_map(|s| if let Stmt::FnDecl { name, .. } = s { Some(name.clone()) } else { None }).collect()
        };
        let fns = names("use io\nlog[csv['t.csv']]\n");
        // (every row of a name called: the one a call runs is decided
        // later, by its arguments — `csv` with the options is `csv_asked`)
        for f in ["_io_csv", "_io_csv_asked", "_core_read", "_core_read_asked", "_core__as_text", "_sys_remote", "_sys_fetch", "_sys_exchange", "_sys__read_answer"] {
            assert!(fns.contains(&f.to_string()), "{} is inlined: {:?}", f, fns);
        }
        for f in ["_io_lines", "_io_lines_asked", "_io__lines_of", "_core_write", "_core_request", "_math_load", "_frame_load"] {
            assert!(!fns.contains(&f.to_string()), "{} is never reached: {:?}", f, fns);
        }
        let mut seen = std::collections::BTreeSet::new();
        assert!(fns.iter().all(|f| seen.insert(f.clone())), "each function once: {:?}", fns);
        let imports = crow_imports();
        assert!(imports.iter().any(|(p, i)| p == "_io" && i == &["sys".to_string()]), "{:?}", imports);
        assert!(imports.iter().any(|(p, i)| p == "_core" && i == &["sys".to_string()]), "{:?}", imports);
        assert!(!imports.iter().any(|(p, _)| p == "_frame" || p == "_math"), "a module nothing reaches loads no scope: {:?}", imports);
        assert_eq!(crow_scope_of("_core_read_asked").as_deref(), Some("_core"));
        assert_eq!(crow_scope_of("_io__lines_of").as_deref(), Some("_io"));
        assert_eq!(crow_scope_of("_lines_of"), None);
        // the options' row, by the module's name: both rows of the name
        let asked = names("use io\nlog[io.lines['t', timeout = 1.0]]\n");
        for f in ["_io_lines", "_io_lines_asked", "_io__lines_of", "_core_read_asked"] {
            assert!(asked.contains(&f.to_string()), "{} is inlined: {:?}", f, asked);
        }
        assert!(!asked.contains(&"_io_csv".to_string()), "{:?}", asked);
        // the core alone, for a program that reads a file
        let core = names("log[read['t.txt']]\n");
        assert!(core.contains(&"_core_read".to_string()) && !core.iter().any(|f| f.starts_with("_io_")), "{:?}", core);
        // a name of the program's that a module it never imports has a
        // row for reaches nothing: a feed's `put` is its parameter
        let feed = names("use web, frame\nfn lines[put, n]\n    put['x']\ns = stream[lines, 2]\n");
        assert_eq!(feed, vec!["lines".to_string()], "nothing is inlined: {:?}", feed);
    }

    /// The modules a program carries are read from its functions, with
    /// each one's own imports — whatever was inlined last on the thread
    /// (a harness lowers a part's tests and the app in turn).
    #[test]
    fn the_modules_carried_are_read_from_the_program() {
        let carried = crow_modules_carried(["_core_read", "_core__as_text", "_sys_fetch", "main", "_io_csv"].into_iter());
        assert_eq!(carried, vec![("_core".to_string(), vec!["sys".to_string()]), ("_io".to_string(), vec!["sys".to_string()]), ("_sys".to_string(), vec!["sys".to_string()])]);
        assert!(crow_modules_carried(["main", "helper"].into_iter()).is_empty());
        let frame = crow_modules_carried(["_frame_load"].into_iter());
        assert_eq!(frame, vec![("_frame".to_string(), vec!["sys".to_string(), "frame".to_string()])]);
    }

    /// A registry resolves a name in the scope of the function it is read
    /// in: the program's imports, or a standard module's own (`sys`, which
    /// the program never sees); the core's Crow source is inlined where a
    /// program calls one of its rows.
    #[test]
    fn a_name_is_resolved_in_its_functions_scope() {
        let mut r = ModuleRegistry::new();
        r.load(module_by_path("math").expect("math"));
        r.load_for("_core", module_by_path("sys").expect("sys"));
        assert!(r.has_function("exp") && !r.has_function("open"));
        let saved = r.enter("_core_request");
        assert!(r.has_function("open") && !r.has_function("exp"), "a module's scope is its own imports");
        assert_eq!(r.providers_of("slice"), vec!["sys"]);
        r.resume(saved);
        assert!(r.has_function("exp") && !r.has_function("open"));
        let stmts = crate::build("r = request['http://x/y']\nlog[r.status]\n").expect("parses");
        let out = inline_crow_modules(&stmts).expect("inlines");
        assert!(out.iter().any(|s| matches!(s, Stmt::FnDecl { name, .. } if name == "_core_request")), "the core's source is inlined for a call of request");
        assert!(crow_imports().iter().any(|(p, i)| p == "_core" && i.iter().any(|m| m == "sys")));
        let none = crate::build("log[1]\n").expect("parses");
        assert!(!inline_crow_modules(&none).expect("inlines").iter().any(|s| matches!(s, Stmt::FnDecl { .. })));
    }
}
