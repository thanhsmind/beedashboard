//! Server-rendered HTML views. Self-contained: layout + CSS + JS as consts.
//! Theme is CSS-variable driven (no-flash head script); code colors come from
//! `/highlight.css` (syntect class-based), so themes switch without re-render.

use mdview_core::bee::{
    feature_cell_span, list_archived_feature_dirs, BeeApprovedGates, BeeBacklog, BeeBuckets,
    BeeCell, BeeDecisionSummary, BeeFeaturePhase, BeePbi, BeeProjectRollup, BeeReview,
    BeeReviewStatus, BeeSession, BeeShippedFeature, BeeSnapshot, BeeState, BeeWorkspace,
    BeeWorktree,
};
use mdview_core::code_source::DirListing;
use mdview_core::config::Config;
use mdview_core::domain::{IndexedFile, Project, RenderedPage, SearchResult};
use mdview_core::render::HighlightedSource;

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
    let body = format!(
        r#"{topbar}
{main}"#,
        topbar = topbar(""),
        main = project_list_main(projects, unassigned_visible, suggestions, register_error),
    );
    layout("Projects", "", &body)
}

/// cross-board (D1, superseded): the home page `/` is ordered Features
/// (cross-project), then this exact project list, unmoved and unreordered
/// inside itself — nothing in [`project_list_main`] changes for this
/// feature. The cross-project Live section D1 originally placed above
/// Features shipped, then was dropped after the user saw it run
/// (`docs/history/board-drop-live/CONTEXT.md`); this function now emits no
/// Live markup at all. `cross_features_html`
/// ([`bee_cross_project_features_section`]) is the caller's own decision of
/// what to show (`server.rs::index_page` applies D8's qualification and
/// D9's empty rule before calling this); empty is treated as "nothing
/// qualified" and this function returns exactly [`project_list_page`]'s own
/// output, not a byte different -- D9's "the page is what it is today" is
/// met by construction, not by matching markup by hand. Otherwise the
/// section renders inside its own themed `<main>` (the same
/// `.bee-hub-theme` scoping [`bee_board_page`] uses, reusing its
/// [`bee_hub_style`] rather than declaring new tokens), directly above the
/// unthemed project list `<main>` [`project_list_main`] already renders.
pub fn home_page(
    projects: &[(Project, usize, Vec<TerminalPaneView>)],
    unassigned_visible: bool,
    suggestions: &[ProjectSuggestion],
    register_error: Option<&str>,
    cross_features_html: &str,
) -> String {
    if cross_features_html.is_empty() {
        return project_list_page(projects, unassigned_visible, suggestions, register_error);
    }
    let body = format!(
        r#"{topbar}
{style}
<main class="fg-page bee-hub-theme">
  {features}
</main>
{list_main}"#,
        topbar = topbar(""),
        style = bee_hub_style(),
        features = cross_features_html,
        list_main = project_list_main(projects, unassigned_visible, suggestions, register_error),
    );
    layout("Projects", "", &body)
}

/// The project list itself — everything [`project_list_page`] used to build
/// inline, factored out so [`home_page`] can render it unchanged beneath the
/// cross-project sections (cross-board D1) without duplicating a single line
/// of this logic. Returns just the `<main>` element; topbar and `layout`
/// wrapping stay each caller's own job.
fn project_list_main(
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
    format!(
        r#"<main class="fg-page"><h2 class="fg-pagehead__title">Projects</h2>{register_banner}{add_form}{listing}{suggestions_block}{unassigned_card}</main>"#,
        register_banner = register_banner,
        add_form = project_add_form(),
        listing = listing,
        suggestions_block = suggestions_block,
        unassigned_card = unassigned_card,
    )
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
    terminal_badges_nav(project_id, panes, "Terminal panes")
}

/// The badge markup itself, factored out of [`project_badges`]
/// (card-terminals-1) so [`bee_hub_card`] can reuse the exact same nav/pill
/// markup for a feature card's own checkout panes -- same classes, same
/// per-pane anchor to `/p/{project_id}/_terminal/pane/{pane_id}`, same
/// [`status_pill`] and program text -- while carrying its own accessible
/// label instead of `project_badges`'s "Terminal panes": a feature card's
/// panes are the terminals running in that feature's own *checkout*, not
/// panes that belong to the feature itself (a Main feature's checkout is
/// shared with every other Main feature of that project, so a label
/// claiming otherwise would be false for every one of them). An empty
/// `panes` renders no container at all, for either caller.
fn terminal_badges_nav(project_id: &str, panes: &[TerminalPaneView], aria_label: &str) -> String {
    if panes.is_empty() {
        return String::new();
    }
    let pid = esc(project_id);
    let mut out = format!(r#"<nav class="proj-row__badges" aria-label="{}">"#, esc(aria_label));
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
  .term-keys button, .term-reply__send, .term-reply__stage { padding: var(--space-2) var(--space-3); }
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
/* scroll-fab: the three screen-moving controls (Older, Newer, Live) belong
   to the screen, not to the keys that type into the pane, so they ride on
   it as a small round-button column in its lower-right corner. Two elements
   carry that: `.term-scroll` is a rail running the screen's full height
   down its right edge, anchored `absolute` against `.term-screen-wrap` —
   the element that IS the screen — and `.term-scroll__stack` is the visible
   button column inside it. The rail is what the screen bounds: the column
   can neither reach past the frame's right edge nor hang below the frame
   onto the keys and the reply composer that follow it in the flow. The
   earlier free `sticky` placement leaned on an auto side margin and a
   negative pull for the same corner, and on a wide window it drifted out of
   both of those bounds; sticky INSIDE the rail cannot, because the rail is
   its containing block.
   scroll-fab-follow: a screen taller than the phone's viewport used to park
   the column at the screen's bottom edge, off-screen for the whole scroll up
   through the history — the one place the buttons are wanted. Sticky within
   the rail keeps the column at the viewport's lower edge while any part of
   the screen is in view, and clamps it back to the screen's own bottom the
   moment the screen ends. The rail takes no pointer events so the screen
   under it stays draggable; the stack takes them back. Out of flow, the
   column still opens no row of its own above the keys, and
   `env(safe-area-inset-bottom)` layered onto the sticky offset keeps it
   clear of an iPhone's home-indicator strip.
   fab-sticks-to-bottom: the rail lays its stack out with `justify-content:
   flex-end`, and that is not cosmetic — it is what makes the sticky offset
   above mean anything. `position: sticky` with a `bottom` offset only ever
   pulls an element UP, when it would otherwise fall below the viewport's
   lower edge; it never pushes one down. As the rail's only in-flow child
   the stack started at the rail's TOP, already in view, so sticky had
   nothing to do and the column pinned to the screen's top-right corner and
   sat there through every scroll. Starting it at the rail's bottom is what
   gives sticky something to hold. `flex-end` rather than an auto margin:
   this rule pair carries no margin at all, by test, so the column can never
   be pushed out of the screen's bounds. */
.term-screen-wrap { position: relative; display: flow-root; }
.term-scroll { position: absolute; right: var(--space-3); top: var(--space-3); bottom: var(--space-3); display: flex; flex-direction: column; justify-content: flex-end; z-index: 2; width: max-content; pointer-events: none; }
.term-scroll__stack { position: sticky; bottom: calc(var(--space-3) + env(safe-area-inset-bottom)); display: flex; flex-direction: column; gap: var(--space-2); width: max-content; padding: var(--space-1); border-radius: var(--radius-sm); background: var(--color-surface-raised); box-shadow: 0 1px 4px rgb(0 0 0 / 0.35); pointer-events: auto; }
/* Each button is a fixed-size circle — equal `width`/`height`, not a
   `min-width` — so `border-radius: 50%` draws a true circle rather than a
   pill. Fixed at 44px it keeps the same touch-target floor the named keys'
   own `min-width: 44px` rule reaches for by a different route
   (`terminal_key_rows_share_one_height_and_arrows_keep_the_wider_box` pins
   that this selector still carries no literal `min-width: 44px` rule — the
   width/height pair here is that different route, not a reintroduction of
   the rule it pins absent). Newer's disabled state only dims the circle
   (`:disabled`); it never changes size, so the column's shape holds steady
   as depth changes. */
.term-scroll button { width: 44px; height: 44px; padding: 0; display: flex; align-items: center; justify-content: center; border: var(--border-width-hairline) solid var(--color-border); border-radius: 50%; background: var(--color-surface-raised); color: var(--color-text); cursor: pointer; font-size: var(--type-caption-size); line-height: 1; }
.term-scroll button:disabled { opacity: 0.4; cursor: not-allowed; }
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
///
/// card-terminals-1: `Clone` so `server.rs::project_feature_panes` can hand
/// the same project-boundary pane list to every feature card that has no
/// worktree of its own, without re-resolving it per feature.
#[derive(Clone)]
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
      <div class="term-scroll__stack">
        <button type="button" data-scroll="older" aria-label="Older">↑</button>
        <button type="button" data-scroll="newer" aria-label="Newer" disabled>↓</button>
        <button type="button" data-scroll="live" aria-label="Live">Live</button>
      </div>
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
      <button type="button" data-key="ctrl+c">Ctrl+C</button>
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
///
/// board-liveness-3 puts one thing back between `{top}` and `{board}`:
/// `{live}` ([`bee_live_strip_section`]), a single dense presence strip —
/// nowhere close to a revival of the D1 Sessions panel it sits where that
/// panel used to live; see that function's own doc comment for exactly how
/// little it carries.
pub fn bee_board_page(project: &Project, snapshot: &BeeSnapshot) -> String {
    let body = format!(
        r#"{topbar}
{style}
<main class="fg-page bee-hub-theme">
  {top}
  {live}
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
        live = bee_live_strip_section(snapshot),
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
/* board-liveness-3: the live strip's own dense-row idiom, one row per live
   session or granted worktree — deliberately never `.bee-hub__row`'s
   feature-link styling, since a strip row never links anywhere of its
   own. */
.bee-strip {{ margin-bottom: var(--space-4); }}
.bee-strip__rows {{ display: flex; flex-direction: column; gap: var(--space-1); }}
.bee-strip__row {{ display: flex; flex-wrap: wrap; align-items: baseline; gap: var(--space-2); padding: var(--space-1) var(--space-2); border-bottom: var(--border-width-hairline) solid var(--color-border); font-size: var(--type-body-sm-size); }}
.bee-strip__row:last-child {{ border-bottom: none; }}
.bee-strip__label {{ font-weight: var(--weight-strong); color: var(--color-text); }}
.bee-strip__meta {{ color: var(--color-text-muted); }}
.bee-strip__row--unresolved .bee-strip__meta {{ color: var(--color-danger); }}
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
/* hub-finished-compact: the Finished group's own dense row — name only,
   linking straight to the detail page — mirroring `.bee-done-line`'s
   one-line idiom rather than the full `.bee-hub__card` shape the other
   two groups keep; plus the nested-details toggle
   ([`bee_hub_finished_rows`]) that pages it ten rows at a time. */
.bee-hub__row {{ display: block; color: var(--color-text); font-size: var(--type-body-sm-size); text-decoration: none; padding: var(--space-1) var(--space-2); border-bottom: var(--border-width-hairline) solid var(--color-border); overflow-wrap: anywhere; }}
.bee-hub__row:hover {{ color: var(--color-action); }}
/* cross-board D5/D10: the cross-project board's own project label and ship
   time on a Finished row — absent on every per-project board row, which
   passes neither and renders unchanged. */
.bee-hub__row-project {{ color: var(--color-text-subtle); font-size: var(--type-caption-size); }}
.bee-hub__row-time {{ color: var(--color-text-subtle); font-size: var(--type-caption-size); float: right; }}
.bee-hub__more {{ margin-top: var(--space-1); }}
.bee-hub__more-summary {{ cursor: pointer; list-style: none; color: var(--color-text-subtle); font-size: var(--type-caption-size); padding: var(--space-1) var(--space-2); }}
.bee-hub__more-summary::-webkit-details-marker {{ display: none; }}
.bee-hub__more-summary:hover {{ color: var(--color-action); }}
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

/// board-liveness-3's "something is running right now" strip, rendered by
/// [`bee_board_page`] directly above [`bee_feature_hub_section`] (see that
/// function's own doc comment for the exact position). **This is not a
/// revival of the retired `bee_sessions_panel`** (board-trim D1 dropped
/// that panel's three cards — sessions, worktrees, workspaces — on
/// purpose, along with the Working-on-now card's own Running now
/// subsection board-declutter dropped before it). Nothing here restores
/// any of that: this is a single dense strip, one plain row per live
/// session and per granted worktree, carrying no card, no
/// worktree/workspace detail block, and no process-health reading of its
/// own — its only job is presence, something this board previously had no
/// way to say at all once those panels were gone.
///
/// One row per live session (`BeeSession.live`, `SESSION_LIVE_MINUTES`):
/// its own `lane` when the session record carries one, else the globally
/// active feature (`state.feature`) when it does not, else the plain
/// fallback "no active lane" — that label's own phase, read from
/// `snapshot.phase_board` (the same lanes-∪-active-feature union
/// [`bee_feature_hub_section`] already reads, so a lane-less session bound
/// to the active feature still gets a real phase whenever `state.json`
/// carries one), its heartbeat age via [`bee_relative_minutes`], and its
/// workspace: the matching `snapshot.workspaces` entry's own `root` when
/// `workspace_id` resolves to one, else the raw `workspace_id` verbatim,
/// else "no workspace recorded".
///
/// One row per granted worktree in `snapshot.worktrees`
/// ([`BeeWorktree`]): its own `branch` and the feature its own
/// `state.json` names active, when `resolved`. An unresolved grant — a
/// dangling directory, a missing or malformed `state.json` — renders its
/// own row naming `unresolved_reason` rather than being silently dropped,
/// exactly `BeeWorktree`'s own contract for that case.
///
/// Nothing live at all (no live session, no worktree grant) renders one
/// plain `<p class="fg-empty">` line rather than disappearing — the
/// section itself is always present, so "nothing is running" is always a
/// stated fact on this page, never an absent element a reader could
/// mistake for a rendering bug.
fn bee_live_strip_section(snapshot: &BeeSnapshot) -> String {
    let active_feature = snapshot.state.as_ref().and_then(|s| s.feature.as_deref());

    let mut live_sessions: Vec<&BeeSession> = snapshot.sessions.iter().filter(|s| s.live).collect();
    live_sessions.sort_by(|a, b| a.id.cmp(&b.id));

    let mut rows = String::new();
    let mut row_count = 0usize;

    for s in &live_sessions {
        let label = s.lane.as_deref().or(active_feature).unwrap_or("no active lane");
        let phase = snapshot
            .phase_board
            .iter()
            .find(|f| f.feature == label)
            .and_then(|f| f.phase.as_deref())
            .unwrap_or("phase unknown");
        let heartbeat = bee_relative_minutes(s.heartbeat_age_minutes);
        // `BeeWorkspace.root` arrives relativized against the project root,
        // so the MAIN workspace — whose root IS that root — arrives as the
        // empty string. Naming it by its own id ("main") beats trailing a
        // separator with nothing after it, which reads as a rendering bug.
        let workspace = s.workspace_id.as_ref().and_then(|id| {
            let named = snapshot
                .workspaces
                .iter()
                .find(|w| &w.id == id)
                .map(|w| w.root.trim())
                .filter(|root| !root.is_empty())
                .unwrap_or(id.as_str());
            (!named.is_empty()).then(|| named.to_string())
        });
        let workspace_html = match &workspace {
            Some(name) => format!(" · {}", esc(name)),
            None => String::new(),
        };
        rows.push_str(&format!(
            r#"<div class="bee-strip__row" data-live-kind="session"><span class="bee-strip__label">{label}</span><span class="bee-strip__meta">{phase} · beat {heartbeat}{workspace_html}</span></div>"#,
            label = esc(label),
            phase = esc(phase),
            heartbeat = esc(&heartbeat),
            workspace_html = workspace_html,
        ));
        row_count += 1;
    }

    for w in &snapshot.worktrees {
        if !w.resolved {
            let reason = w.unresolved_reason.as_deref().unwrap_or("unknown reason");
            rows.push_str(&format!(
                r#"<div class="bee-strip__row bee-strip__row--unresolved" data-live-kind="worktree-unresolved"><span class="bee-strip__label">Worktree {id}</span><span class="bee-strip__meta">could not be read: {reason}</span></div>"#,
                id = esc(&w.id),
                reason = esc(reason),
            ));
            row_count += 1;
            continue;
        }
        let branch = w.branch.as_deref().unwrap_or("unknown branch");
        let feature = w.feature.as_deref().unwrap_or("no active feature");
        rows.push_str(&format!(
            r#"<div class="bee-strip__row" data-live-kind="worktree"><span class="bee-strip__label">{branch}</span><span class="bee-strip__meta">{feature}</span></div>"#,
            branch = esc(branch),
            feature = esc(feature),
        ));
        row_count += 1;
    }

    if row_count == 0 {
        return r#"<section class="fg-card bee-strip" data-live-rows="0">
  <h3 class="bee-panel__head">Live</h3>
  <p class="fg-empty">Nothing is running right now.</p>
</section>"#
            .to_string();
    }

    format!(
        r#"<section class="fg-card bee-strip" data-live-rows="{row_count}"><h3 class="bee-panel__head">Live</h3><div class="bee-strip__rows">{rows}</div></section>"#,
        row_count = row_count,
        rows = rows,
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
/// A feature counts as **live work** (`has_live_work`, board-liveness-2)
/// when ANY of: it has `doing`/`waiting`/`stuck` cells; it is
/// `state.feature`, the globally active one, even with none yet; a live
/// session (`BeeSession.live`, `SESSION_LIVE_MINUTES`) carries a `lane`
/// naming this feature; or a granted worktree in `snapshot.worktrees`
/// names this feature as its own active one. A session heartbeat or a
/// worktree grant is a real signal between units of work — every cell
/// capped, nothing `doing` — that the pre-liveness cell-only count missed
/// entirely, reporting "Waiting 0 / In Progress 0" while two sessions were
/// actively on the repo. Deliberately not phase-based: a lane parked at
/// `swarming`/`exploring` with no session, no grant and no live cell is
/// exactly the D4 ghost shape this rule must never resurrect.
///
/// - **Waiting on you**: a feature with live work whose current-stop gate
///   ([`bee_gate_current_stop`], reused from the retired Review column:
///   the independent-review gate itself never counts, since that gate is
///   user-invoked on its own schedule, never a blocking stop) is still
///   unapproved, OR the active feature while `.bee/HANDOFF.json` reads as
///   a genuine pause (never a `"planned-next"` clean stop) — the note
///   carries no feature name of its own (`compute_attention_items`'s own
///   doc comment says so), so it is folded onto whichever feature
///   `state.json` currently names active. Either pull is gated on
///   `working_now` (`waiting-means-stopped-1`): a live session
///   (`BeeSession.live`, `SESSION_LIVE_MINUTES`) bound to this feature's
///   lane whose own `heartbeat_age_minutes` is inside [`WORKING_MINUTES`]
///   — tighter than `SESSION_LIVE_MINUTES` on purpose (see that constant's
///   own doc). An agent still actively on the feature right now — mid
///   interview, gate not yet asked for — is In Progress, never Waiting; a
///   granted worktree never counts toward `working_now` on its own, since
///   a grant is not a heartbeat and a parked worktree with a gate owed is
///   exactly the case Waiting exists for. Either pull also yields to
///   Finished when the feature has no live cells left: a closed feature
///   owes no decision, and `state.feature`/a bound session/a granted
///   worktree can all keep naming it long after its last cell was
///   archived.
/// - **In Progress**: everything left with live work not already claimed
///   by Waiting, and not yielding to Finished.
/// - **Finished**: everything left with no live *cells* left (a bound
///   session or a granted worktree naming a closed feature's lane never
///   drags it back out — board-finished-wins-1) AND either a lane `phase`
///   of exactly `"compounding-complete"` (bee's own terminal phase —
///   `"terminal"` is a string bee never writes) OR a
///   `.bee/cells/archive/<feature>/` directory of its own
///   (`list_archived_feature_dirs`, checked once up front and reused as a
///   set — no extra store read per feature), including every feature that
///   directory names but never had a lane or active-feature placement at
///   all. Both sourced from `read_archived_cells` for their own
///   done/total counts and last activity, since a finished feature's live
///   `cell_counts` are typically zero (its cells already moved to
///   archive). A feature that fits neither rule (a pre-build, zero-cell
///   lane with no live session and no worktree grant, e.g. still
///   `exploring`) renders nowhere on this list — the pre-redesign board
///   never showed it either, since it never held a cell of its own.
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
/// How fresh a bound session's heartbeat must be for its feature to count
/// as **working now** (`waiting-means-stopped-1`) — deliberately far
/// tighter than `mdview-core`'s own [`SESSION_LIVE_MINUTES`] (30.0), which
/// stays exactly as it is and keeps driving `BeeSession.live` and the Live
/// strip: a terminal left open for twenty minutes still earns its strip
/// row. The two windows answer different questions. `SESSION_LIVE_MINUTES`
/// asks "is this session's heartbeat still worth showing at all" — a
/// generous window, since a stale strip row is merely clutter.
/// `WORKING_MINUTES` asks "is an agent actively at the keyboard on THIS
/// feature right now" — the answer that decides whether an unapproved gate
/// or a pause handoff still owes the owner a decision, or whether the
/// agent already picked it back up. Five minutes is long enough to survive
/// a single tool call or a thinking pause, short enough that a session
/// that truly went idle — mid-interview one minute ago, parked twenty
/// minutes later — falls back out of "working" well before its heartbeat
/// goes fully stale at thirty.
const WORKING_MINUTES: f64 = 5.0;

/// Every Waiting or In Progress card ([`bee_hub_card`]) names its feature,
/// links to its own detail page, its own done/total cell progress, its own
/// last-activity age ([`bee_fmt_trace_time`]), a worktree-state chip
/// ([`bee_hub_worktree_chip`]) and a status chip naming its own group.
/// hub-finished-compact strips all of that from the Finished group: a
/// closed feature owes no decision and no progress reading, so each of its
/// entries is one dense row carrying only its name
/// ([`bee_hub_finished_row`]), and the column itself pages ten rows at a
/// time behind nested `<details>` ([`bee_hub_finished_rows`]) rather than
/// growing without bound as more features close. Every path-shaped value a
/// `BeeCell`/`BeeFeaturePhase` carries already arrives relativized by
/// `mdview_core::bee::read_snapshot` (D9), so nothing further is redacted
/// here -- this view only escapes for HTML safety.
///
/// cross-board-2 splits what used to be one function into two seams: this
/// one reads `project`'s own archive off disk (D2 keeps that read exactly
/// where it always lived) and hands the result to [`bee_classify_features`],
/// which decides -- as plain data, no HTML -- which of the three columns
/// each feature belongs in; [`bee_render_hub_section`] then turns that data
/// back into this exact section, unchanged. The cross-project board
/// (`bee_cross_project_features_section`) reuses only the classification
/// step: it is handed archived-feature names from cross-board-1's
/// `read_rollup` instead of reading the archive itself, so a feature still
/// lands in the same column its own project's board would put it in,
/// merged with every other project's instead of rendered alone.
fn bee_feature_hub_section(project: &Project, snapshot: &BeeSnapshot) -> String {
    let archived_features: std::collections::HashSet<String> =
        list_archived_feature_dirs(&project.root_path).into_iter().collect();
    let placements = bee_classify_features(snapshot, &archived_features);
    bee_render_hub_section(project, &placements)
}

/// One feature already sorted into one of the feature hub's three columns
/// by [`bee_classify_features`] -- the render inputs [`bee_hub_card`] or
/// [`bee_hub_finished_row`] need, captured once so the merge step
/// (`bee_cross_project_features_section`) never re-touches `BeeSnapshot`.
enum BeeHubPlacement {
    Waiting(BeeHubCardData),
    InProgress(BeeHubCardData),
    Finished(BeeHubFinishedData),
}

/// [`bee_hub_card`]'s render inputs for one Waiting or In Progress card.
struct BeeHubCardData {
    feature: String,
    done: usize,
    total: usize,
    last_activity: Option<String>,
    worktree: (String, &'static str),
    reason: Option<String>,
    docs: Option<mdview_core::bee::BeeFeatureDocs>,
}

/// [`bee_hub_finished_row`]'s render inputs for one Finished row.
struct BeeHubFinishedData {
    feature: String,
    docs: Option<mdview_core::bee::BeeFeatureDocs>,
}

/// The feature hub's own column rules (this function's former home, see
/// [`bee_feature_hub_section`]'s doc comment), factored out to plain data
/// instead of HTML so `bee_cross_project_features_section` can run them
/// once per project and merge the results into flat, multi-project columns
/// (D4) rather than re-deriving the rules. `archived_features` is this one
/// project's set of archived feature names -- the per-project caller
/// ([`bee_feature_hub_section`]) reads it off disk; the cross-project
/// caller takes it from cross-board-1's `read_rollup` instead, performing
/// no filesystem read of its own. Iteration order matches the section this
/// used to render directly: `snapshot.phase_board` sorted by feature name,
/// then every archived feature not already placed, sorted by name.
fn bee_classify_features(
    snapshot: &BeeSnapshot,
    archived_features: &std::collections::HashSet<String>,
) -> Vec<BeeHubPlacement> {
    let active_feature = snapshot.state.as_ref().and_then(|s| s.feature.as_deref());
    let handoff_is_pause = snapshot
        .handoff
        .as_ref()
        .map(|h| !matches!(h.kind.as_deref(), Some("planned-next")))
        .unwrap_or(false);

    let mut placements: Vec<BeeHubPlacement> = Vec::new();
    let mut placed: std::collections::HashSet<&str> = std::collections::HashSet::new();

    let mut features: Vec<&BeeFeaturePhase> = snapshot.phase_board.iter().collect();
    features.sort_by(|a, b| a.feature.cmp(&b.feature));

    for f in features {
        placed.insert(f.feature.as_str());
        let live = f.cell_counts.doing + f.cell_counts.waiting + f.cell_counts.stuck;
        let is_active = active_feature == Some(f.feature.as_str());
        let session_bound = snapshot
            .sessions
            .iter()
            .any(|s| s.live && s.lane.as_deref() == Some(f.feature.as_str()));
        let worktree_bound = snapshot
            .worktrees
            .iter()
            .any(|w| w.feature.as_deref() == Some(f.feature.as_str()));
        let has_live_work = live > 0 || is_active || session_bound || worktree_bound;
        // (waiting-means-stopped-1) A grant is not a heartbeat: only a
        // session's own recency counts toward "working right now", never a
        // granted worktree on its own.
        // (working-now-default-lane-1) A session running the DEFAULT
        // pipeline carries no lane of its own — it is tied to its feature
        // through `state.json`'s own `feature` instead — so matching on the
        // lane alone missed the very case this rule exists for: an agent
        // mid-interview on the active feature, its gate not yet asked for.
        // Folding a lane-less session onto `state.feature` is the same fold
        // `waiting_via_handoff` and the Live strip's own label already use.
        let working_now = snapshot.sessions.iter().any(|s| {
            let names_this_feature = match s.lane.as_deref() {
                Some(lane) => lane == f.feature.as_str(),
                None => is_active,
            };
            names_this_feature && s.heartbeat_age_minutes <= WORKING_MINUTES
        });

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

        if !finished_and_idle && !working_now && ((has_live_work && gate_stop.is_some()) || waiting_via_handoff) {
            let reason = match gate_stop {
                Some((_, label)) => format!("{label} gate awaiting your decision"),
                None => "Work is parked, waiting on your decision".to_string(),
            };
            let last_activity = bee_hub_latest_activity(bee_hub_feature_cells(&snapshot.buckets, &f.feature));
            let worktree = bee_hub_worktree_chip(&f.feature, &snapshot.worktrees, &snapshot.workspaces, false);
            let docs = snapshot.feature_docs.get(f.feature.as_str()).cloned();
            placements.push(BeeHubPlacement::Waiting(BeeHubCardData {
                feature: f.feature.clone(),
                done: f.cell_counts.done,
                total: f.cell_counts.total,
                last_activity,
                worktree,
                reason: Some(reason),
                docs,
            }));
        } else if !finished_and_idle && has_live_work {
            let last_activity = bee_hub_latest_activity(bee_hub_feature_cells(&snapshot.buckets, &f.feature));
            let worktree = bee_hub_worktree_chip(&f.feature, &snapshot.worktrees, &snapshot.workspaces, false);
            let docs = snapshot.feature_docs.get(f.feature.as_str()).cloned();
            placements.push(BeeHubPlacement::InProgress(BeeHubCardData {
                feature: f.feature.clone(),
                done: f.cell_counts.done,
                total: f.cell_counts.total,
                last_activity,
                worktree,
                reason: None,
                docs,
            }));
        } else if is_finished {
            let docs = snapshot.feature_docs.get(f.feature.as_str()).cloned();
            placements.push(BeeHubPlacement::Finished(BeeHubFinishedData { feature: f.feature.clone(), docs }));
        }
        // else: no live work, no gate/handoff pull, and neither
        // `compounding-complete` nor archived — a pre-build lane (still
        // `exploring`, no cells yet). Renders nowhere, matching the
        // pre-redesign board's own cell-only precedent.
    }

    let mut archive_only: Vec<&String> =
        archived_features.iter().filter(|name| !placed.contains(name.as_str())).collect();
    archive_only.sort();
    for feature in archive_only {
        let docs = snapshot.feature_docs.get(feature.as_str()).cloned();
        placements.push(BeeHubPlacement::Finished(BeeHubFinishedData { feature: feature.clone(), docs }));
    }

    placements
}

/// Turns [`bee_classify_features`]'s per-project placements back into
/// exactly the section [`bee_feature_hub_section`] rendered before
/// cross-board-2 (D2) -- every card and row carries no project label
/// (`bee_hub_card`/`bee_hub_finished_row`'s `project_label: None`),
/// matching this project's page having only ever shown itself. Passes an
/// empty pane slice to every card (card-terminals-1): the per-project board
/// at `/p/:id/_bee` does not read herdr today, and giving it terminal
/// badges is deliberately out of scope for that cell -- this keeps its
/// output byte-identical.
fn bee_render_hub_section(project: &Project, placements: &[BeeHubPlacement]) -> String {
    let mut waiting_cards = String::new();
    let mut in_progress_cards = String::new();
    let mut finished_rows: Vec<String> = Vec::new();
    let mut waiting_count = 0usize;
    let mut in_progress_count = 0usize;
    let mut finished_count = 0usize;

    for placement in placements {
        match placement {
            BeeHubPlacement::Waiting(data) => {
                waiting_count += 1;
                waiting_cards.push_str(&bee_hub_card(
                    &project.id,
                    &data.feature,
                    "waiting",
                    data.done,
                    data.total,
                    data.last_activity.as_deref(),
                    &data.worktree,
                    data.reason.as_deref(),
                    data.docs.as_ref(),
                    None,
                    &[],
                ));
            }
            BeeHubPlacement::InProgress(data) => {
                in_progress_count += 1;
                in_progress_cards.push_str(&bee_hub_card(
                    &project.id,
                    &data.feature,
                    "in-progress",
                    data.done,
                    data.total,
                    data.last_activity.as_deref(),
                    &data.worktree,
                    data.reason.as_deref(),
                    data.docs.as_ref(),
                    None,
                    &[],
                ));
            }
            BeeHubPlacement::Finished(data) => {
                finished_count += 1;
                finished_rows.push(bee_hub_finished_row(&project.id, &data.feature, data.docs.as_ref(), None, None));
            }
        }
    }
    let finished_cards = bee_hub_finished_rows(&finished_rows);

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

/// The cross-project board's Features section
/// (`docs/history/cross-board/CONTEXT.md` D1/D3/D4/D5/D7/D10): runs
/// [`bee_classify_features`] once per `(project, rollup)` pair -- the exact
/// column rules the per-project board applies to itself -- then merges the
/// results into three flat, multi-project columns instead of one block per
/// project (D4), labels every card and Finished row with its own project's
/// name (D5), and orders and caps the merged Finished sequence per D10/D7:
/// every feature with a ship time first, most recently shipped first, each
/// row showing that time; then every feature without one, alphabetically by
/// feature name across all projects -- concatenating each project's
/// already-sorted list would not be globally sorted, so the merge sorts
/// again. The column counts beside each heading are the sum across
/// projects. Archived-feature names and D10 ship times come from
/// `rollup.archived_features` (cross-board-1's `read_rollup`) -- this
/// function performs no filesystem read of its own. An empty `rollups`
/// still renders the same three empty columns [`bee_hub_group`] always
/// shows for a column with nothing in it; whether to call this at all when
/// nothing qualifies (D9) is the caller's decision.
///
/// `feature_panes` (card-terminals-1) is the already-resolved join
/// (`server.rs::project_feature_panes`): for each project id, a map from
/// feature name to the terminal panes running in that feature's own
/// checkout. This function performs no boundary resolution of its own --
/// it only looks the pane list up per `(project.id, feature)` and hands it
/// to [`bee_hub_card`]. A project or feature absent from the map (the
/// switch off, herdr unreachable, or genuinely no pane) looks up to an
/// empty slice, which `bee_hub_card` renders as no badge container.
pub fn bee_cross_project_features_section(
    rollups: &[(&Project, &BeeProjectRollup)],
    feature_panes: &std::collections::HashMap<String, std::collections::HashMap<String, Vec<TerminalPaneView>>>,
) -> String {
    let mut waiting_cards = String::new();
    let mut in_progress_cards = String::new();
    let mut waiting_count = 0usize;
    let mut in_progress_count = 0usize;
    let no_panes: Vec<TerminalPaneView> = Vec::new();

    struct FinishedEntry {
        shipped_at: Option<time::OffsetDateTime>,
        feature: String,
        html: String,
    }
    let mut finished: Vec<FinishedEntry> = Vec::new();
    let rfc3339 = time::format_description::well_known::Rfc3339;

    for (project, rollup) in rollups {
        let archived_names: std::collections::HashSet<String> =
            rollup.archived_features.iter().map(|a| a.feature.clone()).collect();
        let shipped_by_feature: std::collections::HashMap<&str, Option<&str>> = rollup
            .archived_features
            .iter()
            .map(|a| (a.feature.as_str(), a.shipped_at.as_deref()))
            .collect();
        let project_panes = feature_panes.get(project.id.as_str());

        let placements = bee_classify_features(&rollup.snapshot, &archived_names);
        for placement in placements {
            match placement {
                BeeHubPlacement::Waiting(data) => {
                    waiting_count += 1;
                    let panes = project_panes
                        .and_then(|m| m.get(data.feature.as_str()))
                        .unwrap_or(&no_panes);
                    waiting_cards.push_str(&bee_hub_card(
                        &project.id,
                        &data.feature,
                        "waiting",
                        data.done,
                        data.total,
                        data.last_activity.as_deref(),
                        &data.worktree,
                        data.reason.as_deref(),
                        data.docs.as_ref(),
                        Some(&project.name),
                        panes,
                    ));
                }
                BeeHubPlacement::InProgress(data) => {
                    in_progress_count += 1;
                    let panes = project_panes
                        .and_then(|m| m.get(data.feature.as_str()))
                        .unwrap_or(&no_panes);
                    in_progress_cards.push_str(&bee_hub_card(
                        &project.id,
                        &data.feature,
                        "in-progress",
                        data.done,
                        data.total,
                        data.last_activity.as_deref(),
                        &data.worktree,
                        data.reason.as_deref(),
                        data.docs.as_ref(),
                        Some(&project.name),
                        panes,
                    ));
                }
                BeeHubPlacement::Finished(data) => {
                    let shipped_at_str = shipped_by_feature.get(data.feature.as_str()).copied().flatten();
                    let parsed = shipped_at_str.and_then(|s| time::OffsetDateTime::parse(s, &rfc3339).ok());
                    let html = bee_hub_finished_row(
                        &project.id,
                        &data.feature,
                        data.docs.as_ref(),
                        Some(&project.name),
                        parsed.and(shipped_at_str),
                    );
                    finished.push(FinishedEntry { shipped_at: parsed, feature: data.feature.clone(), html });
                }
            }
        }
    }

    // D10: timed entries first, most recent first; untimed entries after,
    // alphabetically by feature name across every project.
    finished.sort_by(|a, b| match (&a.shipped_at, &b.shipped_at) {
        (Some(x), Some(y)) => y.cmp(x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.feature.cmp(&b.feature),
    });
    let finished_count = finished.len();
    let finished_rows: Vec<String> = finished.into_iter().map(|e| e.html).collect();
    let finished_cards = bee_hub_finished_rows(&finished_rows);

    format!(
        r#"<section class="fg-card bee-hub" data-feature-hub="cross-project">
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

/// The gate a feature is actually stopped at, in bee's fixed order
/// (context, shape, execution, review) — `None` once nothing is owed.
/// Applied to a feature's own current-stop gate ([`bee_feature_hub_section`]'s
/// Waiting on you group).
///
/// A gate that a LATER gate has already been approved past is not a stop
/// (gate-stop-superseded-1). The scan therefore starts after the last
/// approved gate rather than at the beginning: a lane carrying
/// `context=false, shape=true, execution=true` has plainly been through
/// explore and shape whatever its context flag says, and reporting "Explore
/// gate awaiting your decision" for it names a decision nobody is waiting
/// on — the shape this rule was written from, a lane at `planning` with six
/// of seven cells capped and an unstamped context flag, sat in Waiting on
/// you for exactly that reason. The narrower alternative, dropping the
/// context gate from this rule the way `review` is dropped at the call site,
/// was rejected: it would also hide a feature whose interview really did
/// stop for an answer and never went further, which is the case the Waiting
/// group exists to catch.
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
    let last_approved = GATES.iter().rposition(|(key, _)| flag(key));
    let start = last_approved.map_or(0, |i| i + 1);
    GATES.into_iter().skip(start).find(|(key, _)| !flag(key))
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

/// One Waiting or In Progress feature card (D1) — hub-finished-compact
/// retires the Finished group's own use of this helper in favor of
/// [`bee_hub_finished_row`]'s dense line, so `group_key` in practice now
/// only ever arrives as `"waiting"` or `"in-progress"`. Name + link to its
/// own detail page, its own done/total cell progress (a `bee-progress`
/// bar, or no markup at all when `total == 0` — hub-finished-compact drops
/// the old "No cells recorded." filler paragraph), its own last-activity
/// age ([`bee_fmt_trace_time`]), its own worktree-state chip
/// ([`bee_hub_worktree_chip`]) and its own group status chip
/// ([`bee_hub_group_label`]). `reason` carries the Waiting group's own
/// "why" line (its current-stop gate, or a paused handoff) — `None` for
/// In Progress, which has no such single reason to name. `docs`
/// (feature-titles) carries this feature's own `CONTEXT.md` reader result:
/// present with a title, the card's name becomes that human title with the
/// slug demoted to a small muted subtitle beneath it, plus the boundary
/// description as one clamped line; `None`, or a title-less record, falls
/// back to the slug alone, exactly as before this feature. `project_label`
/// is cross-board D5's project name, rendered as one more chip in the
/// existing chip row when `Some`; `None` (every per-project board call)
/// renders no such chip, byte-identical to before cross-board-2.
///
/// `panes` (card-terminals-1) is the terminal panes running in this
/// feature's own checkout -- the worktree-vs-main-checkout join is already
/// resolved by the caller (`server.rs::project_feature_panes`); this
/// function only renders them, as a sibling `<nav>` after the card's own
/// `<a>` rather than nested inside it (an anchor inside an anchor is
/// invalid HTML, the same reason `project_badges` sits beside
/// `proj-row__link` rather than inside it). Reuses
/// [`terminal_badges_nav`]'s exact markup, carrying the accessible label
/// "Terminals in this checkout" rather than anything naming the feature:
/// for a Main feature the panes are shared with every other Main feature of
/// that project, so the label must not claim otherwise. Empty `panes`
/// renders no container at all -- the per-project board's own call
/// (`bee_render_hub_section`) always passes an empty slice, so this stays
/// byte-identical to before cross-board-2 there.
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
    project_label: Option<&str>,
    panes: &[TerminalPaneView],
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
        // hub-finished-compact: an empty card renders no markup at all here
        // — no fabricated "No cells recorded." paragraph — since a card
        // with genuinely nothing to report needs no line saying so.
        String::new()
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
    let project_chip_html = match project_label {
        Some(label) => format!(r#"<span class="fg-chip fg-chip--neutral">{}</span>"#, esc(label)),
        None => String::new(),
    };
    // card-terminals-1: a sibling of the card's own `<a>`, never nested
    // inside it (see this function's doc comment) -- empty when `panes` is
    // empty, so a feature with no pane in its own checkout renders no
    // container at all.
    let terminal_badges_html = terminal_badges_nav(project_id, panes, "Terminals in this checkout");
    format!(
        r#"<a class="fg-card bee-hub__card" data-hub-group="{group_key}" href="/p/{pid}/_bee/feature/{feature_href}">{name_html}<div class="bee-hub__chips">{project_chip_html}<span class="fg-chip fg-chip--{group_tone}">{group_label}</span><span class="fg-chip fg-chip--{wt_tone}">{wt_label}</span></div>{desc_html}{progress_html}{reason_html}{activity_html}</a>{terminal_badges_html}"#,
        group_key = group_key,
        pid = esc(project_id),
        feature_href = esc(feature),
        name_html = name_html,
        project_chip_html = project_chip_html,
        group_tone = group_tone,
        group_label = group_label,
        wt_tone = wt_tone,
        wt_label = esc(wt_label),
        desc_html = desc_html,
        progress_html = progress_html,
        reason_html = reason_html,
        activity_html = activity_html,
        terminal_badges_html = terminal_badges_html,
    )
}

/// One Finished row (hub-finished-compact): just the feature's own name —
/// its CONTEXT title when [`docs`] carries one, else its slug — linking to
/// its own detail page exactly like [`bee_hub_card`], and still carrying
/// `data-hub-group="finished"` so the group's own filtering/testing hooks
/// keep working. Deliberately none of `bee_hub_card`'s description,
/// progress bar, worktree chip, group chip or last-activity line: a closed
/// feature owes no decision and no progress reading, so the board only
/// needs to name it — its detail page is one click away. `project_label`
/// (cross-board D5) and `shipped_at` (cross-board D10, already relative-
/// formatted through [`bee_fmt_trace_time`]) both default to `None` for
/// every per-project board call, which renders byte-identical to before
/// cross-board-2; the cross-project board passes both.
fn bee_hub_finished_row(
    project_id: &str,
    feature: &str,
    docs: Option<&mdview_core::bee::BeeFeatureDocs>,
    project_label: Option<&str>,
    shipped_at: Option<&str>,
) -> String {
    let title = docs.and_then(|d| d.title.as_deref()).filter(|t| !t.is_empty());
    let name = title.unwrap_or(feature);
    let project_html = match project_label {
        Some(label) => format!(r#"<span class="bee-hub__row-project">{}</span> "#, esc(label)),
        None => String::new(),
    };
    let time_html = match shipped_at {
        Some(iso) => format!(r#" <span class="bee-hub__row-time">{}</span>"#, esc(&bee_fmt_trace_time(iso))),
        None => String::new(),
    };
    format!(
        r#"<a class="bee-hub__row" data-hub-group="finished" href="/p/{pid}/_bee/feature/{feature_href}">{project_html}{name}{time_html}</a>"#,
        pid = esc(project_id),
        feature_href = esc(feature),
        project_html = project_html,
        name = esc(name),
        time_html = time_html,
    )
}

/// Pages the Finished column's dense rows ([`bee_hub_finished_row`]) ten at
/// a time: the first ten render directly, in the open flow; every further
/// run of up to ten rows nests inside its own collapsed `<details>`, and
/// each further `<details>` nests inside the previous one rather than
/// sitting beside it as a sibling — so opening one reveals both its own
/// rows and the next page's own toggle in the same click. No JavaScript:
/// `assets/app.js` belongs to another live session and is off-limits to
/// this cell, and a `<details>` element needs none to collapse.
fn bee_hub_finished_rows(rows: &[String]) -> String {
    let split_at = rows.len().min(10);
    let (open, rest) = rows.split_at(split_at);
    let mut out = open.concat();
    if !rest.is_empty() {
        out.push_str(&bee_hub_finished_more(rest));
    }
    out
}

/// One nested paging level for [`bee_hub_finished_rows`] — never called
/// with an empty slice, so its own chunk always holds at least one row.
/// The summary names both how many rows this level opens directly (`n`)
/// and how many rows sit below this level in total (`remaining`: this
/// chunk plus every row still nested beneath it), so the pager never
/// leaves the reader guessing how much of the list is still hidden.
fn bee_hub_finished_more(rows: &[String]) -> String {
    let remaining = rows.len();
    let split_at = rows.len().min(10);
    let (chunk, rest) = rows.split_at(split_at);
    let mut inner = chunk.concat();
    if !rest.is_empty() {
        inner.push_str(&bee_hub_finished_more(rest));
    }
    format!(
        r#"<details class="bee-hub__more"><summary class="bee-hub__more-summary">Show {n} more · {remaining} left</summary>{inner}</details>"#,
        n = chunk.len(),
        remaining = remaining,
        inner = inner,
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
/// and the review queue by state (D7: independent review is presented as
/// owner-invoked, never as pending automatic work — see
/// [`bee_review_queue_body`]).
///
/// board-drop-findings-1 removed the third block, findings by severity.
/// `.bee/backlog.jsonl`'s finding rows are an append-only inbox: they never
/// close, so the counts only ever climb, and the list showed the twenty
/// NEWEST rather than the worst — a block that read like a work list while
/// answering no question the owner had. The reader is untouched
/// (`BeeBacklog::findings` still fills, exactly as board-trim (D1) left the
/// readers it stopped rendering), so `bee backlog findings`, the feedback
/// digest and `bee-grooming` keep their source. The sharp number survives
/// where it belongs: the review body's own open-P1 callout, which counts
/// review sessions rather than the whole log. The PBI
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
/// list and a backlog with no open items each render their own honest empty
/// state rather than a hidden section or a bare `0`.
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

    let review_body = bee_review_queue_body(review);

    format!(
        r#"<section class="fg-card bee-panel">
  <h3 class="bee-panel__head">Backlog &amp; Review</h3>
  <h4 class="bee-panel__subhead">PBIs by status</h4>
  {pbi_body}
  <h4 class="bee-panel__subhead">Review queue by state</h4>
  {review_body}
</section>"#,
        pbi_body = pbi_body,
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
    let breadcrumb = breadcrumb(project, "", &file.rel_path);
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
                "{switch}<span class=\"crumb\">{pname} / {rel}</span>",
                switch = section_switch(project, Section::Docs),
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
/// `base` is the section's root prefix under `/p/:id/` — `""` for Docs,
/// `"_code/"` for the Code section — so both sections share this one
/// function instead of a near-duplicate each.
fn breadcrumb(project: &Project, base: &str, rel_path: &str) -> String {
    let mut crumbs = format!(
        "<a href=\"/p/{pid}/{base}\">{name}</a>",
        pid = esc(&project.id),
        base = base,
        name = esc(&project.name)
    );
    for seg in rel_path.split('/').filter(|s| !s.is_empty()) {
        crumbs.push_str(&format!(" <span class=\"sep\">/</span> {}", esc(seg)));
    }
    format!("<nav class=\"breadcrumb\">{crumbs}</nav>")
}

/// Which section's page is currently active, for `section_switch`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Docs,
    Code,
}

/// Docs|Code toggle for the top bar. The only change `file_page` makes to
/// the existing Docs pages — Code pages carry it via the same function.
fn section_switch(project: &Project, active: Section) -> String {
    let pid = esc(&project.id);
    let docs_current = if active == Section::Docs {
        " aria-current=\"page\""
    } else {
        ""
    };
    let code_current = if active == Section::Code {
        " aria-current=\"page\""
    } else {
        ""
    };
    format!(
        "<nav class=\"section-switch\"><a href=\"/p/{pid}/\"{docs_current}>Docs</a><a href=\"/p/{pid}/_code/\"{code_current}>Code</a></nav>"
    )
}

/// What the main pane of a Code-section file page shows: highlighted source,
/// or a binary notice (never a garbled render of raw bytes).
pub enum CodeBody<'a> {
    Text {
        highlighted: &'a HighlightedSource,
        truncated: bool,
        size: u64,
    },
    Binary {
        size: u64,
    },
}

/// A single source file in the Code section: line-numbered highlighted
/// source (or a binary notice) plus a sidebar showing its containing
/// directory. Deliberately does not reuse `file_page` — that function is
/// bound to `IndexedFile`/`RenderedPage` and carries a TOC, backlinks, and
/// the copy-as-markdown source blob, none of which exist here.
pub fn code_page(
    project: &Project,
    rel_path: &str,
    body: CodeBody,
    sidebar: &DirListing,
) -> String {
    let active = base_name(rel_path);
    let tree = code_tree(project, sidebar, Some(active));
    let breadcrumb = breadcrumb(project, "_code/", rel_path);

    let main = match body {
        CodeBody::Binary { size } => format!(
            "<div class=\"codeview__binary\">Binary file &middot; {size} — cannot be displayed.</div>",
            size = format_size(size)
        ),
        CodeBody::Text {
            highlighted,
            truncated,
            size,
        } => {
            let banner = if truncated {
                "<div class=\"codeview__banner\">File truncated — showing the first part only.</div>"
                    .to_string()
            } else {
                String::new()
            };
            let mut rows = String::with_capacity(highlighted.lines.len() * 64);
            for (i, line) in highlighted.lines.iter().enumerate() {
                let n = i + 1;
                rows.push_str(&format!(
                    "<tr id=\"L{n}\"><td class=\"codeview__num\"><a href=\"#L{n}\">{n}</a></td><td class=\"codeview__line\"><code>{line}</code></td></tr>"
                ));
            }
            format!(
                "{banner}<div class=\"codeview__head\"><span class=\"codeview__lang\">{lang}</span> \
                 <span class=\"codeview__meta\">{lines} lines &middot; {size}</span></div>\
                 <table class=\"codeview__table\">{rows}</table>",
                banner = banner,
                lang = esc(&highlighted.syntax_name),
                lines = highlighted.lines.len(),
                size = format_size(size),
                rows = rows,
            )
        }
    };

    let body_html = format!(
        r#"{topbar}
<div class="layout">
  <aside id="sidebar" class="sidebar">{tree}</aside>
  <div class="sidebar-backdrop"></div>
  <main class="content">
    {breadcrumb}
    <div class="codeview">{main}</div>
  </main>
</div>"#,
        topbar = topbar_full(
            sidebar_toggle(),
            &format!(
                "{switch}<span class=\"crumb\">{pname} / {rel}</span>",
                switch = section_switch(project, Section::Code),
                pname = esc(&project.name),
                rel = esc(rel_path),
            ),
            // The Code view has no markdown to copy, and its own nav slot is
            // empty: the Docs/Code switch already rides in the center area
            // above. This fork's `topbar_full` carries a fourth `nav` slot the
            // upstream call sites predate.
            "",
            "",
        ),
        tree = tree,
        breadcrumb = breadcrumb,
        main = main,
    );
    layout(active, "", &body_html)
}

/// A directory in the Code section: the same listing rendered both in the
/// sidebar (compact nav) and the main pane (with sizes) — the two panes
/// serve different roles, same as a file-explorer's tree-plus-detail split.
pub fn code_dir_page(project: &Project, listing: &DirListing) -> String {
    let tree = code_tree(project, listing, None);
    let breadcrumb = breadcrumb(project, "_code/", &listing.rel_path);

    let mut rows = String::new();
    if !listing.rel_path.is_empty() {
        let parent = parent_dir(&listing.rel_path);
        rows.push_str(&format!(
            "<a class=\"codelist__row codelist__row--dir\" href=\"/p/{pid}/_code/{parent}\">.. </a>",
            pid = esc(&project.id),
            parent = esc(parent),
        ));
    }
    for entry in &listing.entries {
        let rel = child_rel(&listing.rel_path, &entry.name);
        let cls = if entry.is_dir {
            "codelist__row codelist__row--dir"
        } else {
            "codelist__row"
        };
        let label = if entry.is_dir {
            format!("{}/", entry.name)
        } else {
            entry.name.clone()
        };
        let size = if entry.is_dir {
            String::new()
        } else {
            format_size(entry.size)
        };
        rows.push_str(&format!(
            "<a class=\"{cls}\" href=\"/p/{pid}/_code/{rel}\"><span class=\"codelist__name\">{label}</span><span class=\"codelist__size\">{size}</span></a>",
            cls = cls,
            pid = esc(&project.id),
            rel = esc(&rel),
            label = esc(&label),
            size = size,
        ));
    }

    let title = if listing.rel_path.is_empty() {
        project.name.clone()
    } else {
        listing.rel_path.clone()
    };
    let body_html = format!(
        r#"{topbar}
<div class="layout">
  <aside id="sidebar" class="sidebar">{tree}</aside>
  <div class="sidebar-backdrop"></div>
  <main class="content">
    {breadcrumb}
    <div class="codelist">{rows}</div>
  </main>
</div>"#,
        topbar = topbar_full(
            sidebar_toggle(),
            &format!(
                "{switch}<span class=\"crumb\">{pname} / {rel}</span>",
                switch = section_switch(project, Section::Code),
                pname = esc(&project.name),
                rel = esc(&listing.rel_path),
            ),
            "",
            "",
        ),
        tree = tree,
        breadcrumb = breadcrumb,
        rows = rows,
    );
    layout(&title, "", &body_html)
}

/// Sidebar for the Code section: always exactly one directory's contents
/// (same "one folder, zoomable" model the Docs sidebar uses), server-
/// rendered — no client JS, unlike Docs' JSON-payload `file_tree`, because
/// there is no whole-project file list to ship (D1: no index).
fn code_tree(project: &Project, listing: &DirListing, active_file: Option<&str>) -> String {
    let mut out = String::from(
        "<div class=\"fg-sidebar-search\">\
         <input class=\"fg-input\" placeholder=\"Search…\" autocomplete=\"off\" disabled></div>\
         <nav class=\"chapter\"><div class=\"chap-sec\">Files</div>",
    );
    if !listing.rel_path.is_empty() {
        let parent = parent_dir(&listing.rel_path);
        out.push_str(&format!(
            "<a class=\"chap-file chap-dir\" href=\"/p/{pid}/_code/{parent}\">.. </a>",
            pid = esc(&project.id),
            parent = esc(parent),
        ));
    }
    for entry in &listing.entries {
        let rel = child_rel(&listing.rel_path, &entry.name);
        let is_active = !entry.is_dir && active_file == Some(entry.name.as_str());
        let cls = match (entry.is_dir, is_active) {
            (true, _) => "chap-file chap-dir",
            (false, true) => "chap-file active",
            (false, false) => "chap-file",
        };
        let label = if entry.is_dir {
            format!("{}/", entry.name)
        } else {
            entry.name.clone()
        };
        out.push_str(&format!(
            "<a class=\"{cls}\" href=\"/p/{pid}/_code/{rel}\">{label}</a>",
            cls = cls,
            pid = esc(&project.id),
            rel = esc(&rel),
            label = esc(&label),
        ));
    }
    out.push_str("</nav>");
    out
}

/// Join a directory's `rel_path` with one child name into that child's own
/// `rel_path` (root-level children have no leading slash).
fn child_rel(dir_rel_path: &str, name: &str) -> String {
    if dir_rel_path.is_empty() {
        name.to_string()
    } else {
        format!("{dir_rel_path}/{name}")
    }
}

/// Human-readable byte size (binary units — 1 KB = 1024 B), one decimal
/// place past bytes.
fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
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

/// stale-index-refresh-2: `error_page`'s sibling for the one 404 that names a
/// known project — `server.rs::project_path`'s own not-found branch, reached
/// when the project itself resolves but the requested path matches neither
/// an indexed markdown file nor an on-disk asset. A file committed or edited
/// while the daemon was down (or between the startup reconcile sweep and
/// now — `stale-index-refresh-1`) stays invisible until something reindexes
/// it, and a reader who followed a link straight to this page has no
/// terminal handy to run `mdview refresh` in. This renders `error_page`'s
/// same status/message body, then — under the message, not replacing it — a
/// plain HTML `<form>` posting to `server.rs::refresh_project`
/// (`/api/projects/<id>/refresh`) with a hidden `redirect` field carrying the
/// path the reader actually asked for, so submitting it reindexes the
/// project and lands them back where they started. Deliberately no
/// JavaScript: a form post works even with the pane's own scripts idle, and
/// every other `not_found` caller (bad project id first among them) never
/// reaches this function at all — `error_page` above still renders their
/// plain, button-less message.
pub fn error_page_with_refresh(
    status: u16,
    msg: &str,
    project_id: &str,
    requested_path: &str,
) -> String {
    let body = format!(
        r#"{topbar}
<main class="fg-page"><h2 class="fg-pagehead__title">{status}</h2><p class="fg-empty">{msg}</p>
<form class="fg-refresh-index" method="post" action="/api/projects/{project_id}/refresh">
  <input type="hidden" name="redirect" value="{redirect}">
  <button type="submit" class="fg-btn fg-btn--primary">Refresh index</button>
</form>
</main>"#,
        topbar = topbar(""),
        status = status,
        msg = esc(msg),
        project_id = esc(project_id),
        redirect = esc(requested_path),
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

    /// The Older/Newer/Live column is bounded by the screen it moves: the
    /// rail it lives in is positioned against `.term-screen-wrap`, which is
    /// the only ancestor that establishes a containing block, so every one of
    /// its offsets is measured from the screen's own edges. An auto side
    /// margin would hand the placement back to the flow and let the rail
    /// leave the frame on a wide window, which is what this pins against.
    /// scroll-fab-follow: the visible column is `sticky` INSIDE that rail —
    /// it follows the viewport up a screen taller than the phone instead of
    /// parking off-screen at the screen's bottom edge — and sticky inside an
    /// absolute rail still cannot escape the screen, which the free sticky
    /// placement this replaced could.
    #[test]
    fn the_scroll_stack_is_anchored_inside_the_screen_it_moves() {
        let project = sample_project();
        let html = terminal_page(&project, &[], None, &[]);
        assert!(
            html.contains(".term-screen-wrap { position: relative;"),
            "the screen must establish the containing block the rail anchors to: {html}"
        );
        assert!(
            html.contains(
                ".term-scroll { position: absolute; right: var(--space-3); top: var(--space-3); bottom: var(--space-3);"
            ),
            "the rail must be inset from the screen's own right, top and bottom edges: {html}"
        );
        assert!(
            !html.contains(".term-scroll { position: sticky"),
            "the rail itself is placed by the screen, never by the flow: {html}"
        );
        // (fab-sticks-to-bottom-1) Without this the sticky offset below is
        // inert: sticky-bottom only pulls an element UP when it would fall
        // past the viewport's lower edge, so a stack starting at the rail's
        // top simply stays there — the top-right corner, every scroll long.
        assert!(
            html.contains("display: flex; flex-direction: column; justify-content: flex-end;")
                && html
                    .split(".term-scroll { ")
                    .nth(1)
                    .and_then(|rest| rest.split('}').next())
                    .is_some_and(|rule| rule.contains("justify-content: flex-end")),
            "the rail must lay its stack out at the bottom, or the stack's sticky offset holds nothing: {html}"
        );
        assert!(
            html.contains(
                ".term-scroll__stack { position: sticky; bottom: calc(var(--space-3) + env(safe-area-inset-bottom));"
            ),
            "the button column must stick to the viewport's lower edge inside the rail: {html}"
        );
        for selector in [".term-scroll { ", ".term-scroll__stack { "] {
            let rule = html
                .split(selector)
                .nth(1)
                .and_then(|rest| rest.split('}').next())
                .unwrap_or_else(|| panic!("the stylesheet must carry a {selector}rule"));
            assert!(
                !rule.contains("margin"),
                "no margin may push the column out of the screen's bounds: {rule}"
            );
        }
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

    /// (regression, board-finished-wins-1; progress assertion trimmed by
    /// hub-finished-compact, whose own dropped-cell rule no longer applies
    /// anywhere in the product now that the Finished column shows no
    /// counts at all)
    /// A feature that has already closed — every cell archived, `phase` at
    /// bee's own terminal `"compounding-complete"` — kept rendering under
    /// Waiting on you, because `state.json` still names it active and
    /// `.bee/HANDOFF.json` still reads as a pause, and that pull was
    /// evaluated before the Finished branch. The card then showed
    /// "No cells recorded.", since the Waiting branch counts live cells
    /// only and a closed feature has none. Finished now wins whenever no
    /// live cell is left: the card lands under Finished — as a dense row,
    /// carrying no done/total count of its own at all
    /// (hub-finished-compact).
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
            !html.contains("cells done") && !html.contains("No cells recorded."),
            "hub-finished-compact: a Finished row carries no progress count of its own, done or empty: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (board-liveness-2) A feature with zero live cells — its lane's every
    /// gate is deliberately approved here so an unapproved gate can never be
    /// the reason it places — still renders under In Progress once a live
    /// session (`.bee/sessions/*.json`, heartbeat inside
    /// `SESSION_LIVE_MINUTES`) carries a `lane` naming it. This is exactly
    /// the shape the pre-liveness board missed: every cell capped between
    /// units of work, "In Progress 0" while a session actively worked the
    /// feature.
    #[test]
    fn hub_places_a_zero_cell_feature_with_a_live_session_bound_under_in_progress() {
        let root =
            std::env::temp_dir().join(format!("mdview-views-hub-session-bound-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/lanes/session-bound-feat.json",
            r#"{
                "feature": "session-bound-feat",
                "phase": "swarming",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": true}
            }"#,
        );
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        write(
            ".bee/sessions/live.json",
            &format!(r#"{{"id": "live", "last_heartbeat": "{now}", "lane": "session-bound-feat"}}"#),
        );

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(
            html.contains(r#"data-hub-group="in-progress" href="/p/proj-1/_bee/feature/session-bound-feat""#),
            "a zero-cell feature with a live session bound to its lane must render under In Progress: {html}"
        );
        assert!(
            !html.contains(r#"data-hub-group="waiting" href="/p/proj-1/_bee/feature/session-bound-feat""#),
            "with every gate approved, the session-bound feature must not land under Waiting: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (board-liveness-2) The same zero-cell, all-gates-approved shape as
    /// above, but the liveness signal this time is a granted worktree
    /// (`.bee/runtime/worktree-grants.json`) whose own sibling
    /// `.bee/state.json` names this feature as its own active one — never a
    /// session, never a cell.
    #[test]
    fn hub_places_a_zero_cell_feature_named_by_a_granted_worktree_under_in_progress() {
        let root = std::env::temp_dir().join(format!("mdview-views-hub-wt-bound-{}", std::process::id()));
        let sibling = std::env::temp_dir().join(format!("mdview-views-hub-wt-bound-sibling-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
        let write = |dir: &std::path::Path, rel: &str, body: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            &root,
            ".bee/lanes/wt-bound-feat.json",
            r#"{
                "feature": "wt-bound-feat",
                "phase": "swarming",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": true}
            }"#,
        );
        std::fs::create_dir_all(&sibling).unwrap();
        write(&sibling, ".bee/state.json", r#"{"phase":"swarming","feature":"wt-bound-feat","mode":"standard"}"#);
        let grant_id = sibling.file_name().unwrap().to_string_lossy().to_string();
        write(&root, ".bee/runtime/worktree-grants.json", &format!(r#"{{"{grant_id}": true}}"#));

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(
            html.contains(r#"data-hub-group="in-progress" href="/p/proj-1/_bee/feature/wt-bound-feat""#),
            "a zero-cell feature named by a granted worktree's own active feature must render under In Progress: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
    }

    /// (gate-stop-superseded-1) A gate a later gate has already been
    /// approved past is not a stop. The shape this came from: a lane at
    /// `planning` with six of seven cells capped, carrying
    /// `context=false, shape=true, execution=true`, reported "Explore gate
    /// awaiting your decision" and sat under Waiting on you.
    #[test]
    fn gate_stop_skips_a_gate_a_later_approval_already_passed() {
        let gates = |c: Option<bool>, s: Option<bool>, e: Option<bool>, r: Option<bool>| BeeApprovedGates {
            context: c,
            shape: s,
            execution: e,
            review: r,
        };

        // The reported shape: explore unstamped, but shape and execution
        // both approved — nothing is owed before review.
        let g = gates(Some(false), Some(true), Some(true), Some(false));
        assert_eq!(bee_gate_current_stop(Some(&g)), Some(("review", "Independent review")));

        // A feature that really did stop at its interview still reports it.
        let g = gates(Some(false), Some(false), Some(false), Some(false));
        assert_eq!(bee_gate_current_stop(Some(&g)), Some(("context", "Explore")));

        // Context approved, shape not: the shape gate is the stop.
        let g = gates(Some(true), Some(false), Some(false), Some(false));
        assert_eq!(bee_gate_current_stop(Some(&g)), Some(("shape", "Shape")));

        // Execution approved past an unstamped shape: review is next.
        let g = gates(Some(true), Some(false), Some(true), Some(false));
        assert_eq!(bee_gate_current_stop(Some(&g)), Some(("review", "Independent review")));

        // Everything approved: nothing owed at all.
        let g = gates(Some(true), Some(true), Some(true), Some(true));
        assert_eq!(bee_gate_current_stop(Some(&g)), None);

        // No record at all reads as nothing approved yet.
        assert_eq!(bee_gate_current_stop(None), Some(("context", "Explore")));
    }

    /// (waiting-means-stopped-1) The bug this cell fixes: an unapproved
    /// gate used to outrank every liveness signal, so a feature an agent
    /// was actively working — its own session's heartbeat a minute old —
    /// still landed under Waiting on you. A session bound to the lane
    /// whose heartbeat is inside `WORKING_MINUTES` now counts as "working
    /// right now" and keeps the card in In Progress instead.
    #[test]
    fn hub_keeps_a_gate_stopped_feature_working_right_now_under_in_progress() {
        let root = std::env::temp_dir().join(format!("mdview-views-hub-working-now-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/lanes/working-feat.json",
            r#"{
                "feature": "working-feat",
                "phase": "executing",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": true, "shape": false, "execution": false, "review": false}
            }"#,
        );
        let hb = (time::OffsetDateTime::now_utc() - time::Duration::minutes(1))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        write(
            ".bee/sessions/live.json",
            &format!(r#"{{"id": "live", "last_heartbeat": "{hb}", "lane": "working-feat"}}"#),
        );

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(
            html.contains(r#"data-hub-group="in-progress" href="/p/proj-1/_bee/feature/working-feat""#),
            "a feature whose session beat a minute ago must render under In Progress even with a gate unapproved: {html}"
        );
        assert!(
            !html.contains(r#"data-hub-group="waiting" href="/p/proj-1/_bee/feature/working-feat""#),
            "an agent actively working the feature must never see it under Waiting on you: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (regression, working-now-default-lane-1) The case that prompted the
    /// working-now rule was itself missed by it: a session running the
    /// DEFAULT pipeline carries no `lane` of its own, and is tied to its
    /// feature through `.bee/state.json`'s `feature` instead. Matching on
    /// the lane alone left the active feature parked under Waiting on you
    /// while its own agent had beaten a minute earlier. A lane-less session
    /// now folds onto `state.feature` — the same fold `waiting_via_handoff`
    /// and the Live strip's label already use — and only onto it: another
    /// feature's own waiting pull is untouched.
    #[test]
    fn hub_counts_a_lane_less_session_as_working_the_active_feature_only() {
        let root = std::env::temp_dir().join(format!("mdview-views-hub-working-default-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        // The active feature, mid-interview: its explore gate is unapproved.
        write(
            ".bee/state.json",
            r#"{
                "feature": "active-feat",
                "phase": "exploring",
                "approved_gates": {"context": false, "shape": false, "execution": false, "review": false}
            }"#,
        );
        // A second lane, also gate-stopped, that nobody is working. It
        // carries a claimed cell of its own so it has live work to be
        // stopped ON — a zero-cell lane nobody is working renders nowhere
        // at all, which would prove nothing here.
        write(
            ".bee/lanes/other-feat.json",
            r#"{
                "feature": "other-feat",
                "phase": "exploring",
                "mode": "standard",
                "approved_gates": {"context": false, "shape": false, "execution": false, "review": false}
            }"#,
        );
        write(
            ".bee/cells/other-1.json",
            r#"{
                "id": "other-1",
                "feature": "other-feat",
                "lane": "tiny",
                "title": "Cell other-1",
                "action": "do the thing",
                "verify": "cargo test",
                "files": [],
                "read_first": [],
                "deps": [],
                "decisions": [],
                "must_haves": {},
                "behavior_change": false,
                "change_class": "behavior",
                "pbi": null,
                "status": "claimed",
                "tier": "generation",
                "trace": {"worker": "w1", "claimed_at": "2026-08-10T08:00:00Z", "capped_at": null}
            }"#,
        );
        let hb = (time::OffsetDateTime::now_utc() - time::Duration::minutes(1))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        // No "lane" key at all — exactly what the default pipeline writes.
        write(
            ".bee/sessions/default.json",
            &format!(r#"{{"id": "default", "last_heartbeat": "{hb}", "workspace_id": "main"}}"#),
        );

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(
            html.contains(r#"data-hub-group="in-progress" href="/p/proj-1/_bee/feature/active-feat""#),
            "a lane-less session beating a minute ago works the ACTIVE feature: {html}"
        );
        assert!(
            !html.contains(r#"data-hub-group="waiting" href="/p/proj-1/_bee/feature/active-feat""#),
            "the active feature must not sit in Waiting while its own agent is mid-interview: {html}"
        );
        assert!(
            html.contains(r#"data-hub-group="waiting" href="/p/proj-1/_bee/feature/other-feat""#),
            "a lane-less session must not suppress some OTHER feature's own waiting pull: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (regression, working-now-default-lane-1) The same lane-less session,
    /// twenty minutes cold: past `WORKING_MINUTES`, so the active feature's
    /// unapproved gate is owed the owner a decision again.
    #[test]
    fn hub_sends_the_active_feature_to_waiting_once_its_lane_less_session_goes_cold() {
        let root = std::env::temp_dir().join(format!("mdview-views-hub-working-default-cold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/state.json",
            r#"{
                "feature": "active-feat",
                "phase": "exploring",
                "approved_gates": {"context": false, "shape": false, "execution": false, "review": false}
            }"#,
        );
        let hb = (time::OffsetDateTime::now_utc() - time::Duration::minutes(20))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        write(
            ".bee/sessions/default.json",
            &format!(r#"{{"id": "default", "last_heartbeat": "{hb}", "workspace_id": "main"}}"#),
        );

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(
            html.contains(r#"data-hub-group="waiting" href="/p/proj-1/_bee/feature/active-feat""#),
            "twenty minutes cold is not working: the gate owes a decision again: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (waiting-means-stopped-1) The same gate-stopped lane, but the bound
    /// session's heartbeat has gone stale enough (20 minutes, past
    /// `WORKING_MINUTES` but still inside `SESSION_LIVE_MINUTES`) that
    /// nobody is working it right now: the card falls back to Waiting on
    /// you, while the session itself — still live per the 30-minute
    /// window — keeps its own row on the Live strip.
    #[test]
    fn hub_sends_a_gate_stopped_feature_to_waiting_once_its_session_goes_stale_enough() {
        let root = std::env::temp_dir().join(format!("mdview-views-hub-working-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/lanes/stale-working-feat.json",
            r#"{
                "feature": "stale-working-feat",
                "phase": "executing",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": true, "shape": false, "execution": false, "review": false}
            }"#,
        );
        let hb = (time::OffsetDateTime::now_utc() - time::Duration::minutes(20))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        write(
            ".bee/sessions/live.json",
            &format!(r#"{{"id": "live", "last_heartbeat": "{hb}", "lane": "stale-working-feat"}}"#),
        );

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);
        let strip_html = bee_live_strip_section(&snapshot);

        assert!(
            html.contains(r#"data-hub-group="waiting" href="/p/proj-1/_bee/feature/stale-working-feat""#),
            "a session gone stale past WORKING_MINUTES must let the gate pull the card back to Waiting: {html}"
        );
        assert!(
            !html.contains(r#"data-hub-group="in-progress" href="/p/proj-1/_bee/feature/stale-working-feat""#),
            "a stale-past-WORKING_MINUTES session must not keep the card in In Progress: {html}"
        );
        assert!(
            strip_html.contains("stale-working-feat"),
            "the session is still live per SESSION_LIVE_MINUTES (30), so its row must stay on the Live strip: {strip_html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (waiting-means-stopped-1) A pause handoff naming the active feature,
    /// with nobody's session bound to its lane at all, must still pull the
    /// card into Waiting on you — the `working_now` gate only ever
    /// suppresses the pull when an agent really is on it.
    #[test]
    fn hub_sends_a_pause_handoff_feature_to_waiting_when_nobody_is_working_it() {
        let root = std::env::temp_dir().join(format!("mdview-views-hub-handoff-idle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/state.json",
            r#"{
                "feature": "handoff-idle-feat",
                "phase": "executing",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": true}
            }"#,
        );
        write(
            ".bee/lanes/handoff-idle-feat.json",
            r#"{
                "feature": "handoff-idle-feat",
                "phase": "executing",
                "mode": "standard",
                "next_action": "none yet",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": true}
            }"#,
        );
        write(
            ".bee/HANDOFF.json",
            r#"{"written_at": "2026-08-10T09:00:00Z", "next_action": "Nothing pending from me.", "kind": "pause"}"#,
        );

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(
            html.contains(r#"data-hub-group="waiting" href="/p/proj-1/_bee/feature/handoff-idle-feat""#),
            "a genuine pause handoff naming a feature nobody is working must still render under Waiting on you: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (waiting-means-stopped-1) A granted worktree alone — no session, no
    /// heartbeat — must never suppress the waiting pull: a grant is not a
    /// heartbeat, and a parked worktree with a gate owed is exactly the
    /// case Waiting exists for.
    #[test]
    fn hub_worktree_grant_alone_does_not_suppress_the_waiting_pull() {
        let root = std::env::temp_dir().join(format!("mdview-views-hub-wt-no-working-{}", std::process::id()));
        let sibling =
            std::env::temp_dir().join(format!("mdview-views-hub-wt-no-working-sibling-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
        let write = |dir: &std::path::Path, rel: &str, body: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            &root,
            ".bee/lanes/wt-gate-owed-feat.json",
            r#"{
                "feature": "wt-gate-owed-feat",
                "phase": "executing",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": true, "shape": false, "execution": false, "review": false}
            }"#,
        );
        std::fs::create_dir_all(&sibling).unwrap();
        write(&sibling, ".bee/state.json", r#"{"phase":"executing","feature":"wt-gate-owed-feat","mode":"standard"}"#);
        let grant_id = sibling.file_name().unwrap().to_string_lossy().to_string();
        write(&root, ".bee/runtime/worktree-grants.json", &format!(r#"{{"{grant_id}": true}}"#));

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(
            html.contains(r#"data-hub-group="waiting" href="/p/proj-1/_bee/feature/wt-gate-owed-feat""#),
            "a granted worktree with no session bound must not suppress the waiting pull: {html}"
        );
        assert!(
            !html.contains(r#"data-hub-group="in-progress" href="/p/proj-1/_bee/feature/wt-gate-owed-feat""#),
            "the worktree grant alone must not count as working now: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
    }

    /// (board-liveness-3) Existing coverage before this test: the hub tests
    /// above already prove `session_bound`/`worktree_bound` liveness
    /// signals feed feature placement; none of them render or assert
    /// anything about a strip row's own content (lane, phase, heartbeat,
    /// workspace) — that is the gap this test and its three siblings below
    /// close. A live session with a lane bound to a lane record names that
    /// lane, that lane's own phase, its heartbeat age
    /// (`bee_relative_minutes`), and its workspace's own `root`.
    #[test]
    fn live_strip_names_a_live_sessions_lane_phase_and_heartbeat_age() {
        let root = std::env::temp_dir().join(format!("mdview-views-strip-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/lanes/strip-feat.json",
            r#"{
                "feature": "strip-feat",
                "phase": "executing",
                "mode": "standard"
            }"#,
        );
        let hb = (time::OffsetDateTime::now_utc() - time::Duration::minutes(4))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        write(
            ".bee/sessions/live.json",
            &format!(r#"{{"id": "live", "last_heartbeat": "{hb}", "lane": "strip-feat", "workspace_id": "ws-1"}}"#),
        );
        write(
            ".bee/runtime/workspaces/ws-1.json",
            r#"{"id": "ws-1", "type": "worktree", "root": "sibling-dir", "branch": "wt/strip-feat", "attached_sessions": []}"#,
        );

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let html = bee_live_strip_section(&snapshot);

        assert!(html.contains("strip-feat"), "the row must name the session's own lane: {html}");
        assert!(html.contains("executing"), "the row must name that lane's phase: {html}");
        assert!(html.contains("4 minutes ago"), "the row must state the heartbeat age: {html}");
        assert!(html.contains("sibling-dir"), "the row must name the session's own workspace: {html}");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (regression, strip-workspace-label-1) `BeeWorkspace.root` arrives
    /// relativized against the project root, so the MAIN workspace — whose
    /// root IS that root — arrives as the empty string, and the row used to
    /// render "swarming · beat 2 minutes ago · " with nothing after the last
    /// separator: a dangling separator reads as a rendering bug. The row now
    /// falls back to the workspace's own id, and drops the segment entirely
    /// when the session records no workspace at all.
    #[test]
    fn live_strip_names_the_main_workspace_rather_than_trailing_an_empty_separator() {
        let root = std::env::temp_dir().join(format!("mdview-views-strip-main-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        let hb = (time::OffsetDateTime::now_utc() - time::Duration::minutes(2))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        write(
            ".bee/sessions/main-ws.json",
            &format!(r#"{{"id": "main-ws", "last_heartbeat": "{hb}", "lane": "main-feat", "workspace_id": "main"}}"#),
        );
        // The main workspace's own root is the project root itself, which is
        // exactly what relativizes away to "".
        write(
            ".bee/runtime/workspaces/main.json",
            &format!(
                r#"{{"id": "main", "type": "main", "root": "{}", "branch": "main", "attached_sessions": []}}"#,
                root.to_string_lossy()
            ),
        );
        write(
            ".bee/sessions/no-ws.json",
            &format!(r#"{{"id": "no-ws", "last_heartbeat": "{hb}", "lane": "other-feat"}}"#),
        );

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let html = bee_live_strip_section(&snapshot);

        assert!(
            html.contains("beat 2 minutes ago · main"),
            "the main workspace must be named by its own id, not relativized away: {html}"
        );
        assert!(
            !html.contains("· </span>") && !html.contains(" · <"),
            "no row may end on a separator with nothing after it: {html}"
        );
        assert!(
            html.contains("beat 2 minutes ago</span>"),
            "a session with no workspace recorded must drop the segment, separator and all: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (board-liveness-3) A resolved granted worktree — its own sibling
    /// `state.json` names an active feature, this project's own workspace
    /// record names its branch — renders its own row naming both, with no
    /// live session in the fixture at all.
    #[test]
    fn live_strip_names_a_resolved_worktrees_branch_and_active_feature() {
        let root = std::env::temp_dir().join(format!("mdview-views-strip-wt-{}", std::process::id()));
        let sibling = std::env::temp_dir().join(format!("mdview-views-strip-wt-sibling-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
        let write = |dir: &std::path::Path, rel: &str, body: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        std::fs::create_dir_all(&sibling).unwrap();
        write(&sibling, ".bee/state.json", r#"{"phase":"swarming","feature":"wt-strip-feat","mode":"standard"}"#);
        let grant_id = sibling.file_name().unwrap().to_string_lossy().to_string();
        write(
            &root,
            &format!(".bee/runtime/workspaces/{grant_id}.json"),
            &format!(r#"{{"id": "{grant_id}", "type": "worktree", "root": "wt-root", "branch": "wt/wt-strip-feat", "attached_sessions": []}}"#),
        );
        write(&root, ".bee/runtime/worktree-grants.json", &format!(r#"{{"{grant_id}": true}}"#));

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let html = bee_live_strip_section(&snapshot);

        assert!(html.contains("wt/wt-strip-feat"), "the row must name the worktree's own branch: {html}");
        assert!(html.contains("wt-strip-feat"), "the row must name the worktree's own active feature: {html}");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
    }

    /// (board-liveness-3) A granted worktree whose sibling directory does
    /// not exist at all is dangling — `resolve_worktree` reports it
    /// unresolved with a reason, never dropped — and the strip must say so
    /// in its own row rather than rendering nothing for that grant.
    #[test]
    fn live_strip_names_an_unresolved_worktree_grants_reason_rather_than_dropping_it() {
        let root = std::env::temp_dir().join(format!("mdview-views-strip-wt-unresolved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dangling_id = format!("mdview-views-strip-wt-ghost-{}", std::process::id());
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(&dangling_id));
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(".bee/runtime/worktree-grants.json", &format!(r#"{{"{dangling_id}": true}}"#));

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let html = bee_live_strip_section(&snapshot);

        assert!(
            html.contains("could not be read"),
            "an unresolved grant's own row must say it could not be read, never disappear silently: {html}"
        );
        assert!(html.contains(&dangling_id), "the unresolved row must still name the dangling grant: {html}");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (board-liveness-3) No live session and no worktree grant at all: the
    /// strip renders one honest empty line rather than an absent section.
    #[test]
    fn live_strip_renders_one_honest_line_when_nothing_is_live() {
        let root = std::env::temp_dir().join(format!("mdview-views-strip-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".bee")).unwrap();

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let html = bee_live_strip_section(&snapshot);

        assert!(
            html.contains("Nothing is running right now."),
            "with nothing live, the strip must still render an honest empty line: {html}"
        );
        assert!(html.contains("data-live-rows=\"0\""), "{html}");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (board-liveness-2) A lane parked at `swarming` with no live cells, no
    /// live session bound and no granted worktree naming it renders nowhere
    /// on the hub — the exact D4 ghost-card shape this rule must never
    /// resurrect by going phase-based instead of liveness-based.
    #[test]
    fn hub_renders_no_entry_for_a_parked_lane_with_no_liveness_signal_at_all() {
        let root = std::env::temp_dir().join(format!("mdview-views-hub-parked-lane-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/lanes/parked-feat.json",
            r#"{
                "feature": "parked-feat",
                "phase": "swarming",
                "mode": "standard",
                "next_action": "none yet"
            }"#,
        );

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(
            !html.contains("parked-feat"),
            "a parked lane with no live cell, no live session and no worktree grant must render nowhere: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (board-liveness-2, board-finished-wins-1) A closed feature — every
    /// cell archived, `phase` at `"compounding-complete"` — must stay under
    /// Finished even while BOTH a live session's own `lane` names it AND
    /// `.bee/HANDOFF.json` reads as a pause naming `state.feature`: neither
    /// signal drags a feature with no live cells left back out of Finished.
    #[test]
    fn hub_keeps_a_closed_feature_in_finished_even_with_a_bound_session_and_a_pause_handoff() {
        let root =
            std::env::temp_dir().join(format!("mdview-views-hub-closed-session-bound-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/state.json",
            r#"{
                "feature": "closed-session-feat",
                "phase": "compounding-complete",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": false}
            }"#,
        );
        write(
            ".bee/HANDOFF.json",
            r#"{"written_at": "2026-08-10T09:00:00Z", "next_action": "Nothing pending from me.", "kind": "pause"}"#,
        );
        write(
            ".bee/cells/archive/closed-session-feat/cf-1.json",
            r#"{
                "id": "cf-1",
                "feature": "closed-session-feat",
                "lane": "tiny",
                "title": "Cell cf-1",
                "action": "do the thing",
                "verify": "cargo test",
                "files": [],
                "read_first": [],
                "deps": [],
                "decisions": [],
                "must_haves": {},
                "behavior_change": false,
                "change_class": "behavior",
                "pbi": null,
                "status": "capped",
                "tier": "generation",
                "trace": {"worker": "w1", "claimed_at": "2026-08-10T08:00:00Z", "capped_at": "2026-08-10T08:30:00Z"}
            }"#,
        );
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        write(
            ".bee/sessions/live.json",
            &format!(r#"{{"id": "live", "last_heartbeat": "{now}", "lane": "closed-session-feat"}}"#),
        );

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(
            html.contains(r#"data-hub-group="finished" href="/p/proj-1/_bee/feature/closed-session-feat""#),
            "a closed feature must stay under Finished even with a bound live session: {html}"
        );
        assert!(
            !html.contains(r#"data-hub-group="waiting" href="/p/proj-1/_bee/feature/closed-session-feat""#),
            "a closed feature owes no decision even while a session is bound to its lane: {html}"
        );
        assert!(
            !html.contains(r#"data-hub-group="in-progress" href="/p/proj-1/_bee/feature/closed-session-feat""#),
            "a bound live session must never drag a finished feature back into In Progress: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    fn sample_workspace(branch: &str) -> BeeWorkspace {
        BeeWorkspace {
            id: "w-1".to_string(),
            kind: "worktree".to_string(),
            root: "sibling".to_string(),
            branch: Some(branch.to_string()),
            attached_sessions: 0,
            created_at: None,
        }
    }

    /// (hub-finished-compact) Proves directly what the server.rs
    /// `feature_hub_archive_only_feature_with_no_grant_history_renders_under_finished`
    /// test's own now-trimmed "Main" chip assertion used to prove through a
    /// full render: with no currently granted worktree AND no surviving
    /// workspace record naming this feature's own `wt/<feature>` branch,
    /// the chip reads "Main" regardless of `finished` — a feature never
    /// worked in its own worktree has no grant history to report.
    #[test]
    fn bee_hub_worktree_chip_reads_main_with_no_grant_history() {
        let (label, tone) = bee_hub_worktree_chip("solo-feat", &[], &[], true);
        assert_eq!((label.as_str(), tone), ("Main", "neutral"));
        let (label, tone) = bee_hub_worktree_chip("solo-feat", &[], &[], false);
        assert_eq!((label.as_str(), tone), ("Main", "neutral"));
    }

    /// (hub-finished-compact) Proves directly what the server.rs
    /// `feature_hub_open_worktree_grant_wins_over_finished_fallback` test's
    /// own now-trimmed "Open · wt/wt-feat" chip assertion used to prove
    /// through a full render: a currently granted worktree naming this
    /// feature wins over every fallback, `finished` or not.
    #[test]
    fn bee_hub_worktree_chip_reads_open_with_branch_when_a_grant_is_live() {
        let grant = BeeWorktree {
            id: "g-1".to_string(),
            resolved: true,
            unresolved_reason: None,
            feature: Some("wt-feat".to_string()),
            phase: Some("swarming".to_string()),
            mode: Some("standard".to_string()),
            branch: Some("wt/wt-feat".to_string()),
            created_at: None,
            live: false,
            heartbeat_age_minutes: None,
        };
        let (label, tone) = bee_hub_worktree_chip("wt-feat", std::slice::from_ref(&grant), &[], true);
        assert_eq!((label.as_str(), tone), ("Open · wt/wt-feat", "info"));
    }

    /// (hub-finished-compact) The third resolution `bee_hub_worktree_chip`
    /// carries: no currently granted worktree, but a surviving workspace
    /// record whose own `branch` matches this feature's `wt/<feature>`
    /// convention — a grant genuinely existed and is now gone. `finished`
    /// reads that as "Merged"; the same evidence for a feature still live
    /// reads "Main" (never a fabricated "Merged" for work still open).
    #[test]
    fn bee_hub_worktree_chip_reads_merged_only_when_finished_and_a_grant_history_exists() {
        let workspace = sample_workspace("wt/shipped-feat");
        let (label, tone) = bee_hub_worktree_chip("shipped-feat", &[], std::slice::from_ref(&workspace), true);
        assert_eq!((label.as_str(), tone), ("Merged", "success"));

        let (label, tone) = bee_hub_worktree_chip("shipped-feat", &[], std::slice::from_ref(&workspace), false);
        assert_eq!(
            (label.as_str(), tone),
            ("Main", "neutral"),
            "grant history alone must never read Merged for work that has not finished: got {label}"
        );
    }

    /// (hub-finished-compact) A Finished row is exactly a name and a link —
    /// none of `bee_hub_card`'s chip, progress bar or activity markup.
    #[test]
    fn bee_hub_finished_row_renders_only_a_name_and_link() {
        let row = bee_hub_finished_row("proj-1", "shipped-feat", None, None, None);
        assert_eq!(
            row,
            r#"<a class="bee-hub__row" data-hub-group="finished" href="/p/proj-1/_bee/feature/shipped-feat">shipped-feat</a>"#
        );
        assert!(
            !row.contains("bee-hub__chips") && !row.contains("fg-chip") && !row.contains("bee-progress") && !row.contains("Last activity"),
            "a Finished row must carry none of the full card's chip/progress/activity markup: {row}"
        );
    }

    /// (hub-finished-compact, feature-titles parity) A Finished row prefers
    /// the feature's own CONTEXT title over its slug, exactly as
    /// `bee_hub_card` already does for the other two groups — and drops the
    /// slug entirely rather than demoting it to a subtitle, since a dense
    /// row has no room for both.
    #[test]
    fn bee_hub_finished_row_prefers_the_context_title_over_the_slug() {
        let docs = mdview_core::bee::BeeFeatureDocs {
            title: Some("Human Title".to_string()),
            description: None,
            docs: vec![],
        };
        let row = bee_hub_finished_row("proj-1", "slug-feat", Some(&docs), None, None);
        assert!(row.contains(">Human Title</a>"), "{row}");
        assert!(!row.contains(">slug-feat<"), "the slug must not also render once a title exists: {row}");
    }

    /// (hub-finished-compact) Ten or fewer rows render entirely in the open
    /// flow — no `<details>` at all.
    #[test]
    fn bee_hub_finished_rows_with_ten_or_fewer_renders_no_details() {
        let rows: Vec<String> = (0..10).map(|i| format!("<a>{i}</a>")).collect();
        let html = bee_hub_finished_rows(&rows);
        assert!(!html.contains("<details"), "{html}");
        for i in 0..10 {
            assert!(html.contains(&format!("<a>{i}</a>")), "{html}");
        }
    }

    /// (hub-finished-compact) 25 rows page into the first 10 open, then a
    /// `<details>` holding the next 10, with a further `<details>` NESTED
    /// inside that one (never a sibling) holding the final 5 — each
    /// summary naming exactly how many rows it reveals and how many rows
    /// remain below it (this chunk plus everything nested beneath it).
    #[test]
    fn bee_hub_finished_rows_pages_twenty_five_into_ten_open_and_two_nested_details() {
        let rows: Vec<String> = (0..25).map(|i| format!("<a>{i}</a>")).collect();
        let html = bee_hub_finished_rows(&rows);

        let first_details = html.find("<details").expect("expected a details block for the overflow");
        for i in 0..10 {
            let marker = format!("<a>{i}</a>");
            let pos = html.find(&marker).unwrap_or_else(|| panic!("missing {marker}: {html}"));
            assert!(pos < first_details, "row {i} must render open, ahead of the first <details>: {html}");
        }
        for i in 10..25 {
            assert!(html.contains(&format!("<a>{i}</a>")), "{html}");
        }
        assert_eq!(
            html.matches("<details").count(),
            2,
            "25 rows must page into exactly two nested <details>: {html}"
        );
        assert!(html.contains("Show 10 more · 15 left"), "{html}");
        assert!(html.contains("Show 5 more · 5 left"), "{html}");

        let outer_open = html.find("<details").unwrap();
        let outer_close = html.rfind("</details>").unwrap();
        let inner_open = html.rfind("<details").unwrap();
        assert!(
            inner_open > outer_open && inner_open < outer_close,
            "the second <details> must nest inside the first, not sit beside it: {html}"
        );
    }

    // --- cross-board-2: bee_cross_project_features_section ---

    /// One archived cell fixture for `bee_cross_project_features_section`'s
    /// own tests, mirroring `mdview_core::bee`'s own `feature_cell_json`
    /// test helper (not reusable across crates) so an archived feature can
    /// carry, or deliberately lack, a D10 ship time.
    fn cross_board_archived_cell_json(id: &str, feature: &str, capped_at: Option<&str>) -> String {
        let capped_json = capped_at.map(|s| format!("\"{s}\"")).unwrap_or_else(|| "null".to_string());
        format!(
            r#"{{
                "id": "{id}",
                "feature": "{feature}",
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
                "trace": {{"worker": "w1", "claimed_at": "2026-08-01T00:00:00.000Z", "capped_at": {capped_json}}}
            }}"#
        )
    }

    /// (cross-board D3/D4/D5) Three projects, each contributing one feature
    /// in a different one of the three states: the same shape
    /// `hub_sends_a_gate_stopped_feature_to_waiting_once_its_session_goes_stale_enough`
    /// and its siblings already prove one project at a time. Each feature
    /// must land in the same column its own project's board would place it
    /// in, and must carry that project's own name.
    #[test]
    fn cross_project_places_each_feature_in_the_column_its_own_project_would_and_labels_it() {
        let root_a = std::env::temp_dir().join(format!("mdview-views-cross-a-{}", std::process::id()));
        let root_b = std::env::temp_dir().join(format!("mdview-views-cross-b-{}", std::process::id()));
        let root_c = std::env::temp_dir().join(format!("mdview-views-cross-c-{}", std::process::id()));
        for r in [&root_a, &root_b, &root_c] {
            let _ = std::fs::remove_dir_all(r);
        }
        let write = |root: &std::path::Path, rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };

        // Project A: a gate-stopped feature, its session stale past
        // WORKING_MINUTES but still live -> Waiting on you.
        write(
            &root_a,
            ".bee/lanes/waiting-feat.json",
            r#"{
                "feature": "waiting-feat",
                "phase": "executing",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": true, "shape": false, "execution": false, "review": false}
            }"#,
        );
        let stale_hb = (time::OffsetDateTime::now_utc() - time::Duration::minutes(20))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        write(
            &root_a,
            ".bee/sessions/live.json",
            &format!(r#"{{"id": "live", "last_heartbeat": "{stale_hb}", "lane": "waiting-feat"}}"#),
        );

        // Project B: every gate approved, a fresh live session bound to its
        // lane -> In Progress.
        write(
            &root_b,
            ".bee/lanes/progress-feat.json",
            r#"{
                "feature": "progress-feat",
                "phase": "swarming",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": true}
            }"#,
        );
        let fresh_hb = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        write(
            &root_b,
            ".bee/sessions/live.json",
            &format!(r#"{{"id": "live", "last_heartbeat": "{fresh_hb}", "lane": "progress-feat"}}"#),
        );

        // Project C: one archived feature, no lane and no session -> Finished.
        write(
            &root_c,
            ".bee/cells/archive/finished-feat/c-1.json",
            &cross_board_archived_cell_json("c-1", "finished-feat", Some("2026-08-01T05:00:00.000Z")),
        );

        let mut project_a = sample_project();
        project_a.id = "proj-a".into();
        project_a.name = "Project A".into();
        project_a.root_path = root_a.clone();
        let mut project_b = sample_project();
        project_b.id = "proj-b".into();
        project_b.name = "Project B".into();
        project_b.root_path = root_b.clone();
        let mut project_c = sample_project();
        project_c.id = "proj-c".into();
        project_c.name = "Project C".into();
        project_c.root_path = root_c.clone();

        let rollups = mdview_core::bee::read_rollup(&[root_a.clone(), root_b.clone(), root_c.clone()]);
        let pairs: Vec<(&Project, &BeeProjectRollup)> =
            vec![(&project_a, &rollups[0]), (&project_b, &rollups[1]), (&project_c, &rollups[2])];
        let html = bee_cross_project_features_section(&pairs, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"data-hub-group="waiting" href="/p/proj-a/_bee/feature/waiting-feat""#),
            "waiting-feat must land under Waiting, same as its own project's board would: {html}"
        );
        assert!(html.contains("Project A"), "the waiting card must carry its own project's name: {html}");
        assert!(
            html.contains(r#"data-hub-group="in-progress" href="/p/proj-b/_bee/feature/progress-feat""#),
            "progress-feat must land under In Progress: {html}"
        );
        assert!(html.contains("Project B"), "the in-progress card must carry its own project's name: {html}");
        assert!(
            html.contains(r#"data-hub-group="finished" href="/p/proj-c/_bee/feature/finished-feat""#),
            "finished-feat must land under Finished: {html}"
        );
        assert!(html.contains("Project C"), "the finished row must carry its own project's name: {html}");
        assert!(
            html.contains(r#"data-hub-group="waiting" data-hub-count="1""#),
            "the Waiting count must be the sum across projects: {html}"
        );
        assert!(
            html.contains(r#"data-hub-group="in-progress" data-hub-count="1""#),
            "the In Progress count must be the sum across projects: {html}"
        );
        assert!(
            html.contains(r#"data-hub-group="finished" data-hub-count="1""#),
            "the Finished count must be the sum across projects: {html}"
        );

        for r in [&root_a, &root_b, &root_c] {
            let _ = std::fs::remove_dir_all(r);
        }
    }

    /// (cross-board D10) Two projects: some Finished features carry a
    /// usable D10 ship time (every archived cell's `capped_at` parses),
    /// some do not (one cell missing it). Timed entries must render newest
    /// first, each showing its time; untimed entries follow, alphabetically
    /// by feature name across both projects, not grouped by project.
    #[test]
    fn cross_project_finished_orders_timed_newest_first_then_untimed_alphabetically() {
        let root_a = std::env::temp_dir().join(format!("mdview-views-cross-d10-a-{}", std::process::id()));
        let root_b = std::env::temp_dir().join(format!("mdview-views-cross-d10-b-{}", std::process::id()));
        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
        let write = |root: &std::path::Path, rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };

        // Timed, older.
        write(
            &root_a,
            ".bee/cells/archive/older-feat/c-1.json",
            &cross_board_archived_cell_json("c-1", "older-feat", Some("2026-08-01T00:00:00.000Z")),
        );
        // Timed, newer -- in the other project, so the merge must still put
        // it first.
        write(
            &root_b,
            ".bee/cells/archive/newer-feat/c-1.json",
            &cross_board_archived_cell_json("c-1", "newer-feat", Some("2026-08-05T00:00:00.000Z")),
        );
        // Untimed: one cell missing capped_at (mixed capped_at makes the
        // whole feature untimed, per cross-board-1's own rule).
        write(&root_a, ".bee/cells/archive/zeta-feat/c-1.json", &cross_board_archived_cell_json("c-1", "zeta-feat", None));
        write(
            &root_b,
            ".bee/cells/archive/alpha-feat/c-1.json",
            &cross_board_archived_cell_json("c-1", "alpha-feat", None),
        );

        let mut project_a = sample_project();
        project_a.id = "proj-a".into();
        project_a.name = "Project A".into();
        project_a.root_path = root_a.clone();
        let mut project_b = sample_project();
        project_b.id = "proj-b".into();
        project_b.name = "Project B".into();
        project_b.root_path = root_b.clone();

        let rollups = mdview_core::bee::read_rollup(&[root_a.clone(), root_b.clone()]);
        let pairs: Vec<(&Project, &BeeProjectRollup)> = vec![(&project_a, &rollups[0]), (&project_b, &rollups[1])];
        let html = bee_cross_project_features_section(&pairs, &std::collections::HashMap::new());

        let pos_newer = html.find("newer-feat").expect("newer-feat must render");
        let pos_older = html.find("older-feat").expect("older-feat must render");
        let pos_alpha = html.find("alpha-feat").expect("alpha-feat must render");
        let pos_zeta = html.find("zeta-feat").expect("zeta-feat must render");

        assert!(pos_newer < pos_older, "the most recently shipped feature must render first: {html}");
        assert!(pos_older < pos_alpha, "every timed feature must render ahead of every untimed one: {html}");
        assert!(
            pos_alpha < pos_zeta,
            "untimed features must follow, alphabetically by feature name across projects: {html}"
        );
        assert!(
            html.contains(r#"<span class="bee-hub__row-time">"#) && html.contains("ago</span>"),
            "a timed row must show its ship time: {html}"
        );

        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
    }

    /// (cross-board D7) More than ten Finished entries combined across
    /// projects must still page ten open, the rest behind "Show 10 more",
    /// with the remaining count taken from the merged total -- never
    /// computed, and never paged, per project.
    #[test]
    fn cross_project_finished_pages_more_than_ten_combined_entries_behind_show_10_more() {
        let root_a = std::env::temp_dir().join(format!("mdview-views-cross-cap-a-{}", std::process::id()));
        let root_b = std::env::temp_dir().join(format!("mdview-views-cross-cap-b-{}", std::process::id()));
        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
        let write = |root: &std::path::Path, rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        // 6 untimed features in project A, 7 in project B -- 13 combined,
        // neither project alone crosses the per-project cap of 10.
        for n in 0..6 {
            let feature = format!("a-feat-{n:02}");
            write(
                &root_a,
                &format!(".bee/cells/archive/{feature}/c-1.json"),
                &cross_board_archived_cell_json("c-1", &feature, None),
            );
        }
        for n in 0..7 {
            let feature = format!("b-feat-{n:02}");
            write(
                &root_b,
                &format!(".bee/cells/archive/{feature}/c-1.json"),
                &cross_board_archived_cell_json("c-1", &feature, None),
            );
        }

        let mut project_a = sample_project();
        project_a.id = "proj-a".into();
        project_a.root_path = root_a.clone();
        let mut project_b = sample_project();
        project_b.id = "proj-b".into();
        project_b.root_path = root_b.clone();

        let rollups = mdview_core::bee::read_rollup(&[root_a.clone(), root_b.clone()]);
        let pairs: Vec<(&Project, &BeeProjectRollup)> = vec![(&project_a, &rollups[0]), (&project_b, &rollups[1])];
        let html = bee_cross_project_features_section(&pairs, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"data-hub-group="finished" data-hub-count="13""#),
            "the Finished count must be the merged total across both projects: {html}"
        );
        assert!(
            html.contains("Show 3 more · 3 left"),
            "13 combined entries must page 10 open and 3 behind the control, from the merged total: {html}"
        );

        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
    }

    /// (cross-board D4/D5) The same feature slug owned by two different
    /// projects must render as two distinct rows -- different project
    /// labels, different links -- never merged or deduplicated into one.
    #[test]
    fn cross_project_same_feature_slug_in_two_projects_renders_two_distinct_rows() {
        let root_a = std::env::temp_dir().join(format!("mdview-views-cross-dup-a-{}", std::process::id()));
        let root_b = std::env::temp_dir().join(format!("mdview-views-cross-dup-b-{}", std::process::id()));
        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
        let write = |root: &std::path::Path, rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            &root_a,
            ".bee/cells/archive/auth/c-1.json",
            &cross_board_archived_cell_json("c-1", "auth", None),
        );
        write(
            &root_b,
            ".bee/cells/archive/auth/c-1.json",
            &cross_board_archived_cell_json("c-1", "auth", None),
        );

        let mut project_a = sample_project();
        project_a.id = "proj-a".into();
        project_a.name = "Project A".into();
        project_a.root_path = root_a.clone();
        let mut project_b = sample_project();
        project_b.id = "proj-b".into();
        project_b.name = "Project B".into();
        project_b.root_path = root_b.clone();

        let rollups = mdview_core::bee::read_rollup(&[root_a.clone(), root_b.clone()]);
        let pairs: Vec<(&Project, &BeeProjectRollup)> = vec![(&project_a, &rollups[0]), (&project_b, &rollups[1])];
        let html = bee_cross_project_features_section(&pairs, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"href="/p/proj-a/_bee/feature/auth""#),
            "project A's own auth feature must render its own link: {html}"
        );
        assert!(
            html.contains(r#"href="/p/proj-b/_bee/feature/auth""#),
            "project B's own auth feature must render its own, distinct link: {html}"
        );
        assert_eq!(
            html.matches("data-hub-group=\"finished\" href=").count(),
            2,
            "the same feature slug in two projects must render as two rows, never merged into one: {html}"
        );
        assert!(html.contains("Project A") && html.contains("Project B"), "each row must carry its own project's name: {html}");

        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
    }

    /// (cross-board, edge case) A qualifying project with `.bee/` but no
    /// features at all must contribute nothing and break nothing -- the
    /// merged section reads exactly as if that project were absent.
    #[test]
    fn cross_project_a_project_contributing_no_features_changes_nothing() {
        let root_a = std::env::temp_dir().join(format!("mdview-views-cross-empty-a-{}", std::process::id()));
        let root_b = std::env::temp_dir().join(format!("mdview-views-cross-empty-b-{}", std::process::id()));
        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
        let write = |root: &std::path::Path, rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        // Project A has a `.bee/` directory but no lanes, no sessions, no
        // archive -- nothing to contribute.
        write(&root_a, ".bee/state.json", r#"{"phase": "exploring"}"#);
        write(
            &root_b,
            ".bee/cells/archive/only-feat/c-1.json",
            &cross_board_archived_cell_json("c-1", "only-feat", None),
        );

        let mut project_a = sample_project();
        project_a.id = "proj-a".into();
        project_a.root_path = root_a.clone();
        let mut project_b = sample_project();
        project_b.id = "proj-b".into();
        project_b.root_path = root_b.clone();

        let rollups = mdview_core::bee::read_rollup(&[root_a.clone(), root_b.clone()]);
        let with_empty: Vec<(&Project, &BeeProjectRollup)> =
            vec![(&project_a, &rollups[0]), (&project_b, &rollups[1])];
        let without_empty: Vec<(&Project, &BeeProjectRollup)> = vec![(&project_b, &rollups[1])];

        let html_with = bee_cross_project_features_section(&with_empty, &std::collections::HashMap::new());
        let html_without = bee_cross_project_features_section(&without_empty, &std::collections::HashMap::new());

        assert_eq!(
            html_with, html_without,
            "a project contributing no features must change nothing in the merged section"
        );

        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
    }

    /// (card-terminals-1) `bee_hub_card`'s `panes` argument renders the
    /// exact badge markup [`project_badges`] already emits for a project
    /// row -- same `.proj-row__badges` container, same `.proj-row__badge`
    /// anchors, same status pill and program text -- linking to the pane's
    /// own terminal view, but carrying its own accessible label rather than
    /// reusing `project_badges`'s "Terminal panes" wording (a feature
    /// card's panes belong to the checkout, never to the feature itself).
    /// The badges are a sibling of the card's own `<a>`, never nested
    /// inside it -- an anchor inside an anchor is invalid HTML.
    #[test]
    fn bee_hub_card_emits_terminal_badges_matching_project_badges_markup_shape() {
        let panes = vec![TerminalPaneView {
            pane_id: "w1:p1".into(),
            kind: "claude".into(),
            name: "agent-name-must-not-appear".into(),
            status: "working".into(),
            title: String::new(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        }];
        let worktree = ("Main".to_string(), "neutral");
        let card_html = bee_hub_card(
            "proj-a", "feat-a", "waiting", 1, 2, None, &worktree, None, None, None, &panes,
        );
        // project_badges' own markup, with only its aria-label swapped for
        // the checkout-naming one this card must carry -- proving the rest
        // of the shape (container class, anchor class, status pill, program
        // span, href) is exactly project_badges' own, unchanged.
        let expected_badges =
            project_badges("proj-a", &panes).replace("Terminal panes", "Terminals in this checkout");
        assert!(
            card_html.ends_with(&expected_badges),
            "the card must append project_badges' own markup shape (aria-label aside) as a trailing sibling: {card_html}"
        );
        assert!(
            card_html.contains(r#"aria-label="Terminals in this checkout""#),
            "the badge group's accessible label must name the checkout, never the feature: {card_html}"
        );
        assert!(
            card_html.contains(r#"href="/p/proj-a/_terminal/pane/w1:p1""#),
            "the badge must link to the pane's own terminal view: {card_html}"
        );
        assert!(
            !card_html.contains("agent-name-must-not-appear"),
            "the pane's own agent name must never reach this markup (D1a's rule, reused here): {card_html}"
        );
        let card_a_end = card_html.find("</a>").expect("the card's own anchor must close");
        let badge_nav_start = card_html
            .find(r#"<nav class="proj-row__badges""#)
            .expect("badges must render");
        assert!(
            badge_nav_start > card_a_end,
            "the badge nav must be a sibling after the card's own </a>, not nested inside it: {card_html}"
        );
    }

    /// (card-terminals-1) An empty `panes` slice -- the switch off, herdr
    /// unreachable, or genuinely no pane in this feature's checkout --
    /// renders no badge container at all, byte-identical to `bee_hub_card`
    /// before this feature.
    #[test]
    fn bee_hub_card_with_no_panes_renders_no_badge_container() {
        let worktree = ("Main".to_string(), "neutral");
        let card_html = bee_hub_card(
            "proj-a", "feat-a", "waiting", 1, 2, None, &worktree, None, None, None, &[],
        );
        assert!(
            !card_html.contains("proj-row__badges"),
            "an empty pane list must render no badge container: {card_html}"
        );
    }

    /// (hub-finished-compact, integration) The paging arithmetic itself is
    /// proven at the unit level above; this proves `bee_feature_hub_section`
    /// really wires the Finished group's rows through
    /// `bee_hub_finished_rows` end to end, rather than dumping every
    /// archived feature into the open flow.
    #[test]
    fn hub_finished_group_pages_more_than_ten_archived_features_behind_a_details() {
        let root = std::env::temp_dir().join(format!("mdview-views-hub-finished-paged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for n in 0..12 {
            let feature = format!("finished-feat-{n:02}");
            let path = root.join(format!(".bee/cells/archive/{feature}/a.json"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                format!(
                    r#"{{
                        "id": "c-{n}",
                        "feature": "{feature}",
                        "lane": "tiny",
                        "title": "Cell",
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
            )
            .unwrap();
        }

        let snapshot = mdview_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot);

        assert!(html.contains(r#"data-hub-group="finished" data-hub-count="12""#), "{html}");
        assert_eq!(
            html.matches("<details").count(),
            1,
            "12 finished features must page into exactly one <details>: {html}"
        );
        assert!(html.contains("Show 2 more · 2 left"), "{html}");
        for n in 0..12 {
            assert!(
                html.contains(&format!("/_bee/feature/finished-feat-{n:02}")),
                "every finished feature must still render somewhere on the page: {html}"
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }
}
