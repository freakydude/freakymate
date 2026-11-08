//! Filament colours as every source gives them: red, green, blue and an alpha channel (`ff` opaque, `00` clear).

/// Red, green, blue, alpha.
pub type Rgba = [u8; 4];

/// Hex `RRGGBB` or `RRGGBBAA`, with or without `#`; without alpha the colour is opaque.
pub fn parse_hex(text: &str) -> Option<Rgba> {
    let hex = text.trim().trim_start_matches('#');
    if !(hex.len() == 6 || hex.len() == 8) || !hex.is_ascii() {
        return None;
    }
    let channel = |index: usize| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok();
    let alpha = if hex.len() == 8 { channel(3)? } else { 0xff };
    Some([channel(0)?, channel(1)?, channel(2)?, alpha])
}

/// The colour without its alpha channel.
pub fn rgb([red, green, blue, _]: Rgba) -> [u8; 3] {
    [red, green, blue]
}

/// Fully clear: a transparent filament, whose colour is nothing to compare.
pub fn is_clear(color: Rgba) -> bool {
    color[3] == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_has_an_optional_alpha() {
        assert_eq!(parse_hex("#26A69A"), Some([0x26, 0xA6, 0x9A, 0xff]));
        assert_eq!(parse_hex("26a69a80"), Some([0x26, 0xA6, 0x9A, 0x80]));
        assert_eq!(parse_hex("#FFFFFF00").map(is_clear), Some(true));
        assert_eq!(parse_hex("12"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
        assert_eq!(parse_hex("26a69a8"), None);
    }
}
