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

route['/plain', plain]
route['/markup', markup]
route['/moved', moved]
route['/missing', missing]
route['/packed', packed]
route['/table', table_out]
route['/raw', raw]
serve[3000]
```

A session store, a cache, a rate limiter, and a mailer are created from
their submodules:

```crow,norun
use web, web.session, web.cache, web.rate, web.mail

sessions[]
cache[100, 60]
limit[10, 60]
smtp['smtp.example.com', 587, 'user', 'pass']
serve[3000]
```

The remaining `web` names — `sign`/`verify` (JWT), `templates`/`render`,
`uploads`, `stream`, the WebSocket client (`connect send recv close`), `sse`,
`ws`, `cors`, `static_dir`, and the legacy `sqlite` — are specified in section 13
of the spec but do not yet have a compiled example; `crow test --docs`
lists them as *spec only* so the gap stays visible.

Reference: spec section 13 (web).
