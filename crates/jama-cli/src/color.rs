//! The JAMA palette and how it degrades: truecolor when the terminal
//! supports it, 16-colour ANSI otherwise, and no colour at all when
//! `NO_COLOR` is set, `--no-color` is passed, or stdout isn't a TTY.

use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// #B5300F — negatives, errors.
    Signal,
    /// #F2A93B — warnings, pending `!`.
    Amber,
    /// #1E6F6B — balanced, cleared, positive.
    Teal,
    /// Ink-on-stock brand chip, used only for the splash mark.
    Brand,
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    enabled: bool,
    truecolor: bool,
}

impl Palette {
    /// Decide how much colour to use, honouring (in priority order)
    /// `--no-color`, `NO_COLOR`, and whether stdout is actually a TTY.
    pub fn detect(no_color_flag: bool) -> Self {
        let no_color_env = std::env::var_os("NO_COLOR").is_some();
        let is_tty = std::io::stdout().is_terminal();
        let enabled = !no_color_flag && !no_color_env && is_tty;
        let truecolor = enabled
            && std::env::var("COLORTERM")
                .map(|v| v.contains("truecolor") || v.contains("24bit"))
                .unwrap_or(false);
        Self { enabled, truecolor }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn paint(&self, tone: Tone, text: &str) -> String {
        if !self.enabled {
            return text.to_string();
        }
        match tone {
            Tone::Brand => {
                if self.truecolor {
                    format!("\x1b[38;2;42;26;18m\x1b[48;2;255;241;220m{text}\x1b[0m")
                } else {
                    // 16-colour terminals don't have the brand hues; fall
                    // back to a plain reverse-video chip.
                    format!("\x1b[7m{text}\x1b[0m")
                }
            }
            Tone::Signal => self.fg(text, (181, 48, 15), 31),
            Tone::Amber => self.fg(text, (242, 169, 59), 33),
            Tone::Teal => self.fg(text, (30, 111, 107), 36),
        }
    }

    fn fg(&self, text: &str, rgb: (u8, u8, u8), ansi16: u8) -> String {
        if self.truecolor {
            format!("\x1b[38;2;{};{};{}m{text}\x1b[0m", rgb.0, rgb.1, rgb.2)
        } else {
            format!("\x1b[{ansi16}m{text}\x1b[0m")
        }
    }
}

/// Whether we should use Unicode box-drawing characters: only when stdout
/// is a UTF-8 TTY. Non-TTY output (pipes, redirects) gets the ASCII
/// fallback so it stays predictable in scripts and logs.
pub fn use_unicode_lines() -> bool {
    std::io::stdout().is_terminal()
}

pub fn rule_char() -> char {
    if use_unicode_lines() {
        '─'
    } else {
        '-'
    }
}

/// The single-letter block mark shown only on `jama --version` and the
/// `init` welcome — nowhere else.
pub const BRAND_MARK: &str = "ج";
