//! Plain-text table rendering: right-aligned figures, thousands
//! separators, fixed decimals, and grapheme/east-asian-width-aware column
//! alignment so Arabic (and any other non-Latin) text never jitters a
//! column out of line.

use rust_decimal::Decimal;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use jama_core::model::{Amount, Flag};

use crate::color::{Palette, Tone};

/// A single coloured character for a transaction's clearing state: teal
/// `*` for cleared, amber `!` for pending.
pub fn flag_badge(flag: Flag, palette: &Palette) -> String {
    match flag {
        Flag::Cleared => palette.paint(Tone::Teal, "*"),
        Flag::Pending => palette.paint(Tone::Amber, "!"),
    }
}

/// Visible column width of `s`, counting grapheme clusters at their east-
/// asian-aware display width rather than raw byte or `char` length. This
/// is what keeps a table's columns aligned when a payee or account name is
/// written in Arabic (or CJK, or anything with combining marks).
pub fn display_width(s: &str) -> usize {
    s.graphemes(true).map(UnicodeWidthStr::width).sum()
}

pub fn pad_right(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

pub fn pad_left(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - w), s)
    }
}

/// Format a decimal at a fixed precision with thousands separators and a
/// real minus sign (U+2212) for negatives — never a bare ASCII hyphen.
pub fn format_number(number: Decimal, precision: u8) -> String {
    let negative = number.is_sign_negative() && !number.is_zero();
    let abs = number.abs();
    let formatted = format!("{:.*}", precision as usize, abs);
    let (int_part, frac_part) = match formatted.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (formatted.as_str(), None),
    };
    let grouped = group_thousands(int_part);
    let mut out = String::new();
    if negative {
        out.push('\u{2212}');
    }
    out.push_str(&grouped);
    if let Some(f) = frac_part {
        out.push('.');
        out.push_str(f);
    }
    out
}

fn group_thousands(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        let remaining = bytes.len() - i;
        if i > 0 && remaining.is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Render `amount` at its commodity's precision, coloured signal-red when
/// negative.
pub fn format_amount(amount: &Amount, precision: u8, palette: &Palette) -> String {
    let text = format!(
        "{} {}",
        format_number(amount.number, precision),
        amount.commodity
    );
    if amount.number.is_sign_negative() && !amount.number.is_zero() {
        palette.paint(Tone::Signal, &text)
    } else {
        text
    }
}

pub enum Align {
    Left,
    Right,
}

/// A minimal table: headers + rows, one alignment per column. Renders as
/// space-separated, width-aligned plain text with a single rule under the
/// header.
pub struct Table {
    pub headers: Vec<String>,
    pub aligns: Vec<Align>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: Vec<&str>, aligns: Vec<Align>) -> Self {
        Self {
            headers: headers.into_iter().map(str::to_string).collect(),
            aligns,
            rows: Vec::new(),
        }
    }

    pub fn push_row(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }

    pub fn render(&self) -> String {
        let cols = self.headers.len();
        let mut widths: Vec<usize> = self.headers.iter().map(|h| display_width(h)).collect();
        for row in &self.rows {
            for (width, cell) in widths.iter_mut().zip(row.iter()) {
                *width = (*width).max(display_width(strip_ansi(cell).as_str()));
            }
        }

        let mut out = String::new();
        let header_line: Vec<String> = (0..cols)
            .map(|i| self.align_cell(&self.headers[i], widths[i], &self.aligns[i]))
            .collect();
        out.push_str(header_line.join("  ").trim_end());
        out.push('\n');

        let total_width: usize = widths.iter().sum::<usize>() + 2 * cols.saturating_sub(1);
        out.push_str(
            &crate::color::rule_char()
                .to_string()
                .repeat(total_width.max(1)),
        );
        out.push('\n');

        for row in &self.rows {
            let cells: Vec<String> = (0..cols)
                .map(|i| {
                    let empty = String::new();
                    let cell = row.get(i).unwrap_or(&empty);
                    self.align_cell(cell, widths[i], &self.aligns[i])
                })
                .collect();
            out.push_str(cells.join("  ").trim_end());
            out.push('\n');
        }
        out
    }

    fn align_cell(&self, text: &str, width: usize, align: &Align) -> String {
        if !text.contains('\x1b') {
            // Fast, common path: no embedded colour codes to account for.
            return match align {
                Align::Left => pad_right(text, width),
                Align::Right => pad_left(text, width),
            };
        }
        let visible = display_width(strip_ansi(text).as_str());
        let pad = width.saturating_sub(visible);
        match align {
            Align::Left => format!("{}{}", text, " ".repeat(pad)),
            Align::Right => format!("{}{}", " ".repeat(pad), text),
        }
    }
}

/// Strip ANSI SGR escapes so width calculations aren't thrown off by
/// colour codes embedded in a cell.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c2 in chars.by_ref() {
                if c2 == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
