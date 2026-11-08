//! `crow build --target web` (spec, Building for the web): the build
//! refuses what a page in a browser cannot run, writes the page, and the
//! page draws what the program paints — checked in a headless Chrome where
//! one is installed (`CROW_CHROME` names one; else the usual places).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("crow_web_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn crow(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_crow")).args(args).current_dir(dir).output().unwrap()
}

/// Whether this toolchain has the web target's standard library.
fn has_target() -> bool {
    Command::new("rustc")
        .args(["--print", "target-libdir", "--target", "wasm32-unknown-unknown"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|d| Path::new(d.trim()).join("libcore").exists() || std::fs::read_dir(d.trim()).is_ok_and(|mut r| r.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with("libcore")))))
}

fn chrome() -> Option<PathBuf> {
    if let Ok(c) = std::env::var("CROW_CHROME") {
        return Some(PathBuf::from(c));
    }
    ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/google-chrome-stable", "/usr/bin/chromium", "/usr/bin/chromium-browser"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// The title the page at `port` leaves once it has run, from the DOM a
/// headless Chrome dumps (WebGL2 through SwiftShader, a software renderer
/// on every machine), or what Chrome said where it showed no page. On
/// Linux Chrome runs without its sandbox: Ubuntu 24.04, GitHub's runner,
/// restricts the unprivileged user namespaces the sandbox is made of, and
/// the page is the test's own, served from the test's own directory.
fn title_after(chrome: PathBuf, dir: &Path, port: u16) -> String {
    let mut c = Command::new(chrome);
    c.args(["--headless=new", "--no-first-run", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--virtual-time-budget=5000", "--window-size=240,240", "--dump-dom"]);
    if cfg!(target_os = "linux") {
        c.arg("--no-sandbox");
    }
    let out = c.arg(format!("--user-data-dir={}", dir.join("profile").display())).arg(format!("http://127.0.0.1:{}/check.html", port)).output().unwrap();
    let dom = String::from_utf8_lossy(&out.stdout);
    match dom.split("<title>").nth(1).and_then(|t| t.split("</title>").next()) {
        Some(title) => title.to_string(),
        None => format!("no page ({}): {}", out.status, String::from_utf8_lossy(&out.stderr).trim()),
    }
}

#[test]
fn a_program_reaching_a_file_is_refused_for_the_web_naming_what_it_reaches() {
    let dir = scratch("refused");
    std::fs::write(dir.join("reads.co"), "text = read['notes.txt']\nlog[text]\n").unwrap();
    let out = crow(&dir, &["build", "--target", "web", "reads.co"]);
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(said.contains("a page in a browser cannot run what this program reaches"), "{}", said);
    assert!(said.contains("read[] reads a file"), "{}", said);
    assert!(!dir.join("reads_web").exists());
}

#[test]
fn a_target_other_than_the_web_is_refused_with_the_form() {
    let dir = scratch("other");
    std::fs::write(dir.join("p.co"), "log[1]\n").unwrap();
    let out = crow(&dir, &["build", "--target", "ios", "p.co"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("crow build --target takes `web`"));
}

const TRIANGLE: &str = "use sys

vs = '#version 300 es
in vec2 at;
void main() { gl_Position = vec4(at, 0.0, 1.0); }'
fs = '#version 300 es
precision mediump float;
out vec4 color;
void main() { color = vec4(1.0, 0.5, 0.0, 1.0); }'
sys.paint[encode[[['#1', 'createShader', 'VERTEX_SHADER'], ['shaderSource', '#1', '=`vs`'], ['compileShader', '#1'], ['#2', 'createShader', 'FRAGMENT_SHADER'], ['shaderSource', '#2', '=`fs`'], ['compileShader', '#2'], ['#3', 'createProgram'], ['attachShader', '#3', '#1'], ['attachShader', '#3', '#2'], ['linkProgram', '#3'], ['#4', 'createBuffer'], ['bindBuffer', 'ARRAY_BUFFER', '#4'], ['bufferData', 'ARRAY_BUFFER', 'f32:0 0.9 -0.9 -0.9 0.9 -0.9', 'STATIC_DRAW']]]]

fn draw[frame]
    v = frame.views[0]
    calls = [['viewport', str[v.x], str[v.y], str[v.width], str[v.height]], ['clearColor', '0', '0', '1', '1'], ['clear', 'COLOR_BUFFER_BIT'], ['useProgram', '#3'], ['bindBuffer', 'ARRAY_BUFFER', '#4'], ['enableVertexAttribArray', '0'], ['vertexAttribPointer', '0', '2', 'FLOAT', 'false', '0', '0'], ['drawArrays', 'TRIANGLES', '0', '3']]
    sys.paint[encode[calls]]
    --- (a frame's function may answer a value, which nothing reads: a
    --- count here, which had not built compiled)
    len[calls]

log['top level ran']
sys.frames[draw]
";

/// The page's check: two frames drawn, then the pixels at the centre
/// (inside the triangle) and at a corner (outside), and what the program
/// logged, in the title.
const CHECK: &str = r#"<!DOCTYPE html><html><head><title>waiting</title></head><body style="margin:0">
<canvas id="c" style="width:200px;height:200px"></canvas><script src="crow.js"></script><script>
const logged = [];
Crow.start({ canvas: document.getElementById('c'), module: 'tri.wasm', keep: true, said: (k, t) => logged.push(k + ':' + t) }).then((page) => {
  setTimeout(() => {
    const gl = page.gl, px = new Uint8Array(4), at = (x, y) => { gl.readPixels(x, y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px); return Array.from(px).join(','); };
    document.title = 'centre ' + at(gl.drawingBufferWidth >> 1, gl.drawingBufferHeight >> 1) + ' corner ' + at(1, gl.drawingBufferHeight - 2) + ' said ' + logged.join('|');
  }, 1500);
}).catch((e) => { document.title = 'failed ' + e.message; });
</script></body></html>"#;

/// A static server for `dir` on a port of its own, for as long as the test.
fn serve(dir: PathBuf) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let path = head.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_string();
                let path = if path.is_empty() { "index.html".to_string() } else { path };
                let (status, body) = match std::fs::read(dir.join(&path)) {
                    Ok(b) => ("200 OK", b),
                    Err(_) => ("404 Not Found", Vec::new()),
                };
                let kind = if path.ends_with(".wasm") { "application/wasm" } else if path.ends_with(".js") { "text/javascript" } else { "text/html" };
                let _ = write!(stream, "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", status, kind, body.len());
                let _ = stream.write_all(&body);
            });
        }
    });
    port
}

#[test]
fn a_program_built_for_the_web_draws_what_it_paints() {
    if !has_target() {
        eprintln!("skipped: no standard library for wasm32-unknown-unknown (rustup target add wasm32-unknown-unknown)");
        return;
    }
    let dir = scratch("draws");
    std::fs::write(dir.join("tri.co"), TRIANGLE).unwrap();
    // (both modes run it, and draw nothing: no page)
    let interpreted = crow(&dir, &["run", "-i", "tri.co"]);
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), "top level ran\n");
    let out = crow(&dir, &["build", "--target", "web", "tri.co"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let page = dir.join("tri_web");
    let module = std::fs::read(page.join("tri.wasm")).unwrap();
    assert_eq!(&module[..4], b"\0asm");
    assert!(std::fs::read_to_string(page.join("index.html")).unwrap().contains("tri.wasm"));
    assert!(std::fs::read_to_string(page.join("crow.js")).unwrap().contains("crow_frame"));
    let Some(chrome) = chrome() else {
        eprintln!("skipped the drawing: no Chrome (CROW_CHROME names one)");
        return;
    };
    std::fs::write(page.join("check.html"), CHECK).unwrap();
    let port = serve(page);
    let title = title_after(chrome, &dir, port);
    assert!(title.starts_with("centre 255,128,0,255 corner 0,0,255,255"), "{}", title);
    assert!(title.contains("log:top level ran"), "{}", title);
    let _ = std::fs::remove_dir_all(&dir);
}

const POINTER: &str = "use sys

fn draw[frame]
    for p in frame.pointers
        log['pointer `p.x` `p.y` `p.down`']
    len[frame.pointers]

sys.frames[draw]
";

/// A test's page drives the frames itself (`manual`) and hands the page a
/// pointer's events (`give`): a frame then says where the pointer is, as
/// it does for the reader's mouse — `give` had queued the event alone, and
/// a frame's `pointers` stayed empty (Lory's crosshair and hover never
/// showed under a test). A move after a press keeps the button held.
const POINTER_CHECK: &str = r#"<!DOCTYPE html><html><head><title>waiting</title></head><body style="margin:0">
<canvas id="c" style="width:200px;height:200px"></canvas><script src="crow.js"></script><script>
const logged = [];
Crow.start({ canvas: document.getElementById('c'), module: 'pointer.wasm', manual: true, said: (k, t) => logged.push(t) }).then((page) => {
  const at = (type, x, y) => page.give({ type, device: 'mouse', id: 1, x, y, button: 0, shift: false, ctrl: false, alt: false, meta: false });
  page.step(16);
  at('move', 40, 50); page.step(32);
  at('down', 41, 51); page.step(48);
  at('move', 60, 70); page.step(64);
  at('up', 60, 70); page.step(80);
  document.title = 'said ' + logged.join('|');
}).catch((e) => { document.title = 'failed ' + e.message; });
</script></body></html>"#;

#[test]
fn a_pointer_a_test_gives_is_where_the_next_frame_says_it_is() {
    if !has_target() {
        eprintln!("skipped: no standard library for wasm32-unknown-unknown");
        return;
    }
    let dir = scratch("pointer");
    std::fs::write(dir.join("pointer.co"), POINTER).unwrap();
    let out = crow(&dir, &["build", "--target", "web", "pointer.co"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let Some(chrome) = chrome() else {
        eprintln!("skipped the drawing: no Chrome (CROW_CHROME names one)");
        return;
    };
    let page = dir.join("pointer_web");
    std::fs::write(page.join("check.html"), POINTER_CHECK).unwrap();
    let port = serve(page);
    let title = title_after(chrome, &dir, port);
    assert_eq!(title, "said pointer 40 50 false|pointer 41 51 true|pointer 60 70 true", "{}", title);
    let _ = std::fs::remove_dir_all(&dir);
}

const VR: &str = "use sys

fn draw[frame]
    log['vr `frame.vr`']
    if frame.t gt 20 and frame.t lt 40
        sys.paint['[[\"@vr\"]]']
    for s in frame.said
        log['said `s.what`']
    0

sys.frames[draw]
";

/// A frame says whether a headset can be entered (`vr`), and `@vr` enters
/// it — a page draws its own button for that, in WebGL, as Lory's controls
/// are; where none can be entered (headless Chrome has no headset) the
/// failure is said in a later frame, never silent.
const VR_CHECK: &str = r#"<!DOCTYPE html><html><head><title>waiting</title></head><body style="margin:0">
<canvas id="c" style="width:200px;height:200px"></canvas><script src="crow.js"></script><script>
const logged = [];
Crow.start({ canvas: document.getElementById('c'), module: 'vr.wasm', manual: true, said: (k, t) => logged.push(t) }).then((page) => {
  page.step(16);
  page.step(32);
  setTimeout(() => {
    page.step(48);
    document.title = 'said ' + logged.join('|');
  }, 300);
}).catch((e) => { document.title = 'failed ' + e.message; });
</script></body></html>"#;

#[test]
fn a_frame_says_whether_a_headset_can_be_entered_and_entering_none_is_said() {
    if !has_target() {
        eprintln!("skipped: no standard library for wasm32-unknown-unknown");
        return;
    }
    let dir = scratch("vr");
    std::fs::write(dir.join("vr.co"), VR).unwrap();
    let out = crow(&dir, &["build", "--target", "web", "vr.co"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let Some(chrome) = chrome() else {
        eprintln!("skipped the drawing: no Chrome (CROW_CHROME names one)");
        return;
    };
    let page = dir.join("vr_web");
    std::fs::write(page.join("check.html"), VR_CHECK).unwrap();
    let port = serve(page);
    let title = title_after(chrome, &dir, port);
    assert_eq!(title, "said vr false|vr false|vr false|said vr", "{}", title);
}
