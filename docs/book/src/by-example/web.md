# Web: a JSON route

```crow,norun
use web

fn health[req]
    return json[map[status = 'ok']]

route['/health', health]
serve[3000]
```

`route[path, handler]` registers a plain HTTP handler — a function that
receives the request and returns a response; `page[path, view]` registers a
live page (chapter 12). Responses come from `json`, `html`,
`text`, `respond[status, body]`, and `redirect[url]`. The handler is handed
the request: `req.params` holds the path's `:name`s, `req.query` the query
string's values, `req.body` the body, and `req.method`, `req.path` and
`req.headers` the rest; `route['/items', 'POST', add]` answers a method
other than GET.

Reference: spec section 13 (web) and book chapter 12.
