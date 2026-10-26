use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use shipping_tax_recon::invoice::read_pdf;
use shipping_tax_recon::money::{format_money, parse_money, Cents, Rate};
use shipping_tax_recon::province::lookup;
use shipping_tax_recon::report;
use shipping_tax_recon::solve::{reconcile, Invoice, TaxLine};

/// Split a Shopify invoice's consolidated subtotal into per-province subtotals
/// that reproduce each tax line and reconcile to the invoice total.
///
/// Read one or more Shopify bill PDFs:
///   shipping-tax-recon Shopify_2026-07-19.pdf Shopify_2026-08-19.pdf Shopify_2026-09-19.pdf
///
/// Or enter the figures by hand:
///   shipping-tax-recon --subtotal 82.17 --total 88.55 --tax GST=3.55 --tax ON=1.46 --tax QC=1.37
#[derive(Parser)]
#[command(version, verbatim_doc_comment)]
struct Cli {
    /// Shopify bill PDFs to read.
    #[arg(value_name = "PDF", conflicts_with_all = ["subtotal", "total", "taxes"], required_unless_present = "subtotal")]
    invoices: Vec<PathBuf>,

    /// Invoice subtotal before tax.
    #[arg(long, value_parser = parse_money, requires = "taxes")]
    subtotal: Option<Cents>,

    /// Invoice total including tax; checked against subtotal + taxes.
    #[arg(long, value_parser = parse_money)]
    total: Option<Cents>,

    /// A tax line as CODE=AMOUNT, e.g. GST=3.55, ON=1.46, QC=1.37.
    /// Province codes use built-in rates; override with CODE=AMOUNT@RATE (e.g. NS=1.40@14).
    #[arg(long = "tax", value_name = "CODE=AMOUNT[@RATE]")]
    taxes: Vec<String>,

    /// Extra cents of rounding tolerance per tax line, for when tax was rounded per shipment.
    #[arg(long, default_value_t = 0)]
    slack_cents: Cents,

    /// Allow part of the subtotal to carry no tax (e.g. international shipments).
    #[arg(long)]
    allow_untaxed: bool,

    #[arg(long, value_enum, default_value_t = Format::Table)]
    format: Format,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Format {
    Table,
    Csv,
}

fn manual_invoice(cli: &Cli, subtotal: Cents) -> Result<Invoice, String> {
    let mut gst = None;
    let mut lines = Vec::new();
    for spec in &cli.taxes {
        let (code, rest) = spec.split_once('=').ok_or_else(|| format!("expected CODE=AMOUNT, got {spec:?}"))?;
        let (amount, rate) = match rest.split_once('@') {
            Some((a, r)) => (a, Some(Rate::parse_percent(r)?)),
            None => (rest, None),
        };
        let amount = parse_money(amount)?;
        if code.eq_ignore_ascii_case("GST") {
            if gst.replace(amount).is_some() {
                return Err("GST given more than once".into());
            }
            continue;
        }
        let province = lookup(code).ok_or_else(|| format!("unknown province code {code:?}"))?;
        lines.push(TaxLine { province, rate: rate.unwrap_or(province.rate), amount });
    }
    Ok(Invoice {
        subtotal,
        total: cli.total,
        gst: gst.unwrap_or(0),
        lines,
        slack: 0,
        allow_untaxed: false,
    })
}

/// One line listing what was read, so it can be checked against the bill.
fn describe(inv: &Invoice) -> String {
    let mut parts = vec![format!("subtotal {}", format_money(inv.subtotal))];
    if inv.gst > 0 {
        parts.push(format!("GST {}", format_money(inv.gst)));
    }
    for l in &inv.lines {
        parts.push(format!("{} {} {}", l.province.code, l.rate.to_string_pct(), format_money(l.amount)));
    }
    if let Some(total) = inv.total {
        parts.push(format!("total {}", format_money(total)));
    }
    parts.join(", ")
}

fn main() {
    let cli = Cli::parse();
    let inputs: Vec<(String, Result<Invoice, String>)> = match cli.subtotal {
        Some(subtotal) => vec![("manual".into(), manual_invoice(&cli, subtotal))],
        None => cli
            .invoices
            .iter()
            .map(|path| {
                let parsed = read_pdf(path);
                let name = path.file_name().map_or(path.display().to_string(), |n| n.to_string_lossy().into());
                let label = match &parsed {
                    Ok(p) if p.bill.is_some() => format!("{name} (bill #{})", p.bill.as_ref().unwrap()),
                    _ => name,
                };
                (label, parsed.map(|p| p.invoice))
            })
            .collect(),
    };

    let mut failed = false;
    if cli.format == Format::Csv {
        print!("{}", report::CSV_HEADER);
    }
    for (i, (label, invoice)) in inputs.iter().enumerate() {
        let result = invoice.clone().and_then(|mut inv| {
            inv.slack = cli.slack_cents;
            inv.allow_untaxed = cli.allow_untaxed;
            reconcile(&inv).map(|rec| (inv, rec))
        });
        match (result, cli.format) {
            (Ok((inv, rec)), Format::Table) => {
                if i > 0 {
                    println!();
                }
                println!("== {label}\nRead: {}\n", describe(&inv));
                print!("{}", report::table(&rec));
            }
            (Ok((_, rec)), Format::Csv) => print!("{}", report::csv_rows(label, &rec)),
            (Err(e), _) => {
                eprintln!("error: {label}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}
