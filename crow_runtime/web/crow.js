// The page a Crow program built for the web runs in (`crow build --target
// web`; the spec's "Building for the web"). It gives the program's module
// what crow_runtime::host imports — `write`, `now`, `random`, `paint` —
// and calls its exports: `crow_start` once, then `crow_frame` before each
// frame while the program asks for frames (`sys.frames`). It holds no
// engine of its own: a batch the program paints is a list of WebGL2 calls,
// run in order, each argument read by its form (below).
//
// An argument is text:
//   0.5, -2, 1e3          a number
//   COLOR_BUFFER_BIT      the WebGL2 constant of that name (capitals)
//   #3                    the object an earlier call kept as 3
//   =u_color              the text after the '='
//   f32:0 0.5 1           a typed array (f32, f64, i8, u8, i16, u16, i32, u32)
//   base64f32:AACAPw==    a typed array of the bytes base64 writes (little
//                         end first: `sys.float_bytes[xs, 4, true]`), for
//                         what is large
//   true, false, null     themselves
//   @framebuffer          the frame's target: the canvas, or a headset's layer
// A call whose first item is #N keeps its answer as object N.
//
// A call whose name begins with '@' is the host's, not WebGL2's — what a
// page can do that a program cannot do itself:
//   @atlas =font =chars size   a texture of the glyphs, each a distance
//                              field (sharp at any scale); its metrics come
//                              in the next frame's `said`
//   @image =url                a texture of the image; `said` has its size
//   @request =id =method =url =body   an HTTP request from the page; the
//                              answer comes in a later frame's `said`
//   @tone hz seconds =wave gain   a sound (WebAudio)
//   @keyboard =open|=close =text   the hidden text field that brings up a
//                              phone's keyboard and takes what is typed
//   @cursor =css-cursor        the pointer's shape over the canvas
//   @copy =text                the text onto the clipboard
//   @title =text               the page's title
//   @vr                        into the headset: a frame's `vr` says one can
//                              be entered, and the call is made in the frame
//                              after the reader's press (a browser enters
//                              only on a press); `said` has the failure, and
//                              where none can be entered, that is the failure
//
// A frame is described as JSON (see `frame` below): its time, the
// drawing's size, its views, whether a headset can be entered (`vr`), what
// happened since the last frame (`input`),
// where each pointer is (`pointers`), the headset's controllers, and what
// the host answered (`said`).
'use strict';

const Crow = (() => {
  const ARRAYS = { f32: Float32Array, f64: Float64Array, i8: Int8Array, u8: Uint8Array, i16: Int16Array, u16: Uint16Array, i32: Int32Array, u32: Uint32Array };
  const CONSTANT = /^[A-Z][A-Z0-9_]*$/;
  const decoder = new TextDecoder();
  const encoder = new TextEncoder();

  function start(options) {
    const canvas = options.canvas;
    const said = options.said || (() => {});
    const gl = canvas.getContext('webgl2', { antialias: true, xrCompatible: true, preserveDrawingBuffer: !!options.keep });
    if (!gl) {
      said('error', 'this browser has no WebGL2');
      return Promise.reject(new Error('no WebGL2'));
    }
    const objects = new Map();
    let target = null;
    let memory = null;
    let exports = null;
    let stopped = false;
    let lines = { 1: '', 2: '' };
    let last = '';
    let session = null;
    let space = null;
    const painted = [];

    const bytes = (ptr, len) => new Uint8Array(memory.buffer, ptr, len);

    function write(stream, ptr, len) {
      lines[stream] += decoder.decode(bytes(ptr, len));
      let at;
      while ((at = lines[stream].indexOf('\n')) >= 0) {
        const line = lines[stream].slice(0, at);
        lines[stream] = lines[stream].slice(at + 1);
        if (stream === 2) {
          last = line;
          console.error(line);
          said('error', line);
        } else {
          console.log(line);
          said('log', line);
        }
      }
    }

    function random(ptr, len) {
      for (let at = 0; at < len; at += 65536) {
        crypto.getRandomValues(bytes(ptr + at, Math.min(65536, len - at)));
      }
    }

    function argument(a) {
      if (typeof a !== 'string') throw new Error('an argument is text, got ' + JSON.stringify(a));
      if (a === 'true') return true;
      if (a === 'false') return false;
      if (a === 'null') return null;
      if (a.startsWith('#')) {
        const id = Number(a.slice(1));
        if (!objects.has(id)) throw new Error('no object ' + a + ' was kept');
        return objects.get(id);
      }
      if (a.startsWith('=')) return a.slice(1);
      if (a === '@framebuffer') return target;
      const colon = a.indexOf(':');
      if (colon > 6 && a.startsWith('base64') && ARRAYS[a.slice(6, colon)]) {
        // (decoded by the browser where it can: a byte at a time in script,
        // a frame's vertices had been a fifth of the frame)
        let buf;
        if (typeof Uint8Array.fromBase64 === 'function') {
          buf = Uint8Array.fromBase64(a.slice(colon + 1));
        } else {
          const raw = atob(a.slice(colon + 1));
          buf = new Uint8Array(raw.length);
          for (let i = 0; i < raw.length; i++) buf[i] = raw.charCodeAt(i);
        }
        const T = ARRAYS[a.slice(6, colon)];
        return new T(buf.buffer, buf.byteOffset, Math.floor(buf.length / T.BYTES_PER_ELEMENT));
      }
      if (colon > 0 && ARRAYS[a.slice(0, colon)]) {
        const body = a.slice(colon + 1).trim();
        return new ARRAYS[a.slice(0, colon)](body === '' ? [] : body.split(/\s+/).map(Number));
      }
      if (CONSTANT.test(a)) {
        if (gl[a] === undefined) throw new Error('WebGL2 has no constant ' + a);
        return gl[a];
      }
      const n = Number(a);
      if (a.trim() !== '' && !Number.isNaN(n)) return n;
      throw new Error(JSON.stringify(a) + ' is no argument form (a number, a CONSTANT, #id, =text, f32:…, true, false, null)');
    }

    // ---- the host's calls ----
    const answers = [];
    const input = [];
    const pointers = new Map();
    let audio = null;
    let field = null;

    // a distance field of an alpha image (Felzenszwalb and Huttenlocher's
    // exact transform, both ways), 0.5 at the glyph's edge
    function distance_field(alpha, w, h, spread) {
      const INF = 1e20;
      const outer = new Float64Array(w * h), inner = new Float64Array(w * h);
      for (let i = 0; i < w * h; i++) {
        const a = alpha[i] / 255;
        outer[i] = a === 1 ? 0 : a === 0 ? INF : Math.pow(Math.max(0, 0.5 - a), 2);
        inner[i] = a === 1 ? INF : a === 0 ? 0 : Math.pow(Math.max(0, a - 0.5), 2);
      }
      const n = Math.max(w, h);
      const f = new Float64Array(n), d = new Float64Array(n), z = new Float64Array(n + 1), v = new Int32Array(n);
      const pass = (grid, offset, stride, length) => {
        for (let q = 0; q < length; q++) f[q] = grid[offset + q * stride];
        let k = 0;
        v[0] = 0; z[0] = -INF; z[1] = INF;
        for (let q = 1; q < length; q++) {
          let s;
          do {
            const r = v[k];
            s = (f[q] - f[r] + q * q - r * r) / (2 * q - 2 * r);
          } while (s <= z[k] && --k > -1);
          k++; v[k] = q; z[k] = s; z[k + 1] = INF;
        }
        k = 0;
        for (let q = 0; q < length; q++) {
          while (z[k + 1] < q) k++;
          const r = v[k];
          d[q] = f[r] + (q - r) * (q - r);
        }
        for (let q = 0; q < length; q++) grid[offset + q * stride] = d[q];
      };
      for (const grid of [outer, inner]) {
        for (let x = 0; x < w; x++) pass(grid, x, w, h);
        for (let y = 0; y < h; y++) pass(grid, y * w, 1, w);
      }
      const out = new Uint8Array(w * h);
      for (let i = 0; i < w * h; i++) {
        const dist = Math.sqrt(outer[i]) - Math.sqrt(inner[i]);
        out[i] = Math.max(0, Math.min(255, Math.round(255 * (0.5 - dist / (2 * spread)))));
      }
      return out;
    }

    // the glyphs of `chars` in `font` at `size` pixels, one cell each in
    // rows of a square-ish texture, each a distance field; the metrics say
    // where each glyph's cell is and how far it advances
    function atlas(font, chars, size) {
      const spread = Math.max(2, Math.round(size / 8));
      const pad = spread + 1;
      const glyphs = Array.from(chars);
      const probe = document.createElement('canvas').getContext('2d');
      probe.font = size + 'px ' + font;
      const m = probe.measureText('Hg');
      const ascent = Math.ceil(m.fontBoundingBoxAscent || size * 0.8);
      const descent = Math.ceil(m.fontBoundingBoxDescent || size * 0.2);
      const cell_h = ascent + descent + 2 * pad;
      const widths = glyphs.map((c) => Math.ceil(probe.measureText(c).width) + 2 * pad);
      const across = Math.ceil(Math.sqrt(glyphs.length)) * Math.ceil(size + 2 * pad);
      let x = 0, y = 0;
      const cells = [];
      for (const w of widths) {
        if (x + w > across) { x = 0; y += cell_h; }
        cells.push([x, y]);
        x += w;
      }
      const width = Math.max(1, across), height = Math.max(1, y + cell_h);
      const paper = document.createElement('canvas');
      paper.width = width; paper.height = height;
      const ink = paper.getContext('2d', { willReadFrequently: true });
      ink.font = size + 'px ' + font;
      ink.textBaseline = 'alphabetic';
      ink.fillStyle = '#fff';
      // (each glyph an object of its character and its cell, read by its
      // fields: a list of them, where an object keyed by character would be
      // read by computed keys)
      const metrics = [];
      glyphs.forEach((c, i) => {
        ink.fillText(c, cells[i][0] + pad, cells[i][1] + pad + ascent);
        metrics.push({ char: c, x: cells[i][0], y: cells[i][1], width: widths[i], height: cell_h, advance: probe.measureText(c).width });
      });
      const pixels = ink.getImageData(0, 0, width, height).data;
      const alpha = new Uint8Array(width * height);
      for (let i = 0; i < width * height; i++) alpha[i] = pixels[i * 4 + 3];
      const field = distance_field(alpha, width, height, spread);
      const texture = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, width, height, 0, gl.RED, gl.UNSIGNED_BYTE, field);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      return { texture, said: { width, height, size, ascent, descent, pad, spread, glyphs: metrics } };
    }

    function image(url, keep) {
      const texture = gl.createTexture();
      const picture = new Image();
      picture.crossOrigin = 'anonymous';
      picture.onload = () => {
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, picture);
        gl.generateMipmap(gl.TEXTURE_2D);
        answers.push({ what: 'image', id: keep, url, width: picture.width, height: picture.height });
      };
      picture.onerror = () => answers.push({ what: 'image', id: keep, url, failed: 'the image did not load' });
      picture.src = url;
      return texture;
    }

    function request(id, method, url, body) {
      const init = { method };
      if (method !== 'GET' && method !== 'HEAD') {
        init.body = body;
        init.headers = { 'content-type': 'application/json' };
      }
      fetch(url, init)
        .then(async (r) => answers.push({ what: 'answer', id, status: r.status, body: await r.text() }))
        .catch((e) => answers.push({ what: 'answer', id, status: 0, body: '', failed: String(e.message || e) }));
    }

    // a buffer object holding `length` bytes of a URL's, from `offset` (a
    // model's geometry: fetched once per URL, never passing through the
    // program), answered with what = 'buffer' once its bytes are in
    const fetched = new Map();
    function buffer(url, target, offset, length, keep) {
      const made = gl.createBuffer();
      if (!fetched.has(url)) fetched.set(url, fetch(url).then((r) => {
        if (!r.ok) throw new Error('status ' + r.status);
        return r.arrayBuffer();
      }));
      fetched.get(url)
        .then((all) => {
          const from = offset || 0;
          const upto = length > 0 ? from + length : all.byteLength;
          const was = gl.getParameter(gl.VERTEX_ARRAY_BINDING);
          gl.bindVertexArray(null);
          gl.bindBuffer(target, made);
          gl.bufferData(target, new Uint8Array(all, from, upto - from), gl.STATIC_DRAW);
          gl.bindBuffer(target, null);
          gl.bindVertexArray(was);
          answers.push({ what: 'buffer', id: keep, url, bytes: upto - from });
        })
        .catch((e) => answers.push({ what: 'buffer', id: keep, url, failed: String(e.message || e) }));
      return made;
    }

    function tone(hz, seconds, wave, gain) {
      audio = audio || new (window.AudioContext || window.webkitAudioContext)();
      const osc = audio.createOscillator();
      const level = audio.createGain();
      osc.type = wave || 'sine';
      osc.frequency.value = hz;
      const now = audio.currentTime;
      level.gain.setValueAtTime(gain, now);
      level.gain.exponentialRampToValueAtTime(0.0001, now + Math.max(0.01, seconds));
      osc.connect(level).connect(audio.destination);
      osc.start(now);
      osc.stop(now + seconds + 0.02);
    }

    function keyboard(how, text) {
      if (!field) {
        field = document.createElement('textarea');
        field.setAttribute('autocapitalize', 'off');
        field.setAttribute('autocomplete', 'off');
        field.style.cssText = 'position:fixed;left:0;bottom:0;width:1px;height:1px;opacity:0;border:0;padding:0;resize:none';
        document.body.appendChild(field);
        field.addEventListener('input', (e) => {
          if (e.inputType && e.inputType.startsWith('insert') && e.data) input.push({ type: 'text', text: e.data });
          field.value = '';
        });
      }
      if (how === 'open') {
        field.value = '';
        field.focus({ preventScroll: true });
      } else {
        field.blur();
        canvas.focus({ preventScroll: true });
      }
    }

    function host(name, args, keep) {
      switch (name) {
        case '@atlas': {
          const made = atlas(args[0], args[1], args[2]);
          answers.push(Object.assign({ what: 'atlas', id: keep }, made.said));
          return made.texture;
        }
        case '@image': return image(args[0], keep);
        case '@buffer': return buffer(args[0], args[1], args[2], args[3], keep);
        case '@request': return request(args[0], args[1], args[2], args[3]);
        case '@tone': return tone(args[0], args[1], args[2], args[3]);
        case '@keyboard': return keyboard(args[0], args[1]);
        case '@cursor': canvas.style.cursor = args[0]; return null;
        case '@copy': if (navigator.clipboard) navigator.clipboard.writeText(args[0]).catch(() => {}); return null;
        case '@title': document.title = args[0]; return null;
        case '@vr':
          if (!navigator.xr) { answers.push({ what: 'vr', failed: 'this browser has no WebXR' }); return null; }
          enter_vr().catch((e) => answers.push({ what: 'vr', failed: String((e && e.message) || e) })); return null;
        default: throw new Error('the page has no call ' + JSON.stringify(name));
      }
    }

    function run(batch) {
      for (const call of batch) {
        let keep = null;
        let at = 0;
        if (typeof call[0] === 'string' && call[0].startsWith('#')) {
          keep = Number(call[0].slice(1));
          at = 1;
        }
        const name = call[at];
        if (typeof name === 'string' && name.startsWith('@')) {
          const answer = host(name, call.slice(at + 1).map(argument), keep);
          if (keep !== null) objects.set(keep, answer);
          continue;
        }
        const f = gl[name];
        if (typeof f !== 'function') throw new Error('WebGL2 has no call ' + JSON.stringify(name));
        const args = call.slice(at + 1).map(argument);
        const answer = f.apply(gl, args);
        if (keep !== null) objects.set(keep, answer);
        // (a shader or a program that does not build says why, where a
        // reader looks: the console)
        if (name === 'compileShader' && !gl.getShaderParameter(args[0], gl.COMPILE_STATUS)) {
          throw new Error('a shader did not compile: ' + gl.getShaderInfoLog(args[0]));
        }
        if (name === 'linkProgram' && !gl.getProgramParameter(args[0], gl.LINK_STATUS)) {
          throw new Error('a program did not link: ' + gl.getProgramInfoLog(args[0]));
        }
      }
    }

    function paint(ptr, len) {
      const text = decoder.decode(bytes(ptr, len));
      if (options.record) painted.push(text);
      try {
        run(JSON.parse(text));
      } catch (e) {
        said('error', 'paint: ' + e.message);
        console.error('paint:', e.message);
        throw e;
      }
    }

    function stop(e) {
      if (stopped) return;
      stopped = true;
      said('stopped', last || String(e && e.message || e));
    }

    // ---- input: in the drawing's pixels, from its top left ----
    const at_of = (e) => {
      const box = canvas.getBoundingClientRect();
      const ratio = canvas.width / Math.max(1, box.width);
      return [(e.clientX - box.left) * ratio, (e.clientY - box.top) * ratio];
    };
    const mods = (e) => ({ shift: e.shiftKey, ctrl: e.ctrlKey, alt: e.altKey, meta: e.metaKey });
    function pointer(type) {
      return (e) => {
        const [x, y] = at_of(e);
        input.push(Object.assign({ type, id: e.pointerId, device: e.pointerType || 'mouse', x, y, button: e.button }, mods(e)));
        if (type === 'up' || (type === 'leave' && e.pointerType !== 'mouse')) pointers.delete(e.pointerId);
        else pointers.set(e.pointerId, { id: e.pointerId, device: e.pointerType || 'mouse', x, y, down: e.buttons !== 0 });
        if (type === 'down') {
          canvas.setPointerCapture && canvas.setPointerCapture(e.pointerId);
          if (audio && audio.state === 'suspended') audio.resume();
        }
      };
    }
    if (canvas.tabIndex < 0) canvas.tabIndex = 0;
    canvas.style.outline = 'none';
    canvas.addEventListener('pointerdown', pointer('down'));
    canvas.addEventListener('pointermove', pointer('move'));
    canvas.addEventListener('pointerup', pointer('up'));
    canvas.addEventListener('pointercancel', pointer('up'));
    canvas.addEventListener('pointerleave', pointer('leave'));
    canvas.addEventListener('contextmenu', (e) => e.preventDefault());
    canvas.addEventListener('dblclick', (e) => {
      const [x, y] = at_of(e);
      input.push(Object.assign({ type: 'double', id: 'dbl', device: 'mouse', x, y, button: e.button }, mods(e)));
    });
    // (a wheel's device: `trackpad` for a swipe of a trackpad or of a
    // touch mouse, `mouse` for a wheel's notches. No browser says which, so
    // its deltas tell: lines or pages are a wheel's; macOS writes a notch
    // as a multiple of 4.000244140625 pixels (as Mapbox's scroll handler
    // reads it), Windows and Linux as a large delta with a `wheelDelta` a
    // multiple of 120; a delta across is a trackpad's, and so is any other.
    // A pinch is a wheel with Ctrl held, as browsers report it.)
    const wheel_device = (e) => {
      if (e.deltaMode !== 0) return 'mouse';
      if (e.deltaX !== 0) return 'trackpad';
      if (e.deltaY !== 0 && e.deltaY % 4.000244140625 === 0) return 'mouse';
      if (e.wheelDeltaY && e.wheelDeltaY % 120 === 0 && Math.abs(e.deltaY) >= 50) return 'mouse';
      return 'trackpad';
    };
    canvas.addEventListener('wheel', (e) => {
      const [x, y] = at_of(e);
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? canvas.height : 1;
      input.push(Object.assign({ type: 'wheel', x, y, dx: e.deltaX * unit, dy: e.deltaY * unit, device: wheel_device(e) }, mods(e)));
      e.preventDefault();
    }, { passive: false });
    window.addEventListener('keydown', (e) => {
      input.push(Object.assign({ type: 'key', key: e.key, code: e.code, repeat: e.repeat }, mods(e)));
      const printable = e.key.length === 1 && !e.ctrlKey && !e.metaKey;
      if (!printable && ['Tab', 'Backspace', ' ', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'PageUp', 'PageDown', 'Home', 'End'].includes(e.key)) e.preventDefault();
      // (a printable key with the text field closed is still text)
      if (printable && document.activeElement !== field) input.push({ type: 'text', text: e.key });
    });
    window.addEventListener('keyup', (e) => input.push(Object.assign({ type: 'keyup', key: e.key, code: e.code }, mods(e))));
    window.addEventListener('blur', () => input.push({ type: 'blur' }));

    // a headset's controllers and hands: each a ray from its pose, with its
    // buttons and axes; pressing select is a pointer's down and up
    function controllers(xr) {
      const held = [];
      for (const source of session.inputSources) {
        const pose = source.targetRaySpace && xr.getPose(source.targetRaySpace, space);
        if (!pose) continue;
        const m = pose.transform.matrix;
        const pad = source.gamepad;
        held.push({
          hand: source.handedness, device: source.hand ? 'hand' : 'controller',
          origin: [m[12], m[13], m[14]], direction: [-m[8], -m[9], -m[10]], matrix: Array.from(m),
          buttons: pad ? pad.buttons.map((b) => b.value) : [], axes: pad ? Array.from(pad.axes) : [],
        });
      }
      return held;
    }

    // a frame, described to the program as JSON: its time in milliseconds,
    // the drawing's size in pixels, whether a headset shows it, and each
    // view to draw (one on a screen; one per eye in a headset, with the
    // eye's projection and view matrices, column-major as WebGL takes them),
    // then what happened since the last frame
    function frame(described) {
      if (stopped) return false;
      described.input = input.splice(0);
      described.pointers = Array.from(pointers.values());
      described.said = answers.splice(0);
      described.vr = vr;
      if (!described.controllers) described.controllers = [];
      const text = encoder.encode(JSON.stringify(described));
      const ptr = exports.crow_alloc(text.length);
      bytes(ptr, text.length).set(text);
      try {
        return exports.crow_frame(ptr, text.length) === 1;
      } catch (e) {
        stop(e);
        return false;
      }
    }

    function fit() {
      const ratio = window.devicePixelRatio || 1;
      const w = Math.max(1, Math.round(canvas.clientWidth * ratio));
      const h = Math.max(1, Math.round(canvas.clientHeight * ratio));
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
    }

    function on_screen(t) {
      if (session) return;
      fit();
      target = null;
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      const w = canvas.width, h = canvas.height;
      const more = frame({ t, width: w, height: h, xr: false, ratio: window.devicePixelRatio || 1, views: [{ eye: 'none', x: 0, y: 0, width: w, height: h, projection: [], camera: [] }] });
      if (more && !options.manual) requestAnimationFrame(on_screen);
      return more;
    }

    function in_headset(t, xr) {
      const pose = xr.getViewerPose(space);
      const layer = session.renderState.baseLayer;
      session.requestAnimationFrame(in_headset);
      if (!pose) return;
      target = layer.framebuffer;
      gl.bindFramebuffer(gl.FRAMEBUFFER, layer.framebuffer);
      const views = pose.views.map((v) => {
        const vp = layer.getViewport(v);
        return { eye: v.eye, x: vp.x, y: vp.y, width: vp.width, height: vp.height, projection: Array.from(v.projectionMatrix), camera: Array.from(v.transform.inverse.matrix) };
      });
      frame({ t, width: layer.framebufferWidth, height: layer.framebufferHeight, xr: true, ratio: 1, views, controllers: controllers(xr) });
    }

    // whether a headset can be entered: asked as the page starts, and again
    // as the program enters one (a headset may have come since)
    let vr = false;
    const can_enter = () => navigator.xr.isSessionSupported('immersive-vr').then((yes) => (vr = !!yes), () => false);
    if (navigator.xr) can_enter();

    async function enter_vr() {
      if (session || !navigator.xr) return;
      // the browser's answer first: asked to make the context ready for a
      // headset while it still weighed whether there was one, Chrome had
      // answered neither, and the failure was never said (one page in
      // twelve, entering as it started)
      if (!(await can_enter())) throw new Error('no headset can be entered');
      await gl.makeXRCompatible();
      session = await navigator.xr.requestSession('immersive-vr', { optionalFeatures: ['local-floor'] });
      session.updateRenderState({ baseLayer: new XRWebGLLayer(session, gl) });
      try {
        space = await session.requestReferenceSpace('local-floor');
      } catch (e) {
        space = await session.requestReferenceSpace('local');
      }
      const ray = (type) => (e) => input.push({ type, device: 'ray', id: e.inputSource.handedness, hand: e.inputSource.handedness });
      session.addEventListener('selectstart', ray('down'));
      session.addEventListener('selectend', ray('up'));
      session.addEventListener('squeezestart', ray('squeeze'));
      session.addEventListener('squeezeend', ray('unsqueeze'));
      session.addEventListener('end', () => {
        session = null;
        space = null;
        requestAnimationFrame(on_screen);
      });
      session.requestAnimationFrame(in_headset);
    }

    const imports = {
      crow: {
        write,
        now: () => Date.now(),
        random,
        paint,
      },
    };

    const loading = fetch(options.module).then((r) => {
      if (!r.ok) throw new Error('the module ' + options.module + ' was not found (' + r.status + ')');
      return r.arrayBuffer();
    });
    return loading
      .then((bin) => WebAssembly.instantiate(bin, imports))
      .then(({ instance }) => {
        exports = instance.exports;
        memory = exports.memory;
        fit();
        try {
          exports.crow_start();
        } catch (e) {
          stop(e);
          return api();
        }
        if (!options.manual) requestAnimationFrame(on_screen);
        return api();
      });

    function api() {
      return {
        gl,
        enter_vr,
        painted,
        // (a test hands the page input as a reader would make it, and with
        // `manual` set draws each frame itself, at the time it says)
        // (a pointer's event moves the pointer too, as the reader's does)
        give: (event) => {
          input.push(event);
          const kinds = ['down', 'move', 'up', 'leave'];
          if (kinds.includes(event.type) && event.x !== undefined && event.device !== 'ray') {
            if (event.type === 'up' || (event.type === 'leave' && event.device !== 'mouse')) pointers.delete(event.id);
            else pointers.set(event.id, { id: event.id, device: event.device || 'mouse', x: event.x, y: event.y, down: event.type === 'down' || (event.type === 'move' && !!(pointers.get(event.id) || {}).down) });
          }
        },
        step: (t) => on_screen(t === undefined ? performance.now() : t),
        get stopped() {
          return stopped;
        },
        vr_supported: () => (navigator.xr ? can_enter() : Promise.resolve(false)),
      };
    }
  }

  return { start };
})();
