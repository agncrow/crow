//! Exact base-ten numbers (spec, Numbers That Keep Every Digit). A decimal
//! is its digits and a scale — `12.30` is 1230 at scale 2, its two places
//! kept — exact under `+`, `-` and `*`, and rounded a half away from zero
//! to its significant digits where an answer has no end (`/`, `sqrt`, …):
//! 28 of them, or the most a decimal in it was given (`decimal[x, digits =
//! n]`), which every answer it takes part in is then rounded to. Both modes
//! call these functions.
use crate::int::{IBig, Int};
use std::cmp::Ordering;

/// The significant digits an answer with no end has, where no decimal in
/// it was given more.
pub const DEFAULT_DIGITS: u32 = 28;

#[derive(Clone, Debug)]
pub struct Dec {
    /// The value is `digits × 10^-scale`.
    pub digits: IBig,
    pub scale: u32,
    /// The significant digits its answers round to: 0 where it was given
    /// none (exact, and `DEFAULT_DIGITS` for an answer with no end).
    pub prec: u32,
}

fn ten(k: u32) -> IBig {
    IBig::from(10u8).pow(k as usize)
}

/// How many decimal digits a whole number has (0 has one).
fn digit_count(n: &IBig) -> u32 {
    let s = n.to_string();
    s.trim_start_matches('-').len() as u32
}

/// `n / 10^k`, rounded a half away from zero (a digit cut before it moves
/// no tie: a half rounds away either way).
fn shed(n: &IBig, k: u32) -> IBig {
    if k == 0 {
        return n.clone();
    }
    let d = ten(k);
    let neg = *n < IBig::ZERO;
    let a = if neg { -n } else { n.clone() };
    let (q, r) = (&a / &d, &a % &d);
    let q = if &r * IBig::from(2u8) >= d { q + IBig::from(1u8) } else { q };
    if neg { -q } else { q }
}

impl Dec {
    pub fn new(digits: IBig, scale: u32, prec: u32) -> Dec {
        let d = Dec { digits, scale, prec };
        if prec > 0 { d.rounded(prec) } else { d }
    }

    /// The decimal a text writes: digits with a point, a sign before them,
    /// an exponent after (`1.5e3`); the places it writes are kept.
    pub fn parse(s: &str) -> Option<Dec> {
        let t = s.trim();
        let (neg, t) = match t.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, t.strip_prefix('+').unwrap_or(t)),
        };
        let (mant, exp) = match t.find(['e', 'E']) {
            Some(i) => (&t[..i], t[i + 1..].parse::<i64>().ok()?),
            None => (t, 0),
        };
        let (int, frac) = match mant.find('.') {
            Some(i) => (&mant[..i], &mant[i + 1..]),
            None => (mant, ""),
        };
        if (int.is_empty() && frac.is_empty()) || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let digits: IBig = format!("0{}{}", int, frac).parse().ok()?;
        let scale = frac.len() as i64 - exp;
        let (digits, scale) = if scale < 0 { (digits * ten((-scale) as u32), 0) } else { (digits, scale) };
        let scale = u32::try_from(scale).ok()?;
        Some(Dec { digits: if neg { -digits } else { digits }, scale, prec: 0 })
    }

    pub fn of_int(n: &Int) -> Dec {
        Dec { digits: n.big(), scale: 0, prec: 0 }
    }

    /// A float as the decimal it shows: the shortest text that reads back as
    /// it (`decimal[0.1]` is 0.1, not the float's binary value).
    pub fn of_f64(x: f64) -> Result<Dec, String> {
        if !x.is_finite() {
            return Err(format!("{} is no decimal", x));
        }
        Dec::parse(&format!("{}", x)).ok_or_else(|| format!("{} is no decimal", x))
    }

    /// The decimal rounded to `n` significant digits, a half away from zero,
    /// and holding them: every answer it takes part in rounds to them too.
    pub fn with_digits(&self, n: u32) -> Result<Dec, String> {
        if n == 0 {
            return Err("a decimal has at least one digit: digits = 1 or more".into());
        }
        Ok(Dec { prec: n, ..self.clone() }.rounded(n))
    }

    /// Rounded to `p` significant digits (none dropped from a shorter one).
    fn rounded(mut self, p: u32) -> Dec {
        let have = digit_count(&self.digits);
        if have > p {
            let drop = have - p;
            if drop <= self.scale {
                self.digits = shed(&self.digits, drop);
                self.scale -= drop;
            } else {
                // past the point: the whole number rounded at its tens
                let whole = shed(&self.digits, drop);
                self.digits = whole * ten(drop - self.scale);
                self.scale = 0;
            }
        }
        self
    }

    fn joined(a: &Dec, b: &Dec) -> u32 {
        a.prec.max(b.prec)
    }

    /// Both digits at one scale, the larger.
    fn aligned(a: &Dec, b: &Dec) -> (IBig, IBig, u32) {
        let s = a.scale.max(b.scale);
        (&a.digits * ten(s - a.scale), &b.digits * ten(s - b.scale), s)
    }

    pub fn add(&self, o: &Dec) -> Dec {
        let (x, y, s) = Dec::aligned(self, o);
        Dec::new(x + y, s, Dec::joined(self, o))
    }

    pub fn sub(&self, o: &Dec) -> Dec {
        let (x, y, s) = Dec::aligned(self, o);
        Dec::new(x - y, s, Dec::joined(self, o))
    }

    pub fn mul(&self, o: &Dec) -> Dec {
        Dec::new(&self.digits * &o.digits, self.scale + o.scale, Dec::joined(self, o))
    }

    /// `a / b` to the significant digits of the decimals in it (28 where
    /// none was given more), a half away from zero; an exact quotient keeps
    /// no more places than it needs beyond `a`'s less `b`'s (`1.20 / 2` is
    /// 0.60, `10 / 4` is 2.5).
    pub fn div(&self, o: &Dec) -> Result<Dec, String> {
        if o.digits == IBig::ZERO {
            return Err("division by zero".to_string());
        }
        let p = match Dec::joined(self, o) {
            0 => DEFAULT_DIGITS,
            n => n,
        };
        let neg = (self.digits < IBig::ZERO) != (o.digits < IBig::ZERO);
        let a = if self.digits < IBig::ZERO { -&self.digits } else { self.digits.clone() };
        let b = if o.digits < IBig::ZERO { -&o.digits } else { o.digits.clone() };
        if a == IBig::ZERO {
            return Ok(Dec { digits: IBig::ZERO, scale: self.scale.saturating_sub(o.scale), prec: self.prec.max(o.prec) });
        }
        // enough places that the quotient has p + 1 digits
        let k = (p + 1 + digit_count(&b)).saturating_sub(digit_count(&a)) + 1;
        let n = &a * ten(k);
        let (q, r) = (&n / &b, &n % &b);
        // the value is q × 10^(o.scale - self.scale - k)
        let mut scale = self.scale as i64 + k as i64 - o.scale as i64;
        let have = digit_count(&q);
        let (mut q, exact) = if have > p {
            let drop = have - p;
            let lost = &q % ten(drop);
            let exact = r == IBig::ZERO && lost == IBig::ZERO;
            scale -= drop as i64;
            (shed(&q, drop), exact)
        } else {
            (q, r == IBig::ZERO)
        };
        if exact {
            // no more places than the ideal (a's less b's, at least none)
            let ideal = (self.scale as i64 - o.scale as i64).max(0);
            while scale > ideal && &q % IBig::from(10u8) == IBig::ZERO {
                q /= IBig::from(10u8);
                scale -= 1;
            }
        }
        let (q, scale) = if scale < 0 { (q * ten((-scale) as u32), 0u32) } else { (q, scale as u32) };
        Ok(Dec { digits: if neg { -q } else { q }, scale, prec: self.prec.max(o.prec) })
    }

    /// `d ^ n` of an int: exact for n of none or more (repeated `*`), the
    /// quotient's digits for a negative one.
    pub fn pow(&self, e: &Int) -> Result<Dec, String> {
        let Some(n) = e.to_i64() else {
            return Err(format!("{} ^ {}: an exponent past 64 bits makes a number no memory holds", self, e));
        };
        let k = n.unsigned_abs();
        if k > u32::MAX as u64 {
            return Err(format!("{} ^ {}: an exponent past 32 bits makes a number no memory holds", self, e));
        }
        let up = Dec::new(self.digits.pow(k as usize), self.scale.checked_mul(k as u32).ok_or_else(|| format!("{} ^ {}: too many places", self, e))?, self.prec);
        if n >= 0 {
            Ok(up)
        } else {
            Dec { digits: IBig::from(1u8), scale: 0, prec: self.prec }.div(&up)
        }
    }

    pub fn neg(&self) -> Dec {
        Dec { digits: -&self.digits, ..self.clone() }
    }

    pub fn abs(&self) -> Dec {
        if self.digits < IBig::ZERO { self.neg() } else { self.clone() }
    }

    pub fn is_zero(&self) -> bool {
        self.digits == IBig::ZERO
    }

    /// Rounded to `places` places after the point, a half away from zero;
    /// more places than it holds are zeros (`round[decimal['2.5'], 2]` is
    /// 2.50).
    pub fn round_places(&self, places: i64) -> Result<Dec, String> {
        if places >= self.scale as i64 {
            // places it does not have are zeros written out: as many as a
            // value may hold (the count had wrapped past 2^32, and a count
            // near 2^63 grew until the memory ran out — crow check --self)
            let more = crate::stops::size("round", places - self.scale as i64)?;
            let more = u32::try_from(more).map_err(|_| crate::stops::too_large("round", more as u128))?;
            return Ok(Dec { digits: &self.digits * ten(more), scale: self.scale + more, prec: self.prec });
        }
        // every digit rounded away: zero, with no power of ten made for it
        let drop = self.scale as i128 - places as i128;
        if drop > digit_count(&self.digits) as i128 {
            return Ok(Dec { digits: IBig::ZERO, scale: places.max(0) as u32, prec: self.prec });
        }
        let drop = drop as u32;
        if places >= 0 {
            Ok(Dec { digits: shed(&self.digits, drop), scale: places as u32, prec: self.prec })
        } else {
            // to tens, hundreds…: the digits rounded there, the zeros kept
            let whole = shed(&self.digits, drop);
            Ok(Dec { digits: whole * ten((-places) as u32), scale: 0, prec: self.prec })
        }
    }

    /// The nearest int, a half away from zero.
    pub fn round(&self) -> Int {
        Int::of_big(shed(&self.digits, self.scale))
    }

    /// The int toward zero.
    pub fn trunc(&self) -> Int {
        Int::of_big(&self.digits / ten(self.scale))
    }

    pub fn floor(&self) -> Int {
        let t = self.trunc();
        if self.digits < IBig::ZERO && Dec::of_int(&t).cmp(self) != Ordering::Equal { t.sub(&Int::Small(1)) } else { t }
    }

    pub fn ceil(&self) -> Int {
        let t = self.trunc();
        if self.digits > IBig::ZERO && Dec::of_int(&t).cmp(self) != Ordering::Equal { t.add(&Int::Small(1)) } else { t }
    }

    /// The nearest float (its text read as one, which rounds once).
    pub fn to_f64(&self) -> f64 {
        self.to_string().parse::<f64>().unwrap_or(f64::NAN)
    }

    /// Ordered by value (`2.50` equals `2.5`).
    pub fn cmp(&self, o: &Dec) -> Ordering {
        let (x, y, _) = Dec::aligned(self, o);
        x.cmp(&y)
    }

    /// The digits less their trailing zeros (one value, one form: a key).
    pub fn normal(&self) -> (IBig, u32) {
        let mut d = self.digits.clone();
        let mut s = self.scale;
        while s > 0 && &d % IBig::from(10u8) == IBig::ZERO {
            d /= IBig::from(10u8);
            s -= 1;
        }
        (d, s)
    }

    /// The significant digits an answer with no end is computed to.
    fn digits_for(&self) -> u32 {
        if self.prec == 0 { DEFAULT_DIGITS } else { self.prec }
    }

    fn to_dbig(&self, p: u32) -> dashu_float::DBig {
        let x = dashu_float::DBig::from_parts(self.digits.clone(), -(self.scale as isize));
        x.with_precision(p as usize + 10).value()
    }

    fn of_dbig(x: &dashu_float::DBig, p: u32, prec: u32) -> Dec {
        let repr = x.repr();
        let (digits, exp) = (repr.significand().clone(), repr.exponent());
        let d = if exp >= 0 {
            Dec { digits: digits * ten(exp as u32), scale: 0, prec }
        } else {
            Dec { digits, scale: (-exp) as u32, prec }
        };
        let r = d.rounded(p);
        // (a rounded answer keeps no zeros past its last digit)
        let (digits, scale) = r.normal();
        Dec { digits, scale, prec }
    }

    /// `sqrt`, `exp`, `ln`, `sin`, `cos`, `tan`, `atan` of a decimal, to its
    /// significant digits (28 where none was given): its own answer, a half
    /// away from zero.
    pub fn math(&self, f: &str) -> Result<Dec, String> {
        let p = self.digits_for();
        match f {
            "sqrt" if self.digits < IBig::ZERO => return Err(format!("sqrt of a negative decimal: {}", self)),
            "ln" if self.digits <= IBig::ZERO => return Err(format!("ln of a decimal not above zero: {}", self)),
            _ => {}
        }
        let x = self.to_dbig(p);
        let y = match f {
            "sqrt" => x.sqrt(),
            "exp" => x.exp(),
            "ln" => x.ln(),
            "sin" => x.sin(),
            "cos" => x.cos(),
            "tan" => x.tan(),
            "atan" => x.atan(),
            other => return Err(format!("{} of a decimal is not computed: float[d] reads it as a float", other)),
        };
        Ok(Dec::of_dbig(&y, p, self.prec))
    }
}

// ── the rows (crate::table's `decimal`, `round`, `sqrt`, …) ─────────────

/// `decimal[text]`: the decimal the text writes, its places kept.
pub fn of_text(s: &str) -> Result<Dec, String> {
    Dec::parse(s).ok_or_else(|| format!("cannot convert '{}' to decimal", s))
}

/// `decimal[n]` of an int.
pub fn of_int(n: &Int) -> Dec {
    Dec::of_int(n)
}

/// `decimal[x]` of a float: the decimal it shows.
pub fn of_float(x: f64) -> Result<Dec, String> {
    Dec::of_f64(x)
}

/// A float literal written beside a decimal: the number it writes.
pub fn of_literal(x: f64) -> Dec {
    Dec::of_f64(x).unwrap_or_default()
}

/// `decimal[d]` of a decimal: itself.
pub fn same(d: &Dec) -> Dec {
    d.clone()
}

/// `decimal[x, digits = n]`.
pub fn with_digits(d: &Dec, n: i64) -> Result<Dec, String> {
    let n = u32::try_from(n).map_err(|_| format!("decimal[…, digits = {}]: digits is a count from 1", n))?;
    d.with_digits(n)
}

pub fn round(d: &Dec) -> Int {
    d.round()
}

pub fn round_places(d: &Dec, places: i64) -> Result<Dec, String> {
    d.round_places(places)
}

pub fn floor(d: &Dec) -> Int {
    d.floor()
}

pub fn ceil(d: &Dec) -> Int {
    d.ceil()
}

/// `int[d]`: toward zero.
pub fn trunc(d: &Dec) -> Int {
    d.trunc()
}

/// `float[d]`: the nearest float.
pub fn to_float(d: &Dec) -> f64 {
    d.to_f64()
}

pub fn abs(d: &Dec) -> Dec {
    d.abs()
}

pub fn min(a: &Dec, b: &Dec) -> Dec {
    if b.cmp(a) == Ordering::Less { b.clone() } else { a.clone() }
}

pub fn max(a: &Dec, b: &Dec) -> Dec {
    if b.cmp(a) == Ordering::Greater { b.clone() } else { a.clone() }
}

pub fn sqrt(d: &Dec) -> Result<Dec, String> {
    d.math("sqrt")
}

pub fn exp(d: &Dec) -> Result<Dec, String> {
    d.math("exp")
}

pub fn ln(d: &Dec) -> Result<Dec, String> {
    d.math("ln")
}

pub fn sin(d: &Dec) -> Result<Dec, String> {
    d.math("sin")
}

pub fn cos(d: &Dec) -> Result<Dec, String> {
    d.math("cos")
}

pub fn tan(d: &Dec) -> Result<Dec, String> {
    d.math("tan")
}

pub fn atan(d: &Dec) -> Result<Dec, String> {
    d.math("atan")
}

impl PartialEq for Dec {
    fn eq(&self, o: &Dec) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl std::fmt::Display for Dec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let all = self.digits.to_string();
        let (sign, digits) = match all.strip_prefix('-') {
            Some(d) => ("-", d),
            None => ("", all.as_str()),
        };
        let s = self.scale as usize;
        if s == 0 {
            return write!(f, "{}{}", sign, digits);
        }
        let padded = format!("{:0>width$}", digits, width = s + 1);
        let (int, frac) = padded.split_at(padded.len() - s);
        write!(f, "{}{}.{}", sign, int, frac)
    }
}

impl crate::display::CrowDisplay for Dec {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

/// A decimal as JSON: the text it shows, every digit (JSON's readers hold a
/// number as a float, which would lose them).
impl crate::json::ToJson for Dec {
    fn to_json(&self) -> serde_json::Value {
        serde_json::Value::String(self.to_string())
    }
}

impl serde::Serialize for Dec {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Dec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let j = serde_json::Value::deserialize(d)?;
        let text = match &j {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            other => return Err(serde::de::Error::custom(format!("expected a decimal, got {}", other))),
        };
        Dec::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("expected a decimal, got {}", text)))
    }
}

impl Default for Dec {
    fn default() -> Self {
        Dec { digits: IBig::ZERO, scale: 0, prec: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Dec {
        Dec::parse(s).unwrap_or_default()
    }

    /// `+`, `-` and `*` are exact and keep their places; `/` has 28
    /// significant digits a half away from zero, an exact quotient its
    /// ideal places; given digits round every answer they take part in
    #[test]
    fn decimals_are_exact_where_they_can_be() {
        assert_eq!(d("0.1").add(&d("0.2")).to_string(), "0.3");
        assert!(d("0.1").add(&d("0.2")) == d("0.30"));
        assert_eq!(d("12.30").add(&d("1.1")).to_string(), "13.40");
        assert_eq!(d("1.10").mul(&d("2.0")).to_string(), "2.200");
        assert_eq!(d("1").div(&d("3")).unwrap().to_string(), "0.3333333333333333333333333333");
        assert_eq!(d("2").div(&d("3")).unwrap().to_string(), "0.6666666666666666666666666667");
        assert_eq!(d("1.20").div(&d("2")).unwrap().to_string(), "0.60");
        assert_eq!(d("10").div(&d("4")).unwrap().to_string(), "2.5");
        assert_eq!(d("-7").div(&d("2")).unwrap().to_string(), "-3.5");
        assert_eq!(d("1").div(&d("0")).unwrap_err(), "division by zero");
        let fifty = d("2").with_digits(50).unwrap();
        assert_eq!(fifty.math("sqrt").unwrap().to_string(), "1.4142135623730950488016887242096980785696718753769");
        assert_eq!(d("2.345").round_places(2).unwrap().to_string(), "2.35");
        assert_eq!(d("-2.345").round_places(2).unwrap().to_string(), "-2.35");
        assert_eq!(d("2.5").round_places(2).unwrap().to_string(), "2.50");
        // past what a value holds, refused; every digit rounded away, zero
        assert!(d("1.25").round_places(i64::MAX).unwrap_err().starts_with("round: a size too large to hold"));
        assert_eq!(d("1.25").round_places(i64::MIN).unwrap().to_string(), "0");
        assert_eq!(d("1.25").round_places(-3).unwrap().to_string(), "0");
        assert_eq!(d("1250").round_places(-3).unwrap().to_string(), "1000");
        assert_eq!(d("2.5").round().to_string(), "3");
        assert_eq!(d("-2.5").round().to_string(), "-3");
        assert_eq!(d("-2.5").floor().to_string(), "-3");
        assert_eq!(d("-2.5").ceil().to_string(), "-2");
        assert_eq!(d("-2.5").trunc().to_string(), "-2");
        assert_eq!(Dec::of_f64(0.1).unwrap().to_string(), "0.1");
        assert_eq!(d("1.5e3").to_string(), "1500");
        assert_eq!(d("2").pow(&Int::Small(-2)).unwrap().to_string(), "0.25");
        assert_eq!(d("1.1").pow(&Int::Small(3)).unwrap().to_string(), "1.331");
        assert_eq!(d("123.456").with_digits(4).unwrap().to_string(), "123.5");
        assert_eq!(d("123456").with_digits(2).unwrap().to_string(), "120000");
        assert!(d("-1").math("sqrt").is_err() && d("0").math("ln").is_err());
        assert_eq!(d("0.1").to_f64(), 0.1);
    }
}
