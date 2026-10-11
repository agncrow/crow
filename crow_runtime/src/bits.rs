//! `use bits` (spec, bits): an int's bits and a character's code, for the
//! algorithms that work on them — a bitmask's sets, a hash, a parser of
//! character codes. An int is a whole number of any size, read as two's
//! complement with as many bits as it needs (Python's reading): `bitand` of
//! a negative number keeps its high ones, and a shift right rounds down.
//! The rows over 64-bit ints are what a program calls; the width pass hands
//! a number past 64 bits — or a shift whose answer may pass them — to the
//! `…_big` forms (int_width, `table::big_form`), so both modes answer every
//! digit.
use crate::int::Int;
use dashu_int::ops::{BitTest, UnsignedAbs};

/// The most bits a shift may make a number take: past it the number is
/// none a program could hold (2^32 bits is half a gigabyte).
const MOST_BITS: i64 = 1 << 32;

/// `bitand[a, b]`: the bits both have.
pub fn and(a: i64, b: i64) -> i64 {
    a & b
}

/// `bitor[a, b]`: the bits either has.
pub fn or(a: i64, b: i64) -> i64 {
    a | b
}

/// `bitxor[a, b]`: the bits one has and the other has not.
pub fn xor(a: i64, b: i64) -> i64 {
    a ^ b
}

/// `bitcount[a]`: the ones among the bits of the number's size, `|a|`.
pub fn count(a: i64) -> i64 {
    a.unsigned_abs().count_ones() as i64
}

/// `shift[a, n]`: `a` times 2^n — for a negative `n`, divided by 2^-n and
/// rounded down — where the answer fits 64 bits (the width pass hands any
/// other to `shift_big`; a slip past it stops rather than wrap).
pub fn shift(a: i64, n: i64) -> Result<i64, String> {
    if n <= 0 {
        let k = n.unsigned_abs().min(63) as u32;
        return Ok(a >> k);
    }
    if a == 0 {
        return Ok(0);
    }
    if n < 64 {
        let r = a << n;
        if r >> n == a {
            return Ok(r);
        }
    }
    Err(format!("shift[{}, {}] passes 64 bits where an int of 64 bits is held", a, n))
}

/// `bitand` of numbers of any size.
pub fn and_big(a: &Int, b: &Int) -> Int {
    Int::of_big(a.big() & b.big())
}

/// `bitor` of numbers of any size.
pub fn or_big(a: &Int, b: &Int) -> Int {
    Int::of_big(a.big() | b.big())
}

/// `bitxor` of numbers of any size.
pub fn xor_big(a: &Int, b: &Int) -> Int {
    Int::of_big(a.big() ^ b.big())
}

/// `bitcount` of a number of any size.
pub fn count_big(a: &Int) -> i64 {
    match a.to_i64() {
        Some(n) => count(n),
        None => a.big().unsigned_abs().count_ones() as i64,
    }
}

/// `shift` of a number of any size, its answer of any size.
pub fn shift_big(a: &Int, n: i64) -> Result<Int, String> {
    if n <= 0 {
        let k = usize::try_from(n.unsigned_abs()).unwrap_or(usize::MAX);
        return Ok(Int::of_big(a.big() >> k));
    }
    let bits = a.big().unsigned_abs().bit_len() as i64;
    if bits > 0 && bits.saturating_add(n) > MOST_BITS {
        return Err(format!("shift[…, {}]: a number of {} bits is past what a whole number holds", n, bits.saturating_add(n)));
    }
    let k = usize::try_from(n).unwrap_or(usize::MAX);
    Ok(Int::of_big(a.big() << k))
}

/// `code[ch]`: the character's code (its Unicode scalar value).
pub fn code(ch: &str) -> Result<i64, String> {
    let mut chars = ch.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(c as i64),
        _ => Err(format!("code takes one character, got {:?} ({} characters)", ch, ch.chars().count())),
    }
}

/// `char[n]`: the character whose code is `n`.
pub fn char(n: i64) -> Result<String, String> {
    u32::try_from(n)
        .ok()
        .and_then(char::from_u32)
        .map(|c| c.to_string())
        .ok_or_else(|| format!("char takes a character's code — 0 to 1114111, none from 55296 to 57343 — got {}", n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_read_as_twos_complement() {
        assert_eq!(and(12, 10), 8);
        assert_eq!(or(12, 10), 14);
        assert_eq!(xor(12, 10), 6);
        assert_eq!(and(-1, 255), 255);
        assert_eq!(and(-8, 7), 0);
        assert_eq!(count(255), 8);
        assert_eq!(count(-255), 8);
        assert_eq!(count(0), 0);
    }

    #[test]
    fn a_shift_doubles_and_halves_rounding_down() {
        assert_eq!(shift(1, 10), Ok(1024));
        assert_eq!(shift(1024, -3), Ok(128));
        assert_eq!(shift(-7, -1), Ok(-4));
        assert_eq!(shift(5, -100), Ok(0));
        assert_eq!(shift(-5, -100), Ok(-1));
        assert!(shift(1, 63).is_err());
        assert!(shift(1, 64).is_err());
        assert_eq!(shift(0, 1000), Ok(0));
    }

    #[test]
    fn numbers_of_any_size_keep_every_bit() {
        let big = shift_big(&Int::Small(1), 100).unwrap();
        assert_eq!(big.to_string(), "1267650600228229401496703205376");
        assert_eq!(shift_big(&big, -99).unwrap(), Int::Small(2));
        assert_eq!(and_big(&big, &Int::Small(-1)), big);
        assert_eq!(count_big(&big), 1);
        assert_eq!(xor_big(&big, &big), Int::Small(0));
        assert_eq!(shift_big(&Int::Small(-7), -1).unwrap(), Int::Small(-4));
        assert!(shift_big(&Int::Small(1), MOST_BITS + 1).is_err());
    }

    #[test]
    fn a_character_and_its_code() {
        assert_eq!(code("A"), Ok(65));
        assert_eq!(code("é"), Ok(233));
        assert!(code("ab").is_err());
        assert!(code("").is_err());
        assert_eq!(char(65).as_deref(), Ok("A"));
        assert_eq!(char(128512).as_deref(), Ok("😀"));
        assert!(char(-1).is_err());
        assert!(char(0xD800).is_err());
        assert!(char(0x110000).is_err());
    }
}
