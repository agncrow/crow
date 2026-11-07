# Drawing in a Browser

A view is drawn by the browser from HTML. A program that draws its own
pixels — a chart of a million points, a 3D scene, a game, a page shown in
a VR headset — is built for the web instead: `crow build --target web`
compiles it to WebAssembly and writes the page that runs it.

```bash
crow build --target web spin.co
```

`spin_web/` then holds `spin.wasm`, `crow.js` and `index.html`. Serve the
directory over HTTP (any static server will do) and open it in a browser
on a desktop, a phone or a headset.

The program meets its page through two builtins of `sys`. `paint` hands
the page a batch of WebGL2 calls, and `frames` names the function the page
calls before each frame:

```crow
use sys

fn draw[frame]
    v = frame.views[0]
    shade = frame.t / 2000.0 - floor[frame.t / 2000.0]
    calls = [['viewport', str[v.x], str[v.y], str[v.width], str[v.height]], ['clearColor', str[shade], '0.4', '0.6', '1'], ['clear', 'COLOR_BUFFER_BIT']]
    sys.paint[encode[calls]]

sys.frames[draw]
log['drawing from the next frame on']
```

```text
drawing from the next frame on
```

A batch is a list of calls, each a list of text: a WebGL2 function's name,
then its arguments. Each argument is read by its form: a number, a
constant in capitals (`COLOR_BUFFER_BIT`), `#3` for an object an earlier
call kept, `=text` for text itself (a shader's source, a uniform's name),
`f32:0 0.5 1` and the other typed arrays for data. A call that begins
with `#N` keeps its answer as object N:

```crow
use sys

made = [['#1', 'createBuffer'], ['bindBuffer', 'ARRAY_BUFFER', '#1'], ['bufferData', 'ARRAY_BUFFER', 'f32:0 0.7 -0.6 -0.6 0.6 -0.6', 'STATIC_DRAW']]
sys.paint[encode[made]]
log[len[made], 'calls']
```

```text
3 calls
```

The frame the page describes is a parsed value: `t` in milliseconds, the
drawing's `width` and `height` in pixels, `xr` (whether a headset shows
it) and its `views`. A screen has one view; a headset has one per eye,
each with its viewport and the eye's `projection` and `camera` matrices, and
the page's **Enter VR** button starts it — or the program's own button,
drawn like the rest: where a frame's `vr` is true, a batch holding the call
`["@vr"]`, handed in the frame the button is pressed, enters the headset.

Run under `crow run` or `crow run -i` there is no page: a batch draws
nothing and no frame comes, the same in both modes. What a program draws
is therefore tested as the batches it builds, like any other value.

A page in a browser has no files, sockets, processes, databases or
servers, so the build refuses a program that reaches one, naming each
builtin. An engine written in Crow over these two builtins — scenes,
charts, text and controls — is [Lory](https://github.com/agncrow/lory).
