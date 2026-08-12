//! Server-rendered HTML views. Self-contained: layout + CSS + JS as consts.
//! Theme is CSS-variable driven (no-flash head script); code colors come from
//! `/highlight.css` (syntect class-based), so themes switch without re-render.

use mdview_core::bee::{
    feature_cell_span, list_archived_feature_dirs, read_archived_cells, BeeApprovedGates,
    BeeBacklog, BeeBuckets, BeeCell, BeeDecisionSummary, BeeFeaturePhase, BeePbi, BeeReview,
    BeeReviewStatus, BeeShippedFeature, BeeSnapshot, BeeState, BeeWorkspace,
    BeeWorktree,
};
use mdview_core::config::Config;
use mdview_core::domain::{IndexedFile, Project, RenderedPage, SearchResult};

pub fn layout(title: &str, head_extra: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en" data-theme="atelier" class="fg-root">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · Bee Artifact</title>
<script>
// No-flash: apply saved scheme before body renders.
(function() {{
  try {{
    var t = localStorage.getItem('mdview-theme') || 'system';
    var dark = t === 'dark' || (t === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    document.documentElement.setAttribute('data-scheme', dark ? 'dark' : 'light');
  }} catch (e) {{}}
}})();
</script>
<link rel="stylesheet" href="{css_url}">
<link rel="stylesheet" href="/highlight.css">
{head_extra}
</head>
<body>
{body}
<script src="{js_url}"></script>
</body>
</html>"#,
        css_url = app_css_url(),
        js_url = app_js_url(),
    )
}

/// `unassigned_visible` is D5/D4's presence marker, never contents: `true`
/// exactly when both the D7 `terminal.enabled` switch and, per toa-4/D9,
/// the group's own `unassigned_enabled` switch are on (checked with no
/// herdr call and no session, so this unauthenticated page never learns
/// whether any pane is actually unassigned) — renders a link to
/// `/_terminal/unassigned`, whose own route is gated identically. `false`
/// (the default for either switch) renders this page byte-identical to how
/// it looked before this feature existed — the group being off by policy
/// rather than by an empty pane list means this marker must disappear too,
/// not just the panes it would have listed.
/// Split a registered project id into the project it branches from and the
/// branch's own name, when the id carries `bee worktree new`'s separator.
/// `beedashboard--wt--agent-terminal` → `("beedashboard", "agent-terminal")`.
/// Returns `None` for an ordinary project, and for a malformed id whose half
/// on either side of the separator is empty — an id that names no parent, or
/// names one but no branch, is treated as an ordinary project rather than
/// nested under a row that would not be there.
/// The text a timestamp shows before any script runs: the instant cut to the
/// minute, with the date and the clock separated by a space rather than the
/// machine-facing `T`. `2026-08-07T09:17:49.240188408Z` → `2026-08-07 09:17`.
/// Seconds and the sub-second digits are dropped — nobody reads nine decimal
/// places of a last-seen time, and they were pushing the project's own name
/// off the line. The full instant stays in the element's `datetime`, which is
/// what the script and any machine reader use.
fn short_instant(iso: &str) -> String {
    let (date, rest) = match iso.split_once('T') {
        Some(parts) => parts,
        // Not the shape this function knows how to cut; show it whole rather
        // than guess at where its minute ends.
        None => return iso.to_string(),
    };
    let hhmm: String = rest.chars().take(5).collect();
    if hhmm.len() < 5 || !hhmm.contains(':') {
        return iso.to_string();
    }
    format!("{date} {hhmm}")
}

fn worktree_branch(id: &str) -> Option<(&str, &str)> {
    let (parent, branch) = id.split_once("--wt--")?;
    if parent.is_empty() || branch.is_empty() {
        return None;
    }
    Some((parent, branch))
}

/// project-suggestions (D1, D2, D4, D6): one folder an agent-backed herdr
/// pane is running in that sits under no registered project —
/// `server.rs::suggested_projects`'s own output type. `path` is the pane's
/// cwd exactly as herdr reported it (D1, no walk to a repository root, one
/// trailing slash trimmed for dedup). D2 authorizes only a full path and a
/// count on this page — never a name, title, workspace, or tab — so this
/// type carries no such field (unlike `TerminalPaneView`). `pane_count` is
/// how many agent-backed panes share that one directory.
pub struct ProjectSuggestion {
    pub path: String,
    pub pane_count: usize,
}

pub fn project_list_page(
    projects: &[(Project, usize, Vec<TerminalPaneView>)],
    unassigned_visible: bool,
    suggestions: &[ProjectSuggestion],
    register_error: Option<&str>,
) -> String {
    let listing = if projects.is_empty() {
        "<p class=\"fg-empty\">Chưa có project nào trong Bee Artifact. Đăng ký: <code>mdview register &lt;dir&gt;</code> hoặc gọi MCP <code>mdview_view_file</code>.</p>".to_string()
    } else {
        // One row per project, not a grid of cards: a card's width was spent on
        // air while the names — which is what the eye is actually scanning for —
        // wrapped over three lines. Rows put every name on the same left edge.
        // A worktree sits under the project it branches from rather than beside
        // it as a peer, so a repo with three checkouts reads as one project with
        // three branches instead of four unrelated entries. The filesystem path
        // is deliberately omitted (unauthenticated page).
        let registered: std::collections::HashSet<&str> =
            projects.iter().map(|(p, _, _)| p.id.as_str()).collect();
        // Order: every project that is not a branch keeps the order it arrived
        // in, and each one is immediately followed by its own branches, in
        // their own arrival order. A branch is never emitted twice and never
        // emitted before its parent, whatever order the registry hands them in.
        let mut ordered: Vec<(&(Project, usize, Vec<TerminalPaneView>), Option<&str>)> = Vec::new();
        for entry in projects {
            if worktree_branch(&entry.0.id)
                .map(|(parent, _)| registered.contains(parent))
                .unwrap_or(false)
            {
                continue;
            }
            ordered.push((entry, None));
            for child in projects {
                if let Some((parent, branch)) = worktree_branch(&child.0.id) {
                    if parent == entry.0.id {
                        ordered.push((child, Some(branch)));
                    }
                }
            }
        }
        let mut rows = String::new();
        for ((p, count, panes), branch) in ordered {
            // A worktree whose parent is not registered has nothing to nest
            // under, so it stands on its own and keeps its full name — never
            // hidden, and never indented under a row that is not there.
            let (row_class, label) = match branch {
                Some(branch) => ("proj-row proj-row--branch", branch.to_string()),
                None => ("proj-row", p.name.clone()),
            };
            // D1/D1a/D2/D3/D5: a sibling of `proj-row__link`, never nested
            // inside it — an anchor inside an anchor is invalid HTML and
            // browsers unnest it, which would break the row link itself.
            let badges = project_badges(&p.id, panes);
            rows.push_str(&format!(
                r#"<li class="{row_class}">
  <a class="proj-row__link" href="/p/{id}/">
    <span class="proj-row__name">{label}</span>
    <span class="proj-row__meta">{count} markdown files · <time class="proj-row__time" datetime="{seen}">{seen_short}</time></span>
  </a>
  {badges}
  <form class="proj-row__delete" method="post" action="/api/projects/{id}/unregister" data-project="{name}">
    <button type="submit" class="proj-card__del" aria-label="Remove {name} from Bee Artifact" title="Remove from Bee Artifact">✕</button>
  </form>
</li>"#,
                row_class = row_class,
                id = esc(&p.id),
                label = esc(&label),
                name = esc(&p.name),
                count = count,
                seen = esc(&p.last_seen_at),
                seen_short = esc(&short_instant(&p.last_seen_at)),
                badges = badges,
            ));
        }
        format!(r#"<ul class="proj-list">{rows}</ul>"#, rows = rows)
    };
    // project-suggestions (D1-D6): one row per unregistered folder an
    // agent-backed herdr pane is running in, each a one-press form posting
    // straight to the existing `/api/projects/register` route (D9a/D9b's
    // whole guard chain applies unchanged; nothing here validates the path
    // a second time). An empty `suggestions` slice — the gate off, no herdr
    // call reached, or genuinely nothing unregistered running — renders no
    // section at all, byte-identical to the page before this feature.
    let suggestions_block = if suggestions.is_empty() {
        String::new()
    } else {
        let mut rows = String::new();
        for s in suggestions {
            // D2: the visible path text and the hidden input's `value` are
            // the same escaped string, computed once — byte-identical by
            // construction rather than by two separate `esc()` calls that
            // could drift apart.
            let path = esc(&s.path);
            rows.push_str(&format!(
                r#"<li class="proj-row proj-suggestion">
  <div class="proj-row__link proj-suggestion__info">
    <span class="proj-row__name proj-suggestion__path">{path}</span>
    <span class="proj-row__meta">{count} pane{plural}</span>
  </div>
  <form class="proj-suggestion__register" method="post" action="/api/projects/register">
    <input type="hidden" name="path" value="{path}">
    <button type="submit" class="fg-btn fg-btn--primary">Register</button>
  </form>
</li>"#,
                path = path,
                count = s.pane_count,
                plural = if s.pane_count == 1 { "" } else { "s" },
            ));
        }
        format!(
            r#"<section class="proj-suggestions">
  <h3 class="proj-suggestions__title">Suggested projects</h3>
  <p class="fg-empty proj-suggestions__hint">Sessions are running here, but the folder is not registered.</p>
  <ul class="proj-list proj-suggestions__list">{rows}</ul>
</section>"#,
            rows = rows,
        )
    };
    // D5/D4: presence only — no agent name, no cwd, not even a count, ever
    // reaches this markup. The link's own route (`/_terminal/unassigned`)
    // carries the same session/switch/method gate as every other terminal
    // route; this card only says the group exists.
    let unassigned_card = if unassigned_visible {
        r#"<div class="proj-cards">
  <a class="fg-card proj-card__link" href="/_terminal/unassigned">
    <div class="fg-card__title">Unassigned agents</div>
    <div class="fg-card__sub">Agents running outside every registered project</div>
  </a>
</div>"#
            .to_string()
    } else {
        String::new()
    };
    // D10: a fixed, static message keyed by the register route's own fixed
    // error code — the submitted path never reaches this page (see
    // `register_error_message`'s own doc). An unrecognized or absent code
    // renders no banner at all.
    let register_banner = match register_error.and_then(register_error_message) {
        Some(msg) => format!(
            r#"<div class="fg-banner fg-banner--danger"><span class="fg-banner__dot"></span><span class="fg-banner__body">{msg}</span></div>"#,
            msg = esc(msg),
        ),
        None => String::new(),
    };
    let body = format!(
        r#"{topbar}
<main class="fg-page"><h2 class="fg-pagehead__title">Projects</h2>{register_banner}{add_form}{listing}{suggestions_block}{unassigned_card}</main>"#,
        topbar = topbar(""),
        register_banner = register_banner,
        add_form = project_add_form(),
        listing = listing,
        suggestions_block = suggestions_block,
        unassigned_card = unassigned_card,
    );
    layout("Projects", "", &body)
}

/// D7's add-project form — one absolute-path field; the project name is
/// derived server-side from the directory (`server.rs::register_project`
/// passes `None` to `Engine::register`), so there is no second field for it.
/// D8: a plain HTML form post to the new register route, the same
/// method="post"/action shape as the unregister form above — no fetch, no
/// JavaScript. Every value here is static markup: the submitted path is
/// never echoed back onto this page (D9a/D10's refusal messages, below, are
/// fixed text keyed by a fixed error code, never the raw input).
fn project_add_form() -> &'static str {
    r#"<form class="proj-add" method="post" action="/api/projects/register">
  <div class="fg-field">
    <label class="fg-field__label" for="proj-add-path">Register a project</label>
    <input class="fg-input" type="text" id="proj-add-path" name="path" placeholder="/absolute/path/to/project" autocomplete="off">
  </div>
  <button type="submit" class="fg-btn fg-btn--primary">Register</button>
</form>"#
}

/// D10's fixed refusal messages, keyed by `register_project`'s own fixed
/// error codes (`server.rs::validate_register_path`, plus its own generic
/// `"failed"`). Every branch is static text — nothing user-supplied reaches
/// this unauthenticated page, which is the whole point of carrying a code
/// rather than the path itself. An unrecognized code renders no banner,
/// fail-safe rather than fail-loud: it can only arrive by someone hand-
/// crafting the query string, since every code this page itself ever
/// redirects with is one of the branches below.
fn register_error_message(code: &str) -> Option<&'static str> {
    Some(match code {
        "invalid_path" => "Enter an absolute path with no relative (\"..\") segments.",
        "not_found" => "That path does not exist.",
        "not_directory" => "That path is not a directory.",
        "denied" => "That path cannot be registered.",
        "duplicate" => "That project is already registered.",
        "too_large" => "That directory has too many markdown files to register.",
        "too_slow" => "That directory took too long to scan to register.",
        "failed" => "That project could not be registered. Try again.",
        _ => return None,
    })
}

/// D1, as clarified by D1a — D2, D3, D5: one badge per terminal pane in
/// `panes`, which the caller has already matched against this project's own
/// D2 containment boundary (`server.rs::project_panes`, the same query
/// `pane_strip` above draws from at pane-page scope). Each badge is a link
/// to that pane's own terminal view carrying the same [`status_pill`] and
/// program (`kind` — the herdr agent kind, or the literal `shell` for an
/// agent-less pane) `pane_strip` prints; the pane's `name` field — the
/// agent's own name — never reaches this markup (D1a). An empty `panes`
/// list (the switch off, the snapshot unavailable, an unconstructable
/// boundary, or simply no pane inside this project) renders no container at
/// all, so an unbadged row is byte-identical to how every row rendered
/// before this feature (D6) — not an empty `<nav>` that would say the same
/// thing twice.
fn project_badges(project_id: &str, panes: &[TerminalPaneView]) -> String {
    if panes.is_empty() {
        return String::new();
    }
    let pid = esc(project_id);
    let mut out = String::from(r#"<nav class="proj-row__badges" aria-label="Terminal panes">"#);
    for p in panes {
        out.push_str(&format!(
            r#"<a class="proj-row__badge" href="/p/{pid}/_terminal/pane/{pane_id}">{status_pill}<span class="proj-row__badge-program">{program}</span></a>"#,
            pid = pid,
            pane_id = esc(&p.pane_id),
            status_pill = status_pill(&p.status),
            program = esc(&p.kind),
        ));
    }
    out.push_str("</nav>");
    out
}

/// A registered project's landing page: a card linking into the bee board
/// when the project has one (D3), plus a card to open the project's docs
/// when it has any. D6/agent-terminal-8: this is the only page carrying the
/// [`project_tabs`] strip, so it renders for **every** registered project —
/// not only bee ones — otherwise a project with no `.bee/` directory would
/// redirect straight to its entry file and never show the Terminal tab at
/// all. `bee` gates only the Bee board card; the tab strip itself is
/// unconditional.
pub fn project_home_page(project: &Project, entry: Option<&str>, bee: bool) -> String {
    let docs_card = match entry {
        Some(rel) => format!(
            r#"<a class="fg-card proj-card__link" href="/p/{pid}/{rel}">
  <div class="fg-card__title">Browse docs</div>
  <div class="fg-card__sub">{rel}</div>
</a>"#,
            pid = esc(&project.id),
            rel = esc(rel),
        ),
        None => String::new(),
    };
    let bee_card = if bee {
        format!(
            r#"<a class="fg-card proj-card__link" href="/p/{pid}/_bee">
  <div class="fg-card__title">Bee board</div>
  <div class="fg-card__sub">Doing · Waiting · Stuck · Done</div>
</a>"#,
            pid = esc(&project.id),
        )
    } else {
        String::new()
    };
    let body = format!(
        r#"{topbar}
{tab_style}
<main class="fg-page">
  <h2 class="fg-pagehead__title">{name}</h2>
  <div class="proj-cards">
    {bee_card}
    {docs_card}
  </div>
</main>"#,
        topbar = topbar_full(
            "",
            &format!(
                "<span class=\"crumb\">{name}</span>",
                name = esc(&project.name)
            ),
            "",
            &project_tabs(&project.id, "overview"),
        ),
        tab_style = PROJECT_TAB_STYLE,
        name = esc(&project.name),
        bee_card = bee_card,
        docs_card = docs_card,
    );
    layout(&project.name, "", &body)
}

/// Inline styling for [`project_tabs`] — kept beside the pages that render
/// it (same precedent as `bee_board_page`'s own inline `<style>`), not added
/// to `app.css`: this cell's declared files are `server.rs`/`views.rs` only.
const PROJECT_TAB_STYLE: &str = r#"<style>
/* The section nav rides in the top bar beside the brand rather than opening
   a band of its own under it: it is three short links, and a full-width
   bordered strip cost more vertical room on a handset than the links it
   held. No bottom border here — the bar's own edge is the rule. */
.proj-tabs { display: flex; flex-wrap: wrap; gap: var(--space-3); min-width: 0; }
.proj-tab { padding: 0; color: var(--color-text-muted); text-decoration: none; border-bottom: 2px solid transparent; }
.proj-tab--active { color: var(--color-text); border-color: var(--color-action); font-weight: var(--weight-semibold); }
/* A page whose subject is one live screen keeps its blocks close: the page
   frame's default var(--space-5) rhythm is for reading, and here it only
   pushed the screen down behind the fold. */
.fg-page--tight { gap: var(--space-2); }
/* The pane tabs and the control that makes a new pane are one row: picking
   a pane and adding a pane are the same kind of move, and stacking them
   spent a second band on one button. */
.pane-bar { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: var(--space-2); min-width: 0; }
.term-create { display: flex; flex-wrap: wrap; gap: var(--space-1); margin-left: auto; }
/* terminal-pane-scope D4: the pane tab strip that picks which single pane
   this page renders — herdr's own sidebar shape, one entry per pane, each a
   plain link to that pane's own address. Wraps rather than scrolling
   sideways: a project with many panes still needs every one reachable on a
   handset. */
.pane-strip { display: flex; flex-wrap: wrap; gap: var(--space-2); min-width: 0; }
.pane-strip__tab { display: flex; align-items: center; gap: var(--space-1); padding: var(--space-1) var(--space-2); border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--radius-sm); color: var(--color-text-muted); text-decoration: none; background: var(--color-surface-raised); }
.pane-strip__tab--active { color: var(--color-text); border-color: var(--color-action); font-weight: var(--weight-semibold); }
.term-pane__meta { flex: 0 0 auto; color: var(--color-text-muted); font-size: var(--type-body-sm-size); }
/* A pane's frame is a grid, not prose: `pre-wrap` + `word-break` re-flowed
   every long line and broke the box drawing of any TUI agent, and a fixed
   height cut the frame off behind an inner scrollbar. `pre` keeps the grid;
   the box takes its height from its own content, so the whole screen is
   readable without a second scroll inside the page. Only the horizontal axis
   scrolls — that is what replaces wrapping when a line runs wider than the
   card. */
/* Nothing in this card may grow the page sideways. A `pre` of unwrapped
   terminal lines has an enormous min-content width, and every ancestor that
   defaults to `min-width: auto` would happily take it — so the chain from the
   card down is pinned to its container, and the screen scrolls inside itself
   instead of pushing the page out. */
.term-panes, .term-pane, .term-screen-wrap, .term-controls, .term-reply { min-width: 0; max-width: 100%; }
/* A pane sheds the card chrome it used to sit in: the border, the padding and
   the raised surface only framed a frame, and on a narrow screen that inset
   was width the terminal itself needed. The screen's own dark box is the
   card now. */
.term-pane { border: none; background: transparent; box-shadow: none; padding: 0; gap: var(--space-2); }
/* With no card edges left to separate them, panes need their own spacing. */
.term-panes { display: flex; flex-direction: column; gap: var(--space-5); }
/* terminal-scroll-perf-1: the pane itself never scrolls vertically (that is
   the page's job, height: auto above) but it does scroll horizontally, and
   on a phone the page scroll that shows/hides the URL bar must stay the
   browser's own smooth-scrolling path rather than route through anything
   this element could intercept. `touch-action` tells the browser this
   element only ever pans, never pinch-zooms or long-presses, so it can
   start that pan on the first touch frame instead of waiting to see if a
   gesture handler intervenes; `-webkit-overflow-scrolling: touch` is the
   older iOS momentum-scroll opt-in some Safari versions still consult;
   `overscroll-behavior: contain` keeps a pane that hits the end of its own
   horizontal scroll from bubbling the gesture into a page-level scroll
   (or a pull-to-refresh) the operator didn't ask for; `contain: layout
   paint` tells the browser this element's internal layout and paint can
   never affect anything outside it, so a resize-triggered refit's writes
   and reads stay scoped to the pane instead of invalidating the page. */
.term-screen { margin-top: var(--space-2); padding: var(--space-2); background: #1c1f26; color: #d7dae0; border-radius: var(--radius-sm); white-space: pre; font-family: var(--font-mono, monospace); font-size: var(--type-body-sm-size); line-height: 1.25; height: auto; min-width: 0; max-width: 100%; overflow-x: auto; overflow-y: hidden; touch-action: pan-x pan-y; -webkit-overflow-scrolling: touch; overscroll-behavior: contain; contain: layout paint; }
/* Applied by `assets/app.js` only once the fit has bottomed out: at that
   width no readable type size can hold the grid, so the lines wrap instead
   of running off the side. */
.term-screen--wrapped { white-space: pre-wrap; overflow-wrap: anywhere; overflow-x: hidden; }
/* term-frame-blocks: a table or TUI frame `mdview_core::ansi::to_html` wraps
   in its own `<div class="term-frame">` keeps its grid on every screen size,
   never the phone-only `pre-wrap` the rest of `.term-screen` takes below —
   a `<div>` inside a `<pre>` still inherits `white-space` from its ancestor
   (it is an inherited CSS property), so this rule has to restate `pre` and
   `overflow-x: auto` on the frame itself rather than rely on nesting alone.
   Declared once, unconditionally (not inside the narrow-screen `@media`
   block below), it wins over the inherited value at every width — the
   surrounding prose lines are untouched and keep wrapping under that rule
   exactly as before. */
.term-frame { white-space: pre; overflow-x: auto; }
/* A document path an agent printed, now clickable. It keeps whatever colour
   the surrounding ANSI run gave it — recolouring would lose information the
   agent meant to convey — and says it is a link by underlining. */
.term-doc-link { color: inherit; text-decoration: underline; text-underline-offset: 2px; cursor: pointer; }
.term-doc-link:hover { text-decoration-thickness: 2px; }
/* Below this width the answer is the same whatever the measurement says: no
   readable type size fits a terminal frame on a handset, so the lines wrap.
   Stated in CSS rather than left to the script, because a page whose script
   never ran — or ran from the Unassigned page's own copy, which has no fit of
   its own — must still not push the layout sideways. Everything else on the
   card gives up its horizontal ambitions here too: rows wrap instead of
   overflowing, and the buttons share the width rather than being pushed off
   the edge. */
@media (max-width: 720px) {
  .term-screen { white-space: pre-wrap; overflow-wrap: anywhere; overflow-x: hidden; }
  .term-keys button, .term-scroll button, .term-reply__send, .term-reply__stage { padding: var(--space-2) var(--space-3); }
  .term-reply__actions { justify-content: stretch; }
  .term-reply__actions button { flex: 1; }
}
/* The controls read top to bottom in the order they are reached: the screen,
   then the two controls that move the screen, then the keys that drive the
   agent, then the box you write in with its own send row under it. The reply
   box owns the full width — squeezing it beside two buttons left barely a
   phone's worth of room for the one field an operator actually types into. */
.term-reply { display: flex; flex-direction: column; gap: var(--space-2); margin-top: var(--space-2); }
.term-reply__text { width: 100%; min-width: 0; box-sizing: border-box; padding: var(--space-1) var(--space-2); border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--radius-sm); font-family: var(--font-mono, monospace); font-size: var(--type-body-sm-size); line-height: 1.35; background: var(--color-bg); color: var(--color-text); resize: vertical; }
.term-reply__actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--space-2); }
.term-reply__send, .term-reply__stage { padding: var(--space-1) var(--space-3); min-height: 44px; border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--radius-sm); background: var(--color-surface-raised); color: var(--color-text); cursor: pointer; }
/* Send is the primary of the pair — Stage beside it stays the quiet one. */
.term-reply__send { background: var(--color-action); border-color: var(--color-action); color: var(--color-bg); font-weight: var(--weight-semibold); }
/* One tight control block under the screen, on a single line: the arrows and
   the named keys read as one row of controls rather than two bands with a
   gap between them. The row carries no margin of its own and wraps only when
   the card is too narrow to hold both groups. */
.term-controls { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); margin-top: 0; }
.term-keys { display: flex; flex-wrap: wrap; gap: var(--space-1); }
.term-keys button { padding: var(--space-1) var(--space-2); min-height: 44px; border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--radius-sm); background: var(--color-surface-raised); color: var(--color-text); cursor: pointer; font-size: var(--type-caption-size); }
/* D5, amended 2026-08-08: every key in the row stands 44px tall so the row
   reads as one control band — Enter, Esc and Tab match the arrows. The
   arrows alone keep the 44px minimum WIDTH and the body-size glyph (pressed
   repeatedly, often with a thumb); the named keys keep their padding-driven
   width. The `--move` modifier picks the arrows out rather than position:
   once both groups share one row, "the direct child of `.term-controls`"
   reaches the named keys too. */
.term-keys--move button { min-width: 44px; font-size: var(--type-body-size); }
/* The two screen-moving controls belong to the screen, not to the keys that
   type into the pane, so they ride on it: centred, wholly inside its lower
   edge rather than straddling it. `sticky` keeps them reachable while a tall
   screen is scrolled past — the pair a reader wants is the one for the
   screen they are looking at. The negative pull is deeper than the pair's
   own rendered height, which is what both lifts it clear of the edge and
   gives back the flow row it would otherwise open above the keys. */
.term-screen-wrap { position: relative; display: flow-root; }
.term-scroll { position: sticky; bottom: var(--space-3); z-index: 2; display: flex; flex-wrap: wrap; gap: var(--space-2); width: max-content; margin: calc(-1 * var(--space-7)) var(--space-3) 0 auto; padding: var(--space-1); border-radius: var(--radius-sm); background: var(--color-surface-raised); box-shadow: 0 1px 4px rgb(0 0 0 / 0.35); }
/* Bigger than the named keys beside the arrows: these are read at a glance
   and pressed mid-scroll. Width comes from padding, never a `min-width` —
   that 44px target is the arrows' own, and the pair keeps the smaller box
   the touch-target rule reserves for everything else. */
.term-scroll button { padding: var(--space-2) var(--space-4); border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--radius-sm); background: var(--color-surface-raised); color: var(--color-text); cursor: pointer; font-size: var(--type-body-sm-size); }
.term-transcript { margin-top: var(--space-2); padding: var(--space-2); background: var(--color-surface-sunken); border-radius: var(--radius-sm); font-family: var(--font-mono, monospace); font-size: var(--type-body-sm-size); max-height: 24em; overflow-y: auto; }
.term-transcript__line { white-space: pre-wrap; word-break: break-word; }
/* agent-switch-drawer-2: a fixed edge tab reaches the cross-project agent
   feed (`GET /api/agents`) from any terminal page without first hunting
   through the pane bar's own menu, which only ever lists this project's
   own panes. Checkbox-driven the same way `pane_bar`'s own
   `.pane-menu__toggle` is (`assets/app.css`) — no script owns open/closed,
   only Escape/outside-click layers on top via the generic `.js-menu`
   handler in `assets/app.js`, which this markup's own `js-menu` class
   already opts into. */
.agent-drawer__check { position: absolute; opacity: 0; pointer-events: none; }
.agent-drawer__tab {
  position: fixed;
  top: 50%;
  right: 0;
  z-index: var(--z-nav);
  transform: translateY(-50%);
  display: inline-flex;
  align-items: center;
  min-height: 44px;
  padding: var(--space-2) var(--space-3);
  border: var(--border-width-hairline) solid var(--color-border-strong);
  border-right: 0;
  border-radius: var(--radius-sm) 0 0 var(--radius-sm);
  background: var(--color-surface-raised);
  color: var(--color-text-muted);
  font-size: var(--type-body-sm-size);
  cursor: pointer;
  box-shadow: var(--elevation-md);
}
.agent-drawer__check:checked + .agent-drawer__tab,
.agent-drawer__check:focus-visible + .agent-drawer__tab {
  color: var(--color-text);
  border-color: var(--color-action);
}
/* `.fg-drawer` (components.css) is always fixed and full-height; every
   other caller in the app only ever mounts it while open, so this is the
   only page that needs to hide it off-screen itself. */
.agent-drawer .fg-drawer {
  transform: translateX(100%);
  transition: transform var(--motion-fast) var(--ease-standard);
}
.agent-drawer__check:checked ~ .fg-drawer {
  transform: translateX(0);
}
.agent-drawer__section { padding: var(--space-3) var(--space-1) var(--space-1); color: var(--color-text-subtle); font-family: var(--type-label-font); font-size: var(--type-micro-size); letter-spacing: var(--type-label-tracking); text-transform: var(--type-label-case); }
.agent-drawer__section:first-child { padding-top: 0; }
.agent-drawer__item { flex-direction: column; align-items: flex-start; gap: 2px; min-height: 44px; }
.agent-drawer__suffix { color: var(--color-text-muted); font-size: var(--type-caption-size); }
</style>"#;

/// D6: the Terminal tab is always present on a project page, whether or not
/// herdr is running or the terminal has ever been reached — this renders
/// from the project id alone, with no herdr call and no auth check, so its
/// presence never depends on either. `active` is `"overview"`, `"terminal"`
/// or `"transcript"` (agent-terminal-16, D9: the Transcript tab sits beside
/// Terminal, not inside its frame).
/// terminal-pane-scope D4: one entry per pane in the project's own
/// D2-boundary-filtered list, each an ordinary anchor to that pane's own
/// address (`/p/:id/_terminal/pane/:pane_id` or
/// `/p/:id/_transcript/pane/:pane_id` — `kind` picks which). The strip is the
/// only place a pane's identity is printed: workspace and tab, its status
/// pill, then the program the pane is running (`claude`, or `shell` when no
/// agent holds it), so the strip reads the way herdr's own sidebar does and
/// the card below it is nothing but the pane's own output.
/// An empty `panes` list renders nothing — the page's own empty state
/// (`pane_cards`/`transcript_cards`'s `empty_msg`) already says so, and an
/// empty strip would say it twice. No JavaScript: these are links, and one
/// pane per page means `assets/app.js` polls one screen instead of N.
fn pane_strip(project_id: &str, kind: &str, panes: &[TerminalPaneView], selected: Option<&str>) -> String {
    if panes.is_empty() {
        return String::new();
    }
    let mut out = String::from(r#"<nav class="pane-strip" aria-label="Panes">"#);
    for p in panes {
        let active = selected == Some(p.pane_id.as_str());
        out.push_str(&pane_tab(project_id, kind, p, active, ""));
    }
    out.push_str("</nav>");
    out
}

/// One entry in the pane strip. `extra` adds classes to the anchor — the pane
/// bar uses it to mark the standalone copy of the active tab it shows on a
/// narrow screen, where the strip itself is inside the menu.
fn pane_tab(
    project_id: &str,
    kind: &str,
    p: &TerminalPaneView,
    active: bool,
    extra: &str,
) -> String {
    let cls = match (active, extra.is_empty()) {
        (true, true) => "pane-strip__tab pane-strip__tab--active".to_string(),
        (true, false) => format!("pane-strip__tab pane-strip__tab--active {extra}"),
        (false, true) => "pane-strip__tab".to_string(),
        (false, false) => format!("pane-strip__tab {extra}"),
    };
    format!(
        r#"<a class="{cls}" href="/p/{pid}/_{kind}/pane/{pane_id}"><span class="term-pane__id">{workspace} · {tab}</span> {status_pill}<span class="term-pane__meta">{program}</span></a>"#,
        cls = cls,
        pid = esc(project_id),
        kind = kind,
        pane_id = esc(&p.pane_id),
        workspace = esc(&p.workspace),
        tab = esc(&p.tab),
        status_pill = status_pill(&p.status),
        program = esc(&p.kind),
    )
}

/// The row above the pane: on a wide screen the pane strip on the left and
/// the creation controls on the right, exactly as before. On a narrow one it
/// collapses to a single line — the tab of the pane being viewed, and a menu
/// control holding every other pane and the creation controls.
///
/// The active tab is rendered twice on purpose: once standalone for the
/// narrow row, once inside the strip where it keeps its place among its
/// siblings. Only one of the two is ever displayed, so a reader — including
/// one using a screen reader — meets a single copy.
fn pane_bar(
    project_id: &str,
    kind: &str,
    panes: &[TerminalPaneView],
    selected: Option<&str>,
    create: &str,
) -> String {
    let strip = pane_strip(project_id, kind, panes, selected);
    if strip.is_empty() {
        // No panes to switch between: there is nothing for a menu to hold,
        // and the creation controls (when the page has any) stand alone.
        return format!(r#"<div class="pane-bar">{create}</div>"#, create = create);
    }
    let current = panes
        .iter()
        .find(|p| selected == Some(p.pane_id.as_str()))
        .or_else(|| panes.first())
        .map(|p| pane_tab(project_id, kind, p, true, "pane-bar__current"))
        .unwrap_or_default();
    format!(
        r#"<div class="pane-bar js-menu">
  {current}
  <input type="checkbox" id="pane-menu-toggle" class="pane-menu__toggle">
  <label class="pane-menu__button" for="pane-menu-toggle" title="Panes"><span class="menu-label">Panes</span><span aria-hidden="true">☰</span></label>
  <div class="pane-menu__panel">{strip}{create}</div>
</div>"#,
        current = current,
        strip = strip,
        create = create,
    )
}

fn project_tabs(project_id: &str, active: &str) -> String {
    let id = esc(project_id);
    let cls = |key: &str| {
        if key == active {
            "proj-tab proj-tab--active"
        } else {
            "proj-tab"
        }
    };
    format!(
        r#"<nav class="proj-tabs" aria-label="Project sections">
  <a class="{overview_cls}" href="/p/{id}/">Overview</a>
  <a class="{terminal_cls}" href="/p/{id}/_terminal">Terminal</a>
  <a class="{transcript_cls}" href="/p/{id}/_transcript">Transcript</a>
</nav>"#,
        overview_cls = cls("overview"),
        terminal_cls = cls("terminal"),
        transcript_cls = cls("transcript"),
        id = id,
    )
}

/// One agent already resolved against a project's D2 containment boundary
/// (`server.rs::project_panes`) — plain display fields only, no herdr wire
/// type crosses into this module. `workspace` and `tab` are the labels
/// `Snapshot::workspace_label_for_id`/`tab_label_for_id` resolve (herdr's own
/// sidebar reads a pane by the same two labels) — carried here rather than
/// re-joined in this module, since only `server.rs` holds the snapshot.
/// `status` admits a pane with no agent (terminal-pane-scope D2/D3): the
/// caller sets it to `"shell"` rather than borrowing an `AgentStatus`
/// vocabulary the row does not have.
pub struct TerminalPaneView {
    pub pane_id: String,
    pub kind: String,
    pub name: String,
    pub status: String,
    pub title: String,
    pub cwd: String,
    pub workspace: String,
    pub tab: String,
}

/// D3's status pill: maps a [`TerminalPaneView::status`] value onto
/// `.fg-status`'s three tone modifiers
/// (`crates/mdview/assets/atelier/components.css:145-151`). `done` reads
/// ready, `working` reads warn, `blocked` reads blocked; `idle`, `unknown`
/// (`herdr::wire::AgentStatus::Unknown`) and `shell` (no agent at all) all
/// keep the bare, unmodified `.fg-status` — the neutral dot — so a status a
/// row does not have is never borrowed from another state's colour. The
/// pill's own text always names the state, whichever it is.
fn status_pill(status: &str) -> String {
    let modifier = match status {
        "done" => " fg-status--ready",
        "working" => " fg-status--warn",
        "blocked" => " fg-status--blocked",
        _ => "",
    };
    format!(
        r#"<span class="fg-status{modifier}"><span class="fg-status__dot"></span>{status}</span>"#,
        modifier = modifier,
        status = esc(status),
    )
}

/// Shared by [`terminal_page`] and [`unassigned_terminal_page`]: one pane's
/// card — screen viewport, reply form, key buttons — the exact widget set
/// `assets/app.js`'s project-scoped poller/handlers drive. `empty_msg` is
/// rendered instead when `panes` is empty, kept distinct per caller so an
/// empty project and an empty Unassigned group are never confusable with
/// each other, or with [`terminal_down_page`]'s herdr-silent wording.
/// The card carries no heading of its own: [`pane_strip`] already names the
/// pane directly above it, and repeating that identity a second time only
/// pushed the screen further down a handset's viewport.
///
/// terminal-image-attach: `attach` gates the image-attach control (picker
/// button, hidden file input, chip list, error slot) that rides inside the
/// reply form. It is `true` only for [`terminal_page`] — the Unassigned page
/// shares this card markup but has no project-scoped
/// `/p/:id/_terminal/:pane_id/attach` route to upload against (plan finding
/// 7), so [`unassigned_terminal_page`] passes `false` and renders none of it.
fn pane_cards(panes: &[TerminalPaneView], empty_msg: &str, attach: bool) -> String {
    if panes.is_empty() {
        return format!(r#"<p class="fg-empty">{}</p>"#, esc(empty_msg));
    }
    let mut out = String::new();
    for p in panes {
        let attach_block = if attach {
            format!(
                r#"
    <div class="term-attach" data-pane-id="{pane_id}">
      <input type="file" class="term-attach__input" data-pane-id="{pane_id}" accept="image/*" multiple aria-label="Attach images to send to {name}" hidden>
      <button type="button" class="term-attach__btn" data-pane-id="{pane_id}">Attach images</button>
      <ul class="term-attach__chips" data-pane-id="{pane_id}"></ul>
      <p class="term-attach__error" data-pane-id="{pane_id}" role="alert" hidden></p>
    </div>"#,
                pane_id = esc(&p.pane_id),
                name = esc(&p.name),
            )
        } else {
            String::new()
        };
        out.push_str(&format!(
            r#"<div class="fg-card term-pane" data-pane-id="{pane_id}">
  <div class="term-screen-wrap">
    <pre class="term-screen" data-pane-id="{pane_id}" aria-live="polite">Loading screen…</pre>
    <div class="term-scroll" data-pane-id="{pane_id}" aria-label="Scroll {name}'s history">
      <button type="button" data-scroll="older">Older</button>
      <button type="button" data-scroll="live">Live</button>
    </div>
  </div>
  <div class="term-controls">
    <div class="term-keys term-keys--move" data-pane-id="{pane_id}" aria-label="Move around {name}'s screen">
      <button type="button" data-key="up">↑</button>
      <button type="button" data-key="down">↓</button>
      <button type="button" data-key="left">←</button>
      <button type="button" data-key="right">→</button>
    </div>
    <div class="term-keys" data-pane-id="{pane_id}" aria-label="Send a key to {name}">
      <button type="button" data-key="enter">Enter</button>
      <button type="button" data-key="escape">Esc</button>
      <button type="button" data-key="tab">Tab</button>
    </div>
  </div>
  <form class="term-reply" data-pane-id="{pane_id}">
    <textarea class="term-reply__text" rows="3" placeholder="Type a reply… (Ctrl+Enter to send)" aria-label="Reply to {name}" autocomplete="off"></textarea>{attach_block}
    <div class="term-reply__actions">
      <button type="button" class="term-reply__stage">Stage</button>
      <button type="submit" class="term-reply__send">Send</button>
    </div>
  </form>
</div>"#,
            pane_id = esc(&p.pane_id),
            name = esc(&p.name),
            attach_block = attach_block,
        ));
    }
    out
}

/// Inline wiring for [`terminal_create_controls`]'s "New shell"/preset
/// buttons — POSTs to `create/pane` or `create/agent` and reloads the page
/// on success so the freshly created pane joins `assets/app.js`'s own
/// poller on the next render.
///
/// agent-terminal-13: not folded into `assets/app.js` — that file is not
/// among this cell's declared files (`crates/mdview/src/server.rs`,
/// `crates/mdview/src/views.rs`, `crates/mdview-core/src/config.rs`), so the
/// creation controls' own click wiring lives here instead, the same
/// deliberate duplication `UNASSIGNED_TERMINAL_SCRIPT` already documents for
/// the same reason ("a later cell to fold both into one shared script once
/// `assets/app.js` is in scope").
const TERMINAL_CREATE_SCRIPT: &str = r#"<script>
(function () {
  function postJson(url, body) {
    return fetch(url, {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  }
  function afterCreate(promise, failMsg) {
    promise
      .then(function (res) {
        if (res.ok) {
          location.reload();
          return;
        }
        return res.json().then(function (b) {
          alert((b && b.error) || failMsg);
        });
      })
      .catch(function () {
        alert(failMsg);
      });
  }
  Array.prototype.slice
    .call(document.querySelectorAll(".term-create[data-project-id]"))
    .forEach(function (box) {
      var pid = box.getAttribute("data-project-id");
      var paneBtn = box.querySelector(".term-create__pane");
      if (paneBtn) {
        paneBtn.addEventListener("click", function () {
          afterCreate(
            postJson("/p/" + encodeURIComponent(pid) + "/_terminal/create/pane", {}),
            "could not start a shell"
          );
        });
      }
      Array.prototype.slice
        .call(box.querySelectorAll(".term-create__agent[data-preset]"))
        .forEach(function (btn) {
          btn.addEventListener("click", function () {
            afterCreate(
              postJson("/p/" + encodeURIComponent(pid) + "/_terminal/create/agent", {
                preset: btn.getAttribute("data-preset"),
              }),
              "could not start an agent"
            );
          });
        });
    });
})();
</script>"#;

/// D8's creation controls (agent-terminal-13): a "New shell" button is
/// always offered — plain-shell creation needs no preset — plus one button
/// per operator-configured preset **label**, never argv (P4): the argv
/// itself never crosses into this view, only the label the server's
/// `terminal_create_agent` keys it by. Zero configured presets renders zero
/// preset buttons, proving the must-have "with no presets configured, the
/// creation control offers nothing [for agents]" at the render layer — the
/// route-level half of that same truth is `terminal_create_agent`'s own
/// refusal when `body.preset` matches nothing.
fn terminal_create_controls(project_id: &str, presets: &[String]) -> String {
    let preset_buttons: String = presets
        .iter()
        .map(|label| {
            format!(
                r#"<button type="button" class="term-create__agent" data-preset="{attr}">{label}</button>"#,
                attr = esc(label),
                label = esc(label),
            )
        })
        .collect();
    format!(
        r#"<div class="term-create" data-project-id="{pid}">
  <button type="button" class="term-create__pane">New shell</button>
  {preset_buttons}
</div>
{script}"#,
        pid = esc(project_id),
        preset_buttons = preset_buttons,
        script = TERMINAL_CREATE_SCRIPT,
    )
}

/// agent-switch-drawer-2: a right-edge slide-in panel that lists every
/// agent-backed pane across every project (`GET /api/agents`), reachable
/// from any terminal page without first navigating to that pane's own
/// project. Terminal pages only — [`terminal_page`] renders both a
/// project's own pane list and its `/pane/:pane_id` view through this one
/// function, and both get the drawer; the read-only Transcript tab
/// ([`transcript_page`]) and the Unassigned page
/// ([`unassigned_terminal_page`]) do not, since there is no second place to
/// jump *from* to make the drawer worth the screen space there. Checkbox-
/// driven the same way [`pane_bar`]'s own `.pane-menu__toggle` is — no
/// script owns open/closed, only Escape/outside-click layers on top via
/// the generic `.js-menu` handler `assets/app.js` already runs. Entirely
/// static: `assets/app.js` fills `[data-agent-drawer-list]` in from JSON
/// once the drawer opens, so there is nothing here to escape.
const AGENT_SWITCH_DRAWER: &str = r#"<div class="agent-drawer js-menu">
  <input type="checkbox" id="agent-drawer-toggle" class="agent-drawer__check">
  <label for="agent-drawer-toggle" class="agent-drawer__tab">Agents</label>
  <div class="fg-drawer">
    <div class="fg-drawer__head"><span class="fg-drawer__title">Agents</span></div>
    <div class="fg-drawer__body" data-agent-drawer-list></div>
  </div>
</div>"#;

/// `GET /p/:id/_terminal` and `/p/:id/_terminal/pane/:pane_id` up state
/// (D2/D4/D6): one pane's own page, chosen by `selected` from the pane strip
/// (D4) rendered above it. `selected` is `None` only when `panes` is empty
/// (the honest empty state, not a blank page) — a `Some` id not present in
/// `panes` is the caller's authorization refusal (`server.rs`'s
/// `terminal_page_inner`), never reached here. Distinct wording from
/// [`terminal_down_page`] so an empty list is never mistaken for herdr being
/// unreachable, or the reverse. `presets` is the exact configured D8 preset
/// label list (`mdview_core::config::AgentPreset`'s labels, in
/// `Config.terminal.agent_presets` order) — this view never sees argv.
pub fn terminal_page(
    project: &Project,
    panes: &[TerminalPaneView],
    selected: Option<&str>,
    presets: &[String],
) -> String {
    let empty_msg = "No agents are running under this project right now.";
    let rows = match selected.and_then(|pid| panes.iter().find(|p| p.pane_id == pid)) {
        Some(pane) => pane_cards(std::slice::from_ref(pane), empty_msg, true),
        None => pane_cards(&[], empty_msg, true),
    };
    let bar = pane_bar(
        &project.id,
        "terminal",
        panes,
        selected,
        &terminal_create_controls(&project.id, presets),
    );
    // `data-project-id` lets `assets/app.js`'s screen poller build each
    // pane's `/p/:id/_terminal/:pane_id/screen` URL without threading the id
    // through every `.term-screen` element individually.
    let body = format!(
        r#"{topbar}
{tab_style}
<main class="fg-page fg-page--tight" data-project-id="{pid}">
  {bar}
  <div class="term-panes">{rows}</div>
</main>
{drawer}"#,
        topbar = topbar_full(
            "",
            &format!(
                "<span class=\"crumb\">{name} · terminal</span>",
                name = esc(&project.name)
            ),
            "",
            &project_tabs(&project.id, "terminal"),
        ),
        tab_style = PROJECT_TAB_STYLE,
        pid = esc(&project.id),
        bar = bar,
        rows = rows,
        drawer = AGENT_SWITCH_DRAWER,
    );
    layout(&format!("{} · terminal", project.name), "", &body)
}

/// One pane's transcript card (agent-terminal-16, D9): headingless the way
/// [`pane_cards`] is, with a `.term-transcript` viewport in place of
/// `.term-screen`, `.term-reply` and `.term-keys` — this tab is read-only.
/// [`pane_strip`] above it names the pane. `assets/app.js`'s transcript poller
/// fills the viewport in, appending each newly returned record rather than
/// replacing the viewport's contents, so nothing already shown is lost
/// between polls.
fn transcript_cards(panes: &[TerminalPaneView], empty_msg: &str) -> String {
    if panes.is_empty() {
        return format!(r#"<p class="fg-empty">{}</p>"#, esc(empty_msg));
    }
    let mut out = String::new();
    for p in panes {
        out.push_str(&format!(
            r#"<div class="fg-card term-pane" data-pane-id="{pane_id}">
  <div class="term-transcript" data-pane-id="{pane_id}" aria-live="polite">Loading activity…</div>
</div>"#,
            pane_id = esc(&p.pane_id),
        ));
    }
    out
}

/// `GET /p/:id/_transcript` and `/p/:id/_transcript/pane/:pane_id` up state
/// (D4/D9): the Transcript tab beside Terminal, not a toggle inside its
/// frame — one pane's own page, chosen by `selected` from the same pane
/// strip shape `terminal_page` renders, with a transcript viewport in place
/// of a screen. `selected` is `None` only when `panes` is empty; a `Some` id
/// not present in `panes` is the caller's authorization refusal
/// (`server.rs`'s `transcript_page_inner`), never reached here. Zero panes
/// renders the same wording `terminal_page` uses for the same reason (never
/// mistaken for herdr being unreachable, see [`terminal_down_page`], which
/// this tab's down state also reuses — listing which panes belong to this
/// project still needs a herdr snapshot even though transcript content
/// itself doesn't).
pub fn transcript_page(project: &Project, panes: &[TerminalPaneView], selected: Option<&str>) -> String {
    let empty_msg = "No agents are running under this project right now.";
    let rows = match selected.and_then(|pid| panes.iter().find(|p| p.pane_id == pid)) {
        Some(pane) => transcript_cards(std::slice::from_ref(pane), empty_msg),
        None => transcript_cards(&[], empty_msg),
    };
    let bar = pane_bar(&project.id, "transcript", panes, selected, "");
    // `data-project-id` lets `assets/app.js`'s transcript poller build each
    // pane's `/p/:id/_terminal/:pane_id/transcript` URL, mirroring the
    // screen poller's own `data-project-id` use on `terminal_page`.
    let body = format!(
        r#"{topbar}
{tab_style}
<main class="fg-page fg-page--tight" data-project-id="{pid}">
  {bar}
  <div class="term-panes">{rows}</div>
</main>"#,
        topbar = topbar_full(
            "",
            &format!(
                "<span class=\"crumb\">{name} · transcript</span>",
                name = esc(&project.name)
            ),
            "",
            &project_tabs(&project.id, "transcript"),
        ),
        tab_style = PROJECT_TAB_STYLE,
        pid = esc(&project.id),
        bar = bar,
        rows = rows,
    );
    layout(&format!("{} · transcript", project.name), "", &body)
}

/// Inline poller/reply/keys wiring for [`unassigned_terminal_page`], scoped
/// to `.unassigned-panes` so it never touches a project page's own panes.
/// `assets/app.js`'s existing terminal script is not reused here — it
/// resolves every URL from a `data-project-id` attribute
/// (`/p/:id/_terminal/...`), and this group belongs to no project id; that
/// file is also not among this cell's declared files. This duplicates its
/// shape deliberately rather than inventing a different wiring convention —
/// flagged here for a later cell to fold both into one shared script once
/// `assets/app.js` is in scope.
const UNASSIGNED_TERMINAL_SCRIPT: &str = r#"<script>
(function () {
  var POLL_MS = 1500;
  var HERDR_DOWN_TEXT = "herdr is not running";
  var lastRevision = {};

  function screenUrl(paneId) {
    return "/_terminal/unassigned/" + encodeURIComponent(paneId) + "/screen";
  }
  function inputUrl(paneId) {
    return "/_terminal/unassigned/" + encodeURIComponent(paneId) + "/input";
  }
  function keysUrl(paneId) {
    return "/_terminal/unassigned/" + encodeURIComponent(paneId) + "/keys";
  }

  function pollOne(el) {
    var paneId = el.getAttribute("data-pane-id");
    fetch(screenUrl(paneId), { credentials: "same-origin" })
      .then(function (res) {
        // A 502 is the one status `herdr_down_response()` (server.rs) ever
        // sends, and only when herdr itself is unreachable — but the body
        // still has to say so, because a tunnel or proxy in front of this
        // page can hand back its own unrelated 502 HTML on a blip. Every
        // other failure (a thrown fetch below, any other status, a 502
        // whose body isn't that exact JSON) is treated as transient: the
        // pane keeps its last good screen and just gets marked stale, never
        // overwritten with wording that says the agent is gone.
        if (res.status === 502) {
          return res.json().then(function (body) {
            if (body && body.error === HERDR_DOWN_TEXT) {
              el.textContent = HERDR_DOWN_TEXT;
              el.classList.remove("term-screen--stale");
              // The next successful poll must always repaint, even if its
              // revision happens to match whatever was last drawn before
              // the outage — otherwise this banner never clears.
              delete lastRevision[paneId];
              return null;
            }
            el.classList.add("term-screen--stale");
            return null;
          });
        }
        if (!res.ok) { el.classList.add("term-screen--stale"); return null; }
        return res.json();
      })
      .then(function (body) {
        if (!body) return;
        el.classList.remove("term-screen--stale");
        if (lastRevision[paneId] === body.revision) return;
        lastRevision[paneId] = body.revision;
        // `body.text` is safe, pre-escaped HTML from mdview-core's ansi
        // translator (agent-terminal-12) — never the raw pane text — so
        // `innerHTML` here renders ANSI colour/attribute markup rather than
        // showing literal escape characters.
        el.innerHTML = body.text;
      })
      .catch(function () {
        // Thrown fetch (network blip, phone waking from sleep) or an
        // unparseable 502 body — none of these confirm herdr is actually
        // down, so the pane keeps whatever it last showed.
        el.classList.add("term-screen--stale");
      });
  }

  function pollAll() {
    Array.prototype.slice
      .call(document.querySelectorAll(".unassigned-panes .term-screen[data-pane-id]"))
      .forEach(pollOne);
  }
  pollAll();
  setInterval(pollAll, POLL_MS);

  function postJson(url, body) {
    return fetch(url, {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  }

  function sendReply(paneId, text, submit, input) {
    if (!text) return;
    postJson(inputUrl(paneId), { text: text, submit: submit })
      .then(function (res) { if (res.ok && input) input.value = ""; })
      .catch(function () {});
  }

  Array.prototype.slice
    .call(document.querySelectorAll(".unassigned-panes .term-reply[data-pane-id]"))
    .forEach(function (form) {
      var paneId = form.getAttribute("data-pane-id");
      var input = form.querySelector(".term-reply__text");
      var stageBtn = form.querySelector(".term-reply__stage");
      form.addEventListener("submit", function (ev) {
        ev.preventDefault();
        sendReply(paneId, input.value, true, input);
      });
      if (input) {
        input.addEventListener("keydown", function (ev) {
          if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey)) {
            ev.preventDefault();
            sendReply(paneId, input.value, true, input);
          }
        });
      }
      if (stageBtn) {
        stageBtn.addEventListener("click", function () {
          sendReply(paneId, input.value, false, input);
        });
      }
    });

  Array.prototype.slice
    .call(document.querySelectorAll(".unassigned-panes .term-keys[data-pane-id]"))
    .forEach(function (group) {
      var paneId = group.getAttribute("data-pane-id");
      Array.prototype.slice.call(group.querySelectorAll("button[data-key]")).forEach(function (btn) {
        btn.addEventListener("click", function () {
          var key = btn.getAttribute("data-key");
          if (!key) return;
          postJson(keysUrl(paneId), { keys: [key] }).catch(function () {});
        });
      });
    });
})();
</script>"#;

/// `GET /_terminal/unassigned` up state (D5/D4/D6): every herdr pane whose
/// cwd sits under no registered project root, gated identically to
/// [`terminal_page`] (session, D7 switch, method) — this view renders only
/// what the route already decided to hand it, so it carries no gate logic
/// of its own. Zero panes renders a named empty state distinct from both
/// [`terminal_page`]'s own empty wording and [`unassigned_terminal_down_page`].
pub fn unassigned_terminal_page(panes: &[TerminalPaneView]) -> String {
    let rows = pane_cards(panes, "No agents are running outside a registered project right now.", false);
    let body = format!(
        r#"{topbar}
{tab_style}
<main class="fg-page">
  <h2 class="fg-pagehead__title">Unassigned agents</h2>
  <p class="term-pane__meta">Agents running outside every registered project. Registering a project here never happens automatically (D5) — <a href="/">register it from the project list</a> if you want it to have its own Terminal tab.</p>
  <div class="term-panes unassigned-panes">{rows}</div>
</main>
{script}"#,
        topbar = topbar("<span class=\"crumb\">Unassigned agents</span>"),
        tab_style = PROJECT_TAB_STYLE,
        rows = rows,
        script = UNASSIGNED_TERMINAL_SCRIPT,
    );
    layout("Unassigned agents", "", &body)
}

/// `GET /_terminal/unassigned` down state (D6): herdr's socket did not
/// answer — same remedy wording [`terminal_down_page`] renders, so a poller
/// or a reader sees an identical state whether the silence was noticed on a
/// project page or here.
pub fn unassigned_terminal_down_page() -> String {
    let body = format!(
        r#"{topbar}
{tab_style}
<main class="fg-page">
  <h2 class="fg-pagehead__title">Unassigned agents</h2>
  <div class="fg-card term-pane">
    <div class="fg-card__title">herdr is not running</div>
    <div class="term-pane__meta">Start herdr, then reload this page — Bee Artifact does not start it for you unless the herdr supervisor is switched on in Settings.</div>
  </div>
</main>"#,
        topbar = topbar("<span class=\"crumb\">Unassigned agents</span>"),
        tab_style = PROJECT_TAB_STYLE,
    );
    layout("Unassigned agents", "", &body)
}

/// `GET /p/:id/_terminal` down state (D6): herdr's socket did not answer.
/// Names the remedy instead of hiding the tab or showing a raw error —
/// deliberately different wording from the empty-panes state in
/// [`terminal_page`] so the two are never visually or textually confusable.
pub fn terminal_down_page(project: &Project) -> String {
    let body = format!(
        r#"{topbar}
{tab_style}
<main class="fg-page">
  <h2 class="fg-pagehead__title">{name}</h2>
  <div class="fg-card term-pane">
    <div class="fg-card__title">herdr is not running</div>
    <div class="term-pane__meta">Start herdr, then reload this page — Bee Artifact does not start it for you unless the herdr supervisor is switched on in Settings.</div>
  </div>
</main>"#,
        topbar = topbar_full(
            "",
            &format!(
                "<span class=\"crumb\">{name} · terminal</span>",
                name = esc(&project.name)
            ),
            "",
            &project_tabs(&project.id, "terminal"),
        ),
        tab_style = PROJECT_TAB_STYLE,
        name = esc(&project.name),
    );
    layout(&format!("{} · terminal", project.name), "", &body)
}

/// The read-only bee cell board (D4/D5). feature-hub D1 replaces the
/// cell-centric Kanban board (`bee_agent_board_section`, agent-board
/// ab-1/ab-2 — now retired) with a FEATURE-centric grouped list
/// ([`bee_feature_hub_section`]): a manager asks which feature needs them,
/// which is moving, which is done, not which cell an agent holds. Every
/// feature this snapshot can place renders in exactly one of three groups —
/// Waiting on you, In Progress, Finished — see that function's own doc
/// comment for the full membership rule and the D4 ghost-card fix it
/// carries. A feature that has fully shipped (D10, `snapshot.shipped`)
/// still gets its own line in `bee_finished_section` too, unrelated to and
/// unmoved by this change — a distinct, uncapped feature-level list this
/// board has always kept separate from whatever this cell's own Finished
/// group shows. `bee_lanes_panel` stays retired (bbp-11); the feature
/// detail page's own D7 four-bucket view retires in turn under
/// feature-hub-2, replaced by [`bee_feature_page`]'s tabbed drill-down (D2)
/// — every cell this board's buckets fed it now surfaces on that page's own
/// Todos tab instead. Every path-shaped value on a
/// `BeeCell`/`BeeFeaturePhase` already arrives relativized by
/// `mdview_core::bee::read_snapshot` (no absolute path crosses into
/// `BeeSnapshot`'s public fields), so nothing further is redacted here —
/// this view only escapes for HTML safety.
///
/// board-declutter drops the pre-hub top-of-board stack entirely: the
/// lifecycle stepper, the headline KPI tiles, the Ship velocity section, the
/// Needs attention panel and the Working-on-now card (its own Running now
/// subsection included) are gone, along with the view functions and CSS
/// that only ever rendered them — `mdview_core`'s readers for that data
/// (velocity, attention, running workers, the D7 bucket counts) are
/// untouched; the feature detail page and other consumers still read them
/// from `BeeSnapshot`, this page just stops rendering them. `{top}` — now
/// just the page title and "Read <as-of>" line — is followed directly by
/// [`bee_feature_hub_section`] as this page's first main section, then
/// [`bee_finished_section`]'s standalone shipped-features list, unmoved by
/// this cell.
///
/// board-trim (D1) drops the rest of `{panels}`'s own contents down to a
/// single card: the Sessions panel (`bee_sessions_panel`: sessions,
/// worktrees, workspaces) and the Process health panel
/// (`bee_process_health_panel`: file-lock contention, model tier mix, gate
/// bypass, `read_errors`) are gone, along with the view functions and CSS
/// that only ever rendered them. `mdview_core`'s readers for that data
/// (sessions, worktrees, workspaces, reservations, tier mix, config) are
/// untouched — the feature detail page and other consumers still read them
/// from `BeeSnapshot`; this page just stops rendering them. `{panels}` now
/// carries only the Backlog & review card (`bee_backlog_panel`).
pub fn bee_board_page(project: &Project, snapshot: &BeeSnapshot) -> String {
    let body = format!(
        r#"{topbar}
{style}
<main class="fg-page bee-hub-theme">
  {top}
  {board}
  {finished}
  {panels}
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name} · bee</span>",
            name = esc(&project.name)
        )),
        style = bee_hub_style(),
        top = bee_board_top(project),
        board = bee_feature_hub_section(project, snapshot),
        finished = bee_finished_section(&project.id, &snapshot.shipped),
        panels = bee_panels_section(snapshot),
    );
    layout(&format!("{} · bee", project.name), "", &body)
}

/// D3's anthropic.com-inspired palette plus every `.bee-*` layout rule the
/// bee page family (board and, from feature-hub-2, the feature detail page)
/// shares — factored out of `bee_board_page` so the detail page can pick up
/// the exact same `--color-*` token names and card idiom rather than
/// re-declaring them and risking the two pages drifting apart. Returned as
/// one `<style>` block; every page that embeds it wraps its own content in
/// `<main class="fg-page bee-hub-theme">` to opt in (see the palette
/// comment below for why that scoping class exists at all).
fn bee_hub_style() -> String {
    format!(
        r#"<style>
.bee-finished {{ margin-bottom: var(--space-4); }}
/* D3: anthropic.com-inspired palette (cream page, warm panel, near-black
   ink, book-cloth coral accent), scoped to the bee page only via the
   `.bee-hub-theme` class on this page's own `<main>` — every other page
   keeps its default "atelier" theme untouched. This overrides only the
   Tier-2 semantic tokens (`--color-*`) the existing `fg-*` components
   already read, so no markup here or elsewhere had to change to pick it
   up. Dark reuses the exact same toggle this page already had: the
   no-flash head script (`layout`) sets `data-scheme` on `<html>` before
   first paint; this only adds a scoped override keyed off that same
   attribute, never a second toggle mechanism. */
.bee-hub-theme {{
  --color-bg: #FAF9F5;
  --color-surface: #FFFFFF;
  --color-surface-raised: #FFFFFF;
  --color-surface-sunken: #F0EEE6;
  --color-text: #1A1815;
  --color-text-muted: #5A5650;
  --color-text-subtle: #8A8478;
  --color-border: #E4DFD3;
  --color-border-strong: #D8D1C0;
  --color-action: #CC785C;
  --color-action-hover: #B3654B;
  --color-action-press: #9C5540;
  --color-brand: #CC785C;
  --color-brand-tint: #F3E3DC;
  --color-link: #CC785C;
  --color-link-hover: #B3654B;
  --color-on-action: #FFFFFF;
  --color-success: #3D7A4E;
  --color-success-tint: #E1EFE3;
  --color-warning: #B8791A;
  --color-warning-tint: #F5E7D0;
  --color-danger: #C1443B;
  --color-danger-tint: #F5DEDC;
  --color-info: #4A7A78;
  --color-info-tint: #DFEBEA;
  --color-surface-hover: #F6EFE9;
}}
html[data-scheme="dark"] .bee-hub-theme {{
  --color-bg: #241E18;
  --color-surface: #2D261F;
  --color-surface-raised: #342C24;
  --color-surface-sunken: #1C1712;
  --color-text: #F5F0E6;
  --color-text-muted: #C9BFAF;
  --color-text-subtle: #998F7E;
  --color-border: #40372C;
  --color-border-strong: #4E4335;
  --color-action: #D98868;
  --color-action-hover: #E29A7D;
  --color-action-press: #C77552;
  --color-brand: #D98868;
  --color-brand-tint: #3A2B22;
  --color-link: #D98868;
  --color-link-hover: #E29A7D;
  --color-on-action: #241E18;
  --color-success: #6FB584;
  --color-success-tint: #223226;
  --color-warning: #D9A24E;
  --color-warning-tint: #362912;
  --color-danger: #E0796F;
  --color-danger-tint: #3A211E;
  --color-info: #7FADAB;
  --color-info-tint: #22302F;
  --color-surface-hover: #342C24;
}}
.bee-hub {{ margin-bottom: var(--space-4); }}
.bee-hub__groups {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: var(--space-4); }}
/* hub-fallbacks: a grid/flex item's own default `min-width: auto` sizes it
   to its content's min-content width — normally harmless, but a clamped
   `.bee-hub__desc` below is `white-space: nowrap` at its own min-content
   size, and without `min-width: 0` breaking that default at every level
   of this chain (group, cards, card), a long description would still
   force its own column wider than its `minmax(260px, 1fr)` track and push
   the whole page into horizontal scroll on a phone — the same chain
   `.term-panes` and its siblings already pin above for the same reason. */
.bee-hub__group {{ display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }}
.bee-hub__cards {{ display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }}
.bee-hub__card {{ display: flex; flex-direction: column; gap: var(--space-1); min-width: 0; }}
.bee-hub__chips {{ display: flex; flex-wrap: wrap; gap: var(--space-1); }}
.bee-hub__progress-label {{ margin: 0; font-size: var(--type-caption-size); color: var(--color-text-subtle); }}
.bee-hub__reason {{ font-style: italic; }}
/* feature-titles: the card's own slug subtitle (shown only alongside a
   human title read from CONTEXT.md — a title-less card already shows the
   slug as its own title) and its boundary-description line, clamped so no
   card grows unbounded taller than its neighbors. hub-fallbacks swaps the
   single-line `white-space: nowrap` + ellipsis clamp for a 2-line
   `-webkit-line-clamp`: a `nowrap` line has no wrap point of its own, so on
   a narrow card it was the box (and the grid track holding it, absent the
   `min-width: 0` chain above) that grew instead of the text; `line-clamp`
   wraps normally and merely cuts off after its own line count, and
   `overflow-wrap: anywhere` still guards the one word inside it long
   enough to overflow a single line on its own (an unbroken URL, a long
   identifier). */
.bee-hub__slug {{ margin: 0; font-size: var(--type-caption-size); color: var(--color-text-subtle); }}
.bee-hub__desc {{ margin: 0; font-size: var(--type-body-sm-size); color: var(--color-text-muted); display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; overflow-wrap: anywhere; }}
/* feature-titles: the detail header's own title/slug/description stack,
   and the docs row beneath the chip row linking every markdown file the
   feature's docs dir holds (hub-fallbacks; CONTEXT.md/plan.md lead when
   present) through the viewer's own document routes. */
.bee-detail-slug {{ margin: var(--space-1) 0 0 0; font-size: var(--type-body-sm-size); color: var(--color-text-subtle); }}
.bee-detail-desc {{ margin: var(--space-1) 0 0 0; color: var(--color-text-muted); display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; overflow-wrap: anywhere; max-width: 100%; }}
.bee-detail-docs {{ display: flex; flex-wrap: wrap; gap: var(--space-2); margin: 0 0 var(--space-4) 0; }}
.bee-detail-docs a {{ color: var(--color-link); font-size: var(--type-body-sm-size); }}
.bee-done-summary {{ cursor: pointer; list-style: none; padding: var(--space-2) 0; font-weight: var(--weight-strong); color: var(--color-text); }}
.bee-done-summary::-webkit-details-marker {{ display: none; }}
.bee-done-grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: var(--space-2); padding-top: var(--space-2); }}
.bee-done-line {{ display: block; color: var(--color-text-muted); font-size: var(--type-caption-size); text-decoration: none; padding: var(--space-1) 0; border-bottom: var(--border-width-hairline) solid var(--color-border); overflow-wrap: anywhere; }}
.bee-done-line:hover {{ color: var(--color-action); }}
.bee-cell {{ padding: var(--space-2); gap: var(--space-1); }}
.bee-cell .fg-card__title {{ font-size: var(--type-body-sm-size); overflow-wrap: anywhere; }}
.bee-cell__meta {{ color: var(--color-text-subtle); font-size: var(--type-caption-size); word-break: break-word; }}
.bee-cell__detail {{ margin-top: var(--space-1); font-size: var(--type-caption-size); color: var(--color-text-subtle); }}
.bee-cell__detail summary {{ cursor: pointer; color: var(--color-text); }}
.bee-cell__detail p {{ margin: var(--space-1) 0 0 0; overflow-wrap: anywhere; }}
.bee-panels {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: var(--space-4); margin-top: var(--space-4); }}
.bee-panel__head {{ display: flex; align-items: center; gap: var(--space-2); margin: 0; }}
.bee-panel__subhead {{ margin: var(--space-3) 0 var(--space-2) 0; font-size: var(--type-heading-sm-size); }}
.bee-panel__chips {{ display: flex; flex-wrap: wrap; gap: var(--space-2); margin-bottom: var(--space-2); }}
.bee-panel__list {{ display: flex; flex-direction: column; gap: var(--space-2); }}
.bee-severity--p1 {{ font-weight: var(--weight-strong); }}
.bee-asof {{ color: var(--color-text-subtle); font-size: var(--type-body-sm-size); }}
.bee-progress {{ height: 8px; border-radius: var(--radius-pill); background: var(--color-surface-sunken); overflow: hidden; }}
.bee-progress__bar {{ height: 100%; background: var(--color-success); }}
.bee-done-summary:focus-visible {{ outline: var(--focus-width) solid var(--focus-color); outline-offset: var(--focus-offset); }}
/* feature-hub-2: the feature detail page's own header, chip row and
   CSS-only tab pattern — no JS framework, same checkbox/radio-plus-label
   idiom `topbar_full`'s own doc comment already explains the reasoning
   for (a `<details>` element hides its content even when it should not,
   past what any `display` override here can undo; a plain input a browser
   already knows how to toggle needs none of that). */
.bee-detail-head {{ display: flex; align-items: flex-start; justify-content: space-between; flex-wrap: wrap; gap: var(--space-3); margin-bottom: var(--space-2); }}
/* Same shrink chain the hub cards need: without `min-width: 0` this flex
   column grows to its description's widest line instead of wrapping it,
   and the whole detail page scrolls sideways on a phone. */
.bee-detail-head > div {{ min-width: 0; flex: 1 1 16rem; }}
.bee-detail-chips {{ display: flex; flex-wrap: wrap; gap: var(--space-2); margin-bottom: var(--space-4); }}
.bee-tabs {{ margin-top: var(--space-2); }}
.bee-tabs__radio {{ position: absolute; opacity: 0; pointer-events: none; }}
.bee-tabs__nav {{ display: flex; flex-wrap: wrap; gap: var(--space-1); border-bottom: var(--border-width-hairline) solid var(--color-border); margin-bottom: var(--space-4); }}
.bee-tabs__label {{ cursor: pointer; padding: var(--space-2) var(--space-3); color: var(--color-text-muted); font-weight: var(--weight-strong); border-bottom: 2px solid transparent; }}
.bee-tabs__label:hover {{ color: var(--color-text); }}
.bee-tabs__panel {{ display: none; }}
#bee-tab-activity:checked ~ .bee-tabs__nav label[for="bee-tab-activity"],
#bee-tab-todos:checked ~ .bee-tabs__nav label[for="bee-tab-todos"],
#bee-tab-terminal:checked ~ .bee-tabs__nav label[for="bee-tab-terminal"] {{
  color: var(--color-action);
  border-bottom-color: var(--color-action);
}}
#bee-tab-activity:checked ~ .bee-tabs__body #bee-panel-activity,
#bee-tab-todos:checked ~ .bee-tabs__body #bee-panel-todos,
#bee-tab-terminal:checked ~ .bee-tabs__body #bee-panel-terminal {{
  display: block;
}}
#bee-tab-activity:focus-visible ~ .bee-tabs__nav label[for="bee-tab-activity"],
#bee-tab-todos:focus-visible ~ .bee-tabs__nav label[for="bee-tab-todos"],
#bee-tab-terminal:focus-visible ~ .bee-tabs__nav label[for="bee-tab-terminal"] {{
  outline: var(--focus-width) solid var(--focus-color);
  outline-offset: var(--focus-offset);
}}
.bee-activity {{ display: flex; flex-direction: column; gap: var(--space-3); }}
.bee-activity__gates {{ display: flex; flex-wrap: wrap; gap: var(--space-1); }}
.bee-activity__timeline {{ display: flex; flex-direction: column; gap: var(--space-2); }}
.bee-activity__item {{ padding: var(--space-2); gap: var(--space-1); }}
.bee-activity__ts {{ color: var(--color-text-subtle); font-size: var(--type-caption-size); }}
.bee-todos {{ list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }}
.bee-todo {{ display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); padding: var(--space-2); border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--card-radius); background: var(--color-surface); }}
.bee-todo a {{ display: flex; align-items: center; gap: var(--space-2); flex: 1; min-width: 0; color: var(--color-text); text-decoration: none; }}
.bee-todo__mark {{ flex: none; width: 20px; text-align: center; color: var(--color-text-subtle); }}
.bee-todo__title {{ overflow-wrap: anywhere; }}
.bee-todo--done .bee-todo__title {{ text-decoration: line-through; color: var(--color-text-subtle); }}
.bee-todo--done .bee-todo__mark {{ color: var(--color-success); }}
.bee-todo--blocked .bee-todo__mark {{ color: var(--color-danger); }}
.bee-todo--blocked {{ border-color: var(--color-danger); }}
.bee-todo__badge {{ flex: none; }}
.bee-terminal-panes {{ display: flex; flex-direction: column; gap: var(--space-2); }}
.bee-terminal-pane {{ text-decoration: none; }}
/* Narrow-screen pass (bbp-17): every multi-column grid this board declares
   collapses to one column below this breakpoint (matches the sidebar
   breakpoint in app.css) so a phone never needs the page itself to scroll
   sideways — a genuinely wide container (the agent board's columns) keeps
   its own `overflow-x` above instead of forcing the page wider. */
@media (max-width: 700px) {{
  .bee-hub__groups,
  .bee-panels,
  .bee-done-grid {{
    grid-template-columns: 1fr;
  }}
}}
</style>"#
    )
}

/// The board's own header: the project name and when this snapshot was
/// read. board-declutter retires the rest of what this used to carry — the
/// lifecycle stepper, the headline KPI tiles, and the working-on-now/
/// needs-attention row — leaving this a plain page header, immediately
/// followed by [`bee_feature_hub_section`] as the page's first main
/// section.
fn bee_board_top(project: &Project) -> String {
    format!(
        r#"<div class="fg-pagehead">
    <h2 class="fg-pagehead__title">{name}</h2>
    <div class="fg-pagehead__aside"><span class="bee-asof">Read {asof}</span></div>
  </div>"#,
        name = esc(&project.name),
        asof = esc(&bee_board_asof()),
    )
}

/// "Read <UTC timestamp>" for the header line. This is this view's own
/// clock, taken at render time — the board is rendered fresh from disk on
/// every request (D4), never cached, so "when the data was read" and "now"
/// are the same instant. Formatted the same plain way `ymd_utc`
/// (`mdview_core::bee`) builds a date, just with the time appended.
fn bee_board_asof() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC",
        year = now.year(),
        month = now.month() as u8,
        day = now.day(),
        hour = now.hour(),
        minute = now.minute(),
    )
}

/// D1's feature-centric grouped list (fh-1), replacing the retired
/// cell-centric Kanban board (`bee_agent_board_section`, agent-board
/// ab-1/ab-2 — every card it rendered, and the five-column shape itself,
/// is gone). The card unit is now the FEATURE, not the cell: every feature
/// this snapshot can place (`phase_board`'s lanes ∪ active-feature union,
/// plus any feature whose cells have moved entirely to
/// `.bee/cells/archive/` with no lane of its own) renders in exactly one
/// of three groups — Waiting on you, In Progress, Finished — never two,
/// never a duplicate. Group membership is checked in that fixed priority
/// order (D4's "waiting wins over in-progress; finished only when no live
/// cells"):
///
/// - **Waiting on you**: a feature with live work — `doing`/`waiting`/
///   `stuck` cells present, or this is `state.feature`, the globally
///   active one, even with none yet — whose current-stop gate
///   ([`bee_gate_current_stop`], reused from the retired Review column:
///   the independent-review gate itself never counts, since that gate is
///   user-invoked on its own schedule, never a blocking stop) is still
///   unapproved, OR the active feature while `.bee/HANDOFF.json` reads as
///   a genuine pause (never a `"planned-next"` clean stop) — the note
///   carries no feature name of its own (`compute_attention_items`'s own
///   doc comment says so), so it is folded onto whichever feature
///   `state.json` currently names active. Either pull yields to Finished
///   when the feature has no live cells left: a closed feature owes no
///   decision, and `state.feature` keeps naming it long after its last
///   cell was archived.
/// - **In Progress**: everything left with `doing`/`waiting`/`stuck`
///   cells — live work not already claimed by Waiting.
/// - **Finished**: everything left with no live cells AND either a lane
///   `phase` of exactly `"compounding-complete"` (bee's own terminal
///   phase — `"terminal"` is a string bee never writes) OR a
///   `.bee/cells/archive/<feature>/` directory of its own
///   (`list_archived_feature_dirs`, checked once up front and reused as a
///   set — no extra store read per feature), including every feature that
///   directory names but never had a lane or active-feature placement at
///   all. Both sourced from `read_archived_cells` for their own
///   done/total counts and last activity, since a finished feature's live
///   `cell_counts` are typically zero (its cells already moved to
///   archive). A feature that fits neither rule (a pre-build, zero-cell
///   lane, e.g. still `exploring`) renders nowhere on this list — the
///   pre-redesign board never showed it either, since it never held a
///   cell of its own.
///
/// This is also D4's ghost-card fix: the retired Review column rendered a
/// card for ANY phase_board feature sitting on an unapproved gate,
/// regardless of whether it had any live cells left — six merged, fully
/// archived lanes kept showing "gate awaiting your decision" for that
/// reason. Gating Waiting on live work closes that permanently; a stale
/// lane with zero live cells now renders in Finished once its own `phase`
/// reaches `"compounding-complete"` or its cells land in the archive
/// directory (an orchestrator-run cleanup, out of this cell's scope), and
/// nowhere until then — never a ghost.
///
/// Every card names its feature, links to its own detail page, its own
/// done/total cell progress, its own last-activity age
/// ([`bee_fmt_trace_time`]), a worktree-state chip
/// ([`bee_hub_worktree_chip`]) and a status chip naming its own group.
/// Every path-shaped value a `BeeCell`/`BeeFeaturePhase` carries already
/// arrives relativized by `mdview_core::bee::read_snapshot` (D9), so
/// nothing further is redacted here — this view only escapes for HTML
/// safety.
fn bee_feature_hub_section(project: &Project, snapshot: &BeeSnapshot) -> String {
    let active_feature = snapshot.state.as_ref().and_then(|s| s.feature.as_deref());
    let handoff_is_pause = snapshot
        .handoff
        .as_ref()
        .map(|h| !matches!(h.kind.as_deref(), Some("planned-next")))
        .unwrap_or(false);

    let mut waiting_cards = String::new();
    let mut in_progress_cards = String::new();
    let mut finished_cards = String::new();
    let mut waiting_count = 0usize;
    let mut in_progress_count = 0usize;
    let mut finished_count = 0usize;
    let mut placed: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let archived_features: std::collections::HashSet<String> =
        list_archived_feature_dirs(&project.root_path).into_iter().collect();

    let mut features: Vec<&BeeFeaturePhase> = snapshot.phase_board.iter().collect();
    features.sort_by(|a, b| a.feature.cmp(&b.feature));

    for f in features {
        placed.insert(f.feature.as_str());
        let live = f.cell_counts.doing + f.cell_counts.waiting + f.cell_counts.stuck;
        let is_active = active_feature == Some(f.feature.as_str());
        let has_live_work = live > 0 || is_active;

        let gate_stop =
            bee_gate_current_stop(f.approved_gates.as_ref()).filter(|(key, _)| *key != "review");
        let waiting_via_handoff = is_active && handoff_is_pause;
        // A feature whose work already finished is never waiting on
        // anyone. Its gates stopped mattering when its cells reached the
        // archive, and the pause handoff — folded onto `state.feature`,
        // which keeps naming a feature long after it closed — records
        // where a session stopped, not a decision still owed. Only a live
        // cell pulls a finished feature back out of Finished.
        let is_finished = f.phase.as_deref() == Some("compounding-complete")
            || archived_features.contains(f.feature.as_str());
        let finished_and_idle = is_finished && live == 0;

        if !finished_and_idle && ((has_live_work && gate_stop.is_some()) || waiting_via_handoff) {
            waiting_count += 1;
            let reason = match gate_stop {
                Some((_, label)) => format!("{label} gate awaiting your decision"),
                None => "Work is parked, waiting on your decision".to_string(),
            };
            let last_activity = bee_hub_latest_activity(bee_hub_feature_cells(&snapshot.buckets, &f.feature));
            let worktree = bee_hub_worktree_chip(&f.feature, &snapshot.worktrees, &snapshot.workspaces, false);
            let docs = snapshot.feature_docs.get(f.feature.as_str());
            waiting_cards.push_str(&bee_hub_card(
                &project.id,
                &f.feature,
                "waiting",
                f.cell_counts.done,
                f.cell_counts.total,
                last_activity.as_deref(),
                &worktree,
                Some(&reason),
                docs,
            ));
        } else if live > 0 {
            in_progress_count += 1;
            let last_activity = bee_hub_latest_activity(bee_hub_feature_cells(&snapshot.buckets, &f.feature));
            let worktree = bee_hub_worktree_chip(&f.feature, &snapshot.worktrees, &snapshot.workspaces, false);
            let docs = snapshot.feature_docs.get(f.feature.as_str());
            in_progress_cards.push_str(&bee_hub_card(
                &project.id,
                &f.feature,
                "in-progress",
                f.cell_counts.done,
                f.cell_counts.total,
                last_activity.as_deref(),
                &worktree,
                None,
                docs,
            ));
        } else if is_finished {
            finished_count += 1;
            let archived = read_archived_cells(&project.root_path, &f.feature);
            let (done, total) = bee_hub_archived_counts(&archived);
            let last_activity = bee_hub_latest_activity(archived.iter());
            let worktree = bee_hub_worktree_chip(&f.feature, &snapshot.worktrees, &snapshot.workspaces, true);
            let docs = snapshot.feature_docs.get(f.feature.as_str());
            finished_cards.push_str(&bee_hub_card(
                &project.id,
                &f.feature,
                "finished",
                done,
                total,
                last_activity.as_deref(),
                &worktree,
                None,
                docs,
            ));
        }
        // else: no live work, no gate/handoff pull, and neither
        // `compounding-complete` nor archived — a pre-build lane (still
        // `exploring`, no cells yet). Renders nowhere, matching the
        // pre-redesign board's own cell-only precedent.
    }

    let mut archive_only: Vec<String> = archived_features
        .into_iter()
        .filter(|name| !placed.contains(name.as_str()))
        .collect();
    archive_only.sort();
    for feature in archive_only {
        finished_count += 1;
        let archived = read_archived_cells(&project.root_path, &feature);
        let (done, total) = bee_hub_archived_counts(&archived);
        let last_activity = bee_hub_latest_activity(archived.iter());
        let worktree = bee_hub_worktree_chip(&feature, &snapshot.worktrees, &snapshot.workspaces, true);
        let docs = snapshot.feature_docs.get(feature.as_str());
        finished_cards.push_str(&bee_hub_card(
            &project.id,
            &feature,
            "finished",
            done,
            total,
            last_activity.as_deref(),
            &worktree,
            None,
            docs,
        ));
    }

    format!(
        r#"<section class="fg-card bee-hub" data-feature-hub="1">
  <h3 class="bee-panel__head">Features</h3>
  <div class="bee-hub__groups">
    {waiting_group}
    {in_progress_group}
    {finished_group}
  </div>
</section>"#,
        waiting_group = bee_hub_group(
            "Waiting on you",
            "waiting",
            waiting_count,
            &waiting_cards,
            "Nothing waiting on you."
        ),
        in_progress_group = bee_hub_group(
            "In Progress",
            "in-progress",
            in_progress_count,
            &in_progress_cards,
            "Nothing in progress."
        ),
        finished_group = bee_hub_group(
            "Finished",
            "finished",
            finished_count,
            &finished_cards,
            "Nothing finished yet."
        ),
    )
}

/// Every cell in the board's four D7 buckets belonging to one feature —
/// `bee_feature_hub_section`'s own narrowing of `snapshot.buckets`, used
/// only to find that feature's own most recent `claimed_at`/`capped_at`
/// ([`bee_hub_latest_activity`]); it never changes which bucket a cell
/// belongs to.
fn bee_hub_feature_cells<'a, 'b>(
    buckets: &'a BeeBuckets,
    feature: &'b str,
) -> impl Iterator<Item = &'a BeeCell> + use<'a, 'b> {
    buckets
        .doing
        .iter()
        .chain(buckets.waiting.iter())
        .chain(buckets.stuck.iter())
        .chain(buckets.done.iter())
        .filter(move |c| c.feature == feature)
}

/// The first gate in bee's fixed order (context, shape, execution, review)
/// that is not yet approved for one `approved_gates` record — `None` once
/// every gate is approved. Applied here to a feature's own current-stop gate
/// ([`bee_feature_hub_section`]'s Waiting on you group).
fn bee_gate_current_stop(gates: Option<&BeeApprovedGates>) -> Option<(&'static str, &'static str)> {
    const GATES: [(&str, &str); 4] = [
        ("context", "Explore"),
        ("shape", "Shape"),
        ("execution", "Execute"),
        ("review", "Independent review"),
    ];
    let flag = |key: &str| -> bool {
        gates
            .and_then(|g| match key {
                "context" => g.context,
                "shape" => g.shape,
                "execution" => g.execution,
                "review" => g.review,
                _ => None,
            })
            .unwrap_or(false)
    };
    GATES.into_iter().find(|(key, _)| !flag(key))
}

/// One group column of the feature hub (`bee_feature_hub_section`): a
/// header naming the group and its true count, then its cards, or one
/// honest empty line when the group holds nothing right now (bee-board-pm
/// D5's "sections never disappear" rule) — an empty group renders its own
/// wording, never a shared "Nothing here." that could not tell a reader
/// which group came up empty.
fn bee_hub_group(label: &str, key: &str, count: usize, cards_html: &str, empty_line: &str) -> String {
    let body = if cards_html.is_empty() {
        format!(r#"<p class="fg-empty">{}</p>"#, esc(empty_line))
    } else {
        format!(r#"<div class="bee-hub__cards">{cards_html}</div>"#, cards_html = cards_html)
    };
    format!(
        r#"<div class="bee-hub__group" data-hub-group="{key}" data-hub-count="{count}"><h4 class="bee-panel__subhead">{label} <span class="fg-chip fg-chip--neutral">{count}</span></h4>{body}</div>"#,
        key = key,
        count = count,
        label = esc(label),
        body = body,
    )
}

/// A group key's own status chip tone and label — the card's D1 "status
/// icon", rendered as the same `fg-chip` pattern every other status chip on
/// this board already uses rather than a bespoke icon set.
fn bee_hub_group_label(key: &str) -> (&'static str, &'static str) {
    match key {
        "waiting" => ("Waiting on you", "warning"),
        "in-progress" => ("In progress", "info"),
        _ => ("Finished", "success"),
    }
}

/// One feature card (D1): name + link to its own detail page, its own
/// done/total cell progress (a `bee-progress` bar), its own last-activity age
/// ([`bee_fmt_trace_time`]), its own worktree-state chip
/// ([`bee_hub_worktree_chip`]) and its own group status chip
/// ([`bee_hub_group_label`]). `reason` carries the Waiting group's own
/// "why" line (its current-stop gate, or a paused handoff) — `None` for
/// every other group, which has no such single reason to name. `docs`
/// (feature-titles) carries this feature's own `CONTEXT.md` reader result:
/// present with a title, the card's name becomes that human title with the
/// slug demoted to a small muted subtitle beneath it, plus the boundary
/// description as one clamped line; `None`, or a title-less record, falls
/// back to the slug alone, exactly as before this feature.
fn bee_hub_card(
    project_id: &str,
    feature: &str,
    group_key: &str,
    done: usize,
    total: usize,
    last_activity: Option<&str>,
    worktree: &(String, &'static str),
    reason: Option<&str>,
    docs: Option<&mdview_core::bee::BeeFeatureDocs>,
) -> String {
    let (group_label, group_tone) = bee_hub_group_label(group_key);
    let title = docs.and_then(|d| d.title.as_deref()).filter(|t| !t.is_empty());
    let name_html = match title {
        Some(t) => format!(
            r#"<div class="fg-card__title">{title}</div><div class="bee-hub__slug">{feature}</div>"#,
            title = esc(t),
            feature = esc(feature),
        ),
        None => format!(r#"<div class="fg-card__title">{feature}</div>"#, feature = esc(feature)),
    };
    let desc_html = match docs.and_then(|d| d.description.as_deref()).filter(|d| !d.is_empty()) {
        Some(d) => format!(r#"<p class="bee-hub__desc">{}</p>"#, esc(d)),
        None => String::new(),
    };
    let progress_html = if total == 0 {
        r#"<p class="fg-empty">No cells recorded.</p>"#.to_string()
    } else {
        let percent = (done * 100) / total;
        format!(
            r#"<div class="bee-progress"><div class="bee-progress__bar" style="width: {percent}%"></div></div><p class="bee-hub__progress-label">{done}/{total} cell{plural} done</p>"#,
            percent = percent,
            done = done,
            total = total,
            plural = if total == 1 { "" } else { "s" },
        )
    };
    let activity_html = match last_activity {
        Some(iso) => format!(
            r#"<p class="bee-cell__meta">Last activity {}</p>"#,
            esc(&bee_fmt_trace_time(iso))
        ),
        None => r#"<p class="bee-cell__meta">No activity recorded.</p>"#.to_string(),
    };
    let reason_html = match reason {
        Some(r) if !r.is_empty() => format!(r#"<p class="bee-cell__meta bee-hub__reason">{}</p>"#, esc(r)),
        _ => String::new(),
    };
    let (wt_label, wt_tone) = worktree;
    format!(
        r#"<a class="fg-card bee-hub__card" data-hub-group="{group_key}" href="/p/{pid}/_bee/feature/{feature_href}">{name_html}<div class="bee-hub__chips"><span class="fg-chip fg-chip--{group_tone}">{group_label}</span><span class="fg-chip fg-chip--{wt_tone}">{wt_label}</span></div>{desc_html}{progress_html}{reason_html}{activity_html}</a>"#,
        group_key = group_key,
        pid = esc(project_id),
        feature_href = esc(feature),
        name_html = name_html,
        group_tone = group_tone,
        group_label = group_label,
        wt_tone = wt_tone,
        wt_label = esc(wt_label),
        desc_html = desc_html,
        progress_html = progress_html,
        reason_html = reason_html,
        activity_html = activity_html,
    )
}

/// A card's worktree-state chip (D1), read from `snapshot.worktrees`
/// (`.bee/runtime/worktree-grants.json`, resolved against each grant's own
/// sibling `.bee/state.json`): "Open · &lt;branch&gt;" when a currently
/// granted worktree names this feature as its own active one.
///
/// A grant is released on `bee worktree merge` (AGENTS.md) — cleanup drops
/// the worktree directory, its branch, and the grant itself, but (unlike
/// `bee worktree prune`) never the sibling `.bee/runtime/workspaces/<id>.json`
/// record `snapshot.workspaces` already carries. So an absent grant reads
/// two ways, and this is never a guess beyond what that already-read record
/// still shows: when a workspace record survives whose own `branch` matches
/// this feature's `wt/<feature>` convention (`bee worktree new`'s own
/// `-b wt/<feature>`), a grant for this feature genuinely existed and is
/// now gone — `finished` reads that as "Merged" (the worktree that did this
/// work has been folded back). Every other absent-grant case — no grant,
/// and no leftover workspace record either — reads "Main" regardless of
/// `finished`: a feature with no grant history was never worked in its own
/// worktree at all (the tiny/solo-fix path AGENTS.md itself names), so
/// "Merged" would be a fabricated history this project's own store never
/// recorded.
fn bee_hub_worktree_chip(
    feature: &str,
    worktrees: &[BeeWorktree],
    workspaces: &[BeeWorkspace],
    finished: bool,
) -> (String, &'static str) {
    if let Some(w) = worktrees.iter().find(|w| w.feature.as_deref() == Some(feature)) {
        let label = match w.branch.as_deref() {
            Some(b) if !b.is_empty() => format!("Open · {b}"),
            _ => "Open worktree".to_string(),
        };
        return (label, "info");
    }
    let feature_branch = format!("wt/{feature}");
    let grant_existed = workspaces.iter().any(|w| w.branch.as_deref() == Some(feature_branch.as_str()));
    if finished && grant_existed {
        ("Merged".to_string(), "success")
    } else {
        ("Main".to_string(), "neutral")
    }
}

/// A feature's most recent `claimed_at`/`capped_at` across the cells
/// handed to it (`bee_hub_feature_cells` for a live feature,
/// `read_archived_cells` for a finished one) — the later of the two RFC
/// 3339 timestamps a cell carries, across every cell in the slice; `None`
/// when none of them parse or the slice is empty, never a fabricated
/// "just now".
fn bee_hub_latest_activity<'a>(cells: impl Iterator<Item = &'a BeeCell>) -> Option<String> {
    let mut latest: Option<(time::OffsetDateTime, String)> = None;
    for c in cells {
        for ts in [c.claimed_at.as_deref(), c.capped_at.as_deref()].into_iter().flatten() {
            if let Ok(t) = time::OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339) {
                let newer = latest.as_ref().map(|(lt, _)| t > *lt).unwrap_or(true);
                if newer {
                    latest = Some((t, ts.to_string()));
                }
            }
        }
    }
    latest.map(|(_, s)| s)
}

/// D7-style done/total over one feature's archived cells
/// (`read_archived_cells`) — the same bucket rule `read_snapshot`'s own D7
/// buckets and `compute_feature_cell_counts` (`mdview_core::bee`) already
/// apply: `dropped` and any unrecognized status count toward neither
/// `done` nor `total`, so a fully-dropped archive reports an honest
/// `(0, 0)` rather than a fabricated complete or a division by zero.
fn bee_hub_archived_counts(cells: &[BeeCell]) -> (usize, usize) {
    let mut done = 0usize;
    let mut total = 0usize;
    for c in cells {
        match c.status.as_str() {
            "capped" => {
                done += 1;
                total += 1;
            }
            "claimed" | "open" | "blocked" => total += 1,
            _ => {}
        }
    }
    (done, total)
}

/// The board's Finished list (D5/D10), rendered as a native
/// `<details>`/`<summary>` element that is collapsed by default — no `open`
/// attribute, no JavaScript. This is the board's only place FEATURE-level
/// finished work is listed — a feature name renders here exactly once, never
/// twice (a capped cell belonging to that feature separately renders in the
/// agent board's Done column, a cell-level fact this list never duplicates
/// or excludes for). Built over
/// `snapshot.shipped` (D10: every non-dropped cell capped) rather than a
/// cell-status bucket, so it is inherently D8-safe and already uncapped —
/// `compute_shipped_features` (`mdview_core::bee`) applies no
/// `RECENT_DETAIL_CAP`, so no finished feature is ever silently dropped.
/// Grouped one compact line per feature — name, cell count and, when the
/// feature shipped with a timed cycle (D10/D11), its time to finish, reused
/// from `shipped` rather than recomputed here — never one card per cell. The
/// `<summary>` states the true totals (finished feature count, finished cell
/// count) in plain language even while collapsed, so the page never
/// understates the store just because the list is closed. An empty finished
/// list is a plain line, never a zeroed collapsible list.
fn bee_finished_section(project_id: &str, shipped: &[BeeShippedFeature]) -> String {
    let feature_total = shipped.len();
    if feature_total == 0 {
        return r#"<section class="fg-card bee-finished" data-finished-features="0">
  <h3 class="bee-panel__head">Finished</h3>
  <p class="fg-empty">Nothing finished yet.</p>
</section>"#
            .to_string();
    }

    let cell_total: usize = shipped.iter().map(|f| f.cell_count).sum();

    let mut lines = String::new();
    for f in shipped {
        let cycle = match &f.cycle_time {
            Some(span) if span.hours.is_finite() => Some(format!("{:.1}h to finish", span.hours)),
            _ => None,
        };
        let meta = match cycle {
            Some(c) => format!(
                "{count} cell{plural} · {c}",
                count = f.cell_count,
                plural = if f.cell_count == 1 { "" } else { "s" },
                c = c,
            ),
            None => format!(
                "{count} cell{plural}",
                count = f.cell_count,
                plural = if f.cell_count == 1 { "" } else { "s" },
            ),
        };
        lines.push_str(&format!(
            r#"<a class="bee-done-line" href="/p/{pid}/_bee/feature/{feature_href}">{feature} · {meta}</a>"#,
            pid = esc(project_id),
            feature_href = esc(&f.feature),
            feature = esc(&f.feature),
            meta = esc(&meta),
        ));
    }

    let summary = format!(
        "Shipped: {feature_total} feature{fplural} finished · {cell_total} cell{plural} total",
        feature_total = feature_total,
        fplural = if feature_total == 1 { "" } else { "s" },
        cell_total = cell_total,
        plural = if cell_total == 1 { "" } else { "s" },
    );

    format!(
        r#"<section class="fg-card bee-finished" data-finished-features="{feature_total}" data-finished-cells="{cell_total}"><details class="bee-done-details"><summary class="bee-done-summary">{summary}</summary><div class="bee-done-grid">{lines}</div></details></section>"#,
        feature_total = feature_total,
        cell_total = cell_total,
        summary = esc(&summary),
        lines = lines,
    )
}

/// Backlog & review panel (bee-cockpit-6, board-trim), rendered below the
/// board's Finished list on the same page (D4/D1). Pure formatting over
/// `BeeSnapshot` — every field already arrived relativized/redacted from
/// `mdview_core::bee::read_snapshot`, so this view only formats what it is
/// handed, never recomputes any of that logic. This wrapper used to carry
/// two more cards, the Sessions panel (bbp-16/D2: sessions, worktrees,
/// workspaces) and Process health (bbp-16/D5: file-lock contention, model
/// tier mix, gate bypass, `read_errors`) — board-trim (D1) drops both,
/// along with the view functions and CSS that only ever rendered them.
/// `mdview_core`'s readers for that dropped data are untouched; only this
/// page's rendering of them is gone.
fn bee_panels_section(snapshot: &BeeSnapshot) -> String {
    format!(
        r#"<div class="bee-panels">
    {backlog}
  </div>"#,
        backlog = bee_backlog_panel(&snapshot.backlog, &snapshot.review),
    )
}

/// Backlog & review panel (bbp-14): PBI items grouped by current status —
/// each item's own escaped title alongside the status counts, so a manager
/// reads not just how many are proposed or in flight but WHAT they are —
/// findings grouped by severity with the P1 count visually weighted
/// (`bee-severity--p1`) since a P1 blocks, and the review queue by state
/// (D7: independent review is presented as owner-invoked, never as pending
/// automatic work — see [`bee_review_queue_body`]). `findings.recent` is a
/// bounded slice of `findings.total` (`RECENT_DETAIL_CAP` in
/// `mdview_core::bee`) — when it is showing fewer than the true total, the
/// panel says so instead of looking smaller than the real backlog. The PBI
/// card list beneath the status chips shows only OPEN work — every status
/// except `done` and `declined` — since those two are already reflected in
/// the chip counts and would otherwise bury the items still worth reading;
/// the chips themselves keep counting the WHOLE backlog, unfiltered. Each
/// card with a non-empty `cos` (condition-of-satisfaction detail) tucks it
/// into a `<details>` expander so the default view stays scannable. The open
/// list is bounded the same way findings are, at [`BACKLOG_PBI_DISPLAY_CAP`]
/// — a live store the size of `beehive`'s (123 PBIs) turned an early,
/// uncapped draft of this list into exactly the "per-item dump" the status
/// chips exist to avoid; capping it, and stating its true total (of OPEN
/// items) alongside the visible subset, is what keeps this a supporting
/// panel rather than a second scroll of the whole backlog. An empty PBI
/// list, a backlog with no open items, and an empty finding set each render
/// their own honest empty state rather than a hidden section or a bare `0`.
/// How many PBI cards the backlog panel shows before it falls back to a
/// "Showing X of Y" note (bbp-14) — the same cap discipline
/// `mdview_core::bee`'s own `RECENT_DETAIL_CAP` already applies to findings,
/// mirrored here at the view layer since `BeeBacklog::pbis` itself is
/// uncapped (every distinct PBI, so the status counts stay exact).
const BACKLOG_PBI_DISPLAY_CAP: usize = 20;

fn bee_backlog_panel(backlog: &BeeBacklog, review: &BeeReview) -> String {
    let pbi_body = if backlog.pbis.is_empty() {
        "<p class=\"fg-empty\">No backlog items yet.</p>".to_string()
    } else {
        // Status chips always count the WHOLE backlog (done and declined
        // included) so the counts stay exact even though the card list
        // below only shows the open subset.
        let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for pbi in &backlog.pbis {
            *counts.entry(pbi.status.as_str()).or_insert(0) += 1;
        }
        let chips: String = counts
            .iter()
            .map(|(status, count)| {
                format!(
                    r#"<span class="fg-chip fg-chip--neutral">{status}: {count}</span>"#,
                    status = esc(status),
                    count = count,
                )
            })
            .collect();
        // The card list only shows OPEN work (bbp-open-detail): "done" and
        // "declined" PBIs are already reflected in the chips above and
        // would otherwise crowd out the items still worth reading about.
        let open: Vec<&BeePbi> = backlog
            .pbis
            .iter()
            .filter(|pbi| pbi.status != "done" && pbi.status != "declined")
            .collect();
        let total = open.len();
        let shown = open.iter().take(BACKLOG_PBI_DISPLAY_CAP);
        let mut rows = String::new();
        let mut shown_count = 0usize;
        for pbi in shown {
            shown_count += 1;
            let detail = if pbi.cos.trim().is_empty() {
                String::new()
            } else {
                format!(
                    r#"<details class="bee-cell__detail"><summary>Condition of satisfaction</summary><p>{cos}</p></details>"#,
                    cos = esc(&pbi.cos),
                )
            };
            rows.push_str(&format!(
                r#"<div class="fg-card bee-cell"><div class="fg-card__title">{title}</div><div class="bee-cell__meta">{status} · {feature}</div>{detail}</div>"#,
                title = esc(&pbi.title),
                status = esc(&pbi.status),
                feature = esc(&pbi.feature),
                detail = detail,
            ));
        }
        let list_note = if total == 0 {
            "<p class=\"fg-empty\">No open backlog items.</p>".to_string()
        } else if shown_count < total {
            format!(
                r#"<p class="bee-cell__meta">Showing {shown_count} of {total} backlog items.</p>"#,
                shown_count = shown_count,
                total = total,
            )
        } else {
            format!(
                r#"<p class="bee-cell__meta">{total} backlog item{plural} total.</p>"#,
                total = total,
                plural = if total == 1 { "" } else { "s" },
            )
        };
        format!(
            r#"<div class="bee-panel__chips">{chips}</div>{list_note}<div class="bee-panel__list">{rows}</div>"#,
            chips = chips,
            list_note = list_note,
            rows = rows,
        )
    };

    let findings = &backlog.findings;
    let findings_body = if findings.total == 0 {
        "<p class=\"fg-empty\">No findings yet.</p>".to_string()
    } else {
        let sev = &findings.by_severity;
        let sev_chips = format!(
            r#"<span class="fg-chip fg-chip--danger bee-severity--p1">P1: {p1}</span><span class="fg-chip fg-chip--neutral">P2: {p2}</span><span class="fg-chip fg-chip--neutral">P3: {p3}</span>"#,
            p1 = sev.p1,
            p2 = sev.p2,
            p3 = sev.p3,
        );
        let recent_note = if findings.recent.len() < findings.total {
            format!(
                r#"<p class="bee-cell__meta">Showing {shown} of {total} findings.</p>"#,
                shown = findings.recent.len(),
                total = findings.total,
            )
        } else {
            format!(
                r#"<p class="bee-cell__meta">{total} finding{plural} total.</p>"#,
                total = findings.total,
                plural = if findings.total == 1 { "" } else { "s" },
            )
        };
        let mut rows = String::new();
        for f in &findings.recent {
            rows.push_str(&format!(
                r#"<div class="fg-card bee-cell"><div class="fg-card__title">{title}</div><div class="bee-cell__meta">{severity} · {feature}</div></div>"#,
                title = esc(&f.title),
                severity = esc(&f.severity),
                feature = esc(&f.feature),
            ));
        }
        format!(
            r#"<div class="bee-panel__chips">{sev_chips}</div>{recent_note}<div class="bee-panel__list">{rows}</div>"#,
            sev_chips = sev_chips,
            recent_note = recent_note,
            rows = rows,
        )
    };

    let review_body = bee_review_queue_body(review);

    format!(
        r#"<section class="fg-card bee-panel">
  <h3 class="bee-panel__head">Backlog &amp; Review</h3>
  <h4 class="bee-panel__subhead">PBIs by status</h4>
  {pbi_body}
  <h4 class="bee-panel__subhead">Findings by severity</h4>
  {findings_body}
  <h4 class="bee-panel__subhead">Review queue by state</h4>
  {review_body}
</section>"#,
        pbi_body = pbi_body,
        findings_body = findings_body,
        review_body = review_body,
    )
}

/// The review queue's body (bbp-14, D6, D7): unreviewed / in review /
/// settled counts, joined from `.bee/review-candidates.jsonl` against
/// `.bee/reviews/*.json` by `mdview_core::bee`'s own review join, with the
/// open-P1 count called out first as the sharpest number on the panel.
/// Independent review is presented as something the owner invokes, never as
/// a stage the board implies is already running — every sentence here is
/// worded that way, matching the lifecycle stepper's own D7 wording.
///
/// A candidate list of zero is genuinely ambiguous by itself: it is the
/// shape both of "this project has never run a review" and of "everything
/// has already been folded and the candidates file has rolled over" — the
/// snapshot cannot tell those two claims apart from `review.candidates`
/// alone, so rendering `0/0/0` here would be exactly the zero-dressed-as-a-
/// measurement mistake D5's honest-empty-state rule forbids elsewhere. The
/// panel says its state is unknown instead. Once there is at least one
/// candidate, every count below is real and computed — including a store
/// whose candidates are ALL `Unreviewed` because no session has ever named
/// their cells, which is a genuine zero for `In review`/`Settled`, not a
/// manufactured one.
fn bee_review_queue_body(review: &BeeReview) -> String {
    if review.candidates.is_empty() {
        return r#"<p class="fg-empty">Review state unknown — no review candidates or sessions are recorded yet. Independent review is invoked by the owner; it is never presented as work already pending.</p>"#
            .to_string();
    }

    let mut unreviewed = 0usize;
    let mut in_review = 0usize;
    let mut settled = 0usize;
    for c in &review.candidates {
        match c.status {
            BeeReviewStatus::Unreviewed => unreviewed += 1,
            BeeReviewStatus::InReview => in_review += 1,
            BeeReviewStatus::Settled => settled += 1,
        }
    }

    let p1_line = if review.open_p1_findings > 0 {
        let n = review.open_p1_findings;
        format!(
            r#"<p class="bee-cell__meta bee-severity--p1"><strong>{n} open P1 finding{plural}</strong> in a review session not yet settled.</p>"#,
            n = n,
            plural = if n == 1 { "" } else { "s" },
        )
    } else {
        r#"<p class="bee-cell__meta">No open P1 findings.</p>"#.to_string()
    };

    format!(
        r#"{p1_line}<div class="bee-panel__chips"><span class="fg-chip fg-chip--neutral">Unreviewed: {unreviewed}</span><span class="fg-chip fg-chip--neutral">In review: {in_review}</span><span class="fg-chip fg-chip--neutral">Settled: {settled}</span></div><p class="bee-cell__meta">Independent review is invoked by the owner — nothing here runs on its own.</p>"#,
        p1_line = p1_line,
        unreviewed = unreviewed,
        in_review = in_review,
        settled = settled,
    )
}

/// A signed minute count, rendered as plain relative language ("4 minutes
/// ago", "2 hours ago") — used by `bee_fmt_trace_time` (a cell's
/// `claimed_at`/`capped_at`) and the worktree chip's own heartbeat age. A
/// negative age (somehow in the future) reads as "just now" rather than a
/// confusing negative duration; a non-finite value reads "unknown" rather
/// than crashing the format.
fn bee_relative_minutes(minutes: f64) -> String {
    if !minutes.is_finite() {
        return "unknown".to_string();
    }
    let mins = minutes.max(0.0);
    if mins < 1.0 {
        "just now".to_string()
    } else if mins < 60.0 {
        let m = mins.round().max(1.0) as i64;
        format!("{m} minute{plural} ago", plural = if m == 1 { "" } else { "s" })
    } else if mins < 60.0 * 24.0 {
        let h = (mins / 60.0).round().max(1.0) as i64;
        format!("{h} hour{plural} ago", plural = if h == 1 { "" } else { "s" })
    } else {
        let d = (mins / (60.0 * 24.0)).round().max(1.0) as i64;
        format!("{d} day{plural} ago", plural = if d == 1 { "" } else { "s" })
    }
}

/// A cell trace timestamp (`claimed_at`/`capped_at`, an RFC 3339 string),
/// rendered as plain relative language via `bee_relative_minutes` — never
/// the raw ISO string. A value that fails to parse falls back to the raw
/// string itself rather than hiding it: an oddly-shaped-but-present
/// timestamp is still more useful than "unknown".
fn bee_fmt_trace_time(iso: &str) -> String {
    match time::OffsetDateTime::parse(iso, &time::format_description::well_known::Rfc3339) {
        Ok(t) => {
            let now = time::OffsetDateTime::now_utc();
            let minutes = (now - t).as_seconds_f64() / 60.0;
            bee_relative_minutes(minutes)
        }
        Err(_) => iso.to_string(),
    }
}

/// One `.bee/cells/<id>.json` cell in full — everything the board's trimmed
/// `mdview_core::bee::BeeCell` deliberately leaves out (`action`, `verify`,
/// `read_first`, `decisions`, `must_haves.truths`, and the rest of `trace`
/// beyond `worker`/`claimed_at`/`capped_at`). Built by
/// `server.rs::cell_full_from_json` straight from the raw cell JSON, with
/// every path-shaped field already relativized against the project root
/// before it reaches here (same contract as `mdview_core::bee::BeeCell`) —
/// this view only escapes for HTML safety, it never redacts.
pub struct BeeCellFull {
    pub id: String,
    pub feature: String,
    pub title: String,
    pub action: String,
    pub verify: String,
    pub lane: String,
    pub status: String,
    pub tier: Option<String>,
    /// Relative to the project root; never absolute.
    pub files: Vec<String>,
    /// Relative to the project root; never absolute.
    pub read_first: Vec<String>,
    pub decisions: Vec<String>,
    pub must_have_truths: Vec<String>,
    /// `trace.worker`, relativized if it happens to be path-shaped.
    pub worker: Option<String>,
    pub claimed_at: Option<String>,
    pub capped_at: Option<String>,
    pub outcome: Option<String>,
    pub deviations: Vec<String>,
    /// `trace.tests` — bee's own green/red verdict for the cell's `verify`.
    pub tests: Option<String>,
    /// `trace.results`, relativized if it happens to be path-shaped.
    pub results: Option<String>,
}

/// A status string's chip tone, matching the D7 tones `bee_todo_item` and
/// the rest of this board already use so a cell's status chip reads
/// consistently wherever it appears.
fn bee_status_tone(status: &str) -> &'static str {
    match status {
        "blocked" => "danger",
        "capped" => "success",
        _ => "neutral",
    }
}

/// The read-only cell detail page (D4): everything one cell carries, plus
/// its whole trace, reached by clicking any cell card on the board or a
/// feature page. `cell.feature` links back to that feature's own detail
/// page, closing the loop between the two drill-down routes.
pub fn bee_cell_page(project: &Project, cell: &BeeCellFull) -> String {
    let list_or_empty = |items: &[String], empty: &str| -> String {
        if items.is_empty() {
            format!("<p class=\"fg-empty\">{}</p>", esc(empty))
        } else {
            let lis: String = items.iter().map(|i| format!("<li>{}</li>", esc(i))).collect();
            format!("<ul>{lis}</ul>")
        }
    };

    let decisions = if cell.decisions.is_empty() {
        "<p class=\"fg-empty\">No decisions cited.</p>".to_string()
    } else {
        let chips: String = cell
            .decisions
            .iter()
            .map(|d| format!(r#"<span class="fg-chip fg-chip--neutral">{}</span>"#, esc(d)))
            .collect();
        format!(r#"<div class="bee-panel__chips">{chips}</div>"#)
    };

    let tier_chip = cell
        .tier
        .as_deref()
        .map(|t| format!(r#"<span class="fg-chip fg-chip--neutral">tier: {}</span>"#, esc(t)))
        .unwrap_or_default();

    let worker = cell.worker.as_deref().unwrap_or("—");
    let claimed = cell
        .claimed_at
        .as_deref()
        .map(bee_fmt_trace_time)
        .unwrap_or_else(|| "—".to_string());
    let capped = cell
        .capped_at
        .as_deref()
        .map(bee_fmt_trace_time)
        .unwrap_or_else(|| "not capped yet".to_string());
    let outcome = cell.outcome.as_deref().unwrap_or("—");
    let tests = cell.tests.as_deref().unwrap_or("—");
    let results = cell
        .results
        .as_deref()
        .map(|r| format!("<div class=\"bee-cell__meta\">results: {}</div>", esc(r)))
        .unwrap_or_default();

    let deviations = if cell.deviations.is_empty() {
        "<p class=\"fg-empty\">No deviations recorded.</p>".to_string()
    } else {
        let lis: String = cell
            .deviations
            .iter()
            .map(|d| format!("<li>{}</li>", esc(d)))
            .collect();
        format!("<ul>{lis}</ul>")
    };

    let body = format!(
        r#"{topbar}
<main class="fg-page">
  <div class="fg-pagehead">
    <h2 class="fg-pagehead__title">{title}</h2>
    <div class="fg-pagehead__aside">
      <span class="fg-chip fg-chip--{tone}">{status}</span>
      <span class="fg-chip fg-chip--neutral">lane: {lane}</span>
      {tier_chip}
    </div>
  </div>
  <p class="bee-cell__meta">{id} · feature: <a href="/p/{pid}/_bee/feature/{feature_href}">{feature}</a></p>

  <section class="fg-card bee-panel">
    <h3 class="bee-panel__head">Action</h3>
    <p>{action}</p>
  </section>

  <section class="fg-card bee-panel">
    <h3 class="bee-panel__head">Verify</h3>
    <p>{verify}</p>
  </section>

  <div class="bee-panels">
    <section class="fg-card bee-panel">
      <h3 class="bee-panel__head">Files</h3>
      {files}
    </section>
    <section class="fg-card bee-panel">
      <h3 class="bee-panel__head">Read first</h3>
      {read_first}
    </section>
    <section class="fg-card bee-panel">
      <h3 class="bee-panel__head">Decisions cited</h3>
      {decisions}
    </section>
    <section class="fg-card bee-panel">
      <h3 class="bee-panel__head">Must-haves</h3>
      {must_haves}
    </section>
  </div>

  <section class="fg-card bee-panel">
    <h3 class="bee-panel__head">Trace</h3>
    <div class="bee-panel__list">
      <div class="fg-card bee-cell"><div class="fg-card__title">Worker</div><div class="bee-cell__meta">{worker}</div></div>
      <div class="fg-card bee-cell"><div class="fg-card__title">Claimed</div><div class="bee-cell__meta">{claimed}</div></div>
      <div class="fg-card bee-cell"><div class="fg-card__title">Capped</div><div class="bee-cell__meta">{capped}</div></div>
      <div class="fg-card bee-cell"><div class="fg-card__title">Outcome</div><div class="bee-cell__meta">{outcome}</div></div>
      <div class="fg-card bee-cell"><div class="fg-card__title">Test result</div><div class="bee-cell__meta">{tests}</div>{results}</div>
    </div>
    <h4 class="bee-panel__subhead">Deviations</h4>
    {deviations}
  </section>
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name} · {id}</span>",
            name = esc(&project.name),
            id = esc(&cell.id),
        )),
        title = esc(&cell.title),
        tone = bee_status_tone(&cell.status),
        status = esc(&cell.status),
        lane = esc(&cell.lane),
        tier_chip = tier_chip,
        id = esc(&cell.id),
        pid = esc(&project.id),
        feature_href = esc(&cell.feature),
        feature = esc(&cell.feature),
        action = esc(&cell.action),
        verify = esc(&cell.verify),
        files = list_or_empty(&cell.files, "No files listed."),
        read_first = list_or_empty(&cell.read_first, "Nothing to read first."),
        decisions = decisions,
        must_haves = list_or_empty(&cell.must_have_truths, "No must-haves recorded."),
        worker = esc(worker),
        claimed = esc(&claimed),
        capped = esc(&capped),
        outcome = esc(outcome),
        tests = esc(tests),
        results = results,
        deviations = deviations,
    );
    layout(&format!("{} · {}", cell.id, project.name), "", &body)
}

/// The read-only feature detail page (D2/D4, feature-hub-2): a header
/// naming the feature and whether it has shipped (D10, cycle time D11) or
/// closed (archive-visibility), a chip row, and three CSS-only tabs —
/// Activity, Todos, Terminal. Reached from the feature hub's cards
/// (feature-hub-1) or from a cell page's feature link.
///
/// `buckets` already carries any archived cells the caller merged in
/// (archive-visibility) alongside the live ones — every tab below reads
/// from this one already-merged set, so an archived feature's page is as
/// fully populated as a live one's. `is_closed` is true when the feature
/// has no live open/claimed work left and at least one of its cells came
/// from the archive — distinct from `shipped` (D10), which only ever looks
/// at live cells and so reads `None` for a feature whose every cell has
/// moved to `archive/`. `is_closed` is ignored once `shipped` is `Some`.
///
/// `lane_label` is this feature's own route classification (`route.lane`)
/// read from its lane record, or from `state.json` when this is the
/// globally active feature with no lane record of its own — `None` when
/// neither source carries one. `worktrees` is the project's full granted-
/// worktree list and `workspaces` its full workspace-record list
/// (`bee_hub_worktree_chip` picks this feature's own entry from each —
/// `workspaces` is what lets a merged-and-gone grant still read "Merged"
/// rather than "Main", see that function's own doc comment).
/// `decisions` is already filtered to this feature's own `scope` by the
/// caller (`snapshot.decisions.recent`, itself bounded — see
/// `mdview_core::bee::BeeDecisions`). `panes` (feature-titles D2) is this
/// project's own D2-containment-boundary-filtered terminal pane list
/// (`server.rs::project_panes`, the same list `terminal_page` renders) —
/// the Terminal tab links each one straight to its own live page
/// (`terminal_page_for_pane`'s route) rather than re-implementing any of
/// the interactive surface here. `gates` is the same lane-record-or-active-state source `lane_label`
/// reads, for the Activity tab's gate stamps. `docs` (feature-titles,
/// extended by hub-fallbacks) is this feature's own docs reader result,
/// title and description already run through their own fallback chain
/// (`mdview_core::bee::BeeFeatureDocs`'s own doc comment): present with a
/// title, the header's own name becomes that title with the slug demoted
/// to a subtitle beneath it, plus the description as one clamped line, and
/// a docs row linking every markdown file the feature's docs dir holds
/// through this viewer's own document routes; `None` — every fallback
/// source empty — falls back to the slug alone with no docs row, exactly
/// as before this feature.
#[allow(clippy::too_many_arguments)]
pub fn bee_feature_page(
    project: &Project,
    feature: &str,
    buckets: &BeeBuckets,
    shipped: Option<&BeeShippedFeature>,
    is_closed: bool,
    lane_label: Option<&str>,
    worktrees: &[BeeWorktree],
    workspaces: &[BeeWorkspace],
    decisions: &[BeeDecisionSummary],
    panes: &[TerminalPaneView],
    gates: Option<&BeeApprovedGates>,
    docs: Option<&mdview_core::bee::BeeFeatureDocs>,
) -> String {
    let status_banner = match shipped {
        Some(f) => {
            let cycle = match &f.cycle_time {
                Some(span) if span.hours.is_finite() => format!("{:.1}h to finish", span.hours),
                Some(_) => "—".to_string(),
                None => "not timed yet".to_string(),
            };
            format!(
                r#"<div class="fg-banner fg-banner--success"><span class="fg-banner__dot"></span><span class="fg-banner__body">Shipped · {count} cell{plural} · {cycle}</span></div>"#,
                count = f.cell_count,
                plural = if f.cell_count == 1 { "" } else { "s" },
                cycle = esc(&cycle),
            )
        }
        None if is_closed => {
            let count = buckets.done.len();
            format!(
                r#"<div class="fg-card fg-card--sunken"><div class="fg-card__title">Closed · {count} cell{plural} done</div></div>"#,
                count = count,
                plural = if count == 1 { "" } else { "s" },
            )
        }
        None => {
            r#"<div class="fg-card fg-card--sunken"><div class="fg-card__title">Not shipped yet</div></div>"#
                .to_string()
        }
    };

    // A finished feature (shipped or closed) reads its worktree chip's
    // absent-grant fallback as "Merged"; anything still live reads "Main" —
    // see `bee_hub_worktree_chip`'s own doc comment (feature-hub-1).
    let finished = shipped.is_some() || is_closed;
    let worktree = bee_hub_worktree_chip(feature, worktrees, workspaces, finished);

    let all_cells = || {
        buckets
            .doing
            .iter()
            .chain(buckets.waiting.iter())
            .chain(buckets.stuck.iter())
            .chain(buckets.done.iter())
    };
    let duration = feature_cell_span(all_cells());
    let done = buckets.done.len();
    let total = buckets.doing.len() + buckets.waiting.len() + buckets.stuck.len() + done;

    let chip_row = bee_feature_chip_row(lane_label, &worktree, duration.as_ref(), done, total);
    let tabs = bee_feature_tabs(
        &bee_feature_activity_tab(&project.id, buckets, decisions, gates),
        &bee_feature_todos_tab(&project.id, buckets),
        &bee_feature_terminal_tab(&project.id, panes),
    );

    let title = docs.and_then(|d| d.title.as_deref()).filter(|t| !t.is_empty());
    let head_name_html = match title {
        Some(t) => format!(
            r#"<h2 class="fg-pagehead__title">{title}</h2><p class="bee-detail-slug">{feature}</p>"#,
            title = esc(t),
            feature = esc(feature),
        ),
        None => format!(r#"<h2 class="fg-pagehead__title">{feature}</h2>"#, feature = esc(feature)),
    };
    let desc_html = match docs.and_then(|d| d.description.as_deref()).filter(|d| !d.is_empty()) {
        Some(d) => format!(r#"<p class="bee-detail-desc">{}</p>"#, esc(d)),
        None => String::new(),
    };
    let docs_row = bee_feature_docs_row(&project.id, feature, docs);

    let body = format!(
        r#"{topbar}
{style}
<main class="fg-page bee-hub-theme">
  <div class="bee-detail-head">
    <div>
      {head_name_html}
      {desc_html}
    </div>
    {status_banner}
  </div>
  {docs_row}
  {chip_row}
  {tabs}
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name} · {feature}</span>",
            name = esc(&project.name),
            feature = esc(feature),
        )),
        style = bee_hub_style(),
        head_name_html = head_name_html,
        desc_html = desc_html,
        status_banner = status_banner,
        docs_row = docs_row,
        chip_row = chip_row,
        tabs = tabs,
    );
    layout(&format!("{} · {}", feature, project.name), "", &body)
}

/// D2's detail header docs row (feature-titles, extended by hub-fallbacks):
/// links every markdown file [`mdview_core::bee::BeeFeatureDocs::docs`]
/// lists (already sorted `CONTEXT.md`/`plan.md` first), each through this
/// viewer's own document route (`/p/<id>/docs/history/<feature>/…`, the
/// same project-relative shape [`file_page`]'s own links already use) —
/// never a bare filesystem path. Empty when `docs` is `None` or its own
/// `docs` list is empty: a feature with no markdown file under its docs
/// dir at all has nothing to link, whether or not it has a title or
/// description from another fallback tier.
fn bee_feature_docs_row(project_id: &str, feature: &str, docs: Option<&mdview_core::bee::BeeFeatureDocs>) -> String {
    let Some(docs) = docs else {
        return String::new();
    };
    if docs.docs.is_empty() {
        return String::new();
    }
    let pid = esc(project_id);
    let feature_href = esc(feature);
    let mut links = String::new();
    for file in &docs.docs {
        let file_href = esc(file);
        links.push_str(&format!(
            r#"<a href="/p/{pid}/docs/history/{feature_href}/{file_href}">{file_href}</a>"#,
            pid = pid,
            feature_href = feature_href,
            file_href = file_href,
        ));
    }
    format!(r#"<div class="bee-detail-docs">{links}</div>"#, links = links)
}

/// D2's chip row: this feature's own lane classification when known, its
/// worktree chip (already resolved by the caller — branch plus
/// open/merged/main state, see [`bee_hub_worktree_chip`]), its own
/// claim-to-cap duration when at least one cell has both endpoints
/// ([`feature_cell_span`]), and its cell done/total count. Each chip is
/// omitted, never faked, when its own source has nothing to report — a
/// feature with no lane record shows no lane chip rather than a guessed
/// one, and a feature with no timed cell shows no duration chip.
fn bee_feature_chip_row(
    lane_label: Option<&str>,
    worktree: &(String, &'static str),
    duration: Option<&mdview_core::bee::BeeCycleSpan>,
    done: usize,
    total: usize,
) -> String {
    let lane_chip = match lane_label.filter(|l| !l.is_empty()) {
        Some(l) => format!(r#"<span class="fg-chip fg-chip--neutral">lane: {}</span>"#, esc(l)),
        None => String::new(),
    };
    let (wt_label, wt_tone) = worktree;
    let worktree_chip = format!(
        r#"<span class="fg-chip fg-chip--{tone}">{label}</span>"#,
        tone = wt_tone,
        label = esc(wt_label),
    );
    let duration_chip = match duration {
        Some(span) if span.hours.is_finite() => format!(
            r#"<span class="fg-chip fg-chip--neutral">{hours:.1}h claim→cap</span>"#,
            hours = span.hours,
        ),
        _ => String::new(),
    };
    let cells_chip = format!(
        r#"<span class="fg-chip fg-chip--neutral">{done}/{total} cell{plural} done</span>"#,
        done = done,
        total = total,
        plural = if total == 1 { "" } else { "s" },
    );
    format!(
        r#"<div class="bee-detail-chips">{lane_chip}{worktree_chip}{duration_chip}{cells_chip}</div>"#,
        lane_chip = lane_chip,
        worktree_chip = worktree_chip,
        duration_chip = duration_chip,
        cells_chip = cells_chip,
    )
}

/// D2's CSS-only tab shell: three radio inputs (Activity checked by
/// default), a nav of labels, and a body of panels — `#bee-tab-*:checked`
/// selectors in [`bee_hub_style`] show the matching `#bee-panel-*` and
/// highlight the matching label, the same input-plus-label idiom
/// `topbar_full`'s own doc comment explains (a `<details>` element hides
/// its content past any `display` override; a plain input needs none of
/// that). No JavaScript.
fn bee_feature_tabs(activity_html: &str, todos_html: &str, terminal_html: &str) -> String {
    format!(
        r#"<div class="bee-tabs" data-tabs="1">
  <input type="radio" name="bee-detail-tab" id="bee-tab-activity" class="bee-tabs__radio" checked>
  <input type="radio" name="bee-detail-tab" id="bee-tab-todos" class="bee-tabs__radio">
  <input type="radio" name="bee-detail-tab" id="bee-tab-terminal" class="bee-tabs__radio">
  <div class="bee-tabs__nav">
    <label class="bee-tabs__label" for="bee-tab-activity">Activity</label>
    <label class="bee-tabs__label" for="bee-tab-todos">Todos</label>
    <label class="bee-tabs__label" for="bee-tab-terminal">Terminal</label>
  </div>
  <div class="bee-tabs__body">
    <div class="bee-tabs__panel" id="bee-panel-activity">{activity}</div>
    <div class="bee-tabs__panel" id="bee-panel-todos">{todos}</div>
    <div class="bee-tabs__panel" id="bee-panel-terminal">{terminal}</div>
  </div>
</div>"#,
        activity = activity_html,
        todos = todos_html,
        terminal = terminal_html,
    )
}

/// D2's Activity tab: which of the four lifecycle gates this feature's own
/// lane record (or, for the globally active feature with no lane record,
/// `state.json`) currently carries as approved; the most recently capped
/// cell's own test verdict; then a newest-first timeline joining every
/// feature-scoped `decide` event in `decisions` (already filtered to this
/// feature's own `scope` by the caller) with each capped cell's own
/// worker, outcome and `capped_at`. An entry whose own timestamp fails to
/// parse still renders, sorted after every entry that does — never
/// dropped, never guessed into place. Every timestamp renders as relative
/// language ([`bee_fmt_trace_time`]), never the raw ISO string.
fn bee_feature_activity_tab(
    project_id: &str,
    buckets: &BeeBuckets,
    decisions: &[BeeDecisionSummary],
    gates: Option<&BeeApprovedGates>,
) -> String {
    let rfc3339 = time::format_description::well_known::Rfc3339;

    let gates_html = match gates {
        Some(g) => {
            let pairs: [(&str, Option<bool>); 4] =
                [("Context", g.context), ("Shape", g.shape), ("Execution", g.execution), ("Review", g.review)];
            pairs
                .iter()
                .map(|(label, approved)| {
                    let approved = approved.unwrap_or(false);
                    let tone = if approved { "success" } else { "neutral" };
                    let word = if approved { "approved" } else { "not yet approved" };
                    format!(
                        r#"<span class="fg-chip fg-chip--{tone}">{label} {word}</span>"#,
                        tone = tone,
                        label = esc(label),
                        word = word,
                    )
                })
                .collect::<String>()
        }
        None => r#"<span class="fg-empty">No gate record for this feature.</span>"#.to_string(),
    };

    let latest_test = buckets
        .done
        .iter()
        .filter(|c| c.tests.is_some())
        .filter_map(|c| {
            let ts = c.capped_at.as_deref()?;
            let t = time::OffsetDateTime::parse(ts, &rfc3339).ok()?;
            Some((t, c))
        })
        .max_by_key(|(t, _)| t.unix_timestamp_nanos())
        .map(|(_, c)| c);
    let latest_test_html = match latest_test {
        Some(c) => {
            let tests = c.tests.as_deref().unwrap_or("—");
            format!(
                r#"<p class="bee-cell__meta">Latest verify: <span class="fg-chip fg-chip--{tone}">{tests}</span> ({when})</p>"#,
                tone = if tests == "green" { "success" } else { "danger" },
                tests = esc(tests),
                when = esc(&c.capped_at.as_deref().map(bee_fmt_trace_time).unwrap_or_default()),
            )
        }
        None => r#"<p class="fg-empty">No verification recorded yet.</p>"#.to_string(),
    };

    let mut entries: Vec<(Option<time::OffsetDateTime>, String, String)> = Vec::new();
    for d in decisions {
        let parsed = time::OffsetDateTime::parse(&d.date, &rfc3339).ok();
        let when = if parsed.is_some() { bee_fmt_trace_time(&d.date) } else { d.date.clone() };
        entries.push((
            parsed,
            d.date.clone(),
            format!(
                r#"<div class="fg-card bee-cell bee-activity__item"><div class="bee-activity__ts">{when}</div><p>{decision}</p></div>"#,
                when = esc(&when),
                decision = esc(&d.decision),
            ),
        ));
    }
    for c in &buckets.done {
        let Some(capped_at) = c.capped_at.as_deref() else { continue };
        let parsed = time::OffsetDateTime::parse(capped_at, &rfc3339).ok();
        let worker = c.worker.as_deref().unwrap_or("unknown worker");
        let outcome = c.outcome.as_deref().unwrap_or("No outcome recorded.");
        entries.push((
            parsed,
            capped_at.to_string(),
            format!(
                r#"<div class="fg-card bee-cell bee-activity__item"><div class="bee-activity__ts">{when}</div><p><strong>{worker}</strong> capped <a href="/p/{pid}/_bee/cell/{cid_href}">{cid}</a> — {outcome}</p></div>"#,
                when = esc(&bee_fmt_trace_time(capped_at)),
                worker = esc(worker),
                pid = esc(project_id),
                cid_href = esc(&c.id),
                cid = esc(&c.id),
                outcome = esc(outcome),
            ),
        ));
    }
    entries.sort_by(|a, b| match (&a.0, &b.0) {
        (Some(x), Some(y)) => y.cmp(x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.1.cmp(&a.1),
    });

    let timeline_html = if entries.is_empty() {
        r#"<p class="fg-empty">No activity recorded yet.</p>"#.to_string()
    } else {
        let items: String = entries.into_iter().map(|(_, _, html)| html).collect();
        format!(r#"<div class="bee-activity__timeline">{items}</div>"#)
    };

    format!(
        r#"<div class="bee-activity"><div class="bee-activity__gates">{gates_html}</div>{latest_test_html}{timeline_html}</div>"#,
        gates_html = gates_html,
        latest_test_html = latest_test_html,
        timeline_html = timeline_html,
    )
}

/// D2's Todos tab: every one of the feature's own cells (already merged
/// with any archived ones by the caller) as a checklist — a capped cell
/// strikes through, a claimed one carries its own worker as an agent
/// badge, a blocked one carries a red marker, an open one renders plain
/// (see [`bee_todo_item`]). Ordered by cell id for a stable, deterministic
/// read.
fn bee_feature_todos_tab(project_id: &str, buckets: &BeeBuckets) -> String {
    let mut cells: Vec<&BeeCell> = buckets
        .doing
        .iter()
        .chain(buckets.waiting.iter())
        .chain(buckets.stuck.iter())
        .chain(buckets.done.iter())
        .collect();
    if cells.is_empty() {
        return r#"<p class="fg-empty">No cells recorded for this feature.</p>"#.to_string();
    }
    cells.sort_by(|a, b| a.id.cmp(&b.id));
    let items: String = cells.into_iter().map(|c| bee_todo_item(project_id, c)).collect();
    format!(r#"<ul class="bee-todos">{items}</ul>"#)
}

/// One Todos-tab checklist row. `bee-todo--done`'s strikethrough,
/// `bee-todo--blocked`'s red marker and the claimed-only agent badge are
/// all CSS-driven ([`bee_hub_style`]), matching this board's existing
/// class-plus-token idiom rather than an inline style.
fn bee_todo_item(project_id: &str, cell: &BeeCell) -> String {
    let (row_cls, mark) = match cell.status.as_str() {
        "capped" => ("bee-todo--done", "✓"),
        "claimed" => ("bee-todo--claimed", "●"),
        "blocked" => ("bee-todo--blocked", "✕"),
        _ => ("bee-todo--open", "○"),
    };
    let badge = if cell.status == "claimed" {
        match cell.worker.as_deref() {
            Some(w) => {
                format!(r#"<span class="fg-chip fg-chip--accent bee-todo__badge">{}</span>"#, esc(w))
            }
            None => String::new(),
        }
    } else {
        String::new()
    };
    format!(
        r#"<li class="bee-todo {row_cls}"><a href="/p/{pid}/_bee/cell/{cid_href}"><span class="bee-todo__mark" aria-hidden="true">{mark}</span><span class="bee-todo__title">{title}</span></a>{badge}</li>"#,
        row_cls = row_cls,
        pid = esc(project_id),
        cid_href = esc(&cell.id),
        mark = mark,
        title = esc(&cell.title),
        badge = badge,
    )
}

/// D2's Terminal tab (feature-titles): the project's own agent-terminal
/// panes — the same D2-containment-boundary-filtered list `project_panes`
/// computes for `terminal_page` itself, handed in already resolved so this
/// module never reaches herdr directly. Each entry links straight to that
/// pane's own live page (`/p/<id>/_terminal/pane/<pane_id>`,
/// `terminal_page_for_pane`'s route) — the interactive surface (screen,
/// reply form, key buttons) already lives there; this tab adds no input of
/// its own, only the map from "which panes exist" to "where to drive one".
/// An honest empty state when `panes` is empty — whether that is because
/// herdr is down, the terminal family switch is off, or the project
/// genuinely has no panes running right now, the caller has already
/// folded all three into the same fail-closed-to-empty list (matching
/// `project_panes`'s own "fail closed to zero, not a crash" rule), so this
/// tab draws no distinction between them.
fn bee_feature_terminal_tab(project_id: &str, panes: &[TerminalPaneView]) -> String {
    if panes.is_empty() {
        return r#"<p class="fg-empty">No terminal panes running for this project right now.</p>"#.to_string();
    }

    let mut rows = String::new();
    for p in panes {
        rows.push_str(&format!(
            r#"<a class="fg-card bee-cell bee-terminal-pane" href="/p/{pid}/_terminal/pane/{pane_id}"><div class="fg-card__title">{workspace} · {tab}</div><div class="bee-cell__meta">{program}</div><div class="bee-hub__chips">{status_pill}</div></a>"#,
            pid = esc(project_id),
            pane_id = esc(&p.pane_id),
            workspace = esc(&p.workspace),
            tab = esc(&p.tab),
            program = esc(&p.kind),
            status_pill = status_pill(&p.status),
        ));
    }
    format!(r#"<div class="bee-terminal-panes">{rows}</div>"#)
}

pub fn file_page(
    project: &Project,
    file: &IndexedFile,
    page: &RenderedPage,
    files: &[IndexedFile],
    backlinks: &[(String, String)],
) -> String {
    let tree = file_tree(project, files, &file.rel_path);
    let right = right_panel(project, page, backlinks);
    let breadcrumb = breadcrumb(project, &file.rel_path);
    // Raw markdown source for copy-as-markdown: the client maps a DOM selection
    // (via data-sourcepos line ranges) back to these source lines. Escape `<`
    // so a source containing "</script>" can't break out of the tag.
    let source_json = escape_json_for_script(&page.source);
    let head_extra = if page.has_mermaid {
        // Mermaid is vendored and served locally (/static/mermaid.min.js) rather
        // than loaded from a CDN: the daemon commonly runs on a LAN/offline host
        // where a CDN is unreachable, which would leave diagrams unrendered.
        r#"<script src="/static/mermaid.min.js" defer></script>
<script>
(function () {
  // Surface a render failure ON the page (mobile has no dev console), so a
  // broken diagram shows why instead of silently staying blank.
  function fail(msg) {
    document.querySelectorAll('pre.mermaid').forEach(function (p) {
      if (p.querySelector('svg') || p.dataset.err) return;
      p.dataset.err = '1';
      var d = document.createElement('div');
      d.className = 'mermaid-error';
      d.textContent = 'Mermaid did not render: ' + msg;
      p.parentNode.insertBefore(d, p.nextSibling);
    });
  }
  function renderMermaid() {
    if (!window.mermaid) { fail('library /static/mermaid.min.js did not load'); return; }
    window.__mermaid = window.mermaid;
    var dark = document.documentElement.getAttribute('data-scheme') === 'dark';
    try { window.mermaid.initialize({ startOnLoad: false, theme: dark ? 'dark' : 'default' }); }
    catch (e) { fail('initialize: ' + ((e && e.message) || e)); return; }
    var done = function () { document.dispatchEvent(new Event('mdview:mermaid-done')); };
    var onErr = function (e) { fail((e && e.message) || String(e)); done(); };
    try {
      var r = window.mermaid.run({ querySelector: 'pre.mermaid' });
      if (r && r.then) { r.then(done, onErr); } else { done(); }
    } catch (e) { onErr(e); }
  }
  if (document.readyState === 'loading') {
    window.addEventListener('DOMContentLoaded', renderMermaid);
  } else {
    renderMermaid();
  }
})();
</script>"#
    } else {
        ""
    };
    let body = format!(
        r#"{topbar}
<div class="layout">
  <aside id="sidebar" class="sidebar">{tree}</aside>
  <div class="sidebar-backdrop"></div>
  <main class="content">
    {breadcrumb}
    <div class="fg-reading">
      <article class="fg-prose markdown-body">{html}</article>
    </div>
    <script type="application/json" id="mdsource">{source_json}</script>
  </main>
  {right}
</div>"#,
        topbar = topbar_full(
            sidebar_toggle(),
            &format!(
                "<span class=\"crumb\">{pname} / {rel}</span>",
                pname = esc(&project.name),
                rel = esc(&file.rel_path),
            ),
            copy_md_button(),
            "",
        ),
        tree = tree,
        breadcrumb = breadcrumb,
        html = page.html,
        source_json = source_json,
        right = right,
    );
    layout(&page.title, head_extra, &body)
}

/// Escape `<` in an already-serialized JSON blob so a literal `</script>` in
/// the data cannot break out of the `<script>` tag it is embedded in. Shared by
/// every place that inlines JSON into a page, so the guard can never diverge.
fn escape_script_breakout(json: &str) -> String {
    json.replace('<', "\\u003c")
}

/// Serialize `source` as a JSON string literal safe to embed inside a
/// `<script>` tag: escapes `<` to `<` so a source containing a literal
/// "</script>" can't break out of the tag.
fn escape_json_for_script(source: &str) -> String {
    escape_script_breakout(&serde_json::to_string(source).unwrap_or_else(|_| "\"\"".into()))
}

/// Right sidebar: table of contents + backlinks (FR-18). Empty string if neither.
fn right_panel(project: &Project, page: &RenderedPage, backlinks: &[(String, String)]) -> String {
    let mut inner = String::new();
    let toc: Vec<_> = page
        .headings
        .iter()
        .filter(|h| h.level >= 1 && h.level <= 4)
        .collect();
    if !toc.is_empty() {
        inner.push_str("<div class=\"panel-head\">On this page</div><ul class=\"toc\">");
        for h in toc {
            inner.push_str(&format!(
                "<li class=\"toc-l{lvl}\"><a href=\"#{slug}\">{text}</a></li>",
                lvl = h.level,
                slug = esc(&h.slug),
                text = esc(&h.text),
            ));
        }
        inner.push_str("</ul>");
    }
    if !backlinks.is_empty() {
        inner.push_str("<div class=\"panel-head\">Linked from</div><ul class=\"backlinks\">");
        for (rel, title) in backlinks {
            inner.push_str(&format!(
                "<li><a href=\"/p/{pid}/{rel}\">{title}</a></li>",
                pid = esc(&project.id),
                rel = esc(rel),
                title = esc(title),
            ));
        }
        inner.push_str("</ul>");
    }
    if inner.is_empty() {
        String::new()
    } else {
        format!("<aside class=\"rightbar\">{inner}</aside>")
    }
}

/// Breadcrumb of path segments (orientation only; folders are not pages).
fn breadcrumb(project: &Project, rel_path: &str) -> String {
    let mut crumbs = format!(
        "<a href=\"/p/{pid}/\">{name}</a>",
        pid = esc(&project.id),
        name = esc(&project.name)
    );
    for seg in rel_path.split('/') {
        crumbs.push_str(&format!(" <span class=\"sep\">/</span> {}", esc(seg)));
    }
    format!("<nav class=\"breadcrumb\">{crumbs}</nav>")
}

/// The parent folder of a relative path (`""` for a root-level file).
fn parent_dir(rel: &str) -> &str {
    match rel.rfind('/') {
        Some(i) => &rel[..i],
        None => "",
    }
}

/// The last path segment of a relative path.
fn base_name(rel: &str) -> &str {
    match rel.rfind('/') {
        Some(i) => &rel[i + 1..],
        None => rel,
    }
}

/// Chapter sidebar (C2, per D 99e8df73): the search box, plus a `#chapter`
/// container the client script renders into — always one folder's contents with
/// a zoomable breadcrumb. The full file list ships as JSON so the zoom is
/// client-side (no extra routes); a minimal current-folder list is server-
/// rendered inside `#chapter` as a no-JS fallback.
fn file_tree(project: &Project, files: &[IndexedFile], active: &str) -> String {
    // JSON payload for the client renderer: one {p: rel_path, t: title} per file.
    let payload: Vec<_> = files
        .iter()
        .map(|f| serde_json::json!({ "p": f.rel_path, "t": f.title }))
        .collect();
    // Escape `<` so a title containing "</script>" can't break out of the tag.
    let json =
        escape_script_breakout(&serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into()));

    // No-JS fallback: the files directly in the active file's folder, by title.
    let active_dir = parent_dir(active);
    let mut fallback = String::new();
    for f in files
        .iter()
        .filter(|f| parent_dir(&f.rel_path) == active_dir)
    {
        let label = if f.title.is_empty() {
            base_name(&f.rel_path)
        } else {
            &f.title
        };
        let cls = if f.rel_path == active {
            "chap-file active"
        } else {
            "chap-file"
        };
        fallback.push_str(&format!(
            "<a class=\"{cls}\" href=\"/p/{pid}/{rel}\">{label}</a>",
            pid = esc(&project.id),
            rel = esc(&f.rel_path),
            label = esc(label),
        ));
    }

    format!(
        "<form class=\"fg-sidebar-search\" action=\"/p/{pid}/_search\" method=\"get\">\
         <input class=\"fg-input\" name=\"q\" placeholder=\"Search…\" autocomplete=\"off\"></form>\
         <nav class=\"chapter\" id=\"chapter\" data-pid=\"{pid}\" data-root=\"{root}\" \
         data-current=\"{cur}\">{fallback}</nav>\
         <script type=\"application/json\" id=\"filelist\">{json}</script>",
        pid = esc(&project.id),
        root = esc(&project.name),
        cur = esc(active),
        fallback = fallback,
        json = json,
    )
}

fn theme_toggle() -> &'static str {
    r#"<button id="theme-toggle" class="theme-toggle fg-btn fg-btn--ghost" title="Toggle theme">◐</button>"#
}

/// Hamburger that opens the file-tree sidebar on mobile (hidden on wide
/// screens via CSS). Only file pages carry a sidebar, so only they render it.
fn sidebar_toggle() -> &'static str {
    r#"<button id="sidebar-toggle" class="sidebar-toggle" type="button" aria-label="Toggle file navigation" aria-controls="sidebar" aria-expanded="false">☰</button>"#
}

/// Copy-the-whole-page-as-Markdown action for the top bar (file pages only; it
/// reads the `#mdsource` blob). Icon collapses to just the glyph on mobile.
fn copy_md_button() -> &'static str {
    r#"<button id="copy-md" class="copy-md" type="button" title="Copy page as Markdown" aria-label="Copy page as Markdown"><svg class="copy-md__icon" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg><span class="copy-md__txt">Copy</span></button>"#
}

/// Shared top bar for every page: brand, a page-specific center slot (crumb or
/// empty), the Settings link, and the theme toggle. Keeps the Settings link on
/// all pages and stops each view re-inventing its own header.
fn topbar(center: &str) -> String {
    topbar_full("", center, "", "")
}

/// Full top bar: an optional `lead` slot (before the brand), an optional
/// `actions` slot (page-specific buttons that stay on the bar at every width,
/// e.g. the copy-page-as-Markdown button on file pages), and an optional
/// `nav` slot for links that navigate away from this page.
///
/// The nav slot and the Settings link share one menu. On a wide screen the
/// stylesheet hides its control and lays the panel out inline, so the bar
/// reads exactly as it did before this existed. On a narrow one the control
/// becomes the only visible affordance and the panel drops full-width under
/// the bar.
///
/// The open state is a checkbox rather than a `<details>` — deliberately.
/// `<details>` looked like the obvious fit, but a *closed* one has its
/// content hidden by the browser itself (`::details-content`), which no
/// `display` rule of ours overrides, so the wide-screen bar rendered with no
/// navigation at all. A checkbox keeps the panel an ordinary element that CSS
/// alone decides about, at both widths, and still needs no JavaScript:
/// `assets/app.js` only adds the two conveniences the markup has no opinion
/// about — closing on Escape and on a press outside the panel.
fn topbar_full(lead: &str, center: &str, actions: &str, nav: &str) -> String {
    format!(
        r#"<header class="topbar">
  {lead}
  <a href="/" class="home">Bee Artifact</a>
  {center}
  {actions}
  <div class="topbar-menu js-menu">
    <input type="checkbox" id="topbar-menu-toggle" class="topbar-menu__toggle">
    <label class="topbar-menu__button" for="topbar-menu-toggle" title="Menu"><span class="menu-label">Menu</span><span aria-hidden="true">☰</span></label>
    <div class="topbar-menu__panel">
      {nav}
      <a class="nav-link" href="/settings">Settings</a>
    </div>
  </div>
  {toggle}
</header>"#,
        lead = lead,
        center = center,
        actions = actions,
        nav = nav,
        toggle = theme_toggle(),
    )
}

pub fn search_page(project: &Project, query: &str, results: &[SearchResult]) -> String {
    let mut items = String::new();
    if query.trim().is_empty() {
        items.push_str("<p class=\"fg-empty\">Type a query to search this project.</p>");
    } else if results.is_empty() {
        items.push_str(&format!(
            "<p class=\"fg-empty\">No matches for “{}”.</p>",
            esc(query)
        ));
    } else {
        for r in results {
            items.push_str(&format!(
                "<a class=\"fg-card\" href=\"{url}\"><div class=\"fg-card__title\">{title}</div>\
                 <div class=\"fg-card__sub\">{rel}</div><div class=\"fg-card__sub\">{excerpt}</div></a>",
                url = esc(&r.url),
                title = esc(&r.title),
                rel = esc(&r.rel_path),
                excerpt = highlight_excerpt(&r.excerpt),
            ));
        }
    }
    let body = format!(
        r#"{topbar}
<main class="fg-page">
  <form action="/p/{pid}/_search" method="get">
    <input class="fg-input" name="q" value="{q}" placeholder="Search…" autofocus autocomplete="off">
  </form>
  {items}
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name} · search</span>",
            name = esc(&project.name)
        )),
        pid = esc(&project.id),
        q = esc(query),
        items = items,
    );
    layout(&format!("search: {query}"), "", &body)
}

/// FTS snippets contain `<mark>…</mark>`. Escape everything, then restore marks.
fn highlight_excerpt(excerpt: &str) -> String {
    esc(excerpt)
        .replace("&lt;mark&gt;", "<mark class=\"fg-mark\">")
        .replace("&lt;/mark&gt;", "</mark>")
}

/// What the settings page's notification section renders for the Telegram
/// credential (agent-terminal-18). There is no `Full` variant at all: this
/// credential is never rendered back in full, not even once — the form
/// that sets it (`/api/terminal-config`) is write-only for this field, so
/// this is the *only* view any response ever carries, including the one
/// immediately after a save.
pub enum NotifyCredentialView {
    /// No credential has ever been saved.
    NotConfigured,
    /// The last four characters of the saved credential.
    Masked(String),
}

pub fn settings_page(
    cfg: &Config,
    saved: bool,
    notify_credential_save_failed: bool,
    notify_credential_view: NotifyCredentialView,
) -> String {
    // agent-terminal-24: checked first, so a failed credential save is never
    // shadowed by `saved=1` also being set on the same redirect — a user
    // whose token could not be written must see the failure, not the
    // generic success banner (`update_terminal_config` in server.rs never
    // sends both flags at once, but this order makes the page's own
    // guarantee independent of that caller detail).
    let banner = if notify_credential_save_failed {
        "<div class=\"fg-banner fg-banner--danger\"><span class=\"fg-banner__dot\"></span><span class=\"fg-banner__body\">The Telegram bot token could not be saved. Notifications will keep using the previous token, if any — try again.</span></div>"
    } else if saved {
        "<div class=\"fg-banner fg-banner--success\"><span class=\"fg-banner__dot\"></span><span class=\"fg-banner__body\">Saved. Server &amp; indexing changes apply after restart (<code>mdview stop &amp;&amp; mdview serve</code>).</span></div>"
    } else {
        ""
    };
    let checked = |b: bool| if b { "checked" } else { "" };
    let sel = |v: &str, opt: &str| if v == opt { "selected" } else { "" };
    let excludes = cfg.indexing.exclude_patterns.join("\n");

    // D7/D9: the notification credential is never rendered back in full —
    // see `NotifyCredentialView`'s own doc comment for why there is no
    // `Full` variant to match here at all.
    let (notify_credential_hint, notify_credential_placeholder) = match notify_credential_view {
        NotifyCredentialView::NotConfigured => (
            "No Telegram bot token saved yet.".to_string(),
            "Paste the bot token".to_string(),
        ),
        NotifyCredentialView::Masked(masked) => (
            format!("Bot token: {masked} — leave blank to keep it.", masked = esc(&masked)),
            "Leave blank to keep the current token".to_string(),
        ),
    };

    let body = format!(
        r#"{topbar}
<main class="fg-page">
  <h2 class="fg-pagehead__title">Settings <span class="t-caption fg-settings__version">mdview v{version}</span></h2>
  {banner}
  <form class="fg-settings" method="post" action="/api/config">
    <fieldset><legend>Server <span class="fg-chip fg-chip--neutral">restart</span></legend>
      <div class="fg-field-row">
        <div class="fg-field">
          <label class="fg-field__label">Host</label>
          <input class="fg-input" name="host" value="{host}">
          <span class="fg-field__hint">127.0.0.1 (local) or 0.0.0.0 (LAN)</span>
        </div>
        <div class="fg-field">
          <label class="fg-field__label">Port</label>
          <input class="fg-input" type="number" name="port" value="{port}" min="1" max="65535">
        </div>
      </div>
      <div class="fg-field">
        <label class="fg-field__label">Display hostname</label>
        <input class="fg-input" name="hostname" value="{hostname}">
        <span class="fg-field__hint">optional — used in rendered links instead of the IP/host above</span>
      </div>
      <label class="fg-check"><input type="checkbox" name="open_browser" {open}><span class="fg-check__text">Open browser on start</span></label>
    </fieldset>
    <fieldset><legend>MCP <span class="fg-chip fg-chip--neutral">restart</span></legend>
      <label class="fg-check"><input type="checkbox" name="mcp_enabled" {mcp_on}><span class="fg-check__text">Enabled</span></label>
      <div class="fg-field">
        <label class="fg-field__label">Transport</label>
        <div class="fg-select">
          <select name="mcp_transport">
            <option value="stdio" {tr_stdio}>stdio</option>
            <option value="http" {tr_http}>http</option>
          </select>
          <span class="fg-select__chev">▾</span>
        </div>
      </div>
    </fieldset>
    <fieldset><legend>Renderer</legend>
      <div class="fg-field">
        <label class="fg-field__label">Theme</label>
        <div class="fg-select">
          <select name="theme">
            <option value="system" {t_sys}>System</option>
            <option value="light" {t_light}>Light</option>
            <option value="dark" {t_dark}>Dark</option>
          </select>
          <span class="fg-select__chev">▾</span>
        </div>
      </div>
      <div class="fg-field">
        <label class="fg-field__label">Syntax highlight theme</label>
        <input class="fg-input" name="syntax_theme" value="{syntax}">
      </div>
    </fieldset>
    <fieldset><legend>Indexing <span class="fg-chip fg-chip--neutral">restart</span></legend>
      <div class="fg-field-row">
        <div class="fg-field">
          <label class="fg-field__label">Debounce (ms)</label>
          <input class="fg-input" type="number" name="debounce_ms" value="{debounce}" min="0">
        </div>
        <div class="fg-field">
          <label class="fg-field__label">Max file size (MB)</label>
          <input class="fg-input" type="number" name="max_file_size_mb" value="{maxmb}" min="1">
        </div>
      </div>
      <div class="fg-field">
        <label class="fg-field__label">Exclude patterns (one per line)</label>
        <textarea class="fg-input fg-input--area" name="exclude_patterns" rows="5">{excludes}</textarea>
      </div>
    </fieldset>
    <button type="submit" class="fg-btn fg-btn--primary">Save</button>
  </form>
  <form class="fg-settings" id="terminal-config-form" method="post" action="/api/terminal-config">
    <fieldset><legend>Terminal</legend>
      <label class="fg-check"><input type="checkbox" name="enabled" {term_enabled}><span class="fg-check__text">Enable the terminal</span></label>
      <label class="fg-check"><input type="checkbox" name="supervisor_enabled" {term_supervisor}><span class="fg-check__text">Keep herdr running (supervisor)</span></label>
      <label class="fg-check"><input type="checkbox" name="notify_enabled" {term_notify}><span class="fg-check__text">Notify on agent status change</span></label>
      <label class="fg-check"><input type="checkbox" name="unassigned_enabled" {term_unassigned}><span class="fg-check__text">Show unassigned agent panes</span></label>
      <span class="fg-field__hint">Off by default. Turning this on makes every agent pane on this machine readable and writable through the browser, including ones outside any project mdview knows about — unrelated repositories, root shells, other people's agents. It has no boundary check of its own.</span>
    </fieldset>
    <fieldset><legend>Telegram notification</legend>
      <div class="fg-field">
        <label class="fg-field__label">Chat id</label>
        <input class="fg-input" name="notify_chat_id" value="{notify_chat_id}">
        <span class="fg-field__hint">The destination the notifier sends agent status changes to.</span>
      </div>
      <div class="fg-field">
        <label class="fg-field__label">Bot token</label>
        <input class="fg-input" type="password" name="notify_telegram_token" autocomplete="off" placeholder="{notify_credential_placeholder}">
        <span class="fg-field__hint">{notify_credential_hint}</span>
      </div>
    </fieldset>
    <button type="submit" class="fg-btn fg-btn--primary">Save terminal settings</button>
  </form>
</main>"#,
        topbar = topbar("<span class=\"crumb\">Settings</span>"),
        banner = banner,
        version = env!("CARGO_PKG_VERSION"),
        port = cfg.server.port,
        host = esc(&cfg.server.host),
        hostname = esc(cfg.server.hostname.as_deref().unwrap_or("")),
        open = checked(cfg.server.open_browser_on_start),
        t_sys = sel(&cfg.renderer.theme, "system"),
        t_light = sel(&cfg.renderer.theme, "light"),
        t_dark = sel(&cfg.renderer.theme, "dark"),
        syntax = esc(&cfg.renderer.syntax_highlight_theme),
        debounce = cfg.indexing.debounce_ms,
        maxmb = cfg.indexing.max_file_size_mb,
        excludes = esc(&excludes),
        mcp_on = checked(cfg.mcp.enabled),
        term_enabled = checked(cfg.terminal.enabled),
        term_supervisor = checked(cfg.terminal.supervisor_enabled),
        term_notify = checked(cfg.terminal.notify_enabled),
        term_unassigned = checked(cfg.terminal.unassigned_enabled),
        notify_chat_id = esc(cfg.terminal.notify_chat_id.as_deref().unwrap_or("")),
        notify_credential_hint = notify_credential_hint,
        notify_credential_placeholder = esc(&notify_credential_placeholder),
        tr_stdio = sel(&cfg.mcp.transport, "stdio"),
        tr_http = sel(&cfg.mcp.transport, "http"),
    );
    layout("Settings", "", &body)
}

pub fn error_page(status: u16, msg: &str) -> String {
    let body = format!(
        r#"{topbar}
<main class="fg-page"><h2 class="fg-pagehead__title">{status}</h2><p class="fg-empty">{msg}</p></main>"#,
        topbar = topbar(""),
        status = status,
        msg = esc(msg)
    );
    layout(&status.to_string(), "", &body)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub const APP_CSS: &str = concat!(
    include_str!("../assets/atelier/fonts.css"),
    "\n",
    include_str!("../assets/atelier/contract.css"),
    "\n",
    include_str!("../assets/atelier/components.css"),
    "\n",
    include_str!("../assets/atelier/editorial.css"),
    "\n",
    include_str!("../assets/atelier/atelier.css"),
    "\n",
    include_str!("../assets/app.css"),
);
pub const APP_JS: &str = include_str!("../assets/app.js");

/// Cache-busting fingerprints for the two stylesheet/script URLs the layout
/// emits. Both assets are served `no-cache`, which asks a browser to
/// revalidate but leaves it free to keep serving what it already has while it
/// does — and with no validator on the response there is nothing for it to
/// revalidate against, so a stale copy could survive a plain reload. A URL
/// carrying the content's own hash cannot: change the file and the URL is a
/// different URL, which no cache has an entry for.
fn asset_fingerprint(content: &str) -> String {
    format!("{:x}", mdview_core::ansi::revision_of(content))
}

/// `/static/app.css?v=…` — the stylesheet URL with its fingerprint.
pub fn app_css_url() -> String {
    format!("/static/app.css?v={}", asset_fingerprint(APP_CSS))
}

/// `/static/app.js?v=…` — the script URL with its fingerprint.
pub fn app_js_url() -> String {
    format!("/static/app.js?v={}", asset_fingerprint(APP_JS))
}
/// Vendored Mermaid (self-contained UMD build) served at /static/mermaid.min.js
/// so diagrams render without a CDN. Only loaded on pages that contain a diagram.
pub const MERMAID_JS: &str = include_str!("../assets/mermaid.min.js");

#[cfg(test)]
mod tests {
    use super::*;

    /// Every page's stylesheet and script URL carries the asset's own content
    /// fingerprint, so an edit to either lands on a browser that has the old
    /// one cached: the URL itself changes, and no cache holds an entry for a
    /// URL it has never seen. Without this a reload could keep serving the
    /// stale copy — `no-cache` asks for revalidation, but the responses carry
    /// no validator to revalidate against.
    #[test]
    fn asset_urls_carry_a_content_fingerprint() {
        let page = layout("t", "", "<p>body</p>");
        let css = app_css_url();
        let js = app_js_url();
        assert!(css.starts_with("/static/app.css?v="), "{css}");
        assert!(js.starts_with("/static/app.js?v="), "{js}");
        assert_ne!(
            css.split("v=").nth(1),
            js.split("v=").nth(1),
            "two different assets must not fingerprint alike: {css} vs {js}"
        );
        assert!(page.contains(&format!("href=\"{css}\"")), "{page}");
        assert!(page.contains(&format!("src=\"{js}\"")), "{page}");
        // A changed asset must produce a changed URL — the whole point.
        assert_ne!(
            asset_fingerprint("a"),
            asset_fingerprint("b"),
            "different content must fingerprint differently"
        );
    }

    #[test]
    fn relative_minutes_reads_as_plain_relative_language_not_a_timestamp() {
        assert_eq!(bee_relative_minutes(0.2), "just now");
        assert_eq!(bee_relative_minutes(4.0), "4 minutes ago");
        assert_eq!(bee_relative_minutes(1.0), "1 minute ago");
        assert_eq!(bee_relative_minutes(120.0), "2 hours ago");
        assert_eq!(bee_relative_minutes(60.0), "1 hour ago");
        assert_eq!(bee_relative_minutes(60.0 * 24.0 * 3.0), "3 days ago");
        // A heartbeat somehow in the future reads as "just now", never a
        // negative duration.
        assert_eq!(bee_relative_minutes(-5.0), "just now");
        assert_eq!(bee_relative_minutes(f64::NAN), "unknown");
        // Never a raw ISO-8601 shape anywhere in the output.
        for mins in [0.0, 4.0, 90.0, 60.0 * 30.0] {
            assert!(!bee_relative_minutes(mins).contains('T'));
        }
    }

    #[test]
    fn escape_script_breakout_neutralizes_closing_tag_in_array_json() {
        // The sidebar #filelist payload is a JSON array; a file title of
        // "</script>..." must not survive as a raw "<".
        let json = r#"[{"p":"a.md","t":"x</script><script>alert(1)</script>"}]"#;
        let escaped = escape_script_breakout(json);
        assert!(!escaped.contains('<'), "raw '<' leaked: {escaped}");
        assert!(escaped.contains("\\u003c"));
    }

    #[test]
    fn escape_json_for_script_neutralizes_script_breakout() {
        let source = "before </script><script>alert(1)</script> after";
        let escaped = escape_json_for_script(source);
        assert!(
            !escaped.contains('<'),
            "escaped blob must contain no raw '<': {escaped}"
        );
    }

    #[test]
    fn escape_json_for_script_round_trips_to_original_source() {
        let source = "line one\n</script>\nline three with <tag> and \"quotes\"";
        let escaped = escape_json_for_script(source);
        let round_tripped: String =
            serde_json::from_str(&escaped).expect("escaped blob must still be valid JSON");
        assert_eq!(round_tripped, source);
    }

    fn sample_project() -> Project {
        Project {
            id: "proj-1".into(),
            name: "Proj One".into(),
            root_path: std::path::PathBuf::from("/tmp/proj-1"),
            created_at: "2026-08-05T00:00:00Z".into(),
            last_seen_at: "2026-08-05T00:00:00Z".into(),
        }
    }

    /// agent-terminal-13, must-have: "the terminal page gains the creation
    /// controls, offering only the configured preset labels" — every
    /// configured label renders as its own button, carrying the label as
    /// `data-preset` (what `terminal_create_agent`'s body actually reads),
    /// and an unconfigured label never appears.
    #[test]
    fn terminal_page_lists_only_configured_preset_labels() {
        let project = sample_project();
        let presets = vec!["Claude".to_string(), "Codex".to_string()];
        let html = terminal_page(&project, &[], None, &presets);
        assert!(html.contains(r#"data-preset="Claude">Claude</button>"#), "{html}");
        assert!(html.contains(r#"data-preset="Codex">Codex</button>"#), "{html}");
        assert!(!html.contains("data-preset=\"Aider\""), "an unconfigured label must never render: {html}");
        // The plain-shell control is unconditional — it needs no preset.
        assert!(html.contains(r#"<button type="button" class="term-create__pane">New shell</button>"#));
    }

    /// terminal-image-attach: the attach control (picker button, hidden file
    /// input, chip list) rides in the reply form on a project's own terminal
    /// page — it has a project-scoped `/p/:id/_terminal/:pane_id/attach`
    /// route to upload against — but the Unassigned page shares
    /// [`pane_cards`]'s markup with no such route, so it must render none of
    /// it (plan finding 7).
    #[test]
    fn terminal_page_renders_the_attach_control_and_unassigned_does_not() {
        let project = sample_project();
        let panes = vec![TerminalPaneView {
            pane_id: "w1:p1".into(),
            kind: "claude".into(),
            name: "one".into(),
            status: "working".into(),
            title: String::new(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        }];
        let project_html = terminal_page(&project, &panes, Some("w1:p1"), &[]);
        assert!(
            project_html.contains(r#"class="term-attach" data-pane-id="w1:p1""#),
            "the project pane page must render the attach control: {project_html}"
        );
        assert!(
            project_html.contains(r#"class="term-attach__input""#)
                && project_html.contains(r#"type="file""#)
                && project_html.contains("multiple")
                && project_html.contains(r#"accept="image/*""#),
            "the attach control must be a multi-file image picker: {project_html}"
        );
        assert!(
            project_html.contains(r#"class="term-attach__chips""#),
            "the attach control must offer an (initially empty) chip list: {project_html}"
        );

        let unassigned_html = unassigned_terminal_page(&panes);
        assert!(
            !unassigned_html.contains("term-attach"),
            "the Unassigned page has no project-scoped attach route, so it must render no attach markup: {unassigned_html}"
        );
    }

    /// The section nav rides in the top bar, and the control that makes a
    /// new pane shares the pane strip's own row — the two bands they used to
    /// own between the bar and the screen are gone, which is the whole point
    /// of moving them.
    #[test]
    fn terminal_page_puts_the_section_nav_in_the_bar_and_new_shell_on_the_pane_row() {
        let project = sample_project();
        let html = terminal_page(&project, &[], None, &[]);
        let bar_start = html.find("<header class=\"topbar\">").expect("no top bar");
        let bar_end = html.find("</header>").expect("unclosed top bar");
        let bar = &html[bar_start..bar_end];
        assert!(
            bar.contains("class=\"proj-tabs\""),
            "the section nav must ride in the top bar: {html}"
        );
        assert!(
            !html[bar_end..].contains("class=\"proj-tabs\""),
            "the section nav must not also open a band under the bar: {html}"
        );
        let row_start = html.find("class=\"pane-bar\"").expect("no pane row");
        let row_end = html[row_start..]
            .find("<div class=\"term-panes\">")
            .map(|i| row_start + i)
            .expect("no pane list after the pane row");
        assert!(
            html[row_start..row_end].contains("class=\"term-create__pane\""),
            "New shell must share the pane strip's row: {html}"
        );
    }

    /// agent-switch-drawer-2: the terminal page — both a project's own pane
    /// list and its `/pane/:pane_id` render, which share this one function —
    /// carries the right-edge agent switcher drawer, and the checkbox that
    /// opens it is the hook `assets/app.js`'s poller and the generic
    /// `.js-menu` handler both key off.
    #[test]
    fn terminal_page_renders_the_agent_switch_drawer() {
        let project = sample_project();
        let list_html = terminal_page(&project, &[], None, &[]);
        assert!(
            list_html.contains(r#"id="agent-drawer-toggle""#)
                && list_html.contains(r#"data-agent-drawer-list"#),
            "the terminal list page must render the agent switch drawer: {list_html}"
        );

        let panes = vec![TerminalPaneView {
            pane_id: "w1:p1".into(),
            kind: "claude".into(),
            name: "one".into(),
            status: "working".into(),
            title: String::new(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        }];
        let pane_html = terminal_page(&project, &panes, Some("w1:p1"), &[]);
        assert!(
            pane_html.contains(r#"id="agent-drawer-toggle""#),
            "the per-pane terminal page must render the agent switch drawer too: {pane_html}"
        );
    }

    /// Everything in the bar that navigates away from this page — the section
    /// tabs and the Settings link — lives inside one menu. The theme toggle
    /// is not in it: it changes this page rather than leaving it, and stays
    /// reachable in one press at every width.
    #[test]
    fn the_bars_navigation_sits_in_one_no_script_menu() {
        let project = sample_project();
        let html = terminal_page(&project, &[], None, &[]);
        let menu_start = html
            .find(r#"<div class="topbar-menu js-menu">"#)
            .expect("no top bar menu");
        let menu_end = html[menu_start..]
            .find("</header>")
            .map(|i| menu_start + i)
            .expect("unclosed top bar");
        let menu = &html[menu_start..menu_end];
        assert!(
            menu.contains("class=\"proj-tabs\"") && menu.contains(r#"href="/settings""#),
            "the section tabs and Settings must both sit in the menu: {html}"
        );
        assert!(
            !menu.contains("class=\"topbar-menu__panel\"")
                || !menu[menu.find("class=\"topbar-menu__panel\"").unwrap()..]
                    .contains("class=\"theme-toggle\""),
            "the theme toggle changes this page rather than leaving it, so it stays on the bar: {html}"
        );
        // The open state is the checkbox's own, so the menu still works on a
        // page whose script never loaded. A plain button plus a handler
        // would leave it dead there.
        assert!(
            menu.contains(r#"<input type="checkbox" id="topbar-menu-toggle" class="topbar-menu__toggle">"#)
                && menu.contains(r#"<label class="topbar-menu__button" for="topbar-menu-toggle""#),
            "the menu must open from its own checkbox, not from script: {html}"
        );
    }

    /// The pane row costs one line on a narrow screen: the tab of the pane
    /// being viewed stands outside the menu, and every pane — that one
    /// included, in its place among its siblings — plus the creation controls
    /// sit inside it.
    #[test]
    fn the_pane_bar_keeps_the_current_pane_out_and_everything_else_in_the_menu() {
        let project = sample_project();
        let panes = vec![
            TerminalPaneView {
                pane_id: "w1:p1".into(),
                kind: "claude".into(),
                name: "one".into(),
                status: "working".into(),
                title: String::new(),
                cwd: String::new(),
                workspace: "w1".into(),
                tab: "t1".into(),
            },
            TerminalPaneView {
                pane_id: "w1:p2".into(),
                kind: "shell".into(),
                name: String::new(),
                status: "shell".into(),
                title: String::new(),
                cwd: String::new(),
                workspace: "w1".into(),
                tab: "t2".into(),
            },
        ];
        let html = terminal_page(&project, &panes, Some("w1:p2"), &["Claude".to_string()]);
        let panel = html
            .find(r#"<div class="pane-menu__panel">"#)
            .expect("no pane menu panel");
        let before = &html[..panel];
        let after = &html[panel..];
        // Outside the panel: exactly one tab, the one being viewed.
        assert!(
            before.contains("pane-bar__current") && before.contains("pane/w1:p2"),
            "the viewed pane's tab must stand outside the menu: {html}"
        );
        assert!(
            !before.contains("pane/w1:p1"),
            "no other pane may sit on the row: {html}"
        );
        // Inside it: the whole strip, both panes, and the creation controls.
        assert!(
            after.contains("pane/w1:p1") && after.contains("pane/w1:p2"),
            "every pane must be reachable from the menu: {html}"
        );
        assert!(
            after.contains("class=\"term-create__pane\"") && after.contains(r#"data-preset="Claude""#),
            "the creation controls must move into the menu: {html}"
        );
        assert!(
            html.contains(r#"<input type="checkbox" id="pane-menu-toggle" class="pane-menu__toggle">"#),
            "the pane menu must open from its own checkbox, not from script: {html}"
        );
    }

    /// With no panes there is nothing to switch between, so the row carries
    /// no menu at all — only whatever creation controls the page has.
    #[test]
    fn the_pane_bar_grows_no_menu_when_there_are_no_panes() {
        let project = sample_project();
        let html = terminal_page(&project, &[], None, &[]);
        assert!(
            !html.contains("pane-menu__panel") && !html.contains("pane-bar__current"),
            "an empty pane list must not produce a menu: {html}"
        );
        assert!(
            html.contains("class=\"term-create__pane\""),
            "the creation controls must still render: {html}"
        );
    }

    /// The menu is a menu only on a narrow screen: the stylesheet hides its
    /// control by default and reveals it under the breakpoint, so a desktop
    /// bar renders exactly as it did before the menu existed. `<details>`
    /// cannot do this — a closed one has its content hidden by the browser
    /// itself, so the wide bar would carry no navigation at all.
    #[test]
    fn the_bar_menu_is_flat_until_the_narrow_breakpoint() {
        let css = include_str!("../assets/app.css");
        let default_hidden = css
            .find(".topbar-menu__toggle,\n.topbar-menu__button {\n  display: none;\n}")
            .expect("the menu control must be hidden by default (wide screens)");
        let default_panel = css
            .find(".topbar-menu__panel {\n  display: flex;")
            .expect("the panel must lay out inline by default (wide screens)");
        let query = css
            .find("@media (max-width: 720px) {")
            .expect("no narrow-screen block");
        assert!(
            default_hidden < query && default_panel < query,
            "the wide-screen defaults must come before the narrow override, or it wins everywhere"
        );
        let narrow = &css[query..];
        assert!(
            narrow.contains(".topbar-menu__panel {\n    display: none;")
                && narrow.contains(".topbar-menu__toggle:checked ~ .topbar-menu__panel"),
            "under the breakpoint the panel must be closed until the toggle is checked"
        );
        // The pane bar's own menu follows the same rule, and must not turn
        // the wide row into a menu either.
        let pane_hidden = css
            .find(".pane-menu__toggle,\n.pane-menu__button {\n  display: none;\n}")
            .expect("the pane menu control must be hidden by default");
        let standalone_hidden = css
            .find(".pane-bar > .pane-bar__current {\n  display: none;\n}")
            .expect(
                "the standalone active tab must be hidden by default, and by a selector that \
                 outranks .pane-strip__tab's own display",
            );
        assert!(
            pane_hidden < query && standalone_hidden < query,
            "the pane bar's wide-screen defaults must come before the narrow override"
        );
        assert!(
            narrow.contains(".pane-menu__panel {\n    display: none;")
                && narrow.contains(".pane-menu__toggle:checked ~ .pane-menu__panel"),
            "under the breakpoint the pane panel must be closed until the toggle is checked"
        );
    }

    /// agent-terminal-13, must-have: "with no presets configured, the
    /// creation control offers nothing" — zero preset buttons render, while
    /// the plain-shell button (which needs no preset) still does.
    #[test]
    fn terminal_page_renders_no_preset_controls_when_none_configured() {
        let project = sample_project();
        let html = terminal_page(&project, &[], None, &[]);
        // Checked as rendered HTML attribute shapes, not bare substrings:
        // `TERMINAL_CREATE_SCRIPT` itself contains the literal selector
        // `.term-create__agent[data-preset]` and `getAttribute("data-preset")`
        // on every render regardless of preset count, so a plain
        // `.contains("term-create__agent")` would false-negative here.
        assert!(!html.contains("class=\"term-create__agent\""), "{html}");
        assert!(!html.contains("data-preset=\""), "{html}");
        assert!(html.contains("class=\"term-create__pane\""), "{html}");
    }

    /// A preset label carrying HTML metacharacters must render escaped, the
    /// same discipline every other operator/user-controlled string in this
    /// module follows.
    #[test]
    fn terminal_create_controls_escapes_preset_labels() {
        let html = terminal_create_controls("proj-1", &["<script>alert(1)</script>".to_string()]);
        assert!(!html.contains("<script>alert(1)</script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    /// D5 (terminal-pane-scope), amended 2026-08-08 (term-key-height):
    /// every key in the row — Enter, Esc, Tab and the arrows — stands 44px
    /// tall, so the row reads as one band. The arrows alone keep the 44px
    /// minimum WIDTH and the body-size glyph (pressed repeatedly, often
    /// with a thumb); they are picked out by their own `.term-keys--move`
    /// modifier rather than by being a direct child of `.term-controls`,
    /// which since the merge reaches the named keys too. The scroll pair
    /// (`.term-scroll`) and the reply buttons
    /// (`.term-reply__send`/`.term-reply__stage`) carry no such rule.
    #[test]
    fn terminal_key_rows_share_one_height_and_arrows_keep_the_wider_box() {
        let project = sample_project();
        let html = terminal_page(&project, &[], None, &[]);
        assert!(
            html.contains(".term-keys button { padding: var(--space-1) var(--space-2); min-height: 44px;"),
            "the whole key row must stand 44px tall: {html}"
        );
        assert!(
            html.contains(".term-keys--move button { min-width: 44px; font-size: var(--type-body-size); }"),
            "the arrow group must keep its 44px minimum width at body-size type: {html}"
        );
        // The markup carrying that modifier is pinned by the route test
        // `terminal_page_renders_the_reply_bar_and_key_buttons`; this fixture
        // has no panes, so only the stylesheet is in reach here.
        assert!(
            !html.contains(".term-controls > .term-keys button { min-width: 44px")
                && !html.contains(".term-controls .term-keys button { min-width: 44px"),
            "a positional rule would reach the named keys too, so it must not exist: {html}"
        );
        assert!(
            !html.contains(".term-scroll button { min-width: 44px") && !html.contains(".term-scroll { min-width: 44px"),
            "the scroll pair must carry no such rule: {html}"
        );
        assert!(
            !html.contains(".term-reply__send { min-width: 44px") && !html.contains(".term-reply__stage { min-width: 44px"),
            "the reply buttons must carry no such rule: {html}"
        );
    }

    /// toa-4 (D9): the settings page offers the Unassigned group's own
    /// switch, reflects the stored config value (unchecked by default,
    /// checked once turned on), and states plainly — not softened — what
    /// turning it on opens: every agent pane on this machine, including
    /// ones outside any project, readable and writable through the
    /// browser.
    #[test]
    fn settings_page_offers_the_unassigned_switch_with_plain_wording_and_reflects_its_value() {
        let cfg_off = Config::default();
        let html_off = settings_page(&cfg_off, false, false, NotifyCredentialView::NotConfigured);
        assert!(
            html_off.contains(r#"name="unassigned_enabled""#),
            "the settings page must offer the Unassigned group's own switch: {html_off}"
        );
        assert!(
            !html_off.contains(r#"name="unassigned_enabled" checked"#),
            "the switch must render unchecked when the stored config is off: {html_off}"
        );
        assert!(
            html_off.contains("every agent pane on this machine") && html_off.contains("readable and writable"),
            "the switch's own wording must say plainly what turning it on opens, not softened: {html_off}"
        );

        let mut cfg_on = Config::default();
        cfg_on.terminal.unassigned_enabled = true;
        let html_on = settings_page(&cfg_on, false, false, NotifyCredentialView::NotConfigured);
        assert!(
            html_on.contains(r#"name="unassigned_enabled" checked"#),
            "the switch must render checked once the stored config is on: {html_on}"
        );
    }

    /// (regression, board-finished-wins-1) A feature that has already
    /// closed — every cell archived, `phase` at bee's own terminal
    /// `"compounding-complete"` — kept rendering under Waiting on you,
    /// because `state.json` still names it active and `.bee/HANDOFF.json`
    /// still reads as a pause, and that pull was evaluated before the
    /// Finished branch. The card then showed "No cells recorded.", since
    /// the Waiting branch counts live cells only and a closed feature has
    /// none. Finished now wins whenever no live cell is left: the card
    /// lands under Finished with its archived done/total.
    #[test]
    fn hub_sends_a_closed_feature_to_finished_even_while_a_pause_handoff_names_it() {
        let root = std::env::temp_dir().join(format!("mdview-views-hub-closed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/state.json",
            r#"{
                "feature": "closed-feat",
                "phase": "compounding-complete",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": false}
            }"#,
        );
        write(
            ".bee/HANDOFF.json",
            r#"{"written_at": "2026-08-10T09:00:00Z", "next_action": "Nothing pending from me.", "kind": "pause"}"#,
        );
        for id in ["cf-1", "cf-2"] {
            write(
                &format!(".bee/cells/archive/closed-feat/{id}.json"),
                &format!(
                    r#"{{
                        "id": "{id}",
                        "feature": "closed-feat",
                        "lane": "tiny",
                        "title": "Cell {id}",
                        "action": "do the thing",
                        "verify": "cargo test",
                        "files": [],
                        "read_first": [],
                        "deps": [],
                        "decisions": [],
                        "must_haves": {{}},
                        "behavior_change": false,
                        "change_class": "behavior",
                        "pbi": null,
                        "status": "capped",
                        "tier": "generation",
                        "trace": {{"worker": "w1", "claimed_at": "2026-08-10T08:00:00Z", "capped_at": "2026-08-10T08:30:00Z"}}
                    }}"#
                ),
            );
        }

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(
            html.contains(r#"data-hub-group="waiting" data-hub-count="0""#),
            "a closed feature owes no decision, so Waiting on you must be empty: {html}"
        );
        assert!(
            !html.contains(r#"data-hub-group="waiting" href="/p/proj-1/_bee/feature/closed-feat""#),
            "the closed feature must render no Waiting card at all: {html}"
        );
        assert!(
            html.contains(r#"data-hub-group="finished" href="/p/proj-1/_bee/feature/closed-feat""#),
            "the closed feature belongs under Finished: {html}"
        );
        assert!(
            html.contains("2/2 cells done"),
            "its card must count its archived cells, not the empty live set: {html}"
        );
        assert!(
            !html.contains("No cells recorded."),
            "\"No cells recorded.\" was the live-count leak this fixes: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
