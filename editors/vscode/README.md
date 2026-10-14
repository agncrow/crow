# Crow for VS Code

Language support for [Crow](https://github.com/agncrow/crow), a programming
language for data work and web applications: it looks like Python, compiles
to Rust, runs parallel by default and treats errors as values.

The extension gives `.co` files syntax highlighting and connects the
editor to Crow's own language server, `crow lsp`, which is the compiler
itself: the diagnostics you see are the ones `crow run` would give, with
the same words, and so are the completions, the hover text, go to
definition and the document outline.

## Requirements

The `crow` binary, on your PATH. One line installs it on macOS or Linux:

```
curl -fsSL https://raw.githubusercontent.com/agncrow/crow/main/install.sh | bash
```

If the binary lives elsewhere, set `crow.serverPath` to its full path.

## Learn Crow

- [The Crow Book](https://agncrow.github.io/crow/), every code block of
  which runs in both of Crow's execution modes.
- [The language specification](https://agncrow.github.io/crow/language-spec.html),
  with a Design Note for every feature Crow deliberately leaves out.

Crow is in alpha: the language still changes between releases, and bug
reports are welcome in the repository's
[Issues](https://github.com/agncrow/crow/issues).
