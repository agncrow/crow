//! The world a compiled program runs in, from one place: the system's
//! standard streams and clock natively, and the page's in a browser (the
//! web target, spec "Building for the web"). A program built for the web
//! is WebAssembly with no system under it: what it writes goes to the
//! page's console, the time is the page's, random bytes come from the
//! page's `crypto`, and a stop ends the program where the page can say so.
//! The page is `crow.js` (written by `crow build --target web`): it
//! provides the four functions of the `crow` import module below and
//! calls the exports `crow_start` and `crow_frame`.

/// Text a program writes as its output: stdout natively, the page's
/// console in a browser.
pub fn write_out(bytes: &[u8]) {
    #[cfg(target_arch = "wasm32")]
    page::write(1, bytes);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(bytes);
        let _ = out.flush();
    }
}

/// Text a program writes about itself (a stop's reason): stderr natively,
/// the page's console as an error in a browser.
pub fn write_err(bytes: &[u8]) {
    #[cfg(target_arch = "wasm32")]
    page::write(2, bytes);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::Write;
        let mut err = std::io::stderr().lock();
        let _ = err.write_all(bytes);
        let _ = err.flush();
    }
}

/// Seconds since 1970 by the system's clock, or the page's.
pub fn clock() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        page::now() / 1000.0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
    }
}

/// A moment to measure a piece of work from: `Instant` natively. In a
/// browser, where `Instant` has no clock under it, every elapsed time is
/// none, so the work it times is never shared across threads a page does
/// not have (`cores::worth_sharing`).
pub struct Started(#[cfg(not(target_arch = "wasm32"))] std::time::Instant);

impl Started {
    pub fn now() -> Started {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Started(std::time::Instant::now())
        }
        #[cfg(target_arch = "wasm32")]
        {
            Started()
        }
    }

    pub fn elapsed(&self) -> std::time::Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            std::time::Duration::ZERO
        }
    }
}

/// The program stops: what it logged written out, then the reason, then
/// the end — exit status 1 natively; in a browser the WebAssembly traps,
/// and the page shows the reason and that the program stopped.
pub fn stopped(text: &str) -> ! {
    crate::display::flush_output();
    write_err(format!("runtime error: {}\n", text).as_bytes());
    #[cfg(target_arch = "wasm32")]
    std::process::abort();
    #[cfg(not(target_arch = "wasm32"))]
    std::process::exit(1)
}

/// The page's four functions, imported from the module `crow` that
/// `crow.js` hands the WebAssembly as it starts.
#[cfg(target_arch = "wasm32")]
pub mod page {
    #[link(wasm_import_module = "crow")]
    unsafe extern "C" {
        #[link_name = "write"]
        fn page_write(stream: u32, ptr: *const u8, len: usize);
        #[link_name = "now"]
        fn page_now() -> f64;
        #[link_name = "random"]
        fn page_random(ptr: *mut u8, len: usize);
        #[link_name = "paint"]
        fn page_paint(ptr: *const u8, len: usize);
    }

    pub fn write(stream: u32, bytes: &[u8]) {
        // SAFETY: the page reads `len` bytes at `ptr` in this module's
        // memory before the call returns, and keeps no reference to them
        unsafe { page_write(stream, bytes.as_ptr(), bytes.len()) }
    }

    pub fn now() -> f64 {
        // SAFETY: the page answers `Date.now()`, touching no memory
        unsafe { page_now() }
    }

    pub fn random(into: &mut [u8]) {
        // SAFETY: the page writes exactly `len` bytes at `ptr` before the
        // call returns
        unsafe { page_random(into.as_mut_ptr(), into.len()) }
    }

    pub fn paint(batch: &[u8]) {
        // SAFETY: as `write`: read before the call returns, then let go
        unsafe { page_paint(batch.as_ptr(), batch.len()) }
    }

    /// getrandom's source in a browser (the generated project builds with
    /// `--cfg getrandom_backend="custom"`): the page's `crypto`. Both
    /// getrandom versions the runtime's crates use read this one symbol.
    ///
    /// # Safety
    /// getrandom hands a buffer of `len` writable bytes at `dest`.
    #[unsafe(no_mangle)]
    unsafe extern "Rust" fn __getrandom_v03_custom(dest: *mut u8, len: usize) -> Result<(), getrandom::Error> {
        // SAFETY: getrandom's contract, above
        let into = unsafe { std::slice::from_raw_parts_mut(dest, len) };
        random(into);
        Ok(())
    }
}
