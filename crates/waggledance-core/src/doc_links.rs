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
}
