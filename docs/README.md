# Crow documentation

Start here if you are new; jump to the reference if you already write Crow.

## Learn

| Document | What it is |
|---|---|
| [The Crow Book](book/src/SUMMARY.md) | The tutorial, in order: install → guessing game → concepts → functions and scope → collections → parallelism → errors → modules → testing → data project → databases → web UIs → a live dashboard. Every code block is executed in both modes by `crow test --book`. |
| [crow-for-llms.md](crow-for-llms.md) | The language on one page, written for AI assistants — paste it into a system prompt to get an accurate Crow tutor or code generator. |
| [usage.md](usage.md) | The toolchain guide: installing, every `crow` command, project layout, the crowfile, execution modes. |
| [comparison.md](comparison.md) | How Crow compares with Python, Julia, Rust, Go, Elixir, and others, axis by axis. |

## Reference

| Document | What it is |
|---|---|
| [Crow by Example](book/src/by-example/index.md) | One verified program per feature area — numbers, strings, regex, files, JSON, shell, moments, math, stats, nn, io, frames, db, web. Part II of the book site. |
| [The Standard Library](book/src/stdlib/index.md) | Generated reference: one page per module from the spec's tables, every function linked to the book pages whose verified blocks use it (`crow test --docs --write`). |
| [design-rules.md](design-rules.md) | **The design rules.** The nine principles, the implementation and documentation rules, and the feature workflow, numbered so the spec and the code can cite them. `CLAUDE.md` at the root points assistants here. |
| [language-spec.md](language-spec.md) | **The specification.** Every construct and builtin, each module's API, and a Design Note for every deliberately rejected feature (the answer to "why can't I…"). |
| [crow-check.md](crow-check.md) | `crow check` and `crow fix`: automatic verification, and the schema-aware database hazard catalog. |
| [crow-lsp.md](crow-lsp.md) | The language server: what it does and how to wire it into VS Code, Neovim, Helix, Zed. |
| [db-module.md](db-module.md) | `use db` internals: backends, pooling, bulk insert, transactions, migrations. |
| [frame-module.md](frame-module.md) | The frame (DataFrame) type: representation, benchmarks, operations. |
| [math-library.md](math-library.md) | The math runtime: tensor layers, linear algebra, GPU, autograd. |

## Design records

These explain how Crow got here. They are kept for the reasoning; where a code sketch disagrees with the spec, the spec is right.

| Document | What it is |
|---|---|
| [design-review.md](design-review.md) | The July 2026 honest review — ratings, pushbacks, and the log of how each was resolved. |
| [roadmap.md](roadmap.md) | What is done and what remains, by priority. |
| [discovered-features.md](discovered-features.md) | Language features mined from a real trading system's bug history, each written as a proposal; several have shipped. |
| [automatic-verification.md](automatic-verification.md) | The original design of `crow check`: purity, edge cases, snapshots. |
| [web-framework.md](web-framework.md) | The web framework's design and module inventory (vision document; sketches may predate current syntax). |
| [ui-framework.md](ui-framework.md) | The LiveView-style UI framework design, phase by phase (vision document; predates `view` blocks). |
| [style-module-design.md](style-module-design.md) | The default stylesheet, the seven classes and eight tokens, and the `css/app.css` escape hatch. |
| [derive-design.md](derive-design.md) | **Derivations, design round.** Values computed from facts and sources, recomputed when an input changes; freshness for external sources; keyed children; the finch simulation as the worked case. |
| [chart-module-design.md](chart-module-design.md) | The chart element: geometry kernel, budget, roadmap. |
| [ui-engine-design.md](ui-engine-design.md) | The layout engine design: a closed layout vocabulary, one engine in Crow, CSS as the wire format, exact UI tests without a browser (decided in principle, not built). |
| [finch-sim-design.md](finch-sim-design.md) | The second dogfood app — a trading simulator rebuilt in Crow. |

`docs/book/site.crow` renders the book with Crow itself (`use doc`): `crow run -i docs/book/site.crow` writes a static site to `docs/book/crow-site`. See `docs/usage.md`, "Building the book with Crow".
