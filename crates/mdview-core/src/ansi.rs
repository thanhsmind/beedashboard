//! ANSI SGR (Select Graphic Rendition) escape sequences translated to safe,
//! escaped HTML — mdview-core's server-side stand-in for a client-side
//! terminal emulator (agent-terminal-12).
//!
//! NAMED DEVIATION from `approach.md`, which planned to vendor xterm.js as a
//! compiled-in asset the way `mermaid.min.js` is: the terminal screen this
//! feature renders is a polled *static snapshot* (`herdr`'s `pane.read`),
//! never a live PTY — no local echo, no cursor, no resize negotiation — so
//! xterm.js's entire reason for existing goes unused here, while ~300 KB of
//! vendored JavaScript would be untestable in the one place this feature
//! keeps all its proof. This module parses SGR colour/attribute sequences by
//! hand and renders them as `<span class="ansi-…">` markup instead.
//!
//! Framework-free (no axum/tokio/hyper), matching `bee.rs`'s module shape —
//! covered by the same `bee::tests::no_web_framework_dependency_declared`
//! guard, since both live in this crate's `Cargo.toml`.
//!
//! ## Security contract
//!
//! The raw screen text is HTML-escaped **before** any markup is wrapped
//! around it — never the reverse; getting this backwards turns an agent's
//! terminal output into stored HTML injection on the page an operator
//! reads. See `to_html_escapes_html_metacharacters_before_wrapping_markup`.
//!
//! Every escape sequence this parser does not model — cursor movement,
//! screen clears, OSC title-setting, DEC private modes, character-set
//! selection, anything else — is discarded **in full**: its ESC byte, every
//! parameter/intermediate byte, and its final byte. Nothing unrecognised is
//! ever emitted into the page, raw or otherwise.
//!
//! ## Colour model
//!
//! - The sixteen basic colours (SGR 30-37/90-97 foreground, 40-47/100-107
//!   background, 39/49 default) render as classes (`ansi-fg-red`,
//!   `ansi-bg-bright-blue`, …) whose colours are defined in `app.css` as
//!   aliases of the existing light/dark theme tokens, so a screen stays
//!   readable in both — never a hard-coded hex value.
//! - The 256-colour palette (`38;5;n` / `48;5;n`) renders as `ansi-fg-256-n`
//!   / `ansi-bg-256-n` classes carrying the literal xterm palette colour —
//!   these are colours the terminal app itself chose, not theme colours, so
//!   they are intentionally *not* theme-derived. An index below 16 is
//!   folded onto the theme-aware basic-16 classes instead (see
//!   [`Color::from_256`]), since indices 0-15 of the 256-colour palette
//!   *are* the sixteen basic colours by convention.
//! - 24-bit truecolour (`38;2;r;g;b` / `48;2;r;g;b`) is parsed just far
//!   enough to consume its parameters correctly (so the r/g/b numbers are
//!   never misread as unrelated SGR codes) but is not rendered as an exact
//!   colour — out of scope for this cell.
//!
//! ## Inverse (SGR 7)
//!
//! Rather than generate a combinatorial cross-product of CSS rules, inverse
//! video is resolved by swapping the *foreground/background colour values*
//! themselves before emitting classes (see [`Style::effective_fg_bg`]). The
//! `ansi-inverse` class itself carries the CSS fallback for whichever side
//! has no explicit colour (`app.css`); explicit `ansi-fg-*`/`ansi-bg-*`
//! classes declared after it in the stylesheet override per-property, by
//! plain source-order cascade — no per-colour inverse combinators needed.

use std::iter::Peekable;
use std::str::Chars;

/// Translate a raw ANSI-laden screen snapshot into safe HTML.
///
/// Text content is preserved byte-for-byte (including wide CJK and emoji) —
/// only escaped for HTML embedding, never otherwise altered. Every SGR run
/// becomes a `<span class="…">` wrapping its escaped text; a run with no
/// active style renders as plain escaped text, with no wrapper. Every other
/// escape sequence is dropped without emitting anything.
pub fn to_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut style = Style::default();
    let mut run = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if let Some(params) = consume_escape(&mut chars) {
                flush_run(&mut out, &run, &style);
                run.clear();
                apply_sgr(&mut style, &params);
            }
            continue;
        }
        run.push(c);
    }
    flush_run(&mut out, &run, &style);
    out
}

/// Escape a text run and wrap it in a `<span>` iff `style` carries any
/// active class — escaping always happens first, regardless of which
/// branch runs, so there is exactly one place text ever becomes HTML.
fn flush_run(out: &mut String, text: &str, style: &Style) {
    if text.is_empty() {
        return;
    }
    let escaped = escape_html(text);
    let classes = style.classes();
    if classes.is_empty() {
        out.push_str(&escaped);
    } else {
        out.push_str("<span class=\"");
        out.push_str(&classes.join(" "));
        out.push_str("\">");
        out.push_str(&escaped);
        out.push_str("</span>");
    }
}

/// HTML-escape text for safe embedding — the same four metacharacters
/// `crates/mdview/src/views.rs::esc` escapes, kept as an independent copy
/// since this crate never depends on the `mdview` binary crate.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// One basic ANSI colour (0-15: 0-7 normal, 8-15 bright) or a 256-palette
/// index (16-255). Indices 0-15 reaching here via the 256-colour mode
/// (`38;5;n` with `n < 16`) are folded onto `Basic` by [`Color::from_256`]
/// so they stay theme-aware rather than falling back to a literal palette
/// hex.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Color {
    Basic(u8),
    Indexed(u8),
}

impl Color {
    fn from_256(n: u8) -> Color {
        if n < 16 {
            Color::Basic(n)
        } else {
            Color::Indexed(n)
        }
    }

    fn class_suffix(self) -> String {
        match self {
            Color::Basic(n) => BASIC_NAMES[n as usize].to_string(),
            Color::Indexed(n) => format!("256-{n}"),
        }
    }
}

const BASIC_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright-black",
    "bright-red",
    "bright-green",
    "bright-yellow",
    "bright-blue",
    "bright-magenta",
    "bright-cyan",
    "bright-white",
];

/// The active SGR state while scanning a screen. `Default` is "no
/// attributes, terminal-default colours" — SGR 0 resets to exactly this.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Style {
    fg: Option<Color>,
    bg: Option<Color>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    inverse: bool,
}

impl Style {
    /// Inverse video swaps which colour value renders in which visual role
    /// — this is the *only* place that swap happens; the class-building
    /// side just asks for "the foreground/background to render" and gets
    /// the already-inverted answer. See the module doc's "Inverse" section
    /// for why this beats a CSS combinator table.
    fn effective_fg_bg(&self) -> (Option<Color>, Option<Color>) {
        if self.inverse {
            (self.bg, self.fg)
        } else {
            (self.fg, self.bg)
        }
    }

    fn classes(&self) -> Vec<String> {
        let mut classes = Vec::new();
        if self.bold {
            classes.push("ansi-bold".to_string());
        }
        if self.dim {
            classes.push("ansi-dim".to_string());
        }
        if self.italic {
            classes.push("ansi-italic".to_string());
        }
        if self.underline {
            classes.push("ansi-underline".to_string());
        }
        if self.inverse {
            classes.push("ansi-inverse".to_string());
        }
        let (fg, bg) = self.effective_fg_bg();
        if let Some(c) = fg {
            classes.push(format!("ansi-fg-{}", c.class_suffix()));
        }
        if let Some(c) = bg {
            classes.push(format!("ansi-bg-{}", c.class_suffix()));
        }
        classes
    }
}

/// Consumes one escape sequence immediately after an already-consumed ESC
/// (`\u{1b}`) byte. Returns `Some(params)` only for a recognised SGR run
/// (`CSI … m`) with its numeric parameters parsed; every other sequence —
/// or a malformed/truncated one — is consumed in full (as far as it can be,
/// up to end of input) and `None` is returned, so its bytes are dropped
/// rather than ever reaching the output.
fn consume_escape(chars: &mut Peekable<Chars>) -> Option<Vec<u32>> {
    match chars.peek().copied() {
        Some('[') => {
            chars.next();
            consume_csi(chars)
        }
        Some(']') => {
            chars.next();
            consume_osc(chars);
            None
        }
        Some('(') | Some(')') | Some('#') | Some('%') => {
            // Two-byte-suffix escapes (VT100 character-set designation and
            // friends): ESC + intermediate + one designator char.
            chars.next();
            chars.next();
            None
        }
        Some(_) => {
            // A single-byte-suffix escape (ESC 7, ESC 8, ESC =, ESC >, …).
            chars.next();
            None
        }
        None => None, // truncated: lone ESC at end of input, drop it
    }
}

/// Consumes a CSI sequence's parameter bytes, any intermediate bytes, and
/// its final byte. Returns the parsed SGR parameters only when the final
/// byte is `'m'` and every parameter byte was a digit or `;` (a private
/// marker like `?` — used only with DEC modes, never SGR — is treated as
/// "not SGR" and dropped like any other CSI sequence). A final byte is
/// never found (truncated input) drops everything read so far.
fn consume_csi(chars: &mut Peekable<Chars>) -> Option<Vec<u32>> {
    let mut params_buf = String::new();
    let mut plain_params = true;
    loop {
        match chars.next() {
            Some(c) if ('0'..='9').contains(&c) || c == ';' => {
                params_buf.push(c);
            }
            Some(c) if ('\u{20}'..='\u{3f}').contains(&c) => {
                // Any other parameter/intermediate byte (private markers
                // like '?', or true intermediates like ' ') — accepted as
                // part of the sequence so it is consumed, but disqualifies
                // this run from being read as a plain numeric SGR list.
                plain_params = false;
            }
            Some('m') => {
                return if plain_params { Some(parse_sgr_params(&params_buf)) } else { None };
            }
            Some(c) if ('\u{40}'..='\u{7e}').contains(&c) => {
                return None; // a real final byte, just not 'm'
            }
            Some(_) => {
                // Any other byte inside a CSI sequence is not valid CSI
                // syntax; stop treating this as CSI at all rather than
                // looping forever.
                return None;
            }
            None => return None, // truncated: nothing left to read
        }
    }
}

/// Consumes an OSC (Operating System Command) sequence's body, terminated
/// by BEL (`\u{07}`) or ST (`ESC \`). Used e.g. for pane-title-setting
/// sequences, which carry no visual information for a screen snapshot.
/// Runs to end of input harmlessly if no terminator is ever found.
fn consume_osc(chars: &mut Peekable<Chars>) {
    loop {
        match chars.next() {
            Some('\u{07}') | None => return,
            Some('\u{1b}') => {
                if chars.peek() == Some(&'\\') {
                    chars.next();
                    return;
                }
                // A bare ESC inside an OSC body that isn't `ESC \` — not
                // spec-compliant, but treat it as the OSC's own end rather
                // than risk swallowing an unrelated following escape.
                return;
            }
            Some(_) => {}
        }
    }
}

fn parse_sgr_params(buf: &str) -> Vec<u32> {
    if buf.is_empty() {
        return vec![0];
    }
    buf.split(';').map(|p| p.parse::<u32>().unwrap_or(0)).collect()
}

/// Applies one SGR run's parameters to `style` in order, per ECMA-48 /
/// common terminal convention. Unknown codes are ignored — never an error,
/// never a panic — since a screen snapshot must always render *something*
/// rather than refuse on a code this parser doesn't model.
fn apply_sgr(style: &mut Style, params: &[u32]) {
    let mut i = 0;
    while i < params.len() {
        match params[i] {
            0 => *style = Style::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            7 => style.inverse = true,
            21 | 22 => {
                style.bold = false;
                style.dim = false;
            }
            23 => style.italic = false,
            24 => style.underline = false,
            27 => style.inverse = false,
            30..=37 => style.fg = Some(Color::Basic((params[i] - 30) as u8)),
            38 => {
                if let Some(consumed) = apply_extended_color(&params[i + 1..], |c| style.fg = c) {
                    i += consumed;
                } else {
                    break; // malformed extended-colour params: stop parsing safely
                }
            }
            39 => style.fg = None,
            40..=47 => style.bg = Some(Color::Basic((params[i] - 40) as u8)),
            48 => {
                if let Some(consumed) = apply_extended_color(&params[i + 1..], |c| style.bg = c) {
                    i += consumed;
                } else {
                    break;
                }
            }
            49 => style.bg = None,
            90..=97 => style.fg = Some(Color::Basic((params[i] - 90 + 8) as u8)),
            100..=107 => style.bg = Some(Color::Basic((params[i] - 100 + 8) as u8)),
            _ => {} // unrecognised SGR code: ignore, never emitted or errored
        }
        i += 1;
    }
}

/// Handles the tail of a `38;…`/`48;…` extended-colour run (the params
/// *after* the leading 38/48 itself). `set` is called with the resolved
/// colour (or `None`, for an unrendered truecolour value). Returns how many
/// extra parameter slots were consumed (so the caller's index can skip past
/// them), or `None` if the params are too short to be well-formed — the
/// caller stops processing the rest of this SGR run rather than
/// misinterpret a stray number as an unrelated code.
fn apply_extended_color(rest: &[u32], mut set: impl FnMut(Option<Color>)) -> Option<usize> {
    match rest.first() {
        Some(5) => {
            let n = *rest.get(1)?;
            set(Some(Color::from_256(n.min(255) as u8)));
            Some(2)
        }
        Some(2) => {
            // 24-bit truecolour: consume "2;r;g;b" fully (so the r/g/b
            // numbers are never misread as separate SGR codes) without
            // rendering an exact colour — see the module doc.
            rest.get(1)?;
            rest.get(2)?;
            rest.get(3)?;
            set(None);
            Some(4)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- plain text, no escapes ---

    #[test]
    fn plain_text_with_no_escapes_passes_through_unwrapped() {
        assert_eq!(to_html("hello world"), "hello world");
    }

    #[test]
    fn wide_cjk_and_emoji_survive_translation_byte_for_byte() {
        let raw = "屏幕内容 😀\n❯ ";
        assert_eq!(to_html(raw), raw);
    }

    // --- escaping order: escape first, THEN wrap — the security-critical case ---

    #[test]
    fn to_html_escapes_html_metacharacters_before_wrapping_markup() {
        let raw = "\u{1b}[31m<script>alert(1)</script> & \"quoted\"\u{1b}[0m";
        let html = to_html(raw);
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(html.contains("&amp;"), "{html}");
        assert!(html.contains("&quot;quoted&quot;"), "{html}");
        // The colour markup itself must be the only real `<...>` tag present.
        assert!(html.starts_with("<span class=\"ansi-fg-red\">"), "{html}");
    }

    #[test]
    fn plain_text_containing_html_metacharacters_is_still_escaped() {
        let html = to_html("a < b & c > d \"quote\"");
        assert_eq!(html, "a &lt; b &amp; c &gt; d &quot;quote&quot;");
    }

    // --- basic 16 colours ---

    #[test]
    fn basic_foreground_colour_renders_as_a_class() {
        let html = to_html("\u{1b}[31mred text\u{1b}[0m");
        assert_eq!(html, "<span class=\"ansi-fg-red\">red text</span>");
    }

    #[test]
    fn basic_background_colour_renders_as_a_class() {
        let html = to_html("\u{1b}[44mblue bg\u{1b}[0m");
        assert_eq!(html, "<span class=\"ansi-bg-blue\">blue bg</span>");
    }

    #[test]
    fn bright_colours_render_with_a_bright_prefixed_class() {
        let html = to_html("\u{1b}[91mbright red\u{1b}[0m");
        assert_eq!(html, "<span class=\"ansi-fg-bright-red\">bright red</span>");
    }

    #[test]
    fn combined_fg_and_bg_in_one_sgr_run_both_apply() {
        let html = to_html("\u{1b}[31;44mboth\u{1b}[0m");
        assert!(html.contains("ansi-fg-red"), "{html}");
        assert!(html.contains("ansi-bg-blue"), "{html}");
    }

    #[test]
    fn default_colour_codes_clear_the_active_colour() {
        let html = to_html("\u{1b}[31mred\u{1b}[39mplain");
        assert_eq!(html, "<span class=\"ansi-fg-red\">red</span>plain");
    }

    // --- 256-colour palette ---

    #[test]
    fn extended_256_foreground_colour_renders_with_a_numeric_class() {
        let html = to_html("\u{1b}[38;5;196morange-red\u{1b}[0m");
        assert_eq!(html, "<span class=\"ansi-fg-256-196\">orange-red</span>");
    }

    #[test]
    fn extended_256_background_colour_renders_with_a_numeric_class() {
        let html = to_html("\u{1b}[48;5;22mtext\u{1b}[0m");
        assert_eq!(html, "<span class=\"ansi-bg-256-22\">text</span>");
    }

    #[test]
    fn extended_256_colour_index_below_16_uses_the_theme_aware_basic_class() {
        let html = to_html("\u{1b}[38;5;1mtext\u{1b}[0m");
        assert_eq!(html, "<span class=\"ansi-fg-red\">text</span>");
    }

    #[test]
    fn truecolor_params_are_consumed_without_corrupting_later_codes() {
        // The embedded "0" in the RGB triplet must never be misread as SGR
        // reset; bold set beforehand must survive past the truecolour run.
        let html = to_html("\u{1b}[1;38;2;10;0;200mtext\u{1b}[0m");
        assert!(html.contains("ansi-bold"), "{html}");
        assert!(!html.contains("ansi-fg-"), "{html}"); // not rendered, but parsed safely
    }

    // --- attributes ---

    #[test]
    fn bold_dim_italic_underline_each_render_their_own_class() {
        assert_eq!(to_html("\u{1b}[1mx\u{1b}[0m"), "<span class=\"ansi-bold\">x</span>");
        assert_eq!(to_html("\u{1b}[2mx\u{1b}[0m"), "<span class=\"ansi-dim\">x</span>");
        assert_eq!(to_html("\u{1b}[3mx\u{1b}[0m"), "<span class=\"ansi-italic\">x</span>");
        assert_eq!(to_html("\u{1b}[4mx\u{1b}[0m"), "<span class=\"ansi-underline\">x</span>");
    }

    #[test]
    fn inverse_with_no_explicit_colour_renders_the_inverse_class_only() {
        let html = to_html("\u{1b}[7mx\u{1b}[0m");
        assert_eq!(html, "<span class=\"ansi-inverse\">x</span>");
    }

    #[test]
    fn inverse_swaps_which_role_an_explicit_colour_renders_in() {
        // fg=red with inverse must render red as the BACKGROUND class, not
        // the foreground class — see `Style::effective_fg_bg`.
        let html = to_html("\u{1b}[31;7mx\u{1b}[0m");
        assert!(html.contains("ansi-bg-red"), "{html}");
        assert!(!html.contains("ansi-fg-red"), "{html}");
    }

    #[test]
    fn reset_clears_every_attribute_and_colour() {
        let html = to_html("\u{1b}[1;31;4mstyled\u{1b}[0mplain");
        assert_eq!(html, "<span class=\"ansi-bold ansi-underline ansi-fg-red\">styled</span>plain");
    }

    // --- unknown / non-visual sequences: dropped, never emitted raw ---

    #[test]
    fn cursor_movement_sequences_are_dropped_entirely() {
        assert_eq!(to_html("a\u{1b}[2J\u{1b}[1;1Hb"), "ab");
    }

    #[test]
    fn unknown_sgr_codes_are_ignored_without_corrupting_the_rest() {
        let html = to_html("\u{1b}[59;31mtext\u{1b}[0m");
        assert_eq!(html, "<span class=\"ansi-fg-red\">text</span>");
    }

    #[test]
    fn osc_title_sequence_is_dropped_entirely() {
        assert_eq!(to_html("a\u{1b}]0;window title\u{7}b"), "ab");
    }

    #[test]
    fn osc_title_sequence_terminated_by_st_is_dropped_entirely() {
        assert_eq!(to_html("a\u{1b}]0;window title\u{1b}\\b"), "ab");
    }

    #[test]
    fn dec_private_mode_sequences_are_dropped_entirely() {
        // Cursor-visibility toggling (`?25l`/`?25h`) — never SGR, must never
        // be misread as one and must never leak into the page.
        assert_eq!(to_html("a\u{1b}[?25lb\u{1b}[?25hc"), "abc");
    }

    #[test]
    fn single_byte_escape_sequences_are_dropped_entirely() {
        // ESC 7 / ESC 8 (save/restore cursor).
        assert_eq!(to_html("a\u{1b}7b\u{1b}8c"), "abc");
    }

    #[test]
    fn charset_designation_sequences_are_dropped_entirely() {
        assert_eq!(to_html("a\u{1b}(Bb"), "ab");
    }

    #[test]
    fn truncated_escape_at_end_of_input_is_dropped_without_panicking() {
        assert_eq!(to_html("plain\u{1b}"), "plain");
        assert_eq!(to_html("plain\u{1b}["), "plain");
        assert_eq!(to_html("plain\u{1b}[31"), "plain");
    }

    #[test]
    fn empty_input_produces_empty_output() {
        assert_eq!(to_html(""), "");
    }
}
