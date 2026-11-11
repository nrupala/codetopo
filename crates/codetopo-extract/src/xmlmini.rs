// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! A minimal pull tokenizer for the XML dialect XLSX parts use.
//!
//! The XLSX reader needs exactly four operations: find elements by
//! name, read their attributes, read their text, and walk children in
//! document order. This tokenizer provides that and nothing else — no
//! namespaces are resolved (XLSX writers use a default namespace for
//! elements and prefixed names only for relationship ids, which the
//! reader matches by local name), no DTDs, no external entities.
//!
//! Robustness rules: the tokenizer never panics on malformed input;
//! every structural problem surfaces as `Err(String)` and the caller
//! turns it into an extraction error. Entity references handled: the
//! five XML builtins plus numeric character references (`&#65;`,
//! `&#x41;`). CDATA sections are read as text.

/// One XML event.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// `<name attr="v" ...>` — `self_closing` when the tag ends `/>`.
    Start { name: String, attrs: Vec<(String, String)>, self_closing: bool },
    /// `</name>`.
    End { name: String },
    /// Character data (entities already decoded).
    Text(String),
}

/// The local part of a possibly-prefixed name (`r:id` → `id`).
pub fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Look up an attribute by local name.
pub fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| local_name(k) == name)
        .map(|(_, v)| v.as_str())
}

/// Decode the XML entity references in `s`.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let ent = &tail[1..end];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let num = ent.strip_prefix('#');
                let parsed = match num {
                    Some(n) => {
                        if let Some(hex) = n.strip_prefix('x').or_else(|| n.strip_prefix('X')) {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            n.parse::<u32>().ok()
                        }
                    }
                    None => None,
                };
                match parsed.and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => {
                        // Unknown entity: keep it verbatim rather than
                        // dropping bytes silently.
                        out.push_str(&tail[..=end]);
                    }
                }
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Tokenize an XML document into events.
pub fn tokenize(xml: &str) -> Result<Vec<Event>, String> {
    let bytes = xml.as_bytes();
    let mut events = Vec::new();
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        if bytes[i] == b'<' {
            if xml[i..].starts_with("<!--") {
                let end = xml[i + 4..]
                    .find("-->")
                    .map(|p| i + 4 + p + 3)
                    .ok_or("unterminated XML comment")?;
                i = end;
            } else if xml[i..].starts_with("<![CDATA[") {
                let end = xml[i + 9..]
                    .find("]]>")
                    .map(|p| i + 9 + p)
                    .ok_or("unterminated CDATA section")?;
                events.push(Event::Text(xml[i + 9..end].to_string()));
                i = end + 3;
            } else if xml[i..].starts_with("<?") {
                let end = xml[i + 2..]
                    .find("?>")
                    .map(|p| i + 2 + p + 2)
                    .ok_or("unterminated processing instruction")?;
                i = end;
            } else if xml[i..].starts_with("<!") {
                // DOCTYPE and friends: skip to the closing '>'.
                let end = xml[i..]
                    .find('>')
                    .map(|p| i + p + 1)
                    .ok_or("unterminated markup declaration")?;
                i = end;
            } else if xml[i..].starts_with("</") {
                let end = xml[i..]
                    .find('>')
                    .map(|p| i + p)
                    .ok_or("unterminated end tag")?;
                let name = xml[i + 2..end].trim().to_string();
                events.push(Event::End { name });
                i = end + 1;
            } else {
                // Start tag: scan to the closing '>', respecting quotes.
                let mut j = i + 1;
                let mut quote: Option<u8> = None;
                while j < n {
                    match (quote, bytes[j]) {
                        (Some(q), c) if c == q => quote = None,
                        (Some(_), _) => {}
                        (None, b'"') => quote = Some(b'"'),
                        (None, b'\'') => quote = Some(b'\''),
                        (None, b'>') => break,
                        _ => {}
                    }
                    j += 1;
                }
                if j >= n {
                    return Err("unterminated start tag".to_string());
                }
                let inner = &xml[i + 1..j];
                let self_closing = inner.trim_end().ends_with('/');
                let inner = inner.trim_end().trim_end_matches('/');
                let (name, attrs) = parse_tag(inner)?;
                events.push(Event::Start { name, attrs, self_closing });
                i = j + 1;
            }
        } else {
            let end = xml[i..].find('<').map(|p| i + p).unwrap_or(n);
            let text = decode_entities(&xml[i..end]);
            if !text.is_empty() {
                events.push(Event::Text(text));
            }
            i = end;
        }
    }
    Ok(events)
}

/// Parse the inside of a start tag (`name a="1" b='2'`) into its name
/// and attribute list (attribute values entity-decoded).
fn parse_tag(inner: &str) -> Result<(String, Vec<(String, String)>), String> {
    let inner = inner.trim();
    let mut parts = inner.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("").to_string();
    if name.is_empty() {
        return Err("empty tag name".to_string());
    }
    let rest = parts.next().unwrap_or("");
    let mut attrs = Vec::new();
    let b = rest.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        while i < b.len() && b[i] != b'=' && !(b[i] as char).is_whitespace() {
            i += 1;
        }
        let key = &rest[start..i];
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            // Valueless attribute: record it empty rather than failing.
            attrs.push((key.to_string(), String::new()));
            continue;
        }
        i += 1;
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= b.len() || (b[i] != b'"' && b[i] != b'\'') {
            return Err(format!("attribute {key} has no quoted value"));
        }
        let q = b[i];
        i += 1;
        let vstart = i;
        while i < b.len() && b[i] != q {
            i += 1;
        }
        if i >= b.len() {
            return Err(format!("attribute {key} value unterminated"));
        }
        attrs.push((key.to_string(), decode_entities(&rest[vstart..i])));
        i += 1;
    }
    Ok((name, attrs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_elements_attrs_and_text() {
        let ev = tokenize(r#"<a x="1" r:id="rId2"><b>hi &amp; bye</b><c/></a>"#).unwrap();
        assert_eq!(
            ev,
            vec![
                Event::Start {
                    name: "a".into(),
                    attrs: vec![("x".into(), "1".into()), ("r:id".into(), "rId2".into())],
                    self_closing: false
                },
                Event::Start { name: "b".into(), attrs: vec![], self_closing: false },
                Event::Text("hi & bye".into()),
                Event::End { name: "b".into() },
                Event::Start { name: "c".into(), attrs: vec![], self_closing: true },
                Event::End { name: "a".into() },
            ]
        );
    }

    #[test]
    fn numeric_entities_decode() {
        assert_eq!(decode_entities("&#65;&#x42;"), "AB");
        assert_eq!(decode_entities("plain"), "plain");
    }
}
