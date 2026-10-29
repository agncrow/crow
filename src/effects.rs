//! Effects: what a builtin reaches outside the program's values, by kind,
//! and what each context may reach — the language's rules, stated once
//! (design rule 24; docs/extending-crow.md, section 6). A native row
//! declares its kinds (`table::Prop::Does`); a function written in Crow —
//! a program's or a standard module's — reaches the union of what its body
//! calls, which the call graph computes (`ir::Fn::effects`, by the names
//! called); nothing is written in a program. Each rule below had been a
//! list of names in the compiler, and the lists disagreed with each other.

/// A kind of effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Effect {
    /// Writes to the console. Output has an order, so a loop over it keeps
    /// its order; a view or a derivation may write it (the spec, Loops).
    Console,
    /// Reads a file, a directory, or a table from a path.
    Files,
    /// Writes, makes, moves or removes a file or a directory.
    FilesWrite,
    /// Sends or receives over the network: a request, a socket, a page's
    /// data source.
    Network,
    /// Reads a database, or opens a connection to one.
    DbRead,
    /// Writes a database: a statement, an insert, a transaction, a
    /// migration.
    DbWrite,
    /// Sends mail.
    Mail,
    /// Reads the clock.
    Clock,
    /// Draws from the seeded random generator.
    Random,
    /// Reads what the reader types.
    Input,
    /// Reads the process's arguments or environment.
    Process,
    /// Ends the process.
    Exit,
    /// Runs a command.
    Shell,
    /// Acts for the session: a notice, a sign-in or sign-out, a download,
    /// work scheduled for after the commit.
    Session,
    /// Waits: on the reader, on a socket, on the clock, on a command.
    Waits,
    /// Declares what the server serves: a route, a page, the server itself.
    Serving,
    /// Reads what the process holds under a name (`recall`): the program's
    /// own state, shared by its threads.
    Memory,
    /// Writes what the process holds under a name (`remember`, `revise`,
    /// `forget`).
    MemoryWrite,
}

impl Effect {
    pub const ALL: [Effect; 18] = [
        Effect::Console,
        Effect::Files,
        Effect::FilesWrite,
        Effect::Network,
        Effect::DbRead,
        Effect::DbWrite,
        Effect::Mail,
        Effect::Clock,
        Effect::Random,
        Effect::Input,
        Effect::Process,
        Effect::Exit,
        Effect::Shell,
        Effect::Session,
        Effect::Waits,
        Effect::Serving,
        Effect::Memory,
        Effect::MemoryWrite,
    ];

    /// The kind's word, as `crow check`'s reach and the spec name it.
    pub fn word(self) -> &'static str {
        match self {
            Effect::Console => "console",
            Effect::Files => "files",
            Effect::FilesWrite => "files.write",
            Effect::Network => "network",
            Effect::DbRead => "db.read",
            Effect::DbWrite => "db.write",
            Effect::Mail => "mail",
            Effect::Clock => "clock",
            Effect::Random => "random",
            Effect::Input => "input",
            Effect::Process => "process",
            Effect::Exit => "exit",
            Effect::Shell => "shell",
            Effect::Session => "session",
            Effect::Waits => "waits",
            Effect::Serving => "serving",
            Effect::Memory => "held",
            Effect::MemoryWrite => "held.write",
        }
    }

    /// What a builtin of the kind does, for a refusal: `write[] writes a
    /// file`.
    pub fn does(self) -> &'static str {
        match self {
            Effect::Console => "writes to the console",
            Effect::Files => "reads a file",
            Effect::FilesWrite => "writes a file",
            Effect::Network => "reaches the network",
            Effect::DbRead => "reads a database",
            Effect::DbWrite => "writes a database",
            Effect::Mail => "sends a message",
            Effect::Clock => "reads the clock",
            Effect::Random => "draws a random number",
            Effect::Input => "waits for the reader to type",
            Effect::Process => "reads the process's arguments or environment",
            Effect::Exit => "ends the process",
            Effect::Shell => "runs a command",
            Effect::Session => "acts for the session",
            Effect::Waits => "waits",
            Effect::Serving => "declares what the server serves",
            Effect::Memory => "reads what the process holds",
            Effect::MemoryWrite => "changes what the process holds",
        }
    }

    /// Reaches outside the program's values, so the session layer performs
    /// it (`Expr::Effect`): every kind but a draw, since the seeded
    /// generator is the program's own — a draw orders a loop and makes a
    /// function impure to `crow check`, and nothing else.
    pub fn outside(self) -> bool {
        // (and process memory, the program's own state shared by its
        // threads: nothing leaves the program)
        !matches!(self, Effect::Random | Effect::Memory | Effect::MemoryWrite)
    }

    /// Held to the handler's commit: it runs when the handler ends, in
    /// program order, a stop drops it, and a failed argument stops the
    /// handler — an error never reaches the world outside the session
    /// (spec, A handler is a transaction).
    pub fn held(self) -> bool {
        matches!(self, Effect::Session | Effect::DbWrite | Effect::Mail)
    }

    /// Its answer carries the moment it was read (spec, Capture).
    pub fn stamps(self) -> bool {
        matches!(self, Effect::Files | Effect::Network | Effect::DbRead | Effect::Input | Effect::Process | Effect::Shell)
    }

    /// A rehearsal records it, with its arguments, instead of running it:
    /// nothing leaves the machine (a request runs through its own
    /// fixtures; a database write is the rehearsal database's).
    pub fn recorded(self) -> bool {
        matches!(self, Effect::FilesWrite | Effect::Shell)
    }

    /// A function calling it acts for the session, and joins the handler
    /// class (type_infer/classify.rs).
    pub fn acts_for_session(self) -> bool {
        matches!(self, Effect::Session | Effect::Mail)
    }
}

/// Where a call stands: the contexts the language tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// A view: a pure function of the page's state to a tree.
    View,
    /// A derivation: a value computed from what it reads, kept while its
    /// inputs stand.
    Derive,
    /// A handler, or a helper a handler calls: a transaction.
    Handler,
    /// Any other function, and the top level.
    Function,
    /// A test block.
    Test,
}

impl Context {
    /// Whether a call of the kind may stand in the context (the one table:
    /// a view reaches nothing but the console; a derivation reads — files,
    /// the network, a database, the clock — and changes nothing, since it
    /// runs whenever the page needs it; a handler, a function and a test
    /// may do anything, a test's effects replayed from fixtures where they
    /// were recorded). A view may read the clock: a live program has one
    /// moment per event, the render's included (spec, One moment per
    /// event). A draw never asks: the generator is the program's own
    /// (`outside`), so a view or a derivation may draw.
    pub fn allows(self, e: Effect) -> bool {
        match self {
            Context::View => matches!(e, Effect::Console | Effect::Clock | Effect::Random),
            Context::Derive => matches!(e, Effect::Console | Effect::Files | Effect::Network | Effect::DbRead | Effect::Clock | Effect::Random | Effect::Process | Effect::Memory),
            Context::Handler | Context::Function | Context::Test => true,
        }
    }

    /// What the context is, in a refusal.
    pub fn word(self) -> &'static str {
        match self {
            Context::View => "a view",
            Context::Derive => "a derivation",
            Context::Handler => "a handler",
            Context::Function => "a function",
            Context::Test => "a test",
        }
    }

    /// The kind a refusal of a call speaks of, among the call's kinds: one
    /// the context cannot hold, and of those first one a derivation cannot
    /// hold either — a call that reads and waits (`recv`) is a handler's,
    /// whatever else it does. None when the context holds every kind.
    pub fn refused(self, kinds: &[Effect]) -> Option<Effect> {
        let refused: Vec<Effect> = kinds.iter().copied().filter(|e| !self.allows(*e)).collect();
        refused.iter().copied().find(|e| !Context::Derive.allows(*e)).or_else(|| refused.first().copied())
    }

    /// Where a call the context refuses belongs, by its kind: a read a
    /// derivation may make is a derivation's, which the view reads (the
    /// book's `derive rows`, above the view); a write, or any other kind, a
    /// handler's.
    pub fn belongs(self, e: Effect) -> &'static str {
        if self == Context::View && Context::Derive.allows(e) {
            "read it in a derivation the view reads — `derive x` above the view, returning it"
        } else {
            "call it from a handler, or a function a handler calls"
        }
    }

    /// Why the context refuses the call, worded once for every kind: what
    /// the builtin does, what the context is for, and where the call
    /// belongs. `via` names the function the context reaches it through,
    /// where the call is not its own.
    pub fn refusal(self, name: &str, e: Effect, via: Option<&str>) -> String {
        let call = match via {
            Some(f) => format!("calls {}[], which reaches {}[]: it {}", f, name, e.does()),
            None => format!("{}[] {}", name, e.does()),
        };
        let rule = match self {
            Context::View => "a view shows the page's state and does nothing else, since it runs whenever the page needs it (it may write to the console, and read the clock)",
            Context::Derive => "a derivation reads — files, the network, a database, the clock, the process's environment or memory — and changes nothing, since it runs whenever the page needs it",
            _ => "",
        };
        format!("{} — {}: {}", call, rule, self.belongs(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_word_and_a_phrase() {
        for e in Effect::ALL {
            assert!(!e.word().is_empty() && !e.does().is_empty(), "{:?}", e);
        }
        let words: std::collections::BTreeSet<&str> = Effect::ALL.iter().map(|e| e.word()).collect();
        assert_eq!(words.len(), Effect::ALL.len(), "two kinds share a word");
    }

    #[test]
    fn a_view_reaches_only_the_console_and_a_derivation_only_reads() {
        for e in Effect::ALL {
            assert_eq!(Context::View.allows(e), matches!(e, Effect::Console | Effect::Clock | Effect::Random), "{:?}", e);
            let reads = matches!(e, Effect::Console | Effect::Files | Effect::Network | Effect::DbRead | Effect::Clock | Effect::Random | Effect::Process | Effect::Memory);
            assert_eq!(Context::Derive.allows(e), reads, "{:?}", e);
            assert!(Context::Handler.allows(e) && Context::Function.allows(e) && Context::Test.allows(e));
        }
    }

    #[test]
    fn what_is_held_is_what_acts_past_the_session_or_the_database() {
        let held: Vec<Effect> = Effect::ALL.into_iter().filter(|e| e.held()).collect();
        assert_eq!(held, vec![Effect::DbWrite, Effect::Mail, Effect::Session]);
        assert!(Effect::Session.acts_for_session() && Effect::Mail.acts_for_session() && !Effect::DbWrite.acts_for_session());
        assert!(Effect::FilesWrite.recorded() && Effect::Shell.recorded() && !Effect::Files.recorded() && !Effect::Network.recorded());
        assert!(Effect::Files.stamps() && Effect::Shell.stamps() && !Effect::FilesWrite.stamps() && !Effect::Console.stamps());
    }

    #[test]
    fn a_refusal_names_the_call_the_kind_and_the_rule() {
        let own = Context::View.refusal("write", Effect::FilesWrite, None);
        assert!(own.starts_with("write[] writes a file — a view shows"), "{}", own);
        assert!(own.contains("read the clock"), "{}", own);
        let through = Context::Derive.refusal("mail", Effect::Mail, Some("notify"));
        assert!(through.starts_with("calls notify[], which reaches mail[]: it sends a message — a derivation reads"), "{}", through);
        assert!(through.ends_with("call it from a handler, or a function a handler calls"));
        // a read a view may not make is a derivation's, which the view reads
        let read = Context::View.refusal("read", Effect::Files, None);
        assert!(read.ends_with("(it may write to the console, and read the clock): read it in a derivation the view reads — `derive x` above the view, returning it"), "{}", read);
    }

    /// Where a refused call belongs is decided by its kind alone: a read a
    /// derivation may make goes to a derivation, a write or any other kind
    /// to a handler — and a call of several kinds by the one a derivation
    /// cannot hold
    #[test]
    fn a_refused_call_belongs_where_its_kind_says() {
        for e in Effect::ALL {
            if Context::View.allows(e) {
                continue;
            }
            let derived = Context::View.refusal("f", e, None).contains("`derive x`");
            assert_eq!(derived, Context::Derive.allows(e), "{:?}", e);
            assert!(Context::Derive.refusal("f", e, None).ends_with("a function a handler calls") || Context::Derive.allows(e), "{:?}", e);
        }
        assert_eq!(Context::View.refused(&[Effect::Network, Effect::Waits]), Some(Effect::Waits));
        assert_eq!(Context::View.refused(&[Effect::Files]), Some(Effect::Files));
        assert_eq!(Context::View.refused(&[Effect::Console, Effect::Clock, Effect::Random]), None);
        assert_eq!(Context::Derive.refused(&[Effect::Files, Effect::FilesWrite]), Some(Effect::FilesWrite));
    }
}
