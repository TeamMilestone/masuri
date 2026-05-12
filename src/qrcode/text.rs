//! Text extraction from parsed `QrCodeData` entries. Rust port of
//! qrdec/qrdectxt.c (Phase 5).
//!
//! Trimmed-down vs zbar: we keep the structured-append grouping logic and
//! FNC1 alphanumeric escape (handoff decision: SA "전체 구현"), but skip
//! the iconv-based encoding probing. Byte-mode payloads decode as:
//!   1. UTF-8 BOM stripped + validated, else
//!   2. valid UTF-8 (no BOM), else
//!   3. ASCII-only, else
//!   4. Latin-1 (every byte → its Unicode codepoint).
//!
//! ECI / Shift_JIS are stubs — the handoff decision was to leave them as
//! placeholders until a real-world sample needs them. GS25 (the only QR
//! corpus sample today) uses pure-ASCII byte mode, which path 3 above
//! handles exactly.

use super::decode::{
    QrCodeData, QrPayload, QR_MODE_ALNUM, QR_MODE_BYTE, QR_MODE_FNC1_1ST,
    QR_MODE_FNC1_2ND, QR_MODE_KANJI, QR_MODE_NUM,
};

/// Decode one `QrCodeData`'s entries into a UTF-8 string. Returns `None`
/// when the entry list contains a mode we don't know how to render
/// (currently: Kanji on a non-ECI path).
pub fn qr_code_data_decode_text(qrdata: &QrCodeData) -> Option<String> {
    let mut out = String::new();
    let mut fnc1 = false;
    // First pass: any FNC1 marker applies to the whole code.
    for entry in &qrdata.entries {
        if entry.mode == QR_MODE_FNC1_1ST || entry.mode == QR_MODE_FNC1_2ND {
            fnc1 = true;
            break;
        }
    }
    for entry in &qrdata.entries {
        match entry.mode {
            QR_MODE_NUM => {
                if let QrPayload::Data(ref b) = entry.payload {
                    // Always ASCII digits.
                    out.push_str(&String::from_utf8_lossy(b));
                }
            }
            QR_MODE_ALNUM => {
                if let QrPayload::Data(ref b) = entry.payload {
                    let s = std::str::from_utf8(b).ok()?;
                    if fnc1 {
                        // FNC1 escape: %% → %, single % → ASCII GS (0x1D).
                        let bytes = s.as_bytes();
                        let mut i = 0;
                        while i < bytes.len() {
                            if bytes[i] == b'%' {
                                if i + 1 < bytes.len() && bytes[i + 1] == b'%' {
                                    out.push('%');
                                    i += 2;
                                } else {
                                    out.push('\u{1D}'); // group separator
                                    i += 1;
                                }
                            } else {
                                out.push(bytes[i] as char);
                                i += 1;
                            }
                        }
                    } else {
                        out.push_str(s);
                    }
                }
            }
            QR_MODE_BYTE => {
                if let QrPayload::Data(ref b) = entry.payload {
                    out.push_str(&decode_byte_payload(b));
                }
            }
            QR_MODE_KANJI => {
                // Shift_JIS not implemented (handoff decision: placeholder).
                // Return None so callers know they're getting a partial result.
                return None;
            }
            _ => {
                // FNC1 markers, ECI, structured-append headers: nothing to
                // emit in text (SA grouping happens one level up).
            }
        }
    }
    Some(out)
}

/// Decode a byte-mode payload using the auto-detection ladder above.
fn decode_byte_payload(b: &[u8]) -> String {
    // 1. UTF-8 with BOM.
    if b.len() >= 3 && b[0] == 0xEF && b[1] == 0xBB && b[2] == 0xBF {
        if let Ok(s) = std::str::from_utf8(&b[3..]) {
            return s.to_string();
        }
    }
    // 2. Valid UTF-8 without BOM.
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    // 3+4. Fallback: Latin-1 (each byte → its Unicode codepoint). Always
    // succeeds, matches what most QR-decoding apps do when SJIS/ECI isn't
    // wired up.
    b.iter().map(|&c| c as char).collect()
}

/// Top-level: turn a list of QR codes into one text string per logical
/// payload, applying Structured Append grouping. Mirrors
/// `qr_code_data_list_extract_text` (qrdectxt.c:43).
///
/// Returned vector is `(text, qrdata_indices)` per logical payload — the
/// indices let callers reach back to bbox / version info.
pub fn qr_code_data_list_extract_text(qrlist: &[QrCodeData]) -> Vec<(String, Vec<usize>)> {
    let n = qrlist.len();
    let mut mark = vec![false; n];
    let mut results: Vec<(String, Vec<usize>)> = Vec::new();
    for i in 0..n {
        if mark[i] { continue; }
        let sa_size = qrlist[i].sa_size as usize;
        if sa_size == 0 {
            // Standalone code.
            mark[i] = true;
            if let Some(text) = qr_code_data_decode_text(&qrlist[i]) {
                results.push((text, vec![i]));
            }
            continue;
        }
        // Collect SA group members by (sa_size, sa_parity).
        let parity = qrlist[i].sa_parity;
        let mut group: Vec<Option<usize>> = vec![None; sa_size];
        for j in i..n {
            if mark[j] { continue; }
            if qrlist[j].sa_size as usize == sa_size
                && qrlist[j].sa_parity == parity
                && group[qrlist[j].sa_index as usize].is_none()
            {
                group[qrlist[j].sa_index as usize] = Some(j);
                mark[j] = true;
            }
        }
        // Concatenate available segments in sa_index order. Missing segments
        // get a NUL terminator marking the break (matches zbar's PARTIAL
        // semantics).
        let mut concat = String::new();
        let mut indices = Vec::new();
        let mut had_partial = false;
        for (k, slot) in group.iter().enumerate() {
            match *slot {
                Some(idx) => {
                    if let Some(t) = qr_code_data_decode_text(&qrlist[idx]) {
                        concat.push_str(&t);
                        indices.push(idx);
                    } else {
                        had_partial = true;
                    }
                }
                None => {
                    if k + 1 < sa_size { concat.push('\0'); }
                    had_partial = true;
                }
            }
        }
        let _ = had_partial; // tracked for future PARTIAL support
        if !indices.is_empty() {
            results.push((concat, indices));
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qrcode::decode::QrCodeDataEntry;

    fn make_byte_data(bytes: &[u8]) -> QrCodeData {
        let mut qd = QrCodeData::new();
        qd.entries.push(QrCodeDataEntry {
            mode: QR_MODE_BYTE,
            payload: QrPayload::Data(bytes.to_vec()),
        });
        qd
    }

    #[test]
    fn pure_ascii_byte_mode() {
        // GS25 payload shape.
        let qd = make_byte_data(b"HALFIN;210578897804;V5M69;28;VV93;316;216;");
        let s = qr_code_data_decode_text(&qd).unwrap();
        assert_eq!(s, "HALFIN;210578897804;V5M69;28;VV93;316;216;");
    }

    #[test]
    fn utf8_with_bom_stripped() {
        let mut payload = vec![0xEF, 0xBB, 0xBF];
        payload.extend_from_slice("안녕".as_bytes());
        let qd = make_byte_data(&payload);
        let s = qr_code_data_decode_text(&qd).unwrap();
        assert_eq!(s, "안녕");
    }

    #[test]
    fn utf8_without_bom() {
        let qd = make_byte_data("hello, 世界".as_bytes());
        let s = qr_code_data_decode_text(&qd).unwrap();
        assert_eq!(s, "hello, 世界");
    }

    #[test]
    fn latin1_fallback() {
        // 0xC0–0xFF: in valid Latin-1 but isolated → not valid UTF-8.
        let qd = make_byte_data(&[0xC0, 0xE9, 0xFF]);
        let s = qr_code_data_decode_text(&qd).unwrap();
        // Latin-1 0xC0=À, 0xE9=é, 0xFF=ÿ.
        assert_eq!(s, "Àéÿ");
    }

    #[test]
    fn numeric_alphanumeric_concat() {
        let mut qd = QrCodeData::new();
        qd.entries.push(QrCodeDataEntry {
            mode: QR_MODE_NUM, payload: QrPayload::Data(b"12345".to_vec()),
        });
        qd.entries.push(QrCodeDataEntry {
            mode: QR_MODE_ALNUM, payload: QrPayload::Data(b"ABC$%".to_vec()),
        });
        let s = qr_code_data_decode_text(&qd).unwrap();
        assert_eq!(s, "12345ABC$%");
    }

    #[test]
    fn fnc1_alphanumeric_escape() {
        let mut qd = QrCodeData::new();
        qd.entries.push(QrCodeDataEntry {
            mode: QR_MODE_FNC1_1ST, payload: QrPayload::None,
        });
        // Sequence: "01" | "%%" → "%" | "06" | "%" → GS (0x1D) | "TX"
        qd.entries.push(QrCodeDataEntry {
            mode: QR_MODE_ALNUM, payload: QrPayload::Data(b"01%%06%TX".to_vec()),
        });
        let s = qr_code_data_decode_text(&qd).unwrap();
        assert_eq!(s, "01%06\u{1D}TX");
    }

    #[test]
    fn structured_append_groups_in_order() {
        // Two segments: sa_index 0 ("AB"), sa_index 1 ("CD"), sa_size 2,
        // both with the same parity byte.
        let mk = |idx: u8, body: &[u8]| -> QrCodeData {
            let mut qd = make_byte_data(body);
            qd.sa_index = idx;
            qd.sa_size = 2;
            qd.sa_parity = 42;
            qd
        };
        // List in non-sequential order to verify reordering.
        let list = vec![mk(1, b"CD"), mk(0, b"AB")];
        let out = qr_code_data_list_extract_text(&list);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "ABCD");
        // Both qrdata indices are referenced.
        assert_eq!(out[0].1.len(), 2);
    }

    #[test]
    fn kanji_mode_currently_unsupported() {
        let mut qd = QrCodeData::new();
        qd.entries.push(QrCodeDataEntry {
            mode: QR_MODE_KANJI,
            payload: QrPayload::Data(vec![0x82, 0xA0]), // SJIS あ
        });
        assert!(qr_code_data_decode_text(&qd).is_none());
    }
}
