# Korean delivery label regression corpus

Six photos of Korean parcel labels (롯데택배 / cupost) captured 2026-05-07
by the wire-printer field operator on a Galaxy S22 / Z Flip 5. See
`wire-printer/samples/` for the originals.

## Symbology

The 운송장번호 (invoice) barcodes on these labels are **Interleaved 2 of 5
(I2/5)**, not Code 128. Reference zbar (`zbarimg`) decodes 5/6 invoices when
I2/5 is enabled. Masuri before this change had no I2/5 decoder, hence 0/6.

The smaller "169" / "462" companion codes are **Code-39**. Not relevant for
wire-printer (those aren't invoice numbers), so masuri does not need to
decode them — the wire-printer Scanner gates on numeric payloads of length
≥10 anyway.

## Ground truth

12-digit invoice number, verified by `zbarimg` against each image:

| File | I2/5 payload | Notes |
| --- | --- | --- |
| 01_cupost_460335055375.jpeg | `460335055375` | Also a shorter `055375` I2/5 nearby |
| 02_lotte_263733057943.jpeg | `263733057943` | |
| 03_lotte_410663725441.jpeg | `410663725441` | |
| 04_lotte_409695576625.jpeg | `409695576625` | |
| 05_lotte_263733045004.jpeg | (unreadable) | Reference zbar also fails — too blurred / skewed; left in corpus as a known-hard case |
| 06_lotte_536788954376.jpeg | `536788954376` | |

## Baseline (pre-fix)

| File | masuri | zbarimg |
| --- | --- | --- |
| 01 | 0 (only EAN-13 q=1 false positives) | I2/5 `460335055375` ✓ |
| 02 | 0 | I2/5 `263733057943` ✓ |
| 03 | 0 | I2/5 `410663725441` ✓ |
| 04 | 0 | I2/5 `409695576625` ✓ |
| 05 | 0 | (none) |
| 06 | Code-128 `462` (companion, not invoice) | I2/5 `536788954376` ✓ |

**0 / 6 invoice barcodes successfully decoded by masuri before fix.**

## Target

≥4 of 6 invoices decoded after I2/5 port (image 5 is unreachable; image 1
should also yield the secondary `055375` which the wire-printer length gate
rejects).
