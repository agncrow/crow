//! The page a program built for the web draws on (spec, "Building for the
//! web"): `sys.paint[batch]` hands the page a batch of WebGL2 calls, and
//! `sys.frames[f]` asks the page to call `f[frame]` before each frame it
//! shows. Outside a browser — `crow run`, `crow run -i`, a test — there is
//! no page: a batch draws nothing and no frame comes, in both modes alike,
//! so what a program draws is tested as the batches it builds.
//!
//! A batch is JSON text: a list of calls, each a list whose first item is
//! a WebGL2 function's name and the rest its arguments (`crow.js` reads
//! the forms an argument may take). The page runs the calls in order on
//! its WebGL2 context, then shows the frame.

use std::sync::Mutex;

/// The work the page calls before each frame, the last one asked for.
type Work = Box<dyn Fn(serde_json::Value) + Send>;
static FRAME: Mutex<Option<Work>> = Mutex::new(None);

/// What a frame's function answers, which nothing reads: any value, as
/// `later`'s work answers — but a failure (compiled, the program's
/// `CrowResult` that did not hold) stops the program with its line and
/// reason, as a failure dropped at the top level does.
pub trait Done {
    fn done(self);
}

impl<T: crate::display::CrowDisplay> Done for T {
    fn done(self) {
        if let Some(said) = self.failure() {
            failed_frame(&said)
        }
    }
}

/// A frame's function failed: the program stops, naming where.
pub fn failed_frame(said: &str) -> ! {
    crate::host::stopped(&format!("a frame's work failed: {}", said.trim_start_matches("error: ")))
}

/// `sys.paint[batch]`: the batch handed to the page, which runs its calls
/// now; outside a browser, nothing.
pub fn paint(batch: &str) {
    #[cfg(target_arch = "wasm32")]
    crate::host::page::paint(batch.as_bytes());
    #[cfg(not(target_arch = "wasm32"))]
    let _ = batch;
}

/// `sys.frames[f]`: the page calls `f[frame]` before each frame it shows,
/// from when the program's top level has run; `f` replaces a function
/// asked for before it. Outside a browser no frame comes.
pub fn frames<R: Done + 'static>(f: fn(serde_json::Value) -> R) {
    *FRAME.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(move |v| f(v).done()));
}

/// Whether a program asked for frames (the page stops asking for frames
/// when it did not).
pub fn wants_frames() -> bool {
    FRAME.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// One frame: the function `frames` was given, called with what the page
/// says of the frame (its JSON text). A frame that is no JSON is refused
/// in words: the page wrote it, so it is a fault of the page.
pub fn frame(text: &str) {
    // (the work taken out while it runs: a frame may ask for other work,
    // which then replaces it)
    let Some(f) = FRAME.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    match serde_json::from_str(text) {
        Ok(v) => f(v),
        Err(e) => crate::host::stopped(&format!("the page described a frame that is no JSON: {}", e)),
    }
    let mut held = FRAME.lock().unwrap_or_else(|e| e.into_inner());
    if held.is_none() {
        *held = Some(f);
    }
}

/// Room in the program's memory for the page to write `len` bytes into
/// (a frame's description), handed back to `crow_frame`.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn crow_alloc(len: usize) -> *mut u8 {
    let mut room = std::mem::ManuallyDrop::new(Vec::<u8>::with_capacity(len));
    room.as_mut_ptr()
}

/// The page's call before each frame: `len` bytes of JSON at `ptr`, room
/// `crow_alloc` made, taken back here. Answers whether frames are still
/// wanted.
///
/// # Safety
/// `ptr` is what `crow_alloc(len)` answered, and the page wrote `len`
/// bytes there.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crow_frame(ptr: *mut u8, len: usize) -> u32 {
    // SAFETY: the contract above: crow_alloc's room, `len` bytes written
    let bytes = unsafe { Vec::from_raw_parts(ptr, len, len) };
    let text = String::from_utf8_lossy(&bytes);
    frame(&text);
    wants_frames() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    static CALLED: Mutex<Vec<f64>> = Mutex::new(Vec::new());

    fn note(v: serde_json::Value) {
        CALLED.lock().unwrap_or_else(|e| e.into_inner()).push(v["t"].as_f64().unwrap_or(-1.0));
    }

    static SWAPPED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

    fn second(_: serde_json::Value) {
        SWAPPED.lock().unwrap_or_else(|e| e.into_inner()).push("second");
    }

    fn first(_: serde_json::Value) {
        SWAPPED.lock().unwrap_or_else(|e| e.into_inner()).push("first");
        frames(second);
    }

    #[test]
    fn a_frame_may_ask_for_other_work_which_runs_from_the_next_frame() {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        frames(first);
        frame("{}");
        frame("{}");
        frame("{}");
        assert_eq!(*SWAPPED.lock().unwrap_or_else(|e| e.into_inner()), vec!["first", "second", "second"]);
    }

    /// (the frame's work is the process's: one test at a time)
    static ONE: Mutex<()> = Mutex::new(());

    #[test]
    fn a_frame_calls_the_function_asked_for_with_what_the_page_said() {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        paint("[[\"clearColor\", 0, 0, 0, 1]]");
        frames(note);
        assert!(wants_frames());
        frame("{\"t\": 1.5}");
        frame("{\"t\": 2.5}");
        assert_eq!(*CALLED.lock().unwrap_or_else(|e| e.into_inner()), vec![1.5, 2.5]);
    }
}
