//! What is read from source text itself, where servers give no answer: where a function's
//! body starts, which names a line assigns to and which it calls. A light scan with strings
//! and comments left out; what the names stand for is for the servers to say.

use std::path::Path;

use anyhow::{Context, Result};

use crate::config::Language;

/// A file's text and its lines.
pub struct Text {
    pub source: String,
    pub lines: Vec<String>,
}

impl Text {
    pub fn new(source: String) -> Text {
        let lines = source.lines().map(str::to_owned).collect();
        Text { source, lines }
    }

    pub fn read(path: &Path) -> Result<Text> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Text::new(String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// Line `line` (0-based), or nothing past the end.
    pub fn line(&self, line: u32) -> &str {
        usize::try_from(line).ok().and_then(|line| self.lines.get(line)).map_or("", String::as_str)
    }

    pub fn last_line(&self) -> u32 {
        u32::try_from(self.lines.len().saturating_sub(1)).unwrap_or(u32::MAX)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// A name, with PHP's `$`, and the byte it starts at.
    Name(String, usize),
    Punct(char, usize),
}

impl Tok {
    fn at(&self) -> usize {
        match self {
            Tok::Name(_, at) | Tok::Punct(_, at) => *at,
        }
    }

    fn is(&self, c: char) -> bool {
        matches!(self, Tok::Punct(p, _) if *p == c)
    }
}

/// A comment or a string still open at the end of a line.
#[derive(Clone, Copy)]
enum Open {
    Comment,
    Quoted(char),
}

/// Splits lines into names and punctuation, without strings, comments and numbers. A block
/// comment or a string open at the end of a line goes on into the next.
pub struct Lexer {
    language: Language,
    open: Option<Open>,
}

impl Lexer {
    pub fn new(language: Language) -> Lexer {
        Lexer { language, open: None }
    }

    pub fn line(&mut self, line: &str) -> Vec<Tok> {
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let mut toks = Vec::new();
        let mut i = 0;
        while let Some(&(at, c)) = chars.get(i) {
            let next = chars.get(i + 1).map(|&(_, c)| c);
            match self.open {
                Some(Open::Comment) => {
                    if c == '*' && next == Some('/') {
                        self.open = None;
                        i += 1;
                    }
                    i += 1;
                    continue;
                }
                Some(Open::Quoted(quote)) => {
                    if c == quote {
                        self.open = None;
                    }
                    i += if c == '\\' { 2 } else { 1 };
                    continue;
                }
                None => {}
            }
            match c {
                '/' if next == Some('/') => break,
                '#' if self.language == Language::Php && next != Some('[') => break,
                '/' if next == Some('*') => {
                    self.open = Some(Open::Comment);
                    i += 2;
                }
                // A Rust lifetime ('a) opens no character literal.
                '\'' if self.language == Language::Rust && !char_literal(&chars, i) => i += 1,
                '"' | '\'' | '`' => {
                    self.open = Some(Open::Quoted(c));
                    i += 1;
                }
                c if c.is_alphabetic() || c == '_' || c == '$' => {
                    let end = chars[i..].iter().position(|&(_, c)| !name_char(c)).map_or(chars.len(), |n| i + n);
                    let name_end = chars.get(end).map_or(line.len(), |&(at, _)| at);
                    toks.push(Tok::Name(line[at..name_end].to_owned(), at));
                    i = end;
                }
                c if c.is_ascii_digit() => {
                    i += chars[i..].iter().position(|&(_, c)| !name_char(c) && c != '.').unwrap_or(chars.len() - i);
                }
                c if c.is_whitespace() => i += 1,
                c => {
                    toks.push(Tok::Punct(c, at));
                    i += 1;
                }
            }
        }
        toks
    }
}

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Whether the `'` at `i` opens a character literal: `'x'`, `'\n'`, rather than a lifetime.
fn char_literal(chars: &[(usize, char)], i: usize) -> bool {
    let at = |k: usize| chars.get(i + k).map(|&(_, c)| c);
    at(1) == Some('\\') || at(2) == Some('\'')
}

/// The first line of a function's body that has code (0-based): the line after the `{` that
/// opens the body, or the `{` line when code follows the brace there. `name` is the line and
/// byte of the function's name; the body must open by line `last`. The name's line when the
/// body is empty, or there is none (an abstract method, an interface).
pub fn entry(text: &Text, language: Language, name: (u32, usize), last: u32) -> u32 {
    let mut lexer = Lexer::new(language);
    // A `{` inside parentheses or brackets is a default value or a type, not the body.
    let mut depth = 0i32;
    let mut opened = false;
    for number in name.0..=last.max(name.0) {
        let mut toks = lexer.line(text.line(number));
        if number == name.0 {
            toks.retain(|tok| tok.at() >= name.1);
        }
        if opened {
            match toks.first() {
                None => continue,
                Some(first) if first.is('}') => return name.0,
                Some(_) => return number,
            }
        }
        for (k, tok) in toks.iter().enumerate() {
            match tok {
                Tok::Punct('(' | '[', _) => depth += 1,
                Tok::Punct(')' | ']', _) => depth -= 1,
                Tok::Punct(';', _) if depth == 0 => return name.0,
                Tok::Punct('{', _) if depth == 0 => match toks.get(k + 1) {
                    None => {
                        opened = true;
                        break;
                    }
                    Some(after) if after.is('}') => return name.0,
                    Some(_) => return number,
                },
                _ => {}
            }
        }
    }
    name.0
}

const DECLARE: &[&str] = &["let", "const", "var"];

/// The names a line writes, with the byte each starts at: what `let`, `const` and `var`
/// declare, what an assignment (`=`, `+=`, `.=`, `??=`, `:=`…) assigns to and what `++` and
/// `--` change. `a.b = …` writes `b`, `a[i] = …` and `$a[] = …` write `a`, `[$a, $b] = …`
/// and `a, b := …` both.
pub fn writes(toks: &[Tok]) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    // PHP's `declare(strict_types=1)` sets no variable.
    if toks.first().is_some_and(|tok| matches!(tok, Tok::Name(word, _) if word == "declare")) {
        return out;
    }
    for (i, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Name(word, _) if DECLARE.contains(&word.as_str()) => declared(&toks[i + 1..], &mut out),
            Tok::Punct('=', _) => {
                if let Some(start) = assignment(toks, i) {
                    assigned(&toks[..start], &mut out);
                }
            }
            Tok::Punct(c @ ('+' | '-'), at) if toks.get(i + 1).is_some_and(|t| *t == Tok::Punct(*c, at + 1)) => {
                let name = match (i.checked_sub(1).map(|k| &toks[k]), toks.get(i + 2)) {
                    (Some(Tok::Name(name, at)), _) | (_, Some(Tok::Name(name, at))) => Some((name.clone(), *at)),
                    _ => None,
                };
                out.extend(name);
            }
            _ => {}
        }
    }
    out.retain(|(name, _)| name != "_" && name != "mut");
    out.sort_by_key(|(_, at)| *at);
    out.dedup_by_key(|(_, at)| *at);
    out
}

/// Where the operator starts when the `=` at `i` assigns: `=`, `+=`, `??=`, `:=` and the
/// like, and not `==`, `!=`, `<=`, `=>`.
fn assignment(toks: &[Tok], i: usize) -> Option<usize> {
    let at = toks[i].at();
    if toks.get(i + 1).is_some_and(|next| next.at() == at + 1 && (next.is('=') || next.is('>'))) {
        return None;
    }
    let mut start = i;
    let mut run = String::new();
    while let Some(Tok::Punct(c, before)) = start.checked_sub(1).map(|k| &toks[k]) {
        if *before + c.len_utf8() != toks[start].at() {
            break;
        }
        run.insert(0, *c);
        start -= 1;
    }
    const OPERATORS: &[&str] =
        &["", "+", "-", "*", "/", "%", ".", "|", "&", "^", "??", "**", "<<", ">>", "||", "&&", ":"];
    OPERATORS.contains(&run.as_str()).then_some(start)
}

/// The names the left side of an assignment writes.
fn assigned(left: &[Tok], out: &mut Vec<(String, usize)>) {
    match left.last() {
        Some(Tok::Punct(']' | ')', _)) => {
            let Some(open) = opening(left, left.len() - 1) else { return };
            match left[..open].last() {
                // An element of a collection: the collection changes.
                Some(Tok::Name(name, at)) if name != "list" => out.push((name.clone(), *at)),
                // `[$a, $b] = …`, `list($a, $b) = …`: each of them.
                _ => out.extend(names(&left[open + 1..left.len() - 1])),
            }
        }
        Some(Tok::Name(name, at)) => {
            // `x: T =` is typed, and `x T =` in Go; `let x`, `var x T` were declarations.
            match left.len().checked_sub(2).map(|k| &left[k]) {
                Some(Tok::Punct(':', _) | Tok::Name(..)) => return,
                _ => out.push((name.clone(), *at)),
            }
            // Go's `a, b := …`.
            let mut k = left.len() - 1;
            while k >= 2 && left[k - 1].is(',') {
                let Tok::Name(name, at) = &left[k - 2] else { break };
                out.push((name.clone(), *at));
                k -= 2;
            }
        }
        _ => {}
    }
}

/// What `let`, `const`, `var` (and `let mut`) declare: a name, or the names of a pattern
/// (`let (a, b)`, `const { city, country }`, `let Some(x)`).
fn declared(rest: &[Tok], out: &mut Vec<(String, usize)>) {
    let rest = match rest.first() {
        Some(Tok::Name(word, _)) if word == "mut" => &rest[1..],
        _ => rest,
    };
    let pattern = |at: usize| {
        let close = closing(rest, at)?;
        Some(names(&rest[at + 1..close]))
    };
    match rest {
        [Tok::Name(..), Tok::Punct('(' | '{', _), ..] => out.extend(pattern(1).unwrap_or_default()),
        [Tok::Name(name, at), ..] => out.push((name.clone(), *at)),
        [Tok::Punct('(' | '{' | '[', _), ..] => out.extend(pattern(0).unwrap_or_default()),
        _ => {}
    }
}

fn names(toks: &[Tok]) -> Vec<(String, usize)> {
    toks.iter()
        .filter_map(|tok| match tok {
            Tok::Name(name, at) if !matches!(name.as_str(), "mut" | "ref") => Some((name.clone(), *at)),
            _ => None,
        })
        .collect()
}

/// The index of the bracket that closes the one at `open`.
fn closing(toks: &[Tok], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (k, tok) in toks.iter().enumerate().skip(open) {
        match tok {
            Tok::Punct('(' | '[' | '{', _) => depth += 1,
            Tok::Punct(')' | ']' | '}', _) => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// The index of the bracket that opens the one at `close`.
fn opening(toks: &[Tok], close: usize) -> Option<usize> {
    let mut depth = 0;
    for k in (0..=close).rev() {
        match toks[k] {
            Tok::Punct(')' | ']' | '}', _) => depth += 1,
            Tok::Punct('(' | '[' | '{', _) => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether a line leaves brackets open, besides the block of a `for`, an `if` and the like:
/// its statement goes on over the next lines.
pub fn continues(toks: &[Tok]) -> bool {
    const BLOCKS: &[&str] = &["for", "if", "else", "switch", "select", "func", "while", "foreach", "match", "loop"];
    let depth: i32 = toks
        .iter()
        .map(|tok| match tok {
            Tok::Punct('(' | '[' | '{', _) => 1,
            Tok::Punct(')' | ']' | '}', _) => -1,
            _ => 0,
        })
        .sum();
    let block = matches!(toks.first(), Some(Tok::Name(word, _)) if BLOCKS.contains(&word.as_str()));
    depth > i32::from(block)
}

/// Words followed by `(` that call nothing.
const NOT_CALLS: &[&str] = &[
    "if",
    "for",
    "foreach",
    "while",
    "switch",
    "catch",
    "match",
    "return",
    "function",
    "fn",
    "func",
    "isset",
    "empty",
    "unset",
    "list",
    "array",
    "echo",
    "print",
    "require",
    "require_once",
    "include",
    "include_once",
    "typeof",
    "sizeof",
    "await",
    "yield",
    "throw",
    "use",
    "elseif",
    "else",
    "do",
    "case",
    "in",
    "as",
    "and",
    "or",
];

/// The names a line calls, with the byte each starts at: a name before `(` that is no keyword,
/// not a function being declared and no class being made with `new`.
pub fn calls(toks: &[Tok]) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for (i, tok) in toks.iter().enumerate() {
        let Tok::Name(name, at) = tok else { continue };
        if !toks.get(i + 1).is_some_and(|next| next.is('(')) || NOT_CALLS.contains(&name.as_str()) {
            continue;
        }
        let before = i.checked_sub(1).map(|k| &toks[k]);
        if matches!(before, Some(Tok::Name(word, _)) if matches!(word.as_str(), "new" | "function" | "fn" | "func")) {
            continue;
        }
        out.push((name.clone(), *at));
    }
    out
}

/// Whether what follows byte `end` of `line` calls it: `(`, after type arguments
/// (`::<T>(`, `<T>(`) if any.
pub fn called(line: &str, end: usize) -> bool {
    let rest = line.get(end..).unwrap_or_default().trim_start();
    let rest = rest.strip_prefix("::").unwrap_or(rest);
    let rest = match rest.strip_prefix('<') {
        Some(generic) => {
            let mut depth = 1;
            let close = generic.char_indices().find(|&(_, c)| {
                depth += match c {
                    '<' => 1,
                    '>' => -1,
                    _ => 0,
                };
                depth == 0
            });
            close.map_or("", |(at, _)| &generic[at + 1..])
        }
        None => rest,
    };
    rest.trim_start().starts_with('(')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[&str]) -> Text {
        Text::new(lines.join("\n"))
    }

    fn written(language: Language, line: &str) -> Vec<String> {
        writes(&Lexer::new(language).line(line)).into_iter().map(|(name, _)| name).collect()
    }

    #[test]
    fn strings_and_comments_hide_what_they_hold() {
        let mut lexer = Lexer::new(Language::TypeScript);
        let names = |toks: Vec<Tok>| -> Vec<String> {
            toks.into_iter()
                .filter_map(|t| match t {
                    Tok::Name(n, _) => Some(n),
                    Tok::Punct(..) => None,
                })
                .collect()
        };
        assert_eq!(names(lexer.line("const a = `x = ${b}")), ["const", "a"]);
        assert_eq!(names(lexer.line("  (click)=\"render()\"` + c; /* d")), ["c"]);
        assert_eq!(names(lexer.line("e */ f // g")), ["f"]);
        let mut php = Lexer::new(Language::Php);
        assert_eq!(names(php.line("$x = 1; # $y = 2")), ["$x"]);
        assert_eq!(names(php.line("#[Route('/a')] public function a()")), ["Route", "public", "function", "a"]);
        let mut rust = Lexer::new(Language::Rust);
        assert_eq!(names(rust.line("fn f<'a>(x: &'a str) -> char { 'y' }")), ["fn", "f", "a", "x", "a", "str", "char"]);
    }

    #[test]
    fn the_body_starts_after_its_brace_wherever_that_is() {
        let php = text(&[
            "    public static function lineTotal(Line $line): int",
            "    {",
            "",
            "        $amount = $line->qty * $line->price;",
            "    }",
        ]);
        assert_eq!(entry(&php, Language::Php, (0, 27), 4), 3);
        let params = text(&[
            "    public function __construct(",
            "        private readonly Repo $orders = new Repo([]),",
            "    ) {",
            "    }",
        ]);
        // An empty body: the declaration.
        assert_eq!(entry(&params, Language::Php, (0, 20), 3), 0);
        let go = text(&["func (s *Shop) total(o Order) (int, error) { return 0, nil }"]);
        assert_eq!(entry(&go, Language::Go, (0, 15), 0), 0);
        let ts = text(&["  protected render(): void {", "    // rows", "    const rows: Row[] = [];", "  }"]);
        assert_eq!(entry(&ts, Language::TypeScript, (0, 12), 3), 2);
        let abstract_method = text(&["    abstract public function run(): void;", "    public function x() {}"]);
        assert_eq!(entry(&abstract_method, Language::Php, (0, 29), 1), 0);
    }

    #[test]
    fn declarations_and_assignments_write() {
        assert_eq!(written(Language::Php, "$total += Money::lineTotal($line);"), ["$total"]);
        assert_eq!(written(Language::Php, "$this->total = $total;"), ["total"]);
        assert_eq!(written(Language::Php, "$rows[] = sprintf('%s', $x);"), ["$rows"]);
        assert_eq!(written(Language::Php, "[$name, $action] = explode('.', $route);"), ["$name", "$action"]);
        assert_eq!(written(Language::Php, "$s .= 'x'; $n ??= 0; $i++;"), ["$s", "$n", "$i"]);
        assert_eq!(written(Language::Php, "declare(strict_types=1);"), Vec::<String>::new());
        assert_eq!(written(Language::TypeScript, "let total: number = 0;"), ["total"]);
        assert_eq!(written(Language::TypeScript, "const { city, country } = customer.address;"), ["city", "country"]);
        assert_eq!(written(Language::TypeScript, "this.rows.set(rows);"), Vec::<String>::new());
        assert_eq!(written(Language::Rust, "let mut total = 0;"), ["total"]);
        assert_eq!(
            written(Language::Rust, "let Customer { name, address, .. } = &order.customer;"),
            ["name", "address"]
        );
        assert_eq!(written(Language::Go, "for _, line := range order.Lines {"), ["line"]);
        assert_eq!(written(Language::Go, "total, err := sum(order)"), ["total", "err"]);
        assert_eq!(written(Language::Go, "var percent int = 5"), ["percent"]);
    }

    #[test]
    fn comparisons_and_arrows_write_nothing() {
        for line in ["if ($a == $b || $c != 1 || $d <= 2 || $e >= 3) {", "return a === b;", "xs.map((x) => x * 2)"] {
            assert_eq!(written(Language::TypeScript, line), Vec::<String>::new(), "{line}");
        }
        assert_eq!(written(Language::Php, "$f = fn ($x) => $x <=> 1;"), ["$f"]);
    }

    #[test]
    fn calls_are_names_before_a_parenthesis() {
        let toks = Lexer::new(Language::Php).line("$o = new Order(1); if (count($o)) { $this->repo->save($o); }");
        let called: Vec<String> = calls(&toks).into_iter().map(|(name, _)| name).collect();
        assert_eq!(called, ["count", "save"]);
        assert!(called_at("    $total += Money::lineTotal($line);", "lineTotal"));
        assert!(called_at("let x = parse::<u64>(s);", "parse"));
        assert!(called_at("const f = make<Order>(1);", "make"));
        assert!(!called_at("$loyalty, 'onOrderTotalled'", "onOrderTotalled"));
        assert!(!called_at("xs.map(lineTotal)", "lineTotal"));
    }

    #[test]
    fn a_line_with_brackets_left_open_continues() {
        let toks = |line: &str| Lexer::new(Language::Go).line(line);
        assert!(continues(&toks("orders := []Order{")));
        assert!(!continues(&toks("total := orderTotal(order)")));
        assert!(!continues(&toks("for _, line := range order.Lines {")));
        assert!(continues(&toks("if err := check(Order{")));
    }

    fn called_at(line: &str, name: &str) -> bool {
        let at = line.find(name).expect("name");
        called(line, at + name.len())
    }
}
