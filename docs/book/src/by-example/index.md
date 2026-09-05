# Crow by Example

One short, runnable program per area of the language. The chapters taught
the ideas; this is every corner of the language and its modules in
example form, to read through once and come back to. Every block is
executed by `crow test --book` in both execution modes, and `crow test
--docs` fails the build if any builtin or module has no example here or
in the chapters. For the tables, with every function linked back to the
example that shows it, see [The Standard Library](../stdlib/index.md) in
Part II.

| Page | Covers |
|---|---|
| [Numbers and conversion](numbers.md) | `abs sqrt floor ceil round min max type int float str` |
| [Strings](strings.md) | `trim upper lower replace starts ends repeat split join reverse len`, indexing |
| [Formatting](formatting.md) | `fmt` with `decimals commas percent width fill align` |
| [Regular expressions](regex.md) | `matches find sub` |
| [Arrays](arrays.md) | `sort reverse slice push pop concat contains range` |
| [Maps](maps.md) | `map keys values has set del` |
| [Files and directories](files.md) | `mkdir write exists read cp mv ls rm`, error values from I/O |
| [JSON](json.md) | `json encode`, converting open-schema fields |
| [The system: shell, environment, time, arguments](system.md) | `shell env time args hash` |
| [Moments and freshness](moments.md) | `moment calendar parse_time now captured`, `fmt[t, time =]` |
| [Math: tensors](math.md) | creating tensors, element-wise functions, shape operations |
| [Math: reductions and linear algebra](math-linalg.md) | `sum mean min max argmax cumsum sort argsort topk unique where`, `solve inv det svd qr eigen norm dot lstsq` |
| [Math: random, signal, special, and sparse](math-more.md) | `seed rand randn normal shuffle choice`, `fft conv hamming stft lowpass`, `gamma erf bessel complex`, `sparse dense nnz spmv` |
| [Automatic differentiation](grad.md) | `grad` for one and many parameters |
| [Statistics: `use math.stats`](stats.md) | `median std var percentile corr linreg` |
| [Neural-network building blocks: `use math.nn`](nn.md) | `relu sigmoid softmax mse` |
| [Structured files: `use io`](io.md) | `csv lines` |
| [Frames: `use frame`](frames.md) | `frame.load filter sort select head group`, column reductions |
| [Databases: `use db`](db.md) | `connect exec query transact` |
| [Web: a JSON route](web.md) | `route serve`, a JSON response |
| [Web: responses, auth, sessions, and sockets](web-more.md) | `text html redirect respond msgpack arrow bytes sign verify sessions cache limit templates render uploads send recv close smtp` |
| [Views: the whole element vocabulary](views-elements.md) | all 32 elements, `fetch poll field` |
