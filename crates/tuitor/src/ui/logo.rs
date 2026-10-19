//! The logo: six stripes, "the waves", running into the pixel wordmark "tuitor".
//! One logo pixel is one column by half a row: a `▀` in the upper pixel's colour
//! over the lower one's. The pixels are assets/logo_white.svg sampled on the
//! 9.954-unit grid its wordmark is drawn on.
//!
//! The stripe colours are bright, so the logo appears in full only where no data
//! is shown; the top bar gets a small muted mark.

use std::f32::consts::TAU;
use std::ops::Range;
use std::time::Duration;

use ratatui::style::Color;

use super::paint::Painter;
use super::theme::{BAR, BG, WHITE, bold, mix, rgb};

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
/// How much of the stripe colours the top bar's mark keeps.
const MARK_STRENGTH: f32 = 0.4;

/// One sway, in milliseconds.
const PERIOD_MS: u128 = 2800;
/// How far a left edge moves each way, in pixels.
const AMPLITUDE: f32 = 1.5;
/// Phase lag between neighbouring stripes, so the sway runs down the stack.
const LAG: f32 = 0.7;

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

/// The top bar's mark: the logo's second and third stripes with the step between
/// them, muted so they stay quieter than the colours that mean something.
/// Returns the end x.
pub fn mark(p: &mut Painter<'_>, x: u16, y: u16) -> u16 {
    let muted = |stripe: usize| mix(BAR, STRIPES[stripe].color, MARK_STRENGTH);
    put(p, x, y, None, Some(muted(2)));
    put(p, x + 1, y, Some(muted(1)), Some(muted(2)));
    put(p, x + 2, y, Some(muted(1)), Some(muted(2)));
    x + 3
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
}
