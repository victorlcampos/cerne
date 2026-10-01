//! Pictures of the screen for checking the design: a drawn buffer as plain
//! text or as an SVG with its colors, plus the `WIDTHxHEIGHT` size the apps'
//! `--snapshot` option takes.

use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

const CELL_WIDTH: f64 = 9.0;
const CELL_HEIGHT: f64 = 19.0;
const FONT_SIZE: f64 = 15.0;
const BACKGROUND: &str = "#15181e";
const FOREGROUND: &str = "#d6dae0";

/// A size such as "120x40": width and height in cells.
pub fn parse_size(text: &str) -> Result<(u16, u16), String> {
    let (width, height) = text
        .split_once('x')
        .ok_or_else(|| format!("expected WIDTHxHEIGHT, like 120x40, got {text:?}"))?;
    let number = |part: &str| {
        part.trim()
            .parse::<u16>()
            .map_err(|error| format!("{part:?}: {error}"))
    };
    Ok((number(width)?, number(height)?))
}

/// The screen as text, one line per row, without trailing spaces.
pub fn text(buf: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// The screen as an SVG picture, colors and bold, italic, dim, crossed-out
/// and underlined text included.
pub fn svg(buf: &Buffer) -> String {
    let width = f64::from(buf.area.width) * CELL_WIDTH + 24.0;
    let height = f64::from(buf.area.height) * CELL_HEIGHT + 24.0;
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><rect width="100%" height="100%" rx="10" fill="{BACKGROUND}"/><g transform="translate(12 12)" font-family="Menlo, 'SF Mono', monospace" font-size="{FONT_SIZE}">"#
    );
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let mut fg = color(cell.fg, FOREGROUND);
            let mut bg = color(cell.bg, BACKGROUND);
            if cell.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let (left, top) = (f64::from(x) * CELL_WIDTH, f64::from(y) * CELL_HEIGHT);
            if bg != BACKGROUND {
                let _ = write!(
                    out,
                    r#"<rect x="{left}" y="{top}" width="{CELL_WIDTH}" height="{CELL_HEIGHT}" fill="{bg}"/>"#
                );
            }
            let symbol = cell.symbol();
            if symbol.trim().is_empty() {
                continue;
            }
            let mut attributes = String::new();
            if cell.modifier.contains(Modifier::BOLD) {
                attributes.push_str(r#" font-weight="bold""#);
            }
            if cell.modifier.contains(Modifier::ITALIC) {
                attributes.push_str(r#" font-style="italic""#);
            }
            if cell.modifier.contains(Modifier::DIM) {
                attributes.push_str(r#" opacity="0.55""#);
            }
            if cell.modifier.contains(Modifier::CROSSED_OUT) {
                attributes.push_str(r#" text-decoration="line-through""#);
            } else if cell.modifier.contains(Modifier::UNDERLINED) {
                attributes.push_str(r#" text-decoration="underline""#);
            }
            let symbol = symbol
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let baseline = top + CELL_HEIGHT * 0.76;
            let _ = write!(
                out,
                r#"<text x="{left}" y="{baseline}" fill="{fg}"{attributes} xml:space="preserve">{symbol}</text>"#
            );
        }
    }
    out.push_str("</g></svg>\n");
    out
}

fn color(color: Color, reset: &str) -> String {
    let hex = |r: u8, g: u8, b: u8| format!("#{r:02x}{g:02x}{b:02x}");
    match color {
        Color::Reset => reset.to_string(),
        Color::Rgb(r, g, b) => hex(r, g, b),
        Color::Indexed(n) => {
            let (r, g, b) = xterm(n);
            hex(r, g, b)
        }
        named => {
            let (r, g, b) = match named {
                Color::Black => (29, 31, 33),
                Color::Red => (204, 102, 102),
                Color::Green => (181, 189, 104),
                Color::Yellow => (240, 198, 116),
                Color::Blue => (129, 162, 190),
                Color::Magenta => (178, 148, 187),
                Color::Cyan => (138, 190, 183),
                Color::Gray => (197, 200, 198),
                Color::DarkGray => (102, 102, 102),
                Color::LightRed => (213, 78, 83),
                Color::LightGreen => (185, 202, 74),
                Color::LightYellow => (231, 197, 71),
                Color::LightBlue => (122, 166, 218),
                Color::LightMagenta => (195, 151, 216),
                Color::LightCyan => (112, 192, 177),
                _ => (234, 234, 234),
            };
            hex(r, g, b)
        }
    }
}

/// The xterm 256-color palette.
fn xterm(n: u8) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match n {
        0..16 => BASE[usize::from(n)],
        16..232 => {
            let n = n - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(n / 36), level((n / 6) % 6), level(n % 6))
        }
        _ => {
            let gray = 8 + (n - 232) * 10;
            (gray, gray, gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    use super::*;

    fn screen() -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
        buf.set_string(0, 0, "a<b & c", Style::new().fg(Color::Rgb(1, 2, 3)));
        buf.set_string(0, 1, "bold", Style::new().bold().bg(Color::Indexed(196)));
        buf.set_string(5, 1, "link", Style::new().underlined());
        buf.set_string(0, 2, "gone", Style::new().crossed_out().underlined());
        buf
    }

    #[test]
    fn the_xterm_palette_has_its_corners() {
        assert_eq!(xterm(16), (0, 0, 0));
        assert_eq!(xterm(231), (255, 255, 255));
        assert_eq!(xterm(196), (255, 0, 0));
        assert_eq!(xterm(232), (8, 8, 8));
    }

    #[test]
    fn text_keeps_the_rows_and_drops_trailing_spaces() {
        assert_eq!(text(&screen()), "a<b & c\nbold link\ngone\n");
    }

    #[test]
    fn svg_escapes_text_and_keeps_colors_and_styles() {
        let picture = svg(&screen());
        assert!(picture.starts_with("<svg"));
        assert!(picture.ends_with("</svg>\n"));
        assert!(picture.contains(">&lt;</text>"));
        assert!(picture.contains(">&amp;</text>"));
        assert!(picture.contains(r##"fill="#010203""##));
        assert!(
            picture.contains(r##"fill="#ff0000""##),
            "the red background"
        );
        assert!(picture.contains(r#"font-weight="bold""#));
        assert!(picture.contains(r#"text-decoration="underline""#));
        assert!(
            !picture.contains(r#"text-decoration="line-through" text-decoration"#),
            "one decoration per letter"
        );
    }

    #[test]
    fn reads_sizes_like_120x40() {
        assert_eq!(parse_size("120x40"), Ok((120, 40)));
        assert_eq!(parse_size(" 80 x 24 "), Ok((80, 24)));
        assert!(parse_size("120").is_err());
        assert!(parse_size("120x").is_err());
        assert!(parse_size("ax40").is_err());
        assert!(parse_size("70000x40").is_err());
    }
}
