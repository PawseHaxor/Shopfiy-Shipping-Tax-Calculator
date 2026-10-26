use crate::money::{format_money, Cents};
use crate::solve::{Reconciliation, Row, RowKind};

pub fn table(rec: &Reconciliation) -> String {
    let header = ["Bucket", "Subtotal", "GST", "HST/QST/PST", "Total", "Valid subtotal range"];
    let mut cells: Vec<[String; 6]> = rec.rows.iter().map(row_cells).collect();
    let sum = |f: fn(&Row) -> Cents| format_money(rec.rows.iter().map(f).sum());
    let totals = [
        "TOTAL".to_string(),
        sum(|r| r.subtotal),
        sum(|r| r.gst),
        sum(|r| r.provincial),
        sum(Row::total),
        String::new(),
    ];

    let all = std::iter::once(header.map(String::from)).chain(cells.iter().cloned()).chain([totals.clone()]);
    let widths: Vec<usize> = (0..6).map(|c| all.clone().map(|r| r[c].chars().count()).max().unwrap()).collect();
    let fmt = |r: &[String; 6]| {
        let mut line = format!("{:<w$}", r[0], w = widths[0]);
        for c in 1..5 {
            line += &format!("  {:>w$}", r[c], w = widths[c]);
        }
        line += &format!("  {}", r[5]);
        line.trim_end().to_string() + "\n"
    };
    let rule = "-".repeat(widths.iter().sum::<usize>() + 10) + "\n";

    let mut out = fmt(&header.map(String::from)) + &rule;
    for r in cells.drain(..) {
        out += &fmt(&r);
    }
    out += &rule;
    out += &fmt(&totals);
    out += &format!(
        "\nGST is one line on the invoice; it is split across GST-bearing rows in proportion to subtotal.\n\
         Combined GST-taxed subtotal must be within {}..{}. Any subtotals inside the listed ranges\n\
         that still sum to the invoice subtotal are equally valid.\n",
        format_money(rec.gst_group_range.0),
        format_money(rec.gst_group_range.1),
    );
    out
}

pub const CSV_HEADER: &str = "invoice,bucket,subtotal,gst,provincial_tax,total,range_low,range_high\n";

pub fn csv_rows(invoice: &str, rec: &Reconciliation) -> String {
    let mut out = String::new();
    for r in &rec.rows {
        let (lo, hi) = r.range.map_or((String::new(), String::new()), |(lo, hi)| (format_money(lo), format_money(hi)));
        out += &format!(
            "\"{}\",\"{}\",{},{},{},{},{lo},{hi}\n",
            invoice.replace('"', "\"\""),
            r.label,
            format_money(r.subtotal),
            format_money(r.gst),
            format_money(r.provincial),
            format_money(r.total()),
        );
    }
    out
}

fn row_cells(r: &Row) -> [String; 6] {
    let dash = |c: Cents, show: bool| if show { format_money(c) } else { "-".into() };
    [
        r.label.clone(),
        format_money(r.subtotal),
        dash(r.gst, matches!(r.kind, RowKind::WithGst | RowKind::GstOnly)),
        dash(r.provincial, matches!(r.kind, RowKind::Hst | RowKind::WithGst)),
        format_money(r.total()),
        r.range.map_or(String::new(), |(lo, hi)| format!("{}..{}", format_money(lo), format_money(hi))),
    ]
}
