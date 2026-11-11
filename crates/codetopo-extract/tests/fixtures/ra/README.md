# RA fixtures — the known-answer set for the RA Regression Contract

Every expectation in this directory is derived by hand arithmetic
from the printed model below, independently of any engine, and was
cross-checked at authoring time three ways: (1) the hand literals in
this README, (2) an independent decimal evaluation of the printed
formulas, (3) the contract test in `../../ra_contract.rs`. The
authoring script is kept with the stage record
(`stage5-work/fixtures-src/author_fixtures.py`); a change that moves
a number must move this README in the same commit.

RA is analysis-only: these fixtures measure a model's arithmetic.
Nothing here is a recommendation, a target, or advice.

## RA-F2 — the hand-auditable model (`ra-f2-model.xlsx`, `ra-f2.decl.json`)

Four sheets (`is`, `bs`, `cf`, `val`), two columns: B = FY2024
actuals/inputs, C = FY2025E forecast. Declared tolerance 0.01.
41 formula cells (≤ 60 by the plan).

**Inputs.** IS FY2024: revenue 1000, COGS 620, opex 150, interest
10. Drivers (`val`): growth g = 0.10 (B2), gross margin m = 0.40
(B3), opex ratio o = 0.15 (B4), Δreceivables 8 (B6), Δinventory 6
(B7), Δpayables 5 (B8), capex 60 (B9). Parameters: interest rate
0.05 (B5), tax rate 0.25 (B13), WACC 0.10 (B14), terminal growth
0.03 (B15), shares 100 (B16). Opening balance sheet: cash 100,
receivables 80, inventory 60, PP&E 400 → assets 640; payables 50,
debt 200, share capital 300, retained earnings 90 → L+E 640. ✓

**FY2024 IS (formulas).** gross = 1000−620 = 380; EBIT = 380−150 =
230; EBT = 230−10 = 220; tax = 220×0.25 = 55; NI = 165.

**FY2025E IS (formulas).** revenue = 1000×(1+g) = 1100; COGS =
1100×(1−m) = 660; gross = 440; opex = 1100×o = 165; EBIT = 275;
interest = 0.05×debt(200) = 10; EBT = 265; tax = 66.25; NI = 198.75.

**Cash flow FY2025E.** NI 198.75 (`is!C10`), D&A 40 (input), capex
60 (mirror of `val!B9`), ΔWC = 8+6−5 = 9, dividends 30 (input).
Net cash change = 198.75+40−60−9−30 = **139.75**; closing cash =
100+139.75 = **239.75**.

**Forecast balance sheet.** receivables 88, inventory 66, PP&E =
400+60−40 = 420; assets = 239.75+88+66+420 = **813.75**. Payables
55; RE = 90+198.75−30 = **258.75**; L+E = 55+200+300+258.75 =
**813.75** ✓ — the model articulates by construction.

**Valuation.** FCF = NI + D&A − capex − ΔWC = 198.75+40−60−9 =
**169.75** (note: FCF does NOT subtract dividends). PV(FCF) =
169.75/1.1 = 154.31818…; TV = 169.75×1.03/0.07 = **2497.75**;
PV(TV) = 2497.75/1.1 = 2270.68181…; EV = **2425.00**; equity =
EV − debt 200 + closing cash 239.75 = **2464.75**; per-share =
**24.6475**.

**Identities declared** (all execute; all hold on RA-F2):
balance-sheet equation at both columns (bs!B6 = bs!B9+bs!B12;
bs!C6 = bs!C9+bs!C12); net-income rollforward (bs!C11 =
bs!B11+is!C10−cf!C7 = 90+198.75−30); cash articulation (bs!C2 =
bs!B2+cf!C6 = 100+139.75).

**Sources.** `is!B2` and `bs!B2` carry a `sourced_from` edge to
`source::sec-edgar:fixture-filing` (source
`sec-edgar:fixture-10k-fy2024`, as_of 2024-12-31).

## RA-F3 — RA-F2 + three scenarios + a found reference

Scenarios bind only the drivers g/m/o: base (0.10/0.40/0.15), bull
(0.20/0.45/0.12), bear (0.00/0.35/0.18). ΔWC stays 9 in every
scenario — the working-capital drivers are NOT scenario-bound, so
scenario FCF = NI + 11 and scenario Δcash = NI − 19:

| scenario | revenue | NI | FCF | closing cash | RE |
|---|---|---|---|---|---|
| base | 1100 | 198.75 | 169.75 | 239.75 | 258.75 |
| bull | 1200 | 289.50 | 260.50 | 330.50 | 349.50 |
| bear | 1000 | 120.00 | 91.00 | 161.00 | 180.00 |

(Bull NI: 1200×(0.45−0.12) = 396; EBT = 396−10 = 386; tax 96.5; NI
289.5. Bear NI: 1000×0.17 = 170; EBT 160; tax 40; NI 120.) Spread
expectations for FCF/EV/equity/per-share are in
`ra-f3-expectations.json`; per-share closed form per scenario:
(FCF/1.1 + (FCF×1.03/0.07)/1.1 − 200 + cash)/100 → base 24.6475,
bull 38.5192857…, bear 12.61.

A `notes` sheet carries a text cross-reference to `val!B26`,
declared as a `references` edge with NO claimant — a found
reference (extracted truth per edge).

## RA-F5 — four mutations of RA-F2

- **(a)** Opening payables 50 → 57. The balance equation fails by
  exactly 7 at BOTH columns: FY2024 640 vs 647; FY2025E 813.75 vs
  820.75 (the mutation propagates through the forecast subtotals
  bs!C7/C9/C13). Two `ArticulationIdentity` diagnostics, both sides
  named; values NOT repaired (bs!B13 stays 647).
- **(b)** `val!B27 = B26+Z99` reads a cell the workbook does not
  contain: B27 carries no value; an `UnresolvedCall` names
  `val!Z99` at line 27; no edge is invented.
- **(c)** Circular pair: `bs!B8 = 200+is!B7`, `is!B7 =
  val!B5*bs!B8`. The core reports a `Cycle` (edge kind
  computes_into) over {bs!B8, is!B7}; the 26 cells in the cycle's
  shadow carry no value. Cells that still measure: is!C2 = 1100,
  bs!C5 = 420, cf!C5 = 9. (bs!C6 does NOT measure: its SUM range
  includes the unvalued bs!C2 — a range member with no value
  propagates its absence.)
- **(d)** `val!B27` duplicates B26's formula and is ALSO declared
  `presented_as: per_share_value`: a `MultiDefinition` names both
  cells under `ra-f5d-model::presented_as::per_share_value`. Both
  cells still measure 24.6475.

## RA-F6 — the provenance set

RA-F2 + the notes sheet, mutated three ways:

- **(a)** `is!B2`'s source block omits `as_of` → its `sourced_from`
  edge is REFUSED (`MissingProvenance`, missing = ["as_of"]); the
  cell keeps its value 1000. `bs!B2` is sourced from
  `source::stooq:aapl.us:daily-close` (source
  `stooq:aapl.us:daily-close`, as_of 2024-09-27 — the last close on
  or before the FY2024 period-end in the captured series) and IS
  admitted.
- **(b)** An `estimates` edge val!B2 → is!C2 with
  method/window/error is admitted (class estimate; the graph
  classes claims-bearing; `estimates` is in neither the breakage
  nor the closure edge set). A twin val!B3 → is!C3 with `error`
  omitted is refused (`MissingProvenance`, missing = ["error"]).
- **(c)** Two `references` edges from notes!B2: to val!B26 WITH
  claimant "fixture analyst" (declared_claim per edge) and to
  val!B25 without (found reference, extracted_truth per edge).

## Sensitivity oracle (`ra-sensitivity-expectations.json`)

One-sided steps per the sibling's rule (rate drivers: +0.01;
others: +1% of value). Derivations (PS = per-share 24.6475 at base):

- **FCF | g**: dFCF/dg = 1000×(1−0.25)×(0.40−0.15) = **187.5**;
  elasticity 187.5×0.10/169.75 = 0.110457….
- **PS | g**: 1000×0.75×0.25/100 × (1 + 1/1.1 + (1.03/0.07)/1.1) =
  **28.660714…** (the +1 inside is the closing-cash term).
- **PS | WACC**: exact difference of PS(w) = [169.75/(1+w) +
  174.8425/((w−0.03)(1+w)) + 39.75]/100 at w = 0.11 vs 0.10 →
  **−303.125**.
- **PS | terminal growth**: exact difference of the closed form at
  gt = 0.04 vs 0.03 → **404.16666…**.
- **PS | capex** (step 0.6): capex reaches FCF once (val!B20 reads
  val!B9; the cf!C4 mirror is read by nothing downstream), so
  dPS/dcapex = −(1/1.1 + (1.03/0.07)/1.1 + 1)/100 =
  **−0.152857…**. (An earlier draft of this oracle double-counted
  capex; the sibling's re-evaluation exposed it — the derivation
  above is the corrected one.)
- **PS | shares** (step 1): (2464.75/101 − 2464.75/100)/1 =
  **−0.244034…**.
- **EV | payables** (step 0.05): ΔWC falls 1 per AP unit → FCF
  rises 1 → dEV/dAP = 1/1.1 + (1.03/0.07)/1.1 = 100/7 =
  **14.285714…**. Marked `no_path` in the oracle: there is no
  computes_into path through the balance-sheet bridge (closing AP
  bs!C7 is read by no formula); the measured effect runs through
  cf!C5 (ΔWC) instead.

## Provenance of the workbooks

The `.xlsx` files are written by openpyxl with formulas as stored
text and NO cached values — by design: the extractor's evaluator
must recompute everything, and a cached value must never be able
to masquerade as a measurement (the evaluator ignores caches by
construction; the contract proves it in-module).
