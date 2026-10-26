//! Pulls the subtotal, tax lines and total out of a Shopify bill PDF.
//!
//! Only the first (overview) block is read: from the first "Subtotal" up to
//! the "Total" that follows it. Each tax line looks like
//! `ONTARIO HST - ONTARIO (13.0%) #84787 1746 RT0001 $1.46 CAD`, where the
//! name after the dash is the jurisdiction ("CANADA" for GST) and the first
//! dollar amount after the rate is the tax charged.

use std::path::Path;

use regex::Regex;

use crate::money::{parse_money, Cents, Rate};
use crate::province::lookup_name;
use crate::solve::{Invoice, TaxLine};

#[derive(Debug)]
pub struct Parsed {
    pub bill: Option<String>,
    pub invoice: Invoice,
}

pub fn read_pdf(path: &Path) -> Result<Parsed, String> {
    let text = pdf_extract::extract_text(path).map_err(|e| e.to_string())?;
    parse_text(&text)
}

pub fn parse_text(text: &str) -> Result<Parsed, String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let money = r"\$(-?[\d,]+\.\d{2})";
    let subtotal_re = Regex::new(&format!("Subtotal {money}")).unwrap();
    let total_re = Regex::new(&format!("Total {money}")).unwrap();
    let tax_re = Regex::new(&format!(r" - ([A-ZÉ][A-ZÉ .'-]*?) \((\d+(?:\.\d+)?)%\).*?{money}")).unwrap();
    let bill_re = Regex::new(r"Bill #(\d+)").unwrap();

    let sub = subtotal_re.captures(&text).ok_or("no \"Subtotal $…\" found")?;
    let subtotal = parse_money(&sub[1])?;
    let after_subtotal = &text[sub.get(0).unwrap().end()..];
    let tot = total_re.captures(after_subtotal).ok_or("no \"Total $…\" found after the subtotal")?;
    let total = parse_money(&tot[1])?;
    let tax_block = &after_subtotal[..tot.get(0).unwrap().start()];

    let mut gst: Option<Cents> = None;
    let mut lines = Vec::new();
    for cap in tax_re.captures_iter(tax_block) {
        let name = &cap[1];
        let rate = Rate::parse_percent(&cap[2])?;
        let amount = parse_money(&cap[3])?;
        if name.eq_ignore_ascii_case("CANADA") {
            *gst.get_or_insert(0) += amount;
            continue;
        }
        let province = lookup_name(name).ok_or_else(|| format!("unrecognised tax jurisdiction {name:?}"))?;
        lines.push(TaxLine { province, rate, amount });
    }

    let invoice = Invoice {
        subtotal,
        total: Some(total),
        gst: gst.unwrap_or(0),
        lines,
        slack: 0,
        allow_untaxed: false,
    };
    Ok(Parsed { bill: bill_re.captures(&text).map(|c| c[1].to_string()), invoice })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text extracted from bill #591783612 (Sep 19 2026), trimmed.
    const SAMPLE: &str = "Bill #591783612\nPaid on Sep 19, 2026\nTOTAL DUE\n\n$88.55 CAD\n\nOVERVIEW\n\n\
        Apps (1 item) $0.00 CAD\n\nShipping (4 items) $82.17 CAD\n\nCredit $0.00 CAD\n\n\
        Subtotal $82.17 CAD\n\nCANADA GST/TPS - CANADA (5.0%) #84787 1746\nRT0001 $3.55 CAD\n\n\
        ONTARIO HST - ONTARIO (13.0%) #84787 1746\nRT0001 $1.46 CAD\n\n\
        QUEBEC QST/TVQ - QUEBEC (9.975%)\n#1220423162 TQ0001 $1.37 CAD\n\nTotal $88.55 CAD\n\n\
        DETAILED VIEW\n\nSubtotal $82.17 CAD\n\nCANADA GST/TPS - CANADA (5.0%) #84787 1746\nRT0001 $3.55 CAD\n\n\
        Total $88.55 CAD\n";

    #[test]
    fn parses_sample_bill() {
        let parsed = parse_text(SAMPLE).unwrap();
        assert_eq!(parsed.bill.as_deref(), Some("591783612"));
        let inv = parsed.invoice;
        assert_eq!(inv.subtotal, 8217);
        assert_eq!(inv.total, Some(8855));
        assert_eq!(inv.gst, 355);
        let lines: Vec<_> = inv.lines.iter().map(|l| (l.province.code, l.rate, l.amount)).collect();
        assert_eq!(lines, vec![("ON", Rate(13_000), 146), ("QC", Rate(9_975), 137)]);
    }

    /// Overview from bill #388528582 (Jul 8 2025): four tax lines, one of
    /// them wrapping its registration number across lines.
    const FOUR_LINES: &str = "Subtotal $274.33 CAD\nCANADA GST/TPS - CANADA (5.0%) #84787 1746\nRT0001 $12.30 CAD\n\
        NOVA SCOTIA HST - NOVA SCOTIA (14.0%) #84787\n1746 RT0001 $0.03 CAD\n\
        ONTARIO HST - ONTARIO (13.0%) #84787 1746\nRT0001 $3.65 CAD\n\
        QUEBEC QST/TVQ - QUEBEC (9.975%)\n#1220423162 TQ0001 $3.23 CAD\nTotal $293.54 CAD\n";

    #[test]
    fn parses_and_reconciles_four_tax_lines() {
        let inv = parse_text(FOUR_LINES).unwrap().invoice;
        assert_eq!((inv.subtotal, inv.gst, inv.total), (27433, 1230, Some(29354)));
        let lines: Vec<_> = inv.lines.iter().map(|l| (l.province.code, l.rate, l.amount)).collect();
        assert_eq!(
            lines,
            vec![("NS", Rate(14_000), 3), ("ON", Rate(13_000), 365), ("QC", Rate(9_975), 323)]
        );
        let rec = crate::solve::reconcile(&inv).unwrap();
        assert_eq!(rec.rows.iter().map(|r| r.total()).sum::<Cents>(), 29354);
    }

    #[test]
    fn reads_rate_from_the_bill() {
        let text = "Subtotal $10.00 CAD NOVA SCOTIA HST - NOVA SCOTIA (15.0%) #1 RT0001 $1.50 CAD Total $11.50 CAD";
        let inv = parse_text(text).unwrap().invoice;
        assert_eq!(inv.lines[0].province.code, "NS");
        assert_eq!(inv.lines[0].rate, Rate(15_000));
        assert_eq!(inv.gst, 0);
    }

    #[test]
    fn rejects_unknown_jurisdiction() {
        let text = "Subtotal $10.00 CAD FOO TAX - ATLANTIS (5.0%) $0.50 CAD Total $10.50 CAD";
        assert!(parse_text(text).unwrap_err().contains("ATLANTIS"));
    }

    #[test]
    fn requires_subtotal_and_total() {
        assert!(parse_text("Total $1.00 CAD").is_err());
        assert!(parse_text("Subtotal $1.00 CAD").is_err());
    }
}
