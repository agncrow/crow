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

A route's handler is handed its request and nothing else, so it opens what
it needs itself. Every `connect` to one URL in a program is one handle, so
the handlers share its pool — and a query that fails while the database is
away is the handler's to answer:

```crow,norun
use web, db

fn count[req]
    conn = connect[env['DATABASE_URL', 'sqlite://app.db']]
    q = query[conn, 'SELECT count(*) AS n FROM trades']
    if not q.ok
        return respond[503, q.reason]
    return text['trades: `q.n[0]`']

route['/count', count]
serve[3000]
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

A socket reads a live feed. Each failure is the call's error value — here,
a server that does not answer — so a program decides what to do about it:

```crow
use web, web.ws

feed = socket['ws://127.0.0.1:1/stocks']
if feed.ok
    send[feed, 'subscribe']
    log[recv[feed, 5]]
    close[feed]
else
    log['no feed: `feed.kind`']
```

```text
no feed: socket
```

A program asks another server with `request` — a GET, or any method with
a body. Its answer is a record, whatever the status: `status`, `body` and
`headers`. Only a request that got no answer is an error value, as here:

```crow
order = map[symbol = 'AAPL', side = 'buy']
r = request['http://127.0.0.1:1/orders', 'POST', encode[order]]
if not r.ok
    log['not placed: `r.kind`']
else if r.status eq 201
    log['placed: `r.body`']
```

```text
not placed: http
```

A key goes in its headers, and a body that is JSON is sent as JSON:

```crow,norun
token = env['API_TOKEN']
key = map[authorization = 'Bearer `token`']
r = request['https://api.example.com/orders/7', 'DELETE', headers = key]
log[r.status]
```

A query, a form, files and a timeout are options too, and `save` keeps a
download's bytes in a file:

```crow,norun
pw = 'not this one'
found = request['https://api.example.com/search', query = map[q = 'crow lang'], timeout = 5]
signed = request['https://example.com/login', 'POST', form = map[user = 'ada', password = pw]]
sent = request['https://example.com/upload', 'POST', form = map[title = 'July'], files = map[report = 'july.csv']]
logo = request['https://example.com/logo.png', save = 'logo.png']
log[logo.status]
```

Reference: spec section 13 (web), and Requests.
