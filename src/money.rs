//! Money in integer cents and tax rates in thousandths of a percent, so all
//! arithmetic is exact.

pub type Cents = i64;

/// Parses amounts like `82.17`, `$1,234.5`, `3.55 CAD`.
pub fn parse_money(s: &str) -> Result<Cents, String> {
    let cleaned: String = s
        .trim()
        .trim_end_matches("CAD")
        .chars()
        .filter(|c| !matches!(c, '$' | ',' | ' '))
        .collect();
    let (negative, digits) = match cleaned.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, cleaned.as_str()),
    };
    let (whole, frac) = digits.split_once('.').unwrap_or((digits, ""));
    let bad = || format!("invalid amount: {s:?}");
    if whole.is_empty() && frac.is_empty() || frac.len() > 2 {
        return Err(bad());
    }
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| bad())? };
    let frac: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().map_err(|_| bad())? * 10,
        _ => frac.parse().map_err(|_| bad())?,
    };
    let cents = whole * 100 + frac;
    Ok(if negative { -cents } else { cents })
}

pub fn format_money(c: Cents) -> String {
    let sign = if c < 0 { "-" } else { "" };
    format!("{sign}{}.{:02}", c.abs() / 100, c.abs() % 100)
}

/// A tax rate stored in thousandths of a percent (13% = 13_000, 9.975% = 9_975).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate(pub i64);

const SCALE: i64 = 100_000; // thousandths of a percent per unit

impl Rate {
    pub fn parse_percent(s: &str) -> Result<Rate, String> {
        let s = s.trim().trim_end_matches('%');
        let bad = || format!("invalid rate: {s:?}");
        let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
        if frac.len() > 3 {
            return Err(bad());
        }
        let whole: i64 = whole.parse().map_err(|_| bad())?;
        let frac: i64 = if frac.is_empty() { 0 } else { format!("{frac:0<3}").parse().map_err(|_| bad())? };
        Ok(Rate(whole * 1000 + frac))
    }

    /// Tax on `base`, rounded half-up to the cent.
    pub fn tax_on(self, base: Cents) -> Cents {
        (2 * base * self.0 + SCALE) / (2 * SCALE)
    }

    /// Smallest base whose tax rounds to at least `tax`.
    fn min_base_for(self, tax: Cents) -> Cents {
        if tax <= 0 {
            return 0;
        }
        let num = 2 * SCALE * tax - SCALE;
        let den = 2 * self.0;
        (num + den - 1) / den
    }

    /// Inclusive range of bases whose tax rounds to within `slack` cents of `tax`.
    pub fn base_range(self, tax: Cents, slack: Cents) -> (Cents, Cents) {
        (self.min_base_for(tax - slack), self.min_base_for(tax + slack + 1) - 1)
    }

    /// The base that produces exactly `tax` before rounding, to the nearest cent.
    pub fn nominal_base(self, tax: Cents) -> Cents {
        (2 * SCALE * tax + self.0) / (2 * self.0)
    }

    pub fn to_string_pct(self) -> String {
        let frac = format!("{:03}", self.0 % 1000);
        let frac = frac.trim_end_matches('0');
        if frac.is_empty() {
            format!("{}%", self.0 / 1000)
        } else {
            format!("{}.{frac}%", self.0 / 1000)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_money() {
        assert_eq!(parse_money("82.17"), Ok(8217));
        assert_eq!(parse_money("$1,234.5 CAD"), Ok(123450));
        assert_eq!(parse_money("3"), Ok(300));
        assert_eq!(parse_money("-0.05"), Ok(-5));
        assert!(parse_money("1.234").is_err());
        assert!(parse_money("abc").is_err());
    }

    #[test]
    fn formats_money() {
        assert_eq!(format_money(8217), "82.17");
        assert_eq!(format_money(5), "0.05");
        assert_eq!(format_money(-120), "-1.20");
    }

    #[test]
    fn parses_rates() {
        assert_eq!(Rate::parse_percent("13"), Ok(Rate(13_000)));
        assert_eq!(Rate::parse_percent("9.975%"), Ok(Rate(9_975)));
        assert_eq!(Rate::parse_percent("14.0"), Ok(Rate(14_000)));
        assert_eq!(Rate(9_975).to_string_pct(), "9.975%");
        assert_eq!(Rate(13_000).to_string_pct(), "13%");
    }

    #[test]
    fn rounds_half_up() {
        assert_eq!(Rate(5_000).tax_on(70), 4); // 3.5 cents
        assert_eq!(Rate(5_000).tax_on(69), 3);
    }

    #[test]
    fn base_range_is_exactly_the_bases_that_round_to_the_tax() {
        for rate in [Rate(5_000), Rate(9_975), Rate(13_000), Rate(15_000)] {
            for tax in [1, 137, 146, 355] {
                let (lo, hi) = rate.base_range(tax, 0);
                assert_eq!(rate.tax_on(lo), tax);
                assert_eq!(rate.tax_on(hi), tax);
                assert_ne!(rate.tax_on(lo - 1), tax);
                assert_ne!(rate.tax_on(hi + 1), tax);
                let nominal = rate.nominal_base(tax);
                assert!(lo <= nominal && nominal <= hi);
            }
        }
    }
}
