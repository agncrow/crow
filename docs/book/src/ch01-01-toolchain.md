# The Toolchain

One binary, a handful of verbs. Every one of them runs the same compiler
pipeline the editor uses, so nothing you see in one place disagrees with
another.

```text
crow                             the REPL, interpreted
crow new name [--web --ml --db]  scaffold a project (a crowfile, main.co, a .gitignore)
crow run [-i] [file]             execute: compiled when the compiler is here, else interpreted and told; -i interprets
crow run --opt N file            the program at optimization level N (0 to 3; 0 unless said, crow build is 3 with LTO)
crow dev [file]                  serve with the interpreter, reloading as files change (sessions kept)
crow stop                        stop the app this project runs (crow run, crow run -i, crow dev)
crow build [file]                compile to a native binary here, named after the file (./main in a project)
crow test [file]                 run the test blocks
crow check [file]                verify: tests, generated edge cases, contracts, and the app baseline — what changed since the accepted one
crow fix file                    apply the fixes the error messages name, and those crow check offers
crow fmt [file]                  format the files named, or the project's entry; --check only says
crow install                     fetch the crowfile's deps into crow_modules/, pinned by crow.lock
crow db diff [file]              the SQL that makes each database hold the tables the program declares
crow db apply dir url            run the migrations in dir that the database has not applied yet
crow setup [--check]             fetch the Rust toolchain and the runtime for compiled builds, once; --check says what is present
crow upgrade [--check]           replace this crow with the latest release, verified; --version X installs that one; --check says what is newer
crow book [--port N]             in the crow repository, serve this book with every block runnable; inside an app, the app's own book
crow lsp                         the language server (your editor runs it)
crow launch N [file]             run the program as N processes, each reading its rank from CROW_RANK
crow test --parity | --book      in the crow repository: the two execution modes must agree
crow test --docs [--write]       in the crow repository: the documentation must cover the compiler
```

`crow help` lists the commands, and the toolchain guide
(`docs/usage.md`) has every flag.

Two execution modes, one behaviour. `crow run -i` interprets: instant
start, the mode to use while you explore. `crow run` transpiles to Rust
and builds a native binary: the mode to ship. The binary you installed
holds the interpreter and needs nothing else; the compiler is a Rust
toolchain that `crow setup`, or the first `crow build`, fetches into
`~/.crow` once, and until it is there `crow run` says so and interprets.
They are held to identical output by `crow test --parity` over a corpus
of programs and by `crow test --book` over every code block in this book.
Interpret while you explore; compile when you ship. That is the whole
workflow.

`crow upgrade` keeps the binary current: it replaces the one the install
line put in `~/.crow/bin` with the latest release, held to its published
hash, and the new version fetches its own compiler the same way, on its
first compiled build. A crow built from a checkout upgrades by `git pull`
and `./install.sh --dev` or `--source` again; `crow upgrade` says so
rather than replace it.

A project's `crowfile` names the entry point and the resources: keys
`name`, `entry`, `db`, `schema`, `cpu`, `gpu`, `deps`, `jobs`. The next
page makes one.
