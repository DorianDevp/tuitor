//! The logo: six stripes, "the waves", running into the pixel wordmark "tuitor".
//! One logo pixel is one column by half a row: a `▀` in the upper pixel's colour
//! over the lower one's. The pixels are assets/logo_white.svg sampled on the
//! 9.954-unit grid its wordmark is drawn on.
//!
//! The logo appears in full only where no data is shown. The header gets a mark
//! three rows tall: the same logo at a third of the height, one pixel row per
//! stripe, its stripes coming out of the "t" of a pixel wordmark.

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
/// Time between frames while the logo sways: its edges glide, so every frame
/// differs. Nothing animated is redrawn more often.
pub const FRAME: Duration = Duration::from_millis(80);

/// The header's wordmark, a third of the big one's height: one pixel row per
/// stripe, the same letters with stems two pixels wide, the heart a dot. The
/// stripes run in the columns before the "t".
const MARK: [&str; 6] = [
    ".......#........##...#............",
    "......##............##............",
    ".....####.##.##.##.####..###..##.#",
    "......##..##.##.##..##..##.##.####",
    "......##..##.##.##..##..##.##.##..",
    ".......##..####.##...##..###..##..",
];
/// The mark's 6 pixel rows for the big logo's 19 from the top of the "t" to the base.
const MARK_SCALE: f32 = 6.0 / 19.0;
/// The mark's stripes end under its "t", which hides their right edges.
const MARK_STRIPE_END: i32 = 7;
/// Columns of the mark's "t" and the stripes left of it.
const MARK_T: u16 = 9;
/// The mark sways half as far as the big logo, rounded to whole pixels: each
/// stripe steps one pixel out and back, so it stays crisp and calm.
const MARK_SWAY: f32 = 0.5;
/// How finely the header looks ahead for the mark's next step.
const STEP: Duration = Duration::from_millis(10);
/// Rows of the header's mark.
pub const MARK_ROWS: u16 = 3;

/// The header's logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// The stripes coming out of the wordmark "tuitor".
    Word,
    /// Only the stripes and the "t" they come out of, for a narrow header.
    T,
}

impl Mark {
    /// Columns of the mark, without the column left of it that a stripe sways into.
    pub fn width(self) -> u16 {
        match self {
            Mark::Word => MARK[0].len() as u16,
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

/// Draws the header's mark with its first column at `x`; a stripe may sway one column left of it.
pub fn mark(p: &mut Painter<'_>, x: u16, y: u16, mark: Mark, t: Option<Duration>) {
    let shifts = shifts(t);
    for row in 0..MARK_ROWS {
        let upper = 2 * i32::from(row);
        for col in -1..i32::from(mark.width()) {
            let (Some(cx), Some(cy)) = (x.checked_add_signed(col as i16), y.checked_add(row)) else { continue };
            put(p, cx, cy, mark_pixel(mark, col, upper, &shifts), mark_pixel(mark, col, upper + 1, &shifts));
        }
    }
}

/// How far each of the mark's stripes is from rest at time `t`, in whole pixels:
/// the big logo's sway, scaled down and rounded. `None` is the still mark.
fn shifts(t: Option<Duration>) -> [i32; STRIPES.len()] {
    std::array::from_fn(|stripe| (sway(stripe, t) * MARK_SWAY).round() as i32)
}

/// How long after `t` the mark next changes, but at least a frame. It moves in
/// steps, so the loop redraws only for one.
pub fn next_step(t: Duration) -> Duration {
    let now = shifts(Some(t));
    let period = Duration::from_millis(PERIOD_MS as u64);
    let mut after = FRAME;
    while after < period && shifts(Some(t + after)) == now {
        after += STEP;
    }
    after
}

/// Colour of the mark's pixel (x, y) with its stripes shifted by `shifts`.
fn mark_pixel(mark: Mark, x: i32, y: i32, shifts: &[i32; STRIPES.len()]) -> Option<Color> {
    let row = usize::try_from(y).ok()?;
    let pixels = MARK.get(row)?.as_bytes();
    if usize::try_from(x).is_ok_and(|col| x < i32::from(mark.width()) && pixels[col] == b'#') {
        return Some(WHITE);
    }
    let stripe = &STRIPES[row];
    let left = (stripe.left * MARK_SCALE).round() as i32 + shifts[row];
    (left..MARK_STRIPE_END).contains(&x).then_some(stripe.color)
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
        let mut buf = Buffer::empty(Rect::new(0, 0, 1 + MARK[0].len() as u16 + 1, MARK_ROWS));
        super::mark(&mut Painter { buf: &mut buf }, 1, 0, mark, t);
        buf
    }

    /// The mark's stripe pixels, left to right, in row `y`.
    fn stripe_run(mark: Mark, y: i32, shifts: &[i32; STRIPES.len()]) -> Vec<i32> {
        (-2..i32::from(mark.width()) + 2)
            .filter(|&x| mark_pixel(mark, x, y, shifts).is_some_and(|c| c != WHITE))
            .collect()
    }

    #[test]
    fn mark_is_the_logo_at_a_third_of_its_height() {
        assert!(MARK.iter().all(|row| row.len() == MARK[0].len() && row.bytes().all(|b| b"#.".contains(&b))));
        assert_eq!(MARK.len(), STRIPES.len());
        assert_eq!((Mark::Word.width(), Mark::T.width(), MARK_ROWS), (34, 9, 3));
        // The stripes keep the big logo's stepped left edge, scaled down.
        let lefts: Vec<i32> = (0..6).map(|y| stripe_run(Mark::Word, y, &[0; 6])[0]).collect();
        assert_eq!(lefts, [2, 1, 0, 1, 2, 3]);
        // The "t" starts where the big one's crossbar does, scaled down, and stands
        // the full height; the narrow mark ends with it.
        let first_white = |y: usize| MARK[y].find('#');
        assert_eq!(first_white(2), Some((15.0 * MARK_SCALE).round() as usize));
        assert!((0..6).all(|y| first_white(y).is_some_and(|x| x < usize::from(MARK_T))));
        assert!(MARK.iter().all(|row| row[usize::from(MARK_T)..].starts_with('.')));
    }

    #[test]
    fn mark_stripes_come_out_of_the_t_however_they_sway() {
        for shift in -1..=1 {
            for mark in [Mark::Word, Mark::T] {
                for y in 0..6 {
                    let run = stripe_run(mark, y, &[shift; 6]);
                    // One unbroken stripe that runs up to the "t", which covers its end.
                    let t = MARK[y as usize].find('#').map_or(-1, |x| x as i32);
                    assert!(!run.is_empty() && run.windows(2).all(|w| w[1] == w[0] + 1), "row {y}");
                    assert_eq!(run.last(), Some(&(t - 1)), "row {y}, shift {shift}");
                    assert!((t..MARK_STRIPE_END).all(|x| mark_pixel(mark, x, y, &[shift; 6]) == Some(WHITE)));
                    assert!(run[0] >= -1, "the sway stays inside the column left of the mark");
                }
            }
        }
    }

    #[test]
    fn mark_sways_with_the_logo_one_pixel_each_way() {
        assert_eq!(shifts(None), [0; 6]);
        let period = PERIOD_MS as u64;
        let mut seen = [[false; 3]; 6];
        for ms in (0..period).step_by(10) {
            let now = shifts(at(ms));
            assert_eq!(now, shifts(at(ms + 1000 * period)));
            for (s, shift) in now.into_iter().enumerate() {
                assert!((-1..=1).contains(&shift), "{now:?} at {ms} ms");
                // In step with the big logo: out when its edge is out, in when it is in.
                assert!(shift == 0 || shift.signum() as f32 == sway(s, at(ms)).signum());
                seen[s][(shift + 1) as usize] = true;
            }
        }
        assert_eq!(seen, [[true; 3]; 6]);
        // The steps run down the stack.
        let first_out = |s: usize| (1..period).find(|&ms| shifts(at(ms))[s] == 1 && shifts(at(ms - 1))[s] == 0);
        assert!((0..5).all(|s| first_out(s) < first_out(s + 1)), "{:?}", (0..6).map(first_out).collect::<Vec<_>>());
    }

    #[test]
    fn mark_is_redrawn_only_when_it_steps() {
        let period = Duration::from_millis(PERIOD_MS as u64);
        for ms in (0..PERIOD_MS as u64).step_by(30) {
            let t = Duration::from_millis(ms);
            let after = next_step(t);
            assert!((FRAME..period).contains(&after), "{after:?} at {ms} ms");
            assert_ne!(shifts(Some(t + after)), shifts(Some(t)), "at {ms} ms");
            if after > FRAME {
                assert_eq!(shifts(Some(t + after - STEP)), shifts(Some(t)), "late at {ms} ms");
            }
            assert_ne!(render_mark(Mark::Word, Some(t + after)), render_mark(Mark::Word, Some(t)));
        }
        // Six stripes, each out and back both ways: under the frames the big logo draws.
        let (mut t, mut redraws) = (Duration::ZERO, 0);
        while t < period {
            t += next_step(t);
            redraws += 1;
        }
        assert!(redraws <= 24 && redraws < period.as_millis() / FRAME.as_millis(), "{redraws} redraws");
    }

    #[test]
    fn mark_draws_three_rows_of_half_blocks_and_rests_in_the_logo_shape() {
        let still = render_mark(Mark::Word, None);
        let yellow = STRIPES[2].color;
        // Yellow reaches furthest left, over nothing: its row's upper half.
        assert_eq!((still[(1, 1)].symbol(), still[(1, 1)].fg), ("▀", yellow));
        // Pink over red at the top, where the stripes meet the point of the "t".
        let top = &still[(1 + 7, 0)];
        assert_eq!((top.symbol(), top.fg, top.bg), ("▀", WHITE, WHITE));
        let edge = &still[(1 + 6, 0)];
        assert_eq!((edge.symbol(), edge.fg, edge.bg), ("▀", STRIPES[0].color, WHITE));
        // Nothing sways at rest, and the narrow mark is the wide one cut after its "t".
        assert_eq!(still[(0, 1)].symbol(), " ");
        let narrow = render_mark(Mark::T, None);
        for y in 0..MARK_ROWS {
            for x in 0..still.area.width {
                if x <= MARK_T {
                    assert_eq!(narrow[(x, y)], still[(x, y)], "({x}, {y})");
                } else {
                    assert_eq!(narrow[(x, y)].symbol(), " ", "({x}, {y})");
                }
            }
        }
        // Swaying moves only the stripes' left edges.
        let moved = render_mark(Mark::Word, at(700));
        for y in 0..MARK_ROWS {
            for x in 1 + MARK_STRIPE_END as u16..still.area.width {
                assert_eq!(moved[(x, y)], still[(x, y)]);
            }
        }
        assert_ne!(moved, still);
    }
}
