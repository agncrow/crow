# Web: responses and submodules

These blocks are compiled by the book's verification but not run — they
start servers.

Every kind of response a route can return:

```crow,norun
use web, frame

fn plain[req]
    return text['ok']

fn markup[req]
    return html['<b>ok</b>']

fn moved[req]
    return redirect['/']

fn missing[req]
    return respond[404, 'not here']

fn packed[req]
    return msgpack[map[status = 'ok']]

fn table_out[req]
    return arrow[frame.load['data.csv']]

fn raw[req]
    return bytes['raw']

fn numbered[put, n]
    for i in range[n]
        if not put['line `i`\n']
            return

fn flow[req]
    return stream[numbered, int[req.query.n]]

route['/plain', plain]
route['/markup', markup]
route['/moved', moved]
route['/missing', missing]
route['/packed', packed]
route['/table', table_out]
route['/raw', raw]
route['/flow', flow]
serve[3000]
```

`stream[numbered, n]` answers at once and then calls `numbered[put, n]` as
the response is sent: each `put` sends a chunk as it is put, and answers
whether the reader still listens, so `numbered` stops when they have gone.
Both modes answer every one of these alike, to the byte.

A server's options are `serve`'s and a route's: the sites whose pages may
call the app, a directory of files for the addresses no route answers, and
how often each client may call a route:

```crow,norun
use web

fn api[req]
    return json[map[ok = 'yes']]

route['/api', api, limit = 60]
serve[3000, cors = ['https://app.example.com'], files = 'public']
```

A form's fields arrive in `req.form` and its files in `req.files`, each a
copy the handler reads by its `path`.

An API has no page session, so it says who calls it with a token: `sign`
gives one, and `verify` reads its claims back — or says why not:

```crow
use web, web.auth

secret = 'a-secret-of-at-least-thirty-two-chars'
token = sign[map[user = 'alice'], secret]
claims = verify[token, secret]
log[str[claims.user]]
log[verify[token, 'another-secret-of-thirty-two-chars!!'].reason]
```

```text
alice
the token's signature does not match
```

A mailer is made from `web.mail`:

```crow,norun
use web, web.mail

smtp['smtp.example.com', 587, 'user', 'pass']
serve[3000]
```

The WebSocket client (`connect send recv close`) is specified in section 13
of the spec but does not yet have a compiled example; `crow test --docs`
lists it as *spec only* so the gap stays visible.

Reference: spec section 13 (web).
