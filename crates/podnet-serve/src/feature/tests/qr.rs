use super::*;

#[test]
fn a_qr_code_is_a_square_grid_with_a_quiet_zone() {
    let data = "https://example.trycloudflare.com";
    let code = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::L)
        .expect("encode");
    let rendered = render_qr(data).expect("render");
    let lines: Vec<&str> = rendered.lines().collect();

    // Measured behaviour of the encoder: one module per column, plus the
    // four-module quiet zone on each side. Asserted against the symbol
    // itself rather than a hard-coded number, so a different URL still
    // checks out.
    let expected_cols = code.width() + 8;
    assert_eq!(lines[0].chars().count(), expected_cols, "row width");
    // Blocks draws two lines per module row.
    assert_eq!(
        lines.len(),
        expected_cols * 2,
        "Blocks is square on a 1:2 cell"
    );

    for line in &lines {
        assert_eq!(
            line.chars().count(),
            expected_cols,
            "every row is the same width"
        );
    }

    // The quiet zone must be light on all four sides, or a scanner loses
    // the finder patterns.
    for (i, line) in lines.iter().enumerate() {
        let dark_edge = line.starts_with('█') || line.ends_with('█');
        let on_border = i < 4 || i >= lines.len() - 4;
        assert!(!(dark_edge && on_border), "the quiet zone must stay light");
    }
    for line in &lines[..4] {
        for c in line.chars().take(4) {
            assert_ne!(c, '█', "the left quiet zone must stay light");
        }
    }
}

#[test]
fn qr_styles_differ_in_height_and_keep_the_same_width() {
    let data = "https://a.trycloudflare.com";
    let code = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::L)
        .expect("encode");
    let cols = code.width() + 8;

    for (style, rows_per_module) in [
        (QrStyle::Blocks, 2),
        (QrStyle::Ascii, 1),
        (QrStyle::Unicode, 1),
    ] {
        let out = render_qr_with(data, style, qrcode::EcLevel::L).expect("render");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0].chars().count(), cols, "{style:?} width");
        assert_eq!(lines.len(), cols * rows_per_module, "{style:?} height");
    }

    // Blocks is the tall, square one; Ascii is the compact one.
    let blocks = render_qr_with(data, QrStyle::Blocks, qrcode::EcLevel::L).expect("blocks");
    let ascii = render_qr_with(data, QrStyle::Ascii, qrcode::EcLevel::L).expect("ascii");
    assert_ne!(blocks, ascii, "the styles must not render identically");
    assert!(
        blocks.lines().count() > ascii.lines().count(),
        "Blocks doubles the height so modules are square on screen"
    );
}

#[test]
fn a_higher_error_correction_level_produces_a_larger_symbol() {
    // Boundary behaviour of the encoder, and the reason the level is a
    // parameter rather than a constant.
    let data = "https://a.trycloudflare.com";
    let low = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::L)
        .expect("encode L");
    let high = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::H)
        .expect("encode H");
    assert!(
        high.width() >= low.width(),
        "H ({} modules) must not be smaller than L ({})",
        high.width(),
        low.width()
    );
}

#[test]
fn empty_and_oversized_input_are_qr_errors_not_panics() {
    assert!(matches!(render_qr(""), Err(FeatureError::Qr(_))));

    // A few kilobytes exceeds what any QR version holds at level L.
    let huge = "a".repeat(4000);
    match render_qr(&huge) {
        Err(FeatureError::Qr(m)) => assert!(m.contains("4000"), "{m}"),
        other => panic!("expected a Qr error, got {other:?}"),
    }
}

#[test]
fn qr_fits_reports_whether_the_symbol_survives_the_terminal_width() {
    let short = "https://a.trycloudflare.com";
    let long = "https://a-very-long-tunnel-hostname-that-fills-the-screen.trycloudflare.com";
    assert!(qr_fits(short, 200).expect("fits short"));
    assert!(
        !qr_fits(long, 40).expect("fits long"),
        "a wide symbol must be reported as not fitting a narrow terminal"
    );
    assert!(
        qr_fits(short, 1).is_ok(),
        "a narrow terminal is an answer, not an error"
    );
}
