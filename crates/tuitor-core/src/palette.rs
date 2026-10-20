//! The colours of breakpoint sets. A set has no name: it is known by its colour, which is
//! chosen when the set is made, as far as can be from the colours of the other sets and from
//! the colours that mean a state: amber where execution is, green what changed in this step,
//! red an error, and the yellow of a warning. Distances are ΔE in OKLab, where equal numbers
//! look equally far apart.
//!
//! The candidates are a sweep of hues at one lightness and chroma, which read well on the
//! dark background, then the same hues lighter and darker for when those get crowded. A new
//! set takes the candidate whose nearest colour is farthest, as long as that is [`DISTINCT`]
//! at least; when no candidate is, the bar is lowered a step at a time, so that there can be
//! any number of sets, the later ones closer together.

use std::fmt;
use std::sync::OnceLock;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A breakpoint set, which is its colour, `0xRRGGBB`: no two sets have the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SetId(u32);

impl SetId {
    pub const fn from_rgb(rgb: u32) -> SetId {
        SetId(rgb & 0x00ff_ffff)
    }

    /// `0xRRGGBB`.
    pub const fn rgb(self) -> u32 {
        self.0
    }

    /// A set read from a file that names none this tuitor knows, to be left out.
    const UNKNOWN: SetId = SetId(u32::MAX);

    pub fn is_known(self) -> bool {
        self.0 <= 0x00ff_ffff
    }

    /// The first of the colours sets had before they were chosen, rose, which breakpoints
    /// saved before there were sets are in.
    pub const FIRST_OF_OLD: SetId = SetId(OLD[0].0);
}

impl fmt::Display for SetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:06x}", self.0)
    }
}

/// Saved as `"#rrggbb"`.
impl Serialize for SetId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Read from `"#rrggbb"`, or from a number, the place of its colour among the five sets had
/// before they were chosen ([`OLD`]), as files from then have it. Anything else is a set no
/// longer known, left out, so that a file with one still loads.
impl<'de> Deserialize<'de> for SetId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<SetId, D::Error> {
        struct Id;
        impl Visitor<'_> for Id {
            type Value = SetId;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a colour, #rrggbb")
            }

            fn visit_u64<E: de::Error>(self, n: u64) -> Result<SetId, E> {
                let old = usize::try_from(n).ok().and_then(|n| OLD.get(n));
                Ok(old.map_or(SetId::UNKNOWN, |(rgb, _)| SetId(*rgb)))
            }

            fn visit_i64<E: de::Error>(self, n: i64) -> Result<SetId, E> {
                u64::try_from(n).map_or(Ok(SetId::UNKNOWN), |n| self.visit_u64(n))
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<SetId, E> {
                Ok(hex(text).unwrap_or(SetId::UNKNOWN))
            }
        }
        deserializer.deserialize_any(Id)
    }
}

/// `#rrggbb` or `rrggbb`.
pub fn hex(text: &str) -> Option<SetId> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(SetId)
}

/// The colours sets had before they were chosen, in the order new sets took them, with the
/// names commands knew them by: files saved then name them by their place here.
pub const OLD: [(u32, &str); 5] =
    [(0xf0909e, "rose"), (0x4cc2bd, "teal"), (0xcf86eb, "orchid"), (0x7ea9ee, "sky"), (0xdcc15c, "gold")];

/// Amber, where execution is.
pub const AMBER: u32 = 0xffa31a;
/// Green, what changed in this step.
pub const GREEN: u32 = 0x3ddc84;
/// Red, an error.
pub const RED: u32 = 0xff5252;
/// The yellow of a warning, the amber's dimmer neighbour.
pub const WARNING: u32 = 0xd3bb6e;
const STATES: [u32; 4] = [AMBER, GREEN, RED, WARNING];

/// How far a set's colour stays from each state's, whatever else: a set must not look like a
/// state. Measured with lightness counting [`STATE_LIGHTNESS`] of what it does in ΔE, since a
/// darker or lighter amber still reads as amber.
pub const STATE_MARGIN: f64 = 0.11;
const STATE_LIGHTNESS: f64 = 0.5;
/// Degrees a set's hue stays from each state's: a paler or duller red still reads as red.
pub const STATE_HUE: f64 = 20.0;

/// Lightness and chroma (OKLCH) of the candidates, in rings of hues: first one that reads
/// well on the dark background, then a lighter and a darker one for when its hues are
/// crowded, then four between those and the first, duller and brighter.
const RINGS: [&[(f64, f64)]; 3] =
    [&[(0.76, 0.13)], &[(0.85, 0.10), (0.66, 0.13)], &[(0.71, 0.16), (0.81, 0.16), (0.71, 0.09), (0.81, 0.09)]];
/// Degrees between the hues of a ring.
const HUE_STEP: f64 = 2.0;

/// The distance a new set's colour starts out keeping from every other colour, about the
/// least two of the five old colours had.
pub const DISTINCT: f64 = 0.12;
/// What the bar is multiplied by each time no candidate clears it.
const LOWER: f64 = 0.85;
/// Below this, any colour no set has will do.
const FLOOR: f64 = 0.01;

/// A colour in OKLab.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lab {
    pub l: f64,
    pub a: f64,
    pub b: f64,
}

impl Lab {
    pub fn of(rgb: u32) -> Lab {
        let linear = |shift: u32| {
            let c = f64::from((rgb >> shift) & 0xff) / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let (r, g, b) = (linear(16), linear(8), linear(0));
        let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
        let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
        let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
        Lab {
            l: 0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
            a: 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
            b: 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
        }
    }

    /// From lightness, chroma and hue in degrees (OKLCH).
    pub fn lch(l: f64, c: f64, h: f64) -> Lab {
        let (sin, cos) = h.to_radians().sin_cos();
        Lab { l, a: c * cos, b: c * sin }
    }

    pub fn chroma(self) -> f64 {
        self.a.hypot(self.b)
    }

    /// Hue in degrees, 0 to 360.
    pub fn hue(self) -> f64 {
        self.b.atan2(self.a).to_degrees().rem_euclid(360.0)
    }

    /// ΔE: the distance in OKLab.
    pub fn distance(self, other: Lab) -> f64 {
        ((self.l - other.l).powi(2) + (self.a - other.a).powi(2) + (self.b - other.b).powi(2)).sqrt()
    }

    /// The distance from a state's colour, lightness counting less.
    fn state_distance(self, state: Lab) -> f64 {
        let dl = (self.l - state.l) * STATE_LIGHTNESS;
        (dl.powi(2) + (self.a - state.a).powi(2) + (self.b - state.b).powi(2)).sqrt()
    }

    /// `0xRRGGBB`, or `None` when it is outside sRGB.
    pub fn rgb(self) -> Option<u32> {
        let l = (self.l + 0.396_337_777_4 * self.a + 0.215_803_757_3 * self.b).powi(3);
        let m = (self.l - 0.105_561_345_8 * self.a - 0.063_854_172_8 * self.b).powi(3);
        let s = (self.l - 0.089_484_177_5 * self.a - 1.291_485_548 * self.b).powi(3);
        let linear = [
            4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
            -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
            -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s,
        ];
        let mut rgb = 0;
        for c in linear {
            if !(-1e-3..=1.0 + 1e-3).contains(&c) {
                return None;
            }
            let c = c.clamp(0.0, 1.0);
            let encoded = if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
            rgb = (rgb << 8) | (encoded * 255.0).round().clamp(0.0, 255.0) as u32;
        }
        Some(rgb)
    }
}

/// How far `rgb` is from the nearest state's colour, as [`STATE_MARGIN`] measures it.
pub fn state_distance(rgb: u32) -> f64 {
    let lab = Lab::of(rgb);
    STATES.iter().map(|state| lab.state_distance(Lab::of(*state))).fold(f64::INFINITY, f64::min)
}

/// Degrees from `rgb`'s hue to the nearest state's.
pub fn state_hue_distance(rgb: u32) -> f64 {
    let hue = Lab::of(rgb).hue();
    STATES.iter().map(|state| hue_apart(hue, Lab::of(*state).hue())).fold(f64::INFINITY, f64::min)
}

/// Whether a set may have colour `rgb`: far enough from every state's, in ΔE and in hue.
pub fn apart_from_states(rgb: u32) -> bool {
    state_distance(rgb) >= STATE_MARGIN && state_hue_distance(rgb) >= STATE_HUE
}

fn hue_apart(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    ring: usize,
    set: SetId,
    lab: Lab,
    /// From the nearest state's colour.
    state: f64,
}

/// The candidates: the hues of each ring, `step` degrees apart, those outside sRGB with their
/// chroma lowered until they are in it (by 40 % at most), and those too near a state's colour
/// left out.
fn sweep(step: f64) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = Vec::new();
    for (ring, shades) in RINGS.iter().enumerate() {
        for &(l, c) in *shades {
            let mut h = 0.0;
            while h < 360.0 {
                let mut chroma = c;
                let rgb = loop {
                    match Lab::lch(l, chroma, h).rgb() {
                        Some(rgb) => break Some(rgb),
                        None if chroma > c * 0.6 => chroma -= 0.005,
                        None => break None,
                    }
                };
                h += step;
                let Some(rgb) = rgb else { continue };
                let set = SetId(rgb);
                if apart_from_states(rgb) && !candidates.iter().any(|c| c.set == set) {
                    candidates.push(Candidate { ring, set, lab: Lab::of(rgb), state: state_distance(rgb) });
                }
            }
        }
    }
    candidates
}

fn candidates() -> &'static [Candidate] {
    static CANDIDATES: OnceLock<Vec<Candidate>> = OnceLock::new();
    CANDIDATES.get_or_init(|| sweep(HUE_STEP))
}

/// The colour for a new set, given those of the sets there are (`taken`): the candidate
/// farthest from them and from the states, see the module's documentation. The same sets
/// give the same colour.
pub fn next(taken: &[SetId]) -> SetId {
    pick(candidates(), taken).map_or_else(|| finer(taken), |(set, _)| set)
}

/// The colour [`next`] gives, and the bar it cleared: [`DISTINCT`], or lower when no colour
/// was that far from the others; 0 when any colour no set had would do.
pub fn next_with_bar(taken: &[SetId]) -> (SetId, f64) {
    pick(candidates(), taken).unwrap_or_else(|| (finer(taken), 0.0))
}

fn pick(candidates: &[Candidate], taken: &[SetId]) -> Option<(SetId, f64)> {
    let taken_labs: Vec<Lab> = taken.iter().map(|set| Lab::of(set.rgb())).collect();
    let nearest = |c: &Candidate| taken_labs.iter().map(|t| c.lab.distance(*t)).fold(c.state, f64::min);
    let free: Vec<(&Candidate, f64)> =
        candidates.iter().filter(|c| !taken.contains(&c.set)).map(|c| (c, nearest(c))).collect();
    let mut bar = DISTINCT;
    loop {
        for ring in 0..RINGS.len() {
            let mut best: Option<(&Candidate, f64)> = None;
            for &(c, d) in free.iter().filter(|(c, _)| c.ring <= ring) {
                if best.is_none_or(|(_, b)| d > b) {
                    best = Some((c, d));
                }
            }
            if let Some((c, d)) = best
                && d >= bar
            {
                return Some((c.set, bar));
            }
        }
        if bar == 0.0 {
            return None;
        }
        bar = if bar * LOWER < FLOOR { 0.0 } else { bar * LOWER };
    }
}

/// Once every candidate is taken: hues closer together, and in the end any colour no set has.
fn finer(taken: &[SetId]) -> SetId {
    let mut step = HUE_STEP / 2.0;
    while step >= HUE_STEP / 64.0 {
        if let Some((set, _)) = pick(&sweep(step), taken) {
            return set;
        }
        step /= 2.0;
    }
    let first = candidates().first().map_or(0x7f7f7f, |c| c.set.rgb());
    (0..=0x00ff_ffff).map(|n| SetId((first + n) & 0x00ff_ffff)).find(|set| !taken.contains(set)).unwrap_or(SetId(first))
}

/// Words for hues (OKLCH), each at the middle of the hues it names; the old colours' words
/// are at their hues.
const HUES: [(f64, &str); 21] = [
    (10.8, "rose"),
    (28.0, "coral"),
    (47.0, "peach"),
    (66.0, "orange"),
    (82.0, "ochre"),
    (94.5, "gold"),
    (110.0, "olive"),
    (126.0, "lime"),
    (146.0, "leaf"),
    (168.0, "mint"),
    (191.2, "teal"),
    (205.0, "cyan"),
    (220.0, "azure"),
    (240.0, "blue"),
    (259.6, "sky"),
    (274.0, "iris"),
    (289.0, "lavender"),
    (303.0, "violet"),
    (316.1, "orchid"),
    (331.0, "mauve"),
    (348.0, "pink"),
];

/// The word for a colour: its hue's, after `pale-` when it is light, `deep-` when it is dark,
/// else `dusty-` when it is dull and `bright-` when it is vivid; `grey` when it has hardly any
/// hue. The old colours keep their names.
pub fn colour_name(set: SetId) -> String {
    if let Some((_, word)) = OLD.iter().find(|(rgb, _)| *rgb == set.rgb()) {
        return (*word).to_owned();
    }
    let lab = Lab::of(set.rgb());
    if lab.chroma() < 0.03 {
        return "grey".to_owned();
    }
    let apart = |centre: f64| hue_apart(lab.hue(), centre);
    let word = HUES.iter().min_by(|a, b| apart(a.0).total_cmp(&apart(b.0))).map_or("grey", |(_, word)| word);
    let shade = match (lab.l, lab.chroma()) {
        (l, _) if l >= 0.83 => "pale-",
        (l, _) if l <= 0.68 => "deep-",
        (_, c) if c <= 0.10 => "dusty-",
        (_, c) if c >= 0.15 => "bright-",
        _ => "",
    };
    format!("{shade}{word}")
}

/// What `set` is called among `sets`, those of its engine in the order they were made: the
/// word for its colour, with a number after when sets made before it have the same word.
pub fn name(set: SetId, sets: &[SetId]) -> String {
    let word = colour_name(set);
    let Some(index) = sets.iter().position(|s| *s == set) else { return word };
    match sets[..index].iter().filter(|s| colour_name(**s) == word).count() {
        0 => word,
        before => format!("{word}{}", before + 1),
    }
}

/// The set among `sets` called `text` ([`name`]), or of the colour `#rrggbb`.
pub fn parse(text: &str, sets: &[SetId]) -> Option<SetId> {
    let text = text.trim().to_lowercase();
    if let Some(set) = hex(&text).filter(|set| sets.contains(set)) {
        return Some(set);
    }
    sets.iter().copied().find(|set| name(*set, sets) == text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first `n` colours, each made with those before it there.
    fn first(n: usize) -> (Vec<SetId>, Vec<f64>) {
        let mut sets = Vec::new();
        let mut bars = Vec::new();
        for _ in 0..n {
            let (set, bar) = next_with_bar(&sets);
            sets.push(set);
            bars.push(bar);
        }
        (sets, bars)
    }

    fn nearest(set: SetId, others: &[SetId]) -> f64 {
        let lab = Lab::of(set.rgb());
        others.iter().filter(|o| **o != set).map(|o| lab.distance(Lab::of(o.rgb()))).fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn oklab_matches_the_reference_and_goes_back_to_the_same_colour() {
        let white = Lab::of(0xffffff);
        assert!((white.l - 1.0).abs() < 1e-3 && white.chroma() < 1e-3, "{white:?}");
        // Björn Ottosson's figures for sRGB red.
        let red = Lab::of(0xff0000);
        assert!((red.l - 0.628).abs() < 1e-3 && (red.a - 0.2249).abs() < 1e-3 && (red.b - 0.1258).abs() < 1e-3);
        for rgb in [0xf0909e, 0x4cc2bd, 0x0c0c0e, 0xffa31a] {
            assert_eq!(Lab::of(rgb).rgb(), Some(rgb));
        }
        assert_eq!(Lab::lch(0.9, 0.4, 140.0).rgb(), None, "far outside sRGB");
    }

    #[test]
    fn the_first_colours_are_clearly_apart_and_a_bar_is_lowered_only_when_none_clears_it() {
        let (sets, bars) = first(12);
        let clear = bars.iter().take_while(|bar| **bar >= DISTINCT).count();
        assert!(clear >= 6, "{clear} sets before the bar is lowered: {sets:?}");
        for (i, set) in sets.iter().enumerate().take(clear) {
            assert!(nearest(*set, &sets[..=i]) >= DISTINCT, "#{i} {set}");
        }
        // Each colour is as far from those before it as its bar says, and the bars only go down.
        for (i, (set, bar)) in sets.iter().zip(&bars).enumerate() {
            assert!(nearest(*set, &sets[..i]) >= *bar, "#{i} {set} under {bar}");
        }
        assert!(bars.windows(2).all(|w| w[1] <= w[0]), "{bars:?}");
        assert!(bars[11] < DISTINCT, "12 sets do not all fit at the first bar: {bars:?}");
    }

    #[test]
    fn a_hundred_sets_get_a_hundred_colours() {
        let (sets, bars) = first(100);
        let mut unique = sets.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 100);
        assert!(bars[99] > 0.0 && bars[99] < bars[20], "{} then {}", bars[20], bars[99]);
        // More sets than candidates: hues closer together, still each its own colour.
        let all: Vec<SetId> = candidates().iter().map(|c| c.set).collect();
        let (more, bar) = next_with_bar(&all);
        assert!(!all.contains(&more) && apart_from_states(more.rgb()), "{more}");
        assert_eq!(bar, 0.0);
    }

    #[test]
    fn no_colour_looks_like_a_state() {
        let (sets, _) = first(60);
        for set in &sets {
            assert!(apart_from_states(set.rgb()), "{set} ({})", colour_name(*set));
            // Nor plainly: not even within ΔE 0.1 of amber, green or red.
            for state in [AMBER, GREEN, RED] {
                assert!(Lab::of(set.rgb()).distance(Lab::of(state)) > 0.1, "{set} near #{state:06x}");
            }
        }
        // A darker amber, a lighter green and red are each too near.
        for state_like in [0xc77d14, 0x8ef0b4, 0xe04848] {
            assert!(state_distance(state_like) < STATE_MARGIN, "#{state_like:06x}");
        }
        // A dull red is far enough in ΔE, but has red's hue.
        let dull_red = 0xb87971;
        assert!(state_distance(dull_red) >= STATE_MARGIN && state_hue_distance(dull_red) < STATE_HUE);
        assert!(!apart_from_states(dull_red));
    }

    #[test]
    fn colours_are_chosen_again_alike_and_a_freed_one_is_taken_again() {
        let (sets, _) = first(5);
        assert_eq!(first(5).0, sets);
        // The colour a set made from the same others gets does not depend on their order.
        let mut shuffled = sets[..4].to_vec();
        shuffled.reverse();
        assert_eq!(next(&shuffled), sets[4]);
        // Deleting the third set frees its colour: the next set takes it back.
        let without: Vec<SetId> = sets.iter().copied().filter(|s| *s != sets[2]).collect();
        assert_eq!(next(&without), sets[2]);
    }

    #[test]
    fn sets_are_saved_by_colour_and_old_files_by_place() {
        let set = SetId::from_rgb(0x4cc2bd);
        assert_eq!(serde_json::to_string(&set).expect("save"), r##""#4cc2bd""##);
        let read: Vec<SetId> = serde_json::from_str(r##"["#4cc2bd", "#ADA3FF"]"##).expect("load");
        assert_eq!(read, [set, SetId::from_rgb(0xada3ff)]);
        // Before colours were chosen, sets were saved as their colour's place among five.
        let old: Vec<SetId> = serde_json::from_str("[0, 1, 2, 3, 4]").expect("old");
        assert_eq!(old.iter().map(|s| s.rgb()).collect::<Vec<_>>(), OLD.map(|(rgb, _)| rgb));
        let odd: Vec<SetId> = serde_json::from_str(r##"[9, -1, "teal", "#12345"]"##).expect("odd");
        assert!(odd.iter().all(|s| !s.is_known()));
    }

    #[test]
    fn names_go_by_hue_and_shade_and_the_old_colours_keep_theirs() {
        for (rgb, word) in OLD {
            assert_eq!(colour_name(SetId::from_rgb(rgb)), word);
        }
        assert_eq!(colour_name(SetId::from_rgb(0xabd2fe)), "pale-sky");
        assert_eq!(colour_name(SetId::from_rgb(0x0ea99a)), "deep-teal");
        assert_eq!(colour_name(SetId::from_rgb(0x808080)), "grey");
        // Two of one name: the later is numbered, and the number goes with its place.
        let (a, b, c) = (SetId::from_rgb(0xf0909e), SetId::from_rgb(0xee8f9a), SetId::from_rgb(0x4cc2bd));
        let sets = [a, c, b];
        assert_eq!([name(a, &sets), name(b, &sets), name(c, &sets)], ["rose", "rose2", "teal"]);
        assert_eq!(parse("Rose2", &sets), Some(b));
        assert_eq!(parse("#4CC2BD", &sets), Some(c));
        assert_eq!(parse("#4cc2be", &sets), None, "no set has that colour");
        assert_eq!(parse("sky", &sets), None);
    }
}
