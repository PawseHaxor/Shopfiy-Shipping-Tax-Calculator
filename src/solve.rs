//! Finds per-province subtotals that reproduce every tax line on the invoice.
//!
//! Each provincial tax line pins that province's subtotal to the small range
//! of cents whose tax rounds to the charged amount. The GST line does the same
//! for the combined subtotal of every GST-bearing province (QST/PST provinces
//! plus GST-only ones, which the invoice can't tell apart). HST provinces sit
//! outside the GST group. Within those ranges the subtotals must add up to the
//! invoice subtotal exactly; we pick values as close to the unrounded centre
//! of each range as possible.

use crate::money::{format_money, Cents, Rate};
use crate::province::{Kind, Province, GST, PROVINCES};

#[derive(Debug, Clone)]
pub struct TaxLine {
    pub province: &'static Province,
    pub rate: Rate,
    pub amount: Cents,
}

#[derive(Debug, Clone)]
pub struct Invoice {
    pub subtotal: Cents,
    pub total: Option<Cents>,
    pub gst: Cents,
    pub lines: Vec<TaxLine>,
    /// Extra cents of rounding tolerance per tax line, for invoices where tax
    /// was rounded per shipment rather than on the aggregate.
    pub slack: Cents,
    /// Whether part of the subtotal may be untaxed (e.g. international labels).
    pub allow_untaxed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Hst,
    WithGst,
    GstOnly,
    Untaxed,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub label: String,
    pub kind: RowKind,
    pub subtotal: Cents,
    /// Inclusive range of subtotals that would also reproduce the tax line.
    pub range: Option<(Cents, Cents)>,
    pub gst: Cents,
    pub provincial: Cents,
}

impl Row {
    pub fn total(&self) -> Cents {
        self.subtotal + self.gst + self.provincial
    }
}

#[derive(Debug)]
pub struct Reconciliation {
    pub rows: Vec<Row>,
    pub gst_group_range: (Cents, Cents),
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    lo: Cents,
    hi: Cents,
    nominal: Cents,
    value: Cents,
}

impl Bucket {
    fn new(lo: Cents, hi: Cents, nominal: Cents) -> Self {
        Bucket { lo, hi, nominal, value: nominal.clamp(lo, hi) }
    }

    fn deviation_after(&self, step: Cents) -> f64 {
        ((self.value + step - self.nominal).abs()) as f64 / (self.hi - self.lo + 1) as f64
    }
}

/// Moves bucket values one cent at a time until they sum to `target`, each
/// time nudging whichever bucket stays closest (relative to its range width)
/// to its nominal value. Caller guarantees `target` is within the summed bounds.
fn distribute(buckets: &mut [Bucket], target: Cents) {
    let mut diff = target - buckets.iter().map(|b| b.value).sum::<Cents>();
    while diff != 0 {
        let step = diff.signum();
        let best = buckets
            .iter_mut()
            .filter(|b| if step > 0 { b.value < b.hi } else { b.value > b.lo })
            .min_by(|a, b| a.deviation_after(step).total_cmp(&b.deviation_after(step)))
            .expect("target checked against bounds");
        best.value += step;
        diff -= step;
    }
}

pub fn reconcile(inv: &Invoice) -> Result<Reconciliation, String> {
    if let Some(total) = inv.total {
        let computed = inv.subtotal + inv.gst + inv.lines.iter().map(|l| l.amount).sum::<Cents>();
        if computed != total {
            return Err(format!(
                "subtotal + taxes = {} but invoice total is {}; check the amounts entered",
                format_money(computed),
                format_money(total)
            ));
        }
    }
    for (i, line) in inv.lines.iter().enumerate() {
        if line.province.kind == Kind::GstOnly {
            return Err(format!("{} has no provincial tax; include its subtotal via the GST line", line.province.code));
        }
        if inv.lines[..i].iter().any(|l| l.province.code == line.province.code) {
            return Err(format!("{} appears more than once", line.province.code));
        }
    }

    let bucket_for = |rate: Rate, tax: Cents| {
        let (lo, hi) = rate.base_range(tax, inv.slack);
        Bucket::new(lo, hi, rate.nominal_base(tax))
    };
    let hst: Vec<&TaxLine> = inv.lines.iter().filter(|l| l.province.kind == Kind::Hst).collect();
    let with_gst: Vec<&TaxLine> = inv.lines.iter().filter(|l| l.province.kind == Kind::WithGst).collect();
    let mut hst_buckets: Vec<Bucket> = hst.iter().map(|l| bucket_for(l.rate, l.amount)).collect();
    let mut with_gst_buckets: Vec<Bucket> = with_gst.iter().map(|l| bucket_for(l.rate, l.amount)).collect();

    // The GST group must be big enough to hold every QST/PST province's subtotal.
    let mut gst_group = bucket_for(GST, inv.gst);
    let gst_group_range = (gst_group.lo, gst_group.hi);
    let with_gst_min: Cents = with_gst_buckets.iter().map(|b| b.lo).sum();
    gst_group.lo = gst_group.lo.max(with_gst_min);
    if gst_group.lo > gst_group.hi {
        return Err(format!(
            "GST of {} implies a GST-taxed subtotal of at most {}, but the QST/PST lines need at least {}{}",
            format_money(inv.gst),
            format_money(gst_group.hi),
            format_money(with_gst_min),
            slack_hint(inv)
        ));
    }
    gst_group.value = gst_group.value.clamp(gst_group.lo, gst_group.hi);

    let min_taxed: Cents = hst_buckets.iter().map(|b| b.lo).sum::<Cents>() + gst_group.lo;
    let max_taxed: Cents = hst_buckets.iter().map(|b| b.hi).sum::<Cents>() + gst_group.hi;
    if inv.subtotal < min_taxed {
        return Err(format!(
            "the charged taxes need a subtotal of at least {}, but the invoice subtotal is {}{}",
            format_money(min_taxed),
            format_money(inv.subtotal),
            slack_hint(inv)
        ));
    }
    let untaxed = (inv.subtotal - max_taxed).max(0);
    if untaxed > 0 && !inv.allow_untaxed {
        return Err(format!(
            "the charged taxes cover a subtotal of at most {}, leaving {} with no tax. \
             If some shipments are untaxed (e.g. international), pass --allow-untaxed{}",
            format_money(max_taxed),
            format_money(untaxed),
            slack_hint(inv)
        ));
    }

    let mut top: Vec<Bucket> = hst_buckets.iter().copied().chain([gst_group]).collect();
    distribute(&mut top, inv.subtotal - untaxed);
    let gst_group_total = top.pop().unwrap().value;
    hst_buckets.copy_from_slice(&top);

    let with_gst_max: Cents = with_gst_buckets.iter().map(|b| b.hi).sum();
    let with_gst_target =
        with_gst_buckets.iter().map(|b| b.value).sum::<Cents>().clamp(with_gst_min, with_gst_max.min(gst_group_total));
    distribute(&mut with_gst_buckets, with_gst_target);
    let gst_only = gst_group_total - with_gst_target;

    let mut rows = Vec::new();
    for (line, b) in hst.iter().zip(&hst_buckets) {
        rows.push(Row {
            label: format!("{} ({} {})", line.province.name, line.province.tax_name, line.rate.to_string_pct()),
            kind: RowKind::Hst,
            subtotal: b.value,
            range: Some((b.lo, b.hi)),
            gst: 0,
            provincial: line.amount,
        });
    }
    for (line, b) in with_gst.iter().zip(&with_gst_buckets) {
        rows.push(Row {
            label: format!(
                "{} (GST {} + {} {})",
                line.province.name,
                GST.to_string_pct(),
                line.province.tax_name,
                line.rate.to_string_pct()
            ),
            kind: RowKind::WithGst,
            subtotal: b.value,
            range: Some((b.lo, b.hi)),
            gst: 0,
            provincial: line.amount,
        });
    }
    if gst_only > 0 || inv.gst > 0 {
        rows.push(Row {
            label: format!("GST only ({})", gst_only_codes(inv)),
            kind: RowKind::GstOnly,
            subtotal: gst_only,
            range: None,
            gst: 0,
            provincial: 0,
        });
    }
    if inv.allow_untaxed {
        rows.push(Row {
            label: "Untaxed".into(),
            kind: RowKind::Untaxed,
            subtotal: untaxed,
            range: None,
            gst: 0,
            provincial: 0,
        });
    }
    allocate_gst(&mut rows, inv.gst);

    Ok(Reconciliation { rows, gst_group_range })
}

/// The invoice shows one GST line for every GST-bearing province; split it
/// across those rows in proportion to subtotal, by largest remainder, so the
/// pieces sum to the invoice figure exactly.
fn allocate_gst(rows: &mut [Row], gst: Cents) {
    let idx: Vec<usize> = (0..rows.len())
        .filter(|&i| matches!(rows[i].kind, RowKind::WithGst | RowKind::GstOnly))
        .collect();
    let base: Cents = idx.iter().map(|&i| rows[i].subtotal).sum();
    if base == 0 {
        return;
    }
    let mut remainders = Vec::new();
    for &i in &idx {
        let exact = gst * rows[i].subtotal;
        rows[i].gst = exact / base;
        remainders.push((exact % base, i));
    }
    let leftover = gst - idx.iter().map(|&i| rows[i].gst).sum::<Cents>();
    remainders.sort_by_key(|r| std::cmp::Reverse(r.0));
    for &(_, i) in remainders.iter().take(leftover as usize) {
        rows[i].gst += 1;
    }
}

fn gst_only_codes(inv: &Invoice) -> String {
    PROVINCES
        .iter()
        .filter(|p| p.kind != Kind::Hst && !inv.lines.iter().any(|l| l.province.code == p.code))
        .map(|p| p.code)
        .collect::<Vec<_>>()
        .join("/")
}

fn slack_hint(inv: &Invoice) -> &'static str {
    if inv.slack == 0 {
        ". If Shopify rounded tax per shipment, retry with --slack-cents 1 (or more)"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::province::lookup;

    fn line(code: &str, amount: Cents) -> TaxLine {
        let province = lookup(code).unwrap();
        TaxLine { province, rate: province.rate, amount }
    }

    /// Bill #591783612, Sep 19 2026.
    fn sample() -> Invoice {
        Invoice {
            subtotal: 8217,
            total: Some(8855),
            gst: 355,
            lines: vec![line("ON", 146), line("QC", 137)],
            slack: 0,
            allow_untaxed: false,
        }
    }

    fn assert_reconciles(inv: &Invoice, rec: &Reconciliation) {
        let sum = |f: fn(&Row) -> Cents| rec.rows.iter().map(f).sum::<Cents>();
        assert_eq!(sum(|r| r.subtotal), inv.subtotal);
        assert_eq!(sum(|r| r.gst), inv.gst);
        assert_eq!(sum(|r| r.provincial), inv.lines.iter().map(|l| l.amount).sum::<Cents>());
        if let Some(total) = inv.total {
            assert_eq!(sum(Row::total), total);
        }
        let gst_base: Cents = rec
            .rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::WithGst | RowKind::GstOnly))
            .map(|r| r.subtotal)
            .sum();
        assert!((GST.tax_on(gst_base) - inv.gst).abs() <= inv.slack);
        for (line, row) in inv.lines.iter().zip(rec.rows.iter().filter(|r| r.range.is_some())) {
            assert!((line.rate.tax_on(row.subtotal) - line.amount).abs() <= inv.slack, "{}", row.label);
        }
        assert!(rec.rows.iter().all(|r| r.subtotal >= 0));
    }

    #[test]
    fn reconciles_sample_invoice() {
        let inv = sample();
        let rec = reconcile(&inv).unwrap();
        assert_reconciles(&inv, &rec);
        let subtotals: Vec<Cents> = rec.rows.iter().map(|r| r.subtotal).collect();
        assert_eq!(subtotals, vec![1122, 1373, 5722]);
    }

    #[test]
    fn rejects_mismatched_total() {
        let inv = Invoice { total: Some(8856), ..sample() };
        assert!(reconcile(&inv).unwrap_err().contains("invoice total"));
    }

    #[test]
    fn rejects_subtotal_too_small_for_taxes() {
        let inv = Invoice { subtotal: 7000, total: None, ..sample() };
        assert!(reconcile(&inv).unwrap_err().contains("at least"));
    }

    #[test]
    fn leftover_requires_allow_untaxed() {
        let inv = Invoice { subtotal: 9000, total: None, ..sample() };
        assert!(reconcile(&inv).unwrap_err().contains("--allow-untaxed"));
        let inv = Invoice { allow_untaxed: true, ..inv };
        let rec = reconcile(&inv).unwrap();
        assert_reconciles(&inv, &rec);
        assert!(rec.rows.last().unwrap().subtotal > 0);
    }

    #[test]
    fn slack_widens_the_feasible_ranges() {
        // 0.20 more than the taxes can explain at strict rounding.
        let inv = Invoice { subtotal: 8255, total: None, ..sample() };
        assert!(reconcile(&inv).is_err());
        let inv = Invoice { slack: 1, ..inv };
        assert_reconciles(&inv, &reconcile(&inv).unwrap());
    }

    #[test]
    fn handles_many_provinces() {
        let lines = vec![line("ON", 520), line("NS", 140), line("NB", 75), line("QC", 300), line("BC", 210)];
        // Build an exact invoice from known subtotals: ON 40.00, NS 10.00,
        // NB 5.00, QC 30.08, BC 30.00, AB 12.34.
        let gst = GST.tax_on(3008 + 3000 + 1234);
        let subtotal = 4000 + 1000 + 500 + 3008 + 3000 + 1234;
        let inv = Invoice { subtotal, total: None, gst, lines, slack: 0, allow_untaxed: false };
        assert_reconciles(&inv, &reconcile(&inv).unwrap());
    }

    #[test]
    fn rejects_qst_without_enough_gst() {
        let inv = Invoice { subtotal: 1500, total: None, gst: 0, lines: vec![line("QC", 137)], slack: 0, allow_untaxed: false };
        assert!(reconcile(&inv).unwrap_err().contains("QST/PST"));
    }
}
