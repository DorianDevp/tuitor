//! The parts of LSP that paths are built from, read out of the JSON servers send: URIs,
//! positions, symbols, locations and highlights.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// `SymbolKind`s: those of functions, and of what holds them.
pub const FILE: u64 = 1;
pub const MODULE: u64 = 2;
pub const NAMESPACE: u64 = 3;
pub const PACKAGE: u64 = 4;
pub const METHOD: u64 = 6;
pub const CONSTRUCTOR: u64 = 9;
pub const FUNCTION: u64 = 12;
/// `DocumentHighlightKind.Write`.
pub const WRITE: u64 = 3;

/// A position as LSP counts it: a 0-based line, and UTF-16 code units into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Pos {
    pub line: u32,
    pub character: u32,
}

impl Pos {
    pub fn new(line: u32, character: u32) -> Pos {
        Pos { line, character }
    }

    pub fn json(self) -> Value {
        json!({ "line": self.line, "character": self.character })
    }

    fn read(value: &Value) -> Option<Pos> {
        let number = |key: &str| value.get(key)?.as_u64().and_then(|n| u32::try_from(n).ok());
        Some(Pos { line: number("line")?, character: number("character")? })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: Pos,
    pub end: Pos,
}

impl Span {
    pub fn read(value: &Value) -> Option<Span> {
        Some(Span { start: Pos::read(value.get("start")?)?, end: Pos::read(value.get("end")?)? })
    }

    pub fn contains(&self, pos: Pos) -> bool {
        self.start <= pos && pos <= self.end
    }

    pub fn has_line(&self, line: u32) -> bool {
        (self.start.line..=self.end.line).contains(&line)
    }
}

/// `file://` and the path, with what URIs do not allow percent-encoded.
pub fn uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// The path of a `file:` URI.
pub fn path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let mut iter = rest.bytes();
    while let Some(byte) = iter.next() {
        if byte != b'%' {
            bytes.push(byte);
            continue;
        }
        let hex: Vec<u8> = iter.by_ref().take(2).collect();
        let decoded = std::str::from_utf8(&hex).ok().and_then(|h| u8::from_str_radix(h, 16).ok())?;
        bytes.push(decoded);
    }
    Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
}

/// `{ "uri": … }`, what requests about a document name it with.
pub fn document(path: &Path) -> Value {
    json!({ "uri": uri(path) })
}

/// The UTF-16 column of byte `byte` of `line`.
pub fn utf16(line: &str, byte: usize) -> u32 {
    let before = line.get(..byte).unwrap_or(line);
    u32::try_from(before.encode_utf16().count()).unwrap_or(u32::MAX)
}

/// The byte of `line` at UTF-16 column `column`; the end of the line past it.
pub fn byte(line: &str, column: u32) -> usize {
    let mut units = 0;
    for (at, c) in line.char_indices() {
        if units >= column {
            return at;
        }
        units += u32::try_from(c.len_utf16()).unwrap_or(1);
    }
    line.len()
}

/// A symbol of `textDocument/documentSymbol`, with those declared in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: u64,
    pub range: Span,
    /// Its name.
    pub selection: Span,
    pub children: Vec<Symbol>,
}

/// The answer to `textDocument/documentSymbol`: a tree of `DocumentSymbol`s, or a flat list of
/// `SymbolInformation`, which knows no nesting.
pub fn symbols(value: &Value) -> Vec<Symbol> {
    let Some(items) = value.as_array() else { return Vec::new() };
    items.iter().filter_map(symbol).collect()
}

fn symbol(value: &Value) -> Option<Symbol> {
    let name = value.get("name")?.as_str()?.to_owned();
    let kind = value.get("kind")?.as_u64()?;
    let (range, selection) = match value.get("location") {
        Some(location) => {
            let range = Span::read(location.get("range")?)?;
            (range, range)
        }
        None => (Span::read(value.get("range")?)?, Span::read(value.get("selectionRange")?)?),
    };
    let children = value.get("children").map(symbols).unwrap_or_default();
    Some(Symbol { name, kind, range, selection, children })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub path: PathBuf,
    pub span: Span,
}

/// A `Location`, a list of them, or of `LocationLink`s.
pub fn locations(value: &Value) -> Vec<Location> {
    let one = |value: &Value| {
        let uri = value.get("uri").or_else(|| value.get("targetUri"))?.as_str()?;
        let span = value.get("range").or_else(|| value.get("targetSelectionRange"))?;
        Some(Location { path: path(uri)?, span: Span::read(span)? })
    };
    match value {
        Value::Array(items) => items.iter().filter_map(one).collect(),
        Value::Object(_) => one(value).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// An occurrence `textDocument/documentHighlight` gives: of the same variable, field or
/// function as the one asked about.
#[derive(Debug, Clone, PartialEq)]
pub struct Highlight {
    pub span: Span,
    /// Text, read or write; servers that tell no kind say text.
    pub kind: u64,
}

pub fn highlights(value: &Value) -> Vec<Highlight> {
    let Some(items) = value.as_array() else { return Vec::new() };
    let one = |item: &Value| {
        let kind = item.get("kind").and_then(Value::as_u64).unwrap_or(1);
        Some(Highlight { span: Span::read(item.get("range")?)?, kind })
    };
    items.iter().filter_map(one).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_encode_what_they_must_and_decode_back() {
        let odd = Path::new("/home/me/my shop/Zamówienia/50%.php");
        let encoded = uri(odd);
        assert_eq!(encoded, "file:///home/me/my%20shop/Zam%C3%B3wienia/50%25.php");
        assert_eq!(path(&encoded).as_deref(), Some(odd));
        // As some servers send it, with less encoded.
        assert_eq!(path("file:///home/me/my shop/a.php").as_deref(), Some(Path::new("/home/me/my shop/a.php")));
        assert_eq!(path("https://example.com/a.php"), None);
    }

    #[test]
    fn columns_count_utf16_units() {
        let line = "    $opis = 'zażółć 😀'; $x = 1;";
        let x = line.find("$x").expect("$x");
        let column = utf16(line, x);
        assert_eq!(column, u32::try_from(x).expect("small") - 6, "ż ó ł ć are 2 bytes in 1 unit, 😀 4 bytes in 2");
        assert_eq!(byte(line, column), x);
        assert_eq!(byte(line, 999), line.len());
    }

    #[test]
    fn symbols_nest_or_come_flat() {
        let tree = json!([{
            "name": "Money", "kind": 5,
            "range": { "start": { "line": 8, "character": 0 }, "end": { "line": 24, "character": 1 } },
            "selectionRange": { "start": { "line": 8, "character": 12 }, "end": { "line": 8, "character": 17 } },
            "children": [{
                "name": "lineTotal", "kind": 6,
                "range": { "start": { "line": 10, "character": 4 }, "end": { "line": 18, "character": 5 } },
                "selectionRange": { "start": { "line": 11, "character": 27 }, "end": { "line": 11, "character": 36 } }
            }]
        }]);
        let symbols = symbols(&tree);
        assert_eq!(symbols[0].children[0].name, "lineTotal");
        assert_eq!(symbols[0].children[0].selection.start, Pos::new(11, 27));
        let flat = json!([{ "name": "main", "kind": 12, "location": {
            "uri": "file:///w/main.go",
            "range": { "start": { "line": 66, "character": 0 }, "end": { "line": 105, "character": 1 } }
        } }]);
        assert_eq!(super::symbols(&flat)[0].range.end.line, 105);
    }

    #[test]
    fn locations_come_as_one_a_list_or_links() {
        let range = json!({ "start": { "line": 3, "character": 1 }, "end": { "line": 3, "character": 5 } });
        let one = json!({ "uri": "file:///w/a.ts", "range": range });
        let link = json!([{ "targetUri": "file:///w/b.ts", "targetRange": range, "targetSelectionRange": range }]);
        assert_eq!(locations(&one)[0].path, Path::new("/w/a.ts"));
        assert_eq!(locations(&link)[0].path, Path::new("/w/b.ts"));
        assert!(locations(&Value::Null).is_empty());
    }
}
