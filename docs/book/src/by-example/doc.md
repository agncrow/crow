# Documentation: `use doc`

```crow
use doc

toc = doc.chapters['# Guide\n\n[Welcome](index.md)\n\n# Part I\n\n- [Setup](setup.md)\n  - [Install](sub/install.md)']
log[len[toc]]
log[toc.title[2]]
log[toc.depth[2]]
body = doc.markdown['# Setup\n\nRun *this* first.']
log[body]
nav = doc.menu[toc, 'sub/install.md']
log['class="d2 current"' in nav]
log['rel="prev"' in doc.pager[toc, 'sub/install.md']]
log[doc.plain['# Setup\n\nRun *this* first.']]
site_page = doc.document['Setup', body]
log[starts[site_page, '<!DOCTYPE html>']]
```

```text
3
Install
2
<h1 id="setup"><a class="anchor" href="#setup">Setup</a></h1>
<p>Run <em>this</em> first.</p>
true
true
Setup Run this first.
true
```

Six names, all pure: `chapters[]` turns a summary into a frame,
`markdown[]` turns a chapter into HTML, `menu[]` turns the frame into
navigation with collapsible groups and the current page marked, `pager[]`
gives the previous and next links, `plain[]` gives a chapter's words for
a search index, and `document[]` wraps a body in a complete page with the
kernel stylesheet, the runner, the sidebar behaviour, and search. The Crow Book itself is
rendered this way by `docs/book/site.co`, whose one view puts the menu
and the chapter side by side with the layout vocabulary and whose
`test layout[]` checks that arrangement at two widths without a browser.
Because both renderers call the same kernel, `crow run -i` and the
compiled binary write byte-identical pages.

Markdown code spans use backticks, which are interpolation inside a Crow
string. Read chapter text from files, as the book does, rather than
writing it inline.

Reference: spec section 13 (doc).
