//! Helpers that do not touch GTK: link detection, the internal markup format,
//! search text cleanup and the Gnote importer.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::i18n::gettext;
use crate::notes::NoteInfo;

fn url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let ip_number = r"(?:\d{1,2}|1\d{2}|2[0-4]\d|25[0-5])";
        let ip_address = format!(r"(?:(?:{ip_number}.){{3}}{ip_number})");
        let domain = r"(?:(?:[a-z\x{a1}-\x{ffff}0-9]-?)*[a-z\x{a1}-\x{ffff}0-9]+)(?:\.(?:[a-z\x{a1}-\x{ffff}0-9]-?)*[a-z\x{a1}-\x{ffff}0-9]+)*(?:\.(?:[a-z\x{a1}-\x{ffff}]{2,}))";
        let pattern = format!(
            r"(?i)(?:(?:https?|ftp)://)(?:\S+(?::\S*)?@)?(?:{ip_address}|{domain})(?::\d{{2,5}})?(?:/\S*)?(?:\?\S*)?\z"
        );
        Regex::new(&pattern).expect("valid url regex")
    })
}

#[allow(dead_code)]
pub fn ends_with_url(text: &str) -> bool {
    url_regex().is_match(text)
}

/// If `text` ends with a URL, return the *character* offset where it starts.
pub fn get_url_start(text: &str) -> Option<usize> {
    url_regex().find(text).map(|m| text[..m.start()].chars().count())
}

/// One piece of a note's internal markup.
///
/// The format is plain text where `#` starts a command:
/// `##` is a literal `#`, `#check:0` / `#check:1` is a checkbox,
/// `#bullet:` is a bullet and `#tag:name:` opens or closes a formatting tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Text(String),
    Check(bool),
    Bullet,
    Tag(String),
}

fn next_boundary(text: &str, mut i: usize) -> usize {
    i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

pub fn parse_markup(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut pending = String::new();
    let mut current = 0;

    loop {
        let Some(rel) = text[current..].find('#') else {
            pending.push_str(&text[current..]);
            break;
        };
        let next = current + rel;
        pending.push_str(&text[current..next]);
        let rest = &text[next..];

        if rest.starts_with("##") {
            pending.push('#');
            current = next + 2;
        } else if rest.starts_with("#check") {
            flush(&mut tokens, &mut pending);
            tokens.push(Token::Check(bytes.get(next + 7) == Some(&b'1')));
            current = next_boundary(text, next + 8);
        } else if rest.starts_with("#bullet") {
            flush(&mut tokens, &mut pending);
            tokens.push(Token::Bullet);
            current = next_boundary(text, next + 8);
        } else if let Some(end) = rest.starts_with("#tag:").then(|| text[next + 5..].find(':')).flatten() {
            flush(&mut tokens, &mut pending);
            let name = &text[next + 5..next + 5 + end];
            tokens.push(Token::Tag(name.to_string()));
            current = next + 6 + name.len();
        } else {
            eprintln!("formatting error detected - attempting to fix");
            pending.push('#');
            current = next + 1;
        }
    }

    flush(&mut tokens, &mut pending);
    tokens
}

fn flush(tokens: &mut Vec<Token>, pending: &mut String) {
    if !pending.is_empty() {
        tokens.push(Token::Text(std::mem::take(pending)));
    }
}

/// Strip the markup from a note's text and lowercase it, for searching.
pub fn clean_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let find = |c: char, from: usize| {
        chars.get(from..).and_then(|rest| rest.iter().position(|&x| x == c)).map(|p| p + from)
    };
    let slice = |a: usize, b: usize| -> String {
        let b = b.min(chars.len());
        if a >= b {
            String::new()
        } else {
            chars[a..b].iter().collect()
        }
    };
    let starts = |at: usize, s: &str| s.chars().enumerate().all(|(i, c)| chars.get(at + i) == Some(&c));

    let mut current = 0;
    let mut new_text = String::new();
    loop {
        let Some(next) = find('#', current) else {
            // Same as Python's text[current:-1]
            new_text.push_str(&slice(current, chars.len().saturating_sub(1)));
            return new_text.to_lowercase();
        };
        new_text.push_str(&slice(current, next));
        if starts(next, "##") {
            new_text.push('#');
            current = next + 2;
        } else if starts(next, "#check") || starts(next, "#bullet") {
            current = next + 8;
        } else if starts(next, "#tag") {
            current = find(':', next + 6).map(|i| i + 1).unwrap_or(chars.len());
        } else {
            current += 1;
        }
    }
}

const GNOTE_TO_INTERNAL_MAP: &[(&str, &str)] = &[
    ("bold", "bold"),
    ("italic", "italic"),
    ("underline", "underline"),
    ("strikethrough", "strikethrough"),
    ("highlight", "highlight"),
    ("url", "link"),
    ("small", "small"),
    ("large", "large"),
    ("huge", "larger"),
];

const GNOTE_NS: &str = "http://beatniksoftware.com/tomboy";

/// Convert a Gnote/Tomboy `.note` file. Returns (group name, note, is template).
pub fn gnote_to_internal_format(path: &Path) -> Result<(String, NoteInfo, bool), String> {
    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let doc = roxmltree::Document::parse(&data).map_err(|e| e.to_string())?;
    let root = doc.root_element();

    fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
        node.children()
            .find(|n| n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(GNOTE_NS))
    }

    fn process(node: roxmltree::Node<'_, '_>) -> String {
        let tag = GNOTE_TO_INTERNAL_MAP
            .iter()
            .find(|(gnote, _)| *gnote == node.tag_name().name())
            .map(|(_, internal)| format!("#tag:{internal}:"))
            .unwrap_or_default();
        let mut text = tag.clone();
        for c in node.children() {
            if c.is_text() {
                text.push_str(&c.text().unwrap_or("").replace('#', "##"));
            } else if c.is_element() {
                text.push_str(&process(c));
            }
        }
        text.push_str(&tag);
        text
    }

    let mut info = NoteInfo {
        title: child(root, "title").and_then(|n| n.text()).map(str::to_string),
        ..Default::default()
    };
    let content = child(root, "text")
        .and_then(|t| child(t, "note-content"))
        .ok_or_else(|| "missing note-content".to_string())?;
    info.text = process(content);

    let mut category = None;
    let mut is_template = false;
    if let Some(tags) = child(root, "tags") {
        for tag in tags.children().filter(|n| n.is_element()) {
            let text = tag.text().unwrap_or("");
            if text == "system:template" {
                is_template = true;
            } else if let Some(name) = text.strip_prefix("system:notebook:") {
                category = Some(name.to_string());
            }
        }
    }

    Ok((category.unwrap_or_else(|| gettext("Unfiled")), info, is_template))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_urls() {
        assert!(ends_with_url("see https://linuxmint.com"));
        assert!(ends_with_url("ftp://192.168.0.1:21/file"));
        assert!(!ends_with_url("https://linuxmint.com and more"));
        assert!(!ends_with_url("linuxmint.com"));
        assert_eq!(get_url_start("go to http://example.org/a?b=c"), Some(6));
        assert_eq!(get_url_start("é http://example.org"), Some(2));
    }

    #[test]
    fn parses_markup() {
        use Token::*;
        assert_eq!(
            parse_markup("#check:1buy ##milk #tag:bold:now#tag:bold:\n#bullet:x"),
            vec![
                Check(true),
                Text("buy #milk ".into()),
                Tag("bold".into()),
                Text("now".into()),
                Tag("bold".into()),
                Text("\n".into()),
                Bullet,
                Text("x".into()),
            ]
        );
        assert_eq!(parse_markup("a#zb"), vec![Text("a#zb".into())]);
        assert_eq!(parse_markup(""), vec![]);
    }

    #[test]
    fn cleans_text() {
        assert_eq!(clean_text("#check:0Buy ##Milk #tag:bold:now#tag:bold:."), "buy #milk now");
    }

    #[test]
    fn imports_gnote() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.note");
        std::fs::write(
            &path,
            r#"<?xml version="1.0"?>
<note version="0.3" xmlns:link="http://beatniksoftware.com/tomboy/link" xmlns="http://beatniksoftware.com/tomboy">
  <title>Shopping</title>
  <text xml:space="preserve"><note-content version="0.1">Shopping
<bold>eggs</bold> #1 <link:url>http://x.org</link:url></note-content></text>
  <tags><tag>system:notebook:Home</tag></tags>
</note>"#,
        )
        .unwrap();
        let (group, info, template) = gnote_to_internal_format(&path).unwrap();
        assert_eq!(group, "Home");
        assert!(!template);
        assert_eq!(info.title.as_deref(), Some("Shopping"));
        assert_eq!(info.text, "Shopping\n#tag:bold:eggs#tag:bold: ##1 #tag:link:http://x.org#tag:link:");
    }
}
