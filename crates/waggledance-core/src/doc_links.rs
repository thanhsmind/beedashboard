//! Turn documentation paths an agent prints into links back into waggledance.
//!
//! An agent working in a bee project names its own documents constantly —
//! `docs/specs/agent-terminal.md`, `docs/history/<feature>/CONTEXT.md` — and
//! every one of them is a file waggledance already renders. The screen showing
//! those names is one click away from the document itself, and this module is
//! that click.
//!
//! It runs **after** [`crate::ansi::to_html`], over that function's output,
//! not over the raw screen: the ANSI translation is what makes the text safe
//! to embed, and rewriting before it would mean escaping a link's own markup
//! away. Working on the produced HTML means stepping over the markup already
//! there — the `<span class="…">` wrappers each styled run gets — which
//! [`linkify_docs`] does by tracking whether it currently sits inside a tag.
//!
//! Only `docs/…/*.md` becomes a link. waggledance serves rendered markdown; a
//! directory or a `.png` under the same root would produce a link to a page
//! that does not exist, which is worse than plain text (decision locked with
//! the feature: "chỉ file .md dưới docs/").

/// The characters a path may contain. Deliberately narrow: a terminal frame
/// is full of punctuation that abuts a path without belonging to it — a
/// trailing `,` or `)` or a closing quote — and a path that swallowed them
/// would link to nothing.
fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_')
}

/// Rewrite every `docs/…/*.md` path in `html` into a link under `base`.
///
/// `base` is the prefix a path is joined onto — `/p/<project>/` for a
/// same-origin link, or `https://host/p/<project>/` when a display hostname
/// is configured. It is used verbatim; the caller owns the trailing slash.
///
/// `html` must be the output of [`crate::ansi::to_html`] (or otherwise
/// already HTML-escaped). Text inside a tag is never touched, so a path that
/// somehow appeared in an attribute stays where it is.
pub fn linkify_docs(html: &str, base: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    let mut in_tag = false;

    // `i` walks byte offsets but only ever lands on a character boundary: a
    // pane's frame is full of multi-byte glyphs (`❯`, box drawing, CJK), and
    // stepping a byte at a time would slice one in half.
    while i < html.len() {
        let c = html[i..].chars().next().expect("i is a char boundary");
        let clen = c.len_utf8();
        if in_tag {
            out.push(c);
            if c == '>' {
                in_tag = false;
            }
            i += clen;
            continue;
        }
        if c == '<' {
            in_tag = true;
            out.push(c);
            i += clen;
            continue;
        }
        // A candidate starts only where a path could: at the beginning, or
        // after something that is not itself part of a path. Without this,
        // the `docs/` inside `mydocs/x.md` would match.
        let at_boundary = i == 0 || !is_path_char(html[..i].chars().next_back().unwrap_or(' '));
        if at_boundary && html[i..].starts_with("docs/") {
            let rest = &html[i..];
            let end = rest.find(|c: char| !is_path_char(c)).unwrap_or(rest.len());
            let path = &rest[..end];
            if path.ends_with(".md") && !path.contains("..") {
                out.push_str(&format!(
                    r#"<a class="term-doc-link" href="{base}{path}" target="_blank" rel="noopener noreferrer">{path}</a>"#
                ));
                i += end;
                continue;
            }
        }
        out.push(c);
        i += clen;
    }
    out
}

/// The characters a URL may contain. Wide enough for a real path, query
/// string, and fragment; narrow enough to stop at whitespace, ANSI-HTML
/// markup, and the prose punctuation an agent wraps around a link.
fn is_url_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '/' | '.'
                | '-'
                | '_'
                | '~'
                | ':'
                | '?'
                | '#'
                | '['
                | ']'
                | '@'
                | '!'
                | '$'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | '+'
                | ','
                | ';'
                | '='
                | '%'
        )
}

/// Trim trailing punctuation an agent's prose wraps around a URL — a
/// sentence's `.`, `,`, `;`, `:`, `!`, `?`, or a closing bracket/quote that
/// has no opener earlier in the match. `https://x.dev/(a)` keeps its own
/// paired `)`; `(see https://x.dev/foo)` keeps the sentence's `)` outside.
fn trim_trailing_url_punctuation(url: &str) -> &str {
    let mut end = url.len();
    while end > 0 {
        let c = url[..end].chars().next_back().expect("end > 0");
        let clen = c.len_utf8();
        let before = &url[..end - clen];
        let trim = match c {
            '.' | ',' | ';' | ':' | '!' | '?' => true,
            ')' => before.matches('(').count() <= before.matches(')').count(),
            ']' => before.matches('[').count() <= before.matches(']').count(),
            '}' => before.matches('{').count() <= before.matches('}').count(),
            '\'' | '"' => !before.contains(c),
            _ => false,
        };
        if !trim {
            break;
        }
        end -= clen;
    }
    &url[..end]
}

/// Whether `url` names a host after its scheme. Prose that merely *mentions*
/// a scheme — "only `http://` and `https://` qualify" — matches the scheme
/// pattern with nothing behind it, and a link to `http://` goes nowhere; a
/// host has to start with an alphanumeric character to count.
fn has_host(url: &str) -> bool {
    url.split_once("://")
        .and_then(|(_, host)| host.chars().next())
        .is_some_and(|c| c.is_ascii_alphanumeric())
}

/// Rewrite every bare `http://` or `https://` URL in `html` into a link that
/// opens in a new tab.
///
/// `html` must be the output of [`crate::ansi::to_html`] (or otherwise
/// already HTML-escaped), the same contract [`linkify_docs`] runs under —
/// text inside a tag is never touched. It also tracks whether it currently
/// sits inside an `<a>…</a>` pair: a URL that lands in a link's own anchor
/// text (say, one [`linkify_docs`] already produced, or one the caller ran
/// first) must never be wrapped in a second, nested anchor.
///
/// Deliberately narrow: only a literal `http://` or `https://` scheme
/// qualifies. A bare hostname (`example.com`) or a `host:port` with no
/// scheme is never linked — the wide net would catch far too much of an
/// agent's ordinary prose (a git remote, a `host:port` pair, a version
/// number).
///
/// A URL a terminal wrapped across two screen rows is not rejoined — each
/// line is handled on its own, since the character set a match runs over
/// stops at a newline the same way it stops at a space.
pub fn linkify_urls(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    let mut in_tag = false;
    let mut anchor_depth: u32 = 0;

    while i < html.len() {
        let c = html[i..].chars().next().expect("i is a char boundary");
        let clen = c.len_utf8();

        if in_tag {
            out.push(c);
            if c == '>' {
                in_tag = false;
            }
            i += clen;
            continue;
        }

        if c == '<' {
            // Peek the whole tag to see whether it opens or closes an
            // anchor, so text inside one is never linked a second time.
            let tag_end = html[i..].find('>').map(|p| i + p + 1).unwrap_or(html.len());
            let tag_lower = html[i..tag_end].trim_start_matches('<').to_ascii_lowercase();
            if tag_lower.starts_with("a ") || tag_lower.starts_with("a>") {
                anchor_depth += 1;
            } else if tag_lower.starts_with("/a>") {
                anchor_depth = anchor_depth.saturating_sub(1);
            }
            in_tag = true;
            out.push(c);
            i += clen;
            continue;
        }

        if anchor_depth == 0 {
            let at_boundary = i == 0
                || !html[..i]
                    .chars()
                    .next_back()
                    .map(|c: char| c.is_ascii_alphanumeric())
                    .unwrap_or(false);
            let rest = &html[i..];
            if at_boundary && (rest.starts_with("http://") || rest.starts_with("https://")) {
                let end = rest.find(|c: char| !is_url_char(c)).unwrap_or(rest.len());
                let url = trim_trailing_url_punctuation(&rest[..end]);
                if has_host(url) {
                    out.push_str(&format!(
                        r#"<a class="term-url-link" href="{url}" target="_blank" rel="noopener noreferrer">{url}</a>"#
                    ));
                    i += url.len();
                    continue;
                }
            }
        }

        out.push(c);
        i += clen;
    }
    out
}

/// The link prefix for `project_id`: absolute when a display hostname is
/// configured, same-origin otherwise. A hostname carrying its own scheme is
/// taken as given; a bare one is assumed to be reached over https, which is
/// what a configured public hostname is for.
pub fn link_base(project_id: &str, hostname: Option<&str>) -> String {
    match hostname {
        Some(h) if !h.trim().is_empty() => {
            let h = h.trim().trim_end_matches('/');
            if h.starts_with("http://") || h.starts_with("https://") {
                format!("{h}/p/{project_id}/")
            } else {
                format!("https://{h}/p/{project_id}/")
            }
        }
        _ => format!("/p/{project_id}/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_a_markdown_path_under_docs() {
        let out = linkify_docs("see docs/specs/agent-terminal.md now", "/p/bee/");
        assert!(
            out.contains(r#"href="/p/bee/docs/specs/agent-terminal.md""#),
            "{out}"
        );
        assert!(out.contains(r#"target="_blank""#), "{out}");
        assert!(out.contains(r#"rel="noopener noreferrer""#), "{out}");
        assert!(out.starts_with("see "), "{out}");
        assert!(out.ends_with(" now"), "{out}");
    }

    #[test]
    fn leaves_directories_and_non_markdown_alone() {
        for text in ["docs/knowledge/", "docs/assets/logo.png", "docs/"] {
            let out = linkify_docs(text, "/p/bee/");
            assert_eq!(out, text, "must not link {text}");
        }
    }

    /// The path must start where a path can start — the `docs/` buried inside
    /// a longer word is not one.
    #[test]
    fn only_matches_at_a_path_boundary() {
        let out = linkify_docs("mydocs/specs/x.md", "/p/bee/");
        assert_eq!(out, "mydocs/specs/x.md");
    }

    /// Punctuation that merely abuts a path is not part of it: linking it
    /// would produce a URL nobody can serve.
    #[test]
    fn stops_at_punctuation_that_abuts_the_path() {
        let out = linkify_docs("(docs/specs/x.md), next", "/p/bee/");
        assert!(out.contains(r#"href="/p/bee/docs/specs/x.md""#), "{out}");
        assert!(out.contains("), next"), "{out}");
    }

    /// The screen arrives as HTML with styled runs already wrapped; the
    /// rewrite steps over that markup instead of through it.
    #[test]
    fn never_rewrites_inside_a_tag() {
        let html = r#"<span class="ansi-fg-red">docs/specs/x.md</span>"#;
        let out = linkify_docs(html, "/p/bee/");
        assert!(out.starts_with(r#"<span class="ansi-fg-red">"#), "{out}");
        assert!(out.contains(r#"<a class="term-doc-link" href="/p/bee/docs/specs/x.md""#), "{out}");
        assert!(out.ends_with("</span>"), "{out}");
        // The span's own attribute text is untouched — one `<a>` only.
        assert_eq!(out.matches("<a ").count(), 1, "{out}");
    }

    /// A traversal dressed as a doc path never becomes a link — the link
    /// would be an invitation to walk out of the project.
    #[test]
    fn refuses_a_path_with_a_parent_component() {
        let out = linkify_docs("docs/../../etc/passwd.md", "/p/bee/");
        assert_eq!(out, "docs/../../etc/passwd.md");
    }

    /// (regression) A pane's frame is full of multi-byte glyphs — the shell
    /// prompt `❯`, box drawing, CJK. Walking the text a byte at a time sliced
    /// one in half and panicked mid-render, taking the whole screen route
    /// down with it.
    #[test]
    fn survives_multi_byte_glyphs_around_a_path() {
        let out = linkify_docs("❯ cat docs/specs/x.md · 完了 ✓", "/p/bee/");
        assert!(out.starts_with("❯ cat "), "{out}");
        assert!(out.contains(r#"href="/p/bee/docs/specs/x.md""#), "{out}");
        assert!(out.ends_with(" · 完了 ✓"), "{out}");
    }

    #[test]
    fn base_is_same_origin_without_a_hostname() {
        assert_eq!(link_base("bee", None), "/p/bee/");
        assert_eq!(link_base("bee", Some("   ")), "/p/bee/");
    }

    #[test]
    fn base_uses_a_configured_hostname() {
        assert_eq!(
            link_base("bee", Some("waggledance.gogl.be")),
            "https://waggledance.gogl.be/p/bee/"
        );
        assert_eq!(
            link_base("bee", Some("http://box.local:7700/")),
            "http://box.local:7700/p/bee/"
        );
    }

    #[test]
    fn links_a_plain_url() {
        let out = linkify_urls("see https://example.dev/foo now");
        assert!(
            out.contains(r#"<a class="term-url-link" href="https://example.dev/foo" target="_blank" rel="noopener noreferrer">https://example.dev/foo</a>"#),
            "{out}"
        );
        assert!(out.starts_with("see "), "{out}");
        assert!(out.ends_with(" now"), "{out}");
    }

    #[test]
    fn keeps_the_full_stop_outside_a_url_ending_a_sentence() {
        let out = linkify_urls("check https://example.dev/foo.");
        assert!(
            out.contains(r#"href="https://example.dev/foo""#),
            "{out}"
        );
        assert!(out.ends_with("foo</a>."), "{out}");
    }

    /// An agent explaining the rule prints the schemes themselves — "only
    /// http:// and https:// qualify". A scheme with no host behind it links
    /// nowhere, so it stays prose.
    #[test]
    fn leaves_a_scheme_with_no_host_as_plain_text() {
        let out = linkify_urls("only http:// and https:// qualify");
        assert_eq!(out, "only http:// and https:// qualify", "{out}");
    }

    /// Punctuation is not a host either: the trailing `.` trims away and what
    /// remains is still a bare scheme.
    #[test]
    fn leaves_a_scheme_followed_only_by_punctuation_as_plain_text() {
        let out = linkify_urls("the prefix is https://.");
        assert_eq!(out, "the prefix is https://.", "{out}");
    }

    /// A URL inside another link's own anchor text must never be wrapped a
    /// second time — a nested `<a>` is the failure this guards against.
    #[test]
    fn never_nests_an_anchor_inside_an_existing_anchor_text() {
        let html = r#"<a class="term-doc-link" href="/p/bee/x">https://example.dev/foo</a>"#;
        let out = linkify_urls(html);
        assert_eq!(out, html, "{out}");
        assert_eq!(out.matches("<a ").count(), 1, "{out}");
    }

    /// A URL sitting inside another tag's attribute — an `href`, say — is
    /// markup, not screen text, and must stay untouched.
    #[test]
    fn never_rewrites_a_url_inside_an_attribute() {
        let html = r#"<a href="https://example.dev/foo">click here</a>"#;
        let out = linkify_urls(html);
        assert_eq!(out, html, "{out}");
        assert_eq!(out.matches("<a ").count(), 1, "{out}");
    }

    #[test]
    fn leaves_a_bare_hostname_as_text() {
        let text = "visit example.dev or box.local:7700 for more";
        let out = linkify_urls(text);
        assert_eq!(out, text, "{out}");
    }
}
