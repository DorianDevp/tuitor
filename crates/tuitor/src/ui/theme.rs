//! Colours follow one rule: chrome and data are grey; only three things are bright.
//! Amber is where execution is, green is what changed in this step, red is an error.
//!
//! Breakpoints are the one exception: each has the colour of its set, which is all a set is
//! known by. Their shape says what they do, and a hit is a ◉ at full colour on a band, so
//! that it reads the same whatever the set.

use ratatui::style::{Color, Modifier, Style};
use tuitor_core::breakpoints::{SET_COLORS, SetId};

pub const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const BG: Color = rgb(0x0c0c0e);
pub const BAR: Color = rgb(0x16161a);
pub const FG: Color = rgb(0xd6d6d6);
pub const WHITE: Color = rgb(0xf5f5f5);
pub const DIM: Color = rgb(0xa0a0a0);
pub const FAINT: Color = rgb(0x737373);
pub const GHOST: Color = rgb(0x53535a);
pub const LINE: Color = rgb(0x36363d);
pub const LINE_FOCUS: Color = rgb(0x87632a);
pub const TITLE: Color = rgb(0xc8c8c8);
pub const SEL: Color = rgb(0x1f1f25);
pub const SEL2: Color = rgb(0x2b2b33);
pub const CELL: Color = rgb(0x34343e);
pub const SNAPSHOT_BG: Color = rgb(0x22222a);
pub const TYPE: Color = rgb(0x86abb6);
pub const BAR_GREY: Color = rgb(0x878787);
pub const KEY: Color = rgb(0xd0d0d0);
/// Matches of a search in SOURCE, and the one the cursor went to.
pub const FOUND: Color = rgb(0x393945);
pub const FOUND_HERE: Color = rgb(0x6a6a7c);

pub const AMBER: Color = rgb(0xffa31a);
pub const AMBER_DIM: Color = rgb(0xa3732a);
pub const AMBER_BG: Color = rgb(0x2e1f08);
pub const GREEN: Color = rgb(0x3ddc84);
pub const GREEN_DIM: Color = rgb(0x74b892);
pub const RED: Color = rgb(0xff5252);
pub const RED_DIM: Color = rgb(0xc46262);
pub const YELLOW_DIM: Color = rgb(0xd3bb6e);

pub const SYN_KEYWORD: Color = rgb(0x96abc6);
pub const SYN_STRING: Color = rgb(0xbfb38a);
pub const SYN_NUMBER: Color = rgb(0xbea9cb);
pub const SYN_TYPE: Color = rgb(0xa6c2b6);
pub const SYN_PUNCT: Color = rgb(0x8e8e8e);
pub const SYN_VAR: Color = rgb(0xe2e2e2);

/// The colours of breakpoint sets, and the names commands know them by, in the order new
/// sets take them. From the logo's stripes, kept apart from the three bright meanings: its
/// coral and orange would read as an error and as execution, so they are left out, rose
/// is a pink lighter than the red of errors, and teal leans to cyan, away from the green
/// of changes. Sky, which the logo lacks, fills the gap between teal and orchid; gold, the
/// nearest to amber, comes last.
pub const SETS: [(&str, Color); SET_COLORS as usize] = [
    ("rose", rgb(0xf0909e)),
    ("teal", rgb(0x4cc2bd)),
    ("orchid", rgb(0xcf86eb)),
    ("sky", rgb(0x7ea9ee)),
    ("gold", rgb(0xdcc15c)),
];

/// A set's colour: whole for the active set, whose breakpoints stop the program, and half
/// toward the background for the others, which only record.
pub fn set_color(set: SetId, active: bool) -> Color {
    let color = SETS.get(usize::from(set)).map_or(DIM, |(_, color)| *color);
    if active { color } else { mix(color, BG, 0.45) }
}

pub fn set_name(set: SetId) -> &'static str {
    SETS.get(usize::from(set)).map_or("?", |(name, _)| name)
}

/// Muted colour per session tag so sessions are distinguishable but quiet.
pub fn tag(tag: &str) -> Color {
    match tag {
        "php" => rgb(0xa3a9e0),
        "js" | "ts" | "ng" | "web" => rgb(0xd898aa),
        "rs" | "native" => rgb(0xd4a886),
        "go" => rgb(0x82b3c0),
        _ => rgb(0xa899c4),
    }
}

/// `a` moved toward `b` by `amount` (0 is `a`, 1 is `b`).
pub fn mix(a: Color, b: Color, amount: f32) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else { return b };
    let channel = |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * amount).round() as u8;
    Color::Rgb(channel(r1, r2), channel(g1, g2), channel(b1, b2))
}

pub fn fg(color: Color) -> Style {
    Style::new().fg(color)
}

pub fn bold(color: Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

pub fn italic(color: Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::ITALIC)
}
