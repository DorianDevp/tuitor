//! Cell-precise drawing on a ratatui buffer: text that clips with an ellipsis,
//! background bands and the panel frame used everywhere.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::theme::{self, AMBER, DIM, FAINT, LINE, LINE_FOCUS, TITLE};

/// A run of text with one style.
#[derive(Debug, Clone)]
pub struct Seg<'a> {
    pub text: std::borrow::Cow<'a, str>,
    pub style: Style,
}

pub fn seg<'a>(text: impl Into<std::borrow::Cow<'a, str>>, style: Style) -> Seg<'a> {
    Seg { text: text.into(), style }
}

pub fn width(segs: &[Seg<'_>]) -> u16 {
    let total: usize = segs.iter().map(|s| s.text.width()).sum();
    u16::try_from(total).unwrap_or(u16::MAX)
}

/// Shortens `text` to `max` columns, ending with `…` when cut.
pub fn truncate(text: &str, max: usize) -> std::borrow::Cow<'_, str> {
    if text.width() <= max {
        return text.into();
    }
    if max == 0 {
        return "".into();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out.into()
}

/// Shortens from the left (`…/Notification/OrderNotifier.php`), for paths.
pub fn truncate_left(text: &str, max: usize) -> std::borrow::Cow<'_, str> {
    if text.width() <= max {
        return text.into();
    }
    if max == 0 {
        return "".into();
    }
    let mut kept: Vec<char> = Vec::new();
    let mut used = 0;
    for c in text.chars().rev() {
        let w = c.width().unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        kept.push(c);
        used += w;
    }
    let mut out = String::from('…');
    out.extend(kept.into_iter().rev());
    out.into()
}

/// Shortens `text` to `max` columns by cutting out its middle (`vent…-api`), so names that
/// share a beginning or an end stay apart. Below six columns only the beginning is kept.
pub fn truncate_middle(text: &str, max: usize) -> std::borrow::Cow<'_, str> {
    if text.width() <= max || max < 6 {
        return truncate(text, max);
    }
    let tail_width = (max - 1) / 2;
    let head = truncate(text, max - tail_width);
    let head = head.trim_end_matches('…');
    let tail = truncate_left(text, tail_width + 1);
    format!("{head}{tail}").into()
}

/// `text` in lines of at most `max` columns, broken between words; a word longer than
/// that gets a line of its own.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.width() + 1 + word.width() <= max => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}

pub struct Painter<'b> {
    pub buf: &'b mut Buffer,
}

impl Painter<'_> {
    fn area(&self) -> Rect {
        self.buf.area
    }

    /// Writes text at (x, y), clipped at `max_x` (exclusive) with an ellipsis. Returns the end x.
    pub fn text(&mut self, x: u16, y: u16, text: &str, style: Style, max_x: u16) -> u16 {
        let area = self.area();
        if y < area.top() || y >= area.bottom() || x >= max_x.min(area.right()) {
            return x;
        }
        let room = usize::from(max_x.min(area.right()) - x);
        let text = truncate(text, room);
        let (end, _) = self.buf.set_stringn(x, y, &text, room, style);
        end
    }

    pub fn segs(&mut self, mut x: u16, y: u16, segs: &[Seg<'_>], max_x: u16) -> u16 {
        for s in segs {
            if x >= max_x {
                break;
            }
            x = self.text(x, y, &s.text, s.style, max_x);
        }
        x
    }

    /// Right-aligns segments so they end at `right` (exclusive).
    pub fn segs_right(&mut self, right: u16, y: u16, segs: &[Seg<'_>], min_x: u16) -> u16 {
        let w = width(segs);
        let x = right.saturating_sub(w).max(min_x);
        self.segs(x, y, segs, right);
        x
    }

    pub fn fill(&mut self, area: Rect, bg: Color) {
        let area = area.intersection(self.area());
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = self.buf.cell_mut((x, y)) {
                    cell.reset();
                    cell.set_bg(bg);
                }
            }
        }
    }

    /// Sets the background of a horizontal band without touching text.
    pub fn band(&mut self, x: u16, y: u16, w: u16, bg: Color) {
        let area = Rect::new(x, y, w, 1).intersection(self.area());
        for x in area.left()..area.right() {
            if let Some(cell) = self.buf.cell_mut((x, area.y)) {
                cell.set_bg(bg);
            }
        }
    }

    /// Dims everything already drawn (behind an overlay).
    pub fn dim(&mut self, area: Rect) {
        let area = area.intersection(self.area());
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = self.buf.cell_mut((x, y)) {
                    let fg = scale(cell.fg, 0.32);
                    let bg = scale(cell.bg, 0.6);
                    cell.set_style(Style::reset().fg(fg).bg(bg));
                }
            }
        }
    }

    /// The standard panel: thin frame, title (amber when focused), subtitle,
    /// and right-aligned info plus the panel's own key hints.
    pub fn panel(&mut self, area: Rect, panel: &Panel<'_>) {
        if area.width < 4 || area.height < 2 {
            return;
        }
        let border = theme::fg(if panel.focus { LINE_FOCUS } else { LINE });
        let (l, r, t, b) = (area.left(), area.right() - 1, area.top(), area.bottom() - 1);
        let horizontal = "─".repeat(usize::from(area.width - 2));
        self.text(l, t, &format!("┌{horizontal}┐"), border, area.right());
        self.text(l, b, &format!("└{horizontal}┘"), border, area.right());
        for y in t + 1..b {
            self.text(l, y, "│", border, area.right());
            self.text(r, y, "│", border, area.right());
        }

        let mut right: Vec<Seg<'_>> = panel.right.clone();
        if !panel.keys.is_empty() {
            if !right.is_empty() {
                right.push(seg("  ", theme::fg(DIM)));
            }
            for (i, (key, label)) in panel.keys.iter().enumerate() {
                right.push(seg(*key, theme::bold(DIM)));
                let sep = if i + 1 < panel.keys.len() { "  " } else { "" };
                right.push(seg(format!(" {label}{sep}"), theme::fg(FAINT)));
            }
        }
        let right_w = if right.is_empty() { 0 } else { width(&right) + 2 };
        let title_end = r.saturating_sub(right_w).max(l + 2);

        let title_style = theme::bold(if panel.focus { AMBER } else { TITLE });
        let mut x = self.text(l + 2, t, &format!(" {} ", panel.title), title_style, title_end);
        x = self.segs(x, t, &panel.sub, title_end);
        if !panel.sub.is_empty() && x < title_end {
            self.text(x, t, " ", theme::fg(DIM), title_end);
        }
        if !right.is_empty() {
            right.insert(0, seg(" ", theme::fg(DIM)));
            right.push(seg(" ", theme::fg(DIM)));
            self.segs_right(r, t, &right, title_end);
        }
    }
}

#[derive(Default)]
pub struct Panel<'a> {
    pub title: &'a str,
    pub sub: Vec<Seg<'a>>,
    pub right: Vec<Seg<'a>>,
    pub keys: Vec<(&'a str, &'a str)>,
    pub focus: bool,
}

/// Inner area of a panel.
pub fn inner(area: Rect) -> Rect {
    Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), area.height.saturating_sub(2))
}

fn scale(color: Color, factor: f32) -> Color {
    match color {
        Color::Rgb(r, g, b) => {
            let f = |v: u8| (f32::from(v) * factor) as u8;
            Color::Rgb(f(r), f(g), f(b))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("OrderService.php", 8), "OrderSe…");
        assert_eq!(truncate("short", 8), "short");
        assert_eq!(truncate_left("src/Service/OrderService.php", 12), "…Service.php");
    }

    #[test]
    fn middle_truncation_keeps_both_ends() {
        assert_eq!(truncate_middle("ventia-wsparcie-api", 9), "vent…-api");
        assert_eq!(truncate_middle("ventia-wsparcie-frontend", 9), "vent…tend");
        assert_eq!(truncate_middle("orders", 9), "orders");
        assert_eq!(truncate_middle("ledger-reconciliation", 5), "ledg…");
    }

    #[test]
    fn wraps_between_words() {
        assert_eq!(wrap("js-debug starts Google Chrome", 16), ["js-debug starts", "Google Chrome"]);
        assert_eq!(wrap("  runtimeExecutable  ", 8), ["runtimeExecutable"]);
        assert!(wrap("", 8).is_empty());
    }

    #[test]
    fn truncation_respects_wide_chars() {
        assert_eq!(truncate("zażółć", 4), "zaż…");
        assert_eq!(truncate("日本語", 5), "日本…");
    }
}
