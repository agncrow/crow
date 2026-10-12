# A model's answer as a fixture, and a RAG app

The third piece of the LLM domain (docs/llm-design.md, docs/gpt-design.md
are the first two): what makes a program that asks a model testable, and
the app that shows it — documents retrieved for a question and a model
answering from them. This note is the design, to be read before the code
is written (Vijay, 2026-10-11: the order agreed).

## A model's answer as a fixture

**What it is.** `crow check --record-fixtures` keeps what a program's
databases answered, and `crow test --fixtures` runs the tests from what
was kept, with no server (spec, `crow test --fixtures`; crow-check.md).
A request is kept the same way: in a recording, `request` sends it and
keeps its answer under `.crow/fixtures/<hash>.json`; in a replay, `request`
answers from the fixture and sends nothing. Every call that stands on
`request` is covered at once — `llm`'s `chat` and `embed`, a reader of a
URL — so an agent's test runs without the model, deterministically, and
in both modes.

**The key.** A fixture is named by the request as written: the method,
the URL and the body (`fnv` of the three, as the statements' fixtures are
named by their text). A changed prompt is a changed test, which is the
point: the body carries the whole conversation, the tools and the
options. The headers are no part of the key and are not kept — a key is
in a header, and no secret lands in a fixture.

**What is kept.** `{"kind": "request", "method", "url", "body", "reply":
{"status", "headers", "body"}}`: the record `request` answers, as it
answered it — the status, the headers as they came (names in lower case),
the body as text. A `save = path` request's fixture holds its answer as
the program saw it (`body` empty); the file it saved is the recording's,
on disk where the program wrote it.

**A miss.** A request no fixture holds, in a replay, is the request's
error value of kind `'http'`: `no fixture holds POST
https://api.anthropic.com/v1/messages — crow check --record-fixtures
records it where the model is`, named as a statement's miss is (`crow
test --fixtures` lists the misses at the end, as it does for statements).

**Where it lives.** In the Crow body of `request` (src/modules/core.co),
which both modes run: before the request is sent, `sys.replayed[method,
url, body]` answers the fixture where `CROW_REPLAY` names a project (the
switch `crow test --fixtures` already sets) or a recording runs
(`sys.replaying[]`, `sys.recording[]`), and after an answer comes in a
recording, `sys.recorded[method, url, body, status, text, headers]` writes
it — four rows of the `sys` layer over `crow_runtime::fixtures`. A
multipart body's boundary is random, so a request with files is keyed by
its fields and its files' field and path (`sent_as`). A recording runs
the tests that open no app too, for their fixtures alone, since the
replay runs every test. The hash is the one the statements'
fixtures use: `fnv` moves to `crow_runtime`, and `crow_db` reads it from
there (one hash, not two). The rehearsal's own intercept
(`rehearsal::outside`, the interpreter's) still answers a stand-in — `200`
with nothing to say — in a rehearsal, and in a recording lets `request`
run, so the fixture is kept; crow-check.md's paragraph says so.

**Not kept:** a reply followed piece by piece (`follow`, a socket of its
own); a test that must replay uses `chat`. Recording a followed reply's
bytes is a later note. (Built 2026-10-11: `tests/fixture_replay.rs`
records against a server of its own and replays in both modes.)

## The vectors package: `packages/vectors`

A flat index, exact: the documents' embeddings as a matrix with unit
rows, a query's nearest `k` by the inner product (the cosine, on unit
rows), the same numbers in both modes. `index[rows]` (the matrix, each
row normalised), `nearest[index, q, k]` (the positions and scores, best
first), `cosine[a, b]`. Held to faiss's `IndexFlatIP` by a reference
script over a corpus of random unit vectors and queries: the positions
exactly (the reference avoids ties), the scores to 1e-12.

An approximate index (HNSW, held to hnswlib's recall at a stated `ef`)
is the follow-up once a corpus asks for it; a flat index over a few
thousand chunks answers in microseconds compiled and is exact.

## The app: `examples/rag`

The third reference app, after the dashboard and the taskboard: a
directory of parts, served from its folder, its harness tests run by
`crow test`, driven by `crow test --served` in both modes.

| File | What it holds |
|---|---|
| `main.co` | `use web, ui, llm`; `serve`, the page, the model reached (`reach` of a URL the crowfile's `model = …` names, so a local Ollama and a hosted model are one line apart) |
| `corpus.co` | The documents: text files under `docs/` read and cut into chunks at blank lines, each chunk its file and its number; the chunks embedded once (`embed`, a derivation with `fresh`) and indexed |
| `retrieve.co` | `hybrid[question, k]`: the vector neighbours and `packages/retrieval`'s BM25 ranks fused by reciprocal rank (RRF, k = 60), the top chunks with their scores |
| `views/chat.co` | The page: a question, the chunks retrieved shown with their files, the answer with its citations — the conversation kept in the session's facts, `chat` with a system prompt that holds the model to the chunks |
| `tests` | `app[]` harness tests over fixtures: a question's retrieved chunks (no model needed), the answer and its citations (the model's answer replayed), a question nothing matches |

The answer is asked with `chat` (whole), the chunks in the system prompt
numbered, and the model told to cite them as `[n]`; the view links each
`[n]` to its chunk. A reply followed piece by piece into a live page — a
fact grown per piece under `later[]` — is the next step after fixtures
cover `chat`, and a note of its own (a handler commits when it ends, so
the pieces go through `later`).

## Design notes

**A request's fixture is keyed by its body.** Considered: by the URL
alone (rejected: every question would replay the first answer); by a
name the program gives (rejected: a second way to say what the request
already says). Decided: method, URL and body — what the request is.

**Fixtures in the Crow body, not in a backend.** Considered: the
interpreter's effect dispatch and the emitter's call each consulting the
fixtures (rejected: two places, two chances to differ — design rule 12).
Decided: `request`'s own Crow body does both, through two `sys` rows; the
modes agree by construction.

**Exact first.** Considered: HNSW from the start (rejected: an
approximate index is held to a recall, not to an answer, and its graph
depends on an RNG and an insertion order the reference does not share;
the app's corpus does not need it). Decided: a flat index held exactly,
HNSW as a package held to recall when a corpus asks for it.

**Hybrid by reciprocal rank.** Considered: a weighted sum of the two
scores (rejected: the scales are not alike, and the weight is a knob with
no reference). Decided: RRF with k = 60 as the paper states it, over the
two rankings; the retrieval package's BM25 is the lexical side.

## What holds it

- `tests/fixture_replay.rs` gains a request: a program asks a local
  server (the harness's), a recording keeps the answer, the server is
  stopped, `crow test --fixtures` answers from the fixture in both modes,
  and a request the recording never saw is named as a miss.
- `packages/vectors/held.co` against `reference/reference.py` (faiss).
- `examples/rag`'s tests under `crow test --fixtures`, driven by the
  served harness (`tests/served`) for the page, with the fixtures
  recorded against the llm harness's fake server and kept with the
  example.
- The spec: Requests (a recording keeps a request's answer, a replay
  answers it), `crow test --fixtures`; crow-check.md; the book's page on
  verification.
