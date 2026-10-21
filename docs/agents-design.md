# Agents in `use llm` — the loop, and what plugs into it

The second piece of Crow's LLM work, after the client (docs/llm-design.md):
an agent is a model running tools in a loop until it answers, and a
harness is the program around that loop — dispatch, limits, permissions,
context, records. Vijay (2026-10-18): "enhance the llm module to create
agents, harness skills etc.", the ideas explained first (the guide "Agents
in Crow", 2026-10-18, revised 2026-10-20 for the compiler's freeze), then
(2026-10-20) "build the missing pieces with your recommendations where you
need my decisions". This note is the design; the spec's `### llm` section
is the contract, and `src/modules/llm.co` the body.

## What it is

Crow could run an agent before this round: `chat` with `tools = [f, g]`,
the model's `calls` dispatched by the program's own `if`, each result
handed back with `told`, in a `while` the program writes (spec, Tools are
the functions). Written by hand, that loop has three gaps every program
closes again — no limit, a call the `if` does not handle left unanswered
(the next request is refused), a tool that fails ending the run with
nothing told to the model — and limits, error results and a record of the
steps get written again, differently, each time. `act` closes them once:

```
use llm

--- the current weather in a city, in °C
fn weather[city]
    r = request['https://wttr.in/`trim[city]`?format=j1']
    return str[json[r.body].current_condition[0].temp_C]

m = reach['anthropic://claude-sonnet-5']
done = act[m, ask['Is it warmer in Lisbon or Porto?'], tools = [weather], rounds = 8]
log[done.text]            --- the answer
log[done.stop]            --- end, or the limit that stopped the run
log[done.steps]           --- a frame: every model call and tool call, in order
```

Everything in it is module code written in Crow (design rule 20), on two
operations of the standard library's own (`describe` and `apply`, sys:
phase 4 of the compiler's freeze, docs/extending-crow.md section 7): the
compiler learns nothing of llm's (design rule 24). One bug of the
compiler's was fixed on the way, named below.

## The surface

| Function | What it does |
|---|---|
| `act[m, talk, tools = [], rounds = 8, system = '', tokens = 1024, temperature = 1.0, budget = 0]` | The model runs its tools in a loop until it answers or a limit stops it. Each reply's calls are answered in order by the tool of the call's name, applied to its arguments read by each parameter's kind; a name no tool has, an argument missing or of another kind and a tool's own failure are each told to the model as a failed result that says why, so every call is answered. At most `rounds` model calls, at most `budget` tokens in all (0 for none): the limit stops the run after the reply's calls are answered, so the conversation ends well-formed and can be continued. Answers a record: `text` (the model's last words), `stop` (`end`, `length`, `other`, `rounds`, `budget`), `rounds`, `usage` (`input`, `output` tokens in all), `talk` (the conversation as it ended) and `steps`, a frame with a row per model call and per tool call: `step`, `what` (`chat`, `tool`), `name` (the model's or the tool's), `input`, `output`, `ms`, `how` (a chat's stop; a tool's `ok`, `failed`, `arguments`, `unknown`). An error value of kind `'llm'` as `chat`'s, and for `rounds` below one |
| `told[talk, call, text, failed = false]` | As before; `failed = true` marks the result a failure the model is to hear as one: Anthropic's block carries `is_error`, the OpenAI-compatible turn its text alone (the wire has no flag) |

## Design notes

**Who runs the tools.** Considered: (A) the program writes the loop, as
before (kept as a way; rejected as the only way: the three gaps, written
again in each program); (B) the lowering expands `act` into a loop over
hidden rows of llm's, the dispatch written by the compiler per listed tool
(the guide's first recommendation, 2026-10-18 afternoon; rejected that
evening by design rule 24: the compiler names no module); (C) the module
dispatches by name at run time over a table from names to function values
(rejected: Crow has no such value — a list of functions is a group whose
members are known before the program runs — and the checker could not
tell which functions will run). Decided (2026-10-20): B in the form phase
4 made possible — the module writes the loop in Crow, and the dispatch is
`apply` over the group, which the compiler writes for any group of
functions. It answers the three objections of the spec's note "tools are
the functions": every call the model can cause is to a member of a list
the program wrote, and the dispatch is over those members alone; `crow
check` sees each member called, with its effects, since the dispatch is
written calls; and the program's own `if` is the list it wrote — and the
gate, below, once it can be typed. The hand-written loop stays what it is:
`chat`, `said`, `told` are the loop's own pieces.

**The gate waits on one inference rule.** The guide's `allow`: a function
of the call the program hands the loop, asked before every tool runs. Two
shapes were built and each met the compiler as it is. As a plain function
parameter (`act[m, talk, allowed, tools = …]`) it is typed by the loop's
own application of it, but a function taking a function is lowered once
(spec, Specialization: "a function taking a function is not copied per
kind"), so one `act` would serve one tool list per program — no subagent
with tools of its own beside its parent. As a group applied by name
(`allow = [g, h]`, each asked, which composes gates for free) the module
hands the call as a parsed value, and a parameter is typed only by a call
of it or by a single-kind builtin read in its body (`upper[city]`): a body
that reads the call's fields (`call.name eq 'weather'`) types nothing, so
`describe` refuses the gate as "typed by nothing", or "handed a parsed
value and read as nothing". The rule that closes it is small and general:
a parameter whose fields the body reads, and nothing else types, is a
parsed value — `param_reads` noting a field read as `Json`, where it
notes `upper[x]` as text today — which also lets a tool take an object
(`search[query, filters]`), refused the same way now. It is a change to
inference, so under rule 24 it is asked for, not made: the loop ships
without the gate, every tool the program lists allowed as listed, and the
gate comes as `allow = [g]` once the rule is in. Deferred with it: a reach
on `describe` (what a function touches), which a gate that refuses
effectful tools by default would need.

**The outcome told to the model.** A bad argument, a tool's failure and an
unknown name are all told, each as a failed result whose text says why —
`apply`'s own reason for an argument, the tool's reason, `no tool is
named nothing` — rather than ending the run, since a model that hears why
can try another way (the guide, §2–3). The text is the same on both
wires; Anthropic's carries `is_error` beside it. `apply` answers a value
encoded as text, so a text's own words are what the model reads
(`_plain_of`), any other value as its JSON.

**A run's record is a frame.** Considered: a list of records. Decided: a
frame, one row per step, so traces go to ClickHouse and to charts with
no conversion; its columns avoid `kind`, `ok`, `reason` and `trace`,
which are every value's own (a column named `kind` is unreadable, since
`f.kind` is the frame's status). The run's answer is a record (`Done`),
since a parsed value holds no frame: `done.steps` is the frame,
`done.talk` the conversation, `done.usage` a parsed value as a reply's.

**Where it lives.** The loop in `use llm` (and context and skills, when
they come); MCP as `use mcp`, since a server needs no model. A module
written in Crow declares its functions by writing them (phase 2), so a
module is one file and one line of the manifest.

**Names.** `act`, `rounds`, `budget` — short lowercase words, none a
builtin's or an export's (`budget`, `keep` and `step` are options of
other rows, which take nothing from a parameter's name). `tokens` is the
reply's limit, as `chat`'s; `budget` the run's. A helper of the loop was
first named `_applied`, which the streaming code already had: a second
function of one name in a module reads as the first, its parameters
"undefined variables" — the compiler does not name the collision.

**Context: cut, then compact.** `keep = n` replaces the tool results of
rounds older than the last n with a stub naming their size — a result
once used is what fills a window (the guide, §6) — and marks the turn
`cut`, so a cut turn is never cut again and a program reading the
conversation back knows. `compact = n` asks the model, once a reply has
read n tokens or more, for a summary of the run for itself (the task,
what was done and found, what remains), with no tools so it answers in
words, and goes on from a conversation of that summary alone; the step is
recorded as a `compact` row. The summary is asked in one user turn of a
conversation of its own, holding the transcript (who spoke, the calls, the
results): a user turn added after a tool's results breaks the alternation
of roles the wires require, which the first run against a real model
showed ("conversation roles must alternate"). Both happen at a round's start, after the
previous reply's calls were answered, so the conversation sent is always
well-formed. Considered and left to the program: notes kept in a file
(two lines of the program's own tools), and clearing by age in tokens
rather than rounds (a round is what the model sees as a unit).

**Skills.** The Agent Skills format as published (agentskills.io): the
module reads each `SKILL.md`'s frontmatter by the standard's rules —
`name` 1 to 64 characters of lowercase letters, digits and single hyphens,
neither first nor last, matching the folder; `description` 1 to 1024 —
and refuses a folder that breaks one before any model is asked, naming
the skill and the rule. Progressive disclosure is two tools of the
module's own: `skill[name]` answers the instructions, `skill_file[name,
path]` a file inside the skill, a path that leaves it refused; the system
prompt lists each skill's name and description. Scripts are not run: that
is `shell` behind the gate, which waits on the gate. Considered: `crow
check` holding the folder to the rules (rejected: the checker is the
compiler's, which names no module; the module checks, and a test block
can hold a folder with no model).

**MCP.** A module of its own, `use mcp` (spec, section 13), the client on
`request`, the server a function a POST route calls, the 2026-07-28
revision as published: stateless, `_meta` on every request, the headers
the transport mirrors, the errors it names. `servers = [s]` on `act` lists
each server's tools into the schemas the model is told and calls them on
the server; a name two tools share — a program's, a skill's, a server's
— is refused at the start, as the protocol advises against aggregating
colliding names. The schema writer is written twice (llm's for a model,
mcp's for `tools/list`): a helper is a module's own, and an export only
programs can call would be documented for nothing.

**Not in this round** (the guide's §15, in order): the gate, above;
MCP over stdio, a process held open as a handle; subagents need nothing
new (a tool that runs its own `act`, a smaller model and its own tools,
since a function sees no top-level variable) and are shown rather than
built; parallel calls of one reply (a loop with effects runs in order);
traces and evals beyond the steps frame; runs that resume, agents served
as pages.

## What was fixed in the compiler

A function called by name — `apply`'s, or a group's member where the
group was handed — is noted as called with its parameters' kinds in the
scan, so inference types it and the compiled program keeps it
(`note_called_by_name`, phase 4). The note went through `note_call`,
whose scope rule makes a program's functions invisible inside a standard
module's inlined function (a module local named like one is indexed, not
called): `act`'s dispatch over `tools = [weather]` noted nothing, `weather`
stayed `city: ?`, and the compiled program had a call of a function it
did not hold ("cannot find function `weather`"). The note records the
signature itself now (`note_named_call`, past the scope rule, since a
group's names were resolved where the literal was written), and the
dispatch's call of the function is lowered as the program's call it is
(`applying`: the interpreter had met a by-name call of a function it does
not run). A bug fix under rule 24; parity 274 reproduces the first (the
compiled program had not built), tests/llm_server.rs the second.

Found by the loop's servers: a program's top-level value named with a
standard module's prefix (`mcp_s = server[…]`) was printed as a Rust
constant by the emitter's rule for a user module's hoisted values
(`mymath_pi`), and the compiled program did not build; a standard module
hoists no value (the inliner carries its functions and records alone), so
its prefix names none now (`emit_module_consts`). Found by the first run
against a real model: a function copied per signature was scanned
without entering its own scope (`scan_clone` bound the parameters alone),
so its first assignment to a name was not a local of its own and the
program's top-level names were visible inside a standard module's copy —
the copy of `act` made for a second tool list read the program's `one`,
the record the first call answered, as the loop's local `one` and refused
its own push; the copy's scan enters the scope as the function's own does
(parity 275). And two shapes a module
written in Crow needs: a list parameter with a `[]` default makes a copy of
the function whose items are of no kind, so a helper that takes the list
typed by another copy does not build — `act` reads its servers as their
JSON (`json[encode[servers]]`), the same in every copy; and an import's
alias is not resolved inside a module (`use mcp as peer` left `peer`
undefined), while a bare `use mcp` reserves the exports' names from the
module's parameters, which is why the listing is `catalog` and not
`tools`.

## What holds it

- The module's own tests (`crow test src/modules/llm.co`, interpreted and
  compiled): a call answered by the tool of its name — the value as the
  model reads it, a missing argument, a tool's own failure, an unknown
  name; a failed result told as one in each wire format; the loop's
  limits and a model nobody answers.
- `tests/llm_server.rs`: the stand-in server scripts multi-step runs by
  the question asked — a tool called once then answered, a model that
  keeps calling stopped by `rounds` and by `budget`, an unknown name, a
  wrong argument, a tool that fails, two calls in one reply, a reply cut
  by its length — in both wire formats, the model's final text saying
  back what it was told (a failed result marked where the wire carries
  the flag), and the program's lines the same from both modes.
- `crow check --self`: the llm family's live cases `act` and
  `anthropic-act` run the loop against the check's model (the first tool
  called, its result said back) in the five configurations, the reference
  answering every line.
- Parity 274 (the loop without a model: its refusals, the failed result's
  turn, both modes), the book page, `crow test --docs`.

## The pieces

- `src/modules/llm.co` — `act` with `_answered`, `_outcome`, `_plain_of`,
  `_trimmed`, `_compact_ask`/`_compacted`, the skills helpers
  (`_skill_read`, `_skill_name_fault`, `_skills_in`, `_skills_prompt`,
  `_skill_schemas`, `_skill_loaded`, `_skill_file_read`), the servers'
  (`_served_tools`, `_served_call`, `_is_served`), `_result_anthropic`,
  `told`'s `failed`; the record `Done`; `use frame`, `use mcp`.
- `src/modules/mcp.co` — `server`, `discover`, `catalog`, `tool`,
  `answer` and their helpers; its line in the manifest
  (`src/modules/mod.rs`, with crow_web, since it answers responses).
- `src/lower_ir/named.rs` (`note_called_by_name`, `applied`),
  `src/type_infer/calls.rs` (`note_named_call`), `src/lower_ir.rs`
  (`applying`, `calls_fn`, the user-call guard) and
  `src/emit_rs/program.rs` (`emit_module_consts`) — the fixes.
- `src/main.rs` — the stdlib pages derived from the manifest (a hand-kept
  list had lacked mcp's page while the book's index linked it).
- Tests: `tests/llm_server.rs` (the scripted runs, a stand-in MCP
  endpoint), `tests/mcp.rs` (both directions), parity 274, the modules'
  own tests, the self-check's llm cases.
- The spec (`### llm`: the rows, "The loop is the module's", "Skills are
  know-how loaded on demand", the design note "who runs the tools";
  `### mcp`), docs/roadmap.md (P8), the book pages, CHANGELOG, CLAUDE.md.
