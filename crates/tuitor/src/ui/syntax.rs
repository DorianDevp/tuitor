//! A small line lexer: enough to separate keywords, types, strings, numbers and
//! comments in PHP, Rust, TypeScript/JavaScript, C and Go. Colours stay muted on
//! purpose; the debugger's own signals must stand out from the code.

use ratatui::style::Style;

use super::theme::{self, FAINT, FG, SYN_KEYWORD, SYN_NUMBER, SYN_PUNCT, SYN_STRING, SYN_TYPE, SYN_VAR};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Php,
    Rust,
    Js,
    C,
    Go,
    Plain,
}

impl Lang {
    pub fn from_path(path: &std::path::Path) -> Lang {
        match path.extension().and_then(|e| e.to_str()).unwrap_or_default() {
            "php" | "phtml" => Lang::Php,
            "rs" => Lang::Rust,
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => Lang::Js,
            "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "zig" => Lang::C,
            "go" => Lang::Go,
            _ => Lang::Plain,
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Lang::Php => &[
                "abstract",
                "and",
                "array",
                "as",
                "break",
                "case",
                "catch",
                "class",
                "clone",
                "const",
                "continue",
                "declare",
                "default",
                "do",
                "echo",
                "else",
                "elseif",
                "enum",
                "extends",
                "false",
                "final",
                "finally",
                "fn",
                "for",
                "foreach",
                "function",
                "global",
                "if",
                "implements",
                "include",
                "instanceof",
                "interface",
                "match",
                "namespace",
                "new",
                "null",
                "or",
                "private",
                "protected",
                "public",
                "readonly",
                "require",
                "require_once",
                "return",
                "self",
                "static",
                "switch",
                "throw",
                "trait",
                "true",
                "try",
                "use",
                "void",
                "while",
                "yield",
                "iterable",
                "mixed",
                "never",
            ],
            Lang::Rust => &[
                "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
                "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
                "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
                "where", "while", "Some", "None", "Ok", "Err",
            ],
            Lang::Js => &[
                "abstract",
                "as",
                "async",
                "await",
                "break",
                "case",
                "catch",
                "class",
                "const",
                "constructor",
                "continue",
                "default",
                "delete",
                "do",
                "else",
                "enum",
                "export",
                "extends",
                "false",
                "finally",
                "for",
                "from",
                "function",
                "if",
                "implements",
                "import",
                "in",
                "instanceof",
                "interface",
                "let",
                "new",
                "null",
                "of",
                "private",
                "protected",
                "public",
                "readonly",
                "return",
                "static",
                "super",
                "switch",
                "this",
                "throw",
                "true",
                "try",
                "type",
                "typeof",
                "undefined",
                "var",
                "void",
                "while",
                "yield",
            ],
            Lang::C => &[
                "auto",
                "bool",
                "break",
                "case",
                "char",
                "const",
                "continue",
                "default",
                "do",
                "double",
                "else",
                "enum",
                "extern",
                "false",
                "float",
                "for",
                "if",
                "inline",
                "int",
                "long",
                "return",
                "short",
                "signed",
                "sizeof",
                "static",
                "struct",
                "switch",
                "true",
                "typedef",
                "union",
                "unsigned",
                "void",
                "while",
                "pub",
                "fn",
                "var",
                "try",
                "defer",
                "nullptr",
                "class",
                "namespace",
                "template",
            ],
            Lang::Go => &[
                "break",
                "case",
                "chan",
                "const",
                "continue",
                "default",
                "defer",
                "else",
                "fallthrough",
                "false",
                "for",
                "func",
                "go",
                "goto",
                "if",
                "import",
                "interface",
                "map",
                "nil",
                "package",
                "range",
                "return",
                "select",
                "struct",
                "switch",
                "true",
                "type",
                "var",
            ],
            Lang::Plain => &[],
        }
    }
}

/// Splits one line into styled pieces. Lines inside block comments are detected by
/// their leading `*`, which covers doc blocks without tracking state across lines.
pub fn highlight(line: &str, lang: Lang) -> Vec<(String, Style)> {
    let trimmed = line.trim_start();
    if lang != Lang::Plain
        && (trimmed.starts_with("/*") || trimmed.starts_with("* ") || trimmed == "*" || trimmed.starts_with("*/"))
    {
        return vec![(line.to_owned(), theme::italic(FAINT))];
    }
    if lang == Lang::Plain {
        return vec![(line.to_owned(), theme::fg(FG))];
    }

    let keywords = lang.keywords();
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<(String, Style)> = Vec::new();
    let mut i = 0;
    let push = |out: &mut Vec<(String, Style)>, text: String, style: Style| {
        if let Some(last) = out.last_mut()
            && last.1 == style
        {
            last.0.push_str(&text);
            return;
        }
        out.push((text, style));
    };

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let rest: String;
        if c == '/' && next == Some('/') || (lang == Lang::Php && c == '#' && next != Some('[')) {
            rest = chars[i..].iter().collect();
            push(&mut out, rest, theme::italic(FAINT));
            break;
        }
        if c == '/' && next == Some('*') {
            let end = find_seq(&chars, i + 2, &['*', '/']).map_or(chars.len(), |e| e + 2);
            push(&mut out, chars[i..end].iter().collect(), theme::italic(FAINT));
            i = end;
            continue;
        }
        if c == '"' || c == '\'' || (c == '`' && lang == Lang::Js) {
            let end = string_end(&chars, i);
            push(&mut out, chars[i..end].iter().collect(), theme::fg(SYN_STRING));
            i = end;
            continue;
        }
        if c == '$' && lang == Lang::Php && next.is_some_and(is_ident_start) {
            let end = ident_end(&chars, i + 1);
            push(&mut out, chars[i..end].iter().collect(), theme::fg(SYN_VAR));
            i = end;
            continue;
        }
        if is_ident_start(c) {
            let mut end = ident_end(&chars, i);
            if lang == Lang::Rust && chars.get(end) == Some(&'!') {
                end += 1;
            }
            let word: String = chars[i..end].iter().collect();
            let style = if keywords.contains(&word.as_str()) {
                theme::fg(SYN_KEYWORD)
            } else if word.starts_with(|c: char| c.is_ascii_uppercase()) {
                theme::fg(SYN_TYPE)
            } else {
                theme::fg(FG)
            };
            push(&mut out, word, style);
            i = end;
            continue;
        }
        if c.is_ascii_digit() {
            let end = (i..chars.len())
                .find(|&j| !(chars[j].is_ascii_alphanumeric() || chars[j] == '.' || chars[j] == '_'))
                .unwrap_or(chars.len());
            push(&mut out, chars[i..end].iter().collect(), theme::fg(SYN_NUMBER));
            i = end;
            continue;
        }
        let style = if c.is_whitespace() { theme::fg(FG) } else { theme::fg(SYN_PUNCT) };
        push(&mut out, c.to_string(), style);
        i += 1;
    }
    out
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn ident_end(chars: &[char], start: usize) -> usize {
    (start..chars.len()).find(|&j| !(chars[j].is_alphanumeric() || chars[j] == '_')).unwrap_or(chars.len())
}

fn string_end(chars: &[char], start: usize) -> usize {
    let quote = chars[start];
    let mut j = start + 1;
    while j < chars.len() {
        match chars[j] {
            '\\' => j += 2,
            c if c == quote => return j + 1,
            _ => j += 1,
        }
    }
    chars.len()
}

fn find_seq(chars: &[char], from: usize, seq: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(seq.len() - 1)).find(|&j| chars[j..].starts_with(seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str, lang: Lang) -> Vec<(String, ratatui::style::Color)> {
        highlight(line, lang).into_iter().map(|(t, s)| (t, s.fg.unwrap_or_default())).collect()
    }

    #[test]
    fn php_variables_keywords_and_strings() {
        let k = kinds("    foreach ($items as $i) { echo 'x'; }", Lang::Php);
        assert!(k.contains(&("foreach".into(), SYN_KEYWORD)));
        assert!(k.contains(&("$items".into(), SYN_VAR)));
        assert!(k.contains(&("'x'".into(), SYN_STRING)));
    }

    #[test]
    fn php_attribute_is_not_a_comment() {
        let k = kinds("#[IsGranted('ORDER_ASSIGN')]", Lang::Php);
        assert!(k.iter().any(|(t, c)| t == "IsGranted" && *c == SYN_TYPE));
    }

    #[test]
    fn rust_comment_and_macro() {
        let k = kinds("let x = dec!(0.01); // tolerance", Lang::Rust);
        assert!(k.iter().any(|(t, c)| t.trim() == "dec!" && *c == FG));
        assert!(k.contains(&("0.01".into(), SYN_NUMBER)));
        assert!(k.last().is_some_and(|(t, c)| t == "// tolerance" && *c == FAINT));
    }

    #[test]
    fn escaped_quotes_stay_in_string() {
        let k = kinds(r#"s = "a \" b"; c"#, Lang::Js);
        assert!(k.contains(&(r#""a \" b""#.into(), SYN_STRING)));
    }
}
