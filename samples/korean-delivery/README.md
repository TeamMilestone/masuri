# Korean delivery label regression corpus

Photos of Korean parcel labels captured by the wire-printer field operator
on a Galaxy S22 / Z Flip 5. See `wire-printer/samples/` for the originals.

- Images 01–06: 롯데택배 / cupost, captured 2026-05-07. I2/5 운송장.
- Image 07: GS25 반값택배, captured 2026-05-12. QR + 1D dual encoding.

## Symbology

The 운송장번호 (invoice) barcodes on images 01–06 are **Interleaved 2 of 5
(I2/5)**, not Code 128. Reference zbar (`zbarimg`) decodes 5/6 invoices when
I2/5 is enabled. Masuri before the I2/5 port had no I2/5 decoder, hence 0/6.

The smaller "169" / "462" companion codes are **Code-39**. Not relevant for
wire-printer (those aren't invoice numbers), so masuri does not need to
decode them — the wire-printer Scanner gates on numeric payloads of length
≥10 anyway.

Image 07 (GS25 반값택배) uses a **QR code** carrying a semicolon-delimited
payload, with a vertical 1D bar on the right edge of the label (the left
edge of this particular sample shows only printed text, no companion 1D).
zbar reads only the QR; the right-edge 1D is too blurred/angled in this
photo. See `docs/handoff-qr-decoder.md` for the planned QR decoder
integration.

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
| 07_gs25_210578897804.jpeg | `210578897804` | QR payload: `HALFIN;210578897804;V5M69;28;VV93;316;216;` — invoice is field [1]. The right-edge 1D companion goes unread by zbar (left edge has no visible 1D). |

## Baseline (pre-fix)

| File | masuri (pre-I2/5 port) | zbarimg |
| --- | --- | --- |
| 01 | 0 (only EAN-13 q=1 false positives) | I2/5 `460335055375` ✓ |
| 02 | 0 | I2/5 `263733057943` ✓ |
| 03 | 0 | I2/5 `410663725441` ✓ |
| 04 | 0 | I2/5 `409695576625` ✓ |
| 05 | 0 | (none) |
| 06 | Code-128 `462` (companion, not invoice) | I2/5 `536788954376` ✓ |
| 07 | 0 (no QR decoder) | QR `HALFIN;210578897804;…` ✓ |

**0 / 7 invoice barcodes successfully decoded by masuri before fixes.**

## Target

I2/5 port (commit `f1cecb0`): ≥4 of 6 1D invoices decoded (image 05 is
unreachable; image 01 also yields a secondary `055375` which the
wire-printer length gate rejects).

QR port (planned, see `docs/handoff-qr-decoder.md`): image 07 must decode
to the full semicolon payload; wire-printer extracts field [1] as the
12-digit invoice.
