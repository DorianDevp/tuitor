//! Places in what programs print: the `file:line` of an error or a stack frame, as PHP, Node,
//! Rust, Go and C write them, and where that file is here.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// A file and a line, as printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub path: String,
    pub line: u32,
}

/// The first place in `text`:
///
/// - `path:line` and `path:line:column`: PHP's `in /app/src/Order.php:12`, Node's
///   `at total (/w/orders.js:29:9)`, with spaces in the path too between parentheses, and
///   `file:///w/orders.mjs:3:1`, Rust's
///   `panicked at src/main.rs:12:5:`, Go's `/w/main.go:58 +0x1d`, gdb's `at orders.c:41`;
/// - `path(line)`: PHP's stack traces, `#0 /app/src/Order.php(12): …`;
/// - `path on line N`: PHP's `Uncaught … in /app/orders.php on line 12`.
///
/// The file has to have an extension, which tells it from a time (`10:42:07`), a host and port,
/// or Node's own modules (`node:internal/main:33:47`).
pub fn find(text: &str) -> Option<Place> {
    let mut found: Vec<(usize, Place)> = Vec::new();
    found.extend(colons(text));
    found.extend(between_parentheses(text));
    found.extend(parentheses(text));
    found.extend(on_line(text));
    found.into_iter().min_by_key(|(at, _)| *at).map(|(_, place)| place)
}

/// Characters that end a path where it is printed among other text.
fn delimiter(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | '`' | ',' | ';' | '{' | '}')
}

/// The words of `text`, between delimiters, with where each starts.
fn words(text: &str) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let mut start = None;
    for (at, c) in text.char_indices() {
        match (delimiter(c), start) {
            (true, Some(from)) => {
                words.push((from, &text[from..at]));
                start = None;
            }
            (false, None) => start = Some(at),
            _ => {}
        }
    }
    if let Some(from) = start {
        words.push((from, &text[from..]));
    }
    words
}

/// `path:line` and `path:line:column`.
fn colons(text: &str) -> impl Iterator<Item = (usize, Place)> + '_ {
    words(text).into_iter().filter_map(|(at, word)| colon_place(word).map(|place| (at, place)))
}

/// `(path:line:column)`, the whole of what is between them, which may have spaces in it.
fn between_parentheses(text: &str) -> impl Iterator<Item = (usize, Place)> + '_ {
    text.match_indices('(').filter_map(|(open, _)| {
        let inside = &text[open + 1..];
        colon_place(&inside[..inside.find(')')?]).map(|place| (open + 1, place))
    })
}

fn colon_place(word: &str) -> Option<Place> {
    let word = word.strip_prefix("file://").unwrap_or(word).trim_end_matches([':', '.']);
    let (head, last) = number_after_colon(word)?;
    let (path, line) = match number_after_colon(head) {
        Some((path, line)) => (path, line),
        None => (head, last),
    };
    file(path).map(|path| Place { path, line })
}

/// `path(line)`.
fn parentheses(text: &str) -> impl Iterator<Item = (usize, Place)> + '_ {
    text.match_indices('(').filter_map(|(open, _)| {
        let digits = &text[open + 1..];
        let close = digits.find(')')?;
        let line = digits[..close].parse().ok()?;
        let start = word_start(text, open);
        file(&text[start..open]).map(|path| (start, Place { path, line }))
    })
}

/// `path on line N`.
fn on_line(text: &str) -> impl Iterator<Item = (usize, Place)> + '_ {
    text.match_indices(" on line ").filter_map(|(at, marker)| {
        let rest = &text[at + marker.len()..];
        let line = rest[..rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len())].parse().ok()?;
        let start = word_start(text, at);
        file(&text[start..at]).map(|path| (start, Place { path, line }))
    })
}

/// Where the word that ends at `end` starts: after the delimiter before it, whatever its width.
fn word_start(text: &str, end: usize) -> usize {
    let before = text[..end].char_indices().rev().find(|(_, c)| delimiter(*c));
    before.map_or(0, |(at, c)| at + c.len_utf8())
}

/// The place of a stop as EVENTS has it, `at /w/my shop/src/Order.php:12 · Order->total`,
/// read as the engine writes it, which keeps spaces in the path.
pub fn stop(detail: &str) -> Option<Place> {
    detail.match_indices(" · ").find_map(|(at, _)| {
        let (path, line) = number_after_colon(&detail[..at])?;
        Some(Place { path: path.to_owned(), line })
    })
}

/// `text` without the `:N` it ends with, and N.
fn number_after_colon(text: &str) -> Option<(&str, u32)> {
    let (head, number) = text.rsplit_once(':')?;
    let digits = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
    digits.then(|| number.parse().ok().map(|n| (head, n))).flatten()
}

/// `path` when it names a file: one with an extension that has a letter (not an address
/// such as `127.0.0.1`), and not a URL or a runtime's own module.
fn file(path: &str) -> Option<String> {
    let path = path.strip_prefix("file://").unwrap_or(path);
    if path.is_empty() || path.contains("://") || path.starts_with("node:") || path.contains(':') {
        return None;
    }
    let extension = Path::new(path).extension()?.to_str()?;
    let plausible = (1..=6).contains(&extension.len())
        && extension.chars().all(|c| c.is_ascii_alphanumeric())
        && extension.chars().any(|c| c.is_ascii_alphabetic());
    plausible.then(|| path.to_owned())
}

/// Where a printed path is here. One that is not here, from a program that runs elsewhere, goes
/// through `pathMappings` the other way (a path there → the path here); a relative one is taken
/// from each of `dirs` in turn, the first where it is a file.
pub fn here(path: &str, mappings: Option<&Map<String, Value>>, dirs: &[PathBuf]) -> Option<PathBuf> {
    let printed = Path::new(path);
    if printed.is_absolute() {
        if printed.is_file() {
            return Some(printed.to_path_buf());
        }
        // Not mapped, it stays as printed, for SOURCE to say how to map it.
        let mapped = mappings.zip(dirs.first()).and_then(|(mappings, project)| mapped(printed, mappings, project));
        return Some(mapped.unwrap_or_else(|| printed.to_path_buf()));
    }
    dirs.iter().map(|dir| dir.join(printed)).find(|candidate| candidate.is_file())
}

/// `path` through the mapping of the longest path there it starts with.
fn mapped(path: &Path, mappings: &Map<String, Value>, project: &Path) -> Option<PathBuf> {
    let (there, here) = mappings
        .iter()
        .filter_map(|(there, here)| Some((Path::new(there), here.as_str()?)))
        .filter(|(there, _)| path.starts_with(there))
        .max_by_key(|(there, _)| there.components().count())?;
    let rest = path.strip_prefix(there).ok()?;
    Some(project.join(here).join(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(text: &str) -> Option<(String, u32)> {
        find(text).map(|p| (p.path, p.line))
    }

    fn at(path: &str, line: u32) -> Option<(String, u32)> {
        Some((path.to_owned(), line))
    }

    #[test]
    fn php_errors_and_stack_traces() {
        let fatal = "PHP Fatal error:  Uncaught RuntimeException: no stock for SSD-1T in /app/orders.php:47";
        assert_eq!(place(fatal), at("/app/orders.php", 47));
        assert_eq!(
            place("#0 /app/src/Service/OrderService.php(141): App\\Stock->take()"),
            at("/app/src/Service/OrderService.php", 141)
        );
        assert_eq!(place("#1 {main}"), None);
        assert_eq!(
            place("Warning: Undefined variable $total in /var/www/app/public/index.php on line 12"),
            at("/var/www/app/public/index.php", 12)
        );
        assert_eq!(place("  thrown in /app/orders.php on line 47"), at("/app/orders.php", 47));
    }

    #[test]
    fn node_stack_frames() {
        assert_eq!(place("    at lineTotal (/w/examples/js/orders.js:29:9)"), at("/w/examples/js/orders.js", 29));
        assert_eq!(place("    at /w/orders.js:62:3"), at("/w/orders.js", 62));
        assert_eq!(place("    at file:///w/app/main.mjs:3:11"), at("/w/app/main.mjs", 3));
        assert_eq!(place("    at Object.<anonymous> (/w/orders.js:83:1)"), at("/w/orders.js", 83));
        assert_eq!(place("    at node:internal/main/run_main_module:33:47"), None);
        assert_eq!(place("Debug server listening at 127.0.0.1:40073"), None);
        assert_eq!(place("GET http://localhost:4200/main.js:10:5"), None);
    }

    #[test]
    fn rust_go_and_c() {
        assert_eq!(place("thread 'main' panicked at src/main.rs:62:9:"), at("src/main.rs", 62));
        assert_eq!(place("thread 'main' panicked at 'no stock', src/main.rs:62:9"), at("src/main.rs", 62));
        assert_eq!(place("             at ./src/main.rs:62:9"), at("./src/main.rs", 62));
        assert_eq!(place("\t/w/examples/go/main.go:58 +0x1d"), at("/w/examples/go/main.go", 58));
        assert_eq!(place("./main.go:12:5: undefined: total"), at("./main.go", 12));
        assert_eq!(place("Breakpoint 1, main () at orders.c:41"), at("orders.c", 41));
        assert_eq!(place("10:42:07.123 started at 10:42:07"), None);
    }

    #[test]
    fn spaces_of_any_width_and_paths_with_spaces() {
        assert_eq!(place("Total\u{a0}(12)"), None);
        assert_eq!(place("in\u{a0}/app/x.php on line 3"), at("/app/x.php", 3));
        assert_eq!(place("#0\u{3000}/app/x.php(7): f()"), at("/app/x.php", 7));
        assert_eq!(place("    at total (/w/My Shop/orders.js:29:9)"), at("/w/My Shop/orders.js", 29));
        let stop = |detail: &str| super::stop(detail).map(|p| (p.path, p.line));
        assert_eq!(stop("/w/My Shop/src/Order.php:12 · Order->total"), at("/w/My Shop/src/Order.php", 12));
        assert_eq!(stop("orders.c:48 · order_total"), at("orders.c", 48));
        assert_eq!(stop("?"), None);
    }

    #[test]
    fn the_first_place_on_a_line_wins() {
        assert_eq!(place("in /app/a.php:3 called from /app/b.php(9)"), at("/app/a.php", 3));
        assert_eq!(place("#2 /app/b.php(9): f() /app/a.php:3"), at("/app/b.php", 9));
    }

    #[test]
    fn paths_there_map_to_paths_here() {
        let dir = std::env::temp_dir().join(format!("tuitor-places-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("api/src")).expect("dirs");
        std::fs::write(dir.join("api/src/Order.php"), "<?php").expect("file");
        let mappings = serde_json::json!({
            "/var/www/app": dir.join("api").display().to_string(),
            "/var/www/app/vendor": "/elsewhere/vendor",
            "/srv": "./api",
        });
        let mappings = mappings.as_object();
        let dirs = [dir.clone()];
        assert_eq!(here("/var/www/app/src/Order.php", mappings, &dirs), Some(dir.join("api/src/Order.php")));
        assert_eq!(here("/srv/src/Order.php", mappings, &dirs), Some(dir.join("api/src/Order.php")));
        // The longest mapping wins, whether or not the file is here.
        assert_eq!(here("/var/www/app/vendor/x.php", mappings, &dirs), Some(PathBuf::from("/elsewhere/vendor/x.php")));
        // Not mapped: as printed, for SOURCE to say how to map it.
        assert_eq!(here("/opt/lib/x.php", mappings, &dirs), Some(PathBuf::from("/opt/lib/x.php")));
        assert_eq!(here("api/src/Order.php", None, &dirs), Some(dir.join("api/src/Order.php")));
        assert_eq!(here("src/Order.php", None, &dirs), None);
        std::fs::remove_dir_all(dir).ok();
    }
}
