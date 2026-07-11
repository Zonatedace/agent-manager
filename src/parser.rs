use crate::models::TodoItem;
use regex::Regex;
use std::sync::OnceLock;

fn checkbox_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Matches: - [ ] text  /  * [x] text  /  1. [X] text  (with optional indent)
        Regex::new(r"(?i)^(\s*)(?:[-*+]|\d+\.)\s+\[([ xX])\]\s+(.+?)\s*$").unwrap()
    })
}

fn heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(#{1,6})\s+(.+?)\s*$").unwrap())
}

/// Parse markdown checkbox items from a TODO file body.
pub fn parse_todos(content: &str) -> Vec<TodoItem> {
    let mut items = Vec::new();
    let mut current_section: Option<String> = None;

    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;

        if let Some(caps) = heading_re().captures(line) {
            let title = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            // Skip the top-level "# TODO - foo" title as a section; use ## and deeper
            let level = caps.get(1).map(|m| m.as_str().len()).unwrap_or(1);
            if level >= 2 && !title.is_empty() {
                current_section = Some(title.to_string());
            }
            continue;
        }

        if let Some(caps) = checkbox_re().captures(line) {
            let mark = caps.get(2).map(|m| m.as_str()).unwrap_or(" ");
            let text = caps
                .get(3)
                .map(|m| m.as_str().trim())
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                continue;
            }
            let done = mark.eq_ignore_ascii_case("x");
            items.push(TodoItem {
                text,
                done,
                line: line_no,
                section: current_section.clone(),
            });
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open_and_done() {
        let md = r#"
# TODO - demo

## Features

- [ ] open item one
- [x] done item
* [X] also done
1. [ ] numbered open

## Other

- [ ] nested under other
"#;
        let items = parse_todos(md);
        assert_eq!(items.len(), 5);
        assert!(!items[0].done);
        assert_eq!(items[0].text, "open item one");
        assert_eq!(items[0].section.as_deref(), Some("Features"));
        assert!(items[1].done);
        assert!(items[2].done);
        assert!(!items[3].done);
        assert_eq!(items[4].section.as_deref(), Some("Other"));
    }
}
