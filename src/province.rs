//! Canadian sales tax structure by province / territory.

use crate::money::Rate;

pub const GST: Rate = Rate(5_000);

/// How a province's own tax interacts with the federal GST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Harmonized: one HST line replaces GST entirely.
    Hst,
    /// Provincial tax charged alongside GST (QST, PST, RST).
    WithGst,
    /// GST only, no provincial tax.
    GstOnly,
}

#[derive(Debug)]
pub struct Province {
    pub code: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub tax_name: &'static str,
    pub rate: Rate,
}

const fn p(code: &'static str, name: &'static str, kind: Kind, tax_name: &'static str, rate: i64) -> Province {
    Province { code, name, kind, tax_name, rate: Rate(rate) }
}

pub static PROVINCES: &[Province] = &[
    p("ON", "Ontario", Kind::Hst, "HST", 13_000),
    p("NS", "Nova Scotia", Kind::Hst, "HST", 14_000),
    p("NB", "New Brunswick", Kind::Hst, "HST", 15_000),
    p("NL", "Newfoundland and Labrador", Kind::Hst, "HST", 15_000),
    p("PE", "Prince Edward Island", Kind::Hst, "HST", 15_000),
    p("QC", "Quebec", Kind::WithGst, "QST", 9_975),
    p("BC", "British Columbia", Kind::WithGst, "PST", 7_000),
    p("SK", "Saskatchewan", Kind::WithGst, "PST", 6_000),
    p("MB", "Manitoba", Kind::WithGst, "RST", 7_000),
    p("AB", "Alberta", Kind::GstOnly, "", 0),
    p("NT", "Northwest Territories", Kind::GstOnly, "", 0),
    p("NU", "Nunavut", Kind::GstOnly, "", 0),
    p("YT", "Yukon", Kind::GstOnly, "", 0),
];

pub fn lookup(code: &str) -> Option<&'static Province> {
    PROVINCES.iter().find(|p| p.code.eq_ignore_ascii_case(code))
}

/// Matches the full name Shopify prints on tax lines, e.g. "ONTARIO".
pub fn lookup_name(name: &str) -> Option<&'static Province> {
    let name = name.trim().to_uppercase().replace('É', "E");
    PROVINCES.iter().find(|p| p.name.to_uppercase() == name)
}
