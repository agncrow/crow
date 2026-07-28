# The style module — design

**Status: SHIPPED (Layers 0–2 + checker guard), 2026-07-28.** The
default stylesheet lives in `crow_web` (`DEFAULT_CSS`, ~3KB) and is
emitted into every page shell: 8 tokens as CSS custom properties, Layer
0 element defaults, Layer 1 classes (`card stats controls row primary
up down stale`), auto dark/light plus `data-theme` override hooks. A
project's `css/app.css` is inlined after the defaults when present;
`crow check` flags unknown `--crow-*` token overrides with a
did-you-mean. Survey + rulings recorded below; C4-era additions (more
tokens, `active` state, widget variants) return through this doc.

Driven by the finch app's first serve (unstyled browser defaults) and
the framework discussion of 2026-07-27/28.

---

## 1. Module, not framework

What JS calls a framework is mostly compensation for what the host
language doesn't own. Crow already owns the framework layer: component
model and reactivity are `view` + reactive state + LiveView; routing is
`page[]`; state sync is the LiveView protocol; build tooling is
`crow build`. What's missing is the **design-system layer**, and it
should arrive the way `frame`, `moments`, and `db` did — as a module:
one blessed way, vocabulary-audited, shipped with the language,
identical in both modes.

A "UI framework for Crow" would import the disease Crow refuses:
extension points, configuration, competing component kits, a second
place where truth lives. The model to follow is SwiftUI/Flutter, not
Bootstrap: **the platform owns the design system**, and apps look right
with zero styling code.

## 2. The doctrine tension, resolved by layers

Two Crow precedents point in opposite directions:

- **Mini-languages get deleted** (strftime → `time = 'iso'`; format
  strings → `fmt` named args). CSS is the largest mini-language in
  computing.
- **SQL stayed SQL** — a mature domain language lives in literal
  strings/files, checked by tooling, not reinvented.

Resolution: the precedents govern different layers.

### Layer 0 — default beauty, zero code

Crow ships a real default stylesheet in the classless tradition
(water.css / pico.css scale, ~2–4KB): typography scale, spacing rhythm,
styled buttons/inputs/tables/forms, automatic dark/light via
`prefers-color-scheme`. Every Crow app looks decent with no styling
code — the zero-annotation principle applied to appearance. This is
the whole game; everything else is refinement.

### Layer 1 — a tiny blessed class vocabulary

The default stylesheet also styles a deliberately small set of semantic
classes — the ones the dogfood apps already invented unstyled:

- layout/grouping: `card`, `stats`, `controls`, `row`
- domain state (proposed, needs ruling): `up`, `down`, `stale`

Every entry passes the vocabulary rule individually; ~ten is the
ceiling. These are documented in the book, not discovered by reading
the stylesheet.

### Layer 2 — the escape hatch is plain CSS

A `css/app.css` file in the project is served and linked automatically
(crowfile override for the path). CSS is CSS, exactly as SQL is SQL: no
CSS-in-Crow, no inline-style DSL, no styled-components analog. When the
defaults are outgrown, the author writes the real language in its own
file, and Crow does not fight a full override.

**Rejected** (pending pushback): styling via element named args
(`div[pad = 2, color = 'muted']` — a design system as unbounded
vocabulary); CSS strings inside views; a theming *system* (a palette of
tokens, yes; a framework of themes, no).

## 3. One palette, shared with the chart kernel

The chart module's roadmap already names themes (C4). Whatever tokens
exist — background, surface, text, muted, accent, `up`, `down`, grid —
must be defined **once** and read by both the stylesheet and
`crow_runtime::chart`'s color selection, or the first dark-mode chart
will betray the split. Proposed home: the tokens live in the style
module; the chart kernel takes resolved colors as inputs.

## 4. Widgets are elements, earned one at a time

The remaining thing frameworks bundle is interaction components. In
Crow these are new `ui` elements, each individually earned through the
vocabulary rule as dogfood demands them — not a component library
adopted wholesale. Current demand from the sim page: `select` (the
symbol picker is a bare text field today) and `toast` ("order filled").
Tabs and modal-confirm are near-term candidates. A date-picker
framework is not.

## 5. Build order (proposed)

1. Layer 0 stylesheet + Layer 1 classes (small; instantly improves both
   dogfood apps; chart needs the tokens anyway)
2. Chart C0 consumes the tokens
3. `select`, `toast` as the sim page demands them

## 6. Open questions for this round

1. **Does Crow ship opinionated default beauty** (Layer 0)? This is
   the load-bearing ruling; everything else assumes yes.
2. **Raw CSS as the blessed escape hatch** (SQL precedent) — or must
   styling be expressible in Crow itself?
3. **The class list**: how small, and do domain-state classes
   (`up`/`down`/`stale`) belong in the language's stylesheet or in each
   app's own CSS?
4. **Dark mode**: auto-follow the OS only, or also a crowfile/`serve`
   knob (finch wants dark-first)?
5. **Where tokens live** so the chart and stylesheet share one palette
   (§3) — style module owns them?
