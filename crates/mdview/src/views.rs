//! Server-rendered HTML views. Self-contained: layout + CSS + JS as consts.
//! Theme is CSS-variable driven (no-flash head script); code colors come from
//! `/highlight.css` (syntect class-based), so themes switch without re-render.

use mdview_core::bee::{
    BeeBacklog, BeeBuckets, BeeCell, BeeLane, BeeSession, BeeShippedFeature, BeeSnapshot,
    BeeWorkspace,
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

pub fn project_list_page(projects: &[(Project, usize)]) -> String {
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
    let body = format!(
        r#"{topbar}
<main class="fg-page"><h2 class="fg-pagehead__title">Projects</h2>{listing}</main>"#,
        topbar = topbar(""),
        listing = listing
    );
    layout("Projects", "", &body)
}

/// A bee project's landing page (D3): a card linking into the bee board, plus
/// a card to open the project's docs when it has any. Rendered only when the
/// project has a `.bee/` directory — a non-bee project keeps the old
/// redirect-to-entry-file behavior in `server.rs::project_home` untouched.
pub fn project_home_page(project: &Project, entry: Option<&str>) -> String {
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
    let body = format!(
        r#"{topbar}
<main class="fg-page">
  <h2 class="fg-pagehead__title">{name}</h2>
  <div class="proj-cards">
    <a class="fg-card proj-card__link" href="/p/{pid}/_bee">
      <div class="fg-card__title">Bee board</div>
      <div class="fg-card__sub">Doing · Waiting · Stuck · Done</div>
    </a>
    {docs_card}
  </div>
</main>"#,
        topbar = topbar(&format!(
            "<span class=\"crumb\">{name}</span>",
            name = esc(&project.name)
        )),
        name = esc(&project.name),
        pid = esc(&project.id),
        docs_card = docs_card,
    );
    layout(&project.name, "", &body)
}

/// The read-only bee cell board (D4/D7): the project's four cell buckets,
/// each rendered by `bee_bucket_section`. Every path-shaped value on a
/// `BeeCell` already arrives relativized by `mdview_core::bee::read_snapshot`
/// (no absolute path crosses into `BeeSnapshot`'s public fields), so nothing
/// further is redacted here — this view only escapes for HTML safety.
pub fn bee_board_page(project: &Project, snapshot: &BeeSnapshot) -> String {
    let b = &snapshot.buckets;
    let phase = snapshot
        .state
        .as_ref()
        .and_then(|s| s.phase.as_deref())
        .unwrap_or("—");
    let feature = snapshot
        .state
        .as_ref()
        .and_then(|s| s.feature.as_deref())
        .unwrap_or("—");

    let body = format!(
        r#"{topbar}
<style>
.bee-buckets {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: var(--space-4); }}
.bee-buckets-top {{ display: grid; grid-template-columns: repeat(3, 1fr); gap: var(--space-4); margin-bottom: var(--space-4); }}
.bee-done {{ margin-bottom: var(--space-4); }}
.bee-done-list {{ display: flex; flex-direction: column; gap: var(--space-2); }}
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
</style>
<main class="fg-page">
  <div class="fg-pagehead">
    <h2 class="fg-pagehead__title">Bee board · {name}</h2>
    <div class="fg-pagehead__aside">
      <span class="fg-chip fg-chip--neutral">phase: {phase}</span>
      <span class="fg-chip fg-chip--neutral">feature: {feature}</span>
    </div>
  </div>
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
        name = esc(&project.name),
        phase = esc(phase),
        feature = esc(feature),
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

/// Ship-velocity section (D10/D11 downstream): the three headline numbers the
/// user asked for — "1 ngày ship được bao nhiêu, 1 tuần ship được bao nhiêu" —
/// in plain language, followed by the shipped-feature list (cycle time + cell
/// count) and the list of features still open. Rendered above the four D7
/// buckets on the same page. A project with nothing shipped yet gets an
/// honest empty state instead of zeroed-out or NaN numbers — the headline
/// stats are computed only over shipped-and-timed features (see
/// `BeeVelocity`), so a `None` here means "not enough data", never "zero".
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
    {shipped}
    {open}
  </div>
</section>"#,
        stats = stats,
        shipped = bee_shipped_list(project_id, &snapshot.shipped),
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

/// The shipped-feature list: each feature's name, cell count and cycle time
/// (or an honest "not timed yet" note when D11 could find no cycle time).
/// Only called when `shipped` is non-empty — the empty case is handled by
/// `bee_velocity_section` itself, above. Each row links to the feature's
/// detail page (`/p/:id/_bee/feature/:feature`) — the drill-down the board
/// exists to reach.
fn bee_shipped_list(project_id: &str, shipped: &[BeeShippedFeature]) -> String {
    let mut rows = String::new();
    for f in shipped {
        let cycle = match &f.cycle_time {
            Some(span) if span.hours.is_finite() => format!("{:.1}h to finish", span.hours),
            Some(_) => "—".to_string(),
            None => "not timed yet".to_string(),
        };
        rows.push_str(&format!(
            r#"<a class="fg-card bee-cell" href="/p/{pid}/_bee/feature/{feature_href}"><div class="fg-card__title">{feature}</div><div class="bee-cell__meta">{count} cell{plural} · {cycle}</div></a>"#,
            pid = esc(project_id),
            feature_href = esc(&f.feature),
            feature = esc(&f.feature),
            count = f.cell_count,
            plural = if f.cell_count == 1 { "" } else { "s" },
            cycle = esc(&cycle),
        ));
    }
    format!(
        r#"<div class="bee-velocity__col"><h4 class="bee-velocity__subhead">Shipped</h4>{rows}</div>"#,
        rows = rows,
    )
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

/// Cap on the number of feature lines shown in the board's Done section
/// (`bee_done_section`) — same bounded-list contract as
/// `mdview_core::bee::RECENT_DETAIL_CAP`: a shown-vs-true-total note appears
/// whenever the real feature count exceeds this, so a capped list never
/// looks smaller than the store really is.
const BEE_DONE_FEATURE_CAP: usize = 20;

/// The board's Done bucket (D7), rendered as its own full-width section
/// grouped by feature instead of one card per cell — a real store can carry
/// dozens of done cells across a handful of features, and one card each
/// buried the buckets a person is actually watching (Doing/Waiting/Stuck)
/// under a wall of finished work. Each line names the feature, how many of
/// its cells are done, and — when the feature has shipped with a timed
/// cycle (D10/D11) — its time to finish, reused from `shipped` rather than
/// recomputed here; a feature with done cells that has not (yet) fully
/// shipped still gets a line, just without a cycle time. `data-count` on
/// the section stays the true total number of done cells, same as every
/// other D7 bucket, even though the body below groups them by feature. The
/// line list itself is capped at `BEE_DONE_FEATURE_CAP`; capped or not, the
/// section states the true feature count and the true done-cell total so
/// neither number is ever understated.
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
    for (feature, count) in counts.iter().take(BEE_DONE_FEATURE_CAP) {
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
            r#"<a class="fg-card bee-cell" href="/p/{pid}/_bee/feature/{feature_href}"><div class="fg-card__title">{feature}</div><div class="bee-cell__meta">{meta}</div></a>"#,
            pid = esc(project_id),
            feature_href = esc(feature),
            feature = esc(feature),
            meta = esc(&meta),
        ));
    }

    let note = if feature_total > BEE_DONE_FEATURE_CAP {
        format!(
            r#"<p class="bee-cell__meta">Showing {shown} of {feature_total} features · {total} done cell{plural} total.</p>"#,
            shown = BEE_DONE_FEATURE_CAP,
            feature_total = feature_total,
            total = total,
            plural = if total == 1 { "" } else { "s" },
        )
    } else {
        format!(
            r#"<p class="bee-cell__meta">{feature_total} feature{fplural} · {total} done cell{plural} total.</p>"#,
            feature_total = feature_total,
            fplural = if feature_total == 1 { "" } else { "s" },
            total = total,
            plural = if total == 1 { "" } else { "s" },
        )
    };

    format!(
        r#"<section class="fg-card bee-bucket bee-done" data-bucket="done" data-count="{total}"><h3 class="bee-bucket__head">Done <span class="fg-chip fg-chip--success">{total}</span></h3>{note}<div class="bee-done-list">{lines}</div></section>"#,
        total = total,
        note = note,
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

pub fn settings_page(cfg: &Config, saved: bool) -> String {
    let banner = if saved {
        "<div class=\"fg-banner fg-banner--success\"><span class=\"fg-banner__dot\"></span><span class=\"fg-banner__body\">Saved. Server &amp; indexing changes apply after restart (<code>mdview stop &amp;&amp; mdview serve</code>).</span></div>"
    } else {
        ""
    };
    let checked = |b: bool| if b { "checked" } else { "" };
    let sel = |v: &str, opt: &str| if v == opt { "selected" } else { "" };
    let excludes = cfg.indexing.exclude_patterns.join("\n");

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
}
