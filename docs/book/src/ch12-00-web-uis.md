# Building Web UIs

Crow's web story is the LiveView model: your UI logic lives on the
server, in Crow; the browser renders HTML and streams events back over a
WebSocket; state changes re-render automatically. No JavaScript build
step, no client framework, no API layer between your data and your page —
the dashboard in chapter 13 is one file.

Three pieces: `view` blocks describe the page, reactive state makes it
live, components make it composable. Then what every real app needs —
slow work that doesn't freeze the page, tests that drive the page without
a browser, and charts and styling that look right without writing CSS —
and last the rest of the element vocabulary and derivations, the values
a page computes from what it holds.
