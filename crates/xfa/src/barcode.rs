//! PDF417 paper-form barcodes. Encoding is supplied by Apache-2.0 rxing (ZXing),
//! with XFA's module width and correction level; output is vector PDF geometry.
use std::collections::BTreeMap;

use crate::model::Rect;

pub(crate) fn bars(value: &str, properties: &BTreeMap<String, String>, rect: Rect) -> Result<Vec<Rect>, String> {
    let property = |name: &str| properties.get(&format!("ui.barcode.{name}")).map(String::as_str);
    if !property("type").is_some_and(|t| t.eq_ignore_ascii_case("pdf417")) {
        return Err("this barcode type is not supported".into());
    }
    // PDF417 has at most 928 codewords, encoding at most 2710 numeric characters.
    // Bound input before invoking the encoder, including UTF-8 expansion.
    if value.len() > 10_840 {
        return Err("the PDF417 value is too large".into());
    }
    let dimension = |name, default| -> Result<f64, String> {
        let v = property(name).map(crate::parse::measure).unwrap_or(Some(default)).ok_or("invalid barcode module dimension")?;
        if !v.is_finite() || !(0.01..=72.0).contains(&v) {
            return Err("invalid barcode module dimension".into());
        }
        Ok(v)
    };
    let mw = dimension("moduleWidth", 0.25 * 72.0 / 25.4)?;
    if !rect.x.is_finite() || !rect.y.is_finite() || !rect.w.is_finite() || !rect.h.is_finite() || rect.w <= 0.0 || rect.h <= 0.0 {
        return Err("the barcode has no usable area".into());
    }
    let correction = property("errorCorrectionLevel")
        .unwrap_or("2")
        .parse::<u32>()
        .ok()
        .filter(|n| *n <= 8)
        .ok_or("PDF417 error correction level must be between 0 and 8")?;
    // Start/stop and row indicators occupy 69 modules; reserve a two-module quiet zone.
    let cols = ((rect.w / mw - 73.0) / 17.0).floor();
    // XFA 3.3 pp. 442–443: Acrobat ignores PDF417 moduleHeight and fits the
    // symbol to its field. Reserve the minimum three-module row height here;
    // once the payload determines the row count, distribute rows over the height.
    let rows = ((rect.h - 4.0 * mw) / (3.0 * mw)).floor();
    if cols < 1.0 || rows < 3.0 {
        return Err("the barcode does not fit its field".into());
    }
    let count = |name, max| -> Result<Option<u32>, String> {
        property(name).map(|v| v.parse::<u32>().ok().filter(|v| (1..=max).contains(v)).ok_or_else(|| format!("invalid PDF417 {name}"))).transpose()
    };
    let max_cols = count("dataColumnCount", 30)?.unwrap_or(cols.min(30.0) as u32);
    let fixed_rows = count("dataRowCount", 90)?;
    if max_cols as f64 > cols || fixed_rows.is_some_and(|r| r < 3 || r as f64 > rows) {
        return Err("the barcode dimensions do not fit its field".into());
    }
    let max_rows = fixed_rows.unwrap_or(rows.min(90.0) as u32);
    let mut encoder = rxing::pdf417::encoder::PDF417::new();
    encoder.setDimensions(max_cols, max_cols, max_rows, fixed_rows.unwrap_or(3));
    // XFA 3.3 §12: text defaults to UTF-8, independent of PDF417's native default.
    let name = property("charEncoding").unwrap_or("UTF-8");
    let charset = rxing::common::CharacterSet::get_character_set_by_name(name).ok_or("unsupported barcode character encoding")?;
    let bytes = charset.encode(value).map_err(|_| "unsupported barcode character encoding")?;
    if charset.decode(&bytes).ok().as_deref() != Some(value) {
        return Err("the barcode value contains characters its encoding cannot represent".into());
    }
    encoder.setEncoding(Some(charset));
    // Catch a dependency fault at this boundary too, for callers below the engine guard.
    let matrix = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        encoder.generateBarcodeLogic(value, correction).map_err(|_| "the PDF417 value does not fit its field or encoding")?;
        encoder.getBarcodeMatrix().as_ref().map(|m| m.getMatrix()).ok_or("the PDF417 encoder produced no bars")
    }))
    .map_err(|_| "the PDF417 encoder could not process this value")??;
    if matrix.is_empty() || matrix.len() > 90 || matrix.iter().any(|row| row.len() > 600) {
        return Err("the PDF417 encoder exceeded the symbol limits".into());
    }
    let mh = (rect.h - 4.0 * mw) / matrix.len() as f64;
    let mut bars = Vec::new();
    // rxing's low-level matrix is bottom-up; XFA layout coordinates are top-down.
    for (y, row) in matrix.iter().rev().enumerate() {
        let mut x = 0;
        while x < row.len() {
            if row.get(x) != Some(&1) {
                x += 1;
                continue;
            }
            let start = x;
            while row.get(x) == Some(&1) {
                x += 1;
            }
            bars.push(Rect::new(rect.x + (start as f64 + 2.0) * mw, rect.y + 2.0 * mw + y as f64 * mh, (x - start) as f64 * mw, mh));
        }
    }
    Ok(bars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rxing::{BinaryBitmap, Luma8LuminanceSource, Reader, common::HybridBinarizer, pdf417::PDF417Reader};

    #[test]
    fn pdf417_vector_bars_decode_to_the_original_payload() {
        // Rasterize the geometry the PDF writer receives, so orientation, module sizes
        // and quiet zones are tested along with the encoder.
        let payload = "TEST_FORM_1|EXAMPLE|2030-01-02|Québec|".repeat(7);
        let props = BTreeMap::from([
            ("ui.barcode.type".into(), "pdf417".into()),
            ("ui.barcode.moduleWidth".into(), "1pt".into()),
            ("ui.barcode.moduleHeight".into(), "2pt".into()),
            ("ui.barcode.errorCorrectionLevel".into(), "5".into()),
        ]);
        let bars = bars(&payload, &props, Rect::new(0.0, 0.0, 270.0, 144.0)).unwrap();
        assert!(bars.len() > 100);
        let (width, height) = (1080usize, 576usize);
        let mut pixels = vec![255; width * height];
        for r in bars {
            assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= 270.0 && r.y + r.h <= 144.0);
            for y in (r.y * 4.0).round() as usize..((r.y + r.h) * 4.0).round() as usize {
                for x in (r.x * 4.0).round() as usize..((r.x + r.w) * 4.0).round() as usize {
                    pixels[y * width + x] = 0;
                }
            }
        }
        let source = Luma8LuminanceSource::new(pixels, width as u32, height as u32).unwrap();
        let decoded = PDF417Reader.decode(&mut BinaryBitmap::new(HybridBinarizer::new(source))).unwrap();
        assert_eq!(decoded.getText(), payload);
    }

    #[test]
    fn fixed_pdf417_fields_fill_the_available_height() {
        let props = BTreeMap::from([
            ("ui.barcode.type".into(), "pdf417".into()),
            ("ui.barcode.moduleWidth".into(), "0.338mm".into()),
            ("ui.barcode.moduleHeight".into(), "0.676mm".into()),
            ("ui.barcode.errorCorrectionLevel".into(), "5".into()),
        ]);
        let rect = Rect::new(0.0, 0.0, 270.0, 144.0);
        let bars = bars("SYNTHETIC|ANSWER|2030-01-02|", &props, rect).unwrap();
        let bottom = bars.iter().map(|r| r.y + r.h).fold(0.0, f64::max);
        assert!(bottom > 140.0, "fixed-height field was left mostly empty: {bottom}");
        let right = bars.iter().map(|r| r.x + r.w).fold(0.0, f64::max);
        assert!(right > 250.0, "fixed-width field was left mostly empty: {right}");
    }

    #[test]
    fn malformed_or_oversized_barcodes_return_errors() {
        let mut props = BTreeMap::from([("ui.barcode.type".into(), "pdf417".into())]);
        let rect = Rect::new(0.0, 0.0, 270.0, 144.0);
        assert!(bars(&"9".repeat(10_841), &props, rect).is_err());
        assert!(bars("test", &props, Rect::new(0.0, 0.0, 1.0, 1.0)).is_err());
        props.insert("ui.barcode.moduleWidth".into(), "NaNpt".into());
        assert!(bars("test", &props, rect).is_err());
        props.remove("ui.barcode.moduleWidth");
        props.insert("ui.barcode.errorCorrectionLevel".into(), "4294967295".into());
        assert!(bars("test", &props, rect).is_err());
    }
}
