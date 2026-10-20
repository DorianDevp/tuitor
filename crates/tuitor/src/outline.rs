//! The functions and methods of a source file, with their lines, read the way the
//! syntax colouring reads code: line by line, with strings and comments left out and
//! braces counted for where a class or a function ends. No parser and no language
//! server, so it can be wrong on unusual code; it only ever points at lines.

use crate::ui::syntax::Lang;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// The class, impl, trait or receiver type it belongs to.
    pub container: Option<String>,
    /// 1-based line of its declaration.
    pub line: u32,
    /// The line its body closes on; its declaration line when that was not found.
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Punct(char),
    Str,
}

impl Tok {
    fn is(&self, word: &str) -> bool {
        matches!(self, Tok::Ident(w) if w == word)
    }

    fn ident(&self) -> Option<&str> {
        match self {
            Tok::Ident(w) => Some(w),
            _ => None,
        }
    }
}

/// Splits lines into tokens without strings and comments; block comments carry over lines.
struct Lexer {
    lang: Lang,
    in_block: bool,
}

impl Lexer {
    fn line(&mut self, line: &str) -> Vec<Tok> {
        let chars: Vec<char> = line.chars().collect();
        let mut toks = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let (c, next) = (chars[i], chars.get(i + 1).copied());
            if self.in_block {
                if c == '*' && next == Some('/') {
                    self.in_block = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            match c {
                '/' if next == Some('/') => break,
                '#' if self.lang == Lang::Php && next != Some('[') => break,
                '/' if next == Some('*') => {
                    self.in_block = true;
                    i += 2;
                }
                '"' | '\'' | '`' => {
                    // A Rust lifetime ('a) is no string.
                    if c == '\'' && self.lang == Lang::Rust && chars.get(i + 2) != Some(&'\'') {
                        i += 1;
                        continue;
                    }
                    let mut j = i + 1;
                    while j < chars.len() && chars[j] != c {
                        j += if chars[j] == '\\' { 2 } else { 1 };
                    }
                    toks.push(Tok::Str);
                    i = j + 1;
                }
                c if c.is_alphabetic() || c == '_' || c == '$' => {
                    let start = i;
                    i += 1;
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    toks.push(Tok::Ident(chars[start..i].iter().collect()));
                }
                c if c.is_whitespace() || c.is_ascii_digit() => i += 1,
                c => {
                    toks.push(Tok::Punct(c));
                    i += 1;
                }
            }
        }
        toks
    }
}

/// A class or the like whose body is open, and the depth inside it.
struct Open {
    name: Option<String>,
    depth: usize,
}

/// The functions and methods of `lines`, in order.
pub fn outline(lines: &[String], lang: Lang) -> Vec<Symbol> {
    if lang == Lang::Plain {
        return Vec::new();
    }
    let mut lexer = Lexer { lang, in_block: false };
    let toks: Vec<Vec<Tok>> = lines.iter().map(|line| lexer.line(line)).collect();
    let mut symbols: Vec<Symbol> = Vec::new();
    // Symbols whose body is open, with the depth their body starts at.
    let mut bodies: Vec<(usize, usize)> = Vec::new();
    let mut containers: Vec<Open> = Vec::new();
    // A declaration whose `{` is still to come, and what it is.
    let mut pending: Option<Pending> = None;
    let mut depth = 0usize;

    for (index, line) in toks.iter().enumerate() {
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let container = containers.last().filter(|c| c.depth == depth).and_then(|c| c.name.clone());
        let in_container = containers.last().is_some_and(|c| c.depth == depth);
        if let Some(name) = function(line, lang, in_container, depth, || next_opens(&toks, index)) {
            let (name, own) = name;
            symbols.push(Symbol { name, container: own.or(container), line: number, end: number });
            pending = Some(Pending::Function(symbols.len() - 1));
        } else if let Some(name) = container_name(line, lang) {
            pending = Some(Pending::Container(name));
        }
        for tok in line {
            match tok {
                Tok::Punct('{') => {
                    depth += 1;
                    match pending.take() {
                        Some(Pending::Function(at)) => bodies.push((at, depth)),
                        Some(Pending::Container(name)) => containers.push(Open { name: Some(name), depth }),
                        None => {}
                    }
                }
                Tok::Punct('}') => {
                    if let Some(&(at, open)) = bodies.last()
                        && open == depth
                    {
                        symbols[at].end = number;
                        bodies.pop();
                    }
                    if containers.last().is_some_and(|c| c.depth == depth) {
                        containers.pop();
                    }
                    depth = depth.saturating_sub(1);
                }
                // Ended before a body: an abstract method, an arrow function, `struct x;`.
                Tok::Punct(';') => pending = None,
                _ => {}
            }
        }
    }
    symbols
}

enum Pending {
    Function(usize),
    Container(String),
}

/// Whether the next line that has code starts with `{`.
fn next_opens(toks: &[Vec<Tok>], index: usize) -> bool {
    toks[index + 1..].iter().find(|l| !l.is_empty()).is_some_and(|l| l.first() == Some(&Tok::Punct('{')))
}

const CONTROL: &[&str] =
    &["if", "for", "foreach", "while", "switch", "catch", "return", "else", "do", "match", "loop", "sizeof", "new"];

/// A function declared on this line: its name, and the type it belongs to when the
/// declaration says (`Type::name` in C++, a Go receiver).
fn function(
    line: &[Tok],
    lang: Lang,
    in_container: bool,
    depth: usize,
    next_opens: impl Fn() -> bool,
) -> Option<(String, Option<String>)> {
    let named = |at: usize| line.get(at).and_then(Tok::ident).map(str::to_owned);
    match lang {
        Lang::Php => {
            let at = line.iter().position(|t| t.is("function"))?;
            let skip = usize::from(line.get(at + 1) == Some(&Tok::Punct('&')));
            let name = named(at + 1 + skip)?;
            (line.get(at + 2 + skip) == Some(&Tok::Punct('('))).then_some((name, None))
        }
        Lang::Rust => {
            let at = line.iter().position(|t| t.is("fn"))?;
            Some((named(at + 1)?, None))
        }
        Lang::Go => {
            if !line.first()?.is("func") {
                return None;
            }
            if line.get(1) == Some(&Tok::Punct('(')) {
                let close = line.iter().position(|t| *t == Tok::Punct(')'))?;
                let receiver = line[2..close].iter().rev().find_map(Tok::ident).map(str::to_owned);
                return Some((named(close + 1)?, receiver));
            }
            Some((named(1)?, None))
        }
        Lang::Js => js_function(line, in_container),
        Lang::C => {
            // A definition sits at the top level: a type, a name, and `(`.
            if depth > 0 && !in_container {
                return None;
            }
            let paren = line.iter().position(|t| *t == Tok::Punct('('))?;
            let name = named(paren.checked_sub(1)?)?;
            let before = line.get(paren.checked_sub(2)?)?;
            if CONTROL.contains(&name.as_str()) || line.last() == Some(&Tok::Punct(';')) {
                return None;
            }
            let typed = matches!(before, Tok::Ident(w) if !CONTROL.contains(&w.as_str()) && w != "else")
                || *before == Tok::Punct('*')
                || *before == Tok::Punct('&');
            let qualified = paren >= 4 && line[paren - 2] == Tok::Punct(':') && line[paren - 3] == Tok::Punct(':');
            let owner = if qualified { line.get(paren - 4).and_then(Tok::ident).map(str::to_owned) } else { None };
            let body = line.contains(&Tok::Punct('{')) || next_opens();
            ((typed || qualified) && body).then_some((name, owner))
        }
        Lang::Plain => None,
    }
}

/// `function name(`, `const name = (…) =>`, `const name = function`, and in a class body
/// `name(…) {` with any modifiers.
fn js_function(line: &[Tok], in_class: bool) -> Option<(String, Option<String>)> {
    let named = |at: usize| line.get(at).and_then(Tok::ident).map(str::to_owned);
    if let Some(at) = line.iter().position(|t| t.is("function")) {
        let skip = usize::from(line.get(at + 1) == Some(&Tok::Punct('*')));
        if let Some(name) = named(at + 1 + skip) {
            return Some((name, None));
        }
    }
    let declares = line.iter().position(|t| t.is("const") || t.is("let") || t.is("var"));
    if let Some(at) = declares
        && line.get(at + 2) == Some(&Tok::Punct('='))
    {
        let rest = &line[at + 3..];
        let rest = if rest.first().is_some_and(|t| t.is("async")) { &rest[1..] } else { rest };
        let arrow = rest.windows(2).any(|w| w == [Tok::Punct('='), Tok::Punct('>')]);
        if rest.first().is_some_and(|t| t.is("function")) || arrow {
            return Some((named(at + 1)?, None));
        }
    }
    if !in_class {
        return None;
    }
    const MODIFIERS: &[&str] =
        &["public", "private", "protected", "static", "async", "readonly", "override", "get", "set", "abstract"];
    let start = line.iter().position(|t| {
        !t.ident().is_some_and(|w| MODIFIERS.contains(&w)) && *t != Tok::Punct('*') && *t != Tok::Punct('#')
    })?;
    let name = named(start)?;
    let opens_body = line.last() == Some(&Tok::Punct('{'));
    let call = line.get(start + 1) == Some(&Tok::Punct('('));
    (call && opens_body && !CONTROL.contains(&name.as_str())).then_some((name, None))
}

/// The class, interface, trait, enum, impl or struct a line opens.
fn container_name(line: &[Tok], lang: Lang) -> Option<String> {
    let named = |at: usize| line.get(at).and_then(Tok::ident).map(str::to_owned);
    let keywords: &[&str] = match lang {
        Lang::Php => &["class", "interface", "trait", "enum"],
        Lang::Js => &["class", "interface"],
        Lang::Rust => &["trait"],
        Lang::C => &["class", "struct", "namespace"],
        Lang::Go | Lang::Plain => &[],
    };
    if lang == Lang::Rust
        && let Some(at) = line.iter().position(|t| t.is("impl"))
    {
        // `impl<T> Trait for Type<T>` belongs to Type.
        let rest = &line[at + 1..];
        let rest = skip_generics(rest);
        let target = match rest.iter().position(|t| t.is("for")) {
            Some(f) => skip_generics(&rest[f + 1..]),
            None => rest,
        };
        return target.iter().find_map(Tok::ident).map(str::to_owned);
    }
    let at = line.iter().position(|t| t.ident().is_some_and(|w| keywords.contains(&w)))?;
    // `new class(…)` and `class {` have no name to give.
    named(at + 1).filter(|name| name != "extends")
}

fn skip_generics(toks: &[Tok]) -> &[Tok] {
    if toks.first() != Some(&Tok::Punct('<')) {
        return toks;
    }
    let mut depth = 0;
    for (i, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Punct('<') => depth += 1,
            Tok::Punct('>') => {
                depth -= 1;
                if depth == 0 {
                    return &toks[i + 1..];
                }
            }
            _ => {}
        }
    }
    &[]
}

/// The first line of `symbol`'s body that has code: where a breakpoint on entering it goes.
/// Its declaration line when the body is on that line or empty.
pub fn body_start(lines: &[String], lang: Lang, symbol: &Symbol) -> u32 {
    let mut lexer = Lexer { lang, in_block: false };
    let first = usize::try_from(symbol.line).unwrap_or(1).saturating_sub(1);
    let mut opened = false;
    for (index, line) in lines.iter().enumerate().skip(first) {
        let toks = lexer.line(line);
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        if opened {
            match toks.first() {
                None => continue,
                Some(Tok::Punct('}')) => return symbol.line,
                Some(_) => return number,
            }
        }
        if let Some(brace) = toks.iter().position(|t| *t == Tok::Punct('{')) {
            match toks.get(brace + 1) {
                None => opened = true,
                Some(Tok::Punct('}')) => return symbol.line,
                // Code after the brace on the same line.
                Some(_) => return number,
            }
        }
        if index > first + 20 {
            break;
        }
    }
    symbol.line
}

/// A function name without its generic arguments and parameters:
/// `Iterator::fold<core::slice::Iter<Line>, ()>` → `Iterator::fold`. A name that is all in
/// angle brackets (`<anonymous>`) or does not balance them stays as it is.
fn bare(function: &str) -> String {
    if function.starts_with('<') && function.ends_with('>') && !function.contains("::") {
        return function.to_owned();
    }
    let mut out = String::with_capacity(function.len());
    let mut depth = 0usize;
    let mut previous = ' ';
    for c in function.chars() {
        match c {
            '(' if depth == 0 => break,
            '<' => depth += 1,
            // `->` in a type argument (`fn(&Line) -> &str`) closes nothing.
            '>' if depth > 0 && previous != '-' => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
        previous = c;
    }
    if depth == 0 { out } else { function.to_owned() }
}

/// The last part of a function name as debuggers and traces write it:
/// `App\Domain\Money->add`, `Money::zero`, `orders::order_total`, `Order.constructor` → the method.
pub fn short_name(function: &str) -> String {
    let name = bare(function);
    let cut = [name.rfind("->").map(|i| i + 2), name.rfind("::").map(|i| i + 2), name.rfind('.').map(|i| i + 1)];
    cut.into_iter().flatten().max().map_or_else(|| name.clone(), |at| name[at..].to_owned())
}

/// The type or module part of a function name, without what is above it:
/// `App\Domain\Money->add` → `Money`.
pub fn owner_name(function: &str) -> Option<String> {
    let name = bare(function);
    let short = short_name(&name);
    let owner = name.strip_suffix(short.as_str())?.trim_end_matches("->").trim_end_matches("::").trim_end_matches('.');
    let owner = owner.rsplit(['\\', ':', '.']).next()?;
    (!owner.is_empty()).then(|| owner.to_owned())
}

/// The symbol a function name of a stack or a trace stands for.
pub fn find<'a>(symbols: &'a [Symbol], function: &str) -> Option<&'a Symbol> {
    let name = short_name(function);
    let owner = owner_name(function);
    let candidates = || symbols.iter().filter(|s| s.name == name);
    candidates()
        .find(|s| owner.is_some() && s.container == owner)
        .or_else(|| candidates().find(|s| s.container.is_none() || owner.is_none()))
        .or_else(|| candidates().next())
}

/// The innermost symbol whose lines hold `line`.
pub fn around(symbols: &[Symbol], line: u32) -> Option<&Symbol> {
    symbols.iter().filter(|s| s.line <= line && line <= s.end).max_by_key(|s| s.line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    fn names(symbols: &[Symbol]) -> Vec<(String, Option<String>, u32, u32)> {
        symbols.iter().map(|s| (s.name.clone(), s.container.clone(), s.line, s.end)).collect()
    }

    fn sym(name: &str, container: Option<&str>, line: u32, end: u32) -> (String, Option<String>, u32, u32) {
        (name.into(), container.map(str::to_owned), line, end)
    }

    #[test]
    fn php_classes_methods_and_functions() {
        let code = lines(
            "<?php
final class OrderNotifier
{
    public function __construct(private readonly Mailer $mailer) {}

    public function notify(Order $order): void
    {
        $f = function ($x) { return $x; };
        $this->mailer->send('function fake() {}');
    }
}

function lineTotal(Line $line): int
{
    return $line->qty * $line->price; // function not() {
}",
        );
        let symbols = outline(&code, Lang::Php);
        assert_eq!(
            names(&symbols),
            [
                sym("__construct", Some("OrderNotifier"), 4, 4),
                sym("notify", Some("OrderNotifier"), 6, 10),
                sym("lineTotal", None, 13, 16),
            ]
        );
        assert_eq!(body_start(&code, Lang::Php, &symbols[1]), 8);
        assert_eq!(body_start(&code, Lang::Php, &symbols[2]), 15);
        assert_eq!(body_start(&code, Lang::Php, &symbols[0]), 4, "an empty body stays on its line");
        assert_eq!(around(&symbols, 9).map(|s| s.name.as_str()), Some("notify"));
        assert_eq!(around(&symbols, 12), None);
    }

    #[test]
    fn js_functions_arrows_and_class_methods() {
        let code = lines(
            "class Order {
  constructor(id, lines) {
    this.id = id;
  }
  async total() {
    if (this.id) {
      return 1;
    }
  }
}
function lineTotal(line) {
  return line.qty * line.price;
}
const money = (cents) => `${cents}`;
export const discount = async function (c) {
  return 0;
};",
        );
        let symbols = outline(&code, Lang::Js);
        assert_eq!(
            names(&symbols),
            [
                sym("constructor", Some("Order"), 2, 4),
                sym("total", Some("Order"), 5, 9),
                sym("lineTotal", None, 11, 13),
                sym("money", None, 14, 14),
                sym("discount", None, 15, 17),
            ]
        );
        assert_eq!(body_start(&code, Lang::Js, &symbols[2]), 12);
    }

    #[test]
    fn rust_functions_in_impls_and_traits() {
        let code = lines(
            "pub fn order_total<'a>(order: &'a Order) -> u64 {
    order.lines.iter().map(line_total).sum()
}

impl<T: Copy> Display for Ledger<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, \"{}\", '}')
    }
}",
        );
        let symbols = outline(&code, Lang::Rust);
        assert_eq!(names(&symbols), [sym("order_total", None, 1, 3), sym("fmt", Some("Ledger"), 6, 8)]);
    }

    #[test]
    fn c_definitions_but_not_prototypes_or_calls() {
        let code = lines(
            "static long line_total(const struct line *line);

static long line_total(const struct line *line)
{
    return line->qty * line->price;
}

int main(void) {
    long total = order_total(&orders[i]);
    if (total > 0) {
        printf(\"%ld\\n\", total);
    }
    return 0;
}",
        );
        let symbols = outline(&code, Lang::C);
        assert_eq!(names(&symbols), [sym("line_total", None, 3, 6), sym("main", None, 8, 14)]);
        assert_eq!(body_start(&code, Lang::C, &symbols[0]), 5);
        assert_eq!(body_start(&code, Lang::C, &symbols[1]), 9);
    }

    #[test]
    fn go_functions_and_methods() {
        let code = lines(
            "func (o *Order) Total() int {
\treturn 0
}

func main() {
}",
        );
        let symbols = outline(&code, Lang::Go);
        assert_eq!(names(&symbols), [sym("Total", Some("Order"), 1, 3), sym("main", None, 5, 6)]);
    }

    #[test]
    fn names_from_stacks_and_traces_find_their_symbol() {
        assert_eq!(short_name("App\\Domain\\Money->add"), "add");
        assert_eq!(short_name("Money::zero"), "zero");
        assert_eq!(short_name("orders::order_total"), "order_total");
        assert_eq!(short_name("Order.constructor"), "constructor");
        assert_eq!(short_name("lineTotal"), "lineTotal");
        assert_eq!(owner_name("App\\Domain\\Money->add").as_deref(), Some("Money"));
        assert_eq!(owner_name("orders::Ledger::fmt").as_deref(), Some("Ledger"));
        let fold = "core::iter::traits::iterator::Iterator::fold<core::slice::iter::Iter<orders::Line>, ()>";
        assert_eq!((short_name(fold).as_str(), owner_name(fold).as_deref()), ("fold", Some("Iterator")));
        assert_eq!(short_name("<anonymous>"), "<anonymous>");
        assert_eq!(short_name("<alloc::vec::Vec<T> as Extend<T>>::extend"), "extend");
        let closure =
            "core::iter::adapters::map::map_fold::{closure#0}<&orders::Line, &str, (), orders::main::{closure_env#0}>";
        assert_eq!((short_name(closure).as_str(), owner_name(closure).as_deref()), ("{closure#0}", Some("map_fold")));
        assert_eq!(short_name("apply<fn(&Line) -> &str>(f)"), "apply");
        assert_eq!(owner_name("lineTotal"), None);
        let symbols = vec![
            Symbol { name: "__construct".into(), container: Some("Line".into()), line: 3, end: 5 },
            Symbol { name: "__construct".into(), container: Some("Order".into()), line: 9, end: 12 },
        ];
        assert_eq!(find(&symbols, "Order->__construct").map(|s| s.line), Some(9));
        assert_eq!(find(&symbols, "__construct").map(|s| s.line), Some(3));
    }
}
