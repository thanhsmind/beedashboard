//! Server-rendered HTML views. Self-contained: layout + CSS + JS as consts.
//! Theme is CSS-variable driven (no-flash head script); code colors come from
//! `/highlight.css` (syntect class-based), so themes switch without re-render.

use waggledance_core::bee::{
    feature_cell_span, list_archived_feature_dirs, BeeApprovedGates, BeeBacklog, BeeBuckets,
    BeeCell, BeeDecisionSummary, BeeDeferredEntry, BeeFeaturePhase, BeePbi, BeeProjectRollup,
    BeeReview, BeeReviewStatus, BeeSession, BeeShippedFeature, BeeSnapshot, BeeWorkspace,
    BeeWorktree,
};
use waggledance_core::code_source::DirListing;
use waggledance_core::config::Config;
use waggledance_core::domain::{IndexedFile, Project, RenderedPage, SearchResult};
use waggledance_core::render::HighlightedSource;

pub fn layout(title: &str, head_extra: &str, body: &str) -> String {
    let title = esc(title);
    format!(
        r#"<!doctype html>
<html lang="en" data-theme="atelier" class="fg-root">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · Waggle Dance</title>
<script>
// No-flash: apply saved scheme before body renders.
(function() {{
  try {{
    // One-shot storage-key migration (D5 of the waggledance rename): read
    // the new key first; if it is absent, fall back to the old "mdview-theme"
    // key, copy it forward, and delete it — inert on every run after the
    // first, so a user's chosen theme is never silently lost.
    var t = localStorage.getItem('waggledance-theme');
    if (t === null) {{
      var old = localStorage.getItem('mdview-theme');
      if (old !== null) {{
        try {{ localStorage.setItem('waggledance-theme', old); }} catch (e) {{}}
        try {{ localStorage.removeItem('mdview-theme'); }} catch (e) {{}}
        t = old;
      }}
    }}
    t = t || 'system';
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

/// agents-drawer-global: `layout()`'s drawer-appending sibling. Before this,
/// [`agent_switch_drawer`] only rendered on terminal pages and the homepage
/// Terminals tab, each splicing it into their own body by hand — the agent
/// list was unreachable from every other page. This appends the drawer
/// markup to `body` before delegating to `layout()`, so every production
/// page (all ~18 [`layout`] callers below, migrated to this one) carries it
/// and can open it from the topbar's own circular agent toggle
/// ([`topbar_full`]). `homepage` is `true` only for [`home_page`] itself —
/// across every one of its tabs (Kanban, Projects, Terminals), not just
/// Terminals — since home-terminal-parity-2's locked drawer-row shape
/// (`/?tab=terminals&pane=<id>` links, `data-agent-drawer-homepage`) is a
/// home-page property, not a per-tab one; every other caller passes
/// `false` and keeps the project-page shape (rows link `agent.url`). Bare
/// `layout()` is untouched — the low-level escaping tests below call it
/// directly to exercise the raw contract, not this drawer-appending seam.
pub fn layout_with_drawer(title: &str, head_extra: &str, body: &str, homepage: bool) -> String {
    let body_with_drawer = format!(
        "{body}\n{drawer}",
        body = body,
        drawer = agent_switch_drawer(homepage)
    );
    layout(title, head_extra, &body_with_drawer)
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

/// homepage-tabs: which of the home page's two sections `/` renders.
/// `Default` is `Kanban` so that an absent, empty, unknown, or otherwise
/// unparseable `tab` query value (`server.rs`'s `RegisterFlag` visitor)
/// always resolves here rather than needing its own fallback branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HomeTab {
    #[default]
    Kanban,
    Projects,
    /// homepage-terminals D1/D8: the third tab, opening one live agent
    /// terminal. Always offered on the strip regardless of herdr's own
    /// state — [`home_tab_strip`] never gates this variant's anchor on
    /// anything; [`terminals_tab`] is what tells the herdr-off and
    /// no-agent empty causes apart once this tab is actually selected.
    Terminals,
}

/// homepage-tabs: the tab strip above the selected section, reusing the
/// design system's existing `.fg-tabs`/`.fg-tab`/`.fg-tab--on` component
/// (`components.css:188-198`) rather than declaring a second one. Real
/// anchors, not buttons or `<a href="#">` plus JS: the homepage does a full
/// `location.reload()` on any watched change (`app.js:810-840`), and the tab
/// choice has to survive that reload and work with JavaScript off.
fn home_tab_strip(selected: HomeTab) -> String {
    let tab_link = |tab: HomeTab, href: &str, label: &str| {
        let on = tab == selected;
        format!(
            r#"<a class="fg-tab{on_class}" href="{href}"{aria}>{label}</a>"#,
            on_class = if on { " fg-tab--on" } else { "" },
            href = href,
            aria = if on { r#" aria-current="page""# } else { "" },
            label = label,
        )
    };
    format!(
        r#"<nav class="fg-tabs" aria-label="Home sections">{kanban}{projects}{terminals}</nav>"#,
        kanban = tab_link(HomeTab::Kanban, "/?tab=kanban", "Kanban"),
        projects = tab_link(HomeTab::Projects, "/?tab=projects", "Projects"),
        // homepage-terminals D8: no gate here — the anchor renders
        // whatever herdr's own state is; only the tab's own body reads it.
        terminals = tab_link(HomeTab::Terminals, "/?tab=terminals", "Terminals"),
    )
}

/// cross-board (D1, superseded) / homepage-tabs: the home page `/` used to
/// stack Features (cross-project) directly above the project list; it now
/// renders exactly one of the two as a tab, chosen by `tab` (server.rs's
/// `RegisterFlag` query extractor). `cross_features_html`
/// ([`bee_cross_project_features_section`]) is still the caller's own
/// decision of what to show (`server.rs::index_page` applies D8's
/// qualification and D9's empty rule before calling this); empty is treated
/// as "nothing qualified" and the Kanban tab renders its own `fg-empty`
/// state instead of `cross_features_html` (backlog-groom-2 D1) -- the tab
/// strip, and the Terminals tab riding on it, always render regardless of
/// whether any project carries a bee board. A registration error forces the
/// Projects tab regardless of `tab`: the banner [`project_list_main`]
/// renders lives only there, and a user who just submitted the add-project
/// form must see why it failed rather than land back on Kanban.
///
/// `bee_hub_style()` / `.bee-hub-theme` are scoped to Kanban only: every
/// rule the style block declares is itself scoped under the
/// `.bee-hub-theme` selector (`bee_hub_style`'s own doc), so emitting it on
/// the Projects tab's render would add a dead `<style>` block with no
/// selector able to match anything on that page -- there is no reason to
/// ship it there.
#[allow(clippy::too_many_arguments)] // each param is an independent render input server.rs::index_page already assembles; bundling them loses the doc comments pinned to each one above
pub fn home_page(
    projects: &[(Project, usize, Vec<TerminalPaneView>)],
    unassigned_visible: bool,
    suggestions: &[ProjectSuggestion],
    register_error: Option<&str>,
    cross_features_html: &str,
    tab: HomeTab,
    // homepage-terminals: `server.rs::index_page` builds this inventory —
    // `project_panes` over every registered project plus `unassigned_panes`,
    // D3-filtered and D4-ordered — since only that module holds the herdr
    // snapshot and the D2 containment boundaries this join needs.
    terminals_panes: &[TerminalsMenuPane],
    terminals_selected_pane: Option<&str>,
    // D8: `false` means herdr itself is off or unreachable (`index_page`'s
    // own `snapshot: None` fail-open, the same one `badges_enabled`/every
    // other pane list on this page already share) — `true` with an empty
    // `terminals_panes` means herdr answered but no agent is running
    // anywhere. The two read as different messages in `terminals_tab`.
    terminals_herdr_ok: bool,
    // home-terminal-parity-2: the configured D8 preset labels
    // `server.rs::index_page` obtains the same way `terminal_page_inner`
    // already does — threaded through so `terminals_tab` can offer the
    // same "New shell"/preset buttons the project terminal page offers.
    terminals_presets: &[String],
) -> String {
    let tab = if register_error.is_some() {
        HomeTab::Projects
    } else {
        tab
    };
    let (title, section) = match tab {
        HomeTab::Kanban => (
            "Kanban",
            format!(
                r#"{style}
<main class="fg-page bee-hub-theme">
  {features}
</main>"#,
                style = bee_hub_style(),
                // D1 (backlog-groom-2): an empty `cross_features_html`
                // (D9 — no registered project carries a `.bee` board) used
                // to make the whole page early-return the tabless plain
                // project list, which dropped the tab strip and hid the
                // Terminals tab along with it. The Kanban tab now renders
                // its own honest empty state instead, so the strip — and
                // Terminals — stay reachable no matter what the Kanban
                // section has to show.
                features = if cross_features_html.is_empty() {
                    r#"<p class="fg-empty">No project here has a bee board yet.</p>"#.to_string()
                } else {
                    cross_features_html.to_string()
                },
            ),
        ),
        HomeTab::Projects => (
            "Projects",
            project_list_main(projects, unassigned_visible, suggestions, register_error),
        ),
        HomeTab::Terminals => (
            "Terminals",
            terminals_tab(
                terminals_panes,
                terminals_selected_pane,
                terminals_herdr_ok,
                terminals_presets,
            ),
        ),
    };
    let body = format!(
        r#"{topbar}
{tabs}
{section}"#,
        topbar = topbar(""),
        tabs = home_tab_strip(tab),
        section = section,
    );
    layout_with_drawer(title, "", &body, true)
}

/// The project list itself — everything the home page's own Projects tab
/// needs, factored out so [`home_page`] can render it unchanged beneath the
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
        "<p class=\"fg-empty\">Chưa có project nào trong Waggle Dance. Đăng ký: <code>waggledance register &lt;dir&gt;</code> hoặc gọi MCP <code>waggledance_view_file</code>.</p>".to_string()
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
        // On top of that grouping, a group whose parent or branches carry any
        // agent pane (kind != "shell" — the same filter the row badges use)
        // floats above the groups with none, so the projects with agents
        // running are the first thing the eye lands on; the sort is stable,
        // so within each partition arrival order still holds, and a group
        // moves as one unit — a branch never separates from its parent.
        type OrderedProjectRow<'a> = (&'a (Project, usize, Vec<TerminalPaneView>), Option<&'a str>);
        let mut groups: Vec<Vec<OrderedProjectRow>> = Vec::new();
        for entry in projects {
            if worktree_branch(&entry.0.id)
                .map(|(parent, _)| registered.contains(parent))
                .unwrap_or(false)
            {
                continue;
            }
            let mut group: Vec<OrderedProjectRow> = vec![(entry, None)];
            for child in projects {
                if let Some((parent, branch)) = worktree_branch(&child.0.id) {
                    if parent == entry.0.id {
                        group.push((child, Some(branch)));
                    }
                }
            }
            groups.push(group);
        }
        groups.sort_by_key(|group| {
            !group
                .iter()
                .any(|((_, _, panes), _)| panes.iter().any(|p| p.kind != "shell"))
        });
        let ordered: Vec<OrderedProjectRow> = groups.into_iter().flatten().collect();
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
    <button type="submit" class="proj-card__del" aria-label="Remove {name} from Waggle Dance" title="Remove from Waggle Dance">✕</button>
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
        r#"<main class="fg-page">{register_banner}{add_form}{listing}{suggestions_block}{unassigned_card}</main>"#,
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
/// program (`kind` — the herdr agent kind) `pane_strip` prints; the pane's
/// `name` field — the agent's own name — never reaches this markup (D1a).
/// board-badges-agents-only supersedes D2's every-pane-badges rule: an
/// agent-less shell pane (`kind == "shell"`) is skipped here — the board is
/// a fleet overview of running agents, and a plain shell stays reachable
/// through the Terminals tab and its own pane page instead of crowding the
/// board with idle terminals. An empty `panes` list (the switch off, the
/// snapshot unavailable, an unconstructable boundary, or simply no pane
/// inside this project) renders no container at all, so an unbadged row is
/// byte-identical to how every row rendered before this feature (D6) — not
/// an empty `<nav>` that would say the same thing twice; a list of nothing
/// but shell panes renders no container either, for the same reason.
fn project_badges(project_id: &str, panes: &[TerminalPaneView]) -> String {
    terminal_badges_nav(project_id, panes, "Terminal panes", "")
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
/// claiming otherwise would be false for every one of them). Plain shell
/// panes (`kind == "shell"`) are filtered out first (board-badges-agents-only)
/// before the emptiness check, so an all-shell `panes` renders no container
/// at all, exactly like an empty list, for either caller.
fn terminal_badges_nav(
    project_id: &str,
    panes: &[TerminalPaneView],
    aria_label: &str,
    extra_class: &str,
) -> String {
    let panes: Vec<&TerminalPaneView> = panes.iter().filter(|p| p.kind != "shell").collect();
    if panes.is_empty() {
        return String::new();
    }
    let pid = esc(project_id);
    let class = if extra_class.is_empty() {
        "proj-row__badges".to_string()
    } else {
        format!("proj-row__badges {extra_class}")
    };
    let mut out = format!(r#"<nav class="{class}" aria-label="{}">"#, esc(aria_label));
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
    layout_with_drawer(&project.name, "", &body, false)
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
/* term-frame-blocks: a table or TUI frame `waggledance_core::ansi::to_html` wraps
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
/* A http/https URL an agent printed, now clickable — same treatment as a
   doc link: it keeps the surrounding ANSI colour and just underlines. */
.term-url-link { color: inherit; text-decoration: underline; text-underline-offset: 2px; cursor: pointer; }
.term-url-link:hover { text-decoration-thickness: 2px; }
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
  .term-keys button, .term-reply__send, .term-reply__stage, .term-reply__approve { padding: var(--space-2) var(--space-3); }
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
/* Safari on iOS zooms the page whenever it focuses a control smaller than
   16px, and the zoom sticks — on a phone, one tap into the reply box left
   the operator looking at a magnified card. The box is deliberately 13px on
   a desktop, so the size is raised only where a touch pointer is doing the
   focusing. This rule has to live here rather than in `app.css`: each
   terminal page's `<style>` block is inlined into `head_extra`, which
   `layout` emits AFTER the stylesheet link, so an `app.css` rule of equal
   specificity would lose the cascade to the `.term-reply__text` declaration
   above. 16px is the browser's threshold, not a design token — hence the
   literal. The viewport meta tag stays as it is: suppressing the zoom with
   `user-scalable=no` would cost every reader pinch-zoom. */
@media (pointer: coarse) {
  .term-reply__text { font-size: 16px; }
}
.term-reply__actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--space-2); }
.term-reply__send, .term-reply__stage, .term-reply__approve { padding: var(--space-1) var(--space-3); min-height: 44px; border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--radius-sm); background: var(--color-surface-raised); color: var(--color-text); cursor: pointer; }
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
///
/// `link` builds each pane's own href from its pane id — D7: the strip's
/// three callers no longer share one `project_id` to compose a link from
/// (a homepage tab spans projects and includes panes belonging to none), so
/// each caller supplies the link it wants, and this function stays ignorant
/// of what shape that link takes. [`terminal_page`] and [`transcript_page`]
/// pass a closure reproducing today's `/p/{id}/_{kind}/pane/{pane_id}`
/// exactly, so their rendered output is unchanged.
fn pane_strip(
    panes: &[TerminalPaneView],
    selected: Option<&str>,
    link: &dyn Fn(&str) -> String,
) -> String {
    if panes.is_empty() {
        return String::new();
    }
    let mut out = String::from(r#"<nav class="pane-strip" aria-label="Panes">"#);
    for p in panes {
        let active = selected == Some(p.pane_id.as_str());
        out.push_str(&pane_tab(&link(&p.pane_id), p, active, ""));
    }
    out.push_str("</nav>");
    out
}

/// One entry in the pane strip. `link` is the pane's own href, already built
/// by the caller (D7) — this function only escapes and places it, the same
/// way it already escaped the `project_id`/`pane_id` pair it used to compose
/// that href from; `esc` is a plain character replace, so escaping the
/// composed string here is byte-identical to escaping each part before
/// concatenating it. `extra` adds classes to the anchor — the pane bar uses
/// it to mark the standalone copy of the active tab it shows on a narrow
/// screen, where the strip itself is inside the menu.
fn pane_tab(link: &str, p: &TerminalPaneView, active: bool, extra: &str) -> String {
    let cls = match (active, extra.is_empty()) {
        (true, true) => "pane-strip__tab pane-strip__tab--active".to_string(),
        (true, false) => format!("pane-strip__tab pane-strip__tab--active {extra}"),
        (false, true) => "pane-strip__tab".to_string(),
        (false, false) => format!("pane-strip__tab {extra}"),
    };
    format!(
        r#"<a class="{cls}" href="{href}"><span class="term-pane__id">{workspace} · {tab}</span> {status_pill}<span class="term-pane__meta">{program}</span></a>"#,
        cls = cls,
        href = esc(link),
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
///
/// `link` is [`pane_strip`]'s own per-pane link builder, threaded straight
/// through (D7).
fn pane_bar(
    panes: &[TerminalPaneView],
    selected: Option<&str>,
    create: &str,
    link: &dyn Fn(&str) -> String,
) -> String {
    let strip = pane_strip(panes, selected, link);
    if strip.is_empty() {
        // No panes to switch between: there is nothing for a menu to hold,
        // and the creation controls (when the page has any) stand alone.
        return format!(r#"<div class="pane-bar">{create}</div>"#, create = create);
    }
    let current = panes
        .iter()
        .find(|p| selected == Some(p.pane_id.as_str()))
        .or_else(|| panes.first())
        .map(|p| pane_tab(&link(&p.pane_id), p, true, "pane-bar__current"))
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
    pub cwd: String,
    pub workspace: String,
    pub tab: String,
}

/// homepage-terminals: one agent-backed pane the homepage's Terminals tab
/// can switch to — a [`TerminalPaneView`] plus the two things only the
/// caller (`server.rs::index_page`, which alone resolves project membership
/// and the D2 boundaries `project_panes`/`unassigned_panes` need) can know.
#[derive(Clone)]
pub struct TerminalsMenuPane {
    pub view: TerminalPaneView,
    /// The base path this pane's *existing* routes already live under — no
    /// new route, no new guard (plan "Technical design" 1-2):
    /// `/p/<project-id>/_terminal/<pane_id>` for a project pane,
    /// `/_terminal/unassigned/<pane_id>` for one outside every registered
    /// project. `{base}/screen` is exactly the endpoint `assets/app.js`'s
    /// existing poller already knows how to call — see its own
    /// `data-term-base` branch.
    pub base: String,
    /// The project this pane belongs to, or "Unassigned" — labels the menu
    /// row so a pane reads as identifiable across projects, not just by its
    /// own (otherwise meaningless to a reader) pane id. Also this entry's
    /// own D4 tie-break key alongside `view.pane_id` (`server.rs`'s own
    /// sort, never re-sorted here).
    pub project_label: String,
    /// homepage-terminals-2 (D6): `true` for a pane resolved through a
    /// registered project's own `project_panes`, `false` for one from
    /// `unassigned_panes` — set by `server.rs::terminals_menu_panes` at the
    /// same point it already knows which loop built the entry, never
    /// re-derived from `base`'s own string shape here. Gates
    /// [`screen_frame`]'s attach control the same way [`pane_cards`]'s own
    /// `attach` parameter gates [`terminal_page`] vs
    /// [`unassigned_terminal_page`] (plan finding 7): only a project pane
    /// has a `/attach` route to upload against.
    pub is_project_pane: bool,
    /// home-terminal-parity-2: the registered project's own id when
    /// `is_project_pane` is `true`, `None` for a pane `unassigned_panes`
    /// resolved — the one thing `terminal_create_controls` needs
    /// (`/p/:id/_terminal/create/...` is the only create route there is)
    /// that `project_label` alone cannot supply, since a display label is
    /// never a routable id. `terminals_tab` renders create controls only
    /// when the *selected* pane carries a `Some` here.
    pub project_id: Option<String>,
}

/// homepage-terminal-full D1/D2/D6, home-terminal-parity-2: the Terminals
/// tab's own body — the same assembly [`terminal_page`] renders, minus D3's
/// topbar and project nav: a [`pane_bar`] switcher (D7's own per-pane
/// `/?tab=terminals&pane=<pane_id>` links, never a project's own terminal
/// page) carrying the wide strip, the narrow-screen collapsed menu and
/// [`terminal_create_controls`] all inside it (D1), the selected pane's own
/// [`screen_frame`] card below, and the same [`agent_switch_drawer`] the
/// project terminal page renders (in its homepage mode, `homepage: true`:
/// `assets/app.js` groups its rows by project and links each to
/// `/?tab=terminals&pane=<pane_id>` instead of the project page's status
/// groups and `agent.url`). The `<select>` this tab once rendered is gone
/// (D2); `pane_bar` is now the on-page switcher, with the drawer alongside
/// it as a second way to jump straight to any pane across every project.
///
/// `panes` already carries D4's own order (blocked, then working, then the
/// rest, stable within a group by `(project, pane_id)`) — `server.rs`'s
/// job, never re-sorted here; it is still what a bare `?pane`-less load
/// selects by (`panes[0]`). `herdr_ok` is D8's own distinction between
/// the two empty causes when `panes` is empty: `false` reads "herdr is not
/// running" (off or unreachable, indistinguishable to this page — the same
/// fail-open shape every other pane list on the home page already
/// shares), `true` with an empty list reads "no agent is running" (herdr
/// answered; genuinely nothing to show). `presets` is the same configured
/// D8 preset label list [`terminal_page`] already threads through —
/// `server.rs::index_page` obtains it the same way
/// `terminal_page_inner` does.
fn terminals_tab(
    panes: &[TerminalsMenuPane],
    selected_pane: Option<&str>,
    herdr_ok: bool,
    presets: &[String],
) -> String {
    if panes.is_empty() {
        let msg = if herdr_ok {
            "No agents are running right now."
        } else {
            "herdr is not running."
        };
        return format!(
            r#"{tab_style}
<main class="fg-page fg-page--tight">
  <p class="fg-empty">{msg}</p>
</main>"#,
            tab_style = PROJECT_TAB_STYLE,
            msg = esc(msg),
        );
    }

    // D7: an explicit `?pane` naming nothing in `panes` leaves the
    // selection empty rather than silently falling back to another pane —
    // `effective` stays `None`. An absent `?pane` takes D4's own first
    // entry (`panes` is non-empty here).
    let effective: Option<&TerminalsMenuPane> = match selected_pane {
        Some(pid) => panes.iter().find(|p| p.view.pane_id == pid),
        None => Some(&panes[0]),
    };

    let body = match effective {
        Some(pane) => screen_frame(pane),
        None => r#"<p class="fg-empty">This terminal is gone.</p>"#.to_string(),
    };

    // home-terminal-parity-2: create targets the *selected* pane's own
    // project, so a new agent starts where the terminal being
    // watched lives — never a default or the first project in the list.
    // home-terminal-header: agent presets only here (`plain_shell: false`) —
    // the plain "New shell" button belongs to a page that is already scoped
    // to one project, which this tab is not.
    // A selected pane resolved through `unassigned_panes` (`project_id:
    // None`) renders no create controls at all: `/p/:id/_terminal/create/…`
    // is the only create route that exists, and there is no `:id` for a
    // pane outside every registered project.
    let create = effective
        .and_then(|pane| pane.project_id.as_deref())
        .map(|project_id| terminal_create_controls(project_id, presets, false))
        .unwrap_or_default();

    // D7: reproduces the tab's own `/?tab=terminals&pane={pane_id}`
    // address, never leaving the tab for a project's own terminal page.
    // `pane_bar` takes the flat `TerminalPaneView` list `pane_strip`/
    // `pane_tab` already know how to render — cloning is a handful of
    // small structs per render, never a hot path.
    // terminals-tab-project-scope-1: the switcher lists only the panes that
    // share the *effective* pane's own project, not every pane across every
    // project as before — matched on `project_id` (the routable identity
    // `create` above already keys off), never the display-only
    // `project_label`, so a pane with no project (`project_id: None`) lists
    // only the other project-less panes via `Option<String>`'s own equality.
    // Selection resolution above (`effective`, D4's fallback and D7's
    // vanished-pane check) still runs over the *full*, unfiltered `panes`
    // inventory — only the strip handed to `pane_bar` is scoped. When
    // `effective` is `None` (the named pane vanished, D7) there is no
    // project left to scope to, so the switcher falls back to the full
    // list — the cleanest reading available once the pane it would have
    // scoped to no longer exists.
    let strip_panes: Vec<TerminalPaneView> = match effective {
        Some(pane) => panes
            .iter()
            .filter(|p| p.project_id == pane.project_id)
            .map(|p| p.view.clone())
            .collect(),
        None => panes.iter().map(|p| p.view.clone()).collect(),
    };
    // The switcher highlights whichever pane the body actually shows
    // (`effective`, D4's own first-pane fallback included), not the raw,
    // possibly-`None` or possibly-vanished `selected_pane` query value —
    // a highlighted tab always names a pane the reader is actually looking
    // at, or none at all when D4's "this terminal is gone" line is showing.
    let selected_for_bar = effective.map(|p| p.view.pane_id.as_str());
    let bar = pane_bar(&strip_panes, selected_for_bar, &create, &|pane_id: &str| {
        format!("/?tab=terminals&pane={}", pane_id)
    });

    // agents-drawer-global: this used to splice its own
    // `agent_switch_drawer(true)` in here — now [`home_page`] (the only
    // caller of this tab) renders the drawer once, through
    // `layout_with_drawer`, wrapping every home tab alike. A second drawer
    // here would duplicate `#agent-drawer-toggle`.
    format!(
        r#"{tab_style}
<main class="fg-page fg-page--tight">
  {bar}
  {body}
</main>"#,
        tab_style = PROJECT_TAB_STYLE,
        bar = bar,
        body = body,
    )
}

/// homepage-terminals D6: the selected pane's live screen viewport plus
/// [`pane_controls`]'s reply form, key buttons and (project panes only)
/// attach control — [`pane_cards`]'s exact card shape (home-terminal-parity:
/// including its `.term-scroll` history stack — the tab is now a scrollback
/// browser too, not a live-only viewport). `data-term-base` is what lets
/// `assets/app.js`'s existing screen poller *and* its input/keys/attach/
/// scroll wiring build this pane's URLs without a `data-project-id` on this
/// page — see plan "Technical design" 2 and each function's own doc.
///
/// terminals-pane-head-dedupe: the card carries no identity header of its
/// own — the switcher directly above it (`pane_bar`) already names the
/// selected pane (project, `workspace · tab`, status, program), and
/// repeating that line inside the card only pushed the screen further down
/// a handset's viewport, the same reasoning [`pane_cards`] records for the
/// project page.
fn screen_frame(pane: &TerminalsMenuPane) -> String {
    format!(
        r#"<div class="fg-card term-pane" data-pane-id="{pane_id}">
  <div class="term-screen-wrap">
    <pre class="term-screen" data-pane-id="{pane_id}" data-term-base="{base}" aria-live="polite">Loading screen…</pre>
    <div class="term-scroll" data-pane-id="{pane_id}" aria-label="Scroll {name}'s history">
      <div class="term-scroll__stack">
        <button type="button" data-scroll="older" aria-label="Older">↑</button>
        <button type="button" data-scroll="newer" aria-label="Newer" disabled>↓</button>
        <button type="button" data-scroll="live" aria-label="Live">Live</button>
      </div>
    </div>
  </div>
  {controls}
</div>"#,
        pane_id = esc(&pane.view.pane_id),
        name = esc(&pane.view.name),
        base = esc(&pane.base),
        controls = pane_controls(
            &pane.view.pane_id,
            &pane.view.name,
            pane.is_project_pane,
            Some(&pane.base)
        ),
    )
}

/// D3's status pill: maps a [`TerminalPaneView::status`] value onto
/// `.fg-status`'s three tone modifiers
/// (`crates/waggledance/assets/atelier/components.css:145-151`). `done` reads
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
///
/// `base`, when `Some`, is threaded straight into every rendered pane's
/// [`pane_controls`] call (D5) — a homepage tab caller supplies its selected
/// pane's own base since it has no single `data-project-id` to fall back on.
/// [`terminal_page`] and [`unassigned_terminal_page`] both pass `None`:
/// their controls keep resolving through the project-scoped fallback and
/// render byte-identical markup to before this cell.
fn pane_cards(
    panes: &[TerminalPaneView],
    empty_msg: &str,
    attach: bool,
    base: Option<&str>,
) -> String {
    if panes.is_empty() {
        return format!(r#"<p class="fg-empty">{}</p>"#, esc(empty_msg));
    }
    let mut out = String::new();
    for p in panes {
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
  {controls}
</div>"#,
            pane_id = esc(&p.pane_id),
            name = esc(&p.name),
            controls = pane_controls(&p.pane_id, &p.name, attach, base),
        ));
    }
    out
}

/// homepage-terminals-2 (D6): the reply form, key buttons and (`attach`
/// only) attach control — exactly [`pane_cards`]'s own `.term-controls`/
/// `.term-reply` markup, factored out so [`screen_frame`] posts through the
/// very same `assets/app.js` selectors (`.term-reply[data-pane-id]`,
/// `.term-keys[data-pane-id]`) instead of a second copy of this widget set.
///
/// `base`, when `Some`, renders `data-term-base` on each control the same
/// way [`screen_frame`]'s own `.term-screen` already does — `assets/app.js`'s
/// input/keys/attach wiring reads it per-element exactly like the screen
/// poller reads it on `.term-screen`, and falls back to the project page's
/// `data-project-id`-built path when it is absent. [`pane_cards`]'s two
/// callers ([`terminal_page`], [`unassigned_terminal_page`]) both pass
/// `None`: their controls already resolve through that project-scoped
/// fallback and must render byte-identical markup to before this cell.
fn pane_controls(pane_id: &str, name: &str, attach: bool, base: Option<&str>) -> String {
    let base_attr = base
        .map(|b| format!(r#" data-term-base="{}""#, esc(b)))
        .unwrap_or_default();
    let attach_block = if attach {
        format!(
            r#"
    <div class="term-attach" data-pane-id="{pane_id}"{base_attr}>
      <input type="file" class="term-attach__input" data-pane-id="{pane_id}" accept="image/*" multiple aria-label="Attach images to send to {name}" hidden>
      <button type="button" class="term-attach__btn" data-pane-id="{pane_id}">Attach images</button>
      <ul class="term-attach__chips" data-pane-id="{pane_id}"></ul>
      <p class="term-attach__error" data-pane-id="{pane_id}" role="alert" hidden></p>
    </div>"#,
            pane_id = esc(pane_id),
            name = esc(name),
            base_attr = base_attr,
        )
    } else {
        String::new()
    };
    format!(
        r#"<div class="term-controls">
    <div class="term-keys term-keys--move" data-pane-id="{pane_id}"{base_attr} aria-label="Move around {name}'s screen">
      <button type="button" data-key="up">↑</button>
      <button type="button" data-key="down">↓</button>
      <button type="button" data-key="left">←</button>
      <button type="button" data-key="right">→</button>
    </div>
    <div class="term-keys" data-pane-id="{pane_id}"{base_attr} aria-label="Send a key to {name}">
      <button type="button" data-key="enter">Enter</button>
      <button type="button" data-key="escape">Esc</button>
      <button type="button" data-key="tab">Tab</button>
      <button type="button" data-key="ctrl+c">Ctrl+C</button>
    </div>
  </div>
  <form class="term-reply" data-pane-id="{pane_id}"{base_attr}>
    <textarea class="term-reply__text" rows="3" placeholder="Type a reply… (Ctrl+Enter to send)" aria-label="Reply to {name}" autocomplete="off"></textarea>{attach_block}
    <div class="term-reply__actions">
      <button type="button" class="term-reply__approve">Approve</button>
      <button type="button" class="term-reply__stage">Stage</button>
      <button type="submit" class="term-reply__send">Send</button>
    </div>
  </form>"#,
        pane_id = esc(pane_id),
        name = esc(name),
        attach_block = attach_block,
        base_attr = base_attr,
    )
}

/// D8's creation controls (agent-terminal-13): a "New shell" button is
/// always offered — plain-shell creation needs no preset — plus one button
/// per operator-configured preset **label**, never argv (P4): the argv
/// itself never crosses into this view, only the label the server's
/// `terminal_create_agent` keys it by. Zero configured presets renders zero
/// preset buttons, proving the must-have "with no presets configured, the
/// creation control offers nothing [for agents]" at the render layer — the
/// route-level half of that same truth is `terminal_create_agent`'s own
/// refusal when `body.preset` matches nothing.
///
/// home-terminal-header: `plain_shell` gates that always-offered button, and
/// only the homepage Terminals tab passes `false`. There, the Agents drawer
/// is already the way a terminal is reached and started, and a second,
/// project-guessing "New shell" sitting above someone else's running screen
/// only offered a shell in whichever project the *watched* pane happened to
/// belong to. The project terminal page passes `true` and is untouched — a
/// page scoped to one project is exactly where "new shell here" means
/// something. With `plain_shell` false and no presets configured, nothing
/// would render at all, so the `.term-create` box itself is omitted rather
/// than shipped empty — the same silence a pane outside every registered
/// project already gets from [`terminals_tab`].
fn terminal_create_controls(project_id: &str, presets: &[String], plain_shell: bool) -> String {
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
    if !plain_shell && preset_buttons.is_empty() {
        return String::new();
    }
    let pane_button = if plain_shell {
        r#"<button type="button" class="term-create__pane">New shell</button>
  "#
    } else {
        ""
    };
    format!(
        r#"<div class="term-create" data-project-id="{pid}">
  {pane_button}{preset_buttons}
</div>"#,
        pid = esc(project_id),
        pane_button = pane_button,
        preset_buttons = preset_buttons,
    )
}

/// agent-switch-drawer-2: a right-edge slide-in panel that lists every
/// agent-backed pane across every project (`GET /api/agents`). Checkbox-
/// driven the same way [`pane_bar`]'s own `.pane-menu__toggle` is — no
/// script owns open/closed, only Escape/outside-click layers on top via
/// the generic `.js-menu` handler `assets/app.js` already runs. Entirely
/// static: `assets/app.js` fills `[data-agent-drawer-list]` in from JSON
/// once the drawer opens, so there is nothing here to escape.
///
/// agents-drawer-global: this used to render only on terminal pages and
/// the homepage Terminals tab, each with its own floating right-edge
/// `.agent-drawer__tab` pill as the sole affordance to open it. Now
/// [`layout_with_drawer`] renders this on every page, and the topbar's own
/// circular agent toggle ([`topbar_full`], agents-toggle-circle) is the
/// single affordance — the pill (and its CSS) is gone; only the checkbox
/// and the `.fg-drawer` panel markup remain, and any `<label
/// for="agent-drawer-toggle">` anywhere on the page (there is now exactly
/// one, on the topbar) still opens it via the ordinary HTML `for`
/// mechanism.
///
/// home-terminal-parity-2: `homepage` marks the drawer body with
/// `data-agent-drawer-homepage` for [`home_page`]'s own instance — the one
/// data attribute `assets/app.js`'s drawer block reads to tell the two
/// instances apart, since their rows behave differently: the project page
/// (`homepage: false`, unchanged) groups by status and links through
/// `agent.url`; the home page instance groups by project name (blocked,
/// then working, then the rest, within each group) and links each row to
/// `/?tab=terminals&pane=<pane_id>` built from the agent's own `pane_id`.
fn agent_switch_drawer(homepage: bool) -> String {
    let homepage_attr = if homepage {
        " data-agent-drawer-homepage"
    } else {
        ""
    };
    format!(
        r#"<div class="agent-drawer js-menu">
  <input type="checkbox" id="agent-drawer-toggle" class="agent-drawer__check">
  <div class="fg-drawer">
    <div class="fg-drawer__head"><span class="fg-drawer__title">Agents</span></div>
    <div class="fg-drawer__body" data-agent-drawer-list{homepage_attr}></div>
  </div>
</div>"#,
        homepage_attr = homepage_attr,
    )
}

/// `GET /p/:id/_terminal` and `/p/:id/_terminal/pane/:pane_id` up state
/// (D2/D4/D6): one pane's own page, chosen by `selected` from the pane strip
/// (D4) rendered above it. `selected` is `None` only when `panes` is empty
/// (the honest empty state, not a blank page) — a `Some` id not present in
/// `panes` is the caller's authorization refusal (`server.rs`'s
/// `terminal_page_inner`), never reached here. Distinct wording from
/// [`terminal_down_page`] so an empty list is never mistaken for herdr being
/// unreachable, or the reverse. `presets` is the exact configured D8 preset
/// label list (`waggledance_core::config::AgentPreset`'s labels, in
/// `Config.terminal.agent_presets` order) — this view never sees argv.
pub fn terminal_page(
    project: &Project,
    panes: &[TerminalPaneView],
    selected: Option<&str>,
    presets: &[String],
) -> String {
    let empty_msg = "No agents are running under this project right now.";
    let rows = match selected.and_then(|pid| panes.iter().find(|p| p.pane_id == pid)) {
        Some(pane) => pane_cards(std::slice::from_ref(pane), empty_msg, true, None),
        None => pane_cards(&[], empty_msg, true, None),
    };
    // D7: reproduces today's `/p/{id}/_terminal/pane/{pane_id}` exactly, so
    // this page's own links are unchanged.
    let bar = pane_bar(
        panes,
        selected,
        &terminal_create_controls(&project.id, presets, true),
        &|pane_id: &str| format!("/p/{}/_terminal/pane/{}", project.id, pane_id),
    );
    // `data-project-id` lets `assets/app.js`'s screen poller build each
    // pane's `/p/:id/_terminal/:pane_id/screen` URL without threading the id
    // through every `.term-screen` element individually.
    // agents-drawer-global: this used to splice its own
    // `agent_switch_drawer(false)` in here — `layout_with_drawer` below now
    // renders it once for every page. A second drawer here would duplicate
    // `#agent-drawer-toggle`.
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
    );
    layout_with_drawer(&format!("{} · terminal", project.name), "", &body, false)
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
pub fn transcript_page(
    project: &Project,
    panes: &[TerminalPaneView],
    selected: Option<&str>,
) -> String {
    let empty_msg = "No agents are running under this project right now.";
    let rows = match selected.and_then(|pid| panes.iter().find(|p| p.pane_id == pid)) {
        Some(pane) => transcript_cards(std::slice::from_ref(pane), empty_msg),
        None => transcript_cards(&[], empty_msg),
    };
    // D7: reproduces today's `/p/{id}/_transcript/pane/{pane_id}` exactly, so
    // this page's own links are unchanged.
    let bar = pane_bar(panes, selected, "", &|pane_id: &str| {
        format!("/p/{}/_transcript/pane/{}", project.id, pane_id)
    });
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
    layout_with_drawer(&format!("{} · transcript", project.name), "", &body, false)
}

/// `GET /_terminal/unassigned` up state (D5/D4/D6): every herdr pane whose
/// cwd sits under no registered project root, gated identically to
/// [`terminal_page`] (session, D7 switch, method) — this view renders only
/// what the route already decided to hand it, so it carries no gate logic
/// of its own. Zero panes renders a named empty state distinct from both
/// [`terminal_page`]'s own empty wording and [`unassigned_terminal_down_page`].
///
/// backlog-groom-2-4: the poller/reply/keys wiring for this page used to be
/// its own inline `<script>` const here (`UNASSIGNED_TERMINAL_SCRIPT`),
/// duplicating `assets/app.js`'s shape by hand. It now lives in `app.js`
/// itself, scoped to `.unassigned-panes` exactly as before, and reads its
/// route prefix from this `<main>`'s own `data-unassigned-base` rather than
/// a hardcoded string — the one data attribute this page needed to carry to
/// fold cleanly. `assets/app.js`'s *other* terminal poller/posters (the
/// `data-project-id`/`data-term-base` ones) still skip this page entirely
/// (`unassigned-poller-guard` D1, `hasTarget`) — this is a second, separate
/// wiring, not a merge into that one.
pub fn unassigned_terminal_page(panes: &[TerminalPaneView]) -> String {
    let rows = pane_cards(
        panes,
        "No agents are running outside a registered project right now.",
        false,
        None,
    );
    let body = format!(
        r#"{topbar}
{tab_style}
<main class="fg-page" data-unassigned-base="/_terminal/unassigned">
  <h2 class="fg-pagehead__title">Unassigned agents</h2>
  <p class="term-pane__meta">Agents running outside every registered project. Registering a project here never happens automatically (D5) — <a href="/">register it from the project list</a> if you want it to have its own Terminal tab.</p>
  <div class="term-panes unassigned-panes">{rows}</div>
</main>"#,
        topbar = topbar("<span class=\"crumb\">Unassigned agents</span>"),
        tab_style = PROJECT_TAB_STYLE,
        rows = rows,
    );
    layout_with_drawer("Unassigned agents", "", &body, false)
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
    <div class="term-pane__meta">Start herdr, then reload this page — Waggle Dance does not start it for you unless the herdr supervisor is switched on in Settings.</div>
  </div>
</main>"#,
        topbar = topbar("<span class=\"crumb\">Unassigned agents</span>"),
        tab_style = PROJECT_TAB_STYLE,
    );
    layout_with_drawer("Unassigned agents", "", &body, false)
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
    <div class="term-pane__meta">Start herdr, then reload this page — Waggle Dance does not start it for you unless the herdr supervisor is switched on in Settings.</div>
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
    layout_with_drawer(&format!("{} · terminal", project.name), "", &body, false)
}

/// The read-only bee cell board (D4/D5). feature-hub D1 replaces the
/// cell-centric Kanban board (`bee_agent_board_section`, agent-board
/// ab-1/ab-2 — now retired) with a FEATURE-centric grouped list
/// ([`bee_feature_hub_section`]): a manager asks which feature needs them,
/// which is moving, which is done, not which cell an agent holds. Every
/// feature this snapshot can place renders in exactly one of five groups, in
/// this left-to-right order — Todo, In Progress, Review, Compound, Finished
/// (kanban-columns D1) — see that function's own doc comment for the full
/// membership rule, the D10 narrowing that keeps Todo reachable, and the D4
/// ghost-card fix it carries. A feature that has fully shipped (D10,
/// `snapshot.shipped`) still gets its own line in `bee_finished_section` too, unrelated to and
/// unmoved by this change — a distinct, uncapped feature-level list this
/// board has always kept separate from whatever this cell's own Finished
/// group shows. `bee_lanes_panel` stays retired (bbp-11); the feature
/// detail page's own D7 four-bucket view retires in turn under
/// feature-hub-2, replaced by [`bee_feature_page`]'s tabbed drill-down (D2)
/// — every cell this board's buckets fed it now surfaces on that page's own
/// Todos tab instead. Every path-shaped value on a
/// `BeeCell`/`BeeFeaturePhase` already arrives relativized by
/// `waggledance_core::bee::read_snapshot` (no absolute path crosses into
/// `BeeSnapshot`'s public fields), so nothing further is redacted here —
/// this view only escapes for HTML safety.
///
/// board-declutter drops the pre-hub top-of-board stack entirely: the
/// lifecycle stepper, the headline KPI tiles, the Ship velocity section, the
/// Needs attention panel and the Working-on-now card (its own Running now
/// subsection included) are gone, along with the view functions and CSS
/// that only ever rendered them — `waggledance_core`'s readers for that data
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
/// that only ever rendered them. `waggledance_core`'s readers for that data
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
pub fn bee_board_page(
    project: &Project,
    snapshot: &BeeSnapshot,
    feature_panes: &std::collections::HashMap<String, Vec<TerminalPaneView>>,
) -> String {
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
        board = bee_feature_hub_section(project, snapshot, feature_panes),
        finished = bee_finished_section(&project.id, &snapshot.shipped),
        panels = bee_panels_section(snapshot),
    );
    layout_with_drawer(&format!("{} · bee", project.name), "", &body, false)
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
    r#"<style>
.bee-finished { margin-bottom: var(--space-4); }
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
.bee-hub-theme {
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
  /* project-color-identity: ten fixed hues for the cross-project board's
     own per-project accent -- defined here, not alongside
     `--color-accent-alt-*` in contract.css: those five tokens are
     sitewide semantic slots (brand/info/success/warning/danger) every
     theme repoints and every page may read, while a project's board
     colour is a page-local identity nothing outside this page needs, so
     it never grows that sitewide token surface. Picked at one fixed
     lightness/saturation, evenly spread around the wheel, so each stays
     legible both as a thin left border and as small text against this
     page's card surfaces -- deliberately not restated in the
     `data-scheme="dark"` block below: a per-project colour's whole job
     is to read as the same identity everywhere it appears, light or
     dark, unlike the semantic chip tones nearby that do shift with
     scheme. */
  --bee-hub-project-1: #9E2E2E;
  --bee-hub-project-2: #9E712E;
  --bee-hub-project-3: #889E2E;
  --bee-hub-project-4: #449E2E;
  --bee-hub-project-5: #2E9E5B;
  --bee-hub-project-6: #2E9E9E;
  --bee-hub-project-7: #2E5B9E;
  --bee-hub-project-8: #442E9E;
  --bee-hub-project-9: #882E9E;
  --bee-hub-project-10: #9E2E71;
}
html[data-scheme="dark"] .bee-hub-theme {
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
}
.bee-hub { margin-bottom: var(--space-4); }
/* board-liveness-3: the live strip's own dense-row idiom, one row per live
   session or granted worktree — deliberately never `.bee-hub__row`'s
   feature-link styling, since a strip row never links anywhere of its
   own. */
.bee-strip { margin-bottom: var(--space-4); }
.bee-strip__rows { display: flex; flex-direction: column; gap: var(--space-1); }
.bee-strip__row { display: flex; flex-wrap: wrap; align-items: baseline; gap: var(--space-2); padding: var(--space-1) var(--space-2); border-bottom: var(--border-width-hairline) solid var(--color-border); font-size: var(--type-body-sm-size); }
.bee-strip__row:last-child { border-bottom: none; }
.bee-strip__label { font-weight: var(--weight-strong); color: var(--color-text); }
.bee-strip__meta { color: var(--color-text-muted); }
.bee-strip__row--unresolved .bee-strip__meta { color: var(--color-danger); }
/* kanban-columns D1/D12: five explicit tracks in the board's own left-to-
   right column order (Todo, In Progress, Review, Compound, Finished) rather
   than `repeat(auto-fit, minmax(260px, 1fr))` — that auto-fit sizing was
   tuned for three equal columns and wraps badly once four of the five are
   dense row lists needing far less width than the one that still renders
   cards. In Progress keeps the wider, card-shaped track; the four row
   columns share a narrower one. */
.bee-hub__groups { display: grid; grid-template-columns: minmax(200px, 1fr) minmax(280px, 1.6fr) minmax(200px, 1fr) minmax(200px, 1fr) minmax(200px, 1fr); gap: var(--space-4); }
/* hub-fallbacks: a grid/flex item's own default `min-width: auto` sizes it
   to its content's min-content width — normally harmless, but a clamped
   `.bee-hub__desc` below is `white-space: nowrap` at its own min-content
   size, and without `min-width: 0` breaking that default at every level
   of this chain (group, cards, shell, card), a long description would
   still force its own column wider than its own `minmax(...)` track
   and push the whole page into horizontal scroll on a phone — the same
   chain `.term-panes` and its siblings already pin above for the same
   reason. */
.bee-hub__group { display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }
.bee-hub__cards { display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }
/* card-badge-inside: the shell carries `.fg-card`'s box paint (border,
   background, radius, padding) so the card's own `<a>` and its terminal
   badge `<nav>` both land inside one painted box instead of the badges
   sitting as a bare row underneath it — see `bee_hub_card`'s doc comment. */
.bee-hub__shell { display: flex; flex-direction: column; gap: var(--space-2); min-width: 0; }
/* card-collapse-inprogress: `.bee-hub__card` now names the `<details>`
   itself rather than the old whole-card `<a>` -- the flex-column layout
   moved to `.bee-hub__body` below (only that inner div still needs to
   stack the card's own rows), leaving this rule only the `min-width: 0`
   the hub-fallbacks shrink chain still requires from every level between
   the grid track and the clamped description. */
.bee-hub__card { min-width: 0; }
/* card-collapse-inprogress D1/D3/D6: the collapsed header row -- name plus
   chevron -- is the details element's own `<summary>`, native to the
   `<details>`/`<summary>` disclosure this card now uses (no JS, no
   persisted state); the `list-style`/`::-webkit-details-marker` pair
   strips the browser's own default marker so only `.bee-hub__chev` below
   draws one. */
.bee-hub__summary { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); cursor: pointer; list-style: none; }
.bee-hub__summary::-webkit-details-marker { display: none; }
.bee-hub__summary:focus-visible { outline: var(--focus-width) solid var(--focus-color); outline-offset: var(--focus-offset); }
/* Card titles are list entries under a `.bee-panel__subhead` column header,
   not headings of their own -- cap them at the same size as that header
   (they'd otherwise inherit the larger `--type-heading-size` from
   `.fg-card__title`, outweighing the column name above them). */
.bee-hub__summary .fg-card__title { font-size: var(--type-heading-sm-size); }
/* Same rotation `.chap-folders__chev` uses (app.css) for its own disclosure
   chevron, keyed off the native `[open]` attribute a `<details>` toggles
   itself rather than that pattern's own `.is-open` class. */
.bee-hub__chev { flex: none; line-height: 1; color: var(--color-text-subtle); transition: transform var(--motion-fast) var(--ease-standard); }
.bee-hub__card[open] .bee-hub__chev { transform: rotate(90deg); }
/* D2: everything the collapsed header does not show -- the subtitle,
   description, progress bar, reason and activity lines -- plus D4's own
   Feature detail link row, stacked in the same column gap the old
   whole-card `<a>` used. `min-width: 0` keeps the hub-fallbacks shrink
   chain intact now that `.bee-hub__desc` sits one flex level deeper than
   before. */
.bee-hub__body { display: flex; flex-direction: column; gap: var(--space-1); min-width: 0; }
/* D4: the feature-detail link row -- the one place the card still reaches
   its own detail page now that the `<details>` can no longer be that link
   itself. */
.bee-hub__detail-link { display: flex; align-items: center; gap: var(--space-1); color: var(--color-link); font-size: var(--type-body-sm-size); font-weight: var(--weight-strong); text-decoration: none; }
.bee-hub__detail-link:hover { color: var(--color-link-hover); }
/* project-color-identity: the Features board card's own chip row is gone
   (D3), but this rule stays -- [`bee_feature_terminal_tab`]'s pane rows
   still reuse it for their own status-pill layout, unrelated to this
   cell. */
.bee-hub__chips { display: flex; flex-wrap: wrap; gap: var(--space-1); }
/* card-badge-inside: cancels the shared badge-nav class's own full-width
   flex-basis and right/bottom padding (app.css) for this context,
   replacing them with a hairline top rule that separates the badges from
   the card's content inside the shared shell. */
.bee-hub__badges { border-top: var(--border-width-hairline) solid var(--color-border); padding: var(--space-2) 0 0 0; flex: 0 0 auto; }
.bee-hub__progress-label { margin: 0; font-size: var(--type-caption-size); color: var(--color-text-subtle); }
.bee-hub__reason { font-style: italic; }
/* kanban-live-signals D2: the run_state badge rides in the collapsed
   `<summary>` beside the title (see `bee_hub_card`'s own doc comment) --
   the same `.fg-chip` tone modifiers every other badge on this board
   already uses, never a palette of its own. */
.bee-hub__run-state { margin-left: var(--space-2); }
/* kanban-live-signals D1: a small "working now" dot beside the Last
   activity line -- `.fg-status__dot`'s own sizing (components.css) plus a
   gentle pulse animation, since a plain static dot would read identically
   to `.fg-status`'s existing non-live indicators, and this one specifically
   means "a tool call landed in the last couple of minutes", not just "has
   a status". */
.bee-hub__pulse { display: inline-block; width: 7px; height: 7px; border-radius: var(--radius-pill); background: var(--color-success); vertical-align: middle; animation: beeHubPulse 1.6s ease-in-out infinite; }
@keyframes beeHubPulse { 0%, 100% { opacity: 1; box-shadow: 0 0 0 0 color-mix(in srgb, var(--color-success) 55%, transparent); } 50% { opacity: 0.55; box-shadow: 0 0 0 4px color-mix(in srgb, var(--color-success) 0%, transparent); } }
/* kanban-live-signals D3: the deferred-debt badge reuses `.fg-badge` for
   its own collapsed `<summary>` and the board's existing click-to-reveal
   `<details>` idiom (`.bee-cell__detail`, the backlog card's "Condition of
   satisfaction" disclosure) for its per-entry detail list. */
.bee-hub__deferred { margin: 0; }
.bee-hub__deferred > summary { cursor: pointer; list-style: none; }
.bee-hub__deferred > summary::-webkit-details-marker { display: none; }
.bee-hub__deferred-list { margin: var(--space-1) 0 0 0; padding-left: var(--space-4); font-size: var(--type-body-sm-size); color: var(--color-text-muted); }
.bee-hub__deferred-kind { font-weight: var(--weight-strong); color: var(--color-text); }
/* hub-finished-compact, kanban-columns D12: the shared dense-row shape now
   rendered by all four of Todo, Review, Compound and Finished — name only,
   linking straight to the detail page — mirroring `.bee-done-line`'s
   one-line idiom rather than the full `.bee-hub__card` shape only In
   Progress still keeps; plus the nested-details toggle
   ([`bee_hub_finished_rows`]) that pages each of the four ten rows at a
   time. */
.bee-hub__row { display: block; color: var(--color-text); font-size: var(--type-body-sm-size); text-decoration: none; padding: var(--space-1) var(--space-2); border-bottom: var(--border-width-hairline) solid var(--color-border); overflow-wrap: anywhere; }
.bee-hub__row:hover { color: var(--color-action); }
/* cross-board D5/D10: the cross-project board's own project label and ship
   time on a Finished row — absent on every per-project board row, which
   passes neither and renders unchanged. */
.bee-hub__row-project { color: var(--color-text-subtle); font-size: var(--type-caption-size); }
.bee-hub__row-time { color: var(--color-text-subtle); font-size: var(--type-caption-size); float: right; }
.bee-hub__more { margin-top: var(--space-1); }
.bee-hub__more-summary { cursor: pointer; list-style: none; color: var(--color-text-subtle); font-size: var(--type-caption-size); padding: var(--space-1) var(--space-2); }
.bee-hub__more-summary::-webkit-details-marker { display: none; }
.bee-hub__more-summary:hover { color: var(--color-action); }
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
.bee-hub__slug { margin: 0; font-size: var(--type-caption-size); color: var(--color-text-subtle); }
/* project-color-identity: the cross-project board's own per-project accent
   -- a local custom property set by whichever `bee-hub__shell--pN`/
   `bee-hub__row-project--pN` modifier the project's own colour slot landed
   on (the project->colour map built once in
   `bee_cross_project_board_project_colors`, threaded down into both
   [`bee_hub_card`] and [`bee_hub_finished_row`] rather than either one
   computing its own), read by the project-name subtitle and the Finished
   row's project span, and by the shell's own left border (overriding only
   `.fg-card`'s existing left edge -- see `.fg-card--rule` for the same
   pattern -- so this never introduces a border where none already existed,
   keeping the min-width: 0 chain documented above intact). `.bee-hub__project`
   carries no colour of its own; a per-project board card never gets a
   `--pN` modifier at all, so it never gets a border or a colour here. */
.bee-hub__project { margin: 0; font-size: var(--type-caption-size); }
/* project-color-identity: the worktree half of the project line reads
   muted rather than the project's own accent colour -- the accent already
   does one job (telling projects apart at a glance) and a second coloured
   run of text on the same short line would fight it for attention, so the
   worktree state stays the same subdued tone `.bee-hub__slug` already
   uses elsewhere on this card. */
.bee-hub__project-worktree { color: var(--color-text-subtle); }
.bee-hub__shell--p1 { --project-accent: var(--bee-hub-project-1); }
.bee-hub__shell--p2 { --project-accent: var(--bee-hub-project-2); }
.bee-hub__shell--p3 { --project-accent: var(--bee-hub-project-3); }
.bee-hub__shell--p4 { --project-accent: var(--bee-hub-project-4); }
.bee-hub__shell--p5 { --project-accent: var(--bee-hub-project-5); }
.bee-hub__shell--p6 { --project-accent: var(--bee-hub-project-6); }
.bee-hub__shell--p7 { --project-accent: var(--bee-hub-project-7); }
.bee-hub__shell--p8 { --project-accent: var(--bee-hub-project-8); }
.bee-hub__shell--p9 { --project-accent: var(--bee-hub-project-9); }
.bee-hub__shell--p10 { --project-accent: var(--bee-hub-project-10); }
.bee-hub__row-project--p1 { --project-accent: var(--bee-hub-project-1); }
.bee-hub__row-project--p2 { --project-accent: var(--bee-hub-project-2); }
.bee-hub__row-project--p3 { --project-accent: var(--bee-hub-project-3); }
.bee-hub__row-project--p4 { --project-accent: var(--bee-hub-project-4); }
.bee-hub__row-project--p5 { --project-accent: var(--bee-hub-project-5); }
.bee-hub__row-project--p6 { --project-accent: var(--bee-hub-project-6); }
.bee-hub__row-project--p7 { --project-accent: var(--bee-hub-project-7); }
.bee-hub__row-project--p8 { --project-accent: var(--bee-hub-project-8); }
.bee-hub__row-project--p9 { --project-accent: var(--bee-hub-project-9); }
.bee-hub__row-project--p10 { --project-accent: var(--bee-hub-project-10); }
.bee-hub__shell[class*="bee-hub__shell--p"] { border-left: var(--border-width-strong) solid var(--project-accent); }
.bee-hub__project, .bee-hub__row-project { color: var(--project-accent); }
.bee-hub__desc { margin: 0; font-size: var(--type-body-sm-size); color: var(--color-text-muted); display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; overflow-wrap: anywhere; }
/* feature-titles: the detail header's own title/slug/description stack,
   and the docs row beneath the chip row linking every markdown file the
   feature's docs dir holds (hub-fallbacks; CONTEXT.md/plan.md lead when
   present) through the viewer's own document routes. */
.bee-detail-slug { margin: var(--space-1) 0 0 0; font-size: var(--type-body-sm-size); color: var(--color-text-subtle); }
.bee-detail-desc { margin: var(--space-1) 0 0 0; color: var(--color-text-muted); display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; overflow-wrap: anywhere; max-width: 100%; }
.bee-detail-docs { display: flex; flex-wrap: wrap; gap: var(--space-2); margin: 0 0 var(--space-4) 0; }
.bee-detail-docs a { color: var(--color-link); font-size: var(--type-body-sm-size); }
.bee-done-summary { cursor: pointer; list-style: none; padding: var(--space-2) 0; font-weight: var(--weight-strong); color: var(--color-text); }
.bee-done-summary::-webkit-details-marker { display: none; }
.bee-done-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: var(--space-2); padding-top: var(--space-2); }
.bee-done-line { display: block; color: var(--color-text-muted); font-size: var(--type-caption-size); text-decoration: none; padding: var(--space-1) 0; border-bottom: var(--border-width-hairline) solid var(--color-border); overflow-wrap: anywhere; }
.bee-done-line:hover { color: var(--color-action); }
.bee-cell { padding: var(--space-2); gap: var(--space-1); }
.bee-cell .fg-card__title { font-size: var(--type-body-sm-size); overflow-wrap: anywhere; }
.bee-cell__meta { color: var(--color-text-subtle); font-size: var(--type-caption-size); word-break: break-word; }
.bee-cell__detail { margin-top: var(--space-1); font-size: var(--type-caption-size); color: var(--color-text-subtle); }
.bee-cell__detail summary { cursor: pointer; color: var(--color-text); }
.bee-cell__detail p { margin: var(--space-1) 0 0 0; overflow-wrap: anywhere; }
.bee-panels { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: var(--space-4); margin-top: var(--space-4); }
.bee-panel__head { display: flex; align-items: center; gap: var(--space-2); margin: 0; }
.bee-panel__subhead { margin: var(--space-3) 0 var(--space-2) 0; font-size: var(--type-heading-sm-size); }
.bee-panel__chips { display: flex; flex-wrap: wrap; gap: var(--space-2); margin-bottom: var(--space-2); }
.bee-panel__list { display: flex; flex-direction: column; gap: var(--space-2); }
.bee-severity--p1 { font-weight: var(--weight-strong); }
.bee-asof { color: var(--color-text-subtle); font-size: var(--type-body-sm-size); }
.bee-progress { height: 8px; border-radius: var(--radius-pill); background: var(--color-surface-sunken); overflow: hidden; }
.bee-progress__bar { height: 100%; background: var(--color-success); }
.bee-done-summary:focus-visible { outline: var(--focus-width) solid var(--focus-color); outline-offset: var(--focus-offset); }
/* feature-hub-2: the feature detail page's own header, chip row and
   CSS-only tab pattern — no JS framework, same checkbox/radio-plus-label
   idiom `topbar_full`'s own doc comment already explains the reasoning
   for (a `<details>` element hides its content even when it should not,
   past what any `display` override here can undo; a plain input a browser
   already knows how to toggle needs none of that). */
.bee-detail-head { display: flex; align-items: flex-start; justify-content: space-between; flex-wrap: wrap; gap: var(--space-3); margin-bottom: var(--space-2); }
/* Same shrink chain the hub cards need: without `min-width: 0` this flex
   column grows to its description's widest line instead of wrapping it,
   and the whole detail page scrolls sideways on a phone. */
.bee-detail-head > div { min-width: 0; flex: 1 1 16rem; }
.bee-detail-chips { display: flex; flex-wrap: wrap; gap: var(--space-2); margin-bottom: var(--space-4); }
.bee-tabs { margin-top: var(--space-2); }
.bee-tabs__radio { position: absolute; opacity: 0; pointer-events: none; }
.bee-tabs__nav { display: flex; flex-wrap: wrap; gap: var(--space-1); border-bottom: var(--border-width-hairline) solid var(--color-border); margin-bottom: var(--space-4); }
.bee-tabs__label { cursor: pointer; padding: var(--space-2) var(--space-3); color: var(--color-text-muted); font-weight: var(--weight-strong); border-bottom: 2px solid transparent; }
.bee-tabs__label:hover { color: var(--color-text); }
.bee-tabs__panel { display: none; }
#bee-tab-activity:checked ~ .bee-tabs__nav label[for="bee-tab-activity"],
#bee-tab-todos:checked ~ .bee-tabs__nav label[for="bee-tab-todos"],
#bee-tab-terminal:checked ~ .bee-tabs__nav label[for="bee-tab-terminal"] {
  color: var(--color-action);
  border-bottom-color: var(--color-action);
}
#bee-tab-activity:checked ~ .bee-tabs__body #bee-panel-activity,
#bee-tab-todos:checked ~ .bee-tabs__body #bee-panel-todos,
#bee-tab-terminal:checked ~ .bee-tabs__body #bee-panel-terminal {
  display: block;
}
#bee-tab-activity:focus-visible ~ .bee-tabs__nav label[for="bee-tab-activity"],
#bee-tab-todos:focus-visible ~ .bee-tabs__nav label[for="bee-tab-todos"],
#bee-tab-terminal:focus-visible ~ .bee-tabs__nav label[for="bee-tab-terminal"] {
  outline: var(--focus-width) solid var(--focus-color);
  outline-offset: var(--focus-offset);
}
.bee-activity { display: flex; flex-direction: column; gap: var(--space-3); }
.bee-activity__gates { display: flex; flex-wrap: wrap; gap: var(--space-1); }
.bee-activity__timeline { display: flex; flex-direction: column; gap: var(--space-2); }
.bee-activity__item { padding: var(--space-2); gap: var(--space-1); }
.bee-activity__ts { color: var(--color-text-subtle); font-size: var(--type-caption-size); }
.bee-todos { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
.bee-todo { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); padding: var(--space-2); border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--card-radius); background: var(--color-surface); }
.bee-todo a { display: flex; align-items: center; gap: var(--space-2); flex: 1; min-width: 0; color: var(--color-text); text-decoration: none; }
.bee-todo__mark { flex: none; width: 20px; text-align: center; color: var(--color-text-subtle); }
.bee-todo__title { overflow-wrap: anywhere; }
.bee-todo--done .bee-todo__title { text-decoration: line-through; color: var(--color-text-subtle); }
.bee-todo--done .bee-todo__mark { color: var(--color-success); }
.bee-todo--blocked .bee-todo__mark { color: var(--color-danger); }
.bee-todo--blocked { border-color: var(--color-danger); }
.bee-todo__badge { flex: none; }
.bee-terminal-panes { display: flex; flex-direction: column; gap: var(--space-2); }
.bee-terminal-pane { text-decoration: none; }
/* Narrow-screen pass (bbp-17): every multi-column grid this board declares
   collapses to one column below this breakpoint (matches the sidebar
   breakpoint in app.css) so a phone never needs the page itself to scroll
   sideways — a genuinely wide container (the agent board's columns) keeps
   its own `overflow-x` above instead of forcing the page wider. */
@media (max-width: 700px) {
  .bee-hub__groups,
  .bee-panels,
  .bee-done-grid {
    grid-template-columns: 1fr;
  }
  /* inprogress-priority-order-2 (D1): on a narrow screen the one column
     that still carries cards moves to the very top of the stacked board,
     above Todo -- selected through the group's own `data-hub-group`
     attribute (`bee_hub_group`, no markup change) so this rule never
     leaks outside this media query and wide screens render unchanged. */
  .bee-hub__groups > [data-hub-group="in-progress"] {
    order: -1;
  }
}
</style>"#
        .to_string()
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
/// (`waggledance_core::bee`) builds a date, just with the time appended.
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
        let label = s
            .lane
            .as_deref()
            .or(active_feature)
            .unwrap_or("no active lane");
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

/// D1's feature-centric grouped list (fh-1; grown to five columns by
/// kanban-columns), replacing the retired cell-centric Kanban board
/// (`bee_agent_board_section`, agent-board ab-1/ab-2 — every card it
/// rendered, and that five-column shape itself, is gone). The card unit is
/// now the FEATURE, not the cell: every feature this snapshot can place
/// (`phase_board`'s lanes ∪ active-feature union, plus any feature whose
/// cells have moved entirely to `.bee/cells/archive/` with no lane of its
/// own) renders in exactly one of five groups, left to right — **Todo, In
/// Progress, Review, Compound, Finished** (kanban-columns D1) — never two,
/// never a duplicate. The former "Waiting on you" column is gone entirely
/// (D1); a feature that used to land there now lands in **In Progress**,
/// carrying one card line reading `Waiting on you — ` followed by the same
/// reason text this board has always computed (D5, D7, D8). Group
/// membership is checked in this fixed priority order (D11):
///
/// 1. **In Progress** — has live work (below) and is not finished-and-idle.
/// 2. **Finished** — no live *cells* left, and either a terminal phase or an
///    archived-cells directory. Checked before Review so a closed feature
///    stays closed even carrying an unresolved review candidate.
/// 3. **Review** — `snapshot.review.candidates` holds a candidate naming
///    this feature whose `status` is not `Settled` (`Unreviewed` or
///    `InReview`).
/// 4. **Compound** — the lane's own `phase` is exactly `"compounding"`.
/// 5. **Todo** — cells exist, are all `open` (`waiting > 0`), and none is
///    `claimed` (`doing == 0`).
///
/// A feature matching none of the five still renders nowhere, exactly as
/// before this feature.
///
/// A feature counts as **live work** (`has_live_work`, board-liveness-2,
/// narrowed by kanban-columns D10) when ANY of: it has a `doing` or `stuck`
/// cell; it is `state.feature`, the globally active one, even with none
/// yet; a live session (`BeeSession.live`, `SESSION_LIVE_MINUTES`) carries
/// a `lane` naming this feature; or a granted worktree in
/// `snapshot.worktrees` names this feature as its own active one. D10
/// deliberately stops counting an `open` (`waiting`) cell on its own as
/// live: today's rule counted it, which made Todo's "open cells, none
/// claimed" branch dead code — every feature it should catch was pulled
/// into In Progress first. A session heartbeat or a worktree grant is a
/// real signal between units of work — every cell capped, nothing `doing`
/// — that a cell-only count would miss entirely, reporting an empty In
/// Progress column while two sessions were still actively on the repo.
/// Deliberately not phase-based: a lane parked at `swarming`/`exploring`
/// with no session, no grant and no live cell is exactly the D4 ghost
/// shape this rule must never resurrect. `finished_and_idle`, by contrast,
/// keeps counting all three cell kinds (`doing`/`waiting`/`stuck`) — D10
/// narrows only what counts as live *work for placement*, not what counts
/// as a live *cell* still open against a closed feature, so today's
/// Finished behaviour is preserved exactly.
///
/// - **In Progress** membership: any feature with live work that is not
///   finished-and-idle. Within that branch, the card additionally carries
///   a `Waiting on you — ` line (D5, D7, D8) when its current-stop gate
///   ([`bee_gate_current_stop`]: the independent-review gate itself never
///   counts, since that gate is user-invoked on its own schedule, never a
///   blocking stop) is still unapproved, OR it is the active feature while
///   `.bee/HANDOFF.json` reads as a genuine pause (never a `"planned-next"`
///   clean stop) — the note carries no feature name of its own
///   (`compute_attention_items`'s own doc comment says so), so it is
///   folded onto whichever feature `state.json` currently names active.
///   Either pull is suppressed while `working_now` (`waiting-means-stopped-1`):
///   a live session (`BeeSession.live`, `SESSION_LIVE_MINUTES`) bound to
///   this feature's lane whose own `heartbeat_age_minutes` is inside
///   [`WORKING_MINUTES`] — tighter than `SESSION_LIVE_MINUTES` on purpose
///   (see that constant's own doc). An agent still actively on the feature
///   right now — mid interview, gate not yet asked for — carries no
///   `Waiting on you` line; a granted worktree never counts toward
///   `working_now` on its own, since a grant is not a heartbeat and a
///   parked worktree with a gate owed is exactly the case this line exists
///   for.
/// - **Finished** membership: no live *cells* left (a bound session or a
///   granted worktree naming a closed feature's lane never drags it back
///   out — board-finished-wins-1) AND either a lane `phase` of exactly
///   `"compounding-complete"` (bee's own terminal phase — `"terminal"` is
///   a string bee never writes) OR a `.bee/cells/archive/<feature>/`
///   directory of its own (`list_archived_feature_dirs`, checked once up
///   front and reused as a set — no extra store read per feature),
///   including every feature that directory names but never had a lane or
///   active-feature placement at all. Both sourced from
///   `read_archived_cells` for their own done/total counts and last
///   activity, since a finished feature's live `cell_counts` are typically
///   zero (its cells already moved to archive).
///
/// This is also D4's ghost-card fix, carried forward: the pre-fh-1 board
/// rendered a card for ANY phase_board feature sitting on an unapproved
/// gate, regardless of whether it had any live cells left — six merged,
/// fully archived lanes kept showing "gate awaiting your decision" for
/// that reason. Gating the `Waiting on you` line on live work closes that
/// permanently: a stale lane with zero live cells now renders in Compound
/// (its `phase` still reads `"compounding"`) or Finished once its own
/// `phase` reaches `"compounding-complete"` or its cells land in the
/// archive directory (an orchestrator-run cleanup, out of this cell's
/// scope), never with a fabricated decision owed.
///
/// How fresh a bound session's heartbeat must be for its feature to count
/// as **working now** (`waiting-means-stopped-1`) — deliberately far
/// tighter than `waggledance-core`'s own [`SESSION_LIVE_MINUTES`] (30.0), which
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

/// The one In Progress card ([`bee_hub_card`]) names its feature, links to
/// its own detail page, its own done/total cell progress and its own
/// last-activity age ([`bee_fmt_trace_time`]) -- project-color-identity
/// (D2/D3) drops this card's own group and worktree-state chips entirely:
/// the kanban column heading already names the group, and a cross-project
/// card's own worktree state now reads on the project line instead (see
/// [`bee_hub_card`]'s own doc comment). hub-finished-compact strips all of
/// that from the other four columns (kanban-columns D12 grows this from
/// Finished alone to Todo, Review, Compound and Finished): none of the four
/// owes a decision or a progress reading, so each of their entries is one
/// dense row carrying only its name ([`bee_hub_finished_row`], now
/// parameterized by its own group key and href rather than hardcoding
/// Finished's), and each of the four columns pages ten rows at a time
/// behind nested `<details>` ([`bee_hub_finished_rows`]) rather than
/// growing without bound as more features accumulate. Every path-shaped
/// value a `BeeCell`/`BeeFeaturePhase` carries already arrives relativized
/// by `waggledance_core::bee::read_snapshot` (D9), so nothing further is
/// redacted here -- this view only escapes for HTML safety.
///
/// cross-board-2 splits what used to be one function into two seams: this
/// one reads `project`'s own archive off disk (D2 keeps that read exactly
/// where it always lived) and hands the result to [`bee_classify_features`],
/// which decides -- as plain data, no HTML -- which of the five columns
/// each feature belongs in; [`bee_render_hub_section`] then turns that data
/// back into this exact section, unchanged. The cross-project board
/// (`bee_cross_project_features_section`) reuses only the classification
/// step: it is handed archived-feature names from cross-board-1's
/// `read_rollup` instead of reading the archive itself, so a feature still
/// lands in the same column its own project's board would put it in,
/// merged with every other project's instead of rendered alone.
fn bee_feature_hub_section(
    project: &Project,
    snapshot: &BeeSnapshot,
    feature_panes: &std::collections::HashMap<String, Vec<TerminalPaneView>>,
) -> String {
    let archived_features: std::collections::HashSet<String> =
        list_archived_feature_dirs(&project.root_path)
            .into_iter()
            .collect();
    let placements = bee_classify_features(snapshot, &archived_features);
    bee_render_hub_section(project, &placements, &snapshot.backlog.pbis, feature_panes)
}

/// One feature already sorted into one of the feature hub's five columns
/// (kanban-columns D1) by [`bee_classify_features`] -- the render inputs
/// [`bee_hub_card`] or [`bee_hub_finished_row`] need, captured once so the
/// merge step (`bee_cross_project_features_section`) never re-touches
/// `BeeSnapshot`. The former `Waiting` variant is gone (D1); the feature it
/// used to carry now lands in `InProgress` with `BeeHubCardData::reason`
/// set to its `Waiting on you — ` line (D5, D7, D8). `Review`, `Compound`
/// and `Todo` share `BeeHubFinishedData` with `Finished` since all four
/// render through the same dense row (D12) and need nothing beyond a
/// feature name and its docs.
enum BeeHubPlacement {
    InProgress(BeeHubCardData),
    Finished(BeeHubFinishedData),
    Review(BeeHubFinishedData),
    Compound(BeeHubFinishedData),
    Todo(BeeHubFinishedData),
}

/// [`bee_hub_card`]'s render inputs for one In Progress card.
/// `worktree_label` (project-color-identity) is [`bee_hub_worktree_label`]'s
/// plain-text worktree state, not the retired chip's `(label, tone)` pair —
/// only ever read by the caller when it also has a `project_label` to pair
/// it with. `reason` (kanban-columns D7/D8) carries the retired Waiting
/// column's own line, `Waiting on you — ` plus its existing reason text,
/// when this feature is stopped on a gate or a paused handoff; `None` for
/// every other In Progress card.
///
/// kanban-live-signals D1/D2/D3 add three more fields, all sourced from
/// [`BeeSnapshot`] by [`bee_classify_features`] rather than re-read here:
/// `last_activity` (D1) is already [`bee_hub_effective_activity`]'s merge of
/// the cell-derived timestamp and `state.json`'s own `last_activity` — never
/// the raw cell-only value past this point. `run_state` and
/// `last_tool_call` (D1/D2) are `state.json`'s own fields, carried onto this
/// card only when the card's feature is the one `state.json` names active
/// (`BeeState::feature`) — `state.json` is one file per project describing
/// whichever single feature bee is currently pointed at, so attaching its
/// run state or its liveness pulse to a DIFFERENT In Progress card on the
/// same project would misattribute someone else's activity. `deferred` (D3)
/// has no such restriction: each `BeeDeferredEntry` already names its own
/// `feature`, so every card gets exactly its own unresolved debt regardless
/// of which feature `state.json` currently points at.
struct BeeHubCardData {
    feature: String,
    done: usize,
    total: usize,
    last_activity: Option<String>,
    worktree_label: String,
    reason: Option<String>,
    docs: Option<waggledance_core::bee::BeeFeatureDocs>,
    run_state: Option<String>,
    /// Same `is_active` gating as `run_state` (waiting-on-badge decision) —
    /// see [`bee_hub_run_state_badge`] for how this narrows the
    /// awaiting-approval badge to a live human wait.
    waiting_on_live: bool,
    last_tool_call: Option<String>,
    deferred: Vec<BeeDeferredEntry>,
}

/// [`bee_hub_finished_row`]'s render inputs for one Todo, Review, Compound
/// or Finished row (kanban-columns D12) -- every one of the four needs only
/// a feature name and its docs, so they share this one struct rather than
/// each growing its own copy.
struct BeeHubFinishedData {
    feature: String,
    docs: Option<waggledance_core::bee::BeeFeatureDocs>,
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
/// then every archived feature not already placed, sorted by name. Tested
/// in D11's fixed order: In Progress, then Finished, then Review, then
/// Compound, then Todo.
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
        // `live` keeps counting all three cell kinds (`doing`/`waiting`/
        // `stuck`) -- it feeds only `finished_and_idle` below, which must
        // keep reading today's exact Finished behaviour (D10's own
        // rationale: the narrowing is for placement's live-work test, never
        // for what still counts as a live *cell* against a closed feature).
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
        // (kanban-columns D10) An `open` cell alone is no longer live work
        // for placement -- only `doing` (claimed) or `stuck` cells count,
        // plus the three existing pulls. Counting `waiting` here (as the
        // pre-kanban-columns rule did) would swallow every feature Todo is
        // meant to hold into In Progress first, making D2's branch dead
        // code.
        let has_live_work = f.cell_counts.doing + f.cell_counts.stuck > 0
            || is_active
            || session_bound
            || worktree_bound;
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

        // (kanban-columns D11) Placement is tested in this fixed order: In
        // Progress, Finished, Review, Compound, Todo.
        if !finished_and_idle && has_live_work {
            // In Progress absorbs the retired Waiting column (D5, D7): a
            // gate stop or a paused handoff no longer moves the feature to
            // a separate column, it only adds the `Waiting on you — ` line
            // to this same card.
            let reason = if !working_now && (gate_stop.is_some() || waiting_via_handoff) {
                Some(match gate_stop {
                    Some((_, label)) => {
                        format!("Waiting on you — {label} gate awaiting your decision")
                    }
                    None => "Waiting on you — Work is parked, waiting on your decision".to_string(),
                })
            } else {
                None
            };
            let cell_activity =
                bee_hub_latest_activity(bee_hub_feature_cells(&snapshot.buckets, &f.feature));
            // kanban-live-signals D1/D2: `state.json` describes one project's
            // single currently-active feature at a time (`BeeState::feature`)
            // -- its `last_activity`, `run_state` and the tools.jsonl-derived
            // `last_tool_call` liveness pulse belong on THIS card only when
            // `is_active` says this is that feature, never borrowed onto a
            // sibling In Progress card for the same project.
            let last_activity = bee_hub_effective_activity(
                cell_activity.as_deref(),
                if is_active {
                    snapshot
                        .state
                        .as_ref()
                        .and_then(|s| s.last_activity.as_deref())
                } else {
                    None
                },
            );
            let run_state = if is_active {
                snapshot.state.as_ref().and_then(|s| s.run_state.clone())
            } else {
                None
            };
            // waiting-on-badge: the same `is_active` gating as `run_state`
            // above -- a live waiting_on mark only ever belongs to the
            // project's single currently-active feature.
            let waiting_on_live = if is_active {
                snapshot
                    .state
                    .as_ref()
                    .map(|s| s.waiting_on_live)
                    .unwrap_or(false)
            } else {
                false
            };
            let last_tool_call = if is_active {
                snapshot.last_tool_call.clone()
            } else {
                None
            };
            // kanban-live-signals D3: unlike the three fields above, deferred
            // debt carries its own `feature` per entry, so every In Progress
            // card gets exactly its own debt regardless of which feature is
            // currently active.
            let deferred: Vec<BeeDeferredEntry> = snapshot
                .deferred_queue
                .unresolved
                .iter()
                .filter(|e| e.feature.as_deref() == Some(f.feature.as_str()))
                .cloned()
                .collect();
            let worktree_label = bee_hub_worktree_label(
                &f.feature,
                &snapshot.worktrees,
                &snapshot.workspaces,
                false,
            );
            let docs = snapshot.feature_docs.get(f.feature.as_str()).cloned();
            placements.push(BeeHubPlacement::InProgress(BeeHubCardData {
                feature: f.feature.clone(),
                done: f.cell_counts.done,
                total: f.cell_counts.total,
                last_activity,
                worktree_label,
                reason,
                docs,
                run_state,
                waiting_on_live,
                last_tool_call,
                deferred,
            }));
        } else if is_finished {
            let docs = snapshot.feature_docs.get(f.feature.as_str()).cloned();
            placements.push(BeeHubPlacement::Finished(BeeHubFinishedData {
                feature: f.feature.clone(),
                docs,
            }));
        } else if snapshot
            .review
            .candidates
            .iter()
            .any(|c| c.feature == f.feature && c.status != BeeReviewStatus::Settled)
        {
            let docs = snapshot.feature_docs.get(f.feature.as_str()).cloned();
            placements.push(BeeHubPlacement::Review(BeeHubFinishedData {
                feature: f.feature.clone(),
                docs,
            }));
        } else if f.phase.as_deref() == Some("compounding") {
            let docs = snapshot.feature_docs.get(f.feature.as_str()).cloned();
            placements.push(BeeHubPlacement::Compound(BeeHubFinishedData {
                feature: f.feature.clone(),
                docs,
            }));
        } else if f.cell_counts.waiting > 0 && f.cell_counts.doing == 0 {
            // (D2) Cells exist, are all still `open`, and none is
            // `claimed` -- the Todo condition D10's narrowing exists to
            // keep reachable.
            let docs = snapshot.feature_docs.get(f.feature.as_str()).cloned();
            placements.push(BeeHubPlacement::Todo(BeeHubFinishedData {
                feature: f.feature.clone(),
                docs,
            }));
        }
        // else: no live work, not finished, no unresolved candidate, not
        // compounding, and either no cells at all or some already claimed
        // without going live (impossible today, but not this rule's job to
        // assume) — renders nowhere, matching the pre-redesign board's own
        // precedent for a feature with nothing yet to show.
    }

    let mut archive_only: Vec<&String> = archived_features
        .iter()
        .filter(|name| !placed.contains(name.as_str()))
        .collect();
    archive_only.sort();
    for feature in archive_only {
        let docs = snapshot.feature_docs.get(feature.as_str()).cloned();
        placements.push(BeeHubPlacement::Finished(BeeHubFinishedData {
            feature: feature.clone(),
            docs,
        }));
    }

    placements
}

/// Turns [`bee_classify_features`]'s per-project placements back into the
/// section [`bee_feature_hub_section`] rendered before cross-board-2 (D2)
/// -- every card and row carries no project label (`bee_hub_card`/
/// `bee_hub_finished_row`'s `project_label: None`), matching this
/// project's page having only ever shown itself. project-color-identity-3:
/// each card's `worktree_label` still travels through, since `bee_hub_card`
/// now folds it into that page's own slug subtitle in the chip row's
/// place. `feature_panes` (inprogress-priority-order-1, D5) is
/// `server.rs::bee_board`'s own `project_feature_panes` join, keyed by
/// feature name alone -- a single project needs no per-project outer key
/// the way the cross-project map below does. A feature absent from it (no
/// pane in its own checkout, or herdr unreachable) looks up to an empty
/// slice, rendering exactly as an empty pane slice always has -- so a
/// switched-off or herdr-down board still renders byte-identically to
/// before this feature. `pbis` (kanban-columns D2/D3) is
/// `snapshot.backlog.pbis` -- every `proposed` one renders through the same
/// [`bee_hub_finished_row`], appended to `todo_rows` after every feature
/// Todo row is already in it, so the column always reads features first,
/// PBIs second (D2); a single project needs no second accumulator for
/// this, unlike the cross-project merge below.
/// D7: the shared `blocked > working > rest` ranking
/// `terminals_status_rank` (`server.rs:3052`) already applies in the Agents
/// drawer -- matched here, never re-derived independently, so the two
/// rankings can never quietly drift apart. `"idle"`, `"done"`, `"unknown"`
/// and `"shell"` all fall into the same "everything else" tier -- only an
/// exact `"blocked"` or `"working"` status earns a tier of its own.
fn bee_hub_in_progress_tier(panes: &[TerminalPaneView]) -> u8 {
    if panes.iter().any(|p| p.status == "blocked") {
        0
    } else if panes.iter().any(|p| p.status == "working") {
        1
    } else {
        2
    }
}

/// D7's own sort key for one In Progress card: its terminal tier
/// ([`bee_hub_in_progress_tier`]), its own `last_activity` parsed as RFC
/// 3339 the same way [`bee_fmt_trace_time`] already parses it (`None` for
/// an absent or unparseable timestamp), and its feature name -- the final
/// tie-break. [`bee_hub_in_progress_cmp`] is the comparator that reads it,
/// and both boards build this key the same way so neither can drift from
/// the other (D4/D5).
fn bee_hub_in_progress_sort_key(
    feature: &str,
    last_activity: Option<&str>,
    panes: &[TerminalPaneView],
) -> (u8, Option<time::OffsetDateTime>, String) {
    let parsed = last_activity.and_then(|s| {
        time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
    });
    (bee_hub_in_progress_tier(panes), parsed, feature.to_string())
}

/// D7: one shared comparator over [`bee_hub_in_progress_sort_key`], used by
/// both the per-project board ([`bee_render_hub_section`]) and the
/// cross-project homepage ([`bee_cross_project_features_section`], D4) --
/// never a second spelling of the same rule. The tier wins first
/// (ascending: blocked, then working, then rest); within a tier the newest
/// `last_activity` wins, a `None` sorting last -- the same
/// newest-first/untimed-last shape the existing Finished re-sort (D10,
/// below) already imitates for its own timed-vs-untimed split; feature name
/// A→Z breaks every remaining tie.
fn bee_hub_in_progress_cmp(
    a: &(u8, Option<time::OffsetDateTime>, String),
    b: &(u8, Option<time::OffsetDateTime>, String),
) -> std::cmp::Ordering {
    a.0.cmp(&b.0)
        .then_with(|| match (&a.1, &b.1) {
            (Some(x), Some(y)) => y.cmp(x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        })
        .then_with(|| a.2.cmp(&b.2))
}

fn bee_render_hub_section(
    project: &Project,
    placements: &[BeeHubPlacement],
    pbis: &[BeePbi],
    feature_panes: &std::collections::HashMap<String, Vec<TerminalPaneView>>,
) -> String {
    let mut todo_rows: Vec<String> = Vec::new();
    // D7: collected as (sort key, rendered html) rather than appended
    // straight into a String -- placements arrive in name order
    // (`bee_classify_features`), so the cards must be re-sorted by
    // `bee_hub_in_progress_cmp` once every card is built, then flattened.
    type InProgressEntry = ((u8, Option<time::OffsetDateTime>, String), String);
    let mut in_progress_entries: Vec<InProgressEntry> = Vec::new();
    let mut review_rows: Vec<String> = Vec::new();
    let mut compound_rows: Vec<String> = Vec::new();
    let mut finished_rows: Vec<String> = Vec::new();
    let mut todo_count = 0usize;
    let mut in_progress_count = 0usize;
    let mut review_count = 0usize;
    let mut compound_count = 0usize;
    let mut finished_count = 0usize;
    let no_panes: Vec<TerminalPaneView> = Vec::new();

    for placement in placements {
        match placement {
            BeeHubPlacement::InProgress(data) => {
                in_progress_count += 1;
                let panes = feature_panes
                    .get(data.feature.as_str())
                    .unwrap_or(&no_panes);
                let key = bee_hub_in_progress_sort_key(
                    &data.feature,
                    data.last_activity.as_deref(),
                    panes,
                );
                let html = bee_hub_card(&BeeHubCardArgs {
                    project_id: &project.id,
                    feature: &data.feature,
                    group_key: "in-progress",
                    done: data.done,
                    total: data.total,
                    last_activity: data.last_activity.as_deref(),
                    worktree_label: &data.worktree_label,
                    reason: data.reason.as_deref(),
                    docs: data.docs.as_ref(),
                    project_label: None,
                    project_color: None,
                    panes,
                    run_state: data.run_state.as_deref(),
                    waiting_on_live: data.waiting_on_live,
                    last_tool_call: data.last_tool_call.as_deref(),
                    deferred: &data.deferred,
                });
                in_progress_entries.push((key, html));
            }
            BeeHubPlacement::Finished(data) => {
                finished_count += 1;
                finished_rows.push(bee_hub_finished_row(
                    "finished",
                    &bee_hub_feature_href(&project.id, &data.feature),
                    &data.feature,
                    data.docs.as_ref(),
                    None,
                    None,
                    None,
                ));
            }
            BeeHubPlacement::Review(data) => {
                review_count += 1;
                review_rows.push(bee_hub_finished_row(
                    "review",
                    &bee_hub_feature_href(&project.id, &data.feature),
                    &data.feature,
                    data.docs.as_ref(),
                    None,
                    None,
                    None,
                ));
            }
            BeeHubPlacement::Compound(data) => {
                compound_count += 1;
                compound_rows.push(bee_hub_finished_row(
                    "compound",
                    &bee_hub_feature_href(&project.id, &data.feature),
                    &data.feature,
                    data.docs.as_ref(),
                    None,
                    None,
                    None,
                ));
            }
            BeeHubPlacement::Todo(data) => {
                todo_count += 1;
                todo_rows.push(bee_hub_finished_row(
                    "todo",
                    &bee_hub_feature_href(&project.id, &data.feature),
                    &data.feature,
                    data.docs.as_ref(),
                    None,
                    None,
                    None,
                ));
            }
        }
    }
    // (kanban-columns D2/D3) Every `proposed` PBI joins Todo as a dense row
    // below the features already in `todo_rows` -- pushed after the loop
    // above, so it can never land ahead of a feature row. Links to the
    // owning project's own bee board, not a feature page (D3): a proposed
    // PBI generally has neither lane nor docs of its own.
    for pbi in pbis {
        if pbi.status == "proposed" {
            todo_count += 1;
            todo_rows.push(bee_hub_finished_row(
                "todo",
                &bee_hub_pbi_href(&project.id, &pbi.id),
                &pbi.title,
                None,
                None,
                None,
                None,
            ));
        }
    }
    // D7: sorted once, right before rendering -- blocked first, then
    // working, then the rest, newest activity breaking ties within a tier.
    in_progress_entries.sort_by(|a, b| bee_hub_in_progress_cmp(&a.0, &b.0));
    let in_progress_cards: String = in_progress_entries
        .into_iter()
        .map(|(_, html)| html)
        .collect();
    let todo_cards = bee_hub_finished_rows(&todo_rows);
    let review_cards = bee_hub_finished_rows(&review_rows);
    let compound_cards = bee_hub_finished_rows(&compound_rows);
    let finished_cards = bee_hub_finished_rows(&finished_rows);

    format!(
        r#"<section class="fg-card bee-hub" data-feature-hub="1">
  <h3 class="bee-panel__head">Features</h3>
  <div class="bee-hub__groups">
    {todo_group}
    {in_progress_group}
    {review_group}
    {compound_group}
    {finished_group}
  </div>
</section>"#,
        todo_group = bee_hub_group("Todo", "todo", todo_count, &todo_cards, "Nothing in Todo."),
        in_progress_group = bee_hub_group(
            "In Progress",
            "in-progress",
            in_progress_count,
            &in_progress_cards,
            "Nothing in progress."
        ),
        review_group = bee_hub_group(
            "Review",
            "review",
            review_count,
            &review_cards,
            "Nothing in Review."
        ),
        compound_group = bee_hub_group(
            "Compound",
            "compound",
            compound_count,
            &compound_cards,
            "Nothing in Compound."
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
/// (`docs/history/cross-board/CONTEXT.md` D1/D3/D4/D5/D7/D10, grown to five
/// columns by kanban-columns D1/D9): runs [`bee_classify_features`] once
/// per `(project, rollup)` pair -- the exact column rules the per-project
/// board applies to itself, D9's single shared classification rule -- then
/// merges the results into five flat, multi-project columns instead of one
/// block per project (D4), labels every card and every dense row with its
/// own project's name (D5), and orders and caps the merged Finished
/// sequence per D10/D7: every feature with a ship time first, most recently
/// shipped first, each row showing that time; then every feature without
/// one, alphabetically by feature name across all projects -- concatenating
/// each project's already-sorted list would not be globally sorted, so the
/// merge sorts again. Todo, Review and Compound carry no ship time of their
/// own, so their rows merge in the same per-project, alphabetical-within-
/// project order [`bee_classify_features`] already produces, exactly as
/// In Progress's cards always have. The column counts beside each heading
/// are the sum across projects. Archived-feature names and D10 ship times
/// come from `rollup.archived_features` (cross-board-1's `read_rollup`) --
/// this function performs no filesystem read of its own. An empty
/// `rollups` still renders the same five empty columns [`bee_hub_group`]
/// always shows for a column with nothing in it; whether to call this at
/// all when nothing qualifies (D9) is the caller's decision.
///
/// `feature_panes` (card-terminals-1) is the already-resolved join
/// (`server.rs::project_feature_panes`): for each project id, a map from
/// feature name to the terminal panes running in that feature's own
/// checkout. This function performs no boundary resolution of its own --
/// it only looks the pane list up per `(project.id, feature)` and hands it
/// to [`bee_hub_card`]. A project or feature absent from the map (the
/// switch off, herdr unreachable, or genuinely no pane) looks up to an
/// empty slice, which `bee_hub_card` renders as no badge container.
///
/// project-color-identity: this is "where the board page is assembled" --
/// [`bee_classify_features`] runs once per project first, so the project->
/// colour map ([`bee_cross_project_board_project_colors`]) can be built
/// once, up front, over every project id that actually placed a feature
/// somewhere (a card in either live group, or a Finished row), then handed
/// down into every [`bee_hub_card`]/[`bee_hub_finished_row`] call in the
/// render pass below rather than either renderer hashing its own.
///
/// kanban-columns D2/D3: Todo also merges every project's `proposed` PBIs
/// (`rollup.snapshot.backlog.pbis`), built into a *second* accumulator
/// (`todo_pbi_rows`) kept apart from `todo_rows` through the whole render
/// pass and only appended onto it once, right before paging -- appending
/// each project's PBI rows into `todo_rows` in placement order as they're
/// found would interleave one project's PBIs above another project's
/// features and break D2's features-before-PBIs order for the merged
/// column, not just within one project.
/// D3 (backlog-groom-2): the one concise warning strip a corrupt `.bee`
/// file earns on the cross-project board header — reusing the existing
/// `fg-banner fg-banner--warning` styling ([`assets/atelier/components.css`])
/// rather than any bespoke markup, and never the removed multi-item
/// Needs-attention panel this replaces. Sums [`BeeSnapshot::read_errors`]
/// across every rollup on the board (already computed by [`read_snapshot`],
/// no additional I/O here) rather than reading `snapshot.attention`, since
/// that list also folds in unrelated rules (blocked cells, a paused
/// handoff, ...) this strip is not about. Zero unreadable files across
/// every project renders nothing — no empty strip.
fn bee_cross_project_read_errors_strip(rollups: &[(&Project, &BeeProjectRollup)]) -> String {
    let count: usize = rollups
        .iter()
        .map(|(_, rollup)| rollup.snapshot.read_errors.len())
        .sum();
    if count == 0 {
        return String::new();
    }
    format!(
        r#"<div class="fg-banner fg-banner--warning" data-bee-read-errors="{count}"><span class="fg-banner__dot"></span><span class="fg-banner__body">{count} .bee file(s) could not be read; counts may be incomplete</span></div>"#,
        count = count,
    )
}

pub fn bee_cross_project_features_section(
    rollups: &[(&Project, &BeeProjectRollup)],
    feature_panes: &std::collections::HashMap<
        String,
        std::collections::HashMap<String, Vec<TerminalPaneView>>,
    >,
) -> String {
    let mut todo_rows: Vec<String> = Vec::new();
    let mut todo_pbi_rows: Vec<String> = Vec::new();
    // D4/D7: every project's In Progress cards collect into ONE list here
    // (sort key, project id, rendered html) rather than one block per
    // project -- a global sort cannot follow a per-project append order,
    // so this stops appending project-by-project and merges instead.
    type InProgressEntry = ((u8, Option<time::OffsetDateTime>, String), String);
    let mut in_progress_entries: Vec<InProgressEntry> = Vec::new();
    let mut review_rows: Vec<String> = Vec::new();
    let mut compound_rows: Vec<String> = Vec::new();
    let mut todo_count = 0usize;
    let mut in_progress_count = 0usize;
    let mut review_count = 0usize;
    let mut compound_count = 0usize;
    let no_panes: Vec<TerminalPaneView> = Vec::new();

    // Classify every project's placements once; reused below both to build
    // the colour map (over only the projects that actually placed
    // something) and to render.
    let classified: Vec<(&Project, &BeeProjectRollup, Vec<BeeHubPlacement>)> = rollups
        .iter()
        .map(|(project, rollup)| {
            let archived_names: std::collections::HashSet<String> = rollup
                .archived_features
                .iter()
                .map(|a| a.feature.clone())
                .collect();
            let placements = bee_classify_features(&rollup.snapshot, &archived_names);
            (*project, *rollup, placements)
        })
        .collect();
    let project_colors = bee_cross_project_board_project_colors(
        classified
            .iter()
            .filter(|(_, _, placements)| !placements.is_empty())
            .map(|(project, _, _)| project.id.as_str()),
    );

    struct FinishedEntry {
        shipped_at: Option<time::OffsetDateTime>,
        feature: String,
        html: String,
    }
    let mut finished: Vec<FinishedEntry> = Vec::new();
    let rfc3339 = time::format_description::well_known::Rfc3339;

    for (project, rollup, placements) in &classified {
        let shipped_by_feature: std::collections::HashMap<&str, Option<&str>> = rollup
            .archived_features
            .iter()
            .map(|a| (a.feature.as_str(), a.shipped_at.as_deref()))
            .collect();
        let project_panes = feature_panes.get(project.id.as_str());
        let project_color = project_colors.get(project.id.as_str()).copied();

        for placement in placements {
            match placement {
                BeeHubPlacement::InProgress(data) => {
                    in_progress_count += 1;
                    let panes = project_panes
                        .and_then(|m| m.get(data.feature.as_str()))
                        .unwrap_or(&no_panes);
                    let key = bee_hub_in_progress_sort_key(
                        &data.feature,
                        data.last_activity.as_deref(),
                        panes,
                    );
                    let html = bee_hub_card(&BeeHubCardArgs {
                        project_id: &project.id,
                        feature: &data.feature,
                        group_key: "in-progress",
                        done: data.done,
                        total: data.total,
                        last_activity: data.last_activity.as_deref(),
                        worktree_label: &data.worktree_label,
                        reason: data.reason.as_deref(),
                        docs: data.docs.as_ref(),
                        project_label: Some(&project.name),
                        project_color,
                        panes,
                        run_state: data.run_state.as_deref(),
                        waiting_on_live: data.waiting_on_live,
                        last_tool_call: data.last_tool_call.as_deref(),
                        deferred: &data.deferred,
                    });
                    in_progress_entries.push((key, html));
                }
                BeeHubPlacement::Finished(data) => {
                    let shipped_at_str = shipped_by_feature
                        .get(data.feature.as_str())
                        .copied()
                        .flatten();
                    let parsed =
                        shipped_at_str.and_then(|s| time::OffsetDateTime::parse(s, &rfc3339).ok());
                    let html = bee_hub_finished_row(
                        "finished",
                        &bee_hub_feature_href(&project.id, &data.feature),
                        &data.feature,
                        data.docs.as_ref(),
                        Some(&project.name),
                        project_color,
                        parsed.and(shipped_at_str),
                    );
                    finished.push(FinishedEntry {
                        shipped_at: parsed,
                        feature: data.feature.clone(),
                        html,
                    });
                }
                BeeHubPlacement::Review(data) => {
                    review_count += 1;
                    review_rows.push(bee_hub_finished_row(
                        "review",
                        &bee_hub_feature_href(&project.id, &data.feature),
                        &data.feature,
                        data.docs.as_ref(),
                        Some(&project.name),
                        project_color,
                        None,
                    ));
                }
                BeeHubPlacement::Compound(data) => {
                    compound_count += 1;
                    compound_rows.push(bee_hub_finished_row(
                        "compound",
                        &bee_hub_feature_href(&project.id, &data.feature),
                        &data.feature,
                        data.docs.as_ref(),
                        Some(&project.name),
                        project_color,
                        None,
                    ));
                }
                BeeHubPlacement::Todo(data) => {
                    todo_count += 1;
                    todo_rows.push(bee_hub_finished_row(
                        "todo",
                        &bee_hub_feature_href(&project.id, &data.feature),
                        &data.feature,
                        data.docs.as_ref(),
                        Some(&project.name),
                        project_color,
                        None,
                    ));
                }
            }
        }

        // (kanban-columns D2/D3) This project's own `proposed` PBIs join
        // the SECOND Todo accumulator, never `todo_rows` directly -- see
        // this function's own doc comment for why the merge must wait.
        for pbi in &rollup.snapshot.backlog.pbis {
            if pbi.status == "proposed" {
                todo_count += 1;
                todo_pbi_rows.push(bee_hub_finished_row(
                    "todo",
                    &bee_hub_pbi_href(&project.id, &pbi.id),
                    &pbi.title,
                    None,
                    Some(&project.name),
                    project_color,
                    None,
                ));
            }
        }
    }

    // (kanban-columns D2) Every project's feature Todo rows are already in
    // `todo_rows`; every project's PBI Todo rows join only now, appended as
    // one block after them -- so within the merged column every feature
    // row sits above every PBI row, matching D2's own order.
    todo_rows.extend(todo_pbi_rows);

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
    // D4/D7: sorted once, across every project, right before rendering --
    // the same comparator the per-project board uses, so the merged column
    // reads as one list rather than project-by-project blocks.
    in_progress_entries.sort_by(|a, b| bee_hub_in_progress_cmp(&a.0, &b.0));
    let in_progress_cards: String = in_progress_entries
        .into_iter()
        .map(|(_, html)| html)
        .collect();
    let todo_cards = bee_hub_finished_rows(&todo_rows);
    let review_cards = bee_hub_finished_rows(&review_rows);
    let compound_cards = bee_hub_finished_rows(&compound_rows);
    let finished_cards = bee_hub_finished_rows(&finished_rows);
    let read_errors_strip = bee_cross_project_read_errors_strip(rollups);

    format!(
        r#"<section class="fg-card bee-hub" data-feature-hub="cross-project">
  <h3 class="bee-panel__head">Features</h3>
  {read_errors_strip}
  <div class="bee-hub__groups">
    {todo_group}
    {in_progress_group}
    {review_group}
    {compound_group}
    {finished_group}
  </div>
</section>"#,
        read_errors_strip = read_errors_strip,
        todo_group = bee_hub_group("Todo", "todo", todo_count, &todo_cards, "Nothing in Todo."),
        in_progress_group = bee_hub_group(
            "In Progress",
            "in-progress",
            in_progress_count,
            &in_progress_cards,
            "Nothing in progress."
        ),
        review_group = bee_hub_group(
            "Review",
            "review",
            review_count,
            &review_cards,
            "Nothing in Review."
        ),
        compound_group = bee_hub_group(
            "Compound",
            "compound",
            compound_count,
            &compound_cards,
            "Nothing in Compound."
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
fn bee_hub_group(
    label: &str,
    key: &str,
    count: usize,
    cards_html: &str,
    empty_line: &str,
) -> String {
    let body = if cards_html.is_empty() {
        format!(r#"<p class="fg-empty">{}</p>"#, esc(empty_line))
    } else {
        format!(
            r#"<div class="bee-hub__cards">{cards_html}</div>"#,
            cards_html = cards_html
        )
    };
    format!(
        r#"<div class="bee-hub__group" data-hub-group="{key}" data-hub-count="{count}"><h4 class="bee-panel__subhead">{label} <span class="fg-chip fg-chip--neutral">{count}</span></h4>{body}</div>"#,
        key = key,
        count = count,
        label = esc(label),
        body = body,
    )
}

/// project-color-identity: the number of hues [`bee_hub_style`] defines
/// (`--bee-hub-project-1..10`, and the matching `bee-hub__shell--pN`/
/// `bee-hub__row-project--pN` modifiers) -- the modulus
/// [`bee_cross_project_board_project_colors`] slots distinct project ids
/// into.
const PROJECT_PALETTE_LEN: usize = 10;

/// project-color-identity: the cross-project board's own project->colour
/// map, built ONCE where the board page is assembled
/// (`bee_cross_project_features_section`) and threaded down into both
/// [`bee_hub_card`] and [`bee_hub_finished_row`] instead of either one
/// hashing its own id (the five-bucket FNV hash this replaces collided:
/// verified against the running board, `beedashboard` and `beehive` landed
/// on the same bucket with only three projects present). `project_ids` is
/// the caller's own iterator over every project id that actually renders
/// somewhere on the board -- a card in any group, or a Finished row -- so
/// a project contributing nothing never shifts another project's slot
/// (`cross_project_a_project_contributing_no_features_changes_nothing`).
/// Distinct ids are sorted, then handed out slots in that order,
/// `position % PROJECT_PALETTE_LEN`, 1-based so a `Some` from this map
/// plugs straight into a `bee-hub__shell--pN`/`bee-hub__row-project--pN`
/// class suffix. Sorted order is what keeps a project's colour stable
/// across renders; the accepted trade-off is that a newly registered
/// project sorting before others shifts their slot by one.
fn bee_cross_project_board_project_colors<'a>(
    project_ids: impl Iterator<Item = &'a str>,
) -> std::collections::HashMap<&'a str, u8> {
    let mut ids: Vec<&str> = project_ids.collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .enumerate()
        .map(|(position, id)| (id, (position % PROJECT_PALETTE_LEN) as u8 + 1))
        .collect()
}

/// The one In Progress feature card (D1) — hub-finished-compact retired the
/// Finished group's own use of this helper in favor of
/// [`bee_hub_finished_row`]'s dense line, and kanban-columns D1/D12 folded
/// the former Waiting column into In Progress and moved Todo/Review/Compound
/// onto that same dense row, so `group_key` in practice now only ever
/// arrives as `"in-progress"`. Name + link to its
/// own detail page, its own done/total cell progress (a `bee-progress`
/// bar, or no markup at all when `total == 0` — hub-finished-compact drops
/// the old "No cells recorded." filler paragraph) and its own last-activity
/// age ([`bee_fmt_trace_time`]) — project-color-identity (D3) drops this
/// card's own group and worktree chips: the kanban column heading already
/// names the group, and a cross-project card's own worktree state now
/// reads on the project line instead (below). `reason` carries the Waiting
/// group's own "why" line (its current-stop gate, or a paused handoff) —
/// `None` for In Progress, which has no such single reason to name. `docs`
/// (feature-titles) carries this feature's own `CONTEXT.md` reader result:
/// present with a title, the card's name becomes that human title with the
/// slug demoted to a small muted subtitle beneath it, plus the boundary
/// description as one clamped line; `None`, or a title-less record, falls
/// back to the slug alone, exactly as before this feature. `project_label`
/// is cross-board D5's project name; project-color-identity (D2) moved it
/// from a chip in the (now-gone) chip row to the subtitle line under the
/// card title in its own `bee-hub__project` class, replacing the slug
/// there (a title-less card with a project label still gains that
/// subtitle, so the project name is never dropped), and now appends the
/// card's own worktree state after a plain ` / ` — `worktree_label`
/// restates what [`bee_hub_worktree_chip`] would have shown on the now-gone
/// chip (an open worktree's branch name alone, `"worktree"` with no known
/// branch, `"merged"`, or `"Main"`), read from [`bee_hub_worktree_label`],
/// in a muted `bee-hub__project-worktree` span so it never competes with
/// the project name's own accent colour for attention. `project_color`
/// (the project's own slot from [`bee_cross_project_board_project_colors`],
/// built once by the caller and threaded down rather than hashed here)
/// gives the card's shell a `bee-hub__shell--pN` modifier so cards of the
/// same project share one fixed accent. `project_label: None` (every
/// per-project board call) renders no modifier class, byte-identical to
/// before this feature for the colour — but project-color-identity-3
/// still swaps the plain slug subtitle for `<slug> / <worktree>`, in the
/// same `bee-hub__slug` class and reusing the `Some` arm's exact
/// worktree-half span, since a per-project board still needs to say which
/// of its own features has an open worktree even without a project name to
/// hang it off of. A title-less card there (whose own title is already the
/// slug, so feature-titles/hub-fallbacks forbids repeating it below) keeps
/// the subtitle for the worktree state alone, with no slug half and so no
/// separator — project-color-identity-4.
///
/// `panes` (card-terminals-1) is the terminal panes running in this
/// feature's own checkout -- the worktree-vs-main-checkout join is already
/// resolved by the caller (`server.rs::project_feature_panes`); this
/// function only renders them, as a sibling `<nav>` after the card's own
/// `<details>` (card-collapse-inprogress: previously the card's own `<a>`)
/// rather than nested inside it -- an anchor inside the details' own body
/// would still be legal HTML, but keeping the badges outside is what lets
/// them show collapsed and expanded alike (D1), the same reason
/// `project_badges` sits beside `proj-row__link` rather than inside it.
/// card-badge-inside wraps that details/nav pair in a
/// `<div class="fg-card bee-hub__shell">`, moving the box paint
/// (`.fg-card`'s border, background, radius, padding) off the card and
/// onto the shell, so the badges land inside the card's own painted box at
/// its foot instead of on a bare row underneath it -- the details element
/// itself keeps only `bee-hub__card`. The nav carries an extra
/// `bee-hub__badges` class in this context
/// (a hairline top rule replacing the project-row's own bottom/right
/// padding), reusing [`terminal_badges_nav`]'s exact per-badge markup and
/// the accessible label "Terminals in this checkout" rather than anything
/// naming the feature: for a Main feature the panes are shared with every
/// other Main feature of that project, so the label must not claim
/// otherwise. Empty `panes` renders no `<nav>` at all -- the shell still
/// wraps the bare details/summary, so the card looks exactly as it did
/// before this feature aside from the collapse itself -- and the
/// per-project board's own call
/// (`bee_render_hub_section`) always passes an empty slice, so this path
/// renders every time there too.
///
/// kanban-live-signals adds three trailing arguments, all already scoped to
/// this card by [`BeeHubCardData`]'s own doc comment (`bee_classify_features`
/// does the active-feature gating, never this function): `run_state` (D2)
/// renders [`bee_hub_run_state_badge`] in the collapsed `<summary>` itself,
/// beside the title, so the badge stays visible without expanding the card
/// -- `None` renders nothing there. `last_activity`'s own line
/// (`activity_html` below) is `last_activity` unchanged, but now gains
/// [`bee_hub_is_working_now`]'s pulse dot beside it when `last_tool_call`
/// (D1) lands inside the liveness window. `deferred` (D3) renders
/// [`bee_hub_deferred_badge`] in the card's own body -- never the
/// `<summary>`, whose content model cannot nest a `<details>` the way that
/// badge's click-to-reveal detail needs.
struct BeeHubCardArgs<'a> {
    project_id: &'a str,
    feature: &'a str,
    group_key: &'a str,
    done: usize,
    total: usize,
    last_activity: Option<&'a str>,
    worktree_label: &'a str,
    reason: Option<&'a str>,
    docs: Option<&'a waggledance_core::bee::BeeFeatureDocs>,
    project_label: Option<&'a str>,
    project_color: Option<u8>,
    panes: &'a [TerminalPaneView],
    run_state: Option<&'a str>,
    /// Same `is_active` gating as `run_state` (waiting-on-badge decision) —
    /// see [`bee_hub_run_state_badge`] for how this narrows the
    /// awaiting-approval badge to a live human wait.
    waiting_on_live: bool,
    last_tool_call: Option<&'a str>,
    deferred: &'a [BeeDeferredEntry],
}

fn bee_hub_card(args: &BeeHubCardArgs<'_>) -> String {
    let BeeHubCardArgs {
        project_id,
        feature,
        group_key,
        done,
        total,
        last_activity,
        worktree_label,
        reason,
        docs,
        project_label,
        project_color,
        panes,
        run_state,
        waiting_on_live,
        last_tool_call,
        deferred,
    } = *args;
    let title = docs
        .and_then(|d| d.title.as_deref())
        .filter(|t| !t.is_empty());
    // project-color-identity: `project_label: Some` swaps the slug subtitle
    // for the project name plus its worktree state -- and, unlike the old
    // slug-only subtitle (which only ever showed beside a human title),
    // still shows it under a title-less card too, so the project name is
    // never dropped.
    // project-color-identity-3: `project_label: None` (every per-project
    // board call) still only renders the slug subtitle beside a human
    // title (hub-fallbacks: a title-less card's own title already is the
    // slug, so a second copy of it below would be a redundant subtitle),
    // but when it does render, it now appends the card's own worktree
    // state after the same ` / ` separator, reusing the `Some` arm's exact
    // worktree-half span -- a per-project board still needs to say which
    // of its features has an open worktree even though it never shows a
    // project name to hang that information off.
    // project-color-identity-4: a title-less card on that board renders
    // the worktree state on its own, with no slug half and so no
    // separator -- the slug is already the card's title, and hub-fallbacks
    // forbids a second copy of it, but that is no reason for the card to
    // go without the one thing the subtitle now carries.
    let subtitle_html = match project_label {
        Some(label) => format!(
            r#"<div class="bee-hub__project">{label}<span class="bee-hub__project-worktree"> / {worktree}</span></div>"#,
            label = esc(label),
            worktree = esc(worktree_label),
        ),
        None => match title {
            Some(_) => format!(
                r#"<div class="bee-hub__slug">{feature}<span class="bee-hub__project-worktree"> / {worktree}</span></div>"#,
                feature = esc(feature),
                worktree = esc(worktree_label),
            ),
            None => format!(
                r#"<div class="bee-hub__slug"><span class="bee-hub__project-worktree">{worktree}</span></div>"#,
                worktree = esc(worktree_label),
            ),
        },
    };
    // card-collapse-inprogress D1/D2: the card's own name alone (its
    // CONTEXT title, or the slug fallback) is all the collapsed header
    // shows -- `subtitle_html` above now moves into the expandable body
    // below instead of riding along with the title the way the old
    // `name_html` bundled them.
    let title_html = match title {
        Some(t) => format!(
            r#"<div class="fg-card__title">{title}</div>"#,
            title = esc(t)
        ),
        None => format!(
            r#"<div class="fg-card__title">{feature}</div>"#,
            feature = esc(feature)
        ),
    };
    let desc_html = match docs
        .and_then(|d| d.description.as_deref())
        .filter(|d| !d.is_empty())
    {
        Some(d) => format!(r#"<p class="bee-hub__desc">{}</p>"#, esc(d)),
        None => String::new(),
    };
    let progress_html = if total == 0 {
        // hub-finished-compact: an empty card renders no markup at all here
        // — no fabricated "No cells recorded." paragraph — since a card
        // with genuinely nothing to report needs no line saying so.
        String::new()
    } else {
        let percent = (done * 100).checked_div(total).unwrap_or(0);
        format!(
            r#"<div class="bee-progress"><div class="bee-progress__bar" style="width: {percent}%"></div></div><p class="bee-hub__progress-label">{done}/{total} cell{plural} done</p>"#,
            percent = percent,
            done = done,
            total = total,
            plural = if total == 1 { "" } else { "s" },
        )
    };
    // kanban-live-signals D1: the pulse dot rides beside this same line,
    // never its own paragraph -- it is a live annotation of "Last
    // activity", not a second fact.
    let pulse_html = if bee_hub_is_working_now(last_tool_call, time::OffsetDateTime::now_utc()) {
        r#" <span class="bee-hub__pulse" role="status" aria-label="Working now" title="Working now"></span>"#
            .to_string()
    } else {
        String::new()
    };
    let activity_html = match last_activity {
        Some(iso) => format!(
            r#"<p class="bee-cell__meta">Last activity {}{pulse}</p>"#,
            esc(&bee_fmt_trace_time(iso)),
            pulse = pulse_html,
        ),
        None => format!(
            r#"<p class="bee-cell__meta">No activity recorded.{pulse}</p>"#,
            pulse = pulse_html
        ),
    };
    // kanban-live-signals D2: rendered in the collapsed `<summary>` itself
    // (see this function's own doc comment) so the badge stays visible
    // without expanding the card.
    let run_state_html = match run_state {
        Some(rs) if !rs.is_empty() => bee_hub_run_state_badge(rs, waiting_on_live),
        _ => String::new(),
    };
    // kanban-live-signals D3: in the card's own body, grouped with the
    // other "needs a look" lines above the plain activity timestamp.
    let deferred_html = bee_hub_deferred_badge(deferred);
    let reason_html = match reason {
        Some(r) if !r.is_empty() => format!(
            r#"<p class="bee-cell__meta bee-hub__reason">{}</p>"#,
            esc(r)
        ),
        _ => String::new(),
    };
    // D8: a second, independent reason line -- appended after any existing
    // gate/handoff line above, never in its place, since a paused feature
    // and a blocked terminal are two different things waiting on the user
    // at once. Exact wording, matching the existing gate line's own em-dash
    // punctuation (`server.rs`'s `Waiting on you — {label} gate ...`).
    let blocked_reason_html = if panes.iter().any(|p| p.status == "blocked") {
        r#"<p class="bee-cell__meta bee-hub__reason">Waiting on you — a terminal is blocked</p>"#
            .to_string()
    } else {
        String::new()
    };
    // project-color-identity: no chip row at all any more -- the group
    // chip is redundant with the kanban column heading and the worktree
    // chip moved to the project line above -- and the shell gains a
    // `bee-hub__shell--pN` modifier from the project's own colour slot
    // (built once by the caller, see this function's doc comment), so
    // every card of the same project shares one accent. No `project_color`
    // means no modifier at all, keeping a per-project board card's shell
    // exactly `fg-card bee-hub__shell`.
    let shell_class = match project_color {
        Some(idx) => format!("fg-card bee-hub__shell bee-hub__shell--p{idx}"),
        None => "fg-card bee-hub__shell".to_string(),
    };
    // card-terminals-1: a sibling of the card's own `<a>`, never nested
    // inside it (see this function's doc comment) -- empty when `panes` is
    // empty, so a feature with no pane in its own checkout renders no
    // `<nav>` at all. card-badge-inside: the extra `bee-hub__badges` class
    // draws the hairline rule that separates the badges from the card's
    // own content while both sit inside the shared `bee-hub__shell` box.
    let terminal_badges_html = terminal_badges_nav(
        project_id,
        panes,
        "Terminals in this checkout",
        "bee-hub__badges",
    );
    // card-collapse-inprogress D1/D3/D6: a native `<details>` with no
    // `open` attribute renders every card collapsed on every page load,
    // with no persisted state and no JavaScript -- clicking the
    // `<summary>` header is the browser's own toggle. D4 moves the
    // feature-detail link off the card entirely and into its own row at
    // the top of the expandable body, since a `<details>`/`<summary>`
    // pair, unlike the old whole-card `<a>`, cannot itself be a link.
    format!(
        r#"<div class="{shell_class}"><details class="bee-hub__card" data-hub-group="{group_key}"><summary class="bee-hub__summary">{title_html}{run_state_html}<span class="bee-hub__chev" aria-hidden="true">›</span></summary><div class="bee-hub__body"><a class="bee-hub__detail-link" href="/p/{pid}/_bee/feature/{feature_href}">Feature detail<span aria-hidden="true"> →</span></a>{subtitle_html}{desc_html}{progress_html}{reason_html}{blocked_reason_html}{deferred_html}{activity_html}</div></details>{terminal_badges_html}</div>"#,
        shell_class = shell_class,
        group_key = group_key,
        title_html = title_html,
        run_state_html = run_state_html,
        pid = esc(project_id),
        feature_href = esc(feature),
        subtitle_html = subtitle_html,
        desc_html = desc_html,
        progress_html = progress_html,
        reason_html = reason_html,
        blocked_reason_html = blocked_reason_html,
        deferred_html = deferred_html,
        activity_html = activity_html,
        terminal_badges_html = terminal_badges_html,
    )
}

/// A feature's own detail-page href (`/p/:id/_bee/feature/:feature`) — the
/// one place every caller of [`bee_hub_finished_row`] below builds it, so
/// the four dense columns (kanban-columns D12) all link the same way
/// [`bee_hub_card`] already does.
fn bee_hub_feature_href(project_id: &str, feature: &str) -> String {
    format!(
        "/p/{pid}/_bee/feature/{feature_href}",
        pid = esc(project_id),
        feature_href = esc(feature)
    )
}

/// (pbi-detail-1) A proposed PBI's own detail-page href
/// (`/p/:id/_bee/pbi/:pbi_id`) -- [`bee_hub_feature_href`]'s sibling for a
/// PBI that has no feature of its own yet. Both proposed-PBI board renderers
/// (the per-project board and the homepage Kanban tab) build their row's
/// href through this one function, so they can never drift apart.
fn bee_hub_pbi_href(project_id: &str, pbi_id: &str) -> String {
    format!(
        "/p/{pid}/_bee/pbi/{pbi_href}",
        pid = esc(project_id),
        pbi_href = esc(pbi_id)
    )
}

/// (pbi-detail-1) The owning project's whole bee board href
/// (`/p/:id/_bee`) — [`bee_pbi_page`]'s own "back to board" link, the same
/// destination the proposed-PBI board rows pointed at before they gained
/// their own page.
fn bee_hub_project_bee_href(project_id: &str) -> String {
    format!("/p/{pid}/_bee", pid = esc(project_id))
}

/// One dense row (hub-finished-compact; kanban-columns D12 grows this from
/// Finished's own row to the shared row for Todo, Review, Compound and
/// Finished alike): just the feature's own name — its CONTEXT title when
/// [`docs`] carries one, else its slug — linking to `href`, and carrying
/// `data-hub-group="{group_key}"` so each group's own filtering/testing
/// hooks keep working. `group_key` and `href` are both parameters
/// (kanban-columns) rather than hardcoded to Finished's own `"finished"`
/// key and feature-detail link, so the four columns share this one
/// renderer instead of each growing its own copy; every existing Finished
/// call site keeps passing `"finished"` and this same feature-detail href,
/// so its own markup renders byte-identical to before this feature.
/// Deliberately none of `bee_hub_card`'s description, progress bar,
/// worktree chip, group chip or last-activity line: none of these four
/// columns' features owes a decision or a progress reading, so the board
/// only needs to name each one — its detail page is one click away.
/// `project_label` (cross-board D5) and `shipped_at` (cross-board D10,
/// already relative-formatted through [`bee_fmt_trace_time`], and only
/// ever `Some` for a Finished row) both default to `None` for every
/// per-project board call, which renders byte-identical to before
/// cross-board-2; the cross-project board passes a project label for all
/// four columns but a ship time only for Finished. project-color-identity:
/// `project_label: Some` also carries a `bee-hub__row-project--pN` modifier
/// from `project_color` (the project's own slot from
/// [`bee_cross_project_board_project_colors`], built once by the caller and
/// threaded down exactly as [`bee_hub_card`] receives it, never hashed
/// separately here), so a project reads in one fixed colour everywhere it
/// appears.
#[allow(clippy::too_many_arguments)]
fn bee_hub_finished_row(
    group_key: &str,
    href: &str,
    feature: &str,
    docs: Option<&waggledance_core::bee::BeeFeatureDocs>,
    project_label: Option<&str>,
    project_color: Option<u8>,
    shipped_at: Option<&str>,
) -> String {
    let title = docs
        .and_then(|d| d.title.as_deref())
        .filter(|t| !t.is_empty());
    let name = title.unwrap_or(feature);
    let project_html = match (project_label, project_color) {
        (Some(label), Some(idx)) => format!(
            r#"<span class="bee-hub__row-project bee-hub__row-project--p{idx}">{label}</span> "#,
            idx = idx,
            label = esc(label),
        ),
        (Some(label), None) => {
            format!(
                r#"<span class="bee-hub__row-project">{label}</span> "#,
                label = esc(label)
            )
        }
        (None, _) => String::new(),
    };
    let time_html = match shipped_at {
        Some(iso) => format!(
            r#" <span class="bee-hub__row-time">{}</span>"#,
            esc(&bee_fmt_trace_time(iso))
        ),
        None => String::new(),
    };
    format!(
        r#"<a class="bee-hub__row" data-hub-group="{group_key}" href="{href}">{project_html}{name}{time_html}</a>"#,
        group_key = group_key,
        href = href,
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
    if let Some(w) = worktrees
        .iter()
        .find(|w| w.feature.as_deref() == Some(feature))
    {
        let label = match w.branch.as_deref() {
            Some(b) if !b.is_empty() => format!("Open · {b}"),
            _ => "Open worktree".to_string(),
        };
        return (label, "info");
    }
    let feature_branch = format!("wt/{feature}");
    let grant_existed = workspaces
        .iter()
        .any(|w| w.branch.as_deref() == Some(feature_branch.as_str()));
    if finished && grant_existed {
        ("Merged".to_string(), "success")
    } else {
        ("Main".to_string(), "neutral")
    }
}

/// project-color-identity: the cross-project board card's own worktree
/// half of the project line ([`bee_hub_card`]) — restates
/// [`bee_hub_worktree_chip`]'s exact same resolution (open-with-branch,
/// open-with-no-branch, merged-and-gone, or never-worked-in-its-own-
/// worktree) over the same `worktrees`/`workspaces` inputs, as plain text
/// for that line rather than a standalone chip: an open worktree with a
/// known branch reads the branch name alone (never the chip's own
/// "Open · " prefix, which only made sense standing next to a status
/// icon), an open worktree with no branch reads `"worktree"`, a merged one
/// reads `"merged"`, and every other case reads `"Main"` — lowercase for
/// the first three since they now sit mid-sentence after "project / ",
/// `"Main"` kept capitalized as the one case that reads as a proper noun
/// (the checkout, not a worktree state). Left a sibling of
/// `bee_hub_worktree_chip` rather than reusing it: that function still
/// backs the feature detail page's own worktree chip
/// (`bee_feature_chip_row`), which is out of this cell's scope and keeps
/// its existing "Open · <branch>"/"Merged" spellings untouched.
fn bee_hub_worktree_label(
    feature: &str,
    worktrees: &[BeeWorktree],
    workspaces: &[BeeWorkspace],
    finished: bool,
) -> String {
    if let Some(w) = worktrees
        .iter()
        .find(|w| w.feature.as_deref() == Some(feature))
    {
        return match w.branch.as_deref() {
            Some(b) if !b.is_empty() => b.to_string(),
            _ => "worktree".to_string(),
        };
    }
    let feature_branch = format!("wt/{feature}");
    let grant_existed = workspaces
        .iter()
        .any(|w| w.branch.as_deref() == Some(feature_branch.as_str()));
    if finished && grant_existed {
        "merged".to_string()
    } else {
        "Main".to_string()
    }
}

/// A feature's most recent `claimed_at`/`capped_at` across the cells
/// handed to it (`bee_hub_feature_cells` for a live feature,
/// `read_archived_cells` for a finished one) — the later of the two RFC
/// 3339 timestamps a cell carries, across every cell in the slice; `None`
/// when none of them parse or the slice is empty, never a fabricated
/// "just now". kanban-live-signals D1: this cell-derived value is only ONE
/// input the In Progress card's own activity line now takes — see
/// [`bee_hub_effective_activity`], which merges this with `state.json`'s own
/// `last_activity`; a finished card still reads this function alone, since
/// D1's merge is scoped to the live In Progress card only.
fn bee_hub_latest_activity<'a>(cells: impl Iterator<Item = &'a BeeCell>) -> Option<String> {
    let mut latest: Option<(time::OffsetDateTime, String)> = None;
    for c in cells {
        for ts in [c.claimed_at.as_deref(), c.capped_at.as_deref()]
            .into_iter()
            .flatten()
        {
            if let Ok(t) =
                time::OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339)
            {
                let newer = latest.as_ref().map(|(lt, _)| t > *lt).unwrap_or(true);
                if newer {
                    latest = Some((t, ts.to_string()));
                }
            }
        }
    }
    latest.map(|(_, s)| s)
}

/// kanban-live-signals D1: an In Progress card's "Last activity" is the
/// LATER of the cell-derived value ([`bee_hub_latest_activity`]) and
/// `state.json`'s own `last_activity` (`BeeState::last_activity`, carried
/// onto the card only for the currently-active feature — see
/// [`BeeHubCardData`]'s own doc comment). Either side missing or unparsable
/// falls back to whichever side still parses; both missing is still `None`,
/// never a fabricated "just now". The cell-derived fallback this preserves
/// is exactly the CONTEXT.md D1 exception for a bee version whose
/// `state.json` predates `last_activity` — that project must keep reading a
/// real timestamp off its cells rather than going blank.
fn bee_hub_effective_activity(
    cell_activity: Option<&str>,
    state_activity: Option<&str>,
) -> Option<String> {
    let rfc3339 = time::format_description::well_known::Rfc3339;
    let parse = |s: &str| time::OffsetDateTime::parse(s, &rfc3339).ok();
    let cell = cell_activity.and_then(|s| parse(s).map(|t| (t, s)));
    let state = state_activity.and_then(|s| parse(s).map(|t| (t, s)));
    match (cell, state) {
        (Some((ct, cs)), Some((st, ss))) => Some(if st >= ct { ss } else { cs }.to_string()),
        (Some((_, cs)), None) => Some(cs.to_string()),
        (None, Some((_, ss))) => Some(ss.to_string()),
        (None, None) => None,
    }
}

/// kanban-live-signals D1: the ~2-minute "working now" window CONTEXT.md
/// locks in order of magnitude, exact width left to agent discretion — see
/// [`bee_hub_is_working_now`].
const TOOL_CALL_PULSE_MINUTES: i64 = 2;

/// kanban-live-signals D1: true when `last_tool_call`
/// (`BeeSnapshot::last_tool_call`, the newest `ts` cell 1's bounded
/// `tools.jsonl` tail read could find) parses as RFC 3339 and lands within
/// [`TOOL_CALL_PULSE_MINUTES`] of `now`, in EITHER direction — a small clock
/// skew that puts the timestamp a few seconds into the future must not flip
/// the pulse off, so this is a plain magnitude check on the gap rather than
/// a one-sided "not yet" test. A missing or unparsable timestamp never
/// pulses.
fn bee_hub_is_working_now(last_tool_call: Option<&str>, now: time::OffsetDateTime) -> bool {
    let Some(ts) = last_tool_call else {
        return false;
    };
    let Ok(parsed) =
        time::OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339)
    else {
        return false;
    };
    (now - parsed).abs() <= time::Duration::minutes(TOOL_CALL_PULSE_MINUTES)
}

/// kanban-live-signals D2: `run_state`'s colored badge — `.fg-chip`'s
/// existing five tone modifiers (`crates/waggledance/assets/atelier/components.css`,
/// "CHIP / BADGE"), never a new palette of this feature's own. Every
/// recognized state gets a distinct tone; every other state reads calmer.
/// waiting-on-badge (narrowing kanban-live-signals D2): `awaiting-approval`
/// alone gets `--danger`, the highest-contrast tone the existing chip set
/// carries, per CONTEXT.md D2's "visually prominent" requirement — but only
/// while `waiting_on_live` is true, i.e. a human is being waited on right
/// now. bee derives `awaiting-approval` whenever any gate is pending with
/// none later approved, and the user-invoked review gate routinely sits
/// pending with nobody actually waiting, so `awaiting-approval` without a
/// live mark renders the same neutral "Unreviewed" chip an unrecognized
/// state would. An unrecognized future state (bee writes whatever string it
/// wants, verbatim — see `BeeState::run_state`) still renders, in the
/// neutral tone, with its own raw text rather than being swallowed.
fn bee_hub_run_state_badge(run_state: &str, waiting_on_live: bool) -> String {
    let (class, label) = match run_state {
        "shaping" => ("fg-chip--neutral", "Shaping"),
        "awaiting-approval" if waiting_on_live => ("fg-chip--danger", "Awaiting approval"),
        "awaiting-approval" => ("fg-chip--neutral", "Unreviewed"),
        "running" => ("fg-chip--accent", "Running"),
        "blocked" => ("fg-chip--warning", "Blocked"),
        "done" => ("fg-chip--success", "Done"),
        _ => ("fg-chip--neutral", run_state),
    };
    format!(
        r#"<span class="fg-chip {class} bee-hub__run-state">{label}</span>"#,
        class = class,
        label = esc(label),
    )
}

/// kanban-live-signals D3: the deferred-debt badge, empty `entries`
/// rendering nothing at all (D3's "zero debt renders nothing"). Reuses the
/// board's own click-to-reveal `<details>` idiom (`bee-cell__detail`, the
/// backlog card's "Condition of satisfaction" disclosure) rather than a bare
/// `title` tooltip, so the per-entry kind and reason are reachable on
/// mobile, not just on hover. Lives in the card's own `<div
/// class="bee-hub__body">` (never inside `<summary>`, whose content model
/// forbids nesting a `<details>`), one `<li>` per unresolved entry, its kind
/// defaulting to "unspecified" and its reason to "no reason recorded" —
/// both `Option` on [`BeeDeferredEntry`], never fabricated past that.
fn bee_hub_deferred_badge(entries: &[BeeDeferredEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let count = entries.len();
    let items: String = entries
        .iter()
        .map(|e| {
            format!(
                r#"<li><span class="bee-hub__deferred-kind">{kind}</span> — {reason}</li>"#,
                kind = esc(e.kind.as_deref().unwrap_or("unspecified")),
                reason = esc(e.reason.as_deref().unwrap_or("no reason recorded")),
            )
        })
        .collect();
    format!(
        r#"<details class="bee-hub__deferred"><summary class="fg-badge">{count} deferred</summary><ul class="bee-hub__deferred-list">{items}</ul></details>"#,
        count = count,
        items = items,
    )
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
/// `compute_shipped_features` (`waggledance_core::bee`) applies no
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
/// `waggledance_core::bee::read_snapshot`, so this view only formats what it is
/// handed, never recomputes any of that logic. This wrapper used to carry
/// two more cards, the Sessions panel (bbp-16/D2: sessions, worktrees,
/// workspaces) and Process health (bbp-16/D5: file-lock contention, model
/// tier mix, gate bypass, `read_errors`) — board-trim (D1) drops both,
/// along with the view functions and CSS that only ever rendered them.
/// `waggledance_core`'s readers for that dropped data are untouched; only this
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
/// `waggledance_core::bee`'s own `RECENT_DETAIL_CAP` already applies to findings,
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
/// `.bee/reviews/*.json` by `waggledance_core::bee`'s own review join, with the
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
        format!(
            "{m} minute{plural} ago",
            plural = if m == 1 { "" } else { "s" }
        )
    } else if mins < 60.0 * 24.0 {
        let h = (mins / 60.0).round().max(1.0) as i64;
        format!(
            "{h} hour{plural} ago",
            plural = if h == 1 { "" } else { "s" }
        )
    } else {
        let d = (mins / (60.0 * 24.0)).round().max(1.0) as i64;
        format!(
            "{d} day{plural} ago",
            plural = if d == 1 { "" } else { "s" }
        )
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
/// `waggledance_core::bee::BeeCell` deliberately leaves out (`action`, `verify`,
/// `read_first`, `decisions`, `must_haves.truths`, and the rest of `trace`
/// beyond `worker`/`claimed_at`/`capped_at`). Built by
/// `server.rs::cell_full_from_json` straight from the raw cell JSON, with
/// every path-shaped field already relativized against the project root
/// before it reaches here (same contract as `waggledance_core::bee::BeeCell`) —
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
            let lis: String = items
                .iter()
                .map(|i| format!("<li>{}</li>", esc(i)))
                .collect();
            format!("<ul>{lis}</ul>")
        }
    };

    let decisions = if cell.decisions.is_empty() {
        "<p class=\"fg-empty\">No decisions cited.</p>".to_string()
    } else {
        let chips: String = cell
            .decisions
            .iter()
            .map(|d| {
                format!(
                    r#"<span class="fg-chip fg-chip--neutral">{}</span>"#,
                    esc(d)
                )
            })
            .collect();
        format!(r#"<div class="bee-panel__chips">{chips}</div>"#)
    };

    let tier_chip = cell
        .tier
        .as_deref()
        .map(|t| {
            format!(
                r#"<span class="fg-chip fg-chip--neutral">tier: {}</span>"#,
                esc(t)
            )
        })
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
    layout_with_drawer(&format!("{} · {}", cell.id, project.name), "", &body, false)
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
/// `waggledance_core::bee::BeeDecisions`). `panes` (feature-titles D2) is this
/// project's own D2-containment-boundary-filtered terminal pane list
/// (`server.rs::project_panes`, the same list `terminal_page` renders) —
/// the Terminal tab links each one straight to its own live page
/// (`terminal_page_for_pane`'s route) rather than re-implementing any of
/// the interactive surface here. `gates` is the same lane-record-or-active-state source `lane_label`
/// reads, for the Activity tab's gate stamps. `docs` (feature-titles,
/// extended by hub-fallbacks) is this feature's own docs reader result,
/// title and description already run through their own fallback chain
/// (`waggledance_core::bee::BeeFeatureDocs`'s own doc comment): present with a
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
    docs: Option<&waggledance_core::bee::BeeFeatureDocs>,
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

    let title = docs
        .and_then(|d| d.title.as_deref())
        .filter(|t| !t.is_empty());
    let head_name_html = match title {
        Some(t) => format!(
            r#"<h2 class="fg-pagehead__title">{title}</h2><p class="bee-detail-slug">{feature}</p>"#,
            title = esc(t),
            feature = esc(feature),
        ),
        None => format!(
            r#"<h2 class="fg-pagehead__title">{feature}</h2>"#,
            feature = esc(feature)
        ),
    };
    let desc_html = match docs
        .and_then(|d| d.description.as_deref())
        .filter(|d| !d.is_empty())
    {
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
    layout_with_drawer(&format!("{} · {}", feature, project.name), "", &body, false)
}

/// (pbi-detail-1) A proposed backlog item's own detail page — the real
/// destination the proposed-PBI board rows now link to instead of falling
/// back to the whole project board (a proposed PBI has no lane or CONTEXT
/// docs of its own, so it reuses [`bee_feature_page`]'s own layout, chip
/// and card classes rather than a stylesheet block of its own). The owning
/// feature only links when the caller has already confirmed the snapshot
/// knows it — `feature_known` false leaves it named, plain text, exactly
/// like a PBI whose `feature` field is blank.
pub fn bee_pbi_page(
    project: &Project,
    pbi: &waggledance_core::bee::BeePbi,
    feature_known: bool,
) -> String {
    let status_chip = format!(
        r#"<span class="fg-chip fg-chip--neutral">{}</span>"#,
        esc(&pbi.status)
    );
    let feature_html = if pbi.feature.is_empty() {
        String::new()
    } else if feature_known {
        format!(
            r#"<p class="bee-cell__meta">feature: <a href="{href}">{feature}</a></p>"#,
            href = bee_hub_feature_href(&project.id, &pbi.feature),
            feature = esc(&pbi.feature),
        )
    } else {
        format!(
            r#"<p class="bee-cell__meta">feature: {}</p>"#,
            esc(&pbi.feature)
        )
    };
    let cos_html = if pbi.cos.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="bee-detail-desc">{}</p>"#, esc(&pbi.cos))
    };
    let body = format!(
        r#"{topbar}
{style}
<main class="fg-page bee-hub-theme">
  <div class="bee-detail-head">
    <div>
      <h2 class="fg-pagehead__title">{title}</h2>
      {feature_html}
      {cos_html}
    </div>
    {status_chip}
  </div>
  <p class="bee-cell__meta"><a href="{board_href}">Back to board</a></p>
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name} · {title}</span>",
            name = esc(&project.name),
            title = esc(&pbi.title),
        )),
        style = bee_hub_style(),
        title = esc(&pbi.title),
        feature_html = feature_html,
        cos_html = cos_html,
        status_chip = status_chip,
        board_href = bee_hub_project_bee_href(&project.id),
    );
    layout_with_drawer(
        &format!("{} · {}", pbi.title, project.name),
        "",
        &body,
        false,
    )
}

/// D2's detail header docs row (feature-titles, extended by hub-fallbacks):
/// links every markdown file [`waggledance_core::bee::BeeFeatureDocs::docs`]
/// lists (already sorted `CONTEXT.md`/`plan.md` first), each through this
/// viewer's own document route (`/p/<id>/docs/history/<feature>/…`, the
/// same project-relative shape [`file_page`]'s own links already use) —
/// never a bare filesystem path. Empty when `docs` is `None` or its own
/// `docs` list is empty: a feature with no markdown file under its docs
/// dir at all has nothing to link, whether or not it has a title or
/// description from another fallback tier.
fn bee_feature_docs_row(
    project_id: &str,
    feature: &str,
    docs: Option<&waggledance_core::bee::BeeFeatureDocs>,
) -> String {
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
    format!(
        r#"<div class="bee-detail-docs">{links}</div>"#,
        links = links
    )
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
    duration: Option<&waggledance_core::bee::BeeCycleSpan>,
    done: usize,
    total: usize,
) -> String {
    let lane_chip = match lane_label.filter(|l| !l.is_empty()) {
        Some(l) => format!(
            r#"<span class="fg-chip fg-chip--neutral">lane: {}</span>"#,
            esc(l)
        ),
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
            let pairs: [(&str, Option<bool>); 4] = [
                ("Context", g.context),
                ("Shape", g.shape),
                ("Execution", g.execution),
                ("Review", g.review),
            ];
            pairs
                .iter()
                .map(|(label, approved)| {
                    let approved = approved.unwrap_or(false);
                    let tone = if approved { "success" } else { "neutral" };
                    let word = if approved {
                        "approved"
                    } else {
                        "not yet approved"
                    };
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
                tone = if tests == "green" {
                    "success"
                } else {
                    "danger"
                },
                tests = esc(tests),
                when = esc(&c
                    .capped_at
                    .as_deref()
                    .map(bee_fmt_trace_time)
                    .unwrap_or_default()),
            )
        }
        None => r#"<p class="fg-empty">No verification recorded yet.</p>"#.to_string(),
    };

    let mut entries: Vec<(Option<time::OffsetDateTime>, String, String)> = Vec::new();
    for d in decisions {
        let parsed = time::OffsetDateTime::parse(&d.date, &rfc3339).ok();
        let when = if parsed.is_some() {
            bee_fmt_trace_time(&d.date)
        } else {
            d.date.clone()
        };
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
        let Some(capped_at) = c.capped_at.as_deref() else {
            continue;
        };
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
    let items: String = cells
        .into_iter()
        .map(|c| bee_todo_item(project_id, c))
        .collect();
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
                format!(
                    r#"<span class="fg-chip fg-chip--accent bee-todo__badge">{}</span>"#,
                    esc(w)
                )
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
        return r#"<p class="fg-empty">No terminal panes running for this project right now.</p>"#
            .to_string();
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
    let breadcrumb = breadcrumb(project, "", &file.rel_path, true, copy_md_button());
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
    var done = function () { document.dispatchEvent(new Event('waggledance:mermaid-done')); };
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
            &section_switch(project, Section::Docs),
            copy_md_button(),
            "",
        ),
        tree = tree,
        breadcrumb = breadcrumb,
        html = page.html,
        source_json = source_json,
        right = right,
    );
    layout_with_drawer(&page.title, head_extra, &body, false)
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
/// function instead of a near-duplicate each. Renders as a sticky bar split
/// left|right. `right` is pre-rendered HTML for the right half (e.g. Docs'
/// copy-as-markdown button, or a code file's type/size) — `""` for pages
/// with nothing to put there. `narrow`, true for Docs only, caps the bar's
/// width to match `.fg-reading` (the markdown column below it); Code's bar
/// stays full-width, matching its own full-width main pane.
fn breadcrumb(project: &Project, base: &str, rel_path: &str, narrow: bool, right: &str) -> String {
    let mut crumbs = format!(
        "<a href=\"/p/{pid}/{base}\">{name}</a>",
        pid = esc(&project.id),
        base = base,
        name = esc(&project.name)
    );
    for seg in rel_path.split('/').filter(|s| !s.is_empty()) {
        crumbs.push_str(&format!(" <span class=\"sep\">/</span> {}", esc(seg)));
    }
    let cls = if narrow {
        "breadcrumb breadcrumb--reading"
    } else {
        "breadcrumb"
    };
    format!(
        "<nav class=\"{cls}\"><div class=\"breadcrumb__left\">{crumbs}</div><div class=\"breadcrumb__right\">{right}</div></nav>"
    )
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

    let (main, meta) = match body {
        CodeBody::Binary { size } => (
            format!(
                "<div class=\"codeview__binary\">Binary file &middot; {size} — cannot be displayed.</div>",
                size = format_size(size)
            ),
            format!(
                "<span class=\"breadcrumb__meta-lang\">Binary</span> \
                 <span class=\"breadcrumb__meta-size\">{size}</span>",
                size = format_size(size)
            ),
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
            let main = format!(
                "{banner}<table class=\"codeview__table\">{rows}</table>",
                banner = banner,
                rows = rows,
            );
            let meta = format!(
                "<span class=\"breadcrumb__meta-lang\">{lang}</span> \
                 <span class=\"breadcrumb__meta-size\">{size}</span>",
                lang = esc(&highlighted.syntax_name),
                size = format_size(size),
            );
            (main, meta)
        }
    };
    let breadcrumb = breadcrumb(project, "_code/", rel_path, false, &meta);

    let body_html = format!(
        r#"{topbar}
<div class="layout">
  <aside id="sidebar" class="sidebar">{tree}</aside>
  <div class="sidebar-backdrop"></div>
  <main class="content content--code">
    {breadcrumb}
    <div class="codeview">{main}</div>
  </main>
</div>"#,
        topbar = topbar_full(
            sidebar_toggle(),
            &section_switch(project, Section::Code),
            "",
            "",
        ),
        tree = tree,
        breadcrumb = breadcrumb,
        main = main,
    );
    layout_with_drawer(active, "", &body_html, false)
}

/// A directory in the Code section: the same listing rendered both in the
/// sidebar (compact nav) and the main pane (with sizes) — the two panes
/// serve different roles, same as a file-explorer's tree-plus-detail split.
pub fn code_dir_page(project: &Project, listing: &DirListing) -> String {
    let tree = code_tree(project, listing, None);
    let breadcrumb = breadcrumb(project, "_code/", &listing.rel_path, false, "");

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
  <main class="content content--code">
    {breadcrumb}
    <div class="codelist">{rows}</div>
  </main>
</div>"#,
        topbar = topbar_full(
            sidebar_toggle(),
            &section_switch(project, Section::Code),
            "",
            "",
        ),
        tree = tree,
        breadcrumb = breadcrumb,
        rows = rows,
    );
    layout_with_drawer(&title, "", &body_html, false)
}

/// Sidebar for the Code section: always exactly one directory's contents
/// (same "one folder, zoomable" model the Docs sidebar uses), server-
/// rendered — no client JS to build the markup, unlike Docs' JSON-payload
/// `file_tree`, because there is no whole-project file list to ship (D1: no
/// index). Structure matches Docs' own sidebar exactly: a `chap-crumbs`
/// path (project root + each ancestor segment) right below the search box,
/// then folders inside the same `chap-folders` disclosure Docs uses
/// (collapsible, chevron, count, folder-icon via `chap-subfolder`), then
/// files. Crumbs/folders render as real `<a>` links here instead of Docs'
/// JS-focus `<button>`s, since Code navigates to a folder/ancestor rather
/// than zooming into it client-side — the crumbs already cover every
/// "go up" case, so there is no separate `..` entry. A small separate
/// script in `app.js` wires the folder bar's click-to-toggle behavior once
/// at load (this HTML needs no JS to render correctly, only to become
/// interactive).
fn code_tree(project: &Project, listing: &DirListing, active_file: Option<&str>) -> String {
    let mut out = String::from(
        "<div class=\"fg-sidebar-search\">\
         <input class=\"fg-input\" placeholder=\"Search…\" autocomplete=\"off\" disabled></div>\
         <nav class=\"chapter\">",
    );

    let mut crumbs = format!(
        "<a class=\"chap-seg\" href=\"/p/{pid}/_code/\">{name}</a>",
        pid = esc(&project.id),
        name = esc(&project.name),
    );
    let mut acc = String::new();
    for seg in listing.rel_path.split('/').filter(|s| !s.is_empty()) {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(seg);
        crumbs.push_str(&format!(
            "<span class=\"chap-sep\">›</span><a class=\"chap-seg\" href=\"/p/{pid}/_code/{acc}\">{seg}</a>",
            pid = esc(&project.id),
            acc = esc(&acc),
            seg = esc(seg),
        ));
    }
    out.push_str(&format!("<div class=\"chap-crumbs\">{crumbs}</div>"));

    let mut dirs = String::new();
    let mut dir_count = 0usize;
    let mut files = String::new();
    for entry in &listing.entries {
        let rel = child_rel(&listing.rel_path, &entry.name);
        if entry.is_dir {
            dir_count += 1;
            dirs.push_str(&format!(
                "<a class=\"chap-subfolder\" href=\"/p/{pid}/_code/{rel}\">{name}</a>",
                pid = esc(&project.id),
                rel = esc(&rel),
                name = esc(&entry.name),
            ));
        } else {
            let cls = if active_file == Some(entry.name.as_str()) {
                "chap-file active"
            } else {
                "chap-file"
            };
            files.push_str(&format!(
                "<a class=\"{cls}\" href=\"/p/{pid}/_code/{rel}\">{name}</a>",
                cls = cls,
                pid = esc(&project.id),
                rel = esc(&rel),
                name = esc(&entry.name),
            ));
        }
    }
    if dir_count > 0 {
        // Auto-open when this folder has no files of its own (else the
        // sidebar would show nothing until the user expands it) — same
        // rule Docs' own `render()` uses (`foldersOpen || here.length === 0`).
        let open = files.is_empty();
        out.push_str(&format!(
            "<div class=\"chap-folders{open_cls}\">\
             <button class=\"chap-folders__bar\" type=\"button\" aria-expanded=\"{aria}\">\
             <span class=\"chap-folders__chev\">›</span>\
             <span class=\"chap-folders__label\">Subfolders</span>\
             <span class=\"chap-folders__count\">{count}</span>\
             </button>\
             <div class=\"chap-folders__list\"><div class=\"chap-folders__inner\">{dirs}</div></div>\
             </div>",
            open_cls = if open { " is-open" } else { "" },
            aria = open,
            count = dir_count,
            dirs = dirs,
        ));
    }
    if !files.is_empty() {
        out.push_str("<div class=\"chap-sec\">Files</div>");
        out.push_str(&files);
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

/// agents-toggle-circle: circular icon button that opens the global agent
/// switch drawer ([`agent_switch_drawer`]) via `#agent-drawer-toggle`. Sits
/// on the bar next to the theme toggle rather than inside the hamburger
/// menu — see [`topbar_full`]'s doc comment for why.
fn agent_toggle() -> &'static str {
    r#"<label class="agent-toggle" for="agent-drawer-toggle" title="Agents" aria-label="Agents"><svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"></polyline><line x1="12" y1="19" x2="20" y2="19"></line></svg></label>"#
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
/// Three areas — left/center/right — so the bar's width lines up with the
/// sidebar/content/rightbar columns below it on desktop (`.topbar__*` in
/// app.css); on narrower viewports they stay a plain flex row. That structure
/// comes from upstream; the brand text, the fourth `nav` slot and the menu
/// below are this fork's and are kept across it — the name shown to a person
/// on these pages is "Waggle Dance" by a locked decision (`bee-cockpit.md`),
/// never the identifier the command line uses.
///
/// The nav slot and the Settings link share one menu. On a wide screen the
/// stylesheet hides its control and lays the panel out inline, so the bar
/// reads exactly as it did before this existed. On a narrow one the control
/// becomes the only visible affordance and the panel drops full-width under
/// the bar.
///
/// agents-toggle-circle: the Agents affordance lives outside that menu —
/// like the theme toggle, opening the agent drawer acts on this page rather
/// than leaving it, so it stays on the bar at every width instead of
/// collapsing into the menu at narrow ones. It is a plain `<label for=
/// "agent-drawer-toggle">` styled as a circular icon button, not a link —
/// clicking it checks [`agent_switch_drawer`]'s own checkbox (that `id`,
/// wherever the drawer's markup itself sits on the page) and opens the
/// drawer, the same label-targets-a-foreign-checkbox trick the drawer's own
/// now-removed floating pill used. `assets/app.js`'s generic `.js-menu`
/// click handler resolves that cross-menu target so opening the drawer this
/// way also closes the hamburger menu if it happened to be open, rather than
/// the drawer's own outside-click check immediately reclosing what the click
/// just opened.
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
  <div class="topbar__left">
    {lead}
    <a href="/" class="home">Waggle Dance</a>
  </div>
  <div class="topbar__center">
    {center}
  </div>
  <div class="topbar__right">
    {actions}
    <div class="topbar-menu js-menu">
      <input type="checkbox" id="topbar-menu-toggle" class="topbar-menu__toggle">
      <label class="topbar-menu__button" for="topbar-menu-toggle" title="Menu"><span class="menu-label">Menu</span><span aria-hidden="true">☰</span></label>
      <div class="topbar-menu__panel">
        {nav}
        <a class="nav-link" href="/settings">Settings</a>
      </div>
    </div>
    {agent_toggle}
    {toggle}
  </div>
</header>"#,
        lead = lead,
        center = center,
        actions = actions,
        nav = nav,
        agent_toggle = agent_toggle(),
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
    layout_with_drawer(&format!("search: {query}"), "", &body, false)
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
        "<div class=\"fg-banner fg-banner--success\"><span class=\"fg-banner__dot\"></span><span class=\"fg-banner__body\">Saved. Server &amp; indexing changes apply after restart (<code>waggledance stop &amp;&amp; waggledance serve</code>).</span></div>"
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
            format!(
                "Bot token: {masked} — leave blank to keep it.",
                masked = esc(&masked)
            ),
            "Leave blank to keep the current token".to_string(),
        ),
    };

    let body = format!(
        r#"{topbar}
<main class="fg-page">
  <h2 class="fg-pagehead__title">Settings <span class="t-caption fg-settings__version">waggledance v{version}</span></h2>
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
      <span class="fg-field__hint">Off by default. Turning this on makes every agent pane on this machine readable and writable through the browser, including ones outside any project waggledance knows about — unrelated repositories, root shells, other people's agents. It has no boundary check of its own.</span>
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
    layout_with_drawer("Settings", "", &body, false)
}

pub fn error_page(status: u16, msg: &str) -> String {
    let body = format!(
        r#"{topbar}
<main class="fg-page"><h2 class="fg-pagehead__title">{status}</h2><p class="fg-empty">{msg}</p></main>"#,
        topbar = topbar(""),
        status = status,
        msg = esc(msg)
    );
    layout_with_drawer(&status.to_string(), "", &body, false)
}

/// stale-index-refresh-2: `error_page`'s sibling for the one 404 that names a
/// known project — `server.rs::project_path`'s own not-found branch, reached
/// when the project itself resolves but the requested path matches neither
/// an indexed markdown file nor an on-disk asset. A file committed or edited
/// while the daemon was down (or between the startup reconcile sweep and
/// now — `stale-index-refresh-1`) stays invisible until something reindexes
/// it, and a reader who followed a link straight to this page has no
/// terminal handy to run `waggledance refresh` in. This renders `error_page`'s
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
    layout_with_drawer(&status.to_string(), "", &body, false)
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
    format!("{:x}", waggledance_core::ansi::revision_of(content))
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

    /// backlog-groom-2 D1 (#26): an empty cross-project board must not
    /// swallow the tab strip — Terminals has nothing to do with whether any
    /// project carries a bee board, so it must stay reachable even when
    /// Kanban itself has nothing of its own to show.
    #[test]
    fn home_page_with_empty_board_still_renders_tab_strip_and_terminals_tab() {
        let body = home_page(
            &[],
            false,
            &[],
            None,
            "",
            HomeTab::Kanban,
            &[],
            None,
            true,
            &[],
        );
        assert!(
            body.contains("fg-tabs"),
            "an empty bee board must still carry the tab strip: {body}"
        );
        assert!(
            body.contains(r#"href="/?tab=terminals""#),
            "the Terminals tab anchor must stay reachable: {body}"
        );
        assert!(
            body.contains("fg-empty"),
            "the Kanban tab must show its own empty state instead of nothing: {body}"
        );
    }

    /// home-terminal-parity-2, extended by agents-drawer-global: every home
    /// tab (not just Terminals — `layout_with_drawer` wraps the whole page,
    /// after the tab has already been chosen) carries the drawer marked
    /// `data-agent-drawer-homepage`, so `assets/app.js` renders its rows
    /// grouped by project with `/?tab=terminals&pane=<id>` links. Every
    /// other page — a project page here, `settings_page` covers the
    /// non-project case in
    /// `every_page_renders_the_agent_switch_drawer_via_the_topbar_menu` —
    /// gets the plain project-page drawer instead, with no such marker.
    #[test]
    fn only_the_home_page_drawer_carries_the_homepage_marker() {
        let home_html = home_page(
            &[],
            false,
            &[],
            None,
            "",
            HomeTab::Kanban,
            &[],
            None,
            true,
            &[],
        );
        assert!(
            home_html.contains("data-agent-drawer-homepage"),
            "the home page's drawer must carry the homepage marker: {home_html}"
        );

        let project = sample_project();
        let project_html = project_home_page(&project, None, false);
        assert!(
            !project_html.contains("data-agent-drawer-homepage"),
            "a project page's drawer must not carry the homepage marker: {project_html}"
        );
    }

    /// backlog-groom-2 D1: a populated board renders exactly as before — the
    /// tab strip plus the cross-project section verbatim, never the empty
    /// state.
    #[test]
    fn home_page_with_populated_board_renders_kanban_section_unchanged() {
        let marker = r#"<div data-feature-hub="cross-project">MARKER-CONTENT</div>"#;
        let body = home_page(
            &[],
            false,
            &[],
            None,
            marker,
            HomeTab::Kanban,
            &[],
            None,
            true,
            &[],
        );
        assert!(body.contains("fg-tabs"), "{body}");
        assert!(
            body.contains("MARKER-CONTENT"),
            "a populated cross-project section must render verbatim: {body}"
        );
        assert!(
            !body.contains("fg-empty"),
            "a populated board must not show the Kanban empty state: {body}"
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

    /// inprogress-priority-order-2: a minimal In Progress placement for the
    /// D7 sort tests below -- only `feature` and `last_activity` vary, so
    /// each test can build exactly the cards its own claim needs without
    /// restating every `BeeHubCardData` field.
    fn in_progress_card(feature: &str, last_activity: Option<&str>) -> BeeHubPlacement {
        BeeHubPlacement::InProgress(BeeHubCardData {
            feature: feature.to_string(),
            done: 0,
            total: 0,
            last_activity: last_activity.map(|s| s.to_string()),
            worktree_label: "Main".to_string(),
            reason: None,
            docs: None,
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: Vec::new(),
        })
    }

    /// inprogress-priority-order-2: one bare `TerminalPaneView` carrying
    /// only the `status` the D7 sort tests below actually vary.
    fn pane_with_status(status: &str) -> TerminalPaneView {
        TerminalPaneView {
            pane_id: format!("w1:{status}"),
            kind: "claude".into(),
            name: "agent".into(),
            status: status.into(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        }
    }

    /// projects-list-tidy: the Projects tab spends no line on a page
    /// heading (the tab strip already says where you are), and a project
    /// with agents running renders above one without — using the same
    /// kind != "shell" filter as the row badges, so a shell-only project
    /// stays in the idle partition.
    #[test]
    fn project_list_drops_heading_and_floats_agent_active_projects_first() {
        let idle = sample_project();
        let mut active = sample_project();
        active.id = "proj-2".into();
        active.name = "Proj Two".into();
        let shell_pane = TerminalPaneView {
            pane_id: "w1:sh".into(),
            kind: "shell".into(),
            name: "shell".into(),
            status: "idle".into(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        };
        let projects = vec![
            (idle, 3, vec![shell_pane]),
            (active, 5, vec![pane_with_status("working")]),
        ];
        let html = project_list_main(&projects, false, &[], None);
        assert!(
            !html.contains("fg-pagehead__title"),
            "the Projects tab must render no page heading: {html}"
        );
        let two = html
            .find("Proj Two")
            .expect("agent-active project must render");
        let one = html.find("Proj One").expect("idle project must render");
        assert!(
            two < one,
            "the agent-active project must render above the shell-only one: {html}"
        );
    }

    /// inprogress-priority-order-2 (D7, must-have): three cards -- one
    /// carrying a `"blocked"` pane with the OLDEST activity, one carrying a
    /// `"working"` pane with no recorded activity, and one carrying no
    /// pane at all with the NEWEST activity -- must render blocked, then
    /// working, then neither: the terminal tier beats activity outright,
    /// never the other way around.
    #[test]
    fn bee_render_hub_section_orders_in_progress_blocked_then_working_then_rest_regardless_of_activity(
    ) {
        let project = sample_project();
        let placements = vec![
            in_progress_card("feat-blocked", Some("2020-01-01T00:00:00.000Z")),
            in_progress_card("feat-working", None),
            in_progress_card("feat-neither", Some("2026-08-15T00:00:00.000Z")),
        ];
        let mut feature_panes: std::collections::HashMap<String, Vec<TerminalPaneView>> =
            std::collections::HashMap::new();
        feature_panes.insert(
            "feat-blocked".to_string(),
            vec![pane_with_status("blocked")],
        );
        feature_panes.insert(
            "feat-working".to_string(),
            vec![pane_with_status("working")],
        );

        let html = bee_render_hub_section(&project, &placements, &[], &feature_panes);

        let blocked_at = html
            .find(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/feat-blocked""#)
            .unwrap_or_else(|| panic!("feat-blocked's own card must render: {html}"));
        let working_at = html
            .find(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/feat-working""#)
            .unwrap_or_else(|| panic!("feat-working's own card must render: {html}"));
        let neither_at = html
            .find(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/feat-neither""#)
            .unwrap_or_else(|| panic!("feat-neither's own card must render: {html}"));
        assert!(
            blocked_at < working_at && working_at < neither_at,
            "order must be blocked, working, neither -- oldest/no activity on the blocked and working cards must not move them down: {html}"
        );
    }

    /// inprogress-priority-order-2 (D7, must-have): `"idle"`, `"done"`,
    /// `"unknown"` and `"shell"` panes earn no tier of their own -- a card
    /// carrying only one of those sorts purely by activity among the
    /// "rest" tier, the same as a card with no pane at all, and every one
    /// of them still sits behind a genuinely `"working"` card even though
    /// that card's own activity is the oldest of the five.
    #[test]
    fn bee_render_hub_section_gives_idle_done_unknown_and_shell_panes_no_tier() {
        let project = sample_project();
        let placements = vec![
            in_progress_card("e-working", Some("2019-01-01T00:00:00.000Z")),
            in_progress_card("d-idle", Some("2026-04-01T00:00:00.000Z")),
            in_progress_card("c-done", Some("2026-03-01T00:00:00.000Z")),
            in_progress_card("b-unknown", Some("2026-02-01T00:00:00.000Z")),
            in_progress_card("a-shell", Some("2026-01-01T00:00:00.000Z")),
        ];
        let mut feature_panes: std::collections::HashMap<String, Vec<TerminalPaneView>> =
            std::collections::HashMap::new();
        feature_panes.insert("e-working".to_string(), vec![pane_with_status("working")]);
        feature_panes.insert("d-idle".to_string(), vec![pane_with_status("idle")]);
        feature_panes.insert("c-done".to_string(), vec![pane_with_status("done")]);
        feature_panes.insert("b-unknown".to_string(), vec![pane_with_status("unknown")]);
        feature_panes.insert("a-shell".to_string(), vec![pane_with_status("shell")]);

        let html = bee_render_hub_section(&project, &placements, &[], &feature_panes);

        let at = |feature: &str| {
            html.find(&format!(
                r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/{feature}""#
            ))
            .unwrap_or_else(|| panic!("{feature}'s own card must render: {html}"))
        };
        let (e, d, c, b, a) = (
            at("e-working"),
            at("d-idle"),
            at("c-done"),
            at("b-unknown"),
            at("a-shell"),
        );
        assert!(
            e < d && d < c && c < b && b < a,
            "the working card must lead despite the oldest activity, and idle/done/unknown/shell must fall back to plain newest-first activity order among themselves: {html}"
        );
    }

    /// inprogress-priority-order-2 (D7, must-have): within one tier, newer
    /// activity wins; a card with no recorded activity renders last; two
    /// cards tied on both tier and activity fall back to feature name A→Z.
    #[test]
    fn bee_render_hub_section_breaks_ties_within_a_tier_by_activity_then_name() {
        let project = sample_project();
        let placements = vec![
            in_progress_card("bravo", Some("2026-01-01T00:00:00.000Z")),
            in_progress_card("charlie", Some("2026-01-01T00:00:00.000Z")),
            in_progress_card("alpha", None),
        ];
        let mut feature_panes: std::collections::HashMap<String, Vec<TerminalPaneView>> =
            std::collections::HashMap::new();
        for feature in ["bravo", "charlie", "alpha"] {
            feature_panes.insert(feature.to_string(), vec![pane_with_status("working")]);
        }

        let html = bee_render_hub_section(&project, &placements, &[], &feature_panes);

        let at = |feature: &str| {
            html.find(&format!(
                r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/{feature}""#
            ))
            .unwrap_or_else(|| panic!("{feature}'s own card must render: {html}"))
        };
        let (bravo, charlie, alpha) = (at("bravo"), at("charlie"), at("alpha"));
        assert!(
            bravo < charlie,
            "tied on tier and activity, name order must break the tie (bravo before charlie): {html}"
        );
        assert!(
            charlie < alpha,
            "a card with no recorded activity must render last, even though its name (alpha) sorts first: {html}"
        );
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
        assert!(
            html.contains(r#"data-preset="Claude">Claude</button>"#),
            "{html}"
        );
        assert!(
            html.contains(r#"data-preset="Codex">Codex</button>"#),
            "{html}"
        );
        assert!(
            !html.contains("data-preset=\"Aider\""),
            "an unconfigured label must never render: {html}"
        );
        // The plain-shell control is unconditional — it needs no preset.
        assert!(
            html.contains(r#"<button type="button" class="term-create__pane">New shell</button>"#)
        );
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

    /// homepage-terminal-full D7: `pane_tab`/`pane_strip`/`pane_bar` now
    /// take a per-pane link the caller builds instead of composing one from
    /// a single `project_id` — the standalone terminal page's own links
    /// must still be exactly what they were before that change.
    #[test]
    fn terminal_page_pane_links_are_unchanged() {
        let project = sample_project();
        let panes = vec![TerminalPaneView {
            pane_id: "w1:p1".into(),
            kind: "claude".into(),
            name: "one".into(),
            status: "working".into(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        }];
        let html = terminal_page(&project, &panes, Some("w1:p1"), &[]);
        assert!(
            html.contains(r#"href="/p/proj-1/_terminal/pane/w1:p1""#),
            "the pane's link must reproduce today's `/p/{{id}}/_terminal/pane/{{pane_id}}` shape: {html}"
        );
    }

    /// homepage-terminal-full D7: the transcript page's own pane links keep
    /// their `_transcript` shape, unchanged by the same signature change.
    #[test]
    fn transcript_page_pane_links_are_unchanged() {
        let project = sample_project();
        let panes = vec![TerminalPaneView {
            pane_id: "w1:p1".into(),
            kind: "claude".into(),
            name: "one".into(),
            status: "working".into(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        }];
        let html = transcript_page(&project, &panes, Some("w1:p1"));
        assert!(
            html.contains(r#"href="/p/proj-1/_transcript/pane/w1:p1""#),
            "the pane's link must reproduce today's `/p/{{id}}/_transcript/pane/{{pane_id}}` shape: {html}"
        );
    }

    /// homepage-terminal-full D5 (risk map): `pane_cards`' two current
    /// callers both pass `None` for `base`, so the standalone page's
    /// controls keep resolving through the page-root `data-project-id`
    /// fallback and must never carry `data-term-base` themselves — only a
    /// future project-less caller (a homepage tab) would supply one.
    #[test]
    fn terminal_page_controls_carry_no_data_term_base() {
        let project = sample_project();
        let panes = vec![TerminalPaneView {
            pane_id: "w1:p1".into(),
            kind: "claude".into(),
            name: "one".into(),
            status: "working".into(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        }];
        let html = terminal_page(&project, &panes, Some("w1:p1"), &[]);
        assert!(
            !html.contains("data-term-base"),
            "the standalone page's controls must not carry data-term-base: {html}"
        );
    }

    /// agent-switch-drawer-2, moved by agents-drawer-global to the shared
    /// `layout_with_drawer` seam: the terminal page — both a project's own
    /// pane list and its `/pane/:pane_id` render, which share this one
    /// function — carries the agent switcher drawer, and the checkbox that
    /// opens it is the hook `assets/app.js`'s poller and the generic
    /// `.js-menu` handler both key off. See
    /// `every_page_renders_the_agent_switch_drawer_via_the_topbar_menu`
    /// below for the same coverage on a page that has no terminal
    /// involvement at all.
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

    /// agents-drawer-global: `layout_with_drawer` renders the drawer on
    /// every page, not just the terminal ones — the Settings page has no
    /// terminal involvement at all, so it is the clean case that proves the
    /// seam, not just the old terminal-only splice points. agents-toggle-
    /// circle: it must also carry the on-bar circular agent toggle, the
    /// drawer's only remaining affordance now that the floating right-edge
    /// pill is retired and the toggle has moved out of the hamburger menu.
    #[test]
    fn every_page_renders_the_agent_switch_drawer_via_the_topbar_menu() {
        let html = settings_page(
            &Config::default(),
            false,
            false,
            NotifyCredentialView::NotConfigured,
        );
        assert!(
            html.contains(r#"id="agent-drawer-toggle""#) && html.contains("data-agent-drawer-list"),
            "a non-terminal page must still render the agent switch drawer: {html}"
        );
        assert!(
            html.contains(r#"<label class="agent-toggle" for="agent-drawer-toggle""#),
            "the topbar must carry an on-bar Agents toggle that opens the drawer: {html}"
        );
        assert!(
            !html.contains(r#"<label class="nav-link" for="agent-drawer-toggle">Agents</label>"#),
            "the Agents entry must no longer sit inside the hamburger menu: {html}"
        );
        assert!(
            !html.contains("agent-drawer__tab"),
            "the floating agent-drawer pill is retired; the on-bar toggle is the only affordance now: {html}"
        );
    }

    /// Everything in the bar that navigates away from this page — the section
    /// tabs and the Settings link — lives inside one menu. The theme toggle
    /// and (agents-toggle-circle) the Agents toggle are not in it: both act
    /// on this page rather than leaving it, and stay reachable in one press
    /// at every width.
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
            menu.contains(
                r#"<input type="checkbox" id="topbar-menu-toggle" class="topbar-menu__toggle">"#
            ) && menu.contains(r#"<label class="topbar-menu__button" for="topbar-menu-toggle""#),
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
                cwd: String::new(),
                workspace: "w1".into(),
                tab: "t1".into(),
            },
            TerminalPaneView {
                pane_id: "w1:p2".into(),
                kind: "shell".into(),
                name: String::new(),
                status: "shell".into(),
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
            after.contains("class=\"term-create__pane\"")
                && after.contains(r#"data-preset="Claude""#),
            "the creation controls must move into the menu: {html}"
        );
        assert!(
            html.contains(
                r#"<input type="checkbox" id="pane-menu-toggle" class="pane-menu__toggle">"#
            ),
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

    /// A touch browser zooms the page when it focuses a control under 16px,
    /// and the zoom sticks — the layout is left magnified after one tap into
    /// a field. Both stylesheets have to carry the coarse-pointer override,
    /// because they own different halves of the app's fields: `app.css` the
    /// settings forms, `PROJECT_TAB_STYLE` the terminal reply box (whose
    /// declaration is inlined after the stylesheet link and would otherwise
    /// win the cascade). Desktop sizes stay as designed, so the override is
    /// asserted to live inside the media query rather than beside it.
    #[test]
    fn a_touch_pointer_gets_fields_big_enough_not_to_trigger_zoom() {
        for (name, css, selector) in [
            ("app.css", include_str!("../assets/app.css"), ".fg-input"),
            (
                "the terminal style block",
                PROJECT_TAB_STYLE,
                ".term-reply__text",
            ),
        ] {
            let query = css
                .find("@media (pointer: coarse) {")
                .unwrap_or_else(|| panic!("{name} carries no coarse-pointer block"));
            let block_end = css[query..]
                .find("\n}")
                .unwrap_or_else(|| panic!("{name}'s coarse-pointer block never closes"));
            let block = &css[query..query + block_end];
            assert!(
                block.contains(selector) && block.contains("font-size: 16px"),
                "{name} must raise {selector} to 16px on a touch pointer: {block}"
            );
            assert!(
                !css[..query].contains("font-size: 16px"),
                "{name} must keep the 16px size inside the media query, not at every width"
            );
        }
    }

    /// agent-terminal-13, must-have: "with no presets configured, the
    /// creation control offers nothing" — zero preset buttons render, while
    /// the plain-shell button (which needs no preset) still does.
    #[test]
    fn terminal_page_renders_no_preset_controls_when_none_configured() {
        let project = sample_project();
        let html = terminal_page(&project, &[], None, &[]);
        // Checked as rendered HTML attribute shapes, not bare substrings —
        // `.term-create__agent`/`data-preset` never render at all with no
        // presets configured, and views.rs ships no JS whose own literal
        // text could false-positive a substring match (backlog-groom-2-4:
        // the create control's click wiring now lives in `assets/app.js`).
        assert!(!html.contains("class=\"term-create__agent\""), "{html}");
        assert!(!html.contains("data-preset=\""), "{html}");
        assert!(html.contains("class=\"term-create__pane\""), "{html}");
    }

    /// A preset label carrying HTML metacharacters must render escaped, the
    /// same discipline every other operator/user-controlled string in this
    /// module follows.
    #[test]
    fn terminal_create_controls_escapes_preset_labels() {
        let html =
            terminal_create_controls("proj-1", &["<script>alert(1)</script>".to_string()], true);
        assert!(!html.contains("<script>alert(1)</script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    /// home-terminal-header: `plain_shell` is the whole difference between
    /// the two callers. `true` (the project terminal page) keeps the
    /// always-offered plain-shell button; `false` (the homepage Terminals
    /// tab) drops it and leaves the configured agent presets standing. With
    /// `false` and nothing configured, no `.term-create` box renders at all
    /// rather than an empty one — the same silence a pane outside every
    /// registered project already gets.
    #[test]
    fn terminal_create_controls_offer_the_plain_shell_button_only_when_asked() {
        let presets = vec!["Claude".to_string()];

        let with_shell = terminal_create_controls("proj-1", &presets, true);
        assert!(
            with_shell.contains("class=\"term-create__pane\""),
            "{with_shell}"
        );
        assert!(
            with_shell.contains(r#"data-preset="Claude""#),
            "{with_shell}"
        );

        let without_shell = terminal_create_controls("proj-1", &presets, false);
        assert!(
            !without_shell.contains("class=\"term-create__pane\"")
                && !without_shell.contains(">New shell<"),
            "the plain-shell button must not render when it was not asked for: {without_shell}"
        );
        assert!(
            without_shell.contains(r#"data-preset="Claude""#)
                && without_shell.contains(r#"class="term-create" data-project-id="proj-1""#),
            "the agent presets must still render, still targeting the project: {without_shell}"
        );

        let nothing = terminal_create_controls("proj-1", &[], false);
        assert!(
            nothing.is_empty(),
            "no button to offer must render no box at all: {nothing}"
        );
        assert!(
            terminal_create_controls("proj-1", &[], true).contains("class=\"term-create__pane\""),
            "a plain-shell caller with no presets still gets its button"
        );
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
            html.contains(
                ".term-keys button { padding: var(--space-1) var(--space-2); min-height: 44px;"
            ),
            "the whole key row must stand 44px tall: {html}"
        );
        assert!(
            html.contains(
                ".term-keys--move button { min-width: 44px; font-size: var(--type-body-size); }"
            ),
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
            !html.contains(".term-scroll button { min-width: 44px")
                && !html.contains(".term-scroll { min-width: 44px"),
            "the scroll pair must carry no such rule: {html}"
        );
        assert!(
            !html.contains(".term-reply__send { min-width: 44px")
                && !html.contains(".term-reply__stage { min-width: 44px"),
            "the reply buttons must carry no such rule: {html}"
        );
    }

    /// D1 (terminal-approve-button): Approve leads the row — Approve,
    /// Stage, Send — so a one-tap send never sits shoulder to shoulder with
    /// Send, and Stage/Send keep their existing relative order.
    #[test]
    fn pane_controls_renders_approve_before_stage_and_send() {
        let html = pane_controls("pane-1", "Agent One", false, None);
        assert!(
            html.contains(r#"<button type="button" class="term-reply__approve">Approve</button>"#),
            "the Approve button must render: {html}"
        );
        let approve_at = html
            .find("term-reply__approve")
            .expect("Approve button must render");
        let stage_at = html
            .find("term-reply__stage")
            .expect("Stage button must render");
        let send_at = html
            .find("term-reply__send")
            .expect("Send button must render");
        assert!(
            approve_at < stage_at && stage_at < send_at,
            "the row must read Approve, Stage, Send: {html}"
        );
    }

    /// D1: the new button sizes exactly like its siblings — no new colours,
    /// no new tokens — so it joins both the shared reply-button rule and the
    /// mobile sizing tweak alongside term-reply__send/term-reply__stage.
    #[test]
    fn term_reply_approve_shares_the_stage_send_css_rule() {
        let project = sample_project();
        let html = terminal_page(&project, &[], None, &[]);
        assert!(
            html.contains(".term-reply__send, .term-reply__stage, .term-reply__approve {"),
            "the shared reply-button rule must include term-reply__approve: {html}"
        );
        assert!(
            html.contains(
                ".term-keys button, .term-reply__send, .term-reply__stage, .term-reply__approve {"
            ),
            "the mobile sizing tweak must include term-reply__approve: {html}"
        );
    }

    /// D2/D4: `assets/app.js` wires the Approve button the same way it
    /// wires Stage — posting through the existing `postJson`/`inputUrl`
    /// helpers, never a new fetch — so the project page and the homepage
    /// Terminals tab both get a working button for free.
    #[test]
    fn app_js_wires_the_approve_button() {
        let script = include_str!("../assets/app.js");
        assert!(
            script.contains(r#"var approveBtn = form.querySelector(".term-reply__approve");"#),
            "app.js must read the Approve button: {script}"
        );
        assert!(
            script
                .contains(r#"postJson(inputUrl(paneId, base), { text: "Approve", submit: true })"#),
            "app.js's Approve handler must post the exact text Approve with submit true: {script}"
        );
    }

    /// D4: the Unassigned page has no `data-project-id` to key off, so it is
    /// wired by its own scoped IIFE in `assets/app.js` (backlog-groom-2-4,
    /// keyed off `data-unassigned-base`) rather than the shared
    /// `data-project-id`/`data-term-base` wiring above — the Approve handler
    /// has to be duplicated in that IIFE too, or the button on that page
    /// would render inert. Sliced from `main[data-unassigned-base]`'s own
    /// offset in the file to pin it to that IIFE specifically, not the
    /// shared one above (whose `postJson(inputUrl(paneId, base), ...)` call
    /// takes a second `base` argument this one never does).
    #[test]
    fn unassigned_terminal_wiring_in_app_js_wires_the_approve_button() {
        let script = include_str!("../assets/app.js");
        let start = script.find("data-unassigned-base").unwrap_or_else(|| {
            panic!("app.js must read the Unassigned page's own data-unassigned-base: {script}")
        });
        let section = &script[start..];
        assert!(
            section.contains(r#"var approveBtn = form.querySelector(".term-reply__approve");"#),
            "the Unassigned page's own wiring in app.js must read the Approve button: {section}"
        );
        assert!(
            section.contains(r#"postJson(inputUrl(paneId), { text: "Approve", submit: true })"#),
            "the Unassigned page's own wiring must post the exact text Approve with submit true to that page's own input URL: {section}"
        );
    }

    /// unassigned-poller-guard D1: `assets/app.js`'s shared screen poller
    /// and its three posters (input/keys/attach) skip any element that
    /// resolves to NEITHER a valid `data-term-base` NOR the page's own
    /// `data-project-id` — otherwise both build a `/p/null/...` URL and
    /// poll/post there forever. That guard lives entirely in JS with no
    /// repo harness to run it directly (the same JS-only shape
    /// `home-terminal-header-2` recorded); what this pins instead is the
    /// markup contract the guard's premise depends on: the Unassigned page
    /// truly offers neither signal, while the project terminal page and the
    /// homepage Terminals tab each offer one. Manual browser check
    /// (unautomatable, recorded here per that same precedent): open
    /// `/_terminal/unassigned` with a live pane, confirm the network panel
    /// shows no `/p/null/...` request over several poll ticks, and that
    /// pressing Send posts to `/_terminal/unassigned/<pane>/input` exactly
    /// once (not twice).
    #[test]
    fn unassigned_page_offers_neither_project_id_nor_term_base_while_other_pages_offer_one() {
        let pane = pane_with_status("working");

        // The Unassigned page: no `data-project-id` anywhere (its `<main>`
        // carries none), and its `.term-screen` panes (via `pane_cards`,
        // `attach: false`) carry no `data-term-base` either — the shared
        // poller/posters must find nothing to resolve a target from here.
        // `data-unassigned-base` is a distinct attribute the shared
        // `hasTarget` guard never reads (backlog-groom-2-4): it feeds only
        // this page's own separate wiring in `assets/app.js`.
        let unassigned_html = unassigned_terminal_page(std::slice::from_ref(&pane));
        assert!(
            unassigned_html
                .contains(r#"<main class="fg-page" data-unassigned-base="/_terminal/unassigned">"#),
            "the Unassigned page's <main> must render with no data-project-id, only its own scoped base: {unassigned_html}"
        );
        assert!(
            !unassigned_html.contains("data-project-id"),
            "the Unassigned page must carry no data-project-id anywhere: {unassigned_html}"
        );
        assert!(
            !unassigned_html.contains("data-term-base"),
            "the Unassigned page's panes must carry no data-term-base: {unassigned_html}"
        );

        // The project terminal page: `<main>` DOES carry `data-project-id`
        // — one of the two signals the guard accepts.
        let project = sample_project();
        let project_html = terminal_page(&project, std::slice::from_ref(&pane), None, &[]);
        assert!(
            project_html.contains(&format!(
                r#"<main class="fg-page fg-page--tight" data-project-id="{}">"#,
                project.id
            )),
            "the project terminal page's <main> must carry data-project-id: {project_html}"
        );

        // The homepage Terminals tab: its own `<main>` carries no
        // `data-project-id` (D3 — its panes can belong to any project or
        // none), but the selected pane's `.term-screen` DOES carry
        // `data-term-base` — the other signal the guard accepts.
        let menu_pane = TerminalsMenuPane {
            view: pane.clone(),
            base: format!("/p/{}/_terminal/{}", project.id, pane.pane_id),
            project_label: "Proj One".into(),
            is_project_pane: true,
            project_id: Some(project.id.clone()),
        };
        let tab_html = terminals_tab(std::slice::from_ref(&menu_pane), None, true, &[]);
        assert!(
            tab_html.contains(r#"<main class="fg-page fg-page--tight">"#),
            "the homepage Terminals tab's own <main> must render with no data-project-id: {tab_html}"
        );
        assert!(
            tab_html.contains(&format!(r#"data-term-base="{}""#, menu_pane.base)),
            "the homepage Terminals tab's pane must carry data-term-base: {tab_html}"
        );
    }

    /// homepage-terminal-full: one `TerminalsMenuPane` fixture per test, so
    /// each test states only the one or two fields its own claim varies —
    /// `pane_id` names the pane and doubles as its `tab` label so distinct
    /// panes are trivially distinguishable in a rendered switcher;
    /// `project_id: None` is `unassigned_panes`' own shape (D8: no create
    /// controls, is_project_pane: false — no `/attach` route either).
    fn menu_pane(
        pane_id: &str,
        project_id: Option<&str>,
        project_label: &str,
    ) -> TerminalsMenuPane {
        TerminalsMenuPane {
            view: TerminalPaneView {
                pane_id: pane_id.into(),
                kind: "claude".into(),
                name: "agent".into(),
                status: "working".into(),
                cwd: String::new(),
                workspace: "w1".into(),
                tab: pane_id.into(),
            },
            base: format!(
                "/p/{}/_terminal/{}",
                project_id.unwrap_or("unassigned"),
                pane_id
            ),
            project_label: project_label.into(),
            is_project_pane: project_id.is_some(),
            project_id: project_id.map(str::to_string),
        }
    }

    /// homepage-terminal-full D1: the tab renders the standalone page's own
    /// controls — the `pane_bar` switcher (both its wide strip and its
    /// narrow-screen collapsed menu, since two panes are enough to prove
    /// both markers render), and the selected pane's card with its Older/
    /// Newer/Live history controls, movement/action keys and reply form.
    /// agents-drawer-global: the agent switch drawer used to be this tab's
    /// own splice, asserted here — [`home_page`] (this tab's only caller)
    /// now renders it once via `layout_with_drawer`, wrapping every home
    /// tab alike, so that coverage moved to
    /// `only_the_home_page_drawer_carries_the_homepage_marker`. A drawer
    /// here too would duplicate `#agent-drawer-toggle`.
    #[test]
    fn terminals_tab_renders_the_switcher_pane_card_and_history_controls() {
        let panes = vec![
            menu_pane("w1:p1", Some("proj-1"), "Proj One"),
            menu_pane("w1:p2", Some("proj-1"), "Proj One"),
        ];
        let html = terminals_tab(&panes, Some("w1:p1"), true, &[]);

        // The switcher: wide strip and narrow collapsed menu, both present
        // (CSS alone decides which one a given width actually shows).
        assert!(
            html.contains(r#"class="pane-bar js-menu""#) && html.contains(r#"class="pane-strip""#),
            "the tab must render the pane_bar switcher's strip: {html}"
        );
        assert!(
            html.contains(r#"class="pane-menu__toggle""#)
                && html.contains(r#"class="pane-menu__panel""#),
            "the tab must render the pane_bar switcher's narrow-screen menu: {html}"
        );

        // The selected pane's own card: screen viewport, history controls,
        // movement/action keys, reply form.
        assert!(
            html.contains(r#"data-pane-id="w1:p1""#) && html.contains("term-screen"),
            "the selected pane's screen must render: {html}"
        );
        assert!(
            html.contains(r#"data-scroll="older""#)
                && html.contains(r#"data-scroll="newer""#)
                && html.contains(r#"data-scroll="live""#),
            "the Older/Newer/Live history controls must render: {html}"
        );
        assert!(
            html.contains("term-keys--move") && html.contains(r#"class="term-reply""#),
            "the movement keys and reply form must render: {html}"
        );

        // agents-drawer-global: no drawer here — `home_page` (this tab's
        // only caller) renders it once, wrapping the whole page. A second
        // one here would duplicate `#agent-drawer-toggle`.
        assert!(
            !html.contains(r#"id="agent-drawer-toggle""#),
            "the tab itself must not splice its own drawer: {html}"
        );
    }

    /// homepage-terminal-full D5: the tab has no single project id, so every
    /// control on the selected pane's card — the screen viewport and the
    /// reply/keys group [`pane_controls`] renders — carries that pane's own
    /// `data-term-base` rather than falling back to a page-root
    /// `data-project-id` this page does not have.
    #[test]
    fn terminals_tab_controls_carry_the_selected_panes_own_base() {
        let pane = menu_pane("w1:p1", Some("proj-1"), "Proj One");
        let base = pane.base.clone();
        let html = terminals_tab(std::slice::from_ref(&pane), Some("w1:p1"), true, &[]);
        let base_attr = format!(r#"data-term-base="{base}""#);
        // The screen viewport (poller target) and every `pane_controls`
        // group (move keys, action keys, reply form) each carry their own
        // copy — four occurrences on a single-pane render.
        assert!(
            html.matches(base_attr.as_str()).count() >= 4,
            "every pane control must carry data-term-base=\"{base}\": {html}"
        );
        assert!(
            !html.contains("data-project-id"),
            "the tab must never fall back to a page-root data-project-id: {html}"
        );
    }

    /// homepage-terminal-full D8: creation controls belong to the selected
    /// pane's own project — present, and living inside the pane_bar
    /// switcher (D1), when it has one; absent entirely when it does not.
    #[test]
    fn terminals_tab_creation_controls_follow_the_selected_panes_project() {
        let project_pane = menu_pane("w1:p1", Some("proj-1"), "Proj One");
        let with_project = terminals_tab(
            std::slice::from_ref(&project_pane),
            Some("w1:p1"),
            true,
            &["Claude".to_string()],
        );
        assert!(
            with_project.contains(r#"data-preset="Claude">Claude</button>"#),
            "a selected pane with a project must render its preset buttons: {with_project}"
        );
        let bar_start = with_project
            .find(r#"class="pane-bar js-menu""#)
            .expect("no pane bar");
        let bar_end = with_project[bar_start..]
            .find("</main>")
            .map(|i| bar_start + i)
            .expect("no </main> after the pane bar");
        assert!(
            with_project[bar_start..bar_end].contains(r#"data-preset="Claude""#),
            "the creation controls must live inside the pane_bar switcher: {with_project}"
        );

        let unassigned_pane = menu_pane("w1:p2", None, "Unassigned");
        let without_project = terminals_tab(
            std::slice::from_ref(&unassigned_pane),
            Some("w1:p2"),
            true,
            &["Claude".to_string()],
        );
        assert!(
            !without_project.contains(r#"class="term-create""#),
            "a selected pane with no project must render no creation controls: {without_project}"
        );
    }

    /// homepage-terminal-full D2: the one-line `<select>` this tab used to
    /// render, its change listener and its CSS are gone without a trace —
    /// `pane_bar` is the only switcher left.
    #[test]
    fn terminals_tab_renders_no_pane_select_markup() {
        let panes = vec![menu_pane("w1:p1", Some("proj-1"), "Proj One")];
        let html = terminals_tab(&panes, Some("w1:p1"), true, &[]);
        assert!(
            !html.contains("terminals-pane-select") && !html.contains("<select"),
            "no trace of the old select must survive: {html}"
        );
    }

    /// homepage-terminal-full D3: the tab renders neither the project
    /// topbar nor the Overview/Terminal/Transcript project nav — the home
    /// page already has its own topbar and its own Kanban/Projects/
    /// Terminals strip above this tab's own render.
    #[test]
    fn terminals_tab_renders_no_project_topbar_or_nav() {
        let panes = vec![menu_pane("w1:p1", Some("proj-1"), "Proj One")];
        let html = terminals_tab(&panes, Some("w1:p1"), true, &[]);
        assert!(
            !html.contains("<header class=\"topbar\">") && !html.contains(r#"class="proj-tabs""#),
            "the tab must render no project topbar and no project nav: {html}"
        );
    }

    /// homepage-terminal-full D4: both empty states — nothing running, and
    /// the agent host unreachable — read exactly as they did before this
    /// cell, unaffected by the new switcher.
    #[test]
    fn terminals_tab_empty_states_are_unchanged() {
        let nothing_running = terminals_tab(&[], None, true, &[]);
        assert!(
            nothing_running.contains("No agents are running right now."),
            "an empty list with herdr reachable must say nothing is running: {nothing_running}"
        );
        let herdr_down = terminals_tab(&[], None, false, &[]);
        assert!(
            herdr_down.contains("herdr is not running."),
            "an empty list with herdr unreachable must say so: {herdr_down}"
        );
    }

    /// homepage-terminal-full D4: a named-but-gone pane still renders its
    /// own "this terminal is gone" line, with the full pane list still
    /// offered through the switcher — nothing about a vanished pane hides
    /// the way to every other one.
    #[test]
    fn terminals_tab_vanished_pane_keeps_the_full_switcher() {
        let panes = vec![
            menu_pane("w1:p1", Some("proj-1"), "Proj One"),
            menu_pane("w1:p2", Some("proj-1"), "Proj One"),
        ];
        let html = terminals_tab(&panes, Some("does-not-exist"), true, &[]);
        assert!(
            html.contains("This terminal is gone."),
            "a vanished pane must render its own gone line: {html}"
        );
        assert!(
            !html.contains(r#"data-pane-id="w1:p1""#),
            "the gone pane's own screen must not render: {html}"
        );
        assert!(
            html.contains("w1:p1") && html.contains("w1:p2"),
            "the switcher must still list every other pane: {html}"
        );
    }

    /// homepage-terminal-full D4/D7: an absent `?pane` selects the first
    /// pane in `panes`' own order, and the switcher's own links stay inside
    /// the tab rather than jumping to a project's own terminal page.
    #[test]
    fn terminals_tab_absent_pane_selects_the_first_and_switcher_links_stay_in_tab() {
        let panes = vec![
            menu_pane("w1:p1", Some("proj-1"), "Proj One"),
            menu_pane("w1:p2", Some("proj-1"), "Proj One"),
        ];
        let html = terminals_tab(&panes, None, true, &[]);
        assert!(
            html.contains(r#"data-pane-id="w1:p1""#),
            "an absent ?pane must select the first pane: {html}"
        );
        assert!(
            html.contains(r#"href="/?tab=terminals&amp;pane=w1:p1""#)
                && html.contains(r#"href="/?tab=terminals&amp;pane=w1:p2""#),
            "the switcher's links must stay on the tab's own address (D7): {html}"
        );
    }

    /// terminals-tab-project-scope-1: the switcher used to list every pane
    /// across every project; it now lists only the ones sharing the
    /// *effective* pane's own project, matched on `project_id` — a
    /// cross-project pane (`w1:p3`, `proj-2`) neither the selected pane
    /// (`w1:p1`, `proj-1`) nor its project-mate (`w1:p2`) belongs with must
    /// not appear anywhere in the render.
    #[test]
    fn terminals_tab_switcher_omits_panes_from_another_project() {
        let panes = vec![
            menu_pane("w1:p1", Some("proj-1"), "Proj One"),
            menu_pane("w1:p2", Some("proj-1"), "Proj One"),
            menu_pane("w1:p3", Some("proj-2"), "Proj Two"),
        ];
        let html = terminals_tab(&panes, Some("w1:p1"), true, &[]);
        assert!(
            html.contains("w1:p2"),
            "the switcher must still list the selected pane's own project-mate: {html}"
        );
        assert!(
            !html.contains("w1:p3"),
            "the switcher must not list a pane from another project: {html}"
        );
    }

    /// terminals-tab-project-scope-1: a selected pane belonging to no
    /// project at all (`project_id: None`, `unassigned_panes`' own shape)
    /// lists only the *other* project-less panes — a registered project's
    /// pane must not leak into that switcher either.
    #[test]
    fn terminals_tab_no_project_selection_lists_only_project_less_panes() {
        let panes = vec![
            menu_pane("w1:u1", None, "Unassigned"),
            menu_pane("w1:u2", None, "Unassigned"),
            menu_pane("w1:p1", Some("proj-1"), "Proj One"),
        ];
        let html = terminals_tab(&panes, Some("w1:u1"), true, &[]);
        assert!(
            html.contains("w1:u2"),
            "the switcher must still list the other project-less pane: {html}"
        );
        assert!(
            !html.contains("w1:p1"),
            "the switcher must not list a pane that belongs to a registered project: {html}"
        );
    }

    /// terminals-tab-project-scope-1: an absent `?pane` still takes D4's own
    /// first entry of the *full* inventory, even when that first entry's
    /// project differs from another pane later in the same list — selection
    /// resolution is never scoped, only the switcher it feeds is.
    #[test]
    fn terminals_tab_absent_pane_fallback_ignores_project_scope() {
        let panes = vec![
            menu_pane("w1:p2", Some("proj-2"), "Proj Two"),
            menu_pane("w1:p1", Some("proj-1"), "Proj One"),
        ];
        let html = terminals_tab(&panes, None, true, &[]);
        assert!(
            html.contains(r#"data-pane-id="w1:p2""#),
            "an absent ?pane must still select the first pane of the full inventory: {html}"
        );
        assert!(
            !html.contains("w1:p1"),
            "the switcher must scope to the selected (first) pane's own project, proj-2: {html}"
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
    /// bee's own terminal `"compounding-complete"` — used to keep rendering
    /// under Waiting on you, because `state.json` still names it active and
    /// `.bee/HANDOFF.json` still reads as a pause, and that pull was
    /// evaluated before the Finished branch. The card then showed
    /// "No cells recorded.", since the pull counted live cells only and a
    /// closed feature has none. Finished now wins whenever no live cell is
    /// left (kanban-columns D11: In Progress is tested first, but
    /// `finished_and_idle` excludes it) — the card lands under Finished —
    /// as a dense row, carrying no done/total count of its own at all
    /// (hub-finished-compact), and no `Waiting on you` line either, since
    /// that line only ever decorates an In Progress card.
    #[test]
    fn hub_sends_a_closed_feature_to_finished_even_while_a_pause_handoff_names_it() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-closed-{}",
            std::process::id()
        ));
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

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"data-hub-group="in-progress" data-hub-count="0""#),
            "a closed feature owes no decision, so In Progress must be empty: {html}"
        );
        assert!(
            !html.contains(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/closed-feat""#),
            "the closed feature must render no In Progress card, and so no Waiting on you line: {html}"
        );
        assert!(
            html.contains(r#"data-hub-group="finished" href="/p/proj-1/_bee/feature/closed-feat""#),
            "the closed feature belongs under Finished: {html}"
        );
        assert!(
            !html.contains("cells done") && !html.contains("No cells recorded.") && !html.contains("Waiting on you"),
            "hub-finished-compact: a Finished row carries no progress count and no Waiting on you line of its own: {html}"
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
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-session-bound-{}",
            std::process::id()
        ));
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
            &format!(
                r#"{{"id": "live", "last_heartbeat": "{now}", "lane": "session-bound-feat"}}"#
            ),
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/session-bound-feat""#),
            "a zero-cell feature with a live session bound to its lane must render under In Progress: {html}"
        );
        assert!(
            !html.contains("Waiting on you"),
            "with every gate approved, the session-bound feature's card must carry no Waiting on you line: {html}"
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
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-wt-bound-{}",
            std::process::id()
        ));
        let sibling = std::env::temp_dir().join(format!(
            "waggledance-views-hub-wt-bound-sibling-{}",
            std::process::id()
        ));
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
        write(
            &sibling,
            ".bee/state.json",
            r#"{"phase":"swarming","feature":"wt-bound-feat","mode":"standard"}"#,
        );
        let grant_id = sibling.file_name().unwrap().to_string_lossy().to_string();
        write(
            &root,
            ".bee/runtime/worktree-grants.json",
            &format!(r#"{{"{grant_id}": true}}"#),
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/wt-bound-feat""#),
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
        let gates =
            |c: Option<bool>, s: Option<bool>, e: Option<bool>, r: Option<bool>| BeeApprovedGates {
                context: c,
                shape: s,
                execution: e,
                review: r,
            };

        // The reported shape: explore unstamped, but shape and execution
        // both approved — nothing is owed before review.
        let g = gates(Some(false), Some(true), Some(true), Some(false));
        assert_eq!(
            bee_gate_current_stop(Some(&g)),
            Some(("review", "Independent review"))
        );

        // A feature that really did stop at its interview still reports it.
        let g = gates(Some(false), Some(false), Some(false), Some(false));
        assert_eq!(
            bee_gate_current_stop(Some(&g)),
            Some(("context", "Explore"))
        );

        // Context approved, shape not: the shape gate is the stop.
        let g = gates(Some(true), Some(false), Some(false), Some(false));
        assert_eq!(bee_gate_current_stop(Some(&g)), Some(("shape", "Shape")));

        // Execution approved past an unstamped shape: review is next.
        let g = gates(Some(true), Some(false), Some(true), Some(false));
        assert_eq!(
            bee_gate_current_stop(Some(&g)),
            Some(("review", "Independent review"))
        );

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
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-working-now-{}",
            std::process::id()
        ));
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

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/working-feat""#),
            "a feature whose session beat a minute ago must render under In Progress even with a gate unapproved: {html}"
        );
        assert!(
            !html.contains("Waiting on you"),
            "an agent actively working the feature must never see a Waiting on you line on its card: {html}"
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
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-working-default-{}",
            std::process::id()
        ));
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

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/active-feat""#),
            "a lane-less session beating a minute ago works the ACTIVE feature: {html}"
        );
        assert!(
            html.contains(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/other-feat""#),
            "the other gate-stopped feature with a live cell must also render under In Progress: {html}"
        );
        let active_pos = html
            .find(r#"href="/p/proj-1/_bee/feature/active-feat""#)
            .expect("active-feat's own card must render");
        let other_pos = html
            .find(r#"href="/p/proj-1/_bee/feature/other-feat""#)
            .expect("other-feat's own card must render");
        let active_card = &html[active_pos..other_pos.max(active_pos)];
        assert!(
            !active_card.contains("Waiting on you"),
            "the active feature must carry no Waiting on you line while its own agent is mid-interview: {active_card}"
        );
        assert!(
            html[other_pos..].contains("Waiting on you"),
            "a lane-less session must not suppress some OTHER feature's own Waiting on you line: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (regression, working-now-default-lane-1) The same lane-less session,
    /// twenty minutes cold: past `WORKING_MINUTES`, so the active feature's
    /// unapproved gate is owed the owner a decision again.
    #[test]
    fn hub_gives_the_active_feature_its_waiting_on_you_line_once_its_lane_less_session_goes_cold() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-working-default-cold-{}",
            std::process::id()
        ));
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

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/active-feat""#),
            "the feature still has live work (it is the active feature), so it stays under In Progress: {html}"
        );
        assert!(
            html.contains("Waiting on you — Explore gate awaiting your decision"),
            "twenty minutes cold is not working: its card must carry the Waiting on you line again: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (waiting-means-stopped-1) The same gate-stopped lane, but the bound
    /// session's heartbeat has gone stale enough (20 minutes, past
    /// `WORKING_MINUTES` but still inside `SESSION_LIVE_MINUTES`) that
    /// nobody is working it right now: the card — still In Progress, since
    /// it still has live work — regains its Waiting on you line, while the
    /// session itself — still live per the 30-minute window — keeps its own
    /// row on the Live strip.
    #[test]
    fn hub_gives_a_gate_stopped_feature_its_waiting_on_you_line_once_its_session_goes_stale_enough()
    {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-working-stale-{}",
            std::process::id()
        ));
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

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());
        let strip_html = bee_live_strip_section(&snapshot);

        assert!(
            html.contains(
                r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/stale-working-feat""#
            ),
            "the feature still has live work, so it stays under In Progress: {html}"
        );
        assert!(
            html.contains("Waiting on you — Shape gate awaiting your decision"),
            "a session gone stale past WORKING_MINUTES must let the gate pull the Waiting on you line back onto the card: {html}"
        );
        assert!(
            strip_html.contains("stale-working-feat"),
            "the session is still live per SESSION_LIVE_MINUTES (30), so its row must stay on the Live strip: {strip_html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (waiting-means-stopped-1) A pause handoff naming the active feature,
    /// with nobody's session bound to its lane at all, must still put the
    /// Waiting on you line on its In Progress card — the `working_now` gate
    /// only ever suppresses the pull when an agent really is on it.
    #[test]
    fn hub_gives_a_pause_handoff_feature_its_waiting_on_you_line_when_nobody_is_working_it() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-handoff-idle-{}",
            std::process::id()
        ));
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

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(
                r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/handoff-idle-feat""#
            ),
            "the active feature keeps live work and stays under In Progress: {html}"
        );
        assert!(
            html.contains("Waiting on you — Work is parked, waiting on your decision"),
            "a genuine pause handoff naming a feature nobody is working must still carry the Waiting on you line: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (waiting-means-stopped-1) A granted worktree alone — no session, no
    /// heartbeat — must never suppress the Waiting on you line: a grant is
    /// not a heartbeat, and a parked worktree with a gate owed is exactly
    /// the case that line exists for.
    #[test]
    fn hub_worktree_grant_alone_does_not_suppress_the_waiting_on_you_line() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-wt-no-working-{}",
            std::process::id()
        ));
        let sibling = std::env::temp_dir().join(format!(
            "waggledance-views-hub-wt-no-working-sibling-{}",
            std::process::id()
        ));
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
        write(
            &sibling,
            ".bee/state.json",
            r#"{"phase":"executing","feature":"wt-gate-owed-feat","mode":"standard"}"#,
        );
        let grant_id = sibling.file_name().unwrap().to_string_lossy().to_string();
        write(
            &root,
            ".bee/runtime/worktree-grants.json",
            &format!(r#"{{"{grant_id}": true}}"#),
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(
                r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/wt-gate-owed-feat""#
            ),
            "the granted worktree's own liveness pull keeps the card under In Progress: {html}"
        );
        assert!(
            html.contains("Waiting on you — Shape gate awaiting your decision"),
            "a granted worktree with no session bound must not suppress the Waiting on you line: {html}"
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
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-strip-session-{}",
            std::process::id()
        ));
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
            &format!(
                r#"{{"id": "live", "last_heartbeat": "{hb}", "lane": "strip-feat", "workspace_id": "ws-1"}}"#
            ),
        );
        write(
            ".bee/runtime/workspaces/ws-1.json",
            r#"{"id": "ws-1", "type": "worktree", "root": "sibling-dir", "branch": "wt/strip-feat", "attached_sessions": []}"#,
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let html = bee_live_strip_section(&snapshot);

        assert!(
            html.contains("strip-feat"),
            "the row must name the session's own lane: {html}"
        );
        assert!(
            html.contains("executing"),
            "the row must name that lane's phase: {html}"
        );
        assert!(
            html.contains("4 minutes ago"),
            "the row must state the heartbeat age: {html}"
        );
        assert!(
            html.contains("sibling-dir"),
            "the row must name the session's own workspace: {html}"
        );

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
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-strip-main-ws-{}",
            std::process::id()
        ));
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
            &format!(
                r#"{{"id": "main-ws", "last_heartbeat": "{hb}", "lane": "main-feat", "workspace_id": "main"}}"#
            ),
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

        let snapshot = waggledance_core::bee::read_snapshot(&root);
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
        let root =
            std::env::temp_dir().join(format!("waggledance-views-strip-wt-{}", std::process::id()));
        let sibling = std::env::temp_dir().join(format!(
            "waggledance-views-strip-wt-sibling-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
        let write = |dir: &std::path::Path, rel: &str, body: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        std::fs::create_dir_all(&sibling).unwrap();
        write(
            &sibling,
            ".bee/state.json",
            r#"{"phase":"swarming","feature":"wt-strip-feat","mode":"standard"}"#,
        );
        let grant_id = sibling.file_name().unwrap().to_string_lossy().to_string();
        write(
            &root,
            &format!(".bee/runtime/workspaces/{grant_id}.json"),
            &format!(
                r#"{{"id": "{grant_id}", "type": "worktree", "root": "wt-root", "branch": "wt/wt-strip-feat", "attached_sessions": []}}"#
            ),
        );
        write(
            &root,
            ".bee/runtime/worktree-grants.json",
            &format!(r#"{{"{grant_id}": true}}"#),
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let html = bee_live_strip_section(&snapshot);

        assert!(
            html.contains("wt/wt-strip-feat"),
            "the row must name the worktree's own branch: {html}"
        );
        assert!(
            html.contains("wt-strip-feat"),
            "the row must name the worktree's own active feature: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
    }

    /// (board-liveness-3) A granted worktree whose sibling directory does
    /// not exist at all is dangling — `resolve_worktree` reports it
    /// unresolved with a reason, never dropped — and the strip must say so
    /// in its own row rather than rendering nothing for that grant.
    #[test]
    fn live_strip_names_an_unresolved_worktree_grants_reason_rather_than_dropping_it() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-strip-wt-unresolved-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let dangling_id = format!("waggledance-views-strip-wt-ghost-{}", std::process::id());
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(&dangling_id));
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/runtime/worktree-grants.json",
            &format!(r#"{{"{dangling_id}": true}}"#),
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let html = bee_live_strip_section(&snapshot);

        assert!(
            html.contains("could not be read"),
            "an unresolved grant's own row must say it could not be read, never disappear silently: {html}"
        );
        assert!(
            html.contains(&dangling_id),
            "the unresolved row must still name the dangling grant: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (board-liveness-3) No live session and no worktree grant at all: the
    /// strip renders one honest empty line rather than an absent section.
    #[test]
    fn live_strip_renders_one_honest_line_when_nothing_is_live() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-strip-empty-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".bee")).unwrap();

        let snapshot = waggledance_core::bee::read_snapshot(&root);
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
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-parked-lane-{}",
            std::process::id()
        ));
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

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

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
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-closed-session-bound-{}",
            std::process::id()
        ));
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
            &format!(
                r#"{{"id": "live", "last_heartbeat": "{now}", "lane": "closed-session-feat"}}"#
            ),
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(
                r#"data-hub-group="finished" href="/p/proj-1/_bee/feature/closed-session-feat""#
            ),
            "a closed feature must stay under Finished even with a bound live session: {html}"
        );
        assert!(
            !html.contains("Waiting on you"),
            "a closed feature owes no decision even while a session is bound to its lane: {html}"
        );
        assert!(
            !html.contains(
                r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/closed-session-feat""#
            ),
            "a bound live session must never drag a finished feature back into In Progress: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (kanban-columns D11) Placement is tested In Progress, then Finished,
    /// then Review, then Compound, then Todo — Finished before Review is
    /// D11's own deliberate call, so a closed feature stays closed even
    /// while it still carries an unresolved review candidate.
    #[test]
    fn hub_keeps_a_closed_feature_in_finished_even_with_an_unresolved_review_candidate() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-finished-over-review-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/lanes/closed-reviewed-feat.json",
            r#"{
                "feature": "closed-reviewed-feat",
                "phase": "compounding-complete",
                "mode": "standard",
                "next_action": "none"
            }"#,
        );
        write(
            ".bee/review-candidates.jsonl",
            "{\"id\":\"rc-1\",\"type\":\"candidate\",\"date\":\"2026-08-01T00:00:00.000Z\",\"feature\":\"closed-reviewed-feat\",\"head\":\"h1\",\"mode\":\"standard\",\"baseline\":null,\"cells\":[\"cell-x\"]}\n",
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"data-hub-group="finished" href="/p/proj-1/_bee/feature/closed-reviewed-feat""#),
            "a closed feature must stay under Finished even carrying an unresolved review candidate: {html}"
        );
        assert!(
            !html.contains(
                r#"data-hub-group="review" href="/p/proj-1/_bee/feature/closed-reviewed-feat""#
            ),
            "the same feature must never also render under Review: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (kanban-columns D5) In Progress wins every tie: a feature with live
    /// work stays in In Progress even when it also carries an unresolved
    /// review candidate; it only reaches Review once it has no live cells
    /// left and the candidate is still unresolved.
    #[test]
    fn hub_places_live_work_in_progress_over_an_unresolved_review_candidate() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-live-over-review-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/lanes/live-reviewed-feat.json",
            r#"{
                "feature": "live-reviewed-feat",
                "phase": "swarming",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": true}
            }"#,
        );
        write(
            ".bee/cells/a.json",
            r#"{
                "id": "lr-1",
                "feature": "live-reviewed-feat",
                "lane": "tiny",
                "title": "Cell lr-1",
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
        write(
            ".bee/review-candidates.jsonl",
            "{\"id\":\"rc-2\",\"type\":\"candidate\",\"date\":\"2026-08-01T00:00:00.000Z\",\"feature\":\"live-reviewed-feat\",\"head\":\"h1\",\"mode\":\"standard\",\"baseline\":null,\"cells\":[\"cell-y\"]}\n",
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"bee-hub__detail-link" href="/p/proj-1/_bee/feature/live-reviewed-feat""#),
            "live work must keep the feature under In Progress despite the unresolved candidate: {html}"
        );
        assert!(
            !html.contains(r#"data-hub-group="review" href="/p/proj-1/_bee/feature/live-reviewed-feat""#),
            "the same feature must never also render under Review while it still has live work: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (kanban-columns D11) Review is tested before Compound: a feature
    /// with no live work, carrying both an unresolved review candidate and
    /// a `"compounding"` phase, lands in Review.
    #[test]
    fn hub_places_an_unresolved_candidate_in_review_over_a_compounding_phase() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-review-over-compound-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/lanes/both-signal-feat.json",
            r#"{
                "feature": "both-signal-feat",
                "phase": "compounding",
                "mode": "standard",
                "next_action": "none"
            }"#,
        );
        write(
            ".bee/review-candidates.jsonl",
            "{\"id\":\"rc-3\",\"type\":\"candidate\",\"date\":\"2026-08-01T00:00:00.000Z\",\"feature\":\"both-signal-feat\",\"head\":\"h1\",\"mode\":\"standard\",\"baseline\":null,\"cells\":[\"cell-z\"]}\n",
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(
                r#"data-hub-group="review" href="/p/proj-1/_bee/feature/both-signal-feat""#
            ),
            "a feature carrying both signals must stop at Review, its earlier branch: {html}"
        );
        assert!(
            !html.contains(
                r#"data-hub-group="compound" href="/p/proj-1/_bee/feature/both-signal-feat""#
            ),
            "the same feature must never also render under Compound: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// (kanban-columns D4) A `Settled` review candidate (every session
    /// naming one of its cells decided `approved` or `blocked`) must never
    /// pull a feature into Review.
    #[test]
    fn hub_a_settled_review_candidate_does_not_pull_a_feature_into_review() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-settled-review-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            ".bee/lanes/settled-feat.json",
            r#"{
                "feature": "settled-feat",
                "phase": "swarming",
                "mode": "standard",
                "next_action": "none"
            }"#,
        );
        write(
            ".bee/review-candidates.jsonl",
            "{\"id\":\"rc-4\",\"type\":\"candidate\",\"date\":\"2026-08-01T00:00:00.000Z\",\"feature\":\"settled-feat\",\"head\":\"h1\",\"mode\":\"standard\",\"baseline\":null,\"cells\":[\"cell-w\"]}\n",
        );
        write(
            ".bee/reviews/r1.json",
            r#"{"id":"r1","included":[{"type":"cell","id":"cell-w"}],"findings":[],"decision":{"status":"approved"}}"#,
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            !html.contains(r#"data-hub-group="review" href="/p/proj-1/_bee/feature/settled-feat""#),
            "a Settled candidate must never pull its feature into Review: {html}"
        );
        assert!(
            !html.contains("settled-feat"),
            "with no live work, no unresolved candidate, no compounding phase and no open cell, the feature renders nowhere: {html}"
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
        let (label, tone) =
            bee_hub_worktree_chip("wt-feat", std::slice::from_ref(&grant), &[], true);
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
        let (label, tone) =
            bee_hub_worktree_chip("shipped-feat", &[], std::slice::from_ref(&workspace), true);
        assert_eq!((label.as_str(), tone), ("Merged", "success"));

        let (label, tone) =
            bee_hub_worktree_chip("shipped-feat", &[], std::slice::from_ref(&workspace), false);
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
        let row = bee_hub_finished_row(
            "finished",
            "/p/proj-1/_bee/feature/shipped-feat",
            "shipped-feat",
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            row,
            r#"<a class="bee-hub__row" data-hub-group="finished" href="/p/proj-1/_bee/feature/shipped-feat">shipped-feat</a>"#
        );
        assert!(
            !row.contains("bee-hub__chips") && !row.contains("fg-chip") && !row.contains("bee-progress") && !row.contains("Last activity"),
            "a Finished row must carry none of the full card's chip/progress/activity markup: {row}"
        );
    }

    /// (kanban-columns D12) The dense row's group key and href are both
    /// caller-supplied parameters rather than hardcoded to Finished's own
    /// `"finished"` key and feature-detail link — the exact change that lets
    /// Todo, Review and Compound share this one renderer instead of each
    /// growing a copy.
    #[test]
    fn bee_hub_finished_row_carries_the_callers_own_group_key_and_href() {
        let row = bee_hub_finished_row(
            "todo",
            "/p/proj-1/_bee/feature/todo-feat",
            "todo-feat",
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            row,
            r#"<a class="bee-hub__row" data-hub-group="todo" href="/p/proj-1/_bee/feature/todo-feat">todo-feat</a>"#
        );
    }

    /// (hub-finished-compact, feature-titles parity) A dense row prefers
    /// the feature's own CONTEXT title over its slug, exactly as
    /// `bee_hub_card` already does for the one In Progress group — and drops
    /// the slug entirely rather than demoting it to a subtitle, since a
    /// dense row has no room for both.
    #[test]
    fn bee_hub_finished_row_prefers_the_context_title_over_the_slug() {
        let docs = waggledance_core::bee::BeeFeatureDocs {
            title: Some("Human Title".to_string()),
            description: None,
            docs: vec![],
        };
        let row = bee_hub_finished_row(
            "finished",
            "/p/proj-1/_bee/feature/slug-feat",
            "slug-feat",
            Some(&docs),
            None,
            None,
            None,
        );
        assert!(row.contains(">Human Title</a>"), "{row}");
        assert!(
            !row.contains(">slug-feat<"),
            "the slug must not also render once a title exists: {row}"
        );
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

        let first_details = html
            .find("<details")
            .expect("expected a details block for the overflow");
        for i in 0..10 {
            let marker = format!("<a>{i}</a>");
            let pos = html
                .find(&marker)
                .unwrap_or_else(|| panic!("missing {marker}: {html}"));
            assert!(
                pos < first_details,
                "row {i} must render open, ahead of the first <details>: {html}"
            );
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
    /// own tests, mirroring `waggledance_core::bee`'s own `feature_cell_json`
    /// test helper (not reusable across crates) so an archived feature can
    /// carry, or deliberately lack, a D10 ship time.
    fn cross_board_archived_cell_json(id: &str, feature: &str, capped_at: Option<&str>) -> String {
        let capped_json = capped_at
            .map(|s| format!("\"{s}\""))
            .unwrap_or_else(|| "null".to_string());
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
    /// in a different one of the states the per-project board itself proves:
    /// the same shape
    /// `hub_gives_a_gate_stopped_feature_its_waiting_on_you_line_once_its_session_goes_stale_enough`
    /// and its siblings already prove one project at a time. Each feature
    /// must land in the same column its own project's board would place it
    /// in, and must carry that project's own name.
    #[test]
    fn cross_project_places_each_feature_in_the_column_its_own_project_would_and_labels_it() {
        let root_a =
            std::env::temp_dir().join(format!("waggledance-views-cross-a-{}", std::process::id()));
        let root_b =
            std::env::temp_dir().join(format!("waggledance-views-cross-b-{}", std::process::id()));
        let root_c =
            std::env::temp_dir().join(format!("waggledance-views-cross-c-{}", std::process::id()));
        for r in [&root_a, &root_b, &root_c] {
            let _ = std::fs::remove_dir_all(r);
        }
        let write = |root: &std::path::Path, rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };

        // Project A: a gate-stopped feature, its session stale past
        // WORKING_MINUTES but still live -> In Progress, carrying its
        // Waiting on you line.
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
            &format!(
                r#"{{"id": "live", "last_heartbeat": "{fresh_hb}", "lane": "progress-feat"}}"#
            ),
        );

        // Project C: one archived feature, no lane and no session -> Finished.
        write(
            &root_c,
            ".bee/cells/archive/finished-feat/c-1.json",
            &cross_board_archived_cell_json(
                "c-1",
                "finished-feat",
                Some("2026-08-01T05:00:00.000Z"),
            ),
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

        let rollups =
            waggledance_core::bee::read_rollup(&[root_a.clone(), root_b.clone(), root_c.clone()]);
        let pairs: Vec<(&Project, &BeeProjectRollup)> = vec![
            (&project_a, &rollups[0]),
            (&project_b, &rollups[1]),
            (&project_c, &rollups[2]),
        ];
        let html = bee_cross_project_features_section(&pairs, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"bee-hub__detail-link" href="/p/proj-a/_bee/feature/waiting-feat""#),
            "waiting-feat must land under In Progress, same as its own project's board would: {html}"
        );
        assert!(
            html.contains("Waiting on you — Shape gate awaiting your decision"),
            "waiting-feat's card must still carry its own Waiting on you line: {html}"
        );
        assert!(
            html.contains("Project A"),
            "the card must carry its own project's name: {html}"
        );
        assert!(
            html.contains(r#"bee-hub__detail-link" href="/p/proj-b/_bee/feature/progress-feat""#),
            "progress-feat must land under In Progress: {html}"
        );
        assert!(
            html.contains("Project B"),
            "the in-progress card must carry its own project's name: {html}"
        );
        assert!(
            html.contains(
                r#"data-hub-group="finished" href="/p/proj-c/_bee/feature/finished-feat""#
            ),
            "finished-feat must land under Finished: {html}"
        );
        assert!(
            html.contains("Project C"),
            "the finished row must carry its own project's name: {html}"
        );
        assert!(
            html.contains(r#"data-hub-group="in-progress" data-hub-count="2""#),
            "the In Progress count must be the sum across projects, both waiting-feat and progress-feat: {html}"
        );
        assert!(
            html.contains(r#"data-hub-group="finished" data-hub-count="1""#),
            "the Finished count must be the sum across projects: {html}"
        );

        for r in [&root_a, &root_b, &root_c] {
            let _ = std::fs::remove_dir_all(r);
        }
    }

    /// backlog-groom-2 D3: a `.bee/state.json` that fails to parse lands in
    /// [`BeeSnapshot::read_errors`] via [`waggledance_core::bee::read_snapshot`]
    /// exactly as the module doc says, and the board header must surface it
    /// as the one concise warning strip -- not silence, not the removed
    /// multi-item Needs-attention panel.
    #[test]
    fn cross_project_board_with_read_errors_renders_the_warning_strip() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-cross-read-errors-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".bee")).unwrap();
        std::fs::write(root.join(".bee/state.json"), "{ not json").unwrap();

        let mut project = sample_project();
        project.id = "proj-broken".into();
        project.root_path = root.clone();

        let rollups = waggledance_core::bee::read_rollup(std::slice::from_ref(&root));
        let pairs: Vec<(&Project, &BeeProjectRollup)> = vec![(&project, &rollups[0])];
        let html = bee_cross_project_features_section(&pairs, &std::collections::HashMap::new());

        assert!(
            html.contains("fg-banner--warning"),
            "a snapshot with read_errors must render the warning strip: {html}"
        );
        assert!(
            html.contains(r#"data-bee-read-errors="1">"#)
                && html.contains("1 .bee file(s) could not be read; counts may be incomplete"),
            "the strip must name the read_errors count: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// backlog-groom-2 D3: the mirror of
    /// `cross_project_board_with_read_errors_renders_the_warning_strip` --
    /// a clean `.bee/` (no `read_errors`) renders no strip at all, not an
    /// empty one.
    #[test]
    fn cross_project_board_with_no_read_errors_renders_no_warning_strip() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-cross-clean-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".bee")).unwrap();

        let mut project = sample_project();
        project.id = "proj-clean".into();
        project.root_path = root.clone();

        let rollups = waggledance_core::bee::read_rollup(std::slice::from_ref(&root));
        let pairs: Vec<(&Project, &BeeProjectRollup)> = vec![(&project, &rollups[0])];
        let html = bee_cross_project_features_section(&pairs, &std::collections::HashMap::new());

        assert!(
            !html.contains("fg-banner--warning") && !html.contains("could not be read"),
            "a clean snapshot must render no warning strip at all: {html}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// inprogress-priority-order-2 (D4/D6/D7): two projects, each
    /// contributing one In Progress feature and one Todo feature. The In
    /// Progress column must merge into ONE list ordered by the shared D7
    /// comparator -- proj-b's blocked feature outranks proj-a's quiet one
    /// even though it is alphabetically later and belongs to the SECOND
    /// project in input order, proving this is a real cross-project sort,
    /// not a per-project concatenation. The Todo column, D6 says, keeps its
    /// own per-project grouping untouched: proj-a's Todo row must still sit
    /// above proj-b's, in project order, even though proj-b's own feature
    /// name sorts alphabetically first -- proving Todo was never handed
    /// through the new comparator at all.
    #[test]
    fn cross_project_in_progress_interleaves_by_comparator_while_todo_keeps_its_project_grouping() {
        let root_a = std::env::temp_dir().join(format!(
            "waggledance-views-cross-d4-a-{}",
            std::process::id()
        ));
        let root_b = std::env::temp_dir().join(format!(
            "waggledance-views-cross-d4-b-{}",
            std::process::id()
        ));
        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
        let write = |root: &std::path::Path, rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        let open_cell_json = |id: &str, feature: &str| -> String {
            format!(
                r#"{{
                    "id": "{id}",
                    "feature": "{feature}",
                    "lane": "standard",
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
                    "status": "open",
                    "tier": "generation",
                    "trace": {{"worker": "w1", "claimed_at": null, "capped_at": null}}
                }}"#
            )
        };
        let fresh_hb = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();

        // Project A: "aaa-quiet" is In Progress with no pane at all (D7's
        // rest tier); "zzz-todo-a" is Todo.
        write(
            &root_a,
            ".bee/lanes/aaa-quiet.json",
            r#"{
                "feature": "aaa-quiet",
                "phase": "swarming",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": true}
            }"#,
        );
        write(
            &root_a,
            ".bee/sessions/live-a.json",
            &format!(r#"{{"id": "live-a", "last_heartbeat": "{fresh_hb}", "lane": "aaa-quiet"}}"#),
        );
        write(
            &root_a,
            ".bee/lanes/zzz-todo-a.json",
            r#"{
                "feature": "zzz-todo-a",
                "phase": "exploring",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": false, "shape": false, "execution": false, "review": false}
            }"#,
        );
        write(
            &root_a,
            ".bee/cells/zzz-todo-a-1.json",
            &open_cell_json("zzz-todo-a-1", "zzz-todo-a"),
        );

        // Project B: "zzz-blocked" is In Progress and will carry a
        // `"blocked"` pane through `feature_panes` below (D7's top tier);
        // "aaa-todo-b" is Todo, alphabetically ahead of proj-a's own Todo
        // feature, so an alphabetical (rather than per-project) Todo merge
        // would put it first.
        write(
            &root_b,
            ".bee/lanes/zzz-blocked.json",
            r#"{
                "feature": "zzz-blocked",
                "phase": "swarming",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": true}
            }"#,
        );
        write(
            &root_b,
            ".bee/sessions/live-b.json",
            &format!(
                r#"{{"id": "live-b", "last_heartbeat": "{fresh_hb}", "lane": "zzz-blocked"}}"#
            ),
        );
        write(
            &root_b,
            ".bee/lanes/aaa-todo-b.json",
            r#"{
                "feature": "aaa-todo-b",
                "phase": "exploring",
                "mode": "standard",
                "next_action": "keep going",
                "approved_gates": {"context": false, "shape": false, "execution": false, "review": false}
            }"#,
        );
        write(
            &root_b,
            ".bee/cells/aaa-todo-b-1.json",
            &open_cell_json("aaa-todo-b-1", "aaa-todo-b"),
        );

        let mut project_a = sample_project();
        project_a.id = "proj-a".into();
        project_a.name = "Project A".into();
        project_a.root_path = root_a.clone();
        let mut project_b = sample_project();
        project_b.id = "proj-b".into();
        project_b.name = "Project B".into();
        project_b.root_path = root_b.clone();

        let rollups = waggledance_core::bee::read_rollup(&[root_a.clone(), root_b.clone()]);
        let pairs: Vec<(&Project, &BeeProjectRollup)> =
            vec![(&project_a, &rollups[0]), (&project_b, &rollups[1])];

        let blocked_pane = TerminalPaneView {
            pane_id: "w1:blocked-pane".into(),
            kind: "codex".into(),
            name: "agent".into(),
            status: "blocked".into(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        };
        let mut proj_b_panes: std::collections::HashMap<String, Vec<TerminalPaneView>> =
            std::collections::HashMap::new();
        proj_b_panes.insert("zzz-blocked".to_string(), vec![blocked_pane]);
        let mut feature_panes: std::collections::HashMap<
            String,
            std::collections::HashMap<String, Vec<TerminalPaneView>>,
        > = std::collections::HashMap::new();
        feature_panes.insert("proj-b".to_string(), proj_b_panes);

        let html = bee_cross_project_features_section(&pairs, &feature_panes);

        let blocked_at = html
            .find(r#"bee-hub__detail-link" href="/p/proj-b/_bee/feature/zzz-blocked""#)
            .unwrap_or_else(|| panic!("zzz-blocked's own card must render: {html}"));
        let quiet_at = html
            .find(r#"bee-hub__detail-link" href="/p/proj-a/_bee/feature/aaa-quiet""#)
            .unwrap_or_else(|| panic!("aaa-quiet's own card must render: {html}"));
        assert!(
            blocked_at < quiet_at,
            "the blocked feature must sort ahead of the quiet one across projects, D7 tier beating both alphabetical and input order: {html}"
        );

        let todo_a_at = html
            .find(r#"data-hub-group="todo" href="/p/proj-a/_bee/feature/zzz-todo-a""#)
            .unwrap_or_else(|| panic!("zzz-todo-a's own row must render: {html}"));
        let todo_b_at = html
            .find(r#"data-hub-group="todo" href="/p/proj-b/_bee/feature/aaa-todo-b""#)
            .unwrap_or_else(|| panic!("aaa-todo-b's own row must render: {html}"));
        assert!(
            todo_a_at < todo_b_at,
            "Todo must keep its own per-project grouping (D6): proj-a's row must still precede proj-b's, even though proj-b's feature name sorts alphabetically first: {html}"
        );

        for r in [&root_a, &root_b] {
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
        let root_a = std::env::temp_dir().join(format!(
            "waggledance-views-cross-d10-a-{}",
            std::process::id()
        ));
        let root_b = std::env::temp_dir().join(format!(
            "waggledance-views-cross-d10-b-{}",
            std::process::id()
        ));
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
        write(
            &root_a,
            ".bee/cells/archive/zeta-feat/c-1.json",
            &cross_board_archived_cell_json("c-1", "zeta-feat", None),
        );
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

        let rollups = waggledance_core::bee::read_rollup(&[root_a.clone(), root_b.clone()]);
        let pairs: Vec<(&Project, &BeeProjectRollup)> =
            vec![(&project_a, &rollups[0]), (&project_b, &rollups[1])];
        let html = bee_cross_project_features_section(&pairs, &std::collections::HashMap::new());

        let pos_newer = html.find("newer-feat").expect("newer-feat must render");
        let pos_older = html.find("older-feat").expect("older-feat must render");
        let pos_alpha = html.find("alpha-feat").expect("alpha-feat must render");
        let pos_zeta = html.find("zeta-feat").expect("zeta-feat must render");

        assert!(
            pos_newer < pos_older,
            "the most recently shipped feature must render first: {html}"
        );
        assert!(
            pos_older < pos_alpha,
            "every timed feature must render ahead of every untimed one: {html}"
        );
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
        let root_a = std::env::temp_dir().join(format!(
            "waggledance-views-cross-cap-a-{}",
            std::process::id()
        ));
        let root_b = std::env::temp_dir().join(format!(
            "waggledance-views-cross-cap-b-{}",
            std::process::id()
        ));
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

        let rollups = waggledance_core::bee::read_rollup(&[root_a.clone(), root_b.clone()]);
        let pairs: Vec<(&Project, &BeeProjectRollup)> =
            vec![(&project_a, &rollups[0]), (&project_b, &rollups[1])];
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
        let root_a = std::env::temp_dir().join(format!(
            "waggledance-views-cross-dup-a-{}",
            std::process::id()
        ));
        let root_b = std::env::temp_dir().join(format!(
            "waggledance-views-cross-dup-b-{}",
            std::process::id()
        ));
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

        let rollups = waggledance_core::bee::read_rollup(&[root_a.clone(), root_b.clone()]);
        let pairs: Vec<(&Project, &BeeProjectRollup)> =
            vec![(&project_a, &rollups[0]), (&project_b, &rollups[1])];
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
        assert!(
            html.contains("Project A") && html.contains("Project B"),
            "each row must carry its own project's name: {html}"
        );

        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
    }

    /// (cross-board, edge case) A qualifying project with `.bee/` but no
    /// features at all must contribute nothing and break nothing -- the
    /// merged section reads exactly as if that project were absent.
    #[test]
    fn cross_project_a_project_contributing_no_features_changes_nothing() {
        let root_a = std::env::temp_dir().join(format!(
            "waggledance-views-cross-empty-a-{}",
            std::process::id()
        ));
        let root_b = std::env::temp_dir().join(format!(
            "waggledance-views-cross-empty-b-{}",
            std::process::id()
        ));
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

        let rollups = waggledance_core::bee::read_rollup(&[root_a.clone(), root_b.clone()]);
        let with_empty: Vec<(&Project, &BeeProjectRollup)> =
            vec![(&project_a, &rollups[0]), (&project_b, &rollups[1])];
        let without_empty: Vec<(&Project, &BeeProjectRollup)> = vec![(&project_b, &rollups[1])];

        let html_with =
            bee_cross_project_features_section(&with_empty, &std::collections::HashMap::new());
        let html_without =
            bee_cross_project_features_section(&without_empty, &std::collections::HashMap::new());

        assert_eq!(
            html_with, html_without,
            "a project contributing no features must change nothing in the merged section"
        );

        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
    }

    /// (project-color-identity) A project contributing both a card (an In
    /// Progress feature carrying a Waiting on you line) and a Finished row
    /// (an archived feature) gets the same colour slot in both -- the map is
    /// built once, up front, and threaded down, never recomputed per
    /// renderer.
    #[test]
    fn cross_project_same_project_gets_the_same_slot_on_its_card_and_its_finished_row() {
        let root_a = std::env::temp_dir().join(format!(
            "waggledance-views-cross-color-a-{}",
            std::process::id()
        ));
        let root_b = std::env::temp_dir().join(format!(
            "waggledance-views-cross-color-b-{}",
            std::process::id()
        ));
        for r in [&root_a, &root_b] {
            let _ = std::fs::remove_dir_all(r);
        }
        let write = |root: &std::path::Path, rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };

        // Project A: a gate-stopped, stale-session live feature -> a
        // Waiting card, plus an archived feature -> a Finished row.
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
        write(
            &root_a,
            ".bee/cells/archive/finished-feat/c-1.json",
            &cross_board_archived_cell_json("c-1", "finished-feat", None),
        );
        // Project B: an archived feature too, so the colour map has more
        // than one project to sort and slot.
        write(
            &root_b,
            ".bee/cells/archive/other-feat/c-1.json",
            &cross_board_archived_cell_json("c-1", "other-feat", None),
        );

        let mut project_a = sample_project();
        project_a.id = "proj-a".into();
        project_a.name = "Project A".into();
        project_a.root_path = root_a.clone();
        let mut project_b = sample_project();
        project_b.id = "proj-b".into();
        project_b.name = "Project B".into();
        project_b.root_path = root_b.clone();

        let rollups = waggledance_core::bee::read_rollup(&[root_a.clone(), root_b.clone()]);
        let pairs: Vec<(&Project, &BeeProjectRollup)> =
            vec![(&project_a, &rollups[0]), (&project_b, &rollups[1])];
        let html = bee_cross_project_features_section(&pairs, &std::collections::HashMap::new());

        let card_slot = (1..=10)
            .find(|n| {
                html.contains(&format!(
                    "bee-hub__shell--p{n}\"><details class=\"bee-hub__card\" data-hub-group=\"in-progress\">"
                ))
            })
            .unwrap_or_else(|| panic!("proj-a's own In Progress card must carry a colour modifier: {html}"));
        let row_slot = (1..=10)
            .find(|n| html.contains(&format!("bee-hub__row-project--p{n}\">Project A</span>")))
            .unwrap_or_else(|| {
                panic!("proj-a's own Finished row must carry a colour modifier: {html}")
            });
        assert_eq!(
            card_slot, row_slot,
            "the same project must get the same colour slot on its card and its Finished row: {html}"
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
    /// card-badge-inside: both the anchor and the nav now sit inside a
    /// shared `<div class="fg-card bee-hub__shell">`, which carries the
    /// box paint the anchor used to carry alone, so the badges land inside
    /// the card's own box at its foot rather than as a bare row beneath
    /// it.
    #[test]
    fn bee_hub_card_emits_terminal_badges_matching_project_badges_markup_shape() {
        let panes = vec![TerminalPaneView {
            pane_id: "w1:p1".into(),
            kind: "claude".into(),
            name: "agent-name-must-not-appear".into(),
            status: "working".into(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        }];
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &panes,
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        // project_badges' own markup, with only its aria-label swapped for
        // the checkout-naming one this card must carry and its own
        // `bee-hub__badges` class layered onto the shared container class
        // -- proving the rest of the shape (anchor class, status pill,
        // program span, href) is exactly project_badges' own, unchanged.
        let expected_badges = project_badges("proj-a", &panes)
            .replace("Terminal panes", "Terminals in this checkout")
            .replacen(
                r#"class="proj-row__badges""#,
                r#"class="proj-row__badges bee-hub__badges""#,
                1,
            );
        assert!(
            card_html.contains(&expected_badges),
            "the card must embed project_badges' own markup shape (aria-label and bee-hub__badges class aside): {card_html}"
        );
        assert!(
            card_html.ends_with(&format!("{expected_badges}</div>")),
            "the badge nav must close immediately before the shared shell's own closing </div>: {card_html}"
        );
        assert!(
            card_html.starts_with(r#"<div class="fg-card bee-hub__shell">"#),
            "the card anchor and its badge nav must share one fg-card bee-hub__shell wrapper: {card_html}"
        );
        assert!(
            card_html.contains(r#"<details class="bee-hub__card""#),
            "card-collapse-inprogress: the details element itself must no longer carry fg-card -- the shell paints the box now: {card_html}"
        );
        assert!(
            !card_html.contains(r#"class="fg-card bee-hub__card""#),
            "fg-card must move off the card onto the shell, never stay on both: {card_html}"
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
        let details_end = card_html
            .find("</details>")
            .expect("the card's own details must close");
        let badge_nav_start = card_html
            .find(r#"<nav class="proj-row__badges"#)
            .expect("badges must render");
        assert!(
            badge_nav_start > details_end,
            "the badge nav must be a sibling after the card's own </details>, not nested inside it: {card_html}"
        );
    }

    /// board-badges-agents-only: plain-shell panes (kind == "shell") never
    /// badge on the board — a mixed list badges only its agent panes, and a
    /// list of nothing but shells renders no container at all, exactly like
    /// an empty list.
    #[test]
    fn board_badges_skip_plain_shell_panes() {
        let shell = |id: &str| TerminalPaneView {
            pane_id: id.into(),
            kind: "shell".into(),
            name: String::new(),
            status: "shell".into(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t1".into(),
        };
        let agent = TerminalPaneView {
            pane_id: "w1:p2".into(),
            kind: "claude".into(),
            name: "n".into(),
            status: "working".into(),
            cwd: String::new(),
            workspace: "w1".into(),
            tab: "t2".into(),
        };
        let mixed = project_badges("proj-a", &[shell("w1:p1"), agent.clone()]);
        assert!(
            mixed.contains("pane/w1:p2") && !mixed.contains("pane/w1:p1"),
            "a mixed list must badge only the agent pane, never the shell pane: {mixed}"
        );
        assert!(
            !mixed.contains(">shell</span>"),
            "the shell pane's program text must never appear once its badge is skipped: {mixed}"
        );
        let only_shells = project_badges("proj-a", &[shell("w1:p1"), shell("w1:p3")]);
        assert!(
            only_shells.is_empty(),
            "a list of nothing but shell panes must render no container at all, exactly like an empty list: {only_shells}"
        );
    }

    /// (card-terminals-1) An empty `panes` slice -- the switch off, herdr
    /// unreachable, or genuinely no pane in this feature's checkout --
    /// renders no badge container at all. card-badge-inside: the card is
    /// still wrapped in its `fg-card bee-hub__shell` div (the shell now
    /// carries the box paint the anchor used to carry alone), but with no
    /// trailing `<nav>` the card looks exactly as it did before this
    /// feature -- no empty container is left behind.
    #[test]
    fn bee_hub_card_with_no_panes_renders_no_badge_container() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            !card_html.contains("proj-row__badges"),
            "an empty pane list must render no badge container: {card_html}"
        );
        assert!(
            !card_html.contains("bee-hub__badges"),
            "an empty pane list must render no badge nav, including its extra class: {card_html}"
        );
        assert!(
            card_html.starts_with(r#"<div class="fg-card bee-hub__shell">"#),
            "the shell must still wrap the bare card anchor: {card_html}"
        );
        assert!(
            card_html.ends_with("</details></div>"),
            "with no badges, the shell must close right after the card's own </details>: {card_html}"
        );
    }

    /// inprogress-priority-order-2 (D8, must-have): a card carrying a
    /// `"blocked"` pane and no gate/handoff reason of its own gains exactly
    /// one reason line, reading `Waiting on you — a terminal is blocked`,
    /// in the existing `bee-hub__reason` style.
    #[test]
    fn bee_hub_card_with_a_blocked_pane_carries_its_own_waiting_on_you_line() {
        let panes = vec![pane_with_status("blocked")];
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &panes,
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            card_html.contains(r#"<p class="bee-cell__meta bee-hub__reason">Waiting on you — a terminal is blocked</p>"#),
            "a blocked pane must add its own exact reason line: {card_html}"
        );
    }

    /// inprogress-priority-order-2 (D8, must-have): a card that already
    /// carries a gate/handoff reason AND a blocked pane shows BOTH lines,
    /// the existing gate line first -- neither line swallows the other.
    #[test]
    fn bee_hub_card_with_a_gate_reason_and_a_blocked_pane_carries_both_lines_gate_first() {
        let panes = vec![pane_with_status("blocked")];
        let gate_reason = "Waiting on you — Shape gate awaiting your decision";
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: Some(gate_reason),
            docs: None,
            project_label: None,
            project_color: None,
            panes: &panes,
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        let gate_at = card_html.find(gate_reason).unwrap_or_else(|| {
            panic!("the existing gate reason line must still render: {card_html}")
        });
        let blocked_at = card_html
            .find("Waiting on you — a terminal is blocked")
            .unwrap_or_else(|| {
                panic!("the blocked pane's own reason line must also render: {card_html}")
            });
        assert!(
            gate_at < blocked_at,
            "the gate line must come first, the blocked line after it, never the other way round: {card_html}"
        );
    }

    /// inprogress-priority-order-2 (D8, must-have): a card with no gate
    /// reason and no blocked pane carries neither reason line.
    #[test]
    fn bee_hub_card_with_no_blocked_pane_and_no_gate_reason_carries_neither_line() {
        let panes = vec![pane_with_status("working")];
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &panes,
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            !card_html.contains("bee-hub__reason"),
            "with no gate reason and no blocked pane, neither reason line may render: {card_html}"
        );
    }

    /// (card-collapse-inprogress-2) CONTEXT D1/D6: every card renders
    /// collapsed on every page load, with no persisted open/closed state --
    /// a stray `open` attribute here would ship every card pre-expanded and
    /// nothing else in this file would ever notice.
    #[test]
    fn bee_hub_card_renders_collapsed_with_no_open_attribute() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            card_html.contains(r#"<details class="bee-hub__card" data-hub-group="in-progress">"#),
            "the card must render as a details element carrying its own group key: {card_html}"
        );
        assert!(
            !card_html.contains(r#"bee-hub__card" data-hub-group="in-progress" open"#),
            "the details element must carry no `open` attribute -- every card must start collapsed: {card_html}"
        );
        assert!(
            card_html.contains(r#"<summary class="bee-hub__summary">"#)
                && card_html.contains(r#"<span class="bee-hub__chev" aria-hidden="true">"#),
            "the collapsed header must carry both its summary and its chevron: {card_html}"
        );
    }

    /// (card-collapse-inprogress-2) CONTEXT D4: the Feature detail link row
    /// is the one place the card still reaches its own detail page now that
    /// the `<details>`/`<summary>` pair can no longer be that link itself.
    #[test]
    fn bee_hub_card_body_opens_with_the_feature_detail_link() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            card_html.contains(
                r#"<div class="bee-hub__body"><a class="bee-hub__detail-link" href="/p/proj-a/_bee/feature/feat-a">Feature detail"#
            ),
            "the body must open with a Feature detail link carrying the feature's own detail-page href: {card_html}"
        );
    }

    // --- kanban-live-signals D1/D2/D3 ---

    /// kanban-live-signals D1: `state.json`'s own `last_activity` wins when
    /// it is newer than the cell-derived value -- the merge
    /// [`bee_hub_effective_activity`] does, never a re-derivation inside
    /// this test.
    #[test]
    fn bee_hub_effective_activity_prefers_state_last_activity_when_newer() {
        let cell = "2026-08-01T00:00:00.000Z";
        let state = "2026-08-15T15:48:08.674Z";
        assert_eq!(
            bee_hub_effective_activity(Some(cell), Some(state)),
            Some(state.to_string()),
            "the newer state.json timestamp must win"
        );
    }

    /// kanban-live-signals D1: the cell-derived value wins when it is newer
    /// than `state.json`'s own -- the merge is a true max, not a blind
    /// preference for state.json.
    #[test]
    fn bee_hub_effective_activity_prefers_cell_activity_when_newer() {
        let cell = "2026-08-15T15:48:08.674Z";
        let state = "2026-08-01T00:00:00.000Z";
        assert_eq!(
            bee_hub_effective_activity(Some(cell), Some(state)),
            Some(cell.to_string()),
            "the newer cell-derived timestamp must win"
        );
    }

    /// kanban-live-signals D1: a bee version whose `state.json` predates
    /// `last_activity` entirely (the key absent, `None` here) must still
    /// fall back to the cell-derived value, never go blank.
    #[test]
    fn bee_hub_effective_activity_falls_back_to_cell_activity_when_state_absent() {
        let cell = "2026-08-01T00:00:00.000Z";
        assert_eq!(
            bee_hub_effective_activity(Some(cell), None),
            Some(cell.to_string()),
            "an absent state.json last_activity must fall back to the cell-derived value"
        );
    }

    /// kanban-live-signals D1: neither side present is still `None`, never
    /// a fabricated "just now".
    #[test]
    fn bee_hub_effective_activity_both_absent_is_none() {
        assert_eq!(bee_hub_effective_activity(None, None), None);
    }

    /// kanban-live-signals D1: a tool call inside the ~2-minute window
    /// renders the pulse dot beside the activity line.
    #[test]
    fn bee_hub_card_with_a_recent_tool_call_renders_the_pulse_dot() {
        let recent = (time::OffsetDateTime::now_utc() - time::Duration::seconds(30))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: Some("2026-08-15T15:48:08.674Z"),
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: Some(&recent),
            deferred: &[],
        });
        assert!(
            card_html.contains(r#"<span class="bee-hub__pulse""#),
            "a tool call landed 30 seconds ago must render the pulse dot: {card_html}"
        );
    }

    /// kanban-live-signals D1: a tool call outside the ~2-minute window
    /// renders no pulse dot at all.
    #[test]
    fn bee_hub_card_with_an_old_tool_call_renders_no_pulse_dot() {
        let old = (time::OffsetDateTime::now_utc() - time::Duration::minutes(20))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: Some("2026-08-15T15:48:08.674Z"),
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: Some(&old),
            deferred: &[],
        });
        assert!(
            !card_html.contains("bee-hub__pulse"),
            "a tool call 20 minutes old must render no pulse dot: {card_html}"
        );
    }

    /// kanban-live-signals D1: no `last_tool_call` at all renders no pulse
    /// dot -- the same "never fabricated" discipline every other signal
    /// here follows.
    #[test]
    fn bee_hub_card_with_no_tool_call_renders_no_pulse_dot() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            !card_html.contains("bee-hub__pulse"),
            "no last_tool_call must render no pulse dot: {card_html}"
        );
    }

    /// kanban-live-signals D2: `awaiting-approval` renders the run_state
    /// badge in its own prominent `--danger` tone, in the collapsed
    /// `<summary>` so it stays visible without expanding the card.
    #[test]
    fn bee_hub_card_awaiting_approval_run_state_renders_prominent_badge() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: Some("awaiting-approval"),
            waiting_on_live: true,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            card_html.contains(r#"<span class="fg-chip fg-chip--danger bee-hub__run-state">Awaiting approval</span>"#),
            "awaiting-approval with a live waiting_on mark must render its own prominent danger-toned badge: {card_html}"
        );
        let summary_end = card_html.find("</summary>").expect("summary must close");
        let badge_at = card_html
            .find("bee-hub__run-state")
            .expect("badge must render");
        assert!(
            badge_at < summary_end,
            "the run_state badge must render inside the collapsed summary: {card_html}"
        );
    }

    /// waiting-on-badge: `awaiting-approval` without a live `waiting_on`
    /// mark reads as the neutral "Unreviewed" chip, never the danger tone —
    /// this is the routine case (a pending, user-invoked review gate with
    /// nobody actually waiting), not the exceptional one.
    #[test]
    fn bee_hub_card_awaiting_approval_without_live_waiting_on_renders_neutral_unreviewed() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: Some("awaiting-approval"),
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            card_html.contains(
                r#"<span class="fg-chip fg-chip--neutral bee-hub__run-state">Unreviewed</span>"#
            ),
            "awaiting-approval without a live waiting_on mark must render the neutral Unreviewed chip: {card_html}"
        );
        assert!(
            !card_html.contains("fg-chip--danger"),
            "the danger chip must not render without a live waiting_on mark: {card_html}"
        );
    }

    /// kanban-live-signals D2: distinct run_state values get distinct
    /// tones, never sharing awaiting-approval's own prominent one.
    #[test]
    fn bee_hub_card_running_run_state_renders_its_own_distinct_tone() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: Some("running"),
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            card_html.contains(r#"<span class="fg-chip fg-chip--accent bee-hub__run-state">Running</span>"#),
            "running must render its own accent-toned badge, distinct from awaiting-approval's danger tone: {card_html}"
        );
    }

    /// kanban-live-signals D2: no `run_state` key at all (an older
    /// `state.json`, or a non-active feature -- see [`BeeHubCardData`]'s
    /// doc comment) renders no badge.
    #[test]
    fn bee_hub_card_with_no_run_state_renders_no_badge() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            !card_html.contains("bee-hub__run-state"),
            "absent run_state must render no badge at all: {card_html}"
        );
    }

    /// kanban-live-signals D3: unresolved deferred debt renders a count
    /// badge plus its per-entry kind and reason behind the board's own
    /// click-to-reveal detail idiom.
    #[test]
    fn bee_hub_card_deferred_debt_renders_count_badge_and_detail() {
        let deferred = vec![
            BeeDeferredEntry {
                id: "d1".to_string(),
                kind: Some("question".to_string()),
                feature: Some("feat-a".to_string()),
                reason: Some("waiting on a UX call".to_string()),
            },
            BeeDeferredEntry {
                id: "d2".to_string(),
                kind: Some("cleanup".to_string()),
                feature: Some("feat-a".to_string()),
                reason: Some("dead code left behind".to_string()),
            },
        ];
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &deferred,
        });
        assert!(
            card_html.contains(r#"<summary class="fg-badge">2 deferred</summary>"#),
            "two unresolved entries must render a '2 deferred' count badge: {card_html}"
        );
        assert!(
            card_html.contains("question") && card_html.contains("waiting on a UX call"),
            "the first entry's own kind and reason must be reachable in the detail: {card_html}"
        );
        assert!(
            card_html.contains("cleanup") && card_html.contains("dead code left behind"),
            "the second entry's own kind and reason must be reachable in the detail: {card_html}"
        );
    }

    /// kanban-live-signals D3: zero unresolved debt renders nothing at all
    /// -- no empty badge, no empty detail container.
    #[test]
    fn bee_hub_card_with_zero_deferred_debt_renders_nothing() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            !card_html.contains("bee-hub__deferred") && !card_html.contains("deferred"),
            "zero unresolved debt must render no badge and no detail container: {card_html}"
        );
    }

    /// kanban-live-signals D1/D2: `state.json` describes ONE project's
    /// single currently-active feature (`state.feature`) -- its
    /// `last_activity`, `run_state` and tools.jsonl liveness pulse must land
    /// on that feature's own In Progress card only, never bleed onto a
    /// sibling In Progress card for a DIFFERENT feature in the same
    /// project. D3's deferred debt carries no such restriction: each entry
    /// already names its own feature, so both cards get their own.
    #[test]
    fn hub_scopes_state_json_signals_to_the_active_feature_only_but_deferred_debt_to_each_own_feature(
    ) {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-active-feature-scope-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, body: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        write(
            ".bee/state.json",
            &format!(
                r#"{{"feature": "active-feat", "phase": "swarming", "last_activity": "{now}", "run_state": "awaiting-approval", "waiting_on": {{"kind": "gate", "subject": "shape"}}}}"#
            ),
        );
        write(
            ".bee/logs/tools.jsonl",
            &format!(r#"{{"ts":"{now}","tool_name":"Bash"}}"#),
        );
        write(
            ".bee/lanes/active-feat.json",
            r#"{"feature": "active-feat", "phase": "swarming", "mode": "standard", "next_action": "keep going"}"#,
        );
        write(
            ".bee/lanes/other-feat.json",
            r#"{"feature": "other-feat", "phase": "swarming", "mode": "standard", "next_action": "keep going"}"#,
        );
        write(
            ".bee/sessions/s1.json",
            &format!(r#"{{"id": "s1", "last_heartbeat": "{now}", "lane": "active-feat"}}"#),
        );
        write(
            ".bee/sessions/s2.json",
            &format!(r#"{{"id": "s2", "last_heartbeat": "{now}", "lane": "other-feat"}}"#),
        );
        write(
            ".bee/deferred-queue.jsonl",
            "{\"id\": \"d1\", \"event\": \"add\", \"feature\": \"active-feat\", \"kind\": \"question\", \"reason\": \"needs a call\"}\n\
             {\"id\": \"d2\", \"event\": \"add\", \"feature\": \"other-feat\", \"kind\": \"cleanup\", \"reason\": \"leftover code\"}\n",
        );

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        let active_link = "href=\"/p/proj-1/_bee/feature/active-feat\"";
        let other_link = "href=\"/p/proj-1/_bee/feature/other-feat\"";
        let active_start = html
            .find(active_link)
            .expect("active-feat card must render");
        let other_start = html.find(other_link).expect("other-feat card must render");
        let (active_card, other_card) = if active_start < other_start {
            (&html[..other_start], &html[other_start..])
        } else {
            (&html[active_start..], &html[..active_start])
        };

        assert!(
            active_card.contains("bee-hub__run-state")
                && active_card.contains(r#"fg-chip--danger"#),
            "the active feature's card must carry the awaiting-approval badge: {active_card}"
        );
        assert!(
            active_card.contains("bee-hub__pulse"),
            "the active feature's card must carry the working-now pulse: {active_card}"
        );
        assert!(
            !other_card.contains("bee-hub__run-state"),
            "a DIFFERENT feature's card must never borrow the active feature's run_state badge: {other_card}"
        );
        assert!(
            !other_card.contains("bee-hub__pulse"),
            "a DIFFERENT feature's card must never borrow the active feature's liveness pulse: {other_card}"
        );

        assert!(
            active_card.contains("question") && active_card.contains("needs a call"),
            "the active feature's own deferred entry must render on its own card: {active_card}"
        );
        assert!(
            other_card.contains("cleanup") && other_card.contains("leftover code"),
            "the other feature's own deferred entry must render on its own card too, unrelated to activeness: {other_card}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // --- project-color-identity ---

    /// Ten distinct projects get ten distinct slots -- proving the palette
    /// is actually ten wide, not still five.
    #[test]
    fn cross_project_board_project_colors_ten_projects_get_ten_distinct_slots() {
        let ids: Vec<String> = (0..10).map(|n| format!("proj-{n:02}")).collect();
        let colors = bee_cross_project_board_project_colors(ids.iter().map(String::as_str));
        assert_eq!(
            colors.len(),
            10,
            "every one of the ten projects must land in the map: {colors:?}"
        );
        let slots: std::collections::HashSet<u8> = colors.values().copied().collect();
        assert_eq!(
            slots.len(),
            10,
            "ten distinct projects must land on ten distinct slots: {colors:?}"
        );
        for slot in slots {
            assert!(
                (1..=10).contains(&slot),
                "every slot must land in 1..=10: {slot}"
            );
        }
    }

    /// The regression case itself: three real board project ids that
    /// collided on the old five-bucket hash (`beedashboard` and `beehive`,
    /// with only three projects present) must land on three different
    /// slots under the widened, position-based map.
    #[test]
    fn cross_project_board_project_colors_three_real_ids_get_three_different_slots() {
        let colors = bee_cross_project_board_project_colors(
            ["anphabe-gogl", "beedashboard", "beehive"].into_iter(),
        );
        let slots: std::collections::HashSet<u8> = colors.values().copied().collect();
        assert_eq!(
            slots.len(),
            3,
            "three different projects on the board must get three different colours: {colors:?}"
        );
    }

    /// A project contributing nothing to the board never shifts another
    /// project's slot -- the map is built only over ids that actually
    /// place a feature, mirroring
    /// `cross_project_a_project_contributing_no_features_changes_nothing`'s
    /// own rendered-output proof at the map level.
    #[test]
    fn cross_project_board_project_colors_sorted_position_decides_the_slot() {
        let colors = bee_cross_project_board_project_colors(["beta", "alpha", "gamma"].into_iter());
        assert_eq!(
            colors.get("alpha"),
            Some(&1),
            "sorted first must land on slot 1: {colors:?}"
        );
        assert_eq!(
            colors.get("beta"),
            Some(&2),
            "sorted second must land on slot 2: {colors:?}"
        );
        assert_eq!(
            colors.get("gamma"),
            Some(&3),
            "sorted third must land on slot 3: {colors:?}"
        );
    }

    /// A card rendered with a project label carries the project name and
    /// its worktree in the subtitle (not the slug), carries no chip row at
    /// all, and its shell carries the `bee-hub__shell--pN` modifier the
    /// caller handed it.
    #[test]
    fn bee_hub_card_with_project_label_shows_project_worktree_subtitle_and_shell_modifier() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "wt/hold-holder-attribution",
            reason: None,
            docs: None,
            project_label: Some("Project A"),
            project_color: Some(3),
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            card_html.contains(
                r#"<div class="bee-hub__project">Project A<span class="bee-hub__project-worktree"> / wt/hold-holder-attribution</span></div>"#
            ),
            "the project name and its worktree branch must render in the subtitle: {card_html}"
        );
        assert!(
            !card_html.contains("bee-hub__slug"),
            "the slug subtitle must not also render: {card_html}"
        );
        assert!(
            !card_html.contains("bee-hub__chips"),
            "the chip row must be gone entirely: {card_html}"
        );
        assert!(
            !card_html.contains("fg-chip"),
            "no chip markup of any kind may remain: {card_html}"
        );
        assert!(
            card_html.contains("bee-hub__shell--p3"),
            "the shell must carry the colour modifier the caller handed it: {card_html}"
        );
    }

    /// A title-less card with a project label must still gain the project
    /// subtitle -- the project name is never dropped just because the
    /// feature itself has no CONTEXT.md title.
    #[test]
    fn bee_hub_card_with_project_label_and_no_title_still_shows_project_subtitle() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "Main",
            reason: None,
            docs: None,
            project_label: Some("Project A"),
            project_color: Some(3),
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        // card-collapse-inprogress: the title (now in the collapsed
        // `<summary>`) and the subtitle (now in the expandable body) are no
        // longer adjacent in the rendered markup, so each is checked on its
        // own rather than as one contiguous string.
        assert!(
            card_html.contains(r#"<div class="fg-card__title">feat-a</div>"#),
            "a title-less card must still render its slug as the card's own name: {card_html}"
        );
        assert!(
            card_html.contains(
                r#"<div class="bee-hub__project">Project A<span class="bee-hub__project-worktree"> / Main</span></div>"#
            ),
            "a title-less card with a project label must still carry the project subtitle: {card_html}"
        );
    }

    /// A card rendered with no project label keeps its colour and shell
    /// byte-identical to before this feature (no modifier class), but
    /// project-color-identity-3 now folds the worktree state into the same
    /// `bee-hub__slug` subtitle -- a per-project board still needs to say
    /// which of its own features has an open worktree even though it never
    /// shows a project name to hang that information off.
    #[test]
    fn bee_hub_card_with_no_project_label_shows_slug_and_worktree_subtitle() {
        let docs = waggledance_core::bee::BeeFeatureDocs {
            title: Some("Human Title".to_string()),
            description: None,
            docs: vec![],
        };
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "wt/hold-holder-attribution",
            reason: None,
            docs: Some(&docs),
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert_eq!(
            card_html,
            r#"<div class="fg-card bee-hub__shell"><details class="bee-hub__card" data-hub-group="in-progress"><summary class="bee-hub__summary"><div class="fg-card__title">Human Title</div><span class="bee-hub__chev" aria-hidden="true">›</span></summary><div class="bee-hub__body"><a class="bee-hub__detail-link" href="/p/proj-a/_bee/feature/feat-a">Feature detail<span aria-hidden="true"> →</span></a><div class="bee-hub__slug">feat-a<span class="bee-hub__project-worktree"> / wt/hold-holder-attribution</span></div><div class="bee-progress"><div class="bee-progress__bar" style="width: 50%"></div></div><p class="bee-hub__progress-label">1/2 cells done</p><p class="bee-cell__meta">No activity recorded.</p></div></details></div>"#,
            "a card with no project label must keep the byte-identical shell/colour and now carry its worktree state in the slug subtitle: {card_html}"
        );
        assert!(
            !card_html.contains("bee-hub__shell--p"),
            "no project label must mean no colour modifier: {card_html}"
        );
        assert!(
            !card_html.contains("bee-hub__chips"),
            "the chip row must be gone entirely: {card_html}"
        );
        assert!(
            !card_html.contains("bee-hub__project\""),
            "the project subtitle class must not render here: {card_html}"
        );
    }

    /// (hub-fallbacks, project-color-identity-4) A title-less card with no
    /// project label keeps its subtitle for the worktree state alone: its
    /// own title is already the slug, so a second copy beneath it would be
    /// a redundant subtitle -- but the worktree state is the one thing the
    /// card cannot say anywhere else, so it renders with no slug half and
    /// so no separator.
    #[test]
    fn bee_hub_card_with_no_project_label_and_no_title_names_its_worktree_alone() {
        let card_html = bee_hub_card(&BeeHubCardArgs {
            project_id: "proj-a",
            feature: "feat-a",
            group_key: "in-progress",
            done: 1,
            total: 2,
            last_activity: None,
            worktree_label: "merged",
            reason: None,
            docs: None,
            project_label: None,
            project_color: None,
            panes: &[],
            run_state: None,
            waiting_on_live: false,
            last_tool_call: None,
            deferred: &[],
        });
        assert!(
            card_html.contains(
                r#"<div class="bee-hub__slug"><span class="bee-hub__project-worktree">merged</span></div>"#
            ),
            "a title-less card must name its worktree state with no slug half: {card_html}"
        );
        assert_eq!(
            card_html.matches("feat-a<").count(),
            1,
            "the slug is already the title, so it must not be printed a second time: {card_html}"
        );
        assert!(
            !card_html.contains(" / "),
            "with no slug half there is nothing to separate from: {card_html}"
        );
    }

    /// All four worktree-line spellings [`bee_hub_worktree_label`] can hand
    /// `bee_hub_card` render through the slug subtitle unchanged, on the
    /// per-project board's own `project_label: None` path.
    #[test]
    fn bee_hub_card_with_no_project_label_renders_all_four_worktree_spellings() {
        let docs = waggledance_core::bee::BeeFeatureDocs {
            title: Some("Human Title".to_string()),
            description: None,
            docs: vec![],
        };
        for worktree_label in ["hold-holder-attribution", "worktree", "merged", "Main"] {
            let card_html = bee_hub_card(&BeeHubCardArgs {
                project_id: "proj-a",
                feature: "feat-a",
                group_key: "in-progress",
                done: 1,
                total: 2,
                last_activity: None,
                worktree_label,
                reason: None,
                docs: Some(&docs),
                project_label: None,
                project_color: None,
                panes: &[],
                run_state: None,
                waiting_on_live: false,
                last_tool_call: None,
                deferred: &[],
            });
            let expected =
                format!(r#"<span class="bee-hub__project-worktree"> / {worktree_label}</span>"#);
            assert!(
                card_html.contains(&expected),
                "the {worktree_label} spelling must render verbatim in the slug subtitle: {card_html}"
            );
        }
    }

    /// The Finished row for the same project carries the matching
    /// `bee-hub__row-project--pN` modifier on its project span, from the
    /// `project_color` the caller hands it.
    #[test]
    fn bee_hub_finished_row_with_project_label_carries_matching_color_modifier() {
        let row = bee_hub_finished_row(
            "finished",
            "/p/proj-a/_bee/feature/shipped-feat",
            "shipped-feat",
            None,
            Some("Project A"),
            Some(3),
            None,
        );
        assert!(
            row.contains(
                r#"<span class="bee-hub__row-project bee-hub__row-project--p3">Project A</span>"#
            ),
            "the Finished row's project span must carry the colour modifier it was handed: {row}"
        );
    }

    /// The style block carries the ten shell accent rules, the ten
    /// Finished-row accent rules, the border rule and the muted
    /// worktree-half rule.
    #[test]
    fn bee_hub_style_carries_the_ten_accent_rules_and_the_border_rule() {
        let css = bee_hub_style();
        for n in 1..=10 {
            assert!(
                css.contains(&format!(
                    ".bee-hub__shell--p{n} {{ --project-accent: var(--bee-hub-project-{n}); }}"
                )),
                "missing shell accent rule for p{n}: {css}"
            );
            assert!(
                css.contains(&format!(
                    ".bee-hub__row-project--p{n} {{ --project-accent: var(--bee-hub-project-{n}); }}"
                )),
                "missing Finished-row accent rule for p{n}: {css}"
            );
        }
        assert!(
            css.contains(r#".bee-hub__shell[class*="bee-hub__shell--p"] { border-left: var(--border-width-strong) solid var(--project-accent); }"#),
            "missing the shell's own left border rule: {css}"
        );
        assert!(
            css.contains(
                r#".bee-hub__project, .bee-hub__row-project { color: var(--project-accent); }"#
            ),
            "missing the subtitle/Finished-span colour rule: {css}"
        );
        assert!(
            css.contains(".bee-hub__project-worktree {"),
            "missing the project line's own muted worktree-half rule: {css}"
        );
    }

    /// inprogress-priority-order-2 (D1, must-have): the In Progress
    /// ordering rule lives INSIDE the existing `@media (max-width: 700px)`
    /// block, reuses that same breakpoint (no new one), and never leaks
    /// outside it -- a wide-screen render must carry no `order` rule at
    /// all.
    #[test]
    fn bee_hub_style_puts_in_progress_order_rule_only_inside_the_narrow_media_query() {
        let css = bee_hub_style();
        let media_at = css.find("@media (max-width: 700px)").unwrap_or_else(|| {
            panic!("the existing narrow-screen breakpoint must still exist: {css}")
        });
        assert!(
            css.matches("@media (max-width: 700px)").count() == 1,
            "D1 must reuse the existing breakpoint, never add a second one: {css}"
        );
        let media_end = css[media_at..]
            .find("</style>")
            .map(|i| media_at + i)
            .unwrap_or(css.len());
        let media_block = &css[media_at..media_end];
        assert!(
            media_block.contains(r#"[data-hub-group="in-progress"]"#) && media_block.contains("order: -1;"),
            "the In Progress group must be selected by its existing data-hub-group attribute and moved to order -1 inside the media query: {media_block}"
        );
        let before_media = &css[..media_at];
        let after_media = &css[media_end..];
        assert!(
            !before_media.contains("order: -1;") && !after_media.contains("order: -1;"),
            "the order rule must never appear outside the max-width: 700px block, so wide-screen rendering is unchanged: {css}"
        );
    }

    /// The four worktree-line spellings [`bee_hub_worktree_label`] must
    /// carry: an open worktree with a known branch reads the branch name
    /// alone (no chip's `Open · ` prefix).
    #[test]
    fn bee_hub_worktree_label_reads_the_branch_name_alone_for_an_open_worktree() {
        let grant = BeeWorktree {
            id: "g-1".to_string(),
            resolved: true,
            unresolved_reason: None,
            feature: Some("wt-feat".to_string()),
            phase: Some("swarming".to_string()),
            mode: Some("standard".to_string()),
            branch: Some("wt/hold-holder-attribution".to_string()),
            created_at: None,
            live: false,
            heartbeat_age_minutes: None,
        };
        let label = bee_hub_worktree_label("wt-feat", std::slice::from_ref(&grant), &[], false);
        assert_eq!(label, "wt/hold-holder-attribution");
    }

    /// An open worktree with no branch reads `"worktree"`.
    #[test]
    fn bee_hub_worktree_label_reads_worktree_for_an_open_worktree_with_no_branch() {
        let grant = BeeWorktree {
            id: "g-1".to_string(),
            resolved: true,
            unresolved_reason: None,
            feature: Some("wt-feat".to_string()),
            phase: Some("swarming".to_string()),
            mode: Some("standard".to_string()),
            branch: None,
            created_at: None,
            live: false,
            heartbeat_age_minutes: None,
        };
        let label = bee_hub_worktree_label("wt-feat", std::slice::from_ref(&grant), &[], false);
        assert_eq!(label, "worktree");
    }

    /// A merged worktree (no live grant, but a surviving workspace record
    /// naming this feature's own `wt/<feature>` branch, on a finished
    /// feature) reads `"merged"`.
    #[test]
    fn bee_hub_worktree_label_reads_merged_when_finished_and_a_grant_history_exists() {
        let workspace = sample_workspace("wt/shipped-feat");
        let label =
            bee_hub_worktree_label("shipped-feat", &[], std::slice::from_ref(&workspace), true);
        assert_eq!(label, "merged");
    }

    /// Every other case -- no grant, no grant history, or not finished --
    /// reads `"Main"`.
    #[test]
    fn bee_hub_worktree_label_reads_main_otherwise() {
        assert_eq!(bee_hub_worktree_label("solo-feat", &[], &[], true), "Main");
        assert_eq!(bee_hub_worktree_label("solo-feat", &[], &[], false), "Main");
    }

    /// (hub-finished-compact, integration) The paging arithmetic itself is
    /// proven at the unit level above; this proves `bee_feature_hub_section`
    /// really wires the Finished group's rows through
    /// `bee_hub_finished_rows` end to end, rather than dumping every
    /// archived feature into the open flow.
    #[test]
    fn hub_finished_group_pages_more_than_ten_archived_features_behind_a_details() {
        let root = std::env::temp_dir().join(format!(
            "waggledance-views-hub-finished-paged-{}",
            std::process::id()
        ));
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

        let snapshot = waggledance_core::bee::read_snapshot(&root);
        let mut project = sample_project();
        project.root_path = root.clone();
        let html = bee_feature_hub_section(&project, &snapshot, &std::collections::HashMap::new());

        assert!(
            html.contains(r#"data-hub-group="finished" data-hub-count="12""#),
            "{html}"
        );
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

    /// waggledance-rename D5/W5: the display brand shown on every rendered
    /// page is "Waggle Dance", not the old "Bee Artifact" or the binary
    /// name `mdview`/`waggledance`.
    #[test]
    fn rendered_page_title_says_waggle_dance() {
        let html = layout("My Doc", "", "<p>body</p>");
        assert!(
            html.contains("<title>My Doc · Waggle Dance</title>"),
            "{html}"
        );
    }

    /// review-p1-fixes D2: `layout()` interpolates the title raw into
    /// `<title>{title} · Waggle Dance</title>`. A title carrying a
    /// `</title><script>` payload must not be able to close the `<title>`
    /// element and open a live `<script>` — the single sink (`esc()` inside
    /// `layout()`) must catch every caller, including the ~12 that pass
    /// unescaped strings (search query, doc H1, cell id, pbi title, feature,
    /// project name, code-view paths).
    #[test]
    fn layout_escapes_a_title_that_tries_to_break_out_of_the_title_element() {
        let payload = "</title><script>alert(1)</script>";
        let html = layout(payload, "", "<p>body</p>");
        assert!(
            !html.contains("</title><script>"),
            "the raw payload must not survive unescaped: {html}"
        );
        assert!(
            !html.contains("<script>alert(1)</script>"),
            "no live <script> tag must be reachable through the title: {html}"
        );
        assert!(
            html.contains("&lt;/title&gt;&lt;script&gt;alert(1)&lt;/script&gt;"),
            "the payload's angle brackets must be escaped: {html}"
        );
        // The literal suffix layout() appends is never escaped.
        assert!(html.contains("· Waggle Dance</title>"), "{html}");
    }

    /// D2: `&` and `"` in a title must also be escaped — `esc()` covers
    /// `< > & "` and the title sits in an RCDATA context where a stray `&`
    /// could start a malformed entity and a stray `"` could break an
    /// attribute if the title were ever reused elsewhere.
    #[test]
    fn layout_escapes_ampersand_and_quote_in_title() {
        let html = layout(r#"Fish & Chips "deluxe""#, "", "<p>body</p>");
        assert!(
            html.contains("<title>Fish &amp; Chips &quot;deluxe&quot; · Waggle Dance</title>"),
            "{html}"
        );
    }

    /// D2: a plain title with nothing to escape renders unchanged, so the
    /// new `esc()` call is a no-op on the common case.
    #[test]
    fn layout_leaves_a_plain_title_unchanged() {
        let html = layout("Projects", "", "<p>body</p>");
        assert!(
            html.contains("<title>Projects · Waggle Dance</title>"),
            "{html}"
        );
    }

    /// D2 reflected path: `search_page` builds its title from the raw query
    /// (`format!("search: {query}")`) and never pre-escapes it — the sink
    /// inside `layout()` must be the one thing standing between a search
    /// query and a broken-out `<script>`.
    #[test]
    fn search_page_title_is_escaped_by_the_layout_sink() {
        let project = sample_project();
        let html = search_page(&project, "</title><script>", &[]);
        assert!(
            !html.contains("</title><script>"),
            "the raw query must not survive unescaped into the title: {html}"
        );
        assert!(
            html.contains("<title>search: &lt;/title&gt;&lt;script&gt; · Waggle Dance</title>"),
            "{html}"
        );
    }

    /// waggledance-rename D5: renaming `mdview-theme` / `mdview-folders-open`
    /// to `waggledance-*` must not silently drop the theme or folder state a
    /// user already has stored under the old key — so a one-shot fallback
    /// (read old, write new, delete old) survives in exactly two places:
    /// the no-flash theme reader this function inlines into every page, and
    /// the folder-open readers in `assets/app.js`. Nowhere else may the
    /// string "mdview" appear in what actually reaches a browser.
    #[test]
    fn served_html_and_js_never_mention_mdview_outside_the_storage_fallback() {
        let html = layout("t", "", "<p>body</p>");
        let sources: [(&str, &str); 2] =
            [("layout() HTML", html.as_str()), ("assets/app.js", APP_JS)];
        for (label, text) in sources {
            for line in text.lines() {
                if line.contains("mdview") {
                    assert!(
                        line.contains("mdview-theme") || line.contains("mdview-folders-open"),
                        "{label} has a stray 'mdview' outside the one-shot storage fallback: {line}"
                    );
                }
            }
        }
        assert!(
            !APP_CSS.contains("mdview"),
            "APP_CSS must not mention 'mdview' at all — it has no storage fallback"
        );
    }

    /// waggledance-rename W5: `mdview:mermaid-done` was renamed on both the
    /// dispatch side (`file_page`'s embedded script) and the listener side
    /// (`assets/app.js`). Comparing the two sides to each other — instead of
    /// each to its own hardcoded copy of the new name — is what would have
    /// caught a rename that only touched one of them.
    #[test]
    fn mermaid_done_event_name_matches_between_dispatch_and_listener() {
        let project = sample_project();
        let file = IndexedFile {
            project_id: project.id.clone(),
            abs_path: std::path::PathBuf::from("/tmp/proj-1/a.md"),
            rel_path: "a.md".into(),
            title: "A".into(),
            size_bytes: 10,
            modified_at: "2026-08-05T00:00:00Z".into(),
        };
        let page = RenderedPage {
            html: "<p>x</p>".into(),
            title: "A".into(),
            headings: vec![],
            has_mermaid: true,
            source: "x".into(),
        };
        let html = file_page(&project, &file, &page, std::slice::from_ref(&file), &[]);

        let dispatch_marker = "document.dispatchEvent(new Event('";
        let dispatch_start = html
            .find(dispatch_marker)
            .expect("mermaid dispatch site must render when has_mermaid is true")
            + dispatch_marker.len();
        let dispatch_end = html[dispatch_start..]
            .find('\'')
            .expect("unterminated dispatched event name")
            + dispatch_start;
        let dispatch_event = &html[dispatch_start..dispatch_end];

        // Anchored on the shared callback name, `enhanceAll`, never on the
        // event name itself — so this test cannot pass by hardcoding the
        // new name on both sides.
        let listen_suffix = "\", enhanceAll);";
        let listen_end = APP_JS
            .find(listen_suffix)
            .expect("mermaid listener 'addEventListener(\"...\", enhanceAll);' must be present");
        let listen_marker = "document.addEventListener(\"";
        let listen_start = APP_JS[..listen_end]
            .rfind(listen_marker)
            .expect("listener must be wired via document.addEventListener(\"...\", enhanceAll)")
            + listen_marker.len();
        let listen_event = &APP_JS[listen_start..listen_end];

        assert_eq!(
            dispatch_event, listen_event,
            "the mermaid-done event name must be identical on both sides of the handshake"
        );
    }
}
