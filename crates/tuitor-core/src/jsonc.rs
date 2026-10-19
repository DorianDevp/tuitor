//! `.vscode/launch.json` is JSON with comments and trailing commas.

/// Removes `//` and `/* */` comments and trailing commas, leaving strings intact.
pub fn strip(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        let next = chars.peek().copied();
        match (c, next) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
            }
            (',', _) if next_significant_closes(chars.clone()) => {}
            _ => out.push(c),
        }
    }
    out
}

/// True when the next non-blank, non-comment character closes an object or array.
fn next_significant_closes(mut chars: impl Iterator<Item = char>) -> bool {
    let mut chars = std::iter::from_fn(move || chars.next()).peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            (c, _) if c.is_whitespace() => {}
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
            }
            ('}' | ']', _) => return true,
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::strip;
    use serde_json::Value;

    fn parse(input: &str) -> Value {
        serde_json::from_str(&strip(input)).expect("valid after strip")
    }

    #[test]
    fn strips_comments_and_trailing_commas() {
        let v = parse(
            r#"{
                // Use IntelliSense to learn about possible attributes.
                "version": "0.2.0", /* inline */
                "configurations": [
                    { "name": "a", "port": 9003, },
                ],
            }"#,
        );
        assert_eq!(v["configurations"][0]["port"], 9003);
    }

    #[test]
    fn keeps_comment_like_text_inside_strings() {
        let v = parse(r#"{ "url": "http://x/*y*/", "q": "a \" // b", }"#);
        assert_eq!(v["url"], "http://x/*y*/");
        assert_eq!(v["q"], "a \" // b");
    }

    #[test]
    fn trailing_comma_before_comment_and_brace() {
        let v = parse("[1, 2, // two\n]");
        assert_eq!(v, serde_json::json!([1, 2]));
    }
}
