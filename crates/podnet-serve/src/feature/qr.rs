//! QR rendering of a URL for a terminal.

use super::*;

// ---------------------------------------------------------------------------
// QR code
// ---------------------------------------------------------------------------

/// How to draw a QR code in a terminal.
///
/// All three draw one module per character column, so the *width* is the same
/// for all of them; they differ in height and in which glyph is used. That is
/// the trade the operator is choosing between: a symbol tall enough to be
/// square on a normal terminal, or a compact one that fits a narrow window and
/// can be pasted into plain text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QrStyle {
    /// A module is one column by two lines. Character cells are roughly twice
    /// as tall as they are wide, so this is what makes the modules square and
    /// the symbol actually scannable. The default.
    #[default]
    Blocks,
    /// One line per module row, `#` for dark. Half the height of
    /// [`QrStyle::Blocks`], at the cost of vertically squashed modules. Use it
    /// when the symbol must survive being pasted into a channel with plain
    /// text, or when the terminal's font has no block glyphs.
    Ascii,
    /// One line per module row, inverted (`█` on a light background becomes a
    /// dark-on-light figure). Included for terminals whose light-on-dark
    /// rendering of blocks washes out.
    Unicode,
}

impl QrStyle {
    fn render(&self, code: &qrcode::QrCode) -> String {
        let (dark, light, rows) = match self {
            QrStyle::Blocks => ('█', ' ', 2),
            QrStyle::Ascii => ('#', ' ', 1),
            QrStyle::Unicode => (' ', '█', 1),
        };
        code.render::<char>()
            .dark_color(dark)
            .light_color(light)
            .quiet_zone(true)
            .module_dimensions(1, rows)
            .build()
    }
}

/// Render `data` as a QR code.
///
/// Uses `EcLevel::L`: a tunnel URL is shown on a screen and scanned from the
/// same screen a moment later, where the failure mode is a smudge rather than
/// fading, and the lower level keeps the symbol small enough to fit. Use
/// [`render_qr_with`] when the output has to survive being printed on paper.
pub fn render_qr(data: &str) -> Result<String, FeatureError> {
    render_qr_with(data, QrStyle::default(), qrcode::EcLevel::L)
}

/// Render `data` with an explicit style and error-correction level.
///
/// The level is a parameter because the two consumers want different things: a
/// live terminal wants the smallest symbol, a printed label wants the most
/// redundant one. Defaulting one for both gets one of them wrong.
pub fn render_qr_with(
    data: &str,
    style: QrStyle,
    ec_level: qrcode::EcLevel,
) -> Result<String, FeatureError> {
    if data.is_empty() {
        return Err(FeatureError::Qr(
            "nothing to encode; a QR code needs at least one character".to_string(),
        ));
    }
    let code = qrcode::QrCode::with_error_correction_level(data.as_bytes(), ec_level)
        .map_err(|e| FeatureError::Qr(format!("{e} for {} bytes of input", data.len())))?;
    Ok(style.render(&code))
}

/// Whether a symbol for `data` fits `columns` terminal columns.
///
/// Used to decide between printing the QR code and printing the URL alone,
/// rather than emitting a figure that wraps into an unscannable mess.
///
/// Width only. Every style in [`QrStyle`] draws one module per column, so the
/// style changes the height, never the number of columns; a style-aware
/// height check belongs to whatever is doing the drawing.
///
/// There is no style parameter because there is nothing for it to decide here.
pub fn qr_fits(data: &str, columns: u32) -> Result<bool, FeatureError> {
    let code = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::L)
        .map_err(|e| FeatureError::Qr(format!("{e} for {} bytes of input", data.len())))?;
    // +8 is the quiet zone the renderer adds on all sides, 4 modules per side,
    // which at one module per column is four extra columns on each edge.
    Ok(u32::try_from(code.width()).unwrap_or(u32::MAX) + 8 <= columns)
}
