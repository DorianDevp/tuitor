//! The logo: six stripes, "the waves", running into the pixel wordmark "tuitor".
//! One logo pixel is one column by half a row: a `▀` in the upper pixel's colour
//! over the lower one's. The pixels are assets/logo_white.svg sampled on the
//! 9.954-unit grid its wordmark is drawn on.
//!
//! The logo appears in full only where no data is shown. The header gets a mark two
//! rows tall, drawn with octants: the wordmark from the top of the "t" to the base,
//! with four stripes, one per half row, running into it. While nothing waits to be
//! read their colours sweep through the logo's, from its top stripe to its bottom
//! one and back.

use std::f32::consts::TAU;
use std::ops::Range;
use std::time::Duration;

use ratatui::style::Color;

use super::paint::Painter;
use super::theme::{BG, WHITE, bold, mix, rgb};

/// 89 × 21 pixels. The heart over the "i" uses finer cells in the SVG and is
/// copied cell for cell, so it keeps its notch and point.
const WORDMARK: [&str; 21] = [
    "..........................................##.##..........................................",
    ".........................................#######.........................................",
    "....................#....................#######.......#.................................",
    "...................##.....................#####.......##.................................",
    "..................###......................###.......###.................................",
    ".................####.......................#.......####.................................",
    ".................####......####.....####............####.........#######......####.####..",
    "...............#########...####.....####...####...#########....###########....##########.",
    "...............#########...####.....####...####...#########....###########....###########",
    "...............#########...####.....####...####...#########...#####...#####...#####..###.",
    ".................####......####.....####...####.....####......####.....####...####....#..",
    ".................####......####.....####...####.....####......####.....####...####.......",
    ".................####......####.....####...####.....####......####.....####...####.......",
    ".................####......####.....####...####.....####......####.....####...####.......",
    ".................####......####.....####...####.....####......####.....####...####.......",
    ".................####......####.....####...####.....####......####.....####...####.......",
    ".................####......####.....####...####.....####......####.....####...####.......",
    ".................#####.....#####...#####...####.....#####.....#####...#####...####.......",
    "..................######....############...####......######....###########....####.......",
    "..................######....############...####......######....###########....####.......",
    "....................####......#####.####...####........####......#######......####.......",
];

struct Stripe {
    /// Pixel rows, as the SVG's stripes cover them.
    rows: Range<i32>,
    /// Left edge in pixels, unrounded, so the half-size mark scales from the SVG.
    left: f32,
    color: Color,
}

const STRIPES: [Stripe; 6] = [
    Stripe { rows: 2..5, left: 6.479, color: rgb(0xff9292) },
    Stripe { rows: 5..8, left: 3.138, color: rgb(0xff6557) },
    Stripe { rows: 8..12, left: 0.225, color: rgb(0xffd630) },
    Stripe { rows: 12..15, left: 2.167, color: rgb(0xfeb848) },
    Stripe { rows: 15..18, left: 5.676, color: rgb(0xe65df3) },
    Stripe { rows: 18..21, left: 8.890, color: rgb(0x11ad89) },
];

/// Width of the whole logo in pixels.
const WIDTH: u16 = 89;
/// The stripes end under the "t", which hides their right edges.
const STRIPE_END: f32 = 20.0;
/// Columns left of the logo that a swaying stripe may reach.
pub const PAD: u16 = 2;
const NAME: &str = "tuitor";

/// One sway, in milliseconds.
const PERIOD_MS: u128 = 2800;
/// How far a left edge moves each way, in pixels.
const AMPLITUDE: f32 = 1.5;
/// Phase lag between neighbouring stripes, so the sway runs down the stack.
const LAG: f32 = 0.7;
/// Time between frames while the logo sways or the mark's colours roll: both glide,
/// so every frame differs. Nothing animated is redrawn more often.
pub const FRAME: Duration = Duration::from_millis(80);

/// The header's wordmark, eight pixels from the top of the "t" to the base: the
/// big one's letters with stems two pixels wide, the heart a dot over the "i".
/// A pixel is an octant, half a column by a quarter of a row.
const MARK: [&str; 8] = [
    ".........#..........##....#...............",
    "........##...............##...............",
    "......######.##..##.##.######..####..##.##",
    "......######.##..##.##.######.##..##.#####",
    "........##...##..##.##...##...##..##.##..#",
    "........##...##..##.##...##...##..##.##...",
    "........###..##..##.##...###..##..##.##...",
    ".........###..#####.##....###..####..##...",
];
/// Where the mark's four stripes start, top to bottom, one per half row: the logo's
/// stepped edge in whole columns, as a cell holds only two colours.
const MARK_LEFTS: [i32; 4] = [2, 0, 2, 4];
/// The stripes run up to the stem of the "t"; its crossbar covers the second one's end.
const MARK_STEM: i32 = 8;
/// Each stripe's place among the stripes' colours at rest: the colour the logo has
/// at that height.
const MARK_COLOURS: [f32; 4] = [0.0, 2.0, 3.0, 5.0];
/// Columns of the mark's "t" and the stripes left of it.
const MARK_T: u16 = 6;
/// Pixel rows free above the mark, so that it stands centred beside three rows of text.
const MARK_TOP: i32 = 2;
/// Rows of the header's mark: its two rows and half a row free above and below.
pub const MARK_ROWS: u16 = 3;
/// One sweep of the mark's colours down the logo's stripes and back, in milliseconds.
const ROLL_MS: u128 = 6000;
/// Block elements that Unicode encodes outside the octants, by the octants they
/// fill: bit `n` is octant `n + 1`, left to right and top to bottom.
const NOT_OCTANTS: [(u8, char); 26] = [
    (0x00, ' '),
    (0x01, '\u{1CEA8}'),
    (0x02, '\u{1CEAB}'),
    (0x03, '\u{1FB82}'),
    (0x05, '▘'),
    (0x0A, '▝'),
    (0x0F, '▀'),
    (0x14, '\u{1FBE6}'),
    (0x28, '\u{1FBE7}'),
    (0x3F, '\u{1FB85}'),
    (0x40, '\u{1CEA3}'),
    (0x50, '▖'),
    (0x55, '▌'),
    (0x5A, '▞'),
    (0x5F, '▛'),
    (0x80, '\u{1CEA0}'),
    (0xA0, '▗'),
    (0xA5, '▚'),
    (0xAA, '▐'),
    (0xAF, '▜'),
    (0xC0, '▂'),
    (0xF0, '▄'),
    (0xF5, '▙'),
    (0xFA, '▟'),
    (0xFC, '▆'),
    (0xFF, '█'),
];

/// The header's logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// The stripes coming out of the wordmark "tuitor".
    Word,
    /// Only the stripes and the "t" they come out of, for a narrow header.
    T,
}

impl Mark {
    /// Columns of the mark.
    pub fn width(self) -> u16 {
        match self {
            Mark::Word => (MARK[0].len() / 2) as u16,
            Mark::T => MARK_T,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// The whole logo, 11 rows.
    Full,
    /// For small panes: one pixel row per stripe at half the width, and the name as text.
    Small,
}

impl Size {
    /// Columns and rows of the logo, without the sway room left of it.
    pub fn cells(self) -> (u16, u16) {
        match self {
            Size::Full => (WIDTH, 11),
            Size::Small => (self.pixels_wide() + 1 + NAME.len() as u16, 3),
        }
    }

    /// Where text under the logo lines up: the wordmark's "t", or the stripes.
    pub fn indent(self) -> u16 {
        match self {
            Size::Full => 15,
            Size::Small => 0,
        }
    }

    fn scale(self) -> f32 {
        match self {
            Size::Full => 1.0,
            Size::Small => 0.5,
        }
    }

    /// Pixels drawn as half blocks; the small logo's name is text after them.
    fn pixels_wide(self) -> u16 {
        match self {
            Size::Full => WIDTH,
            Size::Small => (STRIPE_END * self.scale()) as u16,
        }
    }

    fn pixel_rows(self) -> i32 {
        match self {
            Size::Full => WORDMARK.len() as i32,
            Size::Small => STRIPES.len() as i32,
        }
    }

    fn stripe_rows(self, stripe: usize) -> Range<i32> {
        match self {
            Size::Full => STRIPES[stripe].rows.clone(),
            Size::Small => stripe as i32..stripe as i32 + 1,
        }
    }
}

/// How far stripe `stripe`'s left edge is from rest at time `t`, in pixels.
/// `None` is the still logo.
pub fn sway(stripe: usize, t: Option<Duration>) -> f32 {
    let Some(t) = t else { return 0.0 };
    // The remainder keeps the phase exact however long tuitor has been open.
    let phase = (t.as_millis() % PERIOD_MS) as f32 / PERIOD_MS as f32;
    AMPLITUDE * (TAU * phase - LAG * stripe as f32).sin()
}

/// Draws the logo with its first column at `x`; stripes may sway up to `PAD` columns left of it.
pub fn draw(p: &mut Painter<'_>, x: u16, y: u16, size: Size, t: Option<Duration>) {
    let (_, rows) = size.cells();
    let wide = size.pixels_wide();
    // An odd pixel height leaves the first row's upper half empty, so the base sits on a cell edge.
    let first = -(size.pixel_rows() % 2);
    for row in 0..rows {
        let upper = first + 2 * i32::from(row);
        for col in -i32::from(PAD)..i32::from(wide) {
            let (Some(cx), Some(cy)) = (x.checked_add_signed(col as i16), y.checked_add(row)) else { continue };
            put(p, cx, cy, pixel(size, col, upper, t), pixel(size, col, upper + 1, t));
        }
    }
    if size == Size::Small {
        p.text(x + wide + 1, y + 1, NAME, bold(WHITE), u16::MAX);
    }
}

/// Draws the header's mark with its first column at `x`.
pub fn mark(p: &mut Painter<'_>, x: u16, y: u16, mark: Mark, t: Option<Duration>) {
    for row in 0..MARK_ROWS {
        for col in 0..mark.width() {
            let (Some(cx), Some(cy)) = (x.checked_add(col), y.checked_add(row)) else { continue };
            let pixels: [Option<Color>; 8] = std::array::from_fn(|i| {
                let (px, py) = (2 * i32::from(col) + (i % 2) as i32, 4 * i32::from(row) + (i / 2) as i32);
                mark_pixel(mark, px, py - MARK_TOP, t)
            });
            let Some(fg) = pixels.iter().flatten().next().copied() else { continue };
            let bits = (0..8).filter(|&i| pixels[i] == Some(fg)).fold(0, |bits, i| bits | 1 << i);
            let Some(cell) = p.buf.cell_mut((cx, cy)) else { continue };
            cell.set_char(octant(bits)).set_fg(fg);
            // A second colour is the background; the mark never puts a third in a cell.
            if let Some(&bg) = pixels.iter().flatten().find(|&&c| c != fg) {
                cell.set_bg(bg);
            }
        }
    }
}

/// Colour of the mark's pixel (x, y) at time `t`, or `None` where the background shows.
fn mark_pixel(mark: Mark, x: i32, y: i32, t: Option<Duration>) -> Option<Color> {
    let (Ok(col), Ok(row)) = (usize::try_from(x), usize::try_from(y)) else { return None };
    let pixels = MARK.get(row)?.as_bytes();
    if col >= 2 * usize::from(mark.width()) {
        return None;
    }
    if pixels.get(col) == Some(&b'#') {
        return Some(WHITE);
    }
    let stripe = row / 2;
    (MARK_LEFTS[stripe]..MARK_STEM).contains(&x).then(|| roll(stripe, t))
}

/// Colour of the mark's stripe `stripe` at time `t`. The colours sweep down the
/// logo's stripes and back every `ROLL_MS`, blending each only into the one next to
/// it, as the stripes lie in the logo; `None` holds them at rest.
fn roll(stripe: usize, t: Option<Duration>) -> Color {
    let last = (STRIPES.len() - 1) as f32;
    // The remainder keeps the phase exact however long tuitor has been open.
    let shift = t.map_or(0.0, |t| (t.as_millis() % ROLL_MS) as f32 / ROLL_MS as f32 * 2.0 * last);
    let at = (MARK_COLOURS[stripe] - shift).rem_euclid(2.0 * last);
    let at = if at > last { 2.0 * last - at } else { at };
    let from = (at as usize).min(STRIPES.len() - 2);
    blend(STRIPES[from].color, STRIPES[from + 1].color, at - from as f32)
}

/// `mix` around the colour wheel: hue, saturation and lightness each move from `a`
/// to `b`, the hue the short way, so that no step between two bright colours greys.
fn blend(a: Color, b: Color, amount: f32) -> Color {
    if amount <= 0.0 {
        return a;
    }
    if amount >= 1.0 {
        return b;
    }
    let ((h1, s1, l1), (h2, s2, l2)) = (hsl(a), hsl(b));
    let turn = (h2 - h1 + 540.0) % 360.0 - 180.0;
    from_hsl((h1 + turn * amount).rem_euclid(360.0), s1 + (s2 - s1) * amount, l1 + (l2 - l1) * amount)
}

/// Hue in degrees, saturation and lightness of an RGB colour.
fn hsl(color: Color) -> (f32, f32, f32) {
    let Color::Rgb(r, g, b) = color else { return (0.0, 0.0, 1.0) };
    let [r, g, b] = [r, g, b].map(|v| f32::from(v) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let (lightness, chroma) = ((max + min) / 2.0, max - min);
    if chroma == 0.0 {
        return (0.0, 0.0, lightness);
    }
    let hue = if max == r {
        ((g - b) / chroma).rem_euclid(6.0)
    } else if max == g {
        (b - r) / chroma + 2.0
    } else {
        (r - g) / chroma + 4.0
    };
    (hue * 60.0, chroma / (1.0 - (2.0 * lightness - 1.0).abs()), lightness)
}

fn from_hsl(hue: f32, saturation: f32, lightness: f32) -> Color {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let x = chroma * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (hue / 60.0) as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let channel = |v: f32| ((v + lightness - chroma / 2.0) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::Rgb(channel(r), channel(g), channel(b))
}

/// The character that fills the octants in `bits` (see `NOT_OCTANTS`).
fn octant(bits: u8) -> char {
    if let Some(&(_, c)) = NOT_OCTANTS.iter().find(|(b, _)| *b == bits) {
        return c;
    }
    // U+1CD00 on holds the rest in the order of their bits.
    let before = NOT_OCTANTS.iter().filter(|(b, _)| *b < bits).count() as u32;
    char::from_u32(0x1CD00 + u32::from(bits) - before).unwrap_or(' ')
}

/// Colour of pixel (x, y), or `None` where the background shows.
fn pixel(size: Size, x: i32, y: i32, t: Option<Duration>) -> Option<Color> {
    if size == Size::Full && wordmark(x, y) {
        return Some(WHITE);
    }
    let stripe = (0..STRIPES.len()).find(|&s| size.stripe_rows(s).contains(&y))?;
    let scale = size.scale();
    if x as f32 >= STRIPE_END * scale {
        return None;
    }
    let left = (STRIPES[stripe].left * scale).round() + sway(stripe, t) * scale;
    // The share of the pixel right of the edge: blending the edge pixel lets the sway glide.
    let cover = (x as f32 + 1.0 - left).clamp(0.0, 1.0);
    (cover > 0.0).then(|| mix(BG, STRIPES[stripe].color, cover))
}

fn wordmark(x: i32, y: i32) -> bool {
    let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else { return false };
    WORDMARK.get(y).and_then(|row| row.as_bytes().get(x)) == Some(&b'#')
}

/// One cell holding two pixels.
fn put(p: &mut Painter<'_>, x: u16, y: u16, upper: Option<Color>, lower: Option<Color>) {
    let Some(cell) = p.buf.cell_mut((x, y)) else { return };
    match (upper, lower) {
        (Some(upper), lower) => {
            cell.set_symbol("▀").set_fg(upper);
            if let Some(lower) = lower {
                cell.set_bg(lower);
            }
        }
        (None, Some(lower)) => {
            cell.set_symbol("▄").set_fg(lower);
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::*;

    fn at(ms: u64) -> Option<Duration> {
        Some(Duration::from_millis(ms))
    }

    fn render(size: Size, t: Option<Duration>) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, PAD + WIDTH, 11));
        draw(&mut Painter { buf: &mut buf }, PAD, 0, size, t);
        buf
    }

    #[test]
    fn pixel_data_fits_the_logo() {
        assert_eq!(WORDMARK.len(), 21);
        assert!(WORDMARK.iter().all(|row| row.len() == usize::from(WIDTH) && row.bytes().all(|b| b"#.".contains(&b))));
        assert_eq!(Size::Full.cells(), (WIDTH, 11));
        assert_eq!(Size::Small.cells(), (17, 3));
        // The stripes stack from the top of the "t" to the base, one after another.
        assert_eq!(STRIPES[0].rows.start, 2);
        assert!(STRIPES.windows(2).all(|pair| pair[0].rows.end == pair[1].rows.start));
        assert_eq!(STRIPES[5].rows.end, 21);
    }

    #[test]
    fn sway_is_bounded_periodic_and_runs_down_the_stack() {
        assert!((0..STRIPES.len()).all(|s| sway(s, None) == 0.0));
        let period = PERIOD_MS as u64;
        for ms in (0..period).step_by(70) {
            for s in 0..STRIPES.len() {
                assert!(sway(s, at(ms)).abs() <= AMPLITUDE);
                assert_eq!(sway(s, at(ms)), sway(s, at(ms + 1000 * period)));
            }
        }
        assert!(f32::from(PAD) >= AMPLITUDE, "the sway stays inside the room left of the logo");
        let peak = |s| (0..period).max_by(|a, b| sway(s, at(*a)).total_cmp(&sway(s, at(*b))));
        assert!(peak(0) < peak(1) && peak(1) < peak(5));
    }

    #[test]
    fn still_logo_is_crisp_and_sits_on_its_last_row() {
        let colors: Vec<Color> = STRIPES.iter().map(|s| s.color).chain([WHITE]).collect();
        for y in 0..21 {
            for x in -i32::from(PAD)..i32::from(WIDTH) {
                assert!(pixel(Size::Full, x, y, None).is_none_or(|c| colors.contains(&c)), "({x}, {y})");
            }
        }
        let still = render(Size::Full, None);
        // The heart's top row is the lower half of the first row.
        assert_eq!((still[(PAD + 42, 0)].symbol(), still[(PAD + 42, 0)].fg), ("▄", WHITE));
        let foot = &still[(PAD + 20, 10)];
        assert_eq!((foot.symbol(), foot.fg, foot.bg), ("▀", WHITE, WHITE));
    }

    #[test]
    fn waves_move_and_the_wordmark_stays() {
        let (a, b) = (render(Size::Full, at(0)), render(Size::Full, at(700)));
        let stripe_cols = PAD + STRIPE_END as u16;
        let moved = (0..11).flat_map(|y| (0..stripe_cols).map(move |x| (x, y))).filter(|&p| a[p] != b[p]).count();
        assert!(moved >= 6, "only {moved} cells moved");
        for y in 0..11 {
            for x in stripe_cols..PAD + WIDTH {
                assert_eq!(a[(x, y)], b[(x, y)]);
            }
        }
        for t in [at(0), at(700), at(1900)] {
            for y in 0..21 {
                for x in 0..i32::from(WIDTH) {
                    if wordmark(x, y) {
                        assert_eq!(pixel(Size::Full, x, y, t), Some(WHITE));
                    }
                }
            }
        }
        assert_eq!(render(Size::Full, None), render(Size::Full, None));
    }

    fn render_mark(mark: Mark, t: Option<Duration>) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, Mark::Word.width() + 1, MARK_ROWS));
        super::mark(&mut Painter { buf: &mut buf }, 0, 0, mark, t);
        buf
    }

    #[test]
    fn octants_have_a_character_each() {
        let chars: std::collections::HashSet<char> = (0..=255).map(octant).collect();
        assert_eq!(chars.len(), 256);
        assert_eq!((octant(0x00), octant(0x0F), octant(0xF0), octant(0xFF)), (' ', '▀', '▄', '█'));
        // The first and last octant: 3 alone, and all but 1.
        assert_eq!((octant(0x04), octant(0xFE)), ('\u{1CD00}', '\u{1CDE5}'));
        let octants = (0..=255).filter(|b| NOT_OCTANTS.iter().all(|(n, _)| n != b));
        assert!(octants.map(octant).all(|c| ('\u{1CD00}'..='\u{1CDE5}').contains(&c)));
    }

    #[test]
    fn mark_is_the_wordmark_with_four_stripes_as_tall() {
        assert!(MARK.iter().all(|row| row.len() == MARK[0].len() && row.bytes().all(|b| b"#.".contains(&b))));
        assert_eq!(MARK.len(), 2 * MARK_LEFTS.len());
        assert_eq!((Mark::Word.width(), Mark::T.width(), MARK_ROWS), (21, 6, 3));
        assert_eq!(2 * usize::from(Mark::Word.width()), MARK[0].len());
        // The narrow mark ends with the "t".
        assert!(MARK.iter().all(|row| row[2 * usize::from(MARK_T)..].starts_with('.')));
        for y in 0..8 {
            // A stripe in every row from the top of the "t" to the base, with the
            // logo's stepped edge, running unbroken into the "t".
            let run: Vec<i32> = (0..2 * i32::from(MARK_T))
                .filter(|&x| mark_pixel(Mark::Word, x, y, None).is_some_and(|c| c != WHITE))
                .collect();
            assert_eq!(run[0], MARK_LEFTS[y as usize / 2], "row {y}");
            assert!(run.windows(2).all(|w| w[1] == w[0] + 1), "row {y}");
            let next = (run[run.len() - 1] + 1..).find_map(|x| mark_pixel(Mark::Word, x, y, None));
            assert_eq!(next, Some(WHITE), "row {y}");
        }
    }

    #[test]
    fn mark_puts_at_most_two_colours_in_a_cell() {
        for ms in (0..ROLL_MS as u64).step_by(100) {
            for mark in [Mark::Word, Mark::T] {
                for row in 0..i32::from(MARK_ROWS) {
                    for col in 0..i32::from(mark.width()) {
                        let mut colours: Vec<Option<Color>> = (0..8)
                            .map(|i| mark_pixel(mark, 2 * col + i % 2, 4 * row + i / 2 - MARK_TOP, at(ms)))
                            .collect();
                        colours.sort_by_key(|c| format!("{c:?}"));
                        colours.dedup();
                        assert!(colours.len() <= 2, "({col}, {row}) at {ms} ms: {colours:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn mark_colours_sweep_down_the_logo_stripes_and_back() {
        let rest: Vec<Color> = (0..4).map(|s| roll(s, None)).collect();
        assert_eq!(rest, [0, 2, 3, 5].map(|s| STRIPES[s].color));
        let period = ROLL_MS as u64;
        // Half a sweep on, the colours stand the other way round.
        assert_eq!((0..4).map(|s| roll(s, at(period / 2))).collect::<Vec<_>>(), [5, 3, 2, 0].map(|s| STRIPES[s].color));
        let frames = period / FRAME.as_millis() as u64;
        for s in 0..4 {
            let colour = |frame: u64| roll(s, at(frame * FRAME.as_millis() as u64));
            for frame in 0..frames {
                assert_eq!(colour(frame), roll(s, at(frame * FRAME.as_millis() as u64 + 1000 * period)));
                // Between neighbouring stripes of the logo nothing greys.
                assert!(hsl(colour(frame)).1 > 0.8, "stripe {s}, frame {frame}: {:?}", colour(frame));
            }
            // The colours glide: they change every frame but where a sweep turns.
            let held = (0..frames).filter(|&f| colour(f) == colour(f + 1)).count();
            assert!(held <= 2, "stripe {s} holds its colour over {held} frames");
        }
    }

    #[test]
    fn mark_stands_centred_in_three_rows_and_only_its_stripes_change() {
        let still = render_mark(Mark::Word, None);
        let (pink, yellow, orange) = (STRIPES[0].color, STRIPES[2].color, STRIPES[3].color);
        // Half a row free above: the top stripe is the first row's lower half.
        assert_eq!((still[(1, 0)].symbol(), still[(1, 0)].fg), ("▄", pink));
        // The second stripe reaches furthest left, over nothing.
        assert_eq!((still[(0, 1)].symbol(), still[(0, 1)].fg), ("▀", yellow));
        let two = &still[(1, 1)];
        assert_eq!((two.symbol(), two.fg, two.bg), ("▀", yellow, orange));
        // The crossbar of the "t" over the third stripe.
        let bar = &still[(3, 1)];
        assert_eq!((bar.symbol(), bar.fg, bar.bg), ("▀", WHITE, orange));
        // Half a row free below: the bottom stripe is the last row's upper half.
        assert_eq!((still[(2, 2)].symbol(), still[(2, 2)].fg), ("▀", STRIPES[5].color));
        // The narrow mark is the wide one cut after its "t".
        let narrow = render_mark(Mark::T, None);
        for y in 0..MARK_ROWS {
            for x in 0..still.area.width {
                if x < MARK_T {
                    assert_eq!(narrow[(x, y)], still[(x, y)], "({x}, {y})");
                } else {
                    assert_eq!(narrow[(x, y)].symbol(), " ", "({x}, {y})");
                }
            }
        }
        // Rolling changes the stripes' colours only.
        let rolled = render_mark(Mark::Word, at(700));
        assert_ne!(rolled, still);
        for y in 0..MARK_ROWS {
            for x in 0..still.area.width {
                assert_eq!(rolled[(x, y)].symbol(), still[(x, y)].symbol(), "({x}, {y})");
                if x >= MARK_T {
                    assert_eq!(rolled[(x, y)], still[(x, y)], "({x}, {y})");
                }
            }
        }
    }
}
