//! Finding a breakpoint's line again after its file changed: edited in an editor, or
//! rewritten by a checkout, a pull or a rebase. A breakpoint keeps what its line read and the
//! nearest lines with text above and below it; in the file as it is now, its line is the one
//! that reads the same with the most of those around it, the nearest to where it was among
//! equals. A line whose own text changed is found by the lines around it. One found by
//! neither is lost: no line of the file is taken for it.

use std::cmp::Reverse;

use serde::{Deserialize, Serialize};

/// Lines with text kept on each side of a breakpoint's line.
const CONTEXT: usize = 2;

/// What a breakpoint's line read, and the nearest lines with text above and below it, nearest
/// first; each with its indentation and trailing space left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub above: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub below: Vec<String>,
}

/// The anchor of 1-based `line` in `lines`, or none when the file is shorter.
pub fn anchor(lines: &[&str], line: u32) -> Option<Anchor> {
    let at = usize::try_from(line).ok()?.checked_sub(1)?;
    let text = lines.get(at)?.trim().to_owned();
    Some(Anchor { text, above: around(lines, at, -1), below: around(lines, at, 1) })
}

/// Where `anchor` is in `lines` now, 1-based, starting from where it was (`was`); none when
/// neither its text nor the lines around it are there.
pub fn find(lines: &[&str], anchor: &Anchor, was: u32) -> Option<u32> {
    let was = usize::try_from(was).unwrap_or(usize::MAX).saturating_sub(1);
    let fits = |at: usize| fit(lines, at, anchor);
    let distance = |at: usize| at.abs_diff(was);

    // The lines that read the same: the one with the most context around it, then the nearest.
    let same = (0..lines.len()).filter(|&at| lines[at].trim() == anchor.text);
    let best = same.map(|at| (fits(at), at)).min_by_key(|&(context, at)| (Reverse(context), distance(at), at));
    if let Some((context, at)) = best {
        // A line that reads like many others (a brace, an empty line) counts only with
        // context, or where it was; there is no telling which of them it was otherwise.
        if context > 0 || !plain(&anchor.text) || at == was {
            return Some(to_line(at));
        }
    }

    // Its own text changed: the line between the same lines above and below.
    if anchor.above.is_empty() || anchor.below.is_empty() {
        return None;
    }
    (0..lines.len())
        .filter(|&at| around(lines, at, -1).first() == anchor.above.first())
        .filter(|&at| around(lines, at, 1).first() == anchor.below.first())
        .filter(|&at| fits(at) >= 2)
        .min_by_key(|&at| distance(at))
        .map(to_line)
}

/// How many of the anchor's context lines read the same around `at`.
fn fit(lines: &[&str], at: usize, anchor: &Anchor) -> usize {
    let same = |kept: &[String], now: Vec<String>| kept.iter().zip(&now).filter(|(a, b)| a == b).count();
    same(&anchor.above, around(lines, at, -1)) + same(&anchor.below, around(lines, at, 1))
}

/// The nearest lines with text before (`step` -1) or after (`step` 1) `at`, nearest first.
fn around(lines: &[&str], at: usize, step: isize) -> Vec<String> {
    let mut found = Vec::new();
    let mut i = at as isize + step;
    while found.len() < CONTEXT && i >= 0 && (i as usize) < lines.len() {
        let text = lines[i as usize].trim();
        if !text.is_empty() {
            found.push(text.to_owned());
        }
        i += step;
    }
    found
}

/// A line too common to tell apart by itself: empty, or punctuation and keywords that end
/// blocks.
fn plain(text: &str) -> bool {
    text.chars().all(|c| !c.is_alphanumeric()) || matches!(text, "else" | "end" | "fi" | "done" | "return;" | "break;")
}

fn to_line(at: usize) -> u32 {
    u32::try_from(at + 1).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEFORE: &str = "\
function total($order)
{
    $sum = 0;
    foreach ($order->lines as $line) {
        $sum += $line->qty * $line->price;
    }
    return $sum;
}
";

    fn lines(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    fn follow(before: &str, line: u32, after: &str) -> Option<u32> {
        let anchor = anchor(&lines(before), line).expect("anchor");
        find(&lines(after), &anchor, line)
    }

    #[test]
    fn an_anchor_keeps_the_line_and_two_lines_with_text_on_each_side() {
        let anchor = anchor(&lines(BEFORE), 5).expect("anchor");
        assert_eq!(anchor.text, "$sum += $line->qty * $line->price;");
        assert_eq!(anchor.above, ["foreach ($order->lines as $line) {", "$sum = 0;"]);
        assert_eq!(anchor.below, ["}", "return $sum;"]);
    }

    #[test]
    fn a_line_stays_where_nothing_moved_it() {
        assert_eq!(follow(BEFORE, 5, BEFORE), Some(5));
    }

    #[test]
    fn a_line_moves_down_with_lines_put_above_it() {
        let after = BEFORE.replace("    $sum = 0;\n", "    // Prices are in cents.\n    $sum = 0;\n    $tax = 0;\n");
        assert_eq!(follow(BEFORE, 5, &after), Some(7));
    }

    #[test]
    fn a_line_that_reads_like_others_goes_by_what_is_around_it() {
        // `return $sum;` comes twice; the one after the loop is the breakpoint's.
        let after = format!("function early($order)\n{{\n    $sum = 1;\n    return $sum;\n}}\n\n{BEFORE}");
        assert_eq!(follow(BEFORE, 7, &after), Some(13));
    }

    #[test]
    fn a_line_whose_text_changed_is_found_between_the_same_lines() {
        let after = BEFORE.replace("$line->qty * $line->price", "$line->total()");
        assert_eq!(follow(BEFORE, 5, &after), Some(5));
        let moved = after.replace("{\n    $sum = 0;", "{\n    // A comment.\n    $sum = 0;");
        assert_eq!(follow(BEFORE, 5, &moved), Some(6));
    }

    #[test]
    fn a_line_gone_with_the_lines_around_it_is_lost() {
        let after = "function total($order)\n{\n    return array_sum($order->totals());\n}\n";
        assert_eq!(follow(BEFORE, 5, after), None);
    }

    #[test]
    fn a_brace_counts_only_with_what_is_around_it() {
        // The loop's closing brace, with the loop gone: another brace is not it.
        let after = "function total($order)\n{\n    return 0;\n}\n";
        assert_eq!(follow(BEFORE, 6, after), None);
    }

    #[test]
    fn of_equal_lines_the_nearest_to_where_it_was_is_taken() {
        let text = "a();\nb();\na();\nb();\na();\n";
        let anchor = Anchor { text: "a();".into(), above: vec![], below: vec![] };
        assert_eq!(find(&lines(text), &anchor, 4), Some(3));
    }
}
