# Crow by Example

Short programs, a page for each area. The chapters taught the ideas; this
is every corner of the language and its modules in example form, to read
through once and come back to. `crow test --book` checks every block: it
runs each one in both execution modes and holds its output to the text
printed under it; a block that serves, reaches another host or ends the
program is built without being run, and one that shows a mistake must be
refused. `crow test --docs` fails if a module is imported by no block of
the book, or a builtin is called by none, here or in the chapters. For the
tables, with every function linked back to the example that shows it, see
[The Standard Library](../stdlib/index.md) in Part II.

| Page | Covers |
|---|---|
| [Numbers and conversion](numbers.md) | `abs sqrt floor ceil round min max type int float str`, ints past 64 bits, `decimal`, `fraction` |
| [Strings](strings.md) | `trim upper lower replace starts ends repeat split join reverse len`, indexing, characters with `split[s, '']`, `occurs translate` |
| [Formatting](formatting.md) | `fmt` with `decimals commas percent width fill align` |
| [Regular expressions](regex.md) | `matches find sub` |
| [Arrays](arrays.md) | `sort reverse slice push pop concat range`, `x in xs` and `not in`, a part of text `in` text, `concat` of a list written down, a mask picking items, a matrix's number and row written in place |
| [Maps](maps.md) | `map keys values set del`, `k in m`, `map[keys, values]`, a map keyed by ints |
| [Files and directories](files.md) | `mkdir write exists read cp mv ls rm`, error values from I/O |
| [JSON](json.md) | `json encode`, open-schema fields, `encode` and `decode` with a scheme (`base64`, `url`) |
| [The system: shell, environment, time, arguments](system.md) | `shell env now args hash`; `sleep exit halt`, built but not run |
| [Moments and freshness](moments.md) | `moment` from parts, from text and in another zone, `now`, fields (`.hour`, `.weekday`), `fmt[t, style =, tz =]`, arithmetic in seconds, `captured` |
| [Math: tensors](math.md) | `*` and `.*`, `transpose det solve`, creating tensors, element-wise functions, shape operations, a mask picking a matrix's rows, counted by `sum` and taken by `where` |
| [Math: reductions and linear algebra](math-linalg.md) | `sum prod mean min max argmax argmin cumsum cumprod topk argsort unique where`, `axis =`; `det inv norm dot triu tril rank cond solve lstsq pinv qr svd cholesky eigen` |
| [Math: random, signal, special, and sparse](math-more.md) | `seed normal uniform poisson bernoulli gamma_dist beta_dist exponential shuffle choice permutation rand`, `fft ifft fft2 conv conv2d hamming hanning stft spectrogram lowpass highpass bandpass`, `gamma erf bessel factorial choose fma`, `complex real imag magnitude phase conjugate`, `sparse dense nnz spmv` |
| [Automatic differentiation](grad.md) | `grad` of a function of numbers, one parameter and several, and of a function of tensors |
| [Statistics: `use math.stats`](stats.md) | `median std var percentile corr linreg cov zscore ttest anova chisq fit kstest` |
| [Neural-network building blocks: `use math.nn`](nn.md) | `relu sigmoid softmax mse bce crossentropy gelu linear`, a misuse refused |
| [Structured files: `use io`](io.md) | `csv lines` |
| [Documentation: `use doc`](doc.md) | `chapters markdown menu pager plain document` |
| [Bits and character codes: `use bits`](bits.md) | `bitand bitor bitxor bitcount shift code char` |
| [Process memory: `use memory`](memory.md) | `remember recall revise forget canonical` |
| [Frames: `use frame`](frames.md) | `frame.load save select sort group pivot window join stack head tail fill unique rename stats complete sample unpivot where bucket`, masks (`f[mask]`), `frame.new[…]`, `df.name = values`, `in` over a column, a `then` chain as lines |
| [Databases: `use db`](db.md) | `connect query transact describe insert`, a transaction rolled back |
| [Web: a JSON route](web.md) | `route serve`, a JSON response |
| [Web: responses and submodules](web-more.md) | `text html redirect respond msgpack arrow bytes stream`, `serve`'s `cors` and `files`, a route's `limit`, a handler's own `connect`, `sign verify` (`web.auth`), `smtp` and `mail` (`web.mail`), `socket send recv close` (`web.ws`), `request` with its options |
| [Views: the whole element vocabulary](views-elements.md) | the 32 HTML elements, the containers `stack row grid overlay scroll`, `details summary dropdown modal drawer toast divider`, `fetch poll field slider`, a layout tested at two widths |
| [The standard library's layer: `use sys`](sys.md) | bytes, a connection (`open send recv close`), a process (`start`), files, digests, the readers' parsers, `describe` and `apply` |
| [A language model as a connection: `use llm`](llm.md) | `reach ask asked said told chat follow next reply embed`, tools as functions, `act` with a gate, a subagent, the loop's `budget keep compact`, skills and `servers` |
| [Tools from other programs: `use mcp`](mcp.md) | `server discover catalog tool`, a server over stdio, `answer` in a route, `listen`, an agent's `servers = [s]` |
