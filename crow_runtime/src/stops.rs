//! The failures no value can carry, checked once. An index outside its
//! array, a key its map lacks, an int divided by zero or grown past 64
//! bits: the program cannot go on, so it stops with the line and these
//! words, the same in both modes (spec, When a Program Stops). The
//! interpreter's operators call these checks and stop with the reason; the
//! compiled program calls the same checks and stops through [`halt`].
//! Neither leans on a Rust panic to say what went wrong: a panic is a
//! check Crow is missing, and [`panic_words`] only names it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

// ── the call's line ─────────────────────────────────────────────────

thread_local! {
    static CALL_LINE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The line of the program a standard module's function (one written in
/// Crow, inlined into the program) was called from: the compiled program
/// notes it before the call, and a stop inside the function names it,
/// as the interpreter's does, never a line of the module's own source.
pub fn at_line(line: usize) {
    CALL_LINE.with(|l| l.set(line));
}

/// The line noted by [`at_line`].
pub fn call_line() -> usize {
    CALL_LINE.with(|l| l.get())
}

// ── indexes ─────────────────────────────────────────────────────────

/// Where index `i` reads in a sequence of `len`: a negative index counts
/// from the end, `-1` the last.
#[inline]
fn resolve(i: i64, len: usize) -> Option<usize> {
    let n = i64::try_from(len).ok()?;
    // n ≥ 0 and i < 0: the sum cannot overflow
    let at = if i < 0 { n + i } else { i };
    if (0..n).contains(&at) { usize::try_from(at).ok() } else { None }
}

/// `3 items`, `1 item`.
fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {}", one) } else { format!("{} {}", n, many) }
}

/// `xs[i]` of an array — a list, a flat tensor, a frame's column: the
/// position it reads.
#[inline(always)]
pub fn item(i: i64, len: usize) -> Result<usize, String> {
    // the common case first — a position in range — as one compare (a
    // negative index is a large unsigned one, so it takes the slow path);
    // the compiled fannkuch spent a third of its time in the slow one
    if (i as u64) < (len as u64) {
        return Ok(i as usize);
    }
    item_slow(i, len)
}

#[cold]
#[inline(never)]
fn item_slow(i: i64, len: usize) -> Result<usize, String> {
    resolve(i, len).ok_or_else(|| format!("index {} is out of range for an array of {}", i, count(len, "item", "items")))
}

/// `xs[i]`'s position in a list of `len` items, or the program stops on
/// `line` (an index past the end, or before the start counted from it):
/// pure arithmetic on every path that answers, the stop never returning,
/// so an optimizer hoists a loop's reads of a list it does not write. A
/// check whose slow path built its message as a value (`item`) is a call
/// that may write anything, and every read after it was made again: the
/// many-body n-body had run at half Rust's speed.
#[inline(always)]
pub fn at(i: i64, len: usize, line: usize) -> usize {
    if (i as u64) < (len as u64) {
        return i as usize;
    }
    match resolve(i, len) {
        Some(k) => k,
        None => out_of_range(i, len, line),
    }
}

/// `at` inside a standard module's function written in Crow: the stop
/// names the line of the call that reached it, read only when it stops.
#[inline(always)]
pub fn at_call(i: i64, len: usize) -> usize {
    if (i as u64) < (len as u64) {
        return i as usize;
    }
    match resolve(i, len) {
        Some(k) => k,
        None => out_of_range(i, len, call_line()),
    }
}

#[cold]
#[inline(never)]
fn out_of_range(i: i64, len: usize, line: usize) -> ! {
    halt(line, &format!("index {} is out of range for an array of {}", i, count(len, "item", "items")))
}

/// `s[i]` of a string, counted in characters.
#[inline(always)]
pub fn char_at(i: i64, len: usize) -> Result<usize, String> {
    if (i as u64) < (len as u64) {
        return Ok(i as usize);
    }
    char_at_slow(i, len)
}

#[cold]
#[inline(never)]
fn char_at_slow(i: i64, len: usize) -> Result<usize, String> {
    resolve(i, len).ok_or_else(|| format!("index {} is out of range for a string of {}", i, count(len, "character", "characters")))
}

/// `m[i]` of a tensor of two or more dimensions: the row it reads.
pub fn row(i: i64, rows: usize) -> Result<usize, String> {
    resolve(i, rows).ok_or_else(|| format!("index {} is out of range for a tensor of {}", i, count(rows, "row", "rows")))
}

/// `m[k]`, `m.k`: the value under a key.
pub fn key<'a, K: Ord + std::fmt::Display, T>(m: &'a BTreeMap<K, T>, k: &K) -> Result<&'a T, String> {
    m.get(k).ok_or_else(|| no_key(k))
}

/// A map without the key a read or a write through it names.
pub fn no_key(k: &dyn std::fmt::Display) -> String {
    format!("map has no key '{}'", k)
}

/// `df.col` of a frame without that column.
pub fn no_column(name: &str) -> String {
    format!("frame has no property or column '{}'", name)
}

/// A frame's column handed where numbers are taken, with a missing cell
/// (spec: a missing cell is an error, not a placeholder).
pub fn missing_cell(row: usize) -> String {
    format!("row {} of the column is missing — fill the column first: fill[frame, 'column', value]", row)
}

/// A frame's column of text or of bools handed where numbers are taken.
pub fn not_numbers(what: &str) -> String {
    format!("a column of {} is not numbers", what)
}

// ── ints ────────────────────────────────────────────────────────────

fn overflow(a: i64, op: &str, b: i64) -> String {
    format!("integer overflow: {} {} {} does not fit in 64 bits", a, op, b)
}

/// `a + b` of ints.
#[inline]
pub fn add(a: i64, b: i64) -> Result<i64, String> {
    a.checked_add(b).ok_or_else(|| overflow(a, "+", b))
}

/// `a - b` of ints.
#[inline]
pub fn sub(a: i64, b: i64) -> Result<i64, String> {
    a.checked_sub(b).ok_or_else(|| overflow(a, "-", b))
}

/// `a * b` of ints.
#[inline]
pub fn mul(a: i64, b: i64) -> Result<i64, String> {
    a.checked_mul(b).ok_or_else(|| overflow(a, "*", b))
}

/// `a / b` of ints: the quotient, rounded toward zero.
#[inline]
pub fn div(a: i64, b: i64) -> Result<i64, String> {
    if b == 0 {
        return Err("division by zero".to_string());
    }
    a.checked_div(b).ok_or_else(|| overflow(a, "/", b))
}

/// `a % b` of ints: the remainder, with the sign of `a`.
#[inline]
pub fn rem(a: i64, b: i64) -> Result<i64, String> {
    if b == 0 {
        return Err("modulo by zero".to_string());
    }
    // exact: the one pair that wraps, the smallest int % -1, has remainder 0
    Ok(a.wrapping_rem(b))
}

/// `a ^ b` of ints. A negative power of an int is a fraction, which an int
/// does not hold: the program says so rather than rounding it away.
pub fn pow(a: i64, b: i64) -> Result<i64, String> {
    if b < 0 {
        return Err(format!("{} ^ {}: an int to a negative power is a fraction — write {}.0 ^ {} for a float", a, b, a, b));
    }
    match u32::try_from(b) {
        Ok(e) => a.checked_pow(e).ok_or_else(|| overflow(a, "^", b)),
        // a power past u32's range: only 0, 1 and -1 stay in range
        Err(_) => match a {
            0 | 1 => Ok(a),
            -1 => Ok(if b % 2 == 0 { 1 } else { -1 }),
            _ => Err(overflow(a, "^", b)),
        },
    }
}

/// `-a` of an int.
#[inline]
pub fn neg(a: i64) -> Result<i64, String> {
    a.checked_neg().ok_or_else(|| format!("integer overflow: -({}) does not fit in 64 bits", a))
}

/// `abs[a]` of an int.
#[inline]
pub fn abs(a: i64) -> Result<i64, String> {
    a.checked_abs().ok_or_else(|| format!("integer overflow: abs[{}] does not fit in 64 bits", a))
}

/// `sum` of ints.
pub fn sum(xs: &[i64]) -> Result<i64, String> {
    xs.iter().try_fold(0i64, |acc, &x| add(acc, x))
}

/// `prod` of ints.
pub fn prod(xs: &[i64]) -> Result<i64, String> {
    xs.iter().try_fold(1i64, |acc, &x| mul(acc, x))
}

// ── sizes ───────────────────────────────────────────────────────────

/// A size the program gives a constructor (`zeros[n]`, `eye[n]`,
/// `repeat[s, n]`): a count, never negative.
pub fn size(what: &str, n: i64) -> Result<usize, String> {
    usize::try_from(n).map_err(|_| format!("{}: a size cannot be negative (got {})", what, n))
}

// ── recursion ───────────────────────────────────────────────────────

/// How deep a recursive function's calls may go before the program stops,
/// the same in both modes (each runs on a stack that holds it: [`STACK`]).
pub const DEPTH: usize = 10_000;

/// The stack a program's thread runs on: deep enough for [`DEPTH`] calls
/// in the interpreter, whose calls are frames of its own.
pub const STACK: usize = 256 << 20;

/// A recursion past [`DEPTH`]: the reason, where both modes had aborted
/// with Rust's stack overflow.
pub fn too_deep() -> String {
    format!("recursion deeper than {} calls — a function that calls itself needs a case that returns without calling", DEPTH)
}

thread_local! {
    static CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A recursive function's call on this thread's stack: entered when the
/// function starts, left when the guard drops.
pub struct Depth;

impl Depth {
    pub fn enter() -> Result<Depth, String> {
        CALLS.with(|c| {
            let n = c.get() + 1;
            if n > DEPTH {
                return Err(too_deep());
            }
            c.set(n);
            Ok(Depth)
        })
    }
}

impl Drop for Depth {
    fn drop(&mut self) {
        CALLS.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

// ── stopping ────────────────────────────────────────────────────────

/// A stop raised off the main thread, carried as the panic's payload to
/// whoever joins that thread: a parallel loop hands it to its caller and
/// the main thread's guard prints it as the program's stop; the live
/// runtime reads it as the handler's.
pub struct Stop(pub String);

/// A stop in a compiled program: the line and the reason on stderr, exit
/// status 1, as the interpreter stops. Off the main thread it travels as
/// a [`Stop`] to the thread that joins it, so a parallel loop reports the
/// stop of its earliest failing item, once, as the interpreter does; on a
/// server's thread the live runtime contains it and the handler stops
/// with nothing it did landing.
#[allow(clippy::panic, reason = "a stop off the main thread travels as a panic to the thread that joins it")]
pub fn halt(line: usize, why: &str) -> ! {
    let text = if line > 0 { format!("line {}: {}", line, why) } else { why.to_string() };
    if std::thread::current().name() == Some("main") {
        crate::display::flush_output();
        eprintln!("runtime error: {}", text);
        std::process::exit(1);
    }
    std::panic::panic_any(Stop(text))
}

/// A compiled program's main, run so that a stop carried back from another
/// thread is the program's stop, printed once. A panic that is not a stop
/// printed itself where it happened, through [`install_hook`].
pub fn main_guard(f: impl FnOnce() + Send + 'static) {
    // on a thread of its own, named for the main thread (so a stop on it
    // exits as the main thread's does), with a stack deep enough for the
    // recursion limit
    let run = move || {
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        // (what the program logged, written before what stopped it)
        crate::display::flush_output();
        if let Err(p) = ran {
            if let Some(Stop(text)) = p.downcast_ref::<Stop>() {
                eprintln!("runtime error: {}", text);
            }
            std::process::exit(1);
        }
    };
    match std::thread::Builder::new().name("main".to_string()).stack_size(STACK).spawn(run) {
        Ok(h) => {
            if h.join().is_err() {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("runtime error: cannot start the program's thread: {}", e);
            std::process::exit(1);
        }
    }
}

/// How many guarded runs are under way: while one is, a panic prints
/// nothing of its own, because the guard reports it as a stop.
static GUARDS: AtomicUsize = AtomicUsize::new(0);

/// Run `f`, and a panic inside it — a check Crow is missing — comes back as
/// the words it stops with instead of unwinding past the caller.
pub fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    GUARDS.fetch_add(1, Ordering::SeqCst);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    GUARDS.fetch_sub(1, Ordering::SeqCst);
    r.map_err(|p| panic_words(p.as_ref()))
}

/// The panic hook both modes install. A stop, or any panic under a guard,
/// prints nothing here: the thread that catches it reports it. Any other
/// panic prints its words where it happened, with the location only when
/// `RUST_BACKTRACE` asks for detail.
pub fn install_hook() {
    std::panic::set_hook(Box::new(|info| {
        if GUARDS.load(Ordering::SeqCst) > 0 || info.payload().downcast_ref::<Stop>().is_some() {
            return;
        }
        crate::display::flush_output();
        let words = panic_words(info.payload());
        match (std::env::var_os("RUST_BACKTRACE"), info.location()) {
            (Some(_), Some(at)) => eprintln!("runtime error: {} (at {}:{})", words, at.file(), at.line()),
            _ => eprintln!("runtime error: {}", words),
        }
    }));
}

/// For a program that runs Crow code beside its own, the interpreter: a
/// panic under a guard prints nothing, because the guard reports it as the
/// program's stop; any other panic is reported as it was before.
pub fn quiet_under_guards() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if GUARDS.load(Ordering::SeqCst) > 0 {
            return;
        }
        prev(info)
    }));
}

/// What a panic said, in Crow's words where Rust's are about Rust: the
/// same text in both modes for the same missing check.
pub fn panic_words(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(Stop(text)) = payload.downcast_ref::<Stop>() {
        return text.clone();
    }
    let msg = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "the program panicked".to_string());
    if let Some(rest) = msg.strip_prefix("index out of bounds: the len is ") {
        if let Some((len, i)) = rest.split_once(" but the index is ") {
            return format!("index {} is out of range for an array of {} items", i.trim(), len.trim());
        }
    }
    if msg.contains("divide by zero") {
        return "division by zero".to_string();
    }
    if msg.contains("remainder with a divisor of zero") {
        return "modulo by zero".to_string();
    }
    if let Some(op) = msg.strip_prefix("attempt to ").and_then(|m| m.strip_suffix(" with overflow")) {
        let what = match op {
            "add" => "an addition",
            "subtract" => "a subtraction",
            "multiply" => "a multiplication",
            "divide" => "a division",
            "negate" => "a negation",
            "calculate the remainder" => "a remainder",
            other => other,
        };
        return format!("integer overflow in {}", what);
    }
    if msg.contains("capacity overflow") {
        return "a size too large to hold".to_string();
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_counts_from_either_end() {
        assert_eq!(item(0, 3), Ok(0));
        assert_eq!(item(-1, 3), Ok(2));
        assert_eq!(item(-3, 3), Ok(0));
        assert_eq!(item(3, 3), Err("index 3 is out of range for an array of 3 items".to_string()));
        assert_eq!(item(-4, 3), Err("index -4 is out of range for an array of 3 items".to_string()));
        assert_eq!(item(0, 0), Err("index 0 is out of range for an array of 0 items".to_string()));
        assert_eq!(char_at(5, 1), Err("index 5 is out of range for a string of 1 character".to_string()));
    }

    /// The compiled program's index answers what `item` answers, and stops
    /// with its words on the line it is given (off the main thread a stop
    /// travels as a panic, which the test catches)
    #[test]
    fn a_compiled_index_is_items_answer() {
        assert_eq!(at(0, 3, 7), 0);
        assert_eq!(at(-1, 3, 7), 2);
        assert_eq!(at(-3, 3, 7), 0);
        let stopped = |f: fn() -> usize| match std::panic::catch_unwind(f) {
            Err(p) => p.downcast_ref::<Stop>().map(|s| s.0.clone()),
            Ok(_) => None,
        };
        assert_eq!(stopped(|| at(3, 3, 7)).as_deref(), Some("line 7: index 3 is out of range for an array of 3 items"));
        assert_eq!(stopped(|| at(-4, 3, 7)).as_deref(), Some("line 7: index -4 is out of range for an array of 3 items"));
        assert_eq!(stopped(|| at_call(0, 0)).as_deref(), Some("index 0 is out of range for an array of 0 items"));
        assert_eq!(row(i64::MIN, 2), Err(format!("index {} is out of range for a tensor of 2 rows", i64::MIN)));
    }

    #[test]
    fn a_missing_key_is_named() {
        let mut m = BTreeMap::new();
        m.insert("a".to_string(), 1);
        assert_eq!(key(&m, &"a".to_string()), Ok(&1));
        assert_eq!(key(&m, &"b".to_string()), Err("map has no key 'b'".to_string()));
    }

    #[test]
    fn an_int_past_64_bits_is_refused_never_wrapped() {
        assert_eq!(add(i64::MAX, 1), Err(format!("integer overflow: {} + 1 does not fit in 64 bits", i64::MAX)));
        assert_eq!(sub(i64::MIN, 1).is_err(), true);
        assert_eq!(mul(i64::MAX, 2).is_err(), true);
        assert_eq!(div(i64::MIN, -1).is_err(), true);
        assert_eq!(neg(i64::MIN).is_err(), true);
        assert_eq!(abs(i64::MIN).is_err(), true);
        assert_eq!(add(2, 3), Ok(5));
        assert_eq!(sum(&[i64::MAX, 1]).is_err(), true);
        assert_eq!(prod(&[2, 3, 4]), Ok(24));
    }

    #[test]
    fn division_rounds_toward_zero_and_refuses_zero() {
        assert_eq!(div(7, 2), Ok(3));
        assert_eq!(div(-7, 2), Ok(-3));
        assert_eq!(rem(-7, 2), Ok(-1));
        assert_eq!(rem(i64::MIN, -1), Ok(0));
        assert_eq!(div(1, 0), Err("division by zero".to_string()));
        assert_eq!(rem(1, 0), Err("modulo by zero".to_string()));
    }

    #[test]
    fn a_power_is_exact_or_refused() {
        assert_eq!(pow(2, 10), Ok(1024));
        assert_eq!(pow(2, 0), Ok(1));
        assert_eq!(pow(-1, 1 << 40), Ok(1));
        assert_eq!(pow(-1, (1 << 40) + 1), Ok(-1));
        assert_eq!(pow(0, 1 << 40), Ok(0));
        assert!(pow(2, 63).is_err());
        assert!(pow(2, 1 << 40).is_err());
        assert_eq!(pow(2, -1), Err("2 ^ -1: an int to a negative power is a fraction — write 2.0 ^ -1 for a float".to_string()));
    }

    #[test]
    fn a_size_is_never_negative() {
        assert_eq!(size("zeros", 3), Ok(3));
        assert_eq!(size("zeros", -2), Err("zeros: a size cannot be negative (got -2)".to_string()));
    }

    #[test]
    fn a_panic_is_named_in_crows_words() {
        let words = |p: &str| panic_words(&p.to_string());
        assert_eq!(words("index out of bounds: the len is 3 but the index is 5"), "index 5 is out of range for an array of 3 items");
        assert_eq!(words("attempt to divide by zero"), "division by zero");
        assert_eq!(words("attempt to calculate the remainder with a divisor of zero"), "modulo by zero");
        assert_eq!(words("attempt to add with overflow"), "integer overflow in an addition");
        assert_eq!(words("capacity overflow"), "a size too large to hold");
        assert_eq!(words("something else"), "something else");
        assert_eq!(panic_words(&Stop("line 3: map has no key 'b'".to_string())), "line 3: map has no key 'b'");
    }

    #[test]
    fn a_guarded_panic_comes_back_as_words() {
        let r: Result<(), String> = guarded(|| {
            let v: Vec<i64> = Vec::new();
            let i = v.len() + 2;
            let _ = v[i];
        });
        assert_eq!(r, Err("index 2 is out of range for an array of 0 items".to_string()));
        assert_eq!(guarded(|| 5), Ok(5));
    }
}
