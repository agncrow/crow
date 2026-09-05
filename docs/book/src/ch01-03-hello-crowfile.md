# Hello, Crowfile!

Single files are fine for scripts; projects get a directory. `crow new`
scaffolds one:

```bash
crow new hello_crow
cd hello_crow
```

Look at what it made — two files:

```text
hello_crow/
  crowfile
  main.crow
```

The `crowfile` is the project manifest. It is itself a Crow file — one
assignment for now:

```text
name = 'hello_crow'
```

Later you'll add keys to it: `db = '...'` so the checker can read your
database schema (chapter 11), `jobs = map[...]` for batch tasks, `deps`
for packages. `main.crow` is the entry point, and it already runs:

```bash
crow run          # compile and run main.crow
crow run -i       # the same program, interpreted — instant start
crow build        # compile only; the binary lands in the project directory
```

`crow run` writes the generated Rust into `.crow_build/` inside the
project. You never need to look at it, and it's safe to delete — the next
run recreates it. The first compile takes a few seconds; after that only
what changed is rebuilt.

Two more commands you'll use in every chapter from here on:

```bash
crow check        # verify the project: your tests plus everything automatic
crow fmt          # format every .crow file in the project
```

For a web project, `crow new myapp --web` adds a `views/` directory
wired into the server (chapter 12); `--db` and `--ml` add database and
training scaffolds. Every scaffolded file is working code.

You now have everything the rest of this book needs. Next: build a real
program before learning any more theory.
