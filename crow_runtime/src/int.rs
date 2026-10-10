//! Whole numbers of any size (spec, Numbers That Keep Every Digit). An int
//! within 64 bits is `Small`, one past them `Big`; every operation's answer
//! is exact, and one that fits 64 bits again is `Small`, so a number has one
//! form and equal ints are equal forms. The compiled program holds an int in
//! this form only where it cannot show the int stays within 64 bits; the
//! interpreter's `Value::Big` holds the `Big` side.
pub use dashu_int::IBig;
use std::cmp::Ordering;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Int {
    Small(i64),
    Big(Arc<IBig>),
}

impl Default for Int {
    fn default() -> Self {
        Int::Small(0)
    }
}

impl From<i64> for Int {
    #[inline]
    fn from(n: i64) -> Self {
        Int::Small(n)
    }
}

impl From<IBig> for Int {
    fn from(b: IBig) -> Self {
        Int::of_big(b)
    }
}

impl Int {
    /// A big number in its one form: `Small` where it fits 64 bits.
    pub fn of_big(b: IBig) -> Int {
        match i64::try_from(&b) {
            Ok(n) => Int::Small(n),
            Err(_) => Int::Big(Arc::new(b)),
        }
    }

    /// The number as a big one (a copy).
    pub fn big(&self) -> IBig {
        match self {
            Int::Small(n) => IBig::from(*n),
            Int::Big(b) => (**b).clone(),
        }
    }

    /// The number in 64 bits, where it fits.
    #[inline]
    pub fn to_i64(&self) -> Option<i64> {
        match self {
            Int::Small(n) => Some(*n),
            Int::Big(_) => None,
        }
    }

    /// The nearest float.
    pub fn to_f64(&self) -> f64 {
        match self {
            Int::Small(n) => *n as f64,
            Int::Big(b) => b.to_f64().value(),
        }
    }

    pub fn is_zero(&self) -> bool {
        matches!(self, Int::Small(0))
    }

    pub fn is_negative(&self) -> bool {
        match self {
            Int::Small(n) => *n < 0,
            Int::Big(b) => b.signum() < IBig::ZERO,
        }
    }

    #[inline]
    pub fn add(&self, o: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, o)
            && let Some(c) = a.checked_add(*b)
        {
            return Int::Small(c);
        }
        Int::of_big(self.big() + o.big())
    }

    #[inline]
    pub fn sub(&self, o: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, o)
            && let Some(c) = a.checked_sub(*b)
        {
            return Int::Small(c);
        }
        Int::of_big(self.big() - o.big())
    }

    #[inline]
    pub fn mul(&self, o: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, o)
            && let Some(c) = a.checked_mul(*b)
        {
            return Int::Small(c);
        }
        Int::of_big(self.big() * o.big())
    }

    /// `a / b`: the quotient, rounded toward zero (spec, Arithmetic).
    pub fn div(&self, o: &Int) -> Result<Int, String> {
        if o.is_zero() {
            return Err("division by zero".to_string());
        }
        if let (Int::Small(a), Int::Small(b)) = (self, o)
            && let Some(c) = a.checked_div(*b)
        {
            return Ok(Int::Small(c));
        }
        Ok(Int::of_big(self.big() / o.big()))
    }

    /// `a % b`: the remainder, with the sign of `a`.
    pub fn rem(&self, o: &Int) -> Result<Int, String> {
        if o.is_zero() {
            return Err("modulo by zero".to_string());
        }
        if let (Int::Small(a), Int::Small(b)) = (self, o)
            && let Some(c) = a.checked_rem(*b)
        {
            return Ok(Int::Small(c));
        }
        Ok(Int::of_big(self.big() % o.big()))
    }

    #[inline]
    pub fn neg(&self) -> Int {
        if let Int::Small(a) = self
            && let Some(c) = a.checked_neg()
        {
            return Int::Small(c);
        }
        Int::of_big(-self.big())
    }

    #[inline]
    pub fn abs(&self) -> Int {
        if self.is_negative() { self.neg() } else { self.clone() }
    }

    /// `a ^ e` of ints: an int to a negative power is a fraction, refused
    /// (spec, the stops); an exponent past 32 bits would be a number past
    /// any memory, refused too.
    pub fn pow(&self, e: &Int) -> Result<Int, String> {
        if e.is_negative() {
            return Err(format!("{} ^ {}: an int to a negative power is a fraction — write {}.0 ^ {} for a float", self, e, self, e));
        }
        if let (Int::Small(a), Int::Small(b)) = (self, e)
            && let Ok(b32) = u32::try_from(*b)
            && let Some(c) = a.checked_pow(b32)
        {
            return Ok(Int::Small(c));
        }
        // (0, 1 and -1 to any power are themselves, or ±1)
        match self {
            Int::Small(0) => return Ok(Int::Small(if e.is_zero() { 1 } else { 0 })),
            Int::Small(1) => return Ok(Int::Small(1)),
            Int::Small(-1) => {
                let odd = match e {
                    Int::Small(n) => n % 2 != 0,
                    Int::Big(b) => (&**b % IBig::from(2)) != IBig::ZERO,
                };
                return Ok(Int::Small(if odd { -1 } else { 1 }));
            }
            _ => {}
        }
        let Some(n) = e.to_i64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n <= u32::MAX as usize) else {
            return Err(format!("{} ^ {}: an exponent past 32 bits makes a number no memory holds", self, e));
        };
        Ok(Int::of_big(self.big().pow(n)))
    }

    /// The int a text writes: digits, a sign before them, nothing around.
    pub fn parse(s: &str) -> Option<Int> {
        if let Ok(n) = s.parse::<i64>() {
            return Some(Int::Small(n));
        }
        let digits = s.strip_prefix(['-', '+']).unwrap_or(s);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.trim_start_matches('+').parse::<IBig>().ok().map(Int::of_big)
    }

    /// The number in 64 bits, or the reason it is refused where 64 bits are
    /// taken (spec: `a whole number past 64 bits where range[…] takes an int
    /// of 64 bits: …`), `who` naming what took it.
    pub fn small_for(&self, who: &str) -> Result<i64, String> {
        self.to_i64().ok_or_else(|| past_64_bits(who, &self.brief()))
    }

    /// The number as a refusal shows it: every digit up to 40 of them, and
    /// past that its first 20 and how many there are (a factorial of a
    /// thousand is 2,568 digits).
    pub fn brief(&self) -> String {
        let all = self.to_string();
        let digits = all.trim_start_matches('-').len();
        if digits <= 40 {
            return all;
        }
        let sign = if all.starts_with('-') { 1 } else { 0 };
        format!("{}… ({} digits)", &all[..sign + 20], digits)
    }

    /// The number as JSON: a number where JSON's readers hold it exactly
    /// (within 64 bits, signed or not), the text of its digits past that —
    /// as a parsed value's whole number past 64 bits would otherwise come
    /// back the nearest float.
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            Int::Small(n) => serde_json::Value::from(*n),
            Int::Big(b) => match u64::try_from(&**b) {
                Ok(u) => serde_json::Value::from(u),
                Err(_) => serde_json::Value::String(b.to_string()),
            },
        }
    }

    /// The int a JSON value holds: a whole number, or the text of one's
    /// digits (as `to_json_value` writes one past 64 bits).
    pub fn of_json(j: &serde_json::Value) -> Option<Int> {
        match j {
            serde_json::Value::Number(n) => n.as_i64().map(Int::Small).or_else(|| n.as_u64().map(|u| Int::of_big(IBig::from(u)))),
            serde_json::Value::String(s) => Int::parse(s),
            _ => None,
        }
    }

    /// The whole number a float holds, cut toward zero, every digit of it
    /// (`int[1e30]` is 1000000000000000019884624838656, the float's own
    /// value); NaN is 0, as it was in 64 bits, and an infinite float is no
    /// whole number.
    pub fn of_float(x: f64) -> Result<Int, String> {
        if x.is_nan() {
            return Ok(Int::Small(0));
        }
        if x.is_infinite() {
            return Err(format!("{} is no whole number: an int of an infinite float", if x > 0.0 { "inf" } else { "-inf" }));
        }
        let t = x.trunc();
        if t.abs() < 9.2e18 {
            return Ok(Int::Small(t as i64));
        }
        // a float past 2^63 is a whole number: its 53 bits, shifted
        let bits = t.to_bits();
        let exp = ((bits >> 52) & 0x7ff) as i64 - 1075;
        let mantissa = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
        let mut b = IBig::from(mantissa);
        if exp > 0 {
            b <<= exp as usize;
        } else {
            b >>= (-exp) as usize;
        }
        Ok(Int::of_big(if t < 0.0 { -b } else { b }))
    }
}

/// The refusal of a whole number past 64 bits where 64 bits are taken, in
/// one wording for both modes.
pub fn past_64_bits(who: &str, number: &str) -> String {
    format!("a whole number past 64 bits where {}: {}", who, number)
}

// ── the rows that keep every digit (crate::table's hidden `__…_big` rows,
// which the width pass calls where an answer may grow past 64 bits) ──

/// `int[x]` of a cell — text, a number, a bool, a moment — every digit kept:
/// text past 64 bits is the int it writes; the rest as `cell::int_of`.
pub fn int_of(c: &crate::cell::Cell) -> Result<Int, String> {
    match c {
        crate::cell::Cell::Text(s) => Int::parse(s).ok_or_else(|| format!("cannot convert '{}' to int", s)),
        crate::cell::Cell::Float(x) => Int::of_float(*x),
        other => crate::cell::int_of(other).map(Int::Small),
    }
}

/// `int[x]` of a float.
pub fn int_of_float(x: f64) -> Result<Int, String> {
    Int::of_float(x)
}

/// `floor[x]`, `ceil[x]`, `round[x]` of a float (a half away from zero).
pub fn floor(x: f64) -> Result<Int, String> {
    Int::of_float(x.floor())
}

pub fn ceil(x: f64) -> Result<Int, String> {
    Int::of_float(x.ceil())
}

pub fn round(x: f64) -> Result<Int, String> {
    Int::of_float(x.round())
}

pub fn abs(n: &Int) -> Int {
    n.abs()
}

pub fn min(a: &Int, b: &Int) -> Int {
    if b < a { b.clone() } else { a.clone() }
}

pub fn max(a: &Int, b: &Int) -> Int {
    if b > a { b.clone() } else { a.clone() }
}

/// `sort[xs]` of whole numbers, ascending.
pub fn sort(mut v: Vec<Int>) -> Vec<Int> {
    v.sort_unstable();
    v
}

impl serde::Serialize for Int {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.to_json_value() {
            serde_json::Value::String(digits) => s.serialize_str(&digits),
            _ => match self {
                Int::Small(n) => s.serialize_i64(*n),
                Int::Big(b) => s.serialize_u64(u64::try_from(&**b).unwrap_or(0)),
            },
        }
    }
}

impl<'de> serde::Deserialize<'de> for Int {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let j = serde_json::Value::deserialize(d)?;
        Int::of_json(&j).ok_or_else(|| serde::de::Error::custom(format!("expected a whole number, got {}", j)))
    }
}

impl crate::display::CrowDisplay for Int {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

impl crate::json::ToJson for Int {
    fn to_json(&self) -> serde_json::Value {
        self.to_json_value()
    }
}

impl PartialEq for Int {
    fn eq(&self, o: &Int) -> bool {
        match (self, o) {
            (Int::Small(a), Int::Small(b)) => a == b,
            (Int::Big(a), Int::Big(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Int {}

impl PartialOrd for Int {
    fn partial_cmp(&self, o: &Int) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Int {
    fn cmp(&self, o: &Int) -> Ordering {
        match (self, o) {
            (Int::Small(a), Int::Small(b)) => a.cmp(b),
            _ => self.big().cmp(&o.big()),
        }
    }
}

impl std::hash::Hash for Int {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        match self {
            Int::Small(n) => n.hash(h),
            Int::Big(b) => b.to_string().hash(h),
        }
    }
}

impl std::fmt::Display for Int {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Int::Small(n) => write!(f, "{}", n),
            Int::Big(b) => write!(f, "{}", b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arithmetic past 64 bits is exact and comes back to 64 bits where its
    /// answer fits; a quotient rounds toward zero and a remainder has the
    /// sign of its left side, as an int's in 64 bits
    #[test]
    fn an_int_grows_and_comes_back() {
        let max = Int::Small(i64::MAX);
        let one = Int::Small(1);
        let past = max.add(&one);
        assert_eq!(past.to_string(), "9223372036854775808");
        assert!(past.to_i64().is_none());
        assert_eq!(past.sub(&one), max);
        assert_eq!(Int::Small(2).pow(&Int::Small(100)).unwrap().to_string(), "1267650600228229401496703205376");
        assert_eq!(Int::Small(-7).div(&Int::Small(2)).unwrap(), Int::Small(-3));
        assert_eq!(Int::Small(-7).rem(&Int::Small(2)).unwrap(), Int::Small(-1));
        let big = Int::parse("-123456789012345678901234567890").unwrap();
        assert_eq!(big.div(&Int::Small(-7)).unwrap().to_string(), "17636684144620811271604938270");
        assert_eq!(big.rem(&Int::Small(7)).unwrap(), Int::Small(0));
        assert_eq!(big.add(&Int::Small(1)).rem(&Int::Small(7)).unwrap(), Int::Small(-6));
        assert_eq!(Int::Small(i64::MIN).neg().to_string(), "9223372036854775808");
        assert_eq!(Int::Small(i64::MIN).abs().to_string(), "9223372036854775808");
        assert!(Int::Small(1).div(&Int::Small(0)).is_err());
        assert!(Int::Small(2).pow(&Int::Small(-1)).is_err());
        assert_eq!(Int::Small(-1).pow(&Int::parse("100000000000000000001").unwrap()).unwrap(), Int::Small(-1));
        assert!(Int::Small(3).pow(&Int::parse("100000000000000000001").unwrap()).is_err());
        assert!(Int::parse("12a").is_none() && Int::parse("").is_none() && Int::parse("-").is_none());
        assert_eq!(Int::parse("+5"), Some(Int::Small(5)));
        assert!(past > max && Int::Small(0).sub(&past) == Int::Small(i64::MIN) && Int::Small(-1).sub(&past) < Int::Small(i64::MIN));
        assert_eq!(past.to_f64(), 9223372036854775808.0);
        assert!(past.small_for("range takes a count of 64 bits").unwrap_err().contains("9223372036854775808"));
    }
}
