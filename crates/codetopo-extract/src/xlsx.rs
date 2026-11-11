// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! XLSX reading: the workbook bytes in, the workbook's own cells out.
//!
//! An `.xlsx` is a ZIP of XML parts. This reader opens the archive
//! (via the `zip` crate), resolves the workbook's sheet list through
//! `xl/workbook.xml` + its relationships part, and reads each
//! worksheet's cells: the stored formula (from `<f>`, verbatim, no
//! leading `=` — the caller adds it per the profile's formula-field
//! rule), the literal value for hard-coded cells, the shared-string
//! text, and the cell's number format (resolved through
//! `xl/styles.xml`) so the RA extractor can read a currency unit from
//! the artefact rather than inferring one.
//!
//! The cached `<v>` of a formula cell is read but NEVER trusted as
//! the cell's value by this crate's consumers: the RA profile's value
//! rule is recomputation from the artefact's own formulas. The cached
//! value is carried here only so a caller can see what the workbook
//! last displayed.
//!
//! Robustness: no panics on malformed input — every structural
//! problem is an [`ExtractError::InvalidWorkbook`].

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use crate::xmlmini::{attr, local_name, tokenize, Event};
use crate::ExtractError;

/// One worksheet, in workbook order.
#[derive(Debug, Clone)]
pub struct Sheet {
    /// The sheet's display name, exactly as the workbook names it.
    pub name: String,
    /// Non-empty cells by reference (`B12`), in first-seen order.
    pub cells: Vec<Cell>,
}

/// One non-empty cell of a worksheet.
#[derive(Debug, Clone)]
pub struct Cell {
    /// The cell reference as stored (`B12`); case is preserved.
    pub reference: String,
    /// 1-based row number parsed from the reference.
    pub row: u32,
    /// 1-based column number parsed from the reference (A = 1).
    pub column: u32,
    /// The stored formula text WITHOUT the leading `=`, when the cell
    /// carries a formula; `None` for literal cells.
    pub formula: Option<String>,
    /// The literal value of a hard-coded numeric cell.
    pub number: Option<f64>,
    /// The text of a string cell (shared string or inline string).
    pub text: Option<String>,
    /// The cached value of a formula cell, as last written by the
    /// workbook's author application. Display data only.
    pub cached: Option<f64>,
    /// The resolved number-format code (e.g. `$#,##0`), when the
    /// cell's style names one.
    pub number_format: Option<String>,
}

/// Read a whole workbook from its bytes.
pub fn read_workbook(bytes: &[u8]) -> Result<Vec<Sheet>, ExtractError> {
    let invalid = |msg: &str| ExtractError::InvalidWorkbook(msg.to_string());
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| ExtractError::InvalidWorkbook(format!("not an xlsx (zip) archive: {e}")))?;

    let workbook_xml = read_part(&mut archive, "xl/workbook.xml")?
        .ok_or_else(|| invalid("xl/workbook.xml missing"))?;
    let rels_xml = read_part(&mut archive, "xl/_rels/workbook.xml.rels")?
        .ok_or_else(|| invalid("xl/_rels/workbook.xml.rels missing"))?;

    // rid → target path (targets are relative to xl/).
    let mut rels: BTreeMap<String, String> = BTreeMap::new();
    for ev in tokenize(&rels_xml).map_err(ExtractError::InvalidWorkbook)? {
        if let Event::Start { name, attrs, .. } = &ev {
            if local_name(name) == "Relationship" {
                if let (Some(id), Some(target)) = (attr(attrs, "Id"), attr(attrs, "Target")) {
                    rels.insert(id.to_string(), target.to_string());
                }
            }
        }
    }

    // (sheet name, rid) in workbook order.
    let mut sheet_refs: Vec<(String, String)> = Vec::new();
    for ev in tokenize(&workbook_xml).map_err(ExtractError::InvalidWorkbook)? {
        if let Event::Start { name, attrs, .. } = &ev {
            if local_name(name) == "sheet" {
                let sname = attr(attrs, "name")
                    .ok_or_else(|| invalid("workbook sheet without a name"))?;
                let rid = attr(attrs, "id")
                    .ok_or_else(|| invalid("workbook sheet without a relationship id"))?;
                sheet_refs.push((sname.to_string(), rid.to_string()));
            }
        }
    }
    if sheet_refs.is_empty() {
        return Err(invalid("workbook declares no sheets"));
    }

    let shared_strings = match read_part(&mut archive, "xl/sharedStrings.xml")? {
        Some(xml) => parse_shared_strings(&xml)?,
        None => Vec::new(),
    };
    let formats = match read_part(&mut archive, "xl/styles.xml")? {
        Some(xml) => parse_styles(&xml)?,
        None => NumberFormats::default(),
    };

    let mut sheets = Vec::new();
    for (name, rid) in &sheet_refs {
        let target = rels
            .get(rid)
            .ok_or_else(|| invalid(&format!("sheet {name}: relationship {rid} not found")))?;
        let path = if let Some(abs) = target.strip_prefix('/') {
            abs.to_string()
        } else {
            format!("xl/{target}")
        };
        let xml = read_part(&mut archive, &path)?
            .ok_or_else(|| invalid(&format!("sheet {name}: part {path} missing")))?;
        sheets.push(parse_sheet(name.clone(), &xml, &shared_strings, &formats)?);
    }
    Ok(sheets)
}

/// Read one ZIP part as UTF-8 text; `Ok(None)` when the part is absent.
fn read_part(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<Option<String>, ExtractError> {
    match archive.by_name(name) {
        Ok(mut f) => {
            let mut s = String::new();
            f.read_to_string(&mut s)
                .map_err(|e| ExtractError::InvalidWorkbook(format!("part {name}: {e}")))?;
            Ok(Some(s))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(ExtractError::InvalidWorkbook(format!("part {name}: {e}"))),
    }
}

/// Parse `xl/sharedStrings.xml`: one string per `<si>`, concatenating
/// every `<t>` descendant (rich-text runs share one `<si>`).
fn parse_shared_strings(xml: &str) -> Result<Vec<String>, ExtractError> {
    let events = tokenize(xml).map_err(ExtractError::InvalidWorkbook)?;
    let mut out = Vec::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut cur = String::new();
    for ev in events {
        match ev {
            Event::Start { name, self_closing, .. } => match local_name(&name) {
                "si" => {
                    in_si = true;
                    cur = String::new();
                    if self_closing {
                        out.push(String::new());
                        in_si = false;
                    }
                }
                "t" if in_si => in_t = true,
                _ => {}
            },
            Event::End { name } => match local_name(&name) {
                "si" if in_si => {
                    out.push(std::mem::take(&mut cur));
                    in_si = false;
                }
                "t" => in_t = false,
                _ => {}
            },
            Event::Text(t) => {
                if in_si && in_t {
                    cur.push_str(&t);
                }
            }
        }
    }
    Ok(out)
}

/// Number formats resolved from `xl/styles.xml`: the built-in table
/// plus the workbook's custom formats, indexed by cellXfs position.
#[derive(Default)]
struct NumberFormats {
    /// format code per cellXfs entry (the cell's `s` attribute).
    by_style: Vec<Option<String>>,
}

/// The OOXML built-in number formats this reader names. Only the
/// currency/accounting family is load-bearing (the RA extractor reads
/// a currency unit from the format code); the rest map to `None`.
fn builtin_format(id: u32) -> Option<&'static str> {
    match id {
        3 => Some("#,##0"),
        4 => Some("#,##0.00"),
        9 => Some("0%"),
        10 => Some("0.00%"),
        14 => Some("mm-dd-yy"),
        44 => Some(r#"_("$"* #,##0.00_);_("$"* \(#,##0.00\);_("$"* "-"??_);_(@_)"#),
        _ => None,
    }
}

fn parse_styles(xml: &str) -> Result<NumberFormats, ExtractError> {
    let events = tokenize(xml).map_err(ExtractError::InvalidWorkbook)?;
    let mut custom: BTreeMap<u32, String> = BTreeMap::new();
    let mut by_style: Vec<Option<String>> = Vec::new();
    let mut section = "";
    for ev in events {
        match ev {
            Event::Start { name, attrs, self_closing } => {
                let ln = local_name(&name).to_string();
                match ln.as_str() {
                    "numFmts" => section = "numFmts",
                    "cellXfs" => section = "cellXfs",
                    "numFmt" if section == "numFmts" => {
                        if let (Some(id), Some(code)) =
                            (attr(&attrs, "numFmtId"), attr(&attrs, "formatCode"))
                        {
                            if let Ok(id) = id.parse::<u32>() {
                                custom.insert(id, code.to_string());
                            }
                        }
                    }
                    "xf" if section == "cellXfs" => {
                        let code = attr(&attrs, "numFmtId")
                            .and_then(|s| s.parse::<u32>().ok())
                            .and_then(|id| {
                                custom
                                    .get(&id)
                                    .map(|c| Some(c.clone()))
                                    .unwrap_or_else(|| builtin_format(id).map(str::to_string))
                            });
                        by_style.push(code);
                        if self_closing {
                            // self-closed xf still counts as one entry.
                        }
                    }
                    _ => {}
                }
            }
            Event::End { name } => match local_name(&name) {
                "numFmts" | "cellXfs" => section = "",
                _ => {}
            },
            Event::Text(_) => {}
        }
    }
    Ok(NumberFormats { by_style })
}

/// Parse one worksheet part into its non-empty cells.
fn parse_sheet(
    name: String,
    xml: &str,
    shared_strings: &[String],
    formats: &NumberFormats,
) -> Result<Sheet, ExtractError> {
    let events = tokenize(xml).map_err(ExtractError::InvalidWorkbook)?;
    let mut cells: Vec<Cell> = Vec::new();
    // The cell under construction: (reference, type attr, style idx).
    let mut cur: Option<(String, String, Option<usize>)> = None;
    let mut formula: Option<String> = None;
    let mut value_text: Option<String> = None;
    let mut inline_text: Option<String> = None;
    let mut in_f = false;
    let mut in_v = false;
    let mut in_is_t = false;

    let finish = |cur: &Option<(String, String, Option<usize>)>,
                  formula: &Option<String>,
                  value_text: &Option<String>,
                  inline_text: &Option<String>,
                  cells: &mut Vec<Cell>|
     -> Result<(), ExtractError> {
        let Some((reference, ty, style)) = cur else { return Ok(()) };
        let (row, column) = parse_ref(reference)
            .ok_or_else(|| ExtractError::InvalidWorkbook(format!("bad cell reference {reference}")))?;
        let number_format = style
            .and_then(|i| formats.by_style.get(i).cloned())
            .unwrap_or(None);
        let mut cell = Cell {
            reference: reference.clone(),
            row,
            column,
            formula: formula.clone(),
            number: None,
            text: None,
            cached: None,
            number_format,
        };
        match (formula.is_some(), ty.as_str()) {
            (true, _) => {
                cell.cached = value_text.as_deref().and_then(|v| v.trim().parse::<f64>().ok());
            }
            (false, "s") => {
                let idx: usize = value_text
                    .as_deref()
                    .and_then(|v| v.trim().parse().ok())
                    .ok_or_else(|| {
                        ExtractError::InvalidWorkbook(format!(
                            "cell {reference}: shared-string index missing"
                        ))
                    })?;
                cell.text = Some(shared_strings.get(idx).cloned().ok_or_else(|| {
                    ExtractError::InvalidWorkbook(format!(
                        "cell {reference}: shared-string index {idx} out of range"
                    ))
                })?);
            }
            (false, "inlineStr") => {
                cell.text = Some(inline_text.clone().unwrap_or_default());
            }
            (false, "str") => {
                cell.text = value_text.clone();
            }
            (false, _) => {
                cell.number = value_text.as_deref().and_then(|v| v.trim().parse::<f64>().ok());
            }
        }
        // A cell with no formula, no number and no text carries
        // nothing the graph can use (style-only cell): skip it.
        if cell.formula.is_none() && cell.number.is_none() && cell.text.is_none() {
            return Ok(());
        }
        cells.push(cell);
        Ok(())
    };

    for ev in events {
        match ev {
            Event::Start { name: ename, attrs, self_closing } => match local_name(&ename) {
                "c" => {
                    finish(&cur, &formula, &value_text, &inline_text, &mut cells)?;
                    let reference = attr(&attrs, "r").unwrap_or("").to_string();
                    let ty = attr(&attrs, "t").unwrap_or("n").to_string();
                    let style = attr(&attrs, "s").and_then(|s| s.parse::<usize>().ok());
                    cur = Some((reference, ty, style));
                    formula = None;
                    value_text = None;
                    inline_text = None;
                    if self_closing {
                        finish(&cur, &formula, &value_text, &inline_text, &mut cells)?;
                        cur = None;
                    }
                }
                "f" => in_f = true,
                "v" => in_v = true,
                "t" => in_is_t = true,
                _ => {}
            },
            Event::End { name: ename } => match local_name(&ename) {
                "c" => {
                    finish(&cur, &formula, &value_text, &inline_text, &mut cells)?;
                    cur = None;
                }
                "f" => in_f = false,
                "v" => in_v = false,
                "t" => in_is_t = false,
                _ => {}
            },
            Event::Text(t) => {
                if in_f {
                    formula = Some(match formula {
                        Some(mut f) => {
                            f.push_str(&t);
                            f
                        }
                        None => t,
                    });
                } else if in_v {
                    value_text = Some(t);
                } else if in_is_t {
                    let cur_text = inline_text.get_or_insert_with(String::new);
                    cur_text.push_str(&t);
                }
            }
        }
    }
    Ok(Sheet { name, cells })
}

/// Parse an A1-style reference into (row, column), both 1-based.
/// `$` anchors are accepted and dropped.
pub fn parse_ref(reference: &str) -> Option<(u32, u32)> {
    let mut col: u32 = 0;
    let mut row: u32 = 0;
    let mut seen_col = false;
    let mut seen_row = false;
    for ch in reference.chars() {
        match ch {
            '$' => {}
            'A'..='Z' | 'a'..='z' => {
                if seen_row {
                    return None;
                }
                seen_col = true;
                col = col.checked_mul(26)?.checked_add((ch.to_ascii_uppercase() as u32) - ('A' as u32) + 1)?;
            }
            '0'..='9' => {
                seen_row = true;
                row = row.checked_mul(10)?.checked_add(ch as u32 - '0' as u32)?;
            }
            _ => return None,
        }
    }
    if seen_col && seen_row && row > 0 {
        Some((row, col))
    } else {
        None
    }
}

/// Render (row, column) back to an A1-style reference (no `$`).
pub fn render_ref(row: u32, column: u32) -> String {
    let mut letters = String::new();
    let mut c = column;
    while c > 0 {
        let rem = (c - 1) % 26;
        letters.insert(0, (b'A' + rem as u8) as char);
        c = (c - 1) / 26;
    }
    format!("{letters}{row}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_round_trip() {
        for (r, c, s) in [(1, 1, "A1"), (12, 2, "B12"), (3, 27, "AA3"), (100, 52, "AZ100")] {
            assert_eq!(parse_ref(s), Some((r, c)));
            assert_eq!(render_ref(r, c), s);
        }
        assert_eq!(parse_ref("$B$12"), Some((12, 2)));
        assert_eq!(parse_ref("12B"), None);
        assert_eq!(parse_ref(""), None);
    }
}
