// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The bounded spreadsheet-formula evaluator.
//!
//! The RA profile's value rule: a cell's value is RECOMPUTED from
//! the artefact's own formulas — never the workbook's cached display
//! value. This evaluator is that recomputation. It is deliberately
//! bounded: the grammar is numbers, cell references (same-sheet and
//! sheet-qualified, `$` anchors accepted), ranges (only as function
//! arguments), `+ - * / % ^`, unary minus, parentheses, string
//! literals (only as `IF` results / comparisons are NOT supported),
//! and a named function set: SUM, AVERAGE, MIN, MAX, ABS, ROUND,
//! SQRT, IF (lazy), NPV (Excel semantics: the first value is
//! discounted one period).
//!
//! Every failure is a typed absence, never a guessed number:
//! - [`EvalError::UnknownFunction`] — a function outside the set:
//!   the cell is `not_measured` (RA Regression Contract C-RA7 case 1).
//! - [`EvalError::UnresolvedRef`] — a reference to a cell that does
//!   not exist in the workbook: the formula_resolution finding.
//! - [`EvalError::Cycle`] — the cell's value depends on itself.
//! - [`EvalError::AbsentInput`] — a read cell exists but carries no
//!   value of its own (its own formula failed); absence propagates.
//! - [`EvalError::DivByZero`] / [`EvalError::Parse`] /
//!   [`EvalError::Type`] — arithmetic that has no value.
//!
//! Determinism: evaluation is memoized depth-first over the
//! reference graph; iteration order never affects results (each
//! cell's value is a pure function of the workbook and the scenario
//! overrides).

use std::collections::BTreeMap;

use crate::xlsx::{parse_ref, Sheet};

/// Why a formula has no value.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    /// The formula text does not parse under the bounded grammar.
    Parse(String),
    /// A function call outside the evaluator's set.
    UnknownFunction(String),
    /// A reference to a cell absent from the workbook (rendered as
    /// written in the formula, e.g. `val!Z99` → `Z99` of sheet `val`).
    UnresolvedRef { sheet: String, reference: String },
    /// A reference to a cell that exists but has no value.
    AbsentInput { sheet: String, reference: String },
    /// The cell participates in (or reads into) a reference cycle.
    Cycle,
    /// Division (or modulo) by zero.
    DivByZero,
    /// An operand of the wrong type (e.g. arithmetic over text).
    Type(String),
}

/// A workbook prepared for evaluation: literal values, formulas, and
/// scenario overrides, all keyed by (sheet, reference).
pub struct EvalBook {
    /// Hard-coded numeric cells: (sheet, ref) → value.
    literals: BTreeMap<(String, String), f64>,
    /// Text cells: present (so references to them resolve) but with
    /// no numeric value.
    texts: std::collections::BTreeSet<(String, String)>,
    /// Formula cells: (sheet, ref) → formula text (no leading `=`).
    formulas: BTreeMap<(String, String), String>,
    /// Scenario overrides: (sheet, ref) → value replacing whatever
    /// the workbook holds for that cell.
    overrides: BTreeMap<(String, String), f64>,
}

impl EvalBook {
    /// Build from parsed sheets. Formula cells' cached values are
    /// ignored by construction — they never enter this structure.
    pub fn from_sheets(sheets: &[Sheet]) -> EvalBook {
        let mut book = EvalBook {
            literals: BTreeMap::new(),
            texts: std::collections::BTreeSet::new(),
            formulas: BTreeMap::new(),
            overrides: BTreeMap::new(),
        };
        for sheet in sheets {
            for cell in &sheet.cells {
                let key = (sheet.name.clone(), cell.reference.clone());
                if let Some(f) = &cell.formula {
                    book.formulas.insert(key, f.clone());
                } else if let Some(n) = cell.number {
                    book.literals.insert(key, n);
                } else if cell.text.is_some() {
                    book.texts.insert(key);
                }
            }
        }
        book
    }

    /// Apply scenario overrides (driver/parameter re-binding). An
    /// override replaces the cell's workbook value or formula result
    /// for this evaluation only; the workbook is not mutated.
    pub fn with_overrides(mut self, overrides: BTreeMap<(String, String), f64>) -> EvalBook {
        self.overrides = overrides;
        self
    }

    /// Does this cell exist in the workbook at all?
    pub fn cell_exists(&self, sheet: &str, reference: &str) -> bool {
        let key = (sheet.to_string(), reference.to_string());
        self.literals.contains_key(&key)
            || self.formulas.contains_key(&key)
            || self.texts.contains(&key)
            || self.overrides.contains_key(&key)
    }

    /// Is this cell a formula cell in the workbook?
    pub fn is_formula(&self, sheet: &str, reference: &str) -> bool {
        self.formulas
            .contains_key(&(sheet.to_string(), reference.to_string()))
    }

    /// Every formula cell, sorted: (sheet, ref, formula text).
    pub fn formula_cells(&self) -> Vec<(String, String, String)> {
        self.formulas
            .iter()
            .map(|((s, r), f)| (s.clone(), r.clone(), f.clone()))
            .collect()
    }

    /// Evaluate one cell. Overrides win; then literals; then the
    /// formula, recursively. Text cells have no value.
    pub fn evaluate(&self, sheet: &str, reference: &str) -> Result<f64, EvalError> {
        let mut session = Session {
            book: self,
            memo: BTreeMap::new(),
            visiting: Vec::new(),
        };
        session.eval_cell(sheet, reference)
    }

    /// Evaluate every formula cell, returning per-cell results in
    /// sorted (sheet, ref) order — the extraction pass uses this so
    /// diagnostics arrive in a deterministic order.
    pub fn evaluate_all(&self) -> Vec<((String, String), Result<f64, EvalError>)> {
        let mut session = Session {
            book: self,
            memo: BTreeMap::new(),
            visiting: Vec::new(),
        };
        let mut out = Vec::new();
        for (sheet, reference, _) in self.formula_cells() {
            let r = session.eval_cell(&sheet, &reference);
            out.push(((sheet, reference), r));
        }
        out
    }
}

/// One evaluation session: memo + cycle-detection stack.
struct Session<'a> {
    book: &'a EvalBook,
    memo: BTreeMap<(String, String), Result<f64, EvalError>>,
    visiting: Vec<(String, String)>,
}

impl Session<'_> {
    fn eval_cell(&mut self, sheet: &str, reference: &str) -> Result<f64, EvalError> {
        let key = (sheet.to_string(), reference.to_string());
        if let Some(r) = self.memo.get(&key) {
            return r.clone();
        }
        if let Some(v) = self.book.overrides.get(&key) {
            let r = Ok(*v);
            self.memo.insert(key, r.clone());
            return r;
        }
        if let Some(v) = self.book.literals.get(&key) {
            let r = Ok(*v);
            self.memo.insert(key, r.clone());
            return r;
        }
        if self.book.texts.contains(&key) {
            let r: Result<f64, EvalError> = Err(EvalError::AbsentInput {
                sheet: sheet.to_string(),
                reference: reference.to_string(),
            });
            self.memo.insert(key, r.clone());
            return r;
        }
        let Some(formula) = self.book.formulas.get(&key).cloned() else {
            return Err(EvalError::UnresolvedRef {
                sheet: sheet.to_string(),
                reference: reference.to_string(),
            });
        };
        if self.visiting.contains(&key) {
            return Err(EvalError::Cycle);
        }
        self.visiting.push(key.clone());
        let parsed = parse_formula(&formula);
        let result = match parsed {
            Ok(ast) => self.eval_expr(&ast, sheet),
            Err(e) => Err(e),
        };
        self.visiting.pop();
        // A Cycle result is context-dependent (a cell may be cyclic
        // only through the current path); do not memoize it as the
        // cell's own value — but every member of the cycle detects
        // the cycle itself when evaluated directly, so absence is
        // still exact for each member.
        if !matches!(result, Err(EvalError::Cycle)) {
            self.memo.insert(key, result.clone());
        }
        result
    }

    fn eval_expr(&mut self, expr: &Expr, sheet: &str) -> Result<f64, EvalError> {
        match expr {
            Expr::Number(n) => Ok(*n),
            Expr::Ref { sheet: rsheet, reference } => {
                let target_sheet = rsheet.as_deref().unwrap_or(sheet);
                match self.eval_cell(target_sheet, reference) {
                    Ok(v) => Ok(v),
                    // A reference that resolves to nothing stays an
                    // unresolved reference (the formula_resolution
                    // finding); a cycle stays a cycle; anything else
                    // means the target exists but is itself valueless,
                    // which this cell reports as an absent input.
                    Err(e @ EvalError::UnresolvedRef { .. }) => Err(e),
                    Err(EvalError::Cycle) => Err(EvalError::Cycle),
                    Err(_) => Err(EvalError::AbsentInput {
                        sheet: target_sheet.to_string(),
                        reference: reference.clone(),
                    }),
                }
            }
            Expr::UnaryMinus(inner) => Ok(-self.eval_expr(inner, sheet)?),
            Expr::Binary { op, lhs, rhs } => {
                let l = self.eval_expr(lhs, sheet)?;
                let r = self.eval_expr(rhs, sheet)?;
                match op {
                    BinOp::Add => Ok(l + r),
                    BinOp::Sub => Ok(l - r),
                    BinOp::Mul => Ok(l * r),
                    BinOp::Div => {
                        if r == 0.0 {
                            Err(EvalError::DivByZero)
                        } else {
                            Ok(l / r)
                        }
                    }
                    BinOp::Mod => {
                        if r == 0.0 {
                            Err(EvalError::DivByZero)
                        } else {
                            Ok(l % r)
                        }
                    }
                    BinOp::Pow => Ok(l.powf(r)),
                }
            }
            Expr::Call { name, args } => self.eval_call(name, args, sheet),
            Expr::Compare { .. } => Err(EvalError::Type(
                "a comparison has no numeric value outside an IF condition".to_string(),
            )),
            Expr::Range { .. } => Err(EvalError::Type(
                "a range is only a function argument".to_string(),
            )),
        }
    }

    /// Flatten a call argument list: ranges expand over their cells
    /// (row-major), single refs and expressions evaluate in place.
    fn eval_args_flat(
        &mut self,
        args: &[Expr],
        sheet: &str,
    ) -> Result<Vec<f64>, EvalError> {
        let mut out = Vec::new();
        for arg in args {
            match arg {
                Expr::Range { sheet: rsheet, from, to } => {
                    let target = rsheet.as_deref().unwrap_or(sheet);
                    let (r1, c1) = parse_ref(from)
                        .ok_or_else(|| EvalError::Parse(format!("bad range start {from}")))?;
                    let (r2, c2) = parse_ref(to)
                        .ok_or_else(|| EvalError::Parse(format!("bad range end {to}")))?;
                    for row in r1.min(r2)..=r1.max(r2) {
                        for col in c1.min(c2)..=c1.max(c2) {
                            let reference = crate::xlsx::render_ref(row, col);
                            // Empty cells inside a range contribute
                            // nothing (spreadsheet semantics); a cell
                            // that exists but has no value propagates
                            // its absence.
                            if !self.book.cell_exists(target, &reference) {
                                continue;
                            }
                            out.push(self.eval_cell(target, &reference)?);
                        }
                    }
                }
                _ => out.push(self.eval_expr(arg, sheet)?),
            }
        }
        Ok(out)
    }

    fn eval_call(&mut self, name: &str, args: &[Expr], sheet: &str) -> Result<f64, EvalError> {
        match name.to_ascii_uppercase().as_str() {
            "IF" => {
                if args.len() < 2 || args.len() > 3 {
                    return Err(EvalError::Parse("IF takes 2 or 3 arguments".into()));
                }
                let cond = self.eval_condition(&args[0], sheet)?;
                if cond {
                    self.eval_expr(&args[1], sheet)
                } else if args.len() == 3 {
                    self.eval_expr(&args[2], sheet)
                } else {
                    Ok(0.0)
                }
            }
            "SUM" => Ok(self.eval_args_flat(args, sheet)?.iter().sum()),
            "AVERAGE" => {
                let vals = self.eval_args_flat(args, sheet)?;
                if vals.is_empty() {
                    Err(EvalError::DivByZero)
                } else {
                    Ok(vals.iter().sum::<f64>() / vals.len() as f64)
                }
            }
            "MIN" => self
                .eval_args_flat(args, sheet)?
                .into_iter()
                .reduce(f64::min)
                .ok_or_else(|| EvalError::Parse("MIN of no values".into())),
            "MAX" => self
                .eval_args_flat(args, sheet)?
                .into_iter()
                .reduce(f64::max)
                .ok_or_else(|| EvalError::Parse("MAX of no values".into())),
            "ABS" => {
                let vals = self.eval_args_flat(args, sheet)?;
                if vals.len() != 1 {
                    return Err(EvalError::Parse("ABS takes one argument".into()));
                }
                Ok(vals[0].abs())
            }
            "SQRT" => {
                let vals = self.eval_args_flat(args, sheet)?;
                if vals.len() != 1 {
                    return Err(EvalError::Parse("SQRT takes one argument".into()));
                }
                if vals[0] < 0.0 {
                    return Err(EvalError::Type("SQRT of a negative number".into()));
                }
                Ok(vals[0].sqrt())
            }
            "ROUND" => {
                let vals = self.eval_args_flat(args, sheet)?;
                if vals.len() != 2 {
                    return Err(EvalError::Parse("ROUND takes two arguments".into()));
                }
                let digits = vals[1] as i32;
                let factor = 10f64.powi(digits);
                Ok((vals[0] * factor).round() / factor)
            }
            "NPV" => {
                // Excel semantics: NPV(rate, v1, v2, ...) discounts
                // v1 by one period, v2 by two, and so on.
                if args.len() < 2 {
                    return Err(EvalError::Parse("NPV takes a rate and values".into()));
                }
                let rate = self.eval_expr(&args[0], sheet)?;
                let vals = self.eval_args_flat(&args[1..], sheet)?;
                let mut acc = 0.0;
                for (i, v) in vals.iter().enumerate() {
                    acc += v / (1.0 + rate).powi(i as i32 + 1);
                }
                Ok(acc)
            }
            other => Err(EvalError::UnknownFunction(other.to_string())),
        }
    }

    /// An IF condition: a comparison (`a>b`, `a>=b`, `a=b`, `a<>b`,
    /// `a<b`, `a<=b`) or a bare numeric expression (nonzero = true).
    fn eval_condition(&mut self, expr: &Expr, sheet: &str) -> Result<bool, EvalError> {
        if let Expr::Compare { op, lhs, rhs } = expr {
            let l = self.eval_expr(lhs, sheet)?;
            let r = self.eval_expr(rhs, sheet)?;
            return Ok(match op {
                CmpOp::Gt => l > r,
                CmpOp::Ge => l >= r,
                CmpOp::Lt => l < r,
                CmpOp::Le => l <= r,
                CmpOp::Eq => l == r,
                CmpOp::Ne => l != r,
            });
        }
        Ok(self.eval_expr(expr, sheet)? != 0.0)
    }
}

// ---------------------------------------------------------------------------
// The parser: formula text → AST.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Number(f64),
    Ref { sheet: Option<String>, reference: String },
    Range { sheet: Option<String>, from: String, to: String },
    UnaryMinus(Box<Expr>),
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Compare { op: CmpOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Call { name: String, args: Vec<Expr> },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CmpOp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    /// A cell reference or sheet-qualified reference, already split:
    /// (sheet, ref). A bare identifier that is not a reference and
    /// not a function name surfaces as `Ident`.
    Ref(Option<String>, String),
    Ident(String),
    Str(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Comma,
    Colon,
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

fn is_cell_ref(s: &str) -> bool {
    parse_ref(s).is_some()
}

fn lex(formula: &str) -> Result<Vec<Tok>, EvalError> {
    let chars: Vec<char> = formula.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                let mut seen_dot = false;
                while i < chars.len()
                    && (chars[i].is_ascii_digit() || (chars[i] == '.' && !seen_dot))
                {
                    if chars[i] == '.' {
                        seen_dot = true;
                    }
                    i += 1;
                }
                // Exponent part.
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        i = j;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let text: String = chars[start..i].iter().collect();
                let n: f64 = text
                    .parse()
                    .map_err(|_| EvalError::Parse(format!("bad number {text}")))?;
                toks.push(Tok::Num(n));
            }
            '\'' => {
                // Quoted sheet name: 'My Sheet'!A1
                let start = i + 1;
                let mut j = start;
                let mut name = String::new();
                loop {
                    if j >= chars.len() {
                        return Err(EvalError::Parse("unterminated quoted sheet name".into()));
                    }
                    if chars[j] == '\'' {
                        if j + 1 < chars.len() && chars[j + 1] == '\'' {
                            name.push('\'');
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    name.push(chars[j]);
                    j += 1;
                }
                i = j + 1;
                if i >= chars.len() || chars[i] != '!' {
                    return Err(EvalError::Parse("quoted sheet name not followed by !".into()));
                }
                i += 1;
                let (reference, ni) = lex_cell_ref(&chars, i)?;
                i = ni;
                toks.push(Tok::Ref(Some(name), reference));
            }
            '"' => {
                let mut j = i + 1;
                let mut s = String::new();
                loop {
                    if j >= chars.len() {
                        return Err(EvalError::Parse("unterminated string literal".into()));
                    }
                    if chars[j] == '"' {
                        if j + 1 < chars.len() && chars[j + 1] == '"' {
                            s.push('"');
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    s.push(chars[j]);
                    j += 1;
                }
                toks.push(Tok::Str(s));
                i = j + 1;
            }
            'A'..='Z' | 'a'..='z' | '_' | '$' => {
                // Identifier, sheet-qualified ref, or plain cell ref.
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric()
                        || chars[i] == '_'
                        || chars[i] == '$'
                        || chars[i] == '.')
                {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                if i < chars.len() && chars[i] == '!' {
                    // Sheet-qualified reference: word is the sheet name.
                    i += 1;
                    let (reference, ni) = lex_cell_ref(&chars, i)?;
                    i = ni;
                    toks.push(Tok::Ref(Some(word), reference));
                } else {
                    let cleaned: String = word.chars().filter(|c| *c != '$').collect();
                    if is_cell_ref(&cleaned) {
                        toks.push(Tok::Ref(None, cleaned.to_ascii_uppercase()));
                    } else {
                        toks.push(Tok::Ident(word));
                    }
                }
            }
            '+' => {
                toks.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                toks.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                toks.push(Tok::Star);
                i += 1;
            }
            '/' => {
                toks.push(Tok::Slash);
                i += 1;
            }
            '%' => {
                toks.push(Tok::Percent);
                i += 1;
            }
            '^' => {
                toks.push(Tok::Caret);
                i += 1;
            }
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                toks.push(Tok::Comma);
                i += 1;
            }
            ':' => {
                toks.push(Tok::Colon);
                i += 1;
            }
            '>' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    toks.push(Tok::Ge);
                    i += 2;
                } else {
                    toks.push(Tok::Gt);
                    i += 1;
                }
            }
            '<' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    toks.push(Tok::Le);
                    i += 2;
                } else if i + 1 < chars.len() && chars[i + 1] == '>' {
                    toks.push(Tok::Ne);
                    i += 2;
                } else {
                    toks.push(Tok::Lt);
                    i += 1;
                }
            }
            '=' => {
                toks.push(Tok::Eq);
                i += 1;
            }
            other => {
                return Err(EvalError::Parse(format!("unexpected character {other:?}")));
            }
        }
    }
    Ok(toks)
}

/// Lex one cell reference at `i` (after a `!`): letters/`$` then digits.
fn lex_cell_ref(chars: &[char], mut i: usize) -> Result<(String, usize), EvalError> {
    let start = i;
    while i < chars.len()
        && (chars[i].is_ascii_alphabetic() || chars[i] == '$' || chars[i].is_ascii_digit())
    {
        i += 1;
    }
    let raw: String = chars[start..i].iter().collect();
    let cleaned: String = raw.chars().filter(|c| *c != '$').collect();
    if is_cell_ref(&cleaned) {
        Ok((cleaned.to_ascii_uppercase(), i))
    } else {
        Err(EvalError::Parse(format!("bad cell reference {raw}")))
    }
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expr(&mut self) -> Result<Expr, EvalError> {
        // Comparison level (lowest precedence, non-associative use).
        let lhs = self.additive()?;
        let op = match self.peek() {
            Some(Tok::Gt) => Some(CmpOp::Gt),
            Some(Tok::Ge) => Some(CmpOp::Ge),
            Some(Tok::Lt) => Some(CmpOp::Lt),
            Some(Tok::Le) => Some(CmpOp::Le),
            Some(Tok::Eq) => Some(CmpOp::Eq),
            Some(Tok::Ne) => Some(CmpOp::Ne),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            let rhs = self.additive()?;
            return Ok(Expr::Compare { op, lhs: Box::new(lhs), rhs: Box::new(rhs) });
        }
        Ok(lhs)
    }

    fn additive(&mut self) -> Result<Expr, EvalError> {
        let mut lhs = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Add,
                Some(Tok::Minus) => BinOp::Sub,
                _ => break,
            };
            self.next();
            let rhs = self.multiplicative()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }

    fn multiplicative(&mut self) -> Result<Expr, EvalError> {
        let mut lhs = self.power()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinOp::Mul,
                Some(Tok::Slash) => BinOp::Div,
                Some(Tok::Percent) => BinOp::Mod,
                _ => break,
            };
            self.next();
            let rhs = self.power()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }

    fn power(&mut self) -> Result<Expr, EvalError> {
        let base = self.unary()?;
        if matches!(self.peek(), Some(Tok::Caret)) {
            self.next();
            let exp = self.power()?;
            return Ok(Expr::Binary {
                op: BinOp::Pow,
                lhs: Box::new(base),
                rhs: Box::new(exp),
            });
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<Expr, EvalError> {
        if matches!(self.peek(), Some(Tok::Minus)) {
            self.next();
            return Ok(Expr::UnaryMinus(Box::new(self.unary()?)));
        }
        if matches!(self.peek(), Some(Tok::Plus)) {
            self.next();
            return self.unary();
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, EvalError> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Number(n)),
            Some(Tok::Ref(sheet, reference)) => {
                if matches!(self.peek(), Some(Tok::Colon)) {
                    self.next();
                    match self.next() {
                        Some(Tok::Ref(sheet2, to)) => {
                            let sheet = sheet.or(sheet2);
                            Ok(Expr::Range { sheet, from: reference, to })
                        }
                        _ => Err(EvalError::Parse("range end is not a reference".into())),
                    }
                } else {
                    Ok(Expr::Ref { sheet, reference })
                }
            }
            Some(Tok::Ident(name)) => {
                if matches!(self.peek(), Some(Tok::LParen)) {
                    self.next();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Tok::RParen)) {
                        loop {
                            args.push(self.expr()?);
                            if matches!(self.peek(), Some(Tok::Comma)) {
                                self.next();
                                continue;
                            }
                            break;
                        }
                    }
                    if !matches!(self.next(), Some(Tok::RParen)) {
                        return Err(EvalError::Parse("call not closed".into()));
                    }
                    Ok(Expr::Call { name, args })
                } else {
                    // A bare name that is neither a cell nor a call:
                    // a defined name — outside the bounded grammar.
                    Err(EvalError::Parse(format!("bare name {name} is not a cell reference")))
                }
            }
            Some(Tok::LParen) => {
                let inner = self.expr()?;
                if !matches!(self.next(), Some(Tok::RParen)) {
                    return Err(EvalError::Parse("parenthesis not closed".into()));
                }
                Ok(inner)
            }
            Some(Tok::Str(_)) => Err(EvalError::Type(
                "a string literal has no numeric value".into(),
            )),
            other => Err(EvalError::Parse(format!("unexpected token {other:?}"))),
        }
    }
}

/// Parse a formula (WITHOUT the leading `=`) into an AST.
fn parse_formula(formula: &str) -> Result<Expr, EvalError> {
    let toks = lex(formula)?;
    let mut parser = Parser { toks, pos: 0 };
    let expr = parser.expr()?;
    if parser.pos != parser.toks.len() {
        return Err(EvalError::Parse("trailing tokens after expression".into()));
    }
    Ok(expr)
}

/// The cell references a formula reads, in first-appearance order,
/// ranges expanded to their member cells: (sheet, reference) with
/// the sheet resolved against the formula's own sheet. This is the
/// computes_into edge source — derived from the formula TEXT, so a
/// reference is an edge even when its value cannot be evaluated.
pub fn formula_reads(
    formula: &str,
    own_sheet: &str,
) -> Result<Vec<(String, String)>, EvalError> {
    let ast = parse_formula(formula)?;
    let mut out: Vec<(String, String)> = Vec::new();
    collect_reads(&ast, own_sheet, &mut out);
    Ok(out)
}

fn collect_reads(expr: &Expr, own_sheet: &str, out: &mut Vec<(String, String)>) {
    match expr {
        Expr::Number(_) => {}
        Expr::Ref { sheet, reference } => {
            push_read(out, sheet.as_deref().unwrap_or(own_sheet), reference);
        }
        Expr::Range { sheet, from, to } => {
            let target = sheet.as_deref().unwrap_or(own_sheet);
            if let (Some((r1, c1)), Some((r2, c2))) = (parse_ref(from), parse_ref(to)) {
                for row in r1.min(r2)..=r1.max(r2) {
                    for col in c1.min(c2)..=c1.max(c2) {
                        push_read(out, target, &crate::xlsx::render_ref(row, col));
                    }
                }
            }
        }
        Expr::UnaryMinus(inner) => collect_reads(inner, own_sheet, out),
        Expr::Binary { lhs, rhs, .. } | Expr::Compare { lhs, rhs, .. } => {
            collect_reads(lhs, own_sheet, out);
            collect_reads(rhs, own_sheet, out);
        }
        Expr::Call { args, .. } => {
            for a in args {
                collect_reads(a, own_sheet, out);
            }
        }
    }
}

fn push_read(out: &mut Vec<(String, String)>, sheet: &str, reference: &str) {
    let item = (sheet.to_string(), reference.to_string());
    if !out.contains(&item) {
        out.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xlsx::{Cell, Sheet};

    fn book() -> EvalBook {
        let sheet = Sheet {
            name: "is".into(),
            cells: vec![
                Cell {
                    reference: "B2".into(),
                    row: 2,
                    column: 2,
                    formula: None,
                    number: Some(1000.0),
                    text: None,
                    cached: None,
                    number_format: None,
                },
                Cell {
                    reference: "C2".into(),
                    row: 2,
                    column: 3,
                    formula: Some("B2*(1+0.1)".into()),
                    number: None,
                    text: None,
                    cached: Some(999.0),
                    number_format: None,
                },
                Cell {
                    reference: "C3".into(),
                    row: 3,
                    column: 3,
                    formula: Some("SUM(C2:B2)".into()),
                    number: None,
                    text: None,
                    cached: None,
                    number_format: None,
                },
                Cell {
                    reference: "C4".into(),
                    row: 4,
                    column: 3,
                    formula: Some("XNPV(0.1,C2)".into()),
                    number: None,
                    text: None,
                    cached: None,
                    number_format: None,
                },
                Cell {
                    reference: "C5".into(),
                    row: 5,
                    column: 3,
                    formula: Some("C6".into()),
                    number: None,
                    text: None,
                    cached: None,
                    number_format: None,
                },
                Cell {
                    reference: "C6".into(),
                    row: 6,
                    column: 3,
                    formula: Some("C5".into()),
                    number: None,
                    text: None,
                    cached: None,
                    number_format: None,
                },
            ],
        };
        EvalBook::from_sheets(&[sheet])
    }

    #[test]
    fn recomputes_and_ignores_cache() {
        let b = book();
        assert_eq!(b.evaluate("is", "C2"), Ok(1100.0));
    }

    #[test]
    fn ranges_and_unknown_functions_and_cycles() {
        let b = book();
        // SUM(C2:B2) — reversed range still spans B2..C2.
        assert_eq!(b.evaluate("is", "C3"), Ok(2100.0));
        assert_eq!(
            b.evaluate("is", "C4"),
            Err(EvalError::UnknownFunction("XNPV".into()))
        );
        assert_eq!(b.evaluate("is", "C5"), Err(EvalError::Cycle));
        assert_eq!(b.evaluate("is", "C6"), Err(EvalError::Cycle));
        assert_eq!(
            b.evaluate("is", "Z9"),
            Err(EvalError::UnresolvedRef { sheet: "is".into(), reference: "Z9".into() })
        );
    }

    #[test]
    fn reads_come_from_the_text() {
        let reads = formula_reads("B2*(1+'val'!C5)+SUM(B2:B4)", "is").unwrap();
        assert_eq!(
            reads,
            vec![
                ("is".to_string(), "B2".to_string()),
                ("val".to_string(), "C5".to_string()),
                ("is".to_string(), "B3".to_string()),
                ("is".to_string(), "B4".to_string()),
            ]
        );
    }
}
