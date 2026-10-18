# llm

A language model as a connection: conversations, replies, tools as the functions themselves, a reply followed piece by piece, and embeddings, over Anthropic's and the OpenAI-compatible wire formats.


Import with `use llm`. A language model as a connection: one way to ask a model, the same in both modes, over two wire formats — Anthropic's Messages API, and the OpenAI-compatible chat completions that Ollama, vLLM, OpenRouter and LiteLLM serve too — so a program written against a local model runs unchanged against a hosted one. Written in Crow on the `sys` layer (design rule 20; `src/modules/llm.co`), its rows in the builtin table its contract; `crow test src/modules/llm.co` runs its own tests, and `tests/llm_server.rs` drives it against a server speaking both formats in both modes. The design is docs/llm-design.md.

```
use llm

m = reach['anthropic://claude-sonnet-5']
talk = ask['What is the capital of Mongolia?']
r = chat[m, talk, system = 'Answer in one word.']
log[r.text]                              --- Ulaanbaatar
log[r.usage.input]                       --- 21
```

| Function | Description | Example |
|----------|-------------|---|
| `reach[url]` | A model as a connection: `anthropic://<model>` or `openai://<host>[:port]/<model>` (any server speaking the OpenAI chat completions: Ollama at `openai://localhost:11434/llama3.2`, vLLM, OpenRouter, LiteLLM). The key is read from the environment when a call is made — `ANTHROPIC_API_KEY` or `OPENAI_API_KEY`, or the variable `?key=NAME` names — and is in no value; a local host (`localhost`, `127.0.0.1`) needs none and is reached over `http`, any other over `https`. Answers the model's description, a parsed value: `m.provider`, `m.model`, `m.base`, `m.key` (the variable's name). An error value of kind `'llm'`: a URL of neither scheme, no model named, the key's variable unset for a remote host | [A language model as a connection: `use llm`](../by-example/llm.md) |
| `ask[text]` | A conversation of one user turn: a parsed value, a list of turns | [A language model as a connection: `use llm`](../by-example/llm.md) · [The standard library's layer: `use sys`](../by-example/sys.md) |
| `ask[talk, text]` | The conversation with another user turn added | [A language model as a connection: `use llm`](../by-example/llm.md) · [The standard library's layer: `use sys`](../by-example/sys.md) |
| `said[talk, reply]` | The conversation with the model's reply added — its text and its tool calls — as the next request needs it | [A language model as a connection: `use llm`](../by-example/llm.md) |
| `told[talk, call, text]` | The conversation with a tool's result added: what the program computed for one of the reply's `calls` | [A language model as a connection: `use llm`](../by-example/llm.md) |
| `chat[m, talk]` | The model's reply: a parsed value with `text`, `calls` (each `id`, `name` and `args`, the arguments as the model wrote them, read as the program's use implies), `usage` (`input` and `output` tokens) and `stop` — `end`, `tool` (the model waits for its calls' results), `length` (its `max` reached) or `other`. By name: `system = text`, `tools = [f, g]` (the functions themselves, see below), `max = tokens` (1024), `temperature = t` (1.0). An error value of kind `'llm'` when no answer came, when the server's status is no success (its message quoted: `the model's server answered 401: invalid x-api-key`) or when its answer is no JSON | [A language model as a connection: `use llm`](../by-example/llm.md) |
| `follow[m, talk]` | The same request, the reply followed as the model writes it: a socket the pieces are read from; the same options and errors as `chat` | [A language model as a connection: `use llm`](../by-example/llm.md) |
| `next[s]` | The next piece of the reply's text, as the model writes it; empty text once the reply has ended. A tool call the model streams is gathered into `reply`, not shown. An error value of kind `'llm'` when the connection breaks | [A language model as a connection: `use llm`](../by-example/llm.md) |
| `reply[s]` | The whole reply the socket has carried so far, in `chat`'s shape — complete once `next` has answered empty text | [A language model as a connection: `use llm`](../by-example/llm.md) · [The standard library's layer: `use sys`](../by-example/sys.md) |
| `embed[m, texts]` | The texts' embeddings: a matrix, a row per text, from an OpenAI-compatible embeddings endpoint (`openai://host/<embedding model>`); an Anthropic connection is refused, since it serves none | [A language model as a connection: `use llm`](../by-example/llm.md) |

`chat`, `follow`, `next` and `embed` are effects: a route's handler that calls one runs off the server's threads (Routes), and `crow check` sees the call.

**Tools are the functions.** `tools = [weather, search]` hands the functions themselves, and the compiler writes each one's schema from what it knows: its name, its parameters by name with the kinds inference found — an int `integer`, a float `number`, text `string`, a bool `boolean`, a list an `array` of its items' kind, a record an `object` of its fields, a parsed value an `object` — and, as the description, the comment written above the function. A parameter's kind is what the program's calls of the function hand it, or, where the program never calls it (a tool only the model calls), what its body reads it as: handed to a builtin whose rows take one kind there, `upper[city]` reads it as text. (Only a call had typed one, and a tool read in its body was refused with the advice to read it there.) A function whose parameter nothing types is refused before the program runs, with the fix (`weather's parameter 'city' is typed by nothing — read it in the body, or call weather[…] once with a value of its kind`), and so is a `tools` that is not a list of functions written. The program makes the calls: the reply's `calls` name a tool and its arguments, the program dispatches on the name, reads the arguments as its use implies and hands each result back with `told`.

```
--- the current weather in a city, in °C
fn weather[city]
    r = request['https://wttr.in/`city`?format=j1']
    return str[json[r.body].current_condition[0].temp_C]

talk = ask['How warm is it in Lisbon right now?']
r = chat[m, talk, tools = [weather]]
while r.stop eq 'tool'
    talk = said[talk, r]
    for call in r.calls
        if call.name eq 'weather'
            talk = told[talk, call, weather[call.args.city]]
    r = chat[m, talk, tools = [weather]]
log[r.text]
```

**A reply followed is a loop the program writes.** A route forwards the pieces as it reads them with the response `stream[feed]`, a page keeps them in a fact a view shows, a script logs them:

```
s = follow[m, ask['Tell me a story in three sentences.']]
piece = next[s]
while piece.ok and len[piece] gt 0
    log[piece]
    piece = next[s]
whole = reply[s]
```

A conversation is a parsed value — a list of turns, each its `role` (`user`, `assistant`, `tool`) and `text`, a model's turn with its `calls`, a tool's with the `id` of the call it answers — because a map holds one kind and a turn holds several, and JSON is the language's shape for that; the builders keep every turn in the one shape both wire formats are written from, so a conversation started against one provider continues against the other. One reply shape the same way: what one provider answers and the other does not is not there until both mean it.

**Design Note: a model as a connection, named `reach`.** Considered: the model named on every call, as most SDKs take a model string (rejected: the URL parsed and the key's presence checked at every call, and a program's calls not read as calls on one model); `connect`, as `db` makes its handle (rejected: `connect` is `db`'s own form, and a program that imports both — an agent that keeps its notes in a database — would write every call qualified); `model` (rejected: the name most programs give the thing they fit). Decided: `reach[url]`, a handle made once; the connection holds no secret, since the key is read from the environment as each call is made, so what it holds is description and a parsed value is readable where a handle kind would be opaque (`m.model` in a log line), with no kind to thread through the compiler (design rule 21).

**Design Note: tools are the functions, the schema the compiler's.** Considered: a tool written by hand — a name, a description and a schema beside the function (rejected: the signature said twice, and the two drift); the module calling the tool for the program and sending the result back by itself, as agent SDKs do (rejected: a call the program never wrote, an effect `crow check` could not see, and a model deciding what runs with no `if` of the program's between); a description handed beside the tool, `describe = map[weather = '…']` (rejected: one thing in two places — the comment above the function is the sentence the code already carries). Decided: `tools = [f, g]`, the compiler writes each schema from the signature it inferred and the function's comment, and the program dispatches.

**Design Note: a reply followed is a socket read piece by piece, not a feed.** Considered: a function handed to `follow` and called for each piece, as a response `stream[feed]` hands its feed a `put` (rejected: a builtin's row takes no function value — one handed to a row is typed by nothing — and a loop the program writes can stop, show, keep or forward a piece where a callback only receives it). Decided: `follow` answers a socket, `next` the pieces, `reply` the whole; the loop is the program's.

Not in this round, each a later note: images and documents in a turn, a conversation the server keeps (prompt caching, stored responses), batches, a model's answer recorded as a fixture so an agent's test replays without the model (on `request`'s rehearsal), and `embed` for a provider that serves no OpenAI-compatible endpoint.



Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
