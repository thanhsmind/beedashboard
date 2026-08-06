//! Server-rendered HTML views. Self-contained: layout + CSS + JS as consts.
//! Theme is CSS-variable driven (no-flash head script); code colors come from
//! `/highlight.css` (syntect class-based), so themes switch without re-render.

use mdview_core::bee::{
    BeeAttentionItem, BeeAttentionSeverity, BeeBacklog, BeeBuckets, BeeCell, BeeLane,
    BeeRunningWorker, BeeSession, BeeShippedFeature, BeeSnapshot, BeeState, BeeWorkspace,
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
<title>{title} · mdview</title>
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
<link rel="stylesheet" href="/static/app.css">
<link rel="stylesheet" href="/highlight.css">
{head_extra}
</head>
<body>
{body}
<script src="/static/app.js"></script>
</body>
</html>"#
    )
}

/// `unassigned_visible` is D5/D4's presence marker, never contents: `true`
/// exactly when the D7 `terminal.enabled` switch is on (checked with no
/// herdr call and no session, so this unauthenticated page never learns
/// whether any pane is actually unassigned) — renders a link to
/// `/_terminal/unassigned`, whose own route is gated like every other
/// terminal route. `false` (the default) renders this page byte-identical
/// to how it looked before this feature existed.
pub fn project_list_page(projects: &[(Project, usize)], unassigned_visible: bool) -> String {
    let listing = if projects.is_empty() {
        "<p class=\"fg-empty\">Chưa có project nào. Đăng ký: <code>mdview register &lt;dir&gt;</code> hoặc gọi MCP <code>mdview_view_file</code>.</p>".to_string()
    } else {
        // Cards (not a table — cards read better on phones/tablets). Each card is
        // a clickable link to the project plus a delete control that unregisters
        // it. The filesystem path is deliberately omitted (unauthenticated page).
        let mut cards = String::new();
        for (p, count) in projects {
            cards.push_str(&format!(
                r#"<div class="proj-card">
  <a class="fg-card proj-card__link" href="/p/{id}/">
    <div class="fg-card__title">{name}</div>
    <div class="fg-card__sub">{count} markdown files · <time class="proj-card__time" datetime="{seen}">{seen}</time></div>
  </a>
  <form class="proj-card__delete" method="post" action="/api/projects/{id}/unregister" data-project="{name}">
    <button type="submit" class="proj-card__del" aria-label="Remove {name} from mdview" title="Remove from mdview">✕</button>
  </form>
</div>"#,
                id = esc(&p.id),
                name = esc(&p.name),
                count = count,
                seen = esc(&p.last_seen_at),
            ));
        }
        format!(r#"<div class="proj-cards">{cards}</div>"#, cards = cards)
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
    let body = format!(
        r#"{topbar}
<main class="fg-page"><h2 class="fg-pagehead__title">Projects</h2>{listing}{unassigned_card}</main>"#,
        topbar = topbar(""),
        listing = listing,
        unassigned_card = unassigned_card,
    );
    layout("Projects", "", &body)
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
  {tabs}
  <div class="proj-cards">
    {bee_card}
    {docs_card}
  </div>
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name}</span>",
            name = esc(&project.name)
        )),
        tab_style = PROJECT_TAB_STYLE,
        name = esc(&project.name),
        tabs = project_tabs(&project.id, "overview"),
        bee_card = bee_card,
        docs_card = docs_card,
    );
    layout(&project.name, "", &body)
}

/// Inline styling for [`project_tabs`] — kept beside the pages that render
/// it (same precedent as `bee_board_page`'s own inline `<style>`), not added
/// to `app.css`: this cell's declared files are `server.rs`/`views.rs` only.
const PROJECT_TAB_STYLE: &str = r#"<style>
.proj-tabs { display: flex; gap: var(--space-4); margin-bottom: var(--space-4); border-bottom: var(--border-width-hairline) solid var(--color-border); }
.proj-tab { padding: var(--space-2) 0; color: var(--color-text-muted); text-decoration: none; border-bottom: 2px solid transparent; }
.proj-tab--active { color: var(--color-text); border-color: var(--color-action); font-weight: var(--weight-semibold); }
.term-pane__cwd { color: var(--color-text-subtle); font-size: var(--type-caption-size); word-break: break-word; }
.term-pane__meta { color: var(--color-text-muted); font-size: var(--type-body-sm-size); }
.term-screen { margin-top: var(--space-2); padding: var(--space-2); background: var(--color-surface-sunken, var(--color-bg-subtle)); border-radius: var(--radius-sm); white-space: pre-wrap; word-break: break-word; font-family: var(--font-mono, monospace); font-size: var(--type-body-sm-size); max-height: 24em; overflow-y: auto; }
.term-reply { display: flex; gap: var(--space-2); margin-top: var(--space-2); }
.term-reply__text { flex: 1; min-width: 0; padding: var(--space-1) var(--space-2); border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--radius-sm); font-family: var(--font-mono, monospace); font-size: var(--type-body-sm-size); background: var(--color-bg); color: var(--color-text); }
.term-reply__send, .term-reply__stage { padding: var(--space-1) var(--space-2); border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--radius-sm); background: var(--color-bg-subtle); color: var(--color-text); cursor: pointer; }
.term-keys { display: flex; flex-wrap: wrap; gap: var(--space-1); margin-top: var(--space-2); }
.term-keys button { padding: var(--space-1) var(--space-2); border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--radius-sm); background: var(--color-bg-subtle); color: var(--color-text); cursor: pointer; font-size: var(--type-caption-size); }
.term-transcript { margin-top: var(--space-2); padding: var(--space-2); background: var(--color-surface-sunken, var(--color-bg-subtle)); border-radius: var(--radius-sm); font-family: var(--font-mono, monospace); font-size: var(--type-body-sm-size); max-height: 24em; overflow-y: auto; }
.term-transcript__line { white-space: pre-wrap; word-break: break-word; }
</style>"#;

/// D6: the Terminal tab is always present on a project page, whether or not
/// herdr is running or the terminal has ever been reached — this renders
/// from the project id alone, with no herdr call and no auth check, so its
/// presence never depends on either. `active` is `"overview"`, `"terminal"`
/// or `"transcript"` (agent-terminal-16, D9: the Transcript tab sits beside
/// Terminal, not inside its frame).
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
/// type crosses into this module.
pub struct TerminalPaneView {
    pub pane_id: String,
    pub kind: String,
    pub name: String,
    pub status: String,
    pub title: String,
    pub cwd: String,
}

/// Shared by [`terminal_page`] and [`unassigned_terminal_page`]: one pane's
/// card — screen viewport, reply form, key buttons — the exact widget set
/// `assets/app.js`'s project-scoped poller/handlers drive. `empty_msg` is
/// rendered instead when `panes` is empty, kept distinct per caller so an
/// empty project and an empty Unassigned group are never confusable with
/// each other, or with [`terminal_down_page`]'s herdr-silent wording.
fn pane_cards(panes: &[TerminalPaneView], empty_msg: &str) -> String {
    if panes.is_empty() {
        return format!(r#"<p class="fg-empty">{}</p>"#, esc(empty_msg));
    }
    let mut out = String::new();
    for p in panes {
        out.push_str(&format!(
            r#"<div class="fg-card term-pane" data-pane-id="{pane_id}">
  <div class="fg-card__title">{name} <span class="fg-chip fg-chip--neutral">{status}</span></div>
  <div class="term-pane__meta">{kind}{title_sep}{title}</div>
  <div class="term-pane__cwd">{cwd}</div>
  <pre class="term-screen" data-pane-id="{pane_id}" aria-live="polite">Loading screen…</pre>
  <form class="term-reply" data-pane-id="{pane_id}">
    <input type="text" class="term-reply__text" placeholder="Type a reply…" aria-label="Reply to {name}" autocomplete="off">
    <button type="submit" class="term-reply__send">Send</button>
    <button type="button" class="term-reply__stage">Stage</button>
  </form>
  <div class="term-keys" data-pane-id="{pane_id}" aria-label="Send a key to {name}">
    <button type="button" data-key="up">↑</button>
    <button type="button" data-key="down">↓</button>
    <button type="button" data-key="left">←</button>
    <button type="button" data-key="right">→</button>
    <button type="button" data-key="enter">Enter</button>
    <button type="button" data-key="escape">Esc</button>
    <button type="button" data-key="tab">Tab</button>
  </div>
</div>"#,
            pane_id = esc(&p.pane_id),
            name = esc(&p.name),
            status = esc(&p.status),
            kind = esc(&p.kind),
            title_sep = if p.title.is_empty() { "" } else { " · " },
            title = esc(&p.title),
            cwd = esc(&p.cwd),
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

/// `GET /p/:id/_terminal` up state (D2/D6): the project-scoped pane list.
/// Zero panes renders a named empty state, not a blank page — distinct
/// wording from [`terminal_down_page`] so an empty list is never mistaken
/// for herdr being unreachable, or the reverse. `presets` is the exact
/// configured D8 preset label list (`mdview_core::config::AgentPreset`'s
/// labels, in `Config.terminal.agent_presets` order) — this view never sees
/// argv.
pub fn terminal_page(project: &Project, panes: &[TerminalPaneView], presets: &[String]) -> String {
    let rows = pane_cards(panes, "No agents are running under this project right now.");
    // `data-project-id` lets `assets/app.js`'s screen poller build each
    // pane's `/p/:id/_terminal/:pane_id/screen` URL without threading the id
    // through every `.term-screen` element individually.
    let body = format!(
        r#"{topbar}
{tab_style}
<main class="fg-page" data-project-id="{pid}">
  <h2 class="fg-pagehead__title">{name}</h2>
  {tabs}
  {create}
  <div class="term-panes">{rows}</div>
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name} · terminal</span>",
            name = esc(&project.name)
        )),
        tab_style = PROJECT_TAB_STYLE,
        pid = esc(&project.id),
        name = esc(&project.name),
        tabs = project_tabs(&project.id, "terminal"),
        create = terminal_create_controls(&project.id, presets),
        rows = rows,
    );
    layout(&format!("{} · terminal", project.name), "", &body)
}

/// One pane's transcript card (agent-terminal-16, D9): the same
/// identity/meta header [`pane_cards`] renders for the screen, with a
/// `.term-transcript` viewport in place of `.term-screen`, `.term-reply` and
/// `.term-keys` — this tab is read-only. `assets/app.js`'s transcript poller
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
  <div class="fg-card__title">{name} <span class="fg-chip fg-chip--neutral">{status}</span></div>
  <div class="term-pane__meta">{kind}{title_sep}{title}</div>
  <div class="term-pane__cwd">{cwd}</div>
  <div class="term-transcript" data-pane-id="{pane_id}" aria-live="polite">Loading activity…</div>
</div>"#,
            pane_id = esc(&p.pane_id),
            name = esc(&p.name),
            status = esc(&p.status),
            kind = esc(&p.kind),
            title_sep = if p.title.is_empty() { "" } else { " · " },
            title = esc(&p.title),
            cwd = esc(&p.cwd),
        ));
    }
    out
}

/// `GET /p/:id/_transcript` up state (D9): the Transcript tab beside
/// Terminal, not a toggle inside its frame — the same project-scoped,
/// D2-boundary-filtered pane list `terminal_page` builds, rendered with a
/// transcript viewport per pane instead of a screen. Zero panes renders the
/// same wording `terminal_page` uses for the same reason (never mistaken for
/// herdr being unreachable, see [`terminal_down_page`], which this tab's
/// down state also reuses — listing which panes belong to this project
/// still needs a herdr snapshot even though transcript content itself
/// doesn't).
pub fn transcript_page(project: &Project, panes: &[TerminalPaneView]) -> String {
    let rows = transcript_cards(panes, "No agents are running under this project right now.");
    // `data-project-id` lets `assets/app.js`'s transcript poller build each
    // pane's `/p/:id/_terminal/:pane_id/transcript` URL, mirroring the
    // screen poller's own `data-project-id` use on `terminal_page`.
    let body = format!(
        r#"{topbar}
{tab_style}
<main class="fg-page" data-project-id="{pid}">
  <h2 class="fg-pagehead__title">{name}</h2>
  {tabs}
  <div class="term-panes">{rows}</div>
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name} · transcript</span>",
            name = esc(&project.name)
        )),
        tab_style = PROJECT_TAB_STYLE,
        pid = esc(&project.id),
        name = esc(&project.name),
        tabs = project_tabs(&project.id, "transcript"),
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
        if (!res.ok) { el.textContent = HERDR_DOWN_TEXT; return null; }
        return res.json();
      })
      .then(function (body) {
        if (!body) return;
        if (lastRevision[paneId] === body.revision) return;
        lastRevision[paneId] = body.revision;
        // `body.text` is safe, pre-escaped HTML from mdview-core's ansi
        // translator (agent-terminal-12) — never the raw pane text — so
        // `innerHTML` here renders ANSI colour/attribute markup rather than
        // showing literal escape characters.
        el.innerHTML = body.text;
      })
      .catch(function () { el.textContent = HERDR_DOWN_TEXT; });
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
    let rows = pane_cards(panes, "No agents are running outside a registered project right now.");
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
    <div class="term-pane__meta">Start herdr, then reload this page — mdview does not start it for you unless the herdr supervisor is switched on in Settings.</div>
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
  {tabs}
  <div class="fg-card term-pane">
    <div class="fg-card__title">herdr is not running</div>
    <div class="term-pane__meta">Start herdr, then reload this page — mdview does not start it for you unless the herdr supervisor is switched on in Settings.</div>
  </div>
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name} · terminal</span>",
            name = esc(&project.name)
        )),
        tab_style = PROJECT_TAB_STYLE,
        name = esc(&project.name),
        tabs = project_tabs(&project.id, "terminal"),
    );
    layout(&format!("{} · terminal", project.name), "", &body)
}

/// The read-only bee cell board (D4/D7): the project's four cell buckets,
/// each rendered by `bee_bucket_section`. Every path-shaped value on a
/// `BeeCell` already arrives relativized by `mdview_core::bee::read_snapshot`
/// (no absolute path crosses into `BeeSnapshot`'s public fields), so nothing
/// further is redacted here — this view only escapes for HTML safety.
pub fn bee_board_page(project: &Project, snapshot: &BeeSnapshot) -> String {
    let b = &snapshot.buckets;

    let body = format!(
        r#"{topbar}
<style>
.bee-buckets {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: var(--space-4); }}
.bee-buckets-top {{ display: grid; grid-template-columns: repeat(3, 1fr); gap: var(--space-4); margin-bottom: var(--space-4); }}
.bee-done {{ margin-bottom: var(--space-4); }}
.bee-done-summary {{ cursor: pointer; list-style: none; padding: var(--space-2) 0; font-weight: var(--weight-strong); color: var(--color-text); }}
.bee-done-summary::-webkit-details-marker {{ display: none; }}
.bee-done-grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: var(--space-2); padding-top: var(--space-2); }}
.bee-done-line {{ display: block; color: var(--color-text-muted); font-size: var(--type-caption-size); text-decoration: none; padding: var(--space-1) 0; border-bottom: var(--border-width-hairline) solid var(--color-border); }}
.bee-done-line:hover {{ color: var(--color-action); }}
.bee-bucket--danger {{ border-color: var(--color-danger); background: var(--color-danger-tint); }}
.bee-bucket__head {{ display: flex; align-items: center; gap: var(--space-2); margin: 0; }}
.bee-cell {{ padding: var(--space-2); gap: var(--space-1); }}
.bee-cell .fg-card__title {{ font-size: var(--type-body-sm-size); }}
.bee-cell__meta {{ color: var(--color-text-subtle); font-size: var(--type-caption-size); word-break: break-word; }}
.bee-velocity {{ margin-bottom: var(--space-4); }}
.bee-velocity__head {{ margin: 0 0 var(--space-3) 0; }}
.bee-stats {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: var(--space-3); margin-bottom: var(--space-4); }}
.bee-stat {{ padding: var(--space-3); align-items: flex-start; gap: var(--space-1); }}
.bee-stat__value {{ font-family: var(--type-heading-font); font-size: var(--type-figure-lg-size); line-height: var(--type-figure-lg-leading); }}
.bee-stat--empty .bee-stat__value {{ color: var(--color-text-subtle); }}
.bee-stat__label {{ color: var(--color-text-subtle); font-size: var(--type-caption-size); }}
.bee-velocity__lists {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: var(--space-4); }}
.bee-velocity__subhead {{ margin: 0 0 var(--space-2) 0; font-size: var(--type-heading-sm-size); }}
.bee-velocity__open-list {{ margin: 0; padding-left: var(--space-4); color: var(--color-text-subtle); }}
.bee-panels {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: var(--space-4); margin-top: var(--space-4); }}
.bee-panel__head {{ display: flex; align-items: center; gap: var(--space-2); margin: 0; }}
.bee-panel__subhead {{ margin: var(--space-3) 0 var(--space-2) 0; font-size: var(--type-heading-sm-size); }}
.bee-panel__chips {{ display: flex; flex-wrap: wrap; gap: var(--space-2); margin-bottom: var(--space-2); }}
.bee-panel__list {{ display: flex; flex-direction: column; gap: var(--space-2); }}
.bee-severity--p1 {{ font-weight: var(--weight-strong); }}
.bee-asof {{ color: var(--color-text-subtle); font-size: var(--type-body-sm-size); }}
.bee-stepper {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: var(--space-3); list-style: none; margin: 0 0 var(--space-4) 0; padding: 0; }}
.bee-step {{ display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-3); border: var(--border-width-hairline) solid var(--color-border); border-radius: var(--card-radius); background: var(--color-surface); }}
.bee-step__mark {{ display: inline-flex; align-items: center; justify-content: center; width: 22px; height: 22px; border-radius: var(--radius-pill); font-size: var(--type-caption-size); font-weight: var(--weight-strong); background: var(--color-surface-sunken); color: var(--color-text-subtle); }}
.bee-step__label {{ font-weight: var(--weight-strong); color: var(--color-text); }}
.bee-step__note {{ color: var(--color-text-subtle); font-size: var(--type-caption-size); }}
.bee-step--done .bee-step__mark {{ background: var(--color-success-tint); color: var(--color-success); }}
.bee-step--current {{ border-color: var(--color-action); }}
.bee-step--current .bee-step__mark {{ background: var(--color-info-tint); color: var(--color-info); }}
.bee-now-grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: var(--space-4); margin-bottom: var(--space-4); }}
.bee-working-now {{ display: flex; flex-direction: column; gap: var(--space-2); }}
.bee-progress {{ height: 8px; border-radius: var(--radius-pill); background: var(--color-surface-sunken); overflow: hidden; }}
.bee-progress__bar {{ height: 100%; background: var(--color-success); }}
.bee-next-action {{ padding: var(--space-2) var(--space-3); }}
.bee-attention__item--danger {{ border-color: var(--color-danger); background: var(--color-danger-tint); }}
.bee-attention__item--warning {{ border-color: var(--color-warning); background: var(--color-warning-tint); }}
.bee-attention__action {{ font-style: italic; }}
</style>
<main class="fg-page">
  {top}
  {running}
  {worktrees}
  {velocity}
  <div class="bee-buckets-top">
    {doing}
    {waiting}
    {stuck}
  </div>
  {done}
  {panels}
  {errors}
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name} · bee</span>",
            name = esc(&project.name)
        )),
        top = bee_board_top(project, snapshot),
        running = bee_running_now_section(&project.id, snapshot),
        worktrees = bee_worktree_section(&snapshot.worktrees),
        velocity = bee_velocity_section(&project.id, snapshot),
        doing = bee_bucket_section(&project.id, "Doing", "doing", &b.doing, "neutral", false),
        waiting = bee_bucket_section(&project.id, "Waiting", "waiting", &b.waiting, "neutral", false),
        stuck = bee_bucket_section(&project.id, "Stuck", "stuck", &b.stuck, "danger", false),
        done = bee_done_section(&project.id, &b.done, &snapshot.shipped),
        panels = bee_panels_section(snapshot),
        errors = bee_read_errors(&snapshot.read_errors),
    );
    layout(&format!("{} · bee", project.name), "", &body)
}

/// D5's fixed top-of-board order, rebuilt in this cell (bbp-5): a header
/// line naming the project and when this snapshot was read, then the
/// lifecycle stepper, the headline numbers, and finally "working on now"
/// beside "needs attention". Everything below this — `{running}` onward in
/// [`bee_board_page`] — is untouched, existing markup; this function only
/// replaces the old two-chip pagehead.
fn bee_board_top(project: &Project, snapshot: &BeeSnapshot) -> String {
    format!(
        r#"<div class="fg-pagehead">
    <h2 class="fg-pagehead__title">{name}</h2>
    <div class="fg-pagehead__aside"><span class="bee-asof">Read {asof}</span></div>
  </div>
  {stepper}
  {kpis}
  <div class="bee-now-grid">
    {working_now}
    {attention}
  </div>"#,
        name = esc(&project.name),
        asof = esc(&bee_board_asof()),
        stepper = bee_lifecycle_stepper(snapshot.state.as_ref()),
        kpis = bee_headline_kpis(snapshot),
        working_now = bee_working_now_card(&project.id, snapshot),
        attention = bee_attention_panel(&snapshot.attention),
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

/// The lifecycle stepper (D5/D7): the four gates `.bee/state.json` actually
/// tracks — context, shape, execution, review — each rendered as one step.
/// A step is `done` only when its gate is `approved_gates.<gate> ==
/// Some(true)` **and** it carries no `gate_revoked_at.<gate>` entry (a gate
/// approved and then revoked is not done); `current` is the first step not
/// done, in gate order. The review step's undone note is always the D7
/// wording — independent review is something a human invokes, never
/// automatic pending work — regardless of whether it was never approved or
/// was approved and then revoked. No `state.json` at all renders one honest
/// line rather than four steps all reading "not yet approved", which would
/// misstate "we have no record" as "record says no".
fn bee_lifecycle_stepper(state: Option<&BeeState>) -> String {
    let Some(state) = state else {
        return r#"<p class="fg-empty">No lifecycle data recorded yet.</p>"#.to_string();
    };

    const GATES: [(&str, &str); 4] = [
        ("context", "Explore"),
        ("shape", "Shape"),
        ("execution", "Execute"),
        ("review", "Independent review"),
    ];

    let approved_flag = |key: &str| -> bool {
        state
            .approved_gates
            .as_ref()
            .and_then(|g| match key {
                "context" => g.context,
                "shape" => g.shape,
                "execution" => g.execution,
                "review" => g.review,
                _ => None,
            })
            .unwrap_or(false)
    };
    let revoked_flag = |key: &str| -> bool {
        state
            .gate_revoked_at
            .as_ref()
            .and_then(|g| match key {
                "context" => g.context.as_deref(),
                "shape" => g.shape.as_deref(),
                "execution" => g.execution.as_deref(),
                "review" => g.review.as_deref(),
                _ => None,
            })
            .is_some()
    };

    let done: Vec<bool> = GATES
        .iter()
        .map(|(key, _)| approved_flag(key) && !revoked_flag(key))
        .collect();
    let current_idx = done.iter().position(|&d| !d);

    let mut items = String::new();
    for (i, (key, label)) in GATES.iter().enumerate() {
        let is_done = done[i];
        let is_current = current_idx == Some(i);
        let is_review = *key == "review";
        let was_revoked = approved_flag(key) && revoked_flag(key);

        let state_cls = if is_done {
            "bee-step--done"
        } else if is_current {
            "bee-step--current"
        } else {
            "bee-step--pending"
        };
        let mark = if is_done {
            "\u{2713}".to_string()
        } else if is_current {
            "\u{25b6}".to_string()
        } else {
            (i + 1).to_string()
        };
        let note = if is_done {
            "Approved".to_string()
        } else if is_review {
            "Runs only when you invoke it — never automatic.".to_string()
        } else if was_revoked {
            "Approved, then revoked.".to_string()
        } else {
            "Not yet approved.".to_string()
        };

        items.push_str(&format!(
            r#"<li class="bee-step {state_cls}" data-step="{key}"><span class="bee-step__mark">{mark}</span><span class="bee-step__label">{label}</span><span class="bee-step__note">{note}</span></li>"#,
            state_cls = state_cls,
            key = key,
            mark = esc(&mark),
            label = esc(label),
            note = esc(&note),
        ));
    }

    format!(r#"<ol class="bee-stepper">{items}</ol>"#, items = items)
}

/// The headline numbers row (D5): the four D7 bucket counts plus how many
/// features have shipped, reusing [`bee_stat_card`] so every tile matches
/// the ship-velocity stats below it. Each is a real count, never a "nothing
/// to measure" absence — a bucket genuinely holding zero cells is honest
/// data, not the missing-data case `bee_stat_card`'s `None` branch exists
/// for.
fn bee_headline_kpis(snapshot: &BeeSnapshot) -> String {
    let b = &snapshot.buckets;
    format!(
        r#"<div class="bee-stats">
    {doing}
    {waiting}
    {stuck}
    {done}
    {shipped}
  </div>"#,
        doing = bee_stat_card("Doing", Some(b.doing.len().to_string())),
        waiting = bee_stat_card("Waiting", Some(b.waiting.len().to_string())),
        stuck = bee_stat_card("Stuck", Some(b.stuck.len().to_string())),
        done = bee_stat_card("Done", Some(b.done.len().to_string())),
        shipped = bee_stat_card("Shipped features", Some(snapshot.shipped.len().to_string())),
    )
}

/// "Working on now" (D5): the active feature named by `state.feature` only
/// — never derived from cell data, which would risk naming the same
/// feature a second time next to the Done section below (see
/// `board_renders_finished_work_in_exactly_one_place`). Carries the route's
/// rationale, progress over this feature's own live (non-dropped) cells
/// with the numerator and denominator both drawn from the D7 buckets —
/// which already exclude `dropped` and unrecognized statuses (D8) — and the
/// recorded next action. No active feature, no route, no live cells or no
/// next action each render their own honest line rather than a fabricated
/// zero or a hidden section.
fn bee_working_now_card(project_id: &str, snapshot: &BeeSnapshot) -> String {
    let feature = snapshot.state.as_ref().and_then(|s| s.feature.as_deref());
    let Some(feature) = feature else {
        return r#"<section class="fg-card bee-panel bee-working-now">
  <h3 class="bee-panel__head">Working on now</h3>
  <p class="fg-empty">No feature is currently active.</p>
</section>"#
            .to_string();
    };

    let rationale = snapshot
        .state
        .as_ref()
        .and_then(|s| s.route.as_ref())
        .and_then(|r| r.rationale.as_deref())
        .filter(|r| !r.is_empty());
    let rationale_html = match rationale {
        Some(r) => format!(r#"<p class="bee-cell__meta">{}</p>"#, esc(r)),
        None => r#"<p class="fg-empty">No route rationale recorded.</p>"#.to_string(),
    };

    let doing = snapshot.buckets.doing.iter().filter(|c| c.feature == feature).count();
    let waiting = snapshot.buckets.waiting.iter().filter(|c| c.feature == feature).count();
    let stuck = snapshot.buckets.stuck.iter().filter(|c| c.feature == feature).count();
    let done = snapshot.buckets.done.iter().filter(|c| c.feature == feature).count();
    let total = doing + waiting + stuck + done;

    let progress_html = if total == 0 {
        r#"<p class="fg-empty">No live cells recorded for this feature yet.</p>"#.to_string()
    } else {
        let percent = (done * 100) / total;
        format!(
            r#"<div class="bee-progress"><div class="bee-progress__bar" style="width: {percent}%"></div></div><p class="bee-cell__meta">{done}/{total} cell{plural} done</p>"#,
            percent = percent,
            done = done,
            total = total,
            plural = if total == 1 { "" } else { "s" },
        )
    };

    let next_action = snapshot
        .state
        .as_ref()
        .and_then(|s| s.next_action.as_deref())
        .filter(|n| !n.is_empty());
    let next_action_html = match next_action {
        Some(n) => format!(
            r#"<div class="fg-card fg-card--sunken bee-next-action"><div class="fg-card__title">Next action</div><p>{}</p></div>"#,
            esc(n)
        ),
        None => r#"<p class="fg-empty">No next action recorded.</p>"#.to_string(),
    };

    format!(
        r#"<section class="fg-card bee-panel bee-working-now">
  <h3 class="bee-panel__head">Working on now · <a href="/p/{pid}/_bee/feature/{feature_href}">{feature}</a></h3>
  {rationale_html}
  {progress_html}
  {next_action_html}
</section>"#,
        pid = esc(project_id),
        feature_href = esc(feature),
        feature = esc(feature),
        rationale_html = rationale_html,
        progress_html = progress_html,
        next_action_html = next_action_html,
    )
}

/// A `BeeAttentionSeverity`'s chip tone — reuses the same `fg-chip--*`
/// tones the rest of the board already uses (`fg-chip--warning`,
/// `fg-chip--danger`), never inventing a new palette. `Serious` and
/// `Critical` share the `danger` tone; `Critical` additionally carries
/// `bee-severity--p1`'s bold weight (already used for P1 backlog findings)
/// so the heaviest items are visually heavier, not just first in order.
fn bee_attention_tone(sev: BeeAttentionSeverity) -> &'static str {
    match sev {
        BeeAttentionSeverity::Warning => "warning",
        BeeAttentionSeverity::Serious => "danger",
        BeeAttentionSeverity::Critical => "danger",
    }
}

/// The plain-English label for a `BeeAttentionSeverity`, shown inside its
/// chip (D1 — English labels throughout).
fn bee_attention_severity_label(sev: BeeAttentionSeverity) -> &'static str {
    match sev {
        BeeAttentionSeverity::Warning => "warning",
        BeeAttentionSeverity::Serious => "serious",
        BeeAttentionSeverity::Critical => "critical",
    }
}

/// D6's "needs attention" panel: `snapshot.attention` rendered verbatim, in
/// the order `compute_attention_items` (`mdview_core::bee`) already sorted
/// it — this view never recomputes or reorders the list, only formats it.
/// An empty list renders one honest line, never an empty bordered panel.
fn bee_attention_panel(items: &[BeeAttentionItem]) -> String {
    if items.is_empty() {
        return r#"<section class="fg-card bee-panel bee-attention">
  <h3 class="bee-panel__head">Needs attention</h3>
  <p class="fg-empty">Nothing needs attention right now.</p>
</section>"#
            .to_string();
    }

    let mut rows = String::new();
    for item in items {
        let tone = bee_attention_tone(item.severity);
        let title_cls = if item.severity == BeeAttentionSeverity::Critical {
            " bee-severity--p1"
        } else {
            ""
        };
        rows.push_str(&format!(
            r#"<div class="fg-card bee-cell bee-attention__item bee-attention__item--{tone}"><div class="fg-card__title{title_cls}"><span class="fg-chip fg-chip--{tone}">{sev_label}</span> {title}</div><div class="bee-cell__meta">{detail}</div><div class="bee-cell__meta bee-attention__action">{action}</div></div>"#,
            tone = tone,
            title_cls = title_cls,
            sev_label = bee_attention_severity_label(item.severity),
            title = esc(&item.title),
            detail = esc(&item.detail),
            action = esc(&item.suggested_action),
        ));
    }

    format!(
        r#"<section class="fg-card bee-panel bee-attention">
  <h3 class="bee-panel__head">Needs attention <span class="fg-chip fg-chip--neutral">{count}</span></h3>
  <div class="bee-panel__list">{rows}</div>
</section>"#,
        count = items.len(),
        rows = rows,
    )
}

/// "Running now" — what is actually in flight this instant, above the
/// velocity numbers: every live session (`.bee/sessions/*.json` reporting
/// within the last 30 minutes) with its heartbeat age in plain relative
/// language, plus every worker `state.json` names whose session is live
/// (`snapshot.running_workers`, already joined and session-verified by
/// `mdview_core::bee::read_snapshot`), each linking to the cell it names.
/// This is deliberately a *separate* surface from the D7 buckets below it:
/// bee's own docs call `state.json`'s `workers[]` hand-maintained and not
/// fully trusted, so a worker naming a cell the store still calls
/// open/waiting never moves that cell into Doing — it only earns an
/// explicit discrepancy note here, right where a person is already looking
/// for "what's running". When nothing is live at all, this renders one
/// quiet empty-state line instead of an empty bordered panel, so it never
/// costs space on a quiet board.
fn bee_running_now_section(project_id: &str, snapshot: &BeeSnapshot) -> String {
    let live_sessions: Vec<&BeeSession> = snapshot.sessions.iter().filter(|s| s.live).collect();
    let workers = &snapshot.running_workers;

    if live_sessions.is_empty() && workers.is_empty() {
        return r#"<p class="fg-empty">Nothing running right now.</p>"#.to_string();
    }

    let mut rows = String::new();
    for s in &live_sessions {
        let source = s.source.as_deref().unwrap_or("session");
        rows.push_str(&format!(
            r#"<div class="fg-card bee-cell"><div class="fg-card__title">{id}</div><div class="bee-cell__meta">{source} · {age}</div></div>"#,
            id = esc(&s.id),
            source = esc(source),
            age = esc(&bee_relative_minutes(s.heartbeat_age_minutes)),
        ));
    }
    for w in workers.iter() {
        rows.push_str(&bee_running_worker_row(project_id, w));
    }

    format!(
        r#"<section class="fg-card bee-panel bee-running">
  <h3 class="bee-panel__head">Running now</h3>
  <div class="bee-panel__list">{rows}</div>
</section>"#,
        rows = rows,
    )
}

/// One `bee_running_now_section` worker row: the cell it names (linked to
/// that cell's detail page when the cell was actually found; plain text
/// when it names a cell that does not exist, per must-have "flagged, not
/// dropped") plus, when the store disagrees with the running process
/// (`w.discrepancy`), an explicit note naming what the store still says.
fn bee_running_worker_row(project_id: &str, w: &BeeRunningWorker) -> String {
    let cell_ref = match (&w.cell, w.cell_found) {
        (Some(cid), true) => format!(
            r#"<a href="/p/{pid}/_bee/cell/{cid_href}">{cid}</a>"#,
            pid = esc(project_id),
            cid_href = esc(cid),
            cid = esc(cid),
        ),
        (Some(cid), false) => esc(cid),
        (None, _) => "no cell named".to_string(),
    };
    let discrepancy = if !w.discrepancy {
        String::new()
    } else {
        let note = match (&w.cell, w.cell_found, w.cell_status.as_deref()) {
            (Some(cid), true, Some(status)) => {
                format!("store still calls {cid} {status}", cid = esc(cid), status = esc(status))
            }
            (Some(cid), false, _) => format!("store has no cell named {cid}", cid = esc(cid)),
            _ => "worker names no cell".to_string(),
        };
        format!(r#"<div class="bee-cell__meta"><span class="fg-chip fg-chip--danger">{note}</span></div>"#)
    };
    format!(
        r#"<div class="fg-card bee-cell"><div class="fg-card__title">{nickname}</div><div class="bee-cell__meta">{cell_ref} · {age}</div>{discrepancy}</div>"#,
        nickname = esc(&w.nickname),
        cell_ref = cell_ref,
        age = esc(&bee_relative_minutes(w.heartbeat_age_minutes)),
        discrepancy = discrepancy,
    )
}

/// Every granted worktree (`.bee/runtime/worktree-grants.json`), each shown
/// by its own lifecycle record — feature, phase, branch, and whether a
/// session there is live — never by its own `.bee/cells/`, which
/// `mdview_core::bee::read_snapshot` deliberately never merges into this
/// project's buckets/shipped set (see that module's doc comment). Live
/// worktrees already sort first in `snapshot.worktrees` itself, so this view
/// only formats what it is handed. A dangling grant (missing directory,
/// missing or malformed `state.json`) renders plainly marked unresolved
/// rather than being dropped. A project with no granted worktrees renders
/// one quiet line instead of an empty bordered panel.
fn bee_worktree_section(worktrees: &[BeeWorktree]) -> String {
    if worktrees.is_empty() {
        return r#"<p class="fg-empty">No worktrees granted.</p>"#.to_string();
    }

    let mut rows = String::new();
    for w in worktrees {
        if w.resolved {
            let feature = w.feature.as_deref().unwrap_or("—");
            let phase = w.phase.as_deref().unwrap_or("—");
            let branch = w.branch.as_deref().unwrap_or("—");
            let (tone, label) = if w.live { ("success", "live") } else { ("neutral", "not live") };
            let age_line = match (w.live, w.heartbeat_age_minutes) {
                (true, Some(mins)) => format!(" · {}", esc(&bee_relative_minutes(mins))),
                _ => String::new(),
            };
            rows.push_str(&format!(
                r#"<div class="fg-card bee-cell"><div class="fg-card__title">{id}</div><div class="bee-cell__meta">feature: {feature} · phase: {phase} · branch: {branch}</div><div class="bee-cell__meta"><span class="fg-chip fg-chip--{tone}">{label}</span>{age_line}</div></div>"#,
                id = esc(&w.id),
                feature = esc(feature),
                phase = esc(phase),
                branch = esc(branch),
                tone = tone,
                label = label,
                age_line = age_line,
            ));
        } else {
            let reason = w.unresolved_reason.as_deref().unwrap_or("unresolved");
            rows.push_str(&format!(
                r#"<div class="fg-card bee-cell"><div class="fg-card__title">{id}</div><div class="bee-cell__meta"><span class="fg-chip fg-chip--danger">unresolved</span> · {reason}</div></div>"#,
                id = esc(&w.id),
                reason = esc(reason),
            ));
        }
    }

    format!(
        r#"<section class="fg-card bee-panel bee-worktrees">
  <h3 class="bee-panel__head">Worktrees <span class="fg-chip fg-chip--neutral">{count}</span></h3>
  <div class="bee-panel__list">{rows}</div>
</section>"#,
        count = worktrees.len(),
        rows = rows,
    )
}

/// Ship-velocity section (D10/D11 downstream): the three headline numbers the
/// user asked for — "1 ngày ship được bao nhiêu, 1 tuần ship được bao nhiêu" —
/// in plain language, followed by the list of features still open — the
/// short thing worth seeing here. The shipped-feature list used to render a
/// second time in this section too (one uncapped `fg-card` per feature,
/// stacked in a narrow column — 23 cards running off the screen on the real
/// beehive store, the thing a user complained about). That duplicate is
/// gone; the shipped/finished list now lives exactly once, collapsed by
/// default, in the board's Done section (`bee_done_section`) below the D7
/// buckets. Rendered above the four D7 buckets on the same page. A project
/// with nothing shipped yet gets an honest empty state instead of zeroed-out
/// or NaN numbers — the headline stats are computed only over
/// shipped-and-timed features (see `BeeVelocity`), so a `None` here means
/// "not enough data", never "zero".
fn bee_velocity_section(project_id: &str, snapshot: &BeeSnapshot) -> String {
    let open_features = bee_open_feature_names(snapshot);

    if snapshot.shipped.is_empty() {
        return format!(
            r#"<section class="fg-card bee-velocity">
  <h3 class="bee-velocity__head">Ship velocity</h3>
  <p class="fg-empty">No features have shipped yet — nothing to measure.</p>
  <div class="bee-velocity__lists">
    {open}
  </div>
</section>"#,
            open = bee_open_features_list(project_id, &open_features),
        );
    }

    let v = &snapshot.velocity;
    let stats = format!(
        r#"<div class="bee-stats">
    {rate_day}
    {rate_week}
    {cycle}
  </div>"#,
        rate_day = bee_stat_card("Shipped per working day", bee_fmt_rate(v.features_per_active_day)),
        rate_week = bee_stat_card("Shipped per week", bee_fmt_rate(v.features_per_week)),
        cycle = bee_stat_card("Typical time to finish", bee_fmt_hours(v.median_cycle_time_hours)),
    );

    format!(
        r#"<section class="fg-card bee-velocity">
  <h3 class="bee-velocity__head">Ship velocity</h3>
  {stats}
  <div class="bee-velocity__lists">
    {open}
  </div>
</section>"#,
        stats = stats,
        open = bee_open_features_list(project_id, &open_features),
    )
}

/// One headline stat card. `value` is already formatted for display;
/// `None` renders an honest "—" (not enough data yet), never a `0` or a
/// division artifact — the caller (`bee_fmt_rate`/`bee_fmt_hours`) is the
/// only place a `None` is manufactured, and only from a `None`/non-finite
/// upstream value.
fn bee_stat_card(label: &str, value: Option<String>) -> String {
    match value {
        Some(v) => format!(
            r#"<div class="fg-card bee-stat"><div class="bee-stat__value">{v}</div><div class="bee-stat__label">{label}</div></div>"#,
            v = esc(&v),
            label = esc(label),
        ),
        None => format!(
            r#"<div class="fg-card bee-stat bee-stat--empty"><div class="bee-stat__value">—</div><div class="bee-stat__label">{label}</div></div>"#,
            label = esc(label),
        ),
    }
}

/// A rate (features per day/week), one decimal place. `None` for a missing
/// or non-finite value — defensive against surfacing a NaN/Infinity even if
/// an upstream invariant ever slipped (division-by-zero is already guarded
/// in `mdview_core::bee::compute_velocity`, but the view never trusts that
/// alone).
fn bee_fmt_rate(v: Option<f64>) -> Option<String> {
    v.filter(|x| x.is_finite()).map(|x| format!("{x:.1}"))
}

/// An hours duration, one decimal place, suffixed `h`. Same finiteness
/// guard as `bee_fmt_rate`.
fn bee_fmt_hours(v: Option<f64>) -> Option<String> {
    v.filter(|x| x.is_finite()).map(|x| format!("{x:.1}h"))
}

/// Distinct feature names still open: any feature with at least one live
/// (non-dropped) cell in Doing, Waiting or Stuck that has not shipped (D10).
/// A feature that has shipped never appears here even if it also happens to
/// have a stray cell in one of those buckets — shipped status wins.
fn bee_open_feature_names(snapshot: &BeeSnapshot) -> Vec<String> {
    let shipped: std::collections::BTreeSet<&str> =
        snapshot.shipped.iter().map(|f| f.feature.as_str()).collect();
    let names: std::collections::BTreeSet<&str> = snapshot
        .buckets
        .doing
        .iter()
        .chain(snapshot.buckets.waiting.iter())
        .chain(snapshot.buckets.stuck.iter())
        .map(|c| c.feature.as_str())
        .filter(|f| !shipped.contains(f))
        .collect();
    names.into_iter().map(String::from).collect()
}

/// Each still-open feature name links to its detail page, same as the
/// shipped list above.
fn bee_open_features_list(project_id: &str, names: &[String]) -> String {
    let body = if names.is_empty() {
        "<p class=\"fg-empty\">Nothing open right now.</p>".to_string()
    } else {
        let items: String = names
            .iter()
            .map(|n| {
                format!(
                    r#"<li><a href="/p/{pid}/_bee/feature/{n_href}">{n}</a></li>"#,
                    pid = esc(project_id),
                    n_href = esc(n),
                    n = esc(n),
                )
            })
            .collect();
        format!(r#"<ul class="bee-velocity__open-list">{items}</ul>"#)
    };
    format!(
        r#"<div class="bee-velocity__col"><h4 class="bee-velocity__subhead">Still open</h4>{body}</div>"#,
        body = body,
    )
}

/// One D7 bucket. `key` is a stable, lowercase machine token (`data-bucket`)
/// so a test can assert a bucket's count without depending on the visible
/// label text; `tone` picks the chip/border color — `"danger"` gives Stuck
/// its own red styling (D7), never folded into Waiting's neutral tone. Each
/// cell card is a link to its detail page (`/p/:id/_bee/cell/:cell_id`) —
/// the drill-down this board exists to reach. `show_files` controls the
/// per-cell file-list meta line: the board (`bee_board_page`) passes `false`
/// — that detail crowded out the buckets a person is actually watching and
/// now lives only on the cell detail page — while the feature detail page
/// (`bee_feature_page`) keeps it, unchanged, at `true`.
fn bee_bucket_section(
    project_id: &str,
    label: &str,
    key: &str,
    cells: &[BeeCell],
    tone: &str,
    show_files: bool,
) -> String {
    let danger_cls = if tone == "danger" {
        " bee-bucket--danger"
    } else {
        ""
    };
    let mut rows = String::new();
    if cells.is_empty() {
        rows.push_str("<p class=\"fg-empty\">Nothing here.</p>");
    } else {
        for c in cells {
            let files = if !show_files || c.files.is_empty() {
                String::new()
            } else {
                format!(
                    "<div class=\"bee-cell__meta\">{}</div>",
                    esc(&c.files.join(", "))
                )
            };
            let worker = c
                .worker
                .as_deref()
                .map(|w| format!("<div class=\"bee-cell__meta\">worker: {}</div>", esc(w)))
                .unwrap_or_default();
            rows.push_str(&format!(
                r#"<a class="fg-card bee-cell" href="/p/{pid}/_bee/cell/{cid_href}"><div class="fg-card__title">{title}</div><div class="fg-card__sub">{id} · {feature} · {lane}</div>{files}{worker}</a>"#,
                pid = esc(project_id),
                cid_href = esc(&c.id),
                title = esc(&c.title),
                id = esc(&c.id),
                feature = esc(&c.feature),
                lane = esc(&c.lane),
                files = files,
                worker = worker,
            ));
        }
    }
    format!(
        r#"<section class="fg-card bee-bucket{danger_cls}" data-bucket="{key}" data-count="{count}"><h3 class="bee-bucket__head">{label} <span class="fg-chip fg-chip--{tone}">{count}</span></h3><div class="bee-bucket__body">{rows}</div></section>"#,
        danger_cls = danger_cls,
        key = key,
        count = cells.len(),
        label = label,
        tone = tone,
        rows = rows,
    )
}

/// The board's Done bucket (D7), rendered as a native `<details>`/`<summary>`
/// element that is collapsed by default — no `open` attribute, no
/// JavaScript. This is now the *only* place finished work is listed on the
/// board: `bee_shipped_list` used to render the same features a second time,
/// uncapped, as a column of full `fg-card`s in the velocity section above
/// (23 bordered cards on the real beehive store, running past a screenful —
/// the thing a user complained about from a screenshot). Grouped by feature
/// rather than one line per cell, same as before — a real store can carry
/// dozens of done cells across a handful of features. The `<summary>` states
/// the true totals (finished feature count, finished cell count) in plain
/// language even while collapsed, so the page never understates the store
/// just because the list is closed. Opening it reveals one compact text line
/// per feature — name, cell count and, when the feature has shipped with a
/// timed cycle (D10/D11), its time to finish, reused from `shipped` rather
/// than recomputed here — laid out in a dense multi-column grid instead of
/// one `fg-card` each. Collapsed-by-default plus the dense grid is what now
/// keeps this small, so every finished feature is shown; none is truncated.
/// `data-count` on the outer section stays the true total number of done
/// cells, same as every other D7 bucket, even though the body groups them by
/// feature.
fn bee_done_section(project_id: &str, done: &[BeeCell], shipped: &[BeeShippedFeature]) -> String {
    let total = done.len();
    if total == 0 {
        return r#"<section class="fg-card bee-bucket bee-done" data-bucket="done" data-count="0"><h3 class="bee-bucket__head">Done <span class="fg-chip fg-chip--success">0</span></h3><p class="fg-empty">Nothing done yet.</p></section>"#
            .to_string();
    }

    let cycle_by_feature: std::collections::BTreeMap<&str, &BeeShippedFeature> =
        shipped.iter().map(|f| (f.feature.as_str(), f)).collect();

    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for c in done {
        *counts.entry(c.feature.as_str()).or_insert(0) += 1;
    }
    let feature_total = counts.len();

    let mut lines = String::new();
    for (feature, count) in counts.iter() {
        let cycle = cycle_by_feature.get(feature).and_then(|f| match &f.cycle_time {
            Some(span) if span.hours.is_finite() => Some(format!("{:.1}h to finish", span.hours)),
            _ => None,
        });
        let meta = match cycle {
            Some(c) => format!(
                "{count} cell{plural} · {c}",
                count = count,
                plural = if *count == 1 { "" } else { "s" },
                c = c,
            ),
            None => format!(
                "{count} cell{plural}",
                count = count,
                plural = if *count == 1 { "" } else { "s" },
            ),
        };
        lines.push_str(&format!(
            r#"<a class="bee-done-line" href="/p/{pid}/_bee/feature/{feature_href}">{feature} · {meta}</a>"#,
            pid = esc(project_id),
            feature_href = esc(feature),
            feature = esc(feature),
            meta = esc(&meta),
        ));
    }

    let summary = format!(
        "Shipped: {feature_total} feature{fplural} finished · {total} cell{plural} total",
        feature_total = feature_total,
        fplural = if feature_total == 1 { "" } else { "s" },
        total = total,
        plural = if total == 1 { "" } else { "s" },
    );

    format!(
        r#"<section class="fg-card bee-bucket bee-done" data-bucket="done" data-count="{total}"><details class="bee-done-details"><summary class="bee-done-summary">{summary}</summary><div class="bee-done-grid">{lines}</div></details></section>"#,
        total = total,
        summary = esc(&summary),
        lines = lines,
    )
}

/// Names of `.bee/` files that could not be read, if any — every path
/// mentioned in `read_errors` already arrives relative to the project root
/// (see `mdview_core::bee`), so this only needs HTML escaping, not redaction.
fn bee_read_errors(errors: &[String]) -> String {
    if errors.is_empty() {
        return String::new();
    }
    let items: String = errors.iter().map(|e| format!("<li>{}</li>", esc(e))).collect();
    format!(
        r#"<div class="fg-card fg-card--sunken"><div class="fg-card__title">Could not read</div><ul>{items}</ul></div>"#,
        items = items
    )
}

/// Backlog, sessions and lanes panels (bee-cockpit-6), rendered below the D7
/// buckets on the same board page (D4/D1). Pure formatting over
/// `BeeSnapshot::backlog`/`sessions`/`lanes`/`workspaces` — every one of
/// those fields already arrived relativized/redacted from
/// `mdview_core::bee::read_snapshot` in bee-cockpit-5 (`BeeSession` carries
/// no `transcript_path`; every path on a `BeeWorkspace` is already
/// relative), so this view only formats what it is handed, never
/// recomputes any of that logic.
fn bee_panels_section(snapshot: &BeeSnapshot) -> String {
    format!(
        r#"<div class="bee-panels">
    {backlog}
    {sessions}
    {lanes}
  </div>"#,
        backlog = bee_backlog_panel(&snapshot.backlog),
        sessions = bee_sessions_panel(&snapshot.sessions),
        lanes = bee_lanes_panel(&snapshot.lanes, &snapshot.workspaces),
    )
}

/// Backlog panel: PBI items grouped by current status (a summary, not a
/// per-item dump, so it stays readable no matter how many PBIs a real store
/// holds), and findings grouped by severity with the P1 count visually
/// weighted (`bee-severity--p1`) since a P1 blocks. `findings.recent` is a
/// bounded slice of `findings.total` (`RECENT_DETAIL_CAP` in
/// `mdview_core::bee`) — when it is showing fewer than the true total, the
/// panel says so instead of looking smaller than the real backlog. An empty
/// PBI list and an empty finding set each render their own honest empty
/// state rather than a hidden section or a bare `0`.
fn bee_backlog_panel(backlog: &BeeBacklog) -> String {
    let pbi_body = if backlog.pbis.is_empty() {
        "<p class=\"fg-empty\">No backlog items yet.</p>".to_string()
    } else {
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
        let total = backlog.pbis.len();
        format!(
            r#"<div class="bee-panel__chips">{chips}</div><p class="bee-cell__meta">{total} backlog item{plural} total</p>"#,
            chips = chips,
            total = total,
            plural = if total == 1 { "" } else { "s" },
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

    format!(
        r#"<section class="fg-card bee-panel">
  <h3 class="bee-panel__head">Backlog</h3>
  <h4 class="bee-panel__subhead">PBIs by status</h4>
  {pbi_body}
  <h4 class="bee-panel__subhead">Findings by severity</h4>
  {findings_body}
</section>"#,
        pbi_body = pbi_body,
        findings_body = findings_body,
    )
}

/// Sessions panel: one entry per `.bee/sessions/*.json` session — its
/// source, its workspace, whether it is live or stale, and its heartbeat
/// age in plain relative language (`bee_fmt_heartbeat_age`), never a raw
/// timestamp. An empty session list renders an honest empty state, not a
/// hidden panel or a bare `0`.
fn bee_sessions_panel(sessions: &[BeeSession]) -> String {
    if sessions.is_empty() {
        return r#"<section class="fg-card bee-panel">
  <h3 class="bee-panel__head">Sessions</h3>
  <p class="fg-empty">No sessions recorded.</p>
</section>"#
            .to_string();
    }
    let mut rows = String::new();
    for s in sessions {
        let (tone, label) = if s.live { ("success", "live") } else { ("neutral", "stale") };
        let source = s.source.as_deref().unwrap_or("—");
        let workspace = s.workspace_id.as_deref().unwrap_or("—");
        rows.push_str(&format!(
            r#"<div class="fg-card bee-cell"><div class="fg-card__title">{id}</div><div class="bee-cell__meta"><span class="fg-chip fg-chip--{tone}">{label}</span> · {source} · {workspace} · {age}</div></div>"#,
            id = esc(&s.id),
            tone = tone,
            label = label,
            source = esc(source),
            workspace = esc(workspace),
            age = esc(&bee_fmt_heartbeat_age(s.heartbeat_age_minutes)),
        ));
    }
    format!(
        r#"<section class="fg-card bee-panel">
  <h3 class="bee-panel__head">Sessions <span class="fg-chip fg-chip--neutral">{count}</span></h3>
  <div class="bee-panel__list">{rows}</div>
</section>"#,
        count = sessions.len(),
        rows = rows,
    )
}

/// A signed minute count, rendered as plain relative language ("4 minutes
/// ago", "2 hours ago") — the shared core of `bee_fmt_heartbeat_age` (a
/// session's `last_heartbeat`) and `bee_fmt_trace_time` (a cell's
/// `claimed_at`/`capped_at`), so both read the same way. A negative age
/// (somehow in the future) reads as "just now" rather than a confusing
/// negative duration; a non-finite value reads "unknown" rather than
/// crashing the format.
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

/// A heartbeat age in minutes, rendered as plain relative language. See
/// `bee_relative_minutes`.
fn bee_fmt_heartbeat_age(minutes: f64) -> String {
    bee_relative_minutes(minutes)
}

/// A cell trace timestamp (`claimed_at`/`capped_at`, an RFC 3339 string),
/// rendered as plain relative language exactly like a session's heartbeat
/// (`bee_fmt_heartbeat_age`) — never the raw ISO string. A value that fails
/// to parse falls back to the raw string itself rather than hiding it: an
/// oddly-shaped-but-present timestamp is still more useful than "unknown".
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

/// Lanes panel: `.bee/lanes/*.json` records and `.bee/runtime/workspaces/*.json`
/// worktrees side by side, so the user can see which feature is running
/// where — the lane names the feature/phase/mode, the workspace names the
/// branch it runs on. Each source renders its own honest empty state when
/// absent, independent of the other.
fn bee_lanes_panel(lanes: &[BeeLane], workspaces: &[BeeWorkspace]) -> String {
    let lanes_body = if lanes.is_empty() {
        "<p class=\"fg-empty\">No lanes running.</p>".to_string()
    } else {
        let mut rows = String::new();
        for l in lanes {
            let phase = l.phase.as_deref().unwrap_or("—");
            let mode = l.mode.as_deref().unwrap_or("—");
            let next = l.next_action.as_deref().unwrap_or("—");
            rows.push_str(&format!(
                r#"<div class="fg-card bee-cell"><div class="fg-card__title">{feature}</div><div class="bee-cell__meta">phase: {phase} · mode: {mode}</div><div class="bee-cell__meta">{next}</div></div>"#,
                feature = esc(&l.feature),
                phase = esc(phase),
                mode = esc(mode),
                next = esc(next),
            ));
        }
        format!(r#"<div class="bee-panel__list">{rows}</div>"#, rows = rows)
    };

    let workspaces_body = if workspaces.is_empty() {
        "<p class=\"fg-empty\">No worktree workspaces yet.</p>".to_string()
    } else {
        let mut rows = String::new();
        for w in workspaces {
            let branch = w.branch.as_deref().unwrap_or("—");
            rows.push_str(&format!(
                r#"<div class="fg-card bee-cell"><div class="fg-card__title">{root}</div><div class="bee-cell__meta">branch: {branch} · {kind} · {attached} session{plural} attached</div></div>"#,
                root = esc(&w.root),
                branch = esc(branch),
                kind = esc(&w.kind),
                attached = w.attached_sessions,
                plural = if w.attached_sessions == 1 { "" } else { "s" },
            ));
        }
        format!(r#"<div class="bee-panel__list">{rows}</div>"#, rows = rows)
    };

    format!(
        r#"<section class="fg-card bee-panel">
  <h3 class="bee-panel__head">Lanes</h3>
  <h4 class="bee-panel__subhead">Lane records</h4>
  {lanes_body}
  <h4 class="bee-panel__subhead">Worktree workspaces</h4>
  {workspaces_body}
</section>"#,
        lanes_body = lanes_body,
        workspaces_body = workspaces_body,
    )
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

/// A status string's chip tone, matching the D7 bucket tones used on the
/// board (`bee_bucket_section`) so a cell's status chip reads consistently
/// wherever it appears.
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

/// The read-only feature detail page (D4): whether the feature has shipped
/// (D10) and its cycle time (D11) when timed, followed by every one of its
/// cells grouped into the same four D7 buckets the board uses — each cell
/// card links to its own detail page. Reached from the board's shipped/open
/// feature lists or from a cell page's feature link.
pub fn bee_feature_page(
    project: &Project,
    feature: &str,
    buckets: &BeeBuckets,
    shipped: Option<&BeeShippedFeature>,
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
        None => {
            r#"<div class="fg-card fg-card--sunken"><div class="fg-card__title">Not shipped yet</div></div>"#
                .to_string()
        }
    };

    let body = format!(
        r#"{topbar}
<main class="fg-page">
  <h2 class="fg-pagehead__title">{feature}</h2>
  {status_banner}
  <div class="bee-buckets">
    {doing}
    {waiting}
    {stuck}
    {done}
  </div>
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name} · {feature}</span>",
            name = esc(&project.name),
            feature = esc(feature),
        )),
        feature = esc(feature),
        status_banner = status_banner,
        doing = bee_bucket_section(&project.id, "Doing", "doing", &buckets.doing, "neutral", true),
        waiting = bee_bucket_section(&project.id, "Waiting", "waiting", &buckets.waiting, "neutral", true),
        stuck = bee_bucket_section(&project.id, "Stuck", "stuck", &buckets.stuck, "danger", true),
        done = bee_bucket_section(&project.id, "Done", "done", &buckets.done, "success", true),
    );
    layout(&format!("{} · {}", feature, project.name), "", &body)
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
    topbar_full("", center, "")
}

/// Full top bar: an optional `lead` slot (before the brand) and an optional
/// `actions` slot (page-specific buttons before the theme toggle, e.g. the
/// copy-page-as-Markdown button on file pages).
fn topbar_full(lead: &str, center: &str, actions: &str) -> String {
    format!(
        r#"<header class="topbar">
  {lead}
  <a href="/" class="home">mdview</a>
  {center}
  {actions}
  <a class="nav-link" href="/settings">Settings</a>
  {toggle}
</header>"#,
        lead = lead,
        center = center,
        actions = actions,
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

/// What the settings page's Terminal section renders for the token (D10).
/// Per P2 [`TerminalTokenView::Full`] is built exclusively from the direct
/// response of the rotate action — no other call site is allowed to
/// reconstruct it from a stored plaintext, because there is no stored
/// plaintext to read: `terminal_auth::TerminalAuth::rotate` is the only
/// function anywhere that ever returns the full value.
pub enum TerminalTokenView {
    /// No token has ever been generated.
    NotGenerated,
    /// The last four characters of the configured token — every render
    /// except the one that just generated or rotated it.
    Masked(String),
    /// The token in full. Rendered exactly once, in the response of the
    /// rotate action itself.
    Full(String),
}

/// What the settings page's notification section renders for the Telegram
/// credential (agent-terminal-18). Unlike [`TerminalTokenView`] there is no
/// `Full` variant at all: this credential is never rendered back in full,
/// not even once — the form that sets it (`/api/terminal-config`) is
/// write-only for this field, so this is the *only* view any response ever
/// carries, including the one immediately after a save.
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
    token_view: TerminalTokenView,
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

    let (token_banner, token_button_label) = match token_view {
        TerminalTokenView::NotGenerated => (
            "<p class=\"fg-field__hint\">No terminal token yet — generate one to switch the terminal on.</p>".to_string(),
            "Generate token",
        ),
        TerminalTokenView::Masked(masked) => (
            format!(
                "<p class=\"fg-field__hint\">Token: <code>{masked}</code></p>",
                masked = esc(&masked)
            ),
            "Rotate token",
        ),
        TerminalTokenView::Full(full) => (
            format!(
                "<div class=\"fg-banner fg-banner--success\"><span class=\"fg-banner__dot\"></span><span class=\"fg-banner__body\">Token generated — copy it now, it will not be shown again: <code>{full}</code></span></div>",
                full = esc(&full)
            ),
            "Rotate token",
        ),
    };

    // D7/D9: the notification credential is never rendered back in full
    // (unlike the terminal token above) — see `NotifyCredentialView`'s own
    // doc comment for why there is no `Full` variant to match here at all.
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
  <form class="fg-settings" method="post" action="/settings/terminal/token">
    <fieldset><legend>Terminal token</legend>
      {token_banner}
      <button type="submit" class="fg-btn">{token_button_label}</button>
    </fieldset>
  </form>
  <form class="fg-settings" method="post" action="/settings/terminal/login">
    <fieldset><legend>Terminal sign-in</legend>
      <div class="fg-field">
        <label class="fg-field__label">Token</label>
        <input class="fg-input" type="password" name="token" autocomplete="off" placeholder="Paste the terminal token">
      </div>
      <span class="fg-field__hint">Needed once per device/browser — signing in starts a session that lasts until the token is next rotated.</span>
      <button type="submit" class="fg-btn fg-btn--primary">Sign in</button>
    </fieldset>
  </form>
  <form class="fg-settings" method="post" action="/api/terminal-config">
    <fieldset><legend>Terminal <span class="fg-chip fg-chip--neutral">token required</span></legend>
      <label class="fg-check"><input type="checkbox" name="enabled" {term_enabled}><span class="fg-check__text">Enable the terminal</span></label>
      <label class="fg-check"><input type="checkbox" name="supervisor_enabled" {term_supervisor}><span class="fg-check__text">Keep herdr running (supervisor)</span></label>
      <label class="fg-check"><input type="checkbox" name="notify_enabled" {term_notify}><span class="fg-check__text">Notify on agent status change</span></label>
      <span class="fg-field__hint">Requires a valid terminal session to save — sign in above with the token first.</span>
    </fieldset>
    <fieldset><legend>Telegram notification <span class="fg-chip fg-chip--neutral">token required</span></legend>
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
        token_banner = token_banner,
        token_button_label = token_button_label,
        term_enabled = checked(cfg.terminal.enabled),
        term_supervisor = checked(cfg.terminal.supervisor_enabled),
        term_notify = checked(cfg.terminal.notify_enabled),
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
/// Vendored Mermaid (self-contained UMD build) served at /static/mermaid.min.js
/// so diagrams render without a CDN. Only loaded on pages that contain a diagram.
pub const MERMAID_JS: &str = include_str!("../assets/mermaid.min.js");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_age_reads_as_plain_relative_language_not_a_timestamp() {
        assert_eq!(bee_fmt_heartbeat_age(0.2), "just now");
        assert_eq!(bee_fmt_heartbeat_age(4.0), "4 minutes ago");
        assert_eq!(bee_fmt_heartbeat_age(1.0), "1 minute ago");
        assert_eq!(bee_fmt_heartbeat_age(120.0), "2 hours ago");
        assert_eq!(bee_fmt_heartbeat_age(60.0), "1 hour ago");
        assert_eq!(bee_fmt_heartbeat_age(60.0 * 24.0 * 3.0), "3 days ago");
        // A heartbeat somehow in the future reads as "just now", never a
        // negative duration.
        assert_eq!(bee_fmt_heartbeat_age(-5.0), "just now");
        assert_eq!(bee_fmt_heartbeat_age(f64::NAN), "unknown");
        // Never a raw ISO-8601 shape anywhere in the output.
        for mins in [0.0, 4.0, 90.0, 60.0 * 30.0] {
            assert!(!bee_fmt_heartbeat_age(mins).contains('T'));
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
        let html = terminal_page(&project, &[], &presets);
        assert!(html.contains(r#"data-preset="Claude">Claude</button>"#), "{html}");
        assert!(html.contains(r#"data-preset="Codex">Codex</button>"#), "{html}");
        assert!(!html.contains("data-preset=\"Aider\""), "an unconfigured label must never render: {html}");
        // The plain-shell control is unconditional — it needs no preset.
        assert!(html.contains(r#"<button type="button" class="term-create__pane">New shell</button>"#));
    }

    /// agent-terminal-13, must-have: "with no presets configured, the
    /// creation control offers nothing" — zero preset buttons render, while
    /// the plain-shell button (which needs no preset) still does.
    #[test]
    fn terminal_page_renders_no_preset_controls_when_none_configured() {
        let project = sample_project();
        let html = terminal_page(&project, &[], &[]);
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
}
