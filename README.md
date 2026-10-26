# shipping-tax-recon

Shopify invoices lump shipping into one subtotal but list tax per province.
This tool splits that subtotal into per-province subtotals that reproduce each
tax line exactly and reconcile to the invoice total.

Pass one or more Shopify bill PDFs (e.g. a quarter's worth):

```bash
cargo run -- ~/Downloads/Shopify_2026-07-19.pdf ~/Downloads/Shopify_2026-08-19.pdf ~/Downloads/Shopify_2026-09-19.pdf
```

Each bill's subtotal, tax lines (with the rates printed on the bill) and total
are read from its overview section, echoed on a `Read:` line for checking, and
reconciled separately. Or enter the figures by hand:

```bash
cargo run -- --subtotal 82.17 --total 88.55 --tax GST=3.55 --tax ON=1.46 --tax QC=1.37
```

- `--tax CODE=AMOUNT` for each tax line on the invoice. `GST` is the federal line;
  province codes (ON, NS, NB, NL, PE, QC, BC, SK, MB) use built-in rates.
  Override a rate with `CODE=AMOUNT@RATE`, e.g. `NS=1.40@14`.
- `--format csv` for spreadsheet import; one row per bucket per bill, with an `invoice` column.
- `--slack-cents N` if the solver reports no solution because Shopify rounded tax per shipment.
- `--allow-untaxed` if part of the subtotal had no tax at all (e.g. international labels).

## How it works

Each tax amount was rounded to the cent, so it pins its subtotal to a small
range (e.g. ON HST $1.46 at 13% means $11.20–$11.26). GST is one line covering
every non-HST province, so it pins their combined subtotal. Provinces with only
GST (and PST provinces where no PST was charged) can't be told apart, so they
share one "GST only" row. The solver picks values within those ranges, as close
to the centre as possible, that sum exactly to the invoice subtotal. The GST
line is then split across the GST-bearing rows in proportion to subtotal.
