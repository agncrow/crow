# A language model as a connection: `use llm`

`reach` makes a model a connection — `anthropic://<model>`, or
`openai://<host>/<model>` for any server speaking the OpenAI chat
completions (Ollama, vLLM, OpenRouter, LiteLLM). The key is read from the
environment when a call is made; a local host needs none. What `reach`
answers is a description the program can read:

```crow
use llm

m = reach['openai://localhost:11434/llama3.2']
log[m.provider]
log[m.model]
log[m.base]
log[m.key]
bad = reach['ftp://somewhere']
log[bad.ok]
log[bad.reason]
```

```text
openai
llama3.2
http://localhost:11434
OPENAI_API_KEY
false
cannot reach 'ftp://somewhere': a model's URL is anthropic://<model> or openai://<host>/<model>
```

A conversation is a parsed value, a list of turns, built a turn at a
time: `ask` the user's, `said` the model's reply, `told` a tool's result.
A reply is a parsed value too — its `text`, its `calls`, its `usage` and
its `stop` — so the builders take what `chat` answers:

```crow
use llm

talk = ask['What is the capital of Mongolia?']
log[len[talk]]
log[talk[0].role]
talk = ask[talk, 'And its population?']
log[len[talk]]
r = json['{"text": "Let me check.", "calls": [{"id": "c1", "name": "lookup", "args": {"city": "Ulaanbaatar"}}], "usage": {"input": 12, "output": 7}, "stop": "tool"}']
talk = said[talk, r]
log[talk[2].role]
log[talk[2].calls[0].name]
talk = told[talk, r.calls[0], '1.6 million']
log[talk[3].role]
log[talk[3].id]
```

```text
1
user
2
assistant
lookup
tool
c1
```

`chat` asks the model; a server nobody answers is the call's error value,
of kind `llm`, and so is a stream that cannot be opened (`follow`), whose
pieces (`next`) and whole (`reply`) then fail with it. `embed` answers a
matrix of embeddings from an OpenAI-compatible endpoint, which Anthropic
does not serve:

```crow
use llm

m = reach['openai://127.0.0.1:1/llama3.2']
talk = ask['Hello?']
r = chat[m, talk, system = 'Answer in one word.', max = 16]
log[r.ok]
log[r.kind]
s = follow[m, talk]
log[s.ok]
piece = next[s]
log[piece.ok]
whole = reply[s]
log[whole.ok]
e = embed[m, ['one', 'two']]
log[e.ok]
a = reach['anthropic://claude-sonnet-5?key=CROW_BOOK_NO_SUCH_KEY']
log[a.reason]
```

```text
false
llm
false
false
false
false
cannot reach 'anthropic://claude-sonnet-5?key=CROW_BOOK_NO_SUCH_KEY': the key's variable CROW_BOOK_NO_SUCH_KEY is not set
```

Tools are the functions themselves: `tools = [weather]` hands the function,
and the compiler writes its schema — the name, the parameters with the
kinds the body reads them as, and the comment above the function as the
description. The program dispatches the model's calls and hands each
result back with `told`:

```crow
use llm

--- the current weather in a city, in °C
fn weather[city]
    return 'sunny in `upper[city]`'

m = reach['openai://127.0.0.1:1/llama3.2']
talk = ask['How warm is it in Lisbon right now?']
r = chat[m, talk, tools = [weather]]
while r.ok and r.stop eq 'tool'
    talk = said[talk, r]
    for call in r.calls
        if call.name eq 'weather'
            talk = told[talk, call, weather[call.args.city]]
    r = chat[m, talk, tools = [weather]]
log[r.ok]
log[weather['Lisbon']]
```

```text
false
sunny in LISBON
```
