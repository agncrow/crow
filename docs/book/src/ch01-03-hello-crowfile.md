# Hello, Crowfile!

Single files are fine for scripts; projects get a directory. `crow new`
scaffolds one:

```bash
crow new myproject
cd myproject
crow run
```

The scaffold includes a `crowfile` — the project manifest naming the entry
point — and a `main.crow`. For a web project, `crow new myapp --web` adds a
`views/` directory wired into the server (chapter 11).

You now have everything the rest of this book needs. Next: build a real
program before learning any more theory.
