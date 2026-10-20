//! Searching the file in SOURCE: the matches of a text, in reading order, and the one to go
//! to from a place. Case is ignored unless the text has a capital letter in it.

use std::path::PathBuf;

/// A search of SOURCE, and the match the cursor went to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    /// The line and first character of the match the cursor went to.
    pub at: Option<(u32, usize)>,
}

/// The search being typed (`/search`, `ctrl-f`), and where SOURCE was before it, which `esc`
/// goes back to.
pub struct Prompt {
    pub input: String,
    pub browse: Option<PathBuf>,
    pub cursor: u32,
    /// The search before this one, which `enter` on nothing typed repeats, and `esc` keeps.
    pub previous: Option<Search>,
}

/// A match: its line (from 1), first character and length in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: u32,
    pub start: usize,
    pub len: usize,
}

/// Every match of `query` in `lines`, in reading order, overlapping ones left out.
pub fn find(lines: &[String], query: &str) -> Vec<Match> {
    let needle: Vec<char> = query.chars().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let exact = query.chars().any(char::is_uppercase);
    let same = |a: char, b: char| if exact { a == b } else { a == b || a.to_lowercase().eq(b.to_lowercase()) };
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut start = 0;
        while start + needle.len() <= chars.len() {
            if chars[start..start + needle.len()].iter().zip(&needle).all(|(a, b)| same(*a, *b)) {
                let line = u32::try_from(index + 1).unwrap_or(u32::MAX);
                found.push(Match { line, start, len: needle.len() });
                start += needle.len();
            } else {
                start += 1;
            }
        }
    }
    found
}

/// The match to go to from `from` (a line and a character in it): the first after it, or with
/// `forward` false the last before it, around the end of the file. With `here`, a match at
/// `from` itself counts, as when a search starts.
pub fn next(matches: &[Match], from: (u32, usize), forward: bool, here: bool) -> Option<usize> {
    let place = |m: &Match| (m.line, m.start);
    let found = if forward {
        matches.iter().position(|m| if here { place(m) >= from } else { place(m) > from })
    } else {
        matches.iter().rposition(|m| if here { place(m) <= from } else { place(m) < from })
    };
    found.or(if matches.is_empty() {
        None
    } else if forward {
        Some(0)
    } else {
        Some(matches.len() - 1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    #[test]
    fn case_matters_only_with_a_capital() {
        let file = lines("$total = 0;\nforeach ($items as $item) {\n    $Total += total($item);\n}");
        let at = |query: &str| find(&file, query).iter().map(|m| (m.line, m.start)).collect::<Vec<_>>();
        assert_eq!(at("total"), [(1, 1), (3, 5), (3, 14)]);
        assert_eq!(at("Total"), [(3, 5)]);
        assert_eq!(at("$item"), [(2, 9), (2, 19), (3, 20)]);
        assert_eq!(at(""), []);
        assert_eq!(find(&lines("aaaa"), "aa").len(), 2, "matches do not overlap");
        // Characters, not bytes, so that columns come out right.
        assert_eq!(find(&lines("// prix € — total"), "total"), [Match { line: 1, start: 12, len: 5 }]);
    }

    #[test]
    fn next_goes_around_the_end_of_the_file() {
        let file = lines("a x\nb\nx c x\nx");
        let matches = find(&file, "x");
        assert_eq!(matches.len(), 4);
        // Starting on a line, a match on it counts.
        assert_eq!(next(&matches, (3, 0), true, true), Some(1));
        assert_eq!(next(&matches, (3, 0), true, false), Some(2));
        assert_eq!(next(&matches, (3, 0), false, false), Some(0));
        assert_eq!(next(&matches, (3, 4), true, false), Some(3));
        assert_eq!(next(&matches, (4, 0), true, false), Some(0), "around the end");
        assert_eq!(next(&matches, (1, 2), false, false), Some(3), "around the start");
        assert_eq!(next(&[], (1, 0), true, true), None);
    }
}
