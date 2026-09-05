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
`text`, `respond[status, body]`, and `redirect[url]`.

Reference: spec section 13 (web) and book chapter 12.
