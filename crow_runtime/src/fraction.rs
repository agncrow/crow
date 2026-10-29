//! Exact ratios of two ints (spec, Numbers That Keep Every Digit): a
//! fraction is held in lowest terms with its sign on the top, and `+`, `-`,
//! `*`, `/` and `^` of fractions are exact. A whole one shows as its int.
//! Both modes call these functions.
use crate::decimal::Dec;
use crate::int::{IBig, Int};
use dashu_ratio::RBig;
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Frac(pub RBig);

fn ten(k: u32) -> IBig {
    IBig::from(10u8).pow(k as usize)
}

impl Frac {
    /// `fraction[n, d]`: n over d in lowest terms; a zero below is no
    /// fraction (an error value of kind `'math'`).
    pub fn of_parts(n: &Int, d: &Int) -> Result<Frac, String> {
        if d.is_zero() {
            return Err(format!("fraction[{}, 0]: a fraction's bottom is not zero", n));
        }
        Ok(Frac(RBig::from_parts_signed(n.big(), d.big())))
    }

    pub fn of_int(n: &Int) -> Frac {
        Frac(RBig::from(n.big()))
    }

    /// A decimal exactly: its digits over its scale's power of ten.
    pub fn of_dec(d: &Dec) -> Frac {
        Frac(RBig::from_parts_signed(d.digits.clone(), ten(d.scale)))
    }

    pub fn add(&self, o: &Frac) -> Frac {
        Frac(&self.0 + &o.0)
    }

    pub fn sub(&self, o: &Frac) -> Frac {
        Frac(&self.0 - &o.0)
    }

    pub fn mul(&self, o: &Frac) -> Frac {
        Frac(&self.0 * &o.0)
    }

    pub fn div(&self, o: &Frac) -> Result<Frac, String> {
        if o.is_zero() {
            return Err("division by zero".to_string());
        }
        Ok(Frac(&self.0 / &o.0))
    }

    /// `f ^ n` of an int, exactly; a zero to a negative power has no value.
    /// 0, 1 and -1 to any power are exact, as an int's are (past 32 bits
    /// they had stopped as a number no memory holds).
    pub fn pow(&self, e: &Int) -> Result<Frac, String> {
        if let Some(b) = self.small_base() {
            return match b {
                0 if e.is_negative() => Err("division by zero".to_string()),
                _ => Ok(Frac::of_int(&Int::Small(b).pow(&e.abs())?)),
            };
        }
        let Some(n) = e.to_i64() else {
            return Err(format!("{} ^ {}: an exponent past 64 bits makes a number no memory holds", self, e));
        };
        let k = n.unsigned_abs();
        if k > u32::MAX as u64 {
            return Err(format!("{} ^ {}: an exponent past 32 bits makes a number no memory holds", self, e));
        }
        if n < 0 && self.is_zero() {
            return Err("division by zero".to_string());
        }
        let num = self.0.numerator().pow(k as usize);
        let den = IBig::from(self.0.denominator().clone()).pow(k as usize);
        Ok(Frac(if n >= 0 { RBig::from_parts_signed(num, den) } else { RBig::from_parts_signed(den, num) }))
    }

    pub fn neg(&self) -> Frac {
        Frac(-&self.0)
    }

    /// The fraction as 0, 1 or -1, where it is one of them.
    fn small_base(&self) -> Option<i64> {
        if self.0.denominator() != &dashu_int::UBig::ONE {
            return None;
        }
        let n = self.0.numerator();
        [0i64, 1, -1].into_iter().find(|b| n == &IBig::from(*b))
    }

    pub fn abs(&self) -> Frac {
        if self.0.numerator() < &IBig::ZERO { self.neg() } else { self.clone() }
    }

    pub fn is_zero(&self) -> bool {
        self.0.numerator() == &IBig::ZERO
    }

    /// The nearest int, a half away from zero.
    pub fn round(&self) -> Int {
        Int::of_big(self.0.round())
    }

    pub fn floor(&self) -> Int {
        Int::of_big(self.0.floor())
    }

    pub fn ceil(&self) -> Int {
        Int::of_big(self.0.ceil())
    }

    /// The int toward zero.
    pub fn trunc(&self) -> Int {
        Int::of_big(self.0.trunc())
    }

    /// The nearest float.
    pub fn to_f64(&self) -> f64 {
        self.0.to_f64().value()
    }

    pub fn cmp_frac(&self, o: &Frac) -> Ordering {
        self.0.cmp(&o.0)
    }
}

// ── the rows (crate::table's `fraction`, `round`, …) ────────────────────

/// `fraction[n, d]`.
pub fn of_parts(n: &Int, d: &Int) -> Result<Frac, String> {
    Frac::of_parts(n, d)
}

/// `fraction[n]` of an int.
pub fn of_int(n: &Int) -> Frac {
    Frac::of_int(n)
}

/// `fraction[d]` of a decimal, exactly.
pub fn of_dec(d: &Dec) -> Frac {
    Frac::of_dec(d)
}

/// `fraction[f]` of a fraction: itself.
pub fn same(f: &Frac) -> Frac {
    f.clone()
}

pub fn round(f: &Frac) -> Int {
    f.round()
}

pub fn floor(f: &Frac) -> Int {
    f.floor()
}

pub fn ceil(f: &Frac) -> Int {
    f.ceil()
}

/// `int[f]`: toward zero.
pub fn trunc(f: &Frac) -> Int {
    f.trunc()
}

/// `float[f]`: the nearest float.
pub fn to_float(f: &Frac) -> f64 {
    f.to_f64()
}

pub fn abs(f: &Frac) -> Frac {
    f.abs()
}

pub fn min(a: &Frac, b: &Frac) -> Frac {
    if b < a { b.clone() } else { a.clone() }
}

pub fn max(a: &Frac, b: &Frac) -> Frac {
    if b > a { b.clone() } else { a.clone() }
}

impl std::fmt::Display for Frac {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (n, d) = (self.0.numerator(), self.0.denominator());
        if *d == dashu_int::UBig::ONE {
            write!(f, "{}", n)
        } else {
            write!(f, "{}/{}", n, d)
        }
    }
}

impl crate::display::CrowDisplay for Frac {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

/// A fraction as JSON: the text it shows (JSON has no ratio).
impl crate::json::ToJson for Frac {
    fn to_json(&self) -> serde_json::Value {
        serde_json::Value::String(self.to_string())
    }
}

impl serde::Serialize for Frac {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Frac {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Frac::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("expected a fraction, got {}", text)))
    }
}

impl Frac {
    /// The fraction its text shows: `n/d`, or an int.
    pub fn parse(s: &str) -> Option<Frac> {
        match s.split_once('/') {
            Some((n, d)) => Frac::of_parts(&Int::parse(n.trim())?, &Int::parse(d.trim())?).ok(),
            None => Int::parse(s.trim()).map(|n| Frac::of_int(&n)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0, 1 and -1 to a power past 32 bits are exact, as an int's are;
    /// another base there stops in the runtime's words
    #[test]
    fn small_bases_to_any_power() {
        let huge = Int::Small(2).pow(&Int::Small(70)).unwrap();
        let odd = huge.add(&Int::Small(1));
        let f = |n: i64| Frac::of_int(&Int::Small(n));
        assert_eq!(f(1).pow(&huge).unwrap().to_string(), "1");
        assert_eq!(f(0).pow(&huge).unwrap().to_string(), "0");
        assert_eq!(f(0).pow(&Int::Small(0)).unwrap().to_string(), "1");
        assert_eq!(f(-1).pow(&huge).unwrap().to_string(), "1");
        assert_eq!(f(-1).pow(&odd).unwrap().to_string(), "-1");
        assert_eq!(f(-1).pow(&odd.neg()).unwrap().to_string(), "-1");
        assert_eq!(f(0).pow(&Int::Small(-1)).unwrap_err(), "division by zero");
        assert!(f(2).pow(&huge).unwrap_err().contains("no memory holds"));
    }

    /// A fraction is in lowest terms with its sign on top, exact under
    /// every operation, a whole one shown as its int
    #[test]
    fn fractions_are_exact() {
        let third = Frac::of_parts(&Int::Small(1), &Int::Small(3)).unwrap();
        assert_eq!(third.to_string(), "1/3");
        assert_eq!(Frac::of_parts(&Int::Small(2), &Int::Small(-4)).unwrap().to_string(), "-1/2");
        assert_eq!(third.add(&third).add(&third).to_string(), "1");
        assert_eq!(third.mul(&third).to_string(), "1/9");
        assert_eq!(third.pow(&Int::Small(-2)).unwrap().to_string(), "9");
        assert!(Frac::of_parts(&Int::Small(1), &Int::Small(0)).is_err());
        let quarter = Frac::of_dec(&Dec::parse("0.25").unwrap_or_default());
        assert_eq!(quarter.to_string(), "1/4");
        assert_eq!(Frac::of_parts(&Int::Small(5), &Int::Small(2)).unwrap().round().to_string(), "3");
        assert_eq!(Frac::of_parts(&Int::Small(-5), &Int::Small(2)).unwrap().round().to_string(), "-3");
        assert_eq!(Frac::of_parts(&Int::Small(-5), &Int::Small(2)).unwrap().floor().to_string(), "-3");
        assert_eq!(third.to_f64(), 1.0 / 3.0);
        assert_eq!(Frac::parse("-1/2"), Frac::of_parts(&Int::Small(-1), &Int::Small(2)).ok());
    }
}
