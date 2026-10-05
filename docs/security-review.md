# crow_web security review

September 30, 2026, for the 1.0 bar (docs/roadmap.md). Scope: everything a
served Crow app exposes — a page's session and its socket, the sign-in
cookie, routes and the requests they read, uploads, static files, the
responses the server writes, and the limits on each. Method: the code read
path by path, and each finding tried over HTTP against a running app, in
both modes (both serve through crow_web, so one finding is both modes').

Each area says what holds, what was found and fixed (with the test that
holds it), and what is left, with a recommendation.

## Page sessions and the live socket

Holds:
- A session's token is 128 bits from the operating system's generator
  (`getrandom`), good once, replaced on every resume, and travels only over
  the socket.
- The socket opens only from the app's own page: its `Origin` must name the
  host the request came to. A client that sends no `Origin` is not a
  browser, so no page on another site can drive a session.
- A session accepts only what a render offered it: a click with the
  arguments the render froze into it, an input or a form a render drew. A
  helper no element names and a fact no input binds are refused and logged.
- Waiting sessions are bounded: ten minutes, at most a thousand.

Found and fixed:
- A message on the socket could be 64 MB (axum's default). A page's events
  are small and its files come by a request of their own, under the uploads
  limit, so a message is now at most 4 MB; a bigger one closes the socket,
  which the page opens again.
- A page's socket is accepted only with the `Host` the reader typed, and
  the book's deployment section (ch. 13) shows the proxy lines that keep it,
  with what each is for.

Left:
- Open sessions are not capped: a client can open many sockets, each a
  session. An app that faces the internet sits behind a proxy that limits
  connections per client (the deployment section says so). A cap on open
  sessions per client address is a candidate after 1.0.

## The sign-in cookie

Holds:
- The cookie is sealed with HMAC-SHA256 and its seal checked in constant
  time before anything in it is read; its end is inside the seal.
- `HttpOnly`, `SameSite=Lax`, and `Secure` when the page came over https
  through a proxy (`X-Forwarded-Proto`).
- A sign-out ends the sign-in everywhere, and the ended sign-ins are kept on
  disk until their cookies would have expired, so a copied cookie is refused
  after a restart too.
- The record is at most 1 KB. It is signed, not encrypted: the reader can
  read their own record, which the spec limits to an id, a name and a role.
- The secret is `CROW_SECRET` (at least 32 characters, refused at start
  otherwise) or a 256-bit key written to `.crow/keys/`, a directory that
  ignores itself in version control.

Found and fixed:
- The key file was written with the default permissions and then narrowed
  to the owner's, a moment in which another user on the machine could read
  it. It is created owner-only (`a_key_is_created_for_its_owner_alone`).

## Routes

Holds:
- A body bigger than the uploads limit (10 MB unless `serve[…, uploads = N]`)
  is answered 413 before the handler runs; a multipart body that does not
  parse is a 400.
- A route's `limit = n` answers each client at most n times a minute (429 and
  `Retry-After`); `X-Forwarded-For` is believed only from a proxy on this
  machine.
- CORS answers only the origins `serve[…, cors = […]]` names.
- API tokens (`sign`, `verify`) are HS256 and nothing else — a token naming
  another algorithm is refused — with a secret of at least 32 characters and
  an expiry by default.
- A query's parameters are bound (`?`); `crow check --db` flags SQL built by
  interpolation as an injection risk (hazard 1f).

Found and fixed:
- A response HTTP cannot carry — a header whose value holds a line break, as
  one a handler built from what a request sent may — was answered
  `internal error` with no reason anywhere. The line break never reached
  the wire (the HTTP library refuses it), but the cause was invisible. It is
  answered 500 with the reason, and the log says the same
  (`a_response_http_cannot_carry_says_why`). Header text beyond ASCII,
  which was refused the same way, is sent as its bytes.

Found and fixed (Vijay's decision, 2026-10-01):
- A handler that failed answered 500 with its name and its reason —
  `count stopped: the database does not answer: error connecting to server
  (Connection refused …)` — so a client learned the program's internals: a
  database's address, a file's path. The client is now told an id, and the
  server's log names the handler and the reason beside it, in both modes
  (`a_handler_that_stops_answers_500`; the lost-connection test reads the
  log for it). A response HTTP cannot carry answers the same way.
- A page's handler that stopped told its reader the reason as the
  session's notice — `skip_symbol stopped: clickhouse query error: …` —
  the same internals, to whoever had the page open. The reader is now told
  that it stopped, that nothing changed, and an id; the server's log has
  the reason beside the id, in both modes, and under `crow dev` the dev
  panel lists it (`tests/dev.rs`, served 06; Vijay, 2026-10-01). A test
  reads the reason from the click's value.
- A value that failed, rendered by a view, showed its reason in place —
  `error: cannot read '/etc/finch/db.txt': …` — on every render, to every
  reader of the page. It now shows `error: the server's log has why, as
  <id>`, and the log has the reason beside the id once (the same id for
  the same reason, so the page is stable under re-rendering), in both
  modes (`crow_web::core::response::failed_in_view`; served 11, 12, 14).
  A derivation still being computed shows `error: loading` as before
  (Vijay, 2026-10-05).

## Uploads

Holds:
- The limit is enforced while the body is read, not after.
- Each file lands in a directory of its own with a random 96-bit name, under
  a name made safe (letters, digits and `._-()` kept, leading dots dropped);
  a route's copies are removed once the handler answers, a page's when its
  session ends; `.crow/uploads` ignores itself in version control.
- An upload is claimed by a one-time ticket that expires.

## Static files

Holds:
- A path that climbs out of the directory (`/../`, `/%2e%2e/`) is a 404.

Found and fixed:
- A file or directory whose name starts with a dot was served:
  `serve[3000, files = '.']` handed out `/.env` and `/.crow/keys/secret` —
  the key the sign-in cookies are sealed with, so anyone could have forged a
  sign-in. A path through any dot-named segment is now no file the app
  serves (a 404), decoded first, so `%2e` is no way around it; `.well-known`,
  where a certificate authority looks, is served
  (`files_named_with_a_dot_are_not_served`).

## Transport

Holds:
- A served app listens on this machine only unless `CROW_HOST` says
  otherwise.

Left:
- TLS, slow-client protection (a client that sends its headers a byte at a
  time) and connection limits are the proxy's: an app facing the internet
  runs behind one, as the deployment section describes.
