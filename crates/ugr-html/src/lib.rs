//! Lightweight HTML document, CSS rule and entry-point support for game UIs.
//!
//! This is intentionally a runtime-oriented DOM model, not a full browser
//! parser or layout engine. It preserves the document structure needed by the
//! JavaScript facade while resolving local script entry points.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Errors produced while loading an HTML entry point.
#[derive(Debug, thiserror::Error)]
pub enum HtmlError {
    #[error("could not read entry file {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("HTML script error: {0}")]
    InvalidScript(String),
}

/// A lightweight DOM node suitable for game UI and scripting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlNode {
    pub tag: String,
    pub attributes: std::collections::BTreeMap<String, String>,
    pub children: Vec<HtmlNode>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CssRule {
    pub selector: String,
    pub declarations: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlDocument {
    pub root: HtmlNode,
    pub styles: Vec<CssRule>,
}

/// Parse the supported document subset (elements, attributes, text and CSS).
pub fn parse_document(source: &str) -> HtmlDocument {
    let mut roots = Vec::new();
    let mut stack: Vec<HtmlNode> = Vec::new();
    let mut cursor = 0;
    while let Some(open) = source[cursor..].find('<') {
        let open = cursor + open;
        if open > cursor {
            append_text(&mut stack, &mut roots, &source[cursor..open]);
        }
        let Some(end_offset) = source[open..].find('>') else {
            break;
        };
        let end = open + end_offset;
        let token = source[open + 1..end].trim();
        if token.starts_with('!') || token.starts_with('?') {
            cursor = end + 1;
            continue;
        }
        if let Some(name) = token.strip_prefix('/') {
            let name = name.trim().to_ascii_lowercase();
            if let Some(position) = stack.iter().rposition(|node| node.tag == name) {
                let mut completed = stack.split_off(position);
                if let Some(node) = completed.pop() {
                    append_node(&mut stack, &mut roots, node);
                }
            }
        } else {
            let self_closing = token.ends_with('/');
            let token = token.trim_end_matches('/').trim();
            let (tag, attributes) = parse_tag(token);
            let node = HtmlNode {
                tag,
                attributes,
                children: Vec::new(),
                text: String::new(),
            };
            if self_closing
                || matches!(
                    node.tag.as_str(),
                    "input" | "img" | "br" | "hr" | "meta" | "link"
                )
            {
                append_node(&mut stack, &mut roots, node);
            } else {
                stack.push(node);
            }
        }
        cursor = end + 1;
    }
    if cursor < source.len() {
        append_text(&mut stack, &mut roots, &source[cursor..]);
    }
    while let Some(node) = stack.pop() {
        append_node(&mut stack, &mut roots, node);
    }
    let root = HtmlNode {
        tag: "document".into(),
        attributes: Default::default(),
        children: roots,
        text: String::new(),
    };
    HtmlDocument {
        root,
        styles: parse_css(source),
    }
}

fn append_text(stack: &mut [HtmlNode], roots: &mut Vec<HtmlNode>, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if let Some(parent) = stack.last_mut() {
        parent.text.push_str(text);
    } else {
        roots.push(HtmlNode {
            tag: "#text".into(),
            attributes: Default::default(),
            children: Vec::new(),
            text: text.into(),
        });
    }
}

fn append_node(stack: &mut [HtmlNode], roots: &mut Vec<HtmlNode>, node: HtmlNode) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        roots.push(node);
    }
}

fn parse_tag(token: &str) -> (String, std::collections::BTreeMap<String, String>) {
    let mut parts = token.splitn(2, char::is_whitespace);
    let tag = parts.next().unwrap_or_default().to_ascii_lowercase();
    let mut attributes = std::collections::BTreeMap::new();
    let input = parts.next().unwrap_or_default();
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'=' {
            index += 1;
        }
        if start == index {
            index += 1;
            continue;
        }
        let name = input[start..index].to_ascii_lowercase();
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'=') {
            index += 1;
        }
        let value_start = index;
        if index < bytes.len() && (bytes[index] == b'"' || bytes[index] == b'\'') {
            let quote = bytes[index];
            index += 1;
            let value_start = index;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            attributes.insert(name, input[value_start..index].to_owned());
            index += usize::from(index < bytes.len());
        } else {
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            attributes.insert(name, input[value_start..index].to_owned());
        }
    }
    (tag, attributes)
}

fn parse_css(source: &str) -> Vec<CssRule> {
    let mut rules = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.to_ascii_lowercase().find("<style") {
        let Some(body_start) = rest[start..].find('>').map(|offset| start + offset + 1) else {
            break;
        };
        let Some(body_end) = rest[body_start..].to_ascii_lowercase().find("</style>") else {
            break;
        };
        let css = &rest[body_start..body_start + body_end];
        for rule in css.split('}') {
            let Some((selector, declarations)) = rule.split_once('{') else {
                continue;
            };
            let mut values = std::collections::BTreeMap::new();
            for declaration in declarations.split(';') {
                if let Some((name, value)) = declaration.split_once(':') {
                    values.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
                }
            }
            if !values.is_empty() {
                rules.push(CssRule {
                    selector: selector.trim().to_owned(),
                    declarations: values,
                });
            }
        }
        rest = &rest[body_start + body_end + 8..];
    }
    rules
}

/// Load a JavaScript or HTML entry file into one executable source string.
pub fn load_entry(path: impl AsRef<Path>) -> Result<String, HtmlError> {
    let path = path.as_ref();
    let source = std::fs::read_to_string(path).map_err(|source| HtmlError::Read {
        path: path.display().to_string(),
        source,
    })?;
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
        })
    {
        let scripts = extract_scripts(&source, path.parent().unwrap_or_else(|| Path::new(".")))?;
        let markup = serde_json::to_string(&source)
            .map_err(|error| HtmlError::InvalidScript(format!("could not encode HTML: {error}")))?;
        Ok(format!(
            "globalThis.__ugr_install_document({markup});\n{scripts}"
        ))
    } else {
        Ok(source)
    }
}

/// Extract executable scripts in document order.
pub fn extract_scripts(document: &str, base_dir: &Path) -> Result<String, HtmlError> {
    let bytes = document.as_bytes();
    let mut cursor = 0;
    let mut output = String::new();
    while let Some(relative) = find_tag(bytes, cursor, b"script") {
        let start = cursor + relative;
        let open_end = match bytes[start..].iter().position(|byte| *byte == b'>') {
            Some(offset) => start + offset,
            None => break,
        };
        let attributes = &document[start + 7..open_end];
        let body_start = open_end + 1;
        let close_start = find_closing_script(bytes, body_start).unwrap_or(bytes.len());
        let script_type = attribute(attributes, "type");
        let executable = script_type.as_deref().is_none_or(|value| {
            value.eq_ignore_ascii_case("text/javascript")
                || value.eq_ignore_ascii_case("application/javascript")
                || value.eq_ignore_ascii_case("module")
        });
        if !executable {
            cursor = if close_start < bytes.len() {
                bytes[close_start..]
                    .iter()
                    .position(|byte| *byte == b'>')
                    .map_or(bytes.len(), |offset| close_start + offset + 1)
            } else {
                bytes.len()
            };
            continue;
        }
        if let Some(src) = attribute(attributes, "src") {
            if src.starts_with("http://") || src.starts_with("https://") || src.starts_with("//") {
                return Err(HtmlError::InvalidScript(format!(
                    "remote HTML scripts are not supported: {src}"
                )));
            }
            let script_path = base_dir.join(PathBuf::from(src));
            let script =
                std::fs::read_to_string(&script_path).map_err(|source| HtmlError::Read {
                    path: script_path.display().to_string(),
                    source,
                })?;
            output.push_str(&script);
        } else {
            output.push_str(&document[body_start..close_start]);
        }
        output.push('\n');
        cursor = if close_start < bytes.len() {
            match bytes[close_start..].iter().position(|byte| *byte == b'>') {
                Some(offset) => close_start + offset + 1,
                None => bytes.len(),
            }
        } else {
            bytes.len()
        };
    }
    Ok(output)
}

fn find_tag(bytes: &[u8], from: usize, name: &[u8]) -> Option<usize> {
    let mut index = from;
    while index + name.len() + 1 < bytes.len() {
        if bytes[index] == b'<'
            && bytes[index + 1..index + 1 + name.len()].eq_ignore_ascii_case(name)
        {
            let next = bytes[index + 1 + name.len()];
            if next.is_ascii_whitespace() || next == b'>' {
                return Some(index - from);
            }
        }
        index += 1;
    }
    None
}

fn find_closing_script(bytes: &[u8], from: usize) -> Option<usize> {
    let mut index = from;
    while index + 9 < bytes.len() {
        if bytes[index] == b'<'
            && bytes[index + 1] == b'/'
            && bytes[index + 2..index + 8].eq_ignore_ascii_case(b"script")
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn attribute(attributes: &str, name: &str) -> Option<String> {
    let bytes = attributes.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'=' {
            index += 1;
        }
        if !attributes[start..index].eq_ignore_ascii_case(name) {
            while index < bytes.len() && bytes[index] != b' ' && bytes[index] != b'\t' {
                index += 1;
            }
            continue;
        }
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'=') {
            index += 1;
        }
        if index >= bytes.len() {
            return Some(String::new());
        }
        let quote = bytes[index];
        if quote == b'\'' || quote == b'"' {
            index += 1;
            let end = bytes[index..].iter().position(|byte| *byte == quote)? + index;
            return Some(attributes[index..end].to_owned());
        }
        let end = bytes[index..]
            .iter()
            .position(u8::is_ascii_whitespace)
            .map_or(bytes.len(), |offset| index + offset);
        return Some(attributes[index..end].to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::extract_scripts;

    #[test]
    fn extracts_inline_and_external_scripts_in_order() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("game.js"), "globalThis.external = 1;").unwrap();
        let source = extract_scripts(
            r#"<html><script src="game.js"></script><script>globalThis.inline = 2;</script></html>"#,
            directory.path(),
        )
        .unwrap();
        assert!(source.find("external").unwrap() < source.find("inline").unwrap());
    }

    #[test]
    fn ignores_non_javascript_script_types() {
        let source = extract_scripts(
            r#"<script type="application/json">{"x":1}</script><script>globalThis.ok = true;</script>"#,
            Path::new("."),
        )
        .unwrap();
        assert!(!source.contains("application/json"));
        assert!(source.contains("globalThis.ok"));
    }

    #[test]
    fn parses_dom_nodes_and_css_rules() {
        let document = super::parse_document(
            r#"<div id="root"><span class="title">Hello</span><input type="text" /></div><style>.title { color: red; }</style>"#,
        );
        assert_eq!(document.root.children[0].tag, "div");
        assert_eq!(document.root.children[0].children[0].tag, "span");
        assert_eq!(
            document.root.children[0].children[1].attributes["type"],
            "text"
        );
        assert_eq!(document.styles[0].selector, ".title");
        assert_eq!(document.styles[0].declarations["color"], "red");
    }
}
