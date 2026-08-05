//! Pure reader for a project's `.bee/` store — bee-cockpit Slice 1.
//!
//! Turns `<root>/.bee/` into a typed [`BeeSnapshot`]. This module is
//! deliberately framework-free (no axum/tokio/hyper) so it stays inside
//! `mdview-core`, per the crate split documented at the top of `lib.rs`.
//!
//! Decisions honored here (see `docs/history/bee-cockpit/CONTEXT.md`):
//! - **D3** — presence is `<root>/.bee/` existing; absence is reported, not
//!   an error.
//! - **D4** — strictly read-only: every path here is opened for reading
//!   only, nothing is ever written.
//! - **D7** — cells sort into four buckets (Doing/Waiting/Stuck/Done);
//!   `dropped` and any unrecognized status land in none of them and are
//!   excluded from every count.
//! - **D8** — `active` is true iff at least one cell is `open` or `claimed`.
//! - **D9** — only live `.bee/cells/*.json` is read; `.bee/cells/archive/`
//!   and `.bee/logs/` are never opened.
//! - **D10** — a feature is **shipped** when every one of its non-dropped
//!   cells is `capped`. A worktree merge into main is never consulted, and a
//!   dropped cell never blocks shipped status; a feature whose cells are
//!   *all* dropped is neither shipped nor counted.
//! - **D11** — a shipped feature's cycle time runs from the earliest
//!   `trace.claimed_at` to the latest `trace.capped_at` across its
//!   non-dropped cells. Either endpoint missing means no cycle time is
//!   reported — never a guessed zero.
//!
//! Slice 2 (bee-cockpit-5) extends the snapshot with the rest of the store —
//! backlog, sessions, lanes and workspaces — always **summarized**, never
//! dumped:
//! - `.bee/backlog.jsonl` mixes two row shapes. `kind == "pbi"` rows are
//!   event-sourced and folded by `id` to the LAST occurrence's status; every
//!   other row is a finding, grouped by `severity` (`P1`/`P2`/`P3`) with a
//!   bounded [`RECENT_DETAIL_CAP`]-sized "recent" slice alongside the true
//!   total.
//! - `.bee/sessions/*.json` sessions are `live` when `last_heartbeat` is
//!   within [`SESSION_LIVE_MINUTES`] of the read, `stale` otherwise, with the
//!   heartbeat age exposed in minutes. `transcript_path` is never read into a
//!   public field — it is an absolute path into the user's home.
//! - `.bee/lanes/*.json` (when present) surface per-feature lane state
//!   alongside the default pipeline's `.bee/state.json`.
//! - `.bee/runtime/workspaces/*.json` surface worktree/workspace records;
//!   `root` is relativized like every other path-shaped field.
//! - `.bee/decisions.jsonl` reports its true total event count plus only the
//!   most recent [`RECENT_DETAIL_CAP`] `decide` events — the full log is
//!   never loaded into the snapshot.
//!
//! bee-board-ux-4 adds worktree liveness: `.bee/runtime/worktree-grants.json`
//! (when present) names every currently-granted worktree. Each granted id is
//! resolved against its own **sibling** `.bee/` — that worktree's own
//! `state.json` for `feature`/`phase`/`mode`, and that worktree's own
//! `.bee/sessions/*.json` for liveness on the same [`SESSION_LIVE_MINUTES`]
//! window — then joined to the `branch`/`created_at` already read from this
//! project's own `.bee/runtime/workspaces/` records above. This is
//! deliberately **never** built from the worktree's own `.bee/cells/`:
//! measured live against a 14-worktree store, every granted worktree held a
//! stale snapshot of the very same live cell set this module already reads,
//! and the only cell any of them disagreed about was the SAME cell, still
//! `claimed` in their snapshot but long since `capped` in the real store.
//! Reading worktree cells into the board would resurrect that one finished
//! cell as in-flight once per worktree. See [`BeeWorktree`]. A dangling
//! grant — sibling directory gone, `state.json` missing or malformed — is
//! reported unresolved, never dropped and never fatal.
//!
//! Every path-shaped value that crosses into a public field is rendered
//! relative to the project root (or reduced to a bare filename when it
//! falls outside the root) — no absolute path may survive into a
//! [`BeeSnapshot`]'s public fields. Malformed JSON degrades to a partial
//! snapshot with a note in [`BeeSnapshot::read_errors`] instead of
//! propagating an error that would take down a page render.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// One live cell, trimmed to what the cockpit board needs. Any path-shaped
/// field (`files`, `worker`) is relativized against the project root before
/// it reaches this struct — see [`relativize`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeCell {
    pub id: String,
    pub feature: String,
    pub title: String,
    pub lane: String,
    /// Raw status string as read from the cell file (`open`, `claimed`,
    /// `blocked`, `capped`, `dropped`, or anything else a future bee
    /// version introduces). Bucketing is derived from this, never stored
    /// redundantly.
    pub status: String,
    pub tier: Option<String>,
    /// Relative to the project root; never absolute.
    pub files: Vec<String>,
    /// `trace.worker`, relativized if it happens to be path-shaped.
    pub worker: Option<String>,
    pub claimed_at: Option<String>,
    pub capped_at: Option<String>,
}

/// The four D7 buckets. A `dropped` cell or one with an unrecognized status
/// lands in none of these and is excluded from every count.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeBuckets {
    /// `status == "claimed"`.
    pub doing: Vec<BeeCell>,
    /// `status == "open"`.
    pub waiting: Vec<BeeCell>,
    /// `status == "blocked"`.
    pub stuck: Vec<BeeCell>,
    /// `status == "capped"`.
    pub done: Vec<BeeCell>,
}

/// The subset of `.bee/state.json` the cockpit shows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeState {
    pub phase: Option<String>,
    pub feature: Option<String>,
    pub mode: Option<String>,
    /// `state.json`'s `workers[]`, verbatim (raw, unjoined). bee's own docs
    /// call this array hand-maintained and not fully trusted, so it is never
    /// used to move a cell between D7 buckets — see [`BeeRunningWorker`] for
    /// the joined, session-verified view this snapshot derives from it.
    pub workers: Vec<BeeWorker>,
}

/// One raw entry from `.bee/state.json`'s `workers[]`. `cell`, `tier` and
/// `status` are each commonly `null` in practice (bee updates this array
/// best-effort, not transactionally with the cell it names), so every field
/// but `nickname` is optional.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeWorker {
    pub nickname: String,
    pub cell: Option<String>,
    pub tier: Option<String>,
    pub status: Option<String>,
}

/// One worker from `.bee/state.json`'s `workers[]`, joined against the live
/// cells and sessions this snapshot already read. A worker only ever
/// appears here when a session sharing its exact nickname is live — bee
/// names a worker-launched session's file after its worker's nickname
/// (`.bee/sessions/<nickname>.json` carries `"id": "<nickname>"`), so that
/// shared identifier is the join key between "a worker the store still
/// lists" and "a process that is actually still reporting in". A worker
/// with no matching session, or one whose matching session has gone stale,
/// is silently absent from this list rather than claimed to be running on
/// no evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeRunningWorker {
    pub nickname: String,
    /// The cell id this worker names, if any.
    pub cell: Option<String>,
    pub tier: Option<String>,
    pub status: Option<String>,
    /// The matching live session's heartbeat age, in minutes.
    pub heartbeat_age_minutes: f64,
    /// True when `cell` names a cell this snapshot actually read.
    pub cell_found: bool,
    /// The named cell's own `status`, when it was found.
    pub cell_status: Option<String>,
    /// True when the store and the running process disagree: the named
    /// cell does not exist, or it exists but its own status is not
    /// `claimed`. Never resolved automatically — surfaced so a human can
    /// see it (D7's buckets stay a pure function of cell status either
    /// way; see `compute_running_workers`).
    pub discrepancy: bool,
}

/// The claim-to-cap span of one shipped feature (D11). Both timestamps are
/// the raw RFC 3339 strings straight from `trace`, plus the derived duration
/// so callers never have to reparse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeCycleSpan {
    /// Earliest `trace.claimed_at` across the feature's non-dropped cells.
    pub started_at: String,
    /// Latest `trace.capped_at` across the feature's non-dropped cells.
    pub ended_at: String,
    /// `ended_at - started_at`, in hours.
    pub hours: f64,
}

/// One feature that has shipped per D10: every one of its non-dropped cells
/// is `capped`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeShippedFeature {
    pub feature: String,
    /// How many non-dropped cells back this feature's shipped status.
    pub cell_count: usize,
    /// `None` when a non-dropped cell is missing `claimed_at` or
    /// `capped_at` (or every one of them is) — a shipped feature is still
    /// reported here, just without a cycle time to guess at (D11).
    pub cycle_time: Option<BeeCycleSpan>,
}

/// One calendar day's shipped-feature count, keyed on that day's last cap
/// date (UTC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeDayCount {
    /// `YYYY-MM-DD`, UTC.
    pub day: String,
    pub count: usize,
}

/// Ship-rate aggregates over the shipped features that report a cycle time
/// (D11) — a shipped feature with no timestamps contributes to
/// [`BeeSnapshot::shipped`] but not to these numbers, since none of them can
/// be placed on a calendar day without one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeVelocity {
    /// Shipped-feature count per calendar day, keyed on each feature's last
    /// cap date. Sorted chronologically.
    pub per_day: Vec<BeeDayCount>,
    /// Distinct calendar days with at least one shipped feature.
    pub active_days: usize,
    /// Shipped-and-timed feature count divided by `active_days`. `None`
    /// when there are no active days — never a division by zero.
    pub features_per_active_day: Option<f64>,
    /// Shipped-and-timed feature count spread over the calendar span from
    /// the first to the last ship day, inclusive, expressed per week.
    /// `None` when nothing shipped with a timestamp.
    pub features_per_week: Option<f64>,
    /// Median of every shipped-and-timed feature's cycle time, in hours.
    /// `None` when nothing shipped with a timestamp.
    pub median_cycle_time_hours: Option<f64>,
}

/// Recent-detail cap shared by every bounded panel added in Slice 2 —
/// backlog findings and decision events. Deliberately small: this snapshot
/// is rebuilt on every page request, and a store the size of the real
/// beehive one (659 backlog rows, 1831 decision events) must never be
/// returned whole.
const RECENT_DETAIL_CAP: usize = 20;

/// A session's heartbeat is considered live within this many minutes of the
/// read; older is stale.
const SESSION_LIVE_MINUTES: f64 = 30.0;

/// One folded PBI (product backlog item) from `.bee/backlog.jsonl`, current
/// state only — the event history that produced it is not kept.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeePbi {
    pub id: String,
    pub title: String,
    /// `proposed`, `in-flight`, `parked`, `done`, `declined`, or anything
    /// else a future bee version introduces — folded from the LAST event
    /// carrying this `id`, never the first.
    pub status: String,
    pub feature: String,
}

/// Per-severity finding counts (`P1`/`P2`/`P3`) over the *whole* backlog,
/// independent of how many are exposed in [`BeeFindings::recent`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeSeverityCounts {
    pub p1: usize,
    pub p2: usize,
    pub p3: usize,
}

/// One non-PBI row from `.bee/backlog.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeFinding {
    pub ts: String,
    /// The row's own `type` field (e.g. `"finding"`, `"proposal"`).
    pub kind: String,
    pub title: String,
    pub detail: String,
    /// `P1`, `P2`, `P3`, or empty when the row carries none.
    pub severity: String,
    pub layer: String,
    pub feature: String,
}

/// Findings from `.bee/backlog.jsonl`, grouped by severity, bounded.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeFindings {
    /// True count of every finding row, independent of the cap below.
    pub total: usize,
    pub by_severity: BeeSeverityCounts,
    /// The most recent findings by `ts`, capped at [`RECENT_DETAIL_CAP`].
    pub recent: Vec<BeeFinding>,
}

/// The `.bee/backlog.jsonl` view: folded PBIs plus grouped, bounded
/// findings. Never a raw dump of the 659-row (or larger) event log.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeBacklog {
    /// Every distinct PBI, folded to its current status.
    pub pbis: Vec<BeePbi>,
    pub findings: BeeFindings,
}

/// One `.bee/sessions/<uuid>.json` session, trimmed to what the cockpit may
/// show. `transcript_path` is deliberately never carried here — it is an
/// absolute path into the user's home.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeSession {
    pub id: String,
    pub started_at: Option<String>,
    /// Minutes between `last_heartbeat` and the read; negative if the
    /// heartbeat is somehow in the future.
    pub heartbeat_age_minutes: f64,
    /// True when `heartbeat_age_minutes <= `[`SESSION_LIVE_MINUTES`].
    pub live: bool,
    pub workspace_id: Option<String>,
    pub source: Option<String>,
}

/// One `.bee/lanes/<feature>.json` per-feature lane record, mirroring the
/// subset of `.bee/state.json` the cockpit already shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeLane {
    pub feature: String,
    pub phase: Option<String>,
    pub mode: Option<String>,
    pub next_action: Option<String>,
}

/// One `.bee/runtime/workspaces/<id>.json` worktree/workspace record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeWorkspace {
    pub id: String,
    /// The row's own `type` field (e.g. `"worktree"`).
    pub kind: String,
    /// Relativized against the project root, or reduced to a bare directory
    /// name when it falls outside the root (workspaces typically live in
    /// sibling directories) — never absolute.
    pub root: String,
    pub branch: Option<String>,
    pub attached_sessions: usize,
    pub created_at: Option<String>,
}

/// One granted worktree (`.bee/runtime/worktree-grants.json`), resolved
/// against its own sibling `.bee/` and joined to the branch/creation time
/// already read from this project's own `.bee/runtime/workspaces/` records
/// — see the module doc comment for why this is never built from the
/// worktree's own `.bee/cells/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeWorktree {
    /// The grant id — already a safe name (it names the sibling directory
    /// and the matching `.bee/runtime/workspaces/<id>.json`'s own `id`), so
    /// this is the only identifier carried here; the sibling directory's
    /// absolute root is read to resolve this record but never stored.
    pub id: String,
    /// False when the sibling directory does not exist, or its own
    /// `.bee/state.json` is missing or malformed. A dangling grant is
    /// reported here, never dropped and never a hard failure.
    pub resolved: bool,
    /// Set when `resolved` is false, naming what could not be read.
    pub unresolved_reason: Option<String>,
    /// The worktree's own `state.json` `feature` — read from its own
    /// `.bee/`, not this project's.
    pub feature: Option<String>,
    /// The worktree's own `state.json` `phase` — the live signal a granted
    /// worktree's cells cannot give (they are a stale snapshot).
    pub phase: Option<String>,
    /// The worktree's own `state.json` `mode`.
    pub mode: Option<String>,
    /// From this project's own `.bee/runtime/workspaces/<id>.json`, never
    /// re-read from the worktree side.
    pub branch: Option<String>,
    /// From this project's own `.bee/runtime/workspaces/<id>.json`.
    pub created_at: Option<String>,
    /// True when at least one of the worktree's own `.bee/sessions/*.json`
    /// is live, using the same [`SESSION_LIVE_MINUTES`] window the main
    /// store's own sessions use.
    pub live: bool,
    /// The freshest live session's heartbeat age, in minutes, when `live`.
    pub heartbeat_age_minutes: Option<f64>,
}

impl BeeWorktree {
    fn unresolved(id: &str, reason: &str, branch: Option<String>, created_at: Option<String>) -> Self {
        BeeWorktree {
            id: id.to_string(),
            resolved: false,
            unresolved_reason: Some(reason.to_string()),
            feature: None,
            phase: None,
            mode: None,
            branch,
            created_at,
            live: false,
            heartbeat_age_minutes: None,
        }
    }
}

/// One `decide`-type event from `.bee/decisions.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeDecisionSummary {
    pub id: String,
    pub date: String,
    pub decision: String,
    pub scope: Option<String>,
}

/// The `.bee/decisions.jsonl` view: the true event count plus only the most
/// recent `decide` events. The full 1831-event log (or larger) is never
/// loaded into the snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeDecisions {
    /// Every event row (`decide`, `tag`, `redact`, `supersede`, `stub`).
    pub total: usize,
    /// The most recent `decide` events, capped at [`RECENT_DETAIL_CAP`].
    pub recent: Vec<BeeDecisionSummary>,
}

/// A typed snapshot of a project's `.bee/` store at read time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeSnapshot {
    /// True when `<root>/.bee/` exists (D3).
    pub present: bool,
    /// `None` when `.bee/state.json` is absent or malformed; see
    /// `read_errors` for why.
    pub state: Option<BeeState>,
    pub buckets: BeeBuckets,
    /// True when at least one cell is `open` or `claimed` (D8).
    pub active: bool,
    /// Every feature that has shipped (D10), regardless of whether its
    /// cycle time could be computed.
    pub shipped: Vec<BeeShippedFeature>,
    /// Ship-rate aggregates derived from `shipped` (D11 downstream).
    pub velocity: BeeVelocity,
    /// `.bee/backlog.jsonl`, summarized (Slice 2).
    pub backlog: BeeBacklog,
    /// `.bee/sessions/*.json`, one entry per session (Slice 2).
    pub sessions: Vec<BeeSession>,
    /// `.bee/lanes/*.json`, empty when the directory is absent (Slice 2).
    pub lanes: Vec<BeeLane>,
    /// `.bee/runtime/workspaces/*.json` (Slice 2).
    pub workspaces: Vec<BeeWorkspace>,
    /// `.bee/decisions.jsonl`, bounded (Slice 2).
    pub decisions: BeeDecisions,
    /// Every currently-granted worktree (`.bee/runtime/worktree-grants.json`),
    /// each resolved against its own sibling `.bee/` — see [`BeeWorktree`].
    /// Never a function of any worktree's own `.bee/cells/`; `buckets`,
    /// `shipped` and `velocity` above stay a pure function of this project's
    /// own live cells regardless of what this field holds.
    pub worktrees: Vec<BeeWorktree>,
    /// Workers named in `state.json`'s `workers[]` whose session is
    /// currently live — the "running now" view. Deliberately separate from
    /// `buckets`: it never rewrites a cell's D7 bucket, it only tells a
    /// reader that a `Waiting`/`Stuck` cell nonetheless has a live process
    /// against it, or flags one that does not agree with the store.
    pub running_workers: Vec<BeeRunningWorker>,
    /// Human-readable notes naming what could not be read. Every path
    /// mentioned here is relative to the project root.
    pub read_errors: Vec<String>,
}

impl BeeSnapshot {
    /// The snapshot for a project whose root has no `.bee/` directory (D3).
    pub fn absent() -> Self {
        BeeSnapshot {
            present: false,
            state: None,
            buckets: BeeBuckets::default(),
            active: false,
            shipped: Vec::new(),
            velocity: BeeVelocity::default(),
            backlog: BeeBacklog::default(),
            sessions: Vec::new(),
            lanes: Vec::new(),
            workspaces: Vec::new(),
            decisions: BeeDecisions::default(),
            worktrees: Vec::new(),
            running_workers: Vec::new(),
            read_errors: Vec::new(),
        }
    }
}

/// Read `<root>/.bee/` into a typed [`BeeSnapshot`].
///
/// Pure and infallible: this function only opens files for reading, never
/// writes anything (D4), and never panics or returns `Err` — a missing or
/// malformed file is recorded in [`BeeSnapshot::read_errors`] and the read
/// continues with whatever else could be parsed.
pub fn read_snapshot(root: &Path) -> BeeSnapshot {
    let bee_dir = root.join(".bee");
    if !bee_dir.is_dir() {
        return BeeSnapshot::absent();
    }

    let mut read_errors = Vec::new();

    let state = read_state(&bee_dir, root, &mut read_errors);

    let mut buckets = BeeBuckets::default();
    let mut active = false;
    // Every successfully-parsed live cell, dropped and unknown-status ones
    // included — the feature/shipped view below needs the full set, unlike
    // the D7 buckets which deliberately drop `dropped` cells.
    let mut all_cells: Vec<BeeCell> = Vec::new();

    let cells_dir = bee_dir.join("cells");
    if cells_dir.is_dir() {
        let mut entries: Vec<PathBuf> = match fs::read_dir(&cells_dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                // .bee/cells/archive/ (D9) has no .json extension of its own
                // and is filtered out here; the is_file() guard below is a
                // second, explicit line of defense against ever descending
                // into it.
                .filter(|p| p.is_file())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
                .collect(),
            Err(e) => {
                read_errors.push(format!(".bee/cells: could not list ({e})"));
                Vec::new()
            }
        };
        entries.sort();

        for path in entries {
            match parse_cell(&path, root) {
                Ok(cell) => {
                    let is_active = matches!(cell.status.as_str(), "open" | "claimed");
                    if is_active {
                        active = true;
                    }
                    all_cells.push(cell.clone());
                    match cell.status.as_str() {
                        "claimed" => buckets.doing.push(cell),
                        "open" => buckets.waiting.push(cell),
                        "blocked" => buckets.stuck.push(cell),
                        "capped" => buckets.done.push(cell),
                        // "dropped" and any unrecognized status: no bucket,
                        // no count (D7), read still succeeds.
                        _ => {}
                    }
                }
                Err(e) => {
                    read_errors.push(format!("{}: {e}", rel_str(&path, root)));
                }
            }
        }
    }

    let shipped = compute_shipped_features(&all_cells);
    let velocity = compute_velocity(&shipped);

    let backlog = read_backlog(&bee_dir, root, &mut read_errors);
    let now = time::OffsetDateTime::now_utc();
    let sessions = read_sessions(&bee_dir, root, now, &mut read_errors);
    let lanes = read_lanes(&bee_dir, root, &mut read_errors);
    let workspaces = read_workspaces(&bee_dir, root, &mut read_errors);
    let decisions = read_decisions(&bee_dir, root, &mut read_errors);
    let worktrees = read_worktrees(root, &workspaces, now, &mut read_errors);

    let running_workers = state
        .as_ref()
        .map(|s| compute_running_workers(&s.workers, &all_cells, &sessions))
        .unwrap_or_default();

    BeeSnapshot {
        present: true,
        state,
        buckets,
        active,
        shipped,
        velocity,
        backlog,
        sessions,
        lanes,
        workspaces,
        decisions,
        worktrees,
        running_workers,
        read_errors,
    }
}

/// Join `state.json`'s raw `workers[]` against the live cells and sessions
/// this snapshot already read (D4 — read-only, no additional I/O). Never
/// mutates or is used to compute `buckets`: D7's buckets stay a pure
/// function of each cell's own `status`, full stop. A worker only survives
/// into the returned list when a session sharing its exact `nickname` is
/// live (see [`BeeRunningWorker`]); a worker with no such session, or a
/// stale one, is silently omitted rather than presented as running on no
/// evidence.
fn compute_running_workers(
    workers: &[BeeWorker],
    all_cells: &[BeeCell],
    sessions: &[BeeSession],
) -> Vec<BeeRunningWorker> {
    let mut out = Vec::new();
    for w in workers {
        let Some(session) = sessions.iter().find(|s| s.id == w.nickname) else {
            continue;
        };
        if !session.live {
            continue;
        }
        let cell_match = w
            .cell
            .as_deref()
            .and_then(|cid| all_cells.iter().find(|c| c.id == cid));
        let cell_found = cell_match.is_some();
        let cell_status = cell_match.map(|c| c.status.clone());
        // A discrepancy is "the store disagrees with the running process":
        // no such cell at all, or a cell whose own status is not `claimed`.
        let discrepancy = cell_status.as_deref() != Some("claimed");
        out.push(BeeRunningWorker {
            nickname: w.nickname.clone(),
            cell: w.cell.clone(),
            tier: w.tier.clone(),
            status: w.status.clone(),
            heartbeat_age_minutes: session.heartbeat_age_minutes,
            cell_found,
            cell_status,
            discrepancy,
        });
    }
    out
}

fn read_state(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> Option<BeeState> {
    let path = bee_dir.join("state.json");
    if !path.is_file() {
        // No state.json is a normal, expected shape (not every .bee/ has
        // reached a phase yet) — silent, not a read error.
        return None;
    }
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) => {
            read_errors.push(format!("{}: could not read ({e})", rel_str(&path, root)));
            return None;
        }
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(v) => Some(BeeState {
            phase: v.get("phase").and_then(Value::as_str).map(String::from),
            feature: v.get("feature").and_then(Value::as_str).map(String::from),
            mode: v.get("mode").and_then(Value::as_str).map(String::from),
            workers: v
                .get("workers")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(parse_worker).collect())
                .unwrap_or_default(),
        }),
        Err(e) => {
            read_errors.push(format!("{}: could not parse ({e})", rel_str(&path, root)));
            None
        }
    }
}

/// Parse one `state.json` `workers[]` entry. `nickname` missing or
/// non-string makes the whole entry unparseable — everything else is
/// optional (see [`BeeWorker`]).
fn parse_worker(v: &Value) -> Option<BeeWorker> {
    let nickname = v.get("nickname").and_then(Value::as_str)?.to_string();
    let cell = v.get("cell").and_then(Value::as_str).map(String::from);
    let tier = v.get("tier").and_then(Value::as_str).map(String::from);
    let status = v.get("status").and_then(Value::as_str).map(String::from);
    Some(BeeWorker { nickname, cell, tier, status })
}

/// Parse one `.bee/cells/<id>.json` file into a [`BeeCell`], relativizing
/// every path-shaped value it carries.
fn parse_cell(path: &Path, root: &Path) -> Result<BeeCell, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("could not read ({e})"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("could not parse ({e})"))?;

    let id = v
        .get("id")
        .and_then(Value::as_str)
        .ok_or("missing \"id\"")?
        .to_string();
    let status = v
        .get("status")
        .and_then(Value::as_str)
        .ok_or("missing \"status\"")?
        .to_string();
    let feature = v
        .get("feature")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let title = v
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let lane = v
        .get("lane")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tier = v.get("tier").and_then(Value::as_str).map(String::from);

    let files = v
        .get("files")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| relativize(s, root))
                .collect()
        })
        .unwrap_or_default();

    let trace = v.get("trace");
    let worker = trace
        .and_then(|t| t.get("worker"))
        .and_then(Value::as_str)
        .map(|s| relativize(s, root));
    let claimed_at = trace
        .and_then(|t| t.get("claimed_at"))
        .and_then(Value::as_str)
        .map(String::from);
    let capped_at = trace
        .and_then(|t| t.get("capped_at"))
        .and_then(Value::as_str)
        .map(String::from);

    Ok(BeeCell {
        id,
        feature,
        title,
        lane,
        status,
        tier,
        files,
        worker,
        claimed_at,
        capped_at,
    })
}

/// Read and summarize `.bee/backlog.jsonl` (D4, D9-adjacent — this file is
/// live store, not archive). A missing file is a normal, expected shape
/// (silent, matching `read_state`); a malformed line degrades to a
/// `read_errors` note naming its line number, and the read continues with
/// whatever else could be parsed.
fn read_backlog(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> BeeBacklog {
    let path = bee_dir.join("backlog.jsonl");
    if !path.is_file() {
        return BeeBacklog::default();
    }
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) => {
            read_errors.push(format!("{}: could not read ({e})", rel_str(&path, root)));
            return BeeBacklog::default();
        }
    };

    // Event-sourced: later occurrences of the same id overwrite earlier
    // ones, so iterating top-to-bottom and inserting into a map naturally
    // folds to the LAST status.
    let mut pbis: std::collections::BTreeMap<String, BeePbi> = std::collections::BTreeMap::new();
    let mut findings: Vec<BeeFinding> = Vec::new();
    let mut by_severity = BeeSeverityCounts::default();
    let mut finding_total = 0usize;

    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                read_errors.push(format!(
                    "{}: line {} could not parse ({e})",
                    rel_str(&path, root),
                    i + 1
                ));
                continue;
            }
        };

        if v.get("kind").and_then(Value::as_str) == Some("pbi") {
            let id = match v.get("id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    read_errors.push(format!(
                        "{}: line {} pbi row missing \"id\"",
                        rel_str(&path, root),
                        i + 1
                    ));
                    continue;
                }
            };
            let title = v.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
            let status = v.get("status").and_then(Value::as_str).unwrap_or_default().to_string();
            let feature = v.get("feature").and_then(Value::as_str).unwrap_or_default().to_string();
            pbis.insert(id.clone(), BeePbi { id, title, status, feature });
        } else {
            finding_total += 1;
            let severity = v.get("severity").and_then(Value::as_str).unwrap_or_default().to_string();
            match severity.as_str() {
                "P1" => by_severity.p1 += 1,
                "P2" => by_severity.p2 += 1,
                "P3" => by_severity.p3 += 1,
                _ => {}
            }
            findings.push(BeeFinding {
                ts: v.get("ts").and_then(Value::as_str).unwrap_or_default().to_string(),
                kind: v.get("type").and_then(Value::as_str).unwrap_or_default().to_string(),
                title: v.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
                detail: v.get("detail").and_then(Value::as_str).unwrap_or_default().to_string(),
                severity,
                layer: v.get("layer").and_then(Value::as_str).unwrap_or_default().to_string(),
                feature: v.get("feature").and_then(Value::as_str).unwrap_or_default().to_string(),
            });
        }
    }

    // Most recent first. `ts` is RFC 3339 with a fixed-width, zero-padded,
    // `Z`-suffixed shape throughout this store, so a plain string compare
    // sorts chronologically without needing to parse every row.
    findings.sort_by(|a, b| b.ts.cmp(&a.ts));
    findings.truncate(RECENT_DETAIL_CAP);

    BeeBacklog {
        pbis: pbis.into_values().collect(),
        findings: BeeFindings {
            total: finding_total,
            by_severity,
            recent: findings,
        },
    }
}

/// Read `.bee/sessions/*.json` (D4). A missing directory yields an empty
/// list, not an error, matching the `.bee/cells` precedent.
fn read_sessions(
    bee_dir: &Path,
    root: &Path,
    now: time::OffsetDateTime,
    read_errors: &mut Vec<String>,
) -> Vec<BeeSession> {
    let dir = bee_dir.join("sessions");
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut entries: Vec<PathBuf> = match fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .collect(),
        Err(e) => {
            read_errors.push(format!(".bee/sessions: could not list ({e})"));
            Vec::new()
        }
    };
    entries.sort();

    let mut sessions = Vec::new();
    for path in entries {
        match parse_session(&path, now) {
            Ok(s) => sessions.push(s),
            Err(e) => read_errors.push(format!("{}: {e}", rel_str(&path, root))),
        }
    }
    sessions
}

/// Parse one `.bee/sessions/<uuid>.json` file. `transcript_path` is read
/// from the source JSON only to be discarded — it never reaches
/// [`BeeSession`], which has no field for it.
fn parse_session(path: &Path, now: time::OffsetDateTime) -> Result<BeeSession, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("could not read ({e})"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("could not parse ({e})"))?;

    let id = v.get("id").and_then(Value::as_str).ok_or("missing \"id\"")?.to_string();
    let heartbeat_str = v
        .get("last_heartbeat")
        .and_then(Value::as_str)
        .ok_or("missing \"last_heartbeat\"")?;
    let heartbeat = parse_rfc3339(heartbeat_str).ok_or("unparseable \"last_heartbeat\"")?;

    let started_at = v.get("started_at").and_then(Value::as_str).map(String::from);
    let workspace_id = v.get("workspace_id").and_then(Value::as_str).map(String::from);
    let source = v.get("source").and_then(Value::as_str).map(String::from);

    let heartbeat_age_minutes = (now - heartbeat).as_seconds_f64() / 60.0;
    let live = heartbeat_age_minutes <= SESSION_LIVE_MINUTES;

    Ok(BeeSession {
        id,
        started_at,
        heartbeat_age_minutes,
        live,
        workspace_id,
        source,
    })
}

/// Read `.bee/lanes/*.json` (D4). Absent (most projects never create it)
/// yields an empty list, not an error.
fn read_lanes(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> Vec<BeeLane> {
    let dir = bee_dir.join("lanes");
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut entries: Vec<PathBuf> = match fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .collect(),
        Err(e) => {
            read_errors.push(format!(".bee/lanes: could not list ({e})"));
            Vec::new()
        }
    };
    entries.sort();

    let mut lanes = Vec::new();
    for path in entries {
        match parse_lane(&path) {
            Ok(l) => lanes.push(l),
            Err(e) => read_errors.push(format!("{}: {e}", rel_str(&path, root))),
        }
    }
    lanes
}

fn parse_lane(path: &Path) -> Result<BeeLane, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("could not read ({e})"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("could not parse ({e})"))?;

    let feature = v
        .get("feature")
        .and_then(Value::as_str)
        .ok_or("missing \"feature\"")?
        .to_string();
    let phase = v.get("phase").and_then(Value::as_str).map(String::from);
    let mode = v.get("mode").and_then(Value::as_str).map(String::from);
    let next_action = v.get("next_action").and_then(Value::as_str).map(String::from);

    Ok(BeeLane { feature, phase, mode, next_action })
}

/// Read `.bee/runtime/workspaces/*.json` (D4). Absent yields an empty list.
fn read_workspaces(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> Vec<BeeWorkspace> {
    let dir = bee_dir.join("runtime").join("workspaces");
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut entries: Vec<PathBuf> = match fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .collect(),
        Err(e) => {
            read_errors.push(format!(".bee/runtime/workspaces: could not list ({e})"));
            Vec::new()
        }
    };
    entries.sort();

    let mut workspaces = Vec::new();
    for path in entries {
        match parse_workspace(&path, root) {
            Ok(w) => workspaces.push(w),
            Err(e) => read_errors.push(format!("{}: {e}", rel_str(&path, root))),
        }
    }
    workspaces
}

fn parse_workspace(path: &Path, root: &Path) -> Result<BeeWorkspace, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("could not read ({e})"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("could not parse ({e})"))?;

    let id = v.get("id").and_then(Value::as_str).ok_or("missing \"id\"")?.to_string();
    let kind = v.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
    let root_field = v
        .get("root")
        .and_then(Value::as_str)
        .map(|s| relativize(s, root))
        .unwrap_or_default();
    let branch = v.get("branch").and_then(Value::as_str).map(String::from);
    let attached_sessions = v
        .get("attached_sessions")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    let created_at = v.get("created_at").and_then(Value::as_str).map(String::from);

    Ok(BeeWorkspace {
        id,
        kind,
        root: root_field,
        branch,
        attached_sessions,
        created_at,
    })
}

/// Read `.bee/runtime/worktree-grants.json` (D4) and resolve each granted id
/// against its own sibling `.bee/` — see [`BeeWorktree`]. A missing file
/// yields an empty list, not an error, matching every other optional-file
/// precedent in this module (`.bee/lanes`, `.bee/runtime/workspaces`). A
/// present-but-malformed grants file (not valid JSON, or not a JSON object)
/// is a read error and also yields an empty list — that is the grants file
/// itself failing, distinct from one granted *id* being dangling, which
/// [`resolve_worktree`] reports per-entry instead.
fn read_worktrees(
    root: &Path,
    workspaces: &[BeeWorkspace],
    now: time::OffsetDateTime,
    read_errors: &mut Vec<String>,
) -> Vec<BeeWorktree> {
    let path = root.join(".bee").join("runtime").join("worktree-grants.json");
    if !path.is_file() {
        return Vec::new();
    }
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) => {
            read_errors.push(format!("{}: could not read ({e})", rel_str(&path, root)));
            return Vec::new();
        }
    };
    let v: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            read_errors.push(format!("{}: could not parse ({e})", rel_str(&path, root)));
            return Vec::new();
        }
    };
    let Some(obj) = v.as_object() else {
        read_errors.push(format!("{}: not a JSON object", rel_str(&path, root)));
        return Vec::new();
    };

    let mut out: Vec<BeeWorktree> = obj
        .iter()
        .filter(|(_, granted)| granted.as_bool() == Some(true))
        .map(|(id, _)| resolve_worktree(id, root, workspaces, now))
        .collect();

    // Live first (must-have), resolved before unresolved next, id as a
    // stable tiebreak so the order is deterministic across reads.
    out.sort_by(|a, b| {
        (!a.live, !a.resolved, a.id.as_str()).cmp(&(!b.live, !b.resolved, b.id.as_str()))
    });
    out
}

/// Resolve one granted worktree id against its own sibling directory, which
/// sits beside `project_root` (worktrees are siblings, per
/// `.bee/runtime/workspaces/<id>.json`'s own `root`, which this function
/// deliberately never reads — [`read_worktrees`] already has that project's
/// join value from `workspaces`). The sibling's absolute path is used only
/// to open files for reading (D4); it never survives into the returned
/// [`BeeWorktree`] — only `id`, already a safe name, is carried.
fn resolve_worktree(
    id: &str,
    project_root: &Path,
    workspaces: &[BeeWorkspace],
    now: time::OffsetDateTime,
) -> BeeWorktree {
    let workspace = workspaces.iter().find(|w| w.id == id);
    let branch = workspace.and_then(|w| w.branch.clone());
    let created_at = workspace.and_then(|w| w.created_at.clone());

    let Some(sibling_root) = project_root.parent().map(|p| p.join(id)) else {
        return BeeWorktree::unresolved(id, "project root has no parent directory", branch, created_at);
    };
    if !sibling_root.is_dir() {
        return BeeWorktree::unresolved(id, "worktree directory not found", branch, created_at);
    }

    let state_path = sibling_root.join(".bee").join("state.json");
    let raw = match fs::read_to_string(&state_path) {
        Ok(raw) => raw,
        Err(_) => {
            return BeeWorktree::unresolved(id, "state.json missing or unreadable", branch, created_at)
        }
    };
    let v: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return BeeWorktree::unresolved(id, "state.json could not be parsed", branch, created_at),
    };

    let feature = v.get("feature").and_then(Value::as_str).map(String::from);
    let phase = v.get("phase").and_then(Value::as_str).map(String::from);
    let mode = v.get("mode").and_then(Value::as_str).map(String::from);

    let (live, heartbeat_age_minutes) = worktree_liveness(&sibling_root, now);

    BeeWorktree {
        id: id.to_string(),
        resolved: true,
        unresolved_reason: None,
        feature,
        phase,
        mode,
        branch,
        created_at,
        live,
        heartbeat_age_minutes,
    }
}

/// The worktree's own `.bee/sessions/*.json` liveness (D4), reusing
/// [`parse_session`] and the same [`SESSION_LIVE_MINUTES`] window the main
/// store's own sessions already use. An absent or empty sessions directory
/// yields `(false, None)`, not an error — most worktrees genuinely have no
/// session recorded locally. When more than one session is live, the
/// freshest (smallest) heartbeat age wins.
fn worktree_liveness(sibling_root: &Path, now: time::OffsetDateTime) -> (bool, Option<f64>) {
    let dir = sibling_root.join(".bee").join("sessions");
    if !dir.is_dir() {
        return (false, None);
    }
    let Ok(rd) = fs::read_dir(&dir) else {
        return (false, None);
    };
    let mut freshest: Option<f64> = None;
    for entry in rd.filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(session) = parse_session(&p, now) {
            if session.live {
                freshest = Some(match freshest {
                    Some(cur) => cur.min(session.heartbeat_age_minutes),
                    None => session.heartbeat_age_minutes,
                });
            }
        }
    }
    (freshest.is_some(), freshest)
}

/// Read `.bee/decisions.jsonl` (D4). A missing file is a normal, expected
/// shape (silent, no error). The full event log is never held past this
/// function's local counting — only `total` and a bounded `recent` slice of
/// `decide` events survive into the returned [`BeeDecisions`].
fn read_decisions(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> BeeDecisions {
    let path = bee_dir.join("decisions.jsonl");
    if !path.is_file() {
        return BeeDecisions::default();
    }
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) => {
            read_errors.push(format!("{}: could not read ({e})", rel_str(&path, root)));
            return BeeDecisions::default();
        }
    };

    let mut total = 0usize;
    let mut recent_decides: Vec<BeeDecisionSummary> = Vec::new();

    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                read_errors.push(format!(
                    "{}: line {} could not parse ({e})",
                    rel_str(&path, root),
                    i + 1
                ));
                continue;
            }
        };
        total += 1;
        if v.get("type").and_then(Value::as_str) == Some("decide") {
            recent_decides.push(BeeDecisionSummary {
                id: v.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                date: v.get("date").and_then(Value::as_str).unwrap_or_default().to_string(),
                decision: v.get("decision").and_then(Value::as_str).unwrap_or_default().to_string(),
                scope: v.get("scope").and_then(Value::as_str).map(String::from),
            });
            // The file is append-ordered, so the tail is always the most
            // recent; trimming the head as we go keeps memory bounded even
            // against a log the size of the real 1831-event store instead
            // of accumulating every decide event before truncating once.
            if recent_decides.len() > RECENT_DETAIL_CAP {
                recent_decides.remove(0);
            }
        }
    }

    BeeDecisions { total, recent: recent_decides }
}

/// Group `cells` by `feature` and derive the D10/D11 shipped-feature view.
///
/// A feature whose live (non-dropped) set is empty — every one of its cells
/// is `dropped` — is skipped entirely: not shipped, not counted. A feature
/// is shipped when every remaining live cell is `capped` (D10); a worktree
/// merge is never consulted here, matching `no_merge_lookup`.
fn compute_shipped_features(cells: &[BeeCell]) -> Vec<BeeShippedFeature> {
    let mut by_feature: std::collections::BTreeMap<&str, Vec<&BeeCell>> =
        std::collections::BTreeMap::new();
    for cell in cells {
        by_feature.entry(cell.feature.as_str()).or_default().push(cell);
    }

    let mut shipped = Vec::new();
    for (name, group) in by_feature {
        let live: Vec<&BeeCell> = group.into_iter().filter(|c| c.status != "dropped").collect();
        if live.is_empty() {
            // All-dropped feature: not shipped, not counted.
            continue;
        }
        if !live.iter().all(|c| c.status == "capped") {
            continue;
        }
        shipped.push(BeeShippedFeature {
            feature: name.to_string(),
            cell_count: live.len(),
            cycle_time: compute_cycle_time(&live),
        });
    }
    shipped
}

/// Earliest `claimed_at` to latest `capped_at` across `live` (D11). `None`
/// when either endpoint has no parseable timestamp — never a guessed zero.
fn compute_cycle_time(live: &[&BeeCell]) -> Option<BeeCycleSpan> {
    let starts: Vec<(&str, time::OffsetDateTime)> = live
        .iter()
        .filter_map(|c| c.claimed_at.as_deref())
        .filter_map(|s| parse_rfc3339(s).map(|t| (s, t)))
        .collect();
    let ends: Vec<(&str, time::OffsetDateTime)> = live
        .iter()
        .filter_map(|c| c.capped_at.as_deref())
        .filter_map(|s| parse_rfc3339(s).map(|t| (s, t)))
        .collect();

    let (start_str, start_t) = starts.iter().min_by_key(|(_, t)| t.unix_timestamp_nanos())?;
    let (end_str, end_t) = ends.iter().max_by_key(|(_, t)| t.unix_timestamp_nanos())?;

    let hours = (*end_t - *start_t).as_seconds_f64() / 3600.0;
    Some(BeeCycleSpan {
        started_at: (*start_str).to_string(),
        ended_at: (*end_str).to_string(),
        hours,
    })
}

/// Parse an RFC 3339 timestamp (as bee's `trace` fields carry it). Anything
/// unparseable is treated as absent rather than aborting the read.
fn parse_rfc3339(s: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}

/// The `YYYY-MM-DD` UTC calendar day of `dt`.
fn ymd_utc(dt: time::OffsetDateTime) -> String {
    let utc = dt.to_offset(time::UtcOffset::UTC);
    format!("{:04}-{:02}-{:02}", utc.year(), utc.month() as u8, utc.day())
}

/// Ship-rate aggregates over the shipped features that report a cycle time
/// (D11). A shipped feature with no cycle time cannot be placed on a
/// calendar day, so it contributes to `shipped` but not to any of these
/// numbers; every division here is guarded against an empty denominator.
fn compute_velocity(shipped: &[BeeShippedFeature]) -> BeeVelocity {
    let timed: Vec<&BeeShippedFeature> = shipped.iter().filter(|f| f.cycle_time.is_some()).collect();
    if timed.is_empty() {
        return BeeVelocity::default();
    }

    let mut per_day: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut hours: Vec<f64> = Vec::new();
    for f in &timed {
        let span = f.cycle_time.as_ref().expect("filtered to Some above");
        // ended_at was itself parsed successfully to build `span`, so
        // reparsing it here for its calendar day cannot fail in practice;
        // an unparseable string still degrades to "no day" rather than a
        // panic, matching the module's read-degrades-gracefully stance.
        if let Some(end_t) = parse_rfc3339(&span.ended_at) {
            *per_day.entry(ymd_utc(end_t)).or_insert(0) += 1;
        }
        hours.push(span.hours);
    }

    let active_days = per_day.len();
    let features_per_active_day = if active_days == 0 {
        None
    } else {
        Some(timed.len() as f64 / active_days as f64)
    };

    let features_per_week = match (per_day.keys().next(), per_day.keys().next_back()) {
        (Some(first), Some(last)) => {
            let first_jd = parse_ymd(first).map(|d| d.to_julian_day());
            let last_jd = parse_ymd(last).map(|d| d.to_julian_day());
            match (first_jd, last_jd) {
                (Some(first_jd), Some(last_jd)) => {
                    let span_days = (last_jd - first_jd + 1).max(1) as f64;
                    Some(timed.len() as f64 * 7.0 / span_days)
                }
                _ => None,
            }
        }
        _ => None,
    };

    BeeVelocity {
        per_day: per_day.into_iter().map(|(day, count)| BeeDayCount { day, count }).collect(),
        active_days,
        features_per_active_day,
        features_per_week,
        median_cycle_time_hours: median(hours),
    }
}

/// Parse a `YYYY-MM-DD` string (as produced by [`ymd_utc`]) back into a
/// [`time::Date`] for calendar-span arithmetic.
fn parse_ymd(s: &str) -> Option<time::Date> {
    let mut parts = s.splitn(3, '-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    let month = time::Month::try_from(month).ok()?;
    time::Date::from_calendar_date(year, month, day).ok()
}

/// The median of `values`. `None` for an empty slice.
fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).expect("cycle-time hours are always finite"));
    let n = values.len();
    if n % 2 == 1 {
        Some(values[n / 2])
    } else {
        Some((values[n / 2 - 1] + values[n / 2]) / 2.0)
    }
}

/// Render `s` relative to `root` when it names a path under `root`. When `s`
/// is not absolute it is returned unchanged (the common case — most
/// path-shaped fields, like `trace.worker`, are plain identifiers, not
/// paths). When `s` is absolute but falls outside `root`, it is reduced to
/// its bare filename so no absolute prefix of any kind survives into a
/// public field.
fn relativize(s: &str, root: &Path) -> String {
    let p = Path::new(s);
    if !p.is_absolute() {
        return s.to_string();
    }
    match p.strip_prefix(root) {
        Ok(rel) => to_forward_slashes(rel),
        Err(_) => p
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(absolute path redacted)".to_string()),
    }
}

/// Render a path known to be a descendant of `root` relative to it.
fn rel_str(path: &Path, root: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) => to_forward_slashes(rel),
        Err(_) => path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(absolute path redacted)".to_string()),
    }
}

fn to_forward_slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn cell_json(id: &str, status: &str) -> String {
        format!(
            r#"{{
                "id": "{id}",
                "feature": "demo",
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
                "status": "{status}",
                "tier": "generation",
                "trace": {{"worker": "w1"}}
            }}"#
        )
    }

    fn fresh_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mdview-bee-{name}-{}-{}",
            std::process::id(),
            name.len() // cheap per-name salt, keeps directories distinct across test fns
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Recursively collect (relative path, content bytes) for everything
    /// under `dir`, for the D4 read-only probe.
    fn snapshot_tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
        fn walk(base: &Path, cur: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for entry in std::fs::read_dir(cur).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    let rel = path.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                    let content = std::fs::read(&path).unwrap();
                    out.push((rel, content));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[test]
    fn buckets_all_five_statuses_dropped_absent() {
        let root = fresh_root("all-statuses");
        write(&root, ".bee/state.json", r#"{"phase":"swarming","feature":"demo","mode":"standard"}"#);
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));
        write(&root, ".bee/cells/c-claimed.json", &cell_json("c-claimed", "claimed"));
        write(&root, ".bee/cells/c-blocked.json", &cell_json("c-blocked", "blocked"));
        write(&root, ".bee/cells/c-capped.json", &cell_json("c-capped", "capped"));
        write(&root, ".bee/cells/c-dropped.json", &cell_json("c-dropped", "dropped"));

        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert_eq!(snap.buckets.doing.len(), 1);
        assert_eq!(snap.buckets.waiting.len(), 1);
        assert_eq!(snap.buckets.stuck.len(), 1);
        assert_eq!(snap.buckets.done.len(), 1);
        assert_eq!(snap.state.as_ref().unwrap().phase.as_deref(), Some("swarming"));

        let all_ids: Vec<&str> = snap
            .buckets
            .doing
            .iter()
            .chain(&snap.buckets.waiting)
            .chain(&snap.buckets.stuck)
            .chain(&snap.buckets.done)
            .map(|c| c.id.as_str())
            .collect();
        assert!(!all_ids.contains(&"c-dropped"), "dropped cell leaked into a bucket: {all_ids:?}");
        assert_eq!(all_ids.len(), 4);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn active_true_with_open_or_claimed_false_otherwise() {
        let active_root = fresh_root("active-yes");
        write(&active_root, ".bee/cells/a.json", &cell_json("a", "open"));
        assert!(read_snapshot(&active_root).active);
        std::fs::remove_dir_all(&active_root).ok();

        let inactive_root = fresh_root("active-no");
        write(&inactive_root, ".bee/cells/a.json", &cell_json("a", "capped"));
        write(&inactive_root, ".bee/cells/b.json", &cell_json("b", "dropped"));
        assert!(!read_snapshot(&inactive_root).active);
        std::fs::remove_dir_all(&inactive_root).ok();
    }

    #[test]
    fn bee_dir_absent_is_reported_not_error() {
        let root = fresh_root("no-bee");
        // no .bee/ created at all
        let snap = read_snapshot(&root);
        assert!(!snap.present);
        assert!(!snap.active);
        assert_eq!(snap.buckets.doing.len(), 0);
        assert_eq!(snap.buckets.waiting.len(), 0);
        assert_eq!(snap.buckets.stuck.len(), 0);
        assert_eq!(snap.buckets.done.len(), 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_cells_dir_yields_four_zero_buckets() {
        let root = fresh_root("empty-cells");
        std::fs::create_dir_all(root.join(".bee/cells")).unwrap();
        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert_eq!(snap.buckets.doing.len(), 0);
        assert_eq!(snap.buckets.waiting.len(), 0);
        assert_eq!(snap.buckets.stuck.len(), 0);
        assert_eq!(snap.buckets.done.len(), 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unknown_status_counted_nowhere_read_still_succeeds() {
        let root = fresh_root("unknown-status");
        write(&root, ".bee/cells/weird.json", &cell_json("weird", "quarantined"));
        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert!(snap.read_errors.is_empty(), "unknown status should not be a read error: {:?}", snap.read_errors);
        assert_eq!(snap.buckets.doing.len(), 0);
        assert_eq!(snap.buckets.waiting.len(), 0);
        assert_eq!(snap.buckets.stuck.len(), 0);
        assert_eq!(snap.buckets.done.len(), 0);
        assert!(!snap.active);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn archived_cells_contribute_to_no_count() {
        let root = fresh_root("archive");
        write(&root, ".bee/cells/live.json", &cell_json("live", "capped"));
        write(
            &root,
            ".bee/cells/archive/demo/archived-1.json",
            &cell_json("archived-1", "capped"),
        );
        write(
            &root,
            ".bee/cells/archive/demo/archived-2.json",
            &cell_json("archived-2", "open"),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.buckets.done.len(), 1, "only the live capped cell should count");
        assert_eq!(snap.buckets.doing.len(), 0);
        assert_eq!(snap.buckets.waiting.len(), 0);
        assert!(!snap.active, "the archived open cell must not flip active");
        assert!(snap.buckets.done.iter().all(|c| c.id == "live"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_state_and_truncated_cell_degrade_to_partial_snapshot() {
        let root = fresh_root("malformed");
        write(&root, ".bee/state.json", "{ this is not valid json");
        write(&root, ".bee/cells/good.json", &cell_json("good", "open"));
        write(&root, ".bee/cells/bad.json", "{\"id\": \"bad\", \"status\": \"open\"");

        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert!(snap.state.is_none());
        assert_eq!(snap.buckets.waiting.len(), 1, "the well-formed cell must still parse");
        assert_eq!(snap.buckets.waiting[0].id, "good");
        assert_eq!(snap.read_errors.len(), 2, "expected notes for state.json and bad.json: {:?}", snap.read_errors);
        assert!(snap.read_errors.iter().any(|e| e.contains("state.json")));
        assert!(snap.read_errors.iter().any(|e| e.contains("bad.json")));
        // every read_errors entry must itself be relative
        for e in &snap.read_errors {
            assert!(!e.contains(&root.to_string_lossy().into_owned()), "read_errors leaked the fixture root: {e}");
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_absolute_path_survives_into_public_fields() {
        let root = fresh_root("security");
        let root_str = root.to_string_lossy().into_owned();
        let outside_abs = std::env::temp_dir()
            .join("mdview-bee-outside-file.rs")
            .to_string_lossy()
            .into_owned();
        let inside_abs = root.join("src/inside.rs").to_string_lossy().into_owned();
        let worker_abs = root.join("workers/reader-1").to_string_lossy().into_owned();

        let body = format!(
            r#"{{
                "id": "leaky",
                "feature": "demo",
                "lane": "standard",
                "title": "Leaky cell",
                "action": "x",
                "verify": "x",
                "files": ["{}", "{}"],
                "read_first": [],
                "deps": [],
                "decisions": [],
                "must_haves": {{}},
                "behavior_change": false,
                "change_class": "behavior",
                "pbi": null,
                "status": "open",
                "tier": "generation",
                "trace": {{"worker": "{}"}}
            }}"#,
            inside_abs.replace('\\', "\\\\"),
            outside_abs.replace('\\', "\\\\"),
            worker_abs.replace('\\', "\\\\"),
        );
        write(&root, ".bee/cells/leaky.json", &body);

        let snap = read_snapshot(&root);
        assert_eq!(snap.buckets.waiting.len(), 1);
        let cell = &snap.buckets.waiting[0];

        for f in &cell.files {
            assert!(!Path::new(f).is_absolute(), "leaked absolute path in files[]: {f}");
            assert!(!f.contains(&root_str), "leaked fixture root in files[]: {f}");
        }
        let worker = cell.worker.as_deref().unwrap_or_default();
        assert!(!Path::new(worker).is_absolute(), "leaked absolute path in worker: {worker}");
        assert!(!worker.contains(&root_str), "leaked fixture root in worker: {worker}");

        // the in-root file must have relativized cleanly (not just filename-reduced)
        assert!(cell.files.iter().any(|f| f == "src/inside.rs"), "files: {:?}", cell.files);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reading_never_writes_the_bee_tree() {
        let root = fresh_root("read-only");
        write(&root, ".bee/state.json", r#"{"phase":"swarming"}"#);
        write(&root, ".bee/cells/a.json", &cell_json("a", "open"));
        write(&root, ".bee/cells/archive/demo/z.json", &cell_json("z", "capped"));

        let before = snapshot_tree(&root);
        let _ = read_snapshot(&root);
        let after = snapshot_tree(&root);

        assert_eq!(before, after, ".bee/ tree changed after a read");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_web_framework_dependency_declared() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
        for forbidden in ["axum", "tokio", "hyper"] {
            assert!(
                !manifest.lines().any(|l| l.trim_start().starts_with(forbidden)),
                "mdview-core/Cargo.toml must not depend on {forbidden}"
            );
        }
    }

    // --- bee-cockpit-3: shipped features, cycle time, velocity (D10/D11) ---

    fn feature_cell_json(
        id: &str,
        feature: &str,
        status: &str,
        claimed_at: Option<&str>,
        capped_at: Option<&str>,
    ) -> String {
        let claimed_json = claimed_at.map(|s| format!("\"{s}\"")).unwrap_or_else(|| "null".to_string());
        let capped_json = capped_at.map(|s| format!("\"{s}\"")).unwrap_or_else(|| "null".to_string());
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
                "status": "{status}",
                "tier": "generation",
                "trace": {{"worker": "w1", "claimed_at": {claimed_json}, "capped_at": {capped_json}}}
            }}"#
        )
    }

    #[test]
    fn shipped_feature_all_capped_reports_cycle_time() {
        let root = fresh_root("shipped-simple");
        write(
            &root,
            ".bee/cells/f-1.json",
            &feature_cell_json(
                "f-1",
                "feat-a",
                "capped",
                Some("2026-08-01T00:00:00.000Z"),
                Some("2026-08-01T02:00:00.000Z"),
            ),
        );
        write(
            &root,
            ".bee/cells/f-2.json",
            &feature_cell_json(
                "f-2",
                "feat-a",
                "capped",
                Some("2026-08-01T01:00:00.000Z"),
                Some("2026-08-01T04:00:00.000Z"),
            ),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.shipped.len(), 1);
        let f = &snap.shipped[0];
        assert_eq!(f.feature, "feat-a");
        assert_eq!(f.cell_count, 2);
        let ct = f.cycle_time.as_ref().expect("both timestamps present, cycle time expected");
        assert_eq!(ct.started_at, "2026-08-01T00:00:00.000Z", "must be the earliest claim");
        assert_eq!(ct.ended_at, "2026-08-01T04:00:00.000Z", "must be the latest cap");
        assert!((ct.hours - 4.0).abs() < 1e-9, "hours: {}", ct.hours);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn shipped_feature_with_dropped_cell_still_ships_per_d10() {
        let root = fresh_root("shipped-dropped-mix");
        write(
            &root,
            ".bee/cells/f-1.json",
            &feature_cell_json(
                "f-1",
                "feat-b",
                "capped",
                Some("2026-08-01T00:00:00.000Z"),
                Some("2026-08-01T01:00:00.000Z"),
            ),
        );
        // A dropped cell must never block shipped status, and its own
        // (earlier) claimed_at must not leak into the span.
        write(
            &root,
            ".bee/cells/f-2.json",
            &feature_cell_json("f-2", "feat-b", "dropped", Some("2025-01-01T00:00:00.000Z"), None),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.shipped.len(), 1, "feature with capped+dropped cells must be shipped: {:?}", snap.shipped);
        let f = &snap.shipped[0];
        assert_eq!(f.feature, "feat-b");
        assert_eq!(f.cell_count, 1, "the dropped cell must not count toward cell_count");
        let ct = f.cycle_time.as_ref().expect("cycle time expected from the one live cell");
        assert_eq!(ct.started_at, "2026-08-01T00:00:00.000Z", "dropped cell's timestamp must not be used");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn velocity_aggregates_per_day_active_day_and_median() {
        let root = fresh_root("velocity-aggregate");
        // Two features ship on 2026-08-01, one on 2026-08-02.
        write(
            &root,
            ".bee/cells/x1.json",
            &feature_cell_json(
                "x1",
                "feat-x",
                "capped",
                Some("2026-08-01T00:00:00.000Z"),
                Some("2026-08-01T01:00:00.000Z"),
            ),
        );
        write(
            &root,
            ".bee/cells/y1.json",
            &feature_cell_json(
                "y1",
                "feat-y",
                "capped",
                Some("2026-08-01T02:00:00.000Z"),
                Some("2026-08-01T03:00:00.000Z"),
            ),
        );
        write(
            &root,
            ".bee/cells/z1.json",
            &feature_cell_json(
                "z1",
                "feat-z",
                "capped",
                Some("2026-08-02T00:00:00.000Z"),
                Some("2026-08-02T01:00:00.000Z"),
            ),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.shipped.len(), 3);

        let vel = &snap.velocity;
        assert_eq!(vel.per_day.len(), 2);
        assert_eq!(vel.per_day[0].day, "2026-08-01");
        assert_eq!(vel.per_day[0].count, 2);
        assert_eq!(vel.per_day[1].day, "2026-08-02");
        assert_eq!(vel.per_day[1].count, 1);
        assert_eq!(vel.active_days, 2);
        assert!((vel.features_per_active_day.unwrap() - 1.5).abs() < 1e-9);
        // calendar span 2026-08-01..=2026-08-02 is 2 days -> 3 features / (2/7 weeks)
        let expected_per_week = 3.0 * 7.0 / 2.0;
        assert!((vel.features_per_week.unwrap() - expected_per_week).abs() < 1e-9);
        // each feature's cycle time is exactly 1h -> median is 1h
        assert!((vel.median_cycle_time_hours.unwrap() - 1.0).abs() < 1e-9);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn feature_with_one_open_cell_is_not_shipped() {
        let root = fresh_root("not-shipped-open");
        write(
            &root,
            ".bee/cells/a.json",
            &feature_cell_json(
                "a",
                "feat-open",
                "capped",
                Some("2026-08-01T00:00:00.000Z"),
                Some("2026-08-01T01:00:00.000Z"),
            ),
        );
        write(&root, ".bee/cells/b.json", &feature_cell_json("b", "feat-open", "open", None, None));

        let snap = read_snapshot(&root);
        assert!(
            snap.shipped.iter().all(|f| f.feature != "feat-open"),
            "a feature with one open cell must not be shipped: {:?}",
            snap.shipped
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn feature_with_only_dropped_cells_is_not_shipped() {
        let root = fresh_root("all-dropped-feature");
        write(
            &root,
            ".bee/cells/a.json",
            &feature_cell_json("a", "feat-dead", "dropped", Some("2026-08-01T00:00:00.000Z"), None),
        );
        write(&root, ".bee/cells/b.json", &feature_cell_json("b", "feat-dead", "dropped", None, None));

        let snap = read_snapshot(&root);
        assert!(
            snap.shipped.is_empty(),
            "a feature whose cells are all dropped must not be shipped: {:?}",
            snap.shipped
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn shipped_feature_missing_a_timestamp_reports_no_cycle_time() {
        let root = fresh_root("missing-timestamp");
        // Both cells are capped, but neither carries a claimed_at anywhere
        // in the feature - the start endpoint is entirely absent.
        write(
            &root,
            ".bee/cells/a.json",
            &feature_cell_json("a", "feat-notime", "capped", None, Some("2026-08-01T01:00:00.000Z")),
        );
        write(
            &root,
            ".bee/cells/b.json",
            &feature_cell_json("b", "feat-notime", "capped", None, Some("2026-08-01T02:00:00.000Z")),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.shipped.len(), 1);
        assert_eq!(snap.shipped[0].feature, "feat-notime");
        assert!(
            snap.shipped[0].cycle_time.is_none(),
            "missing claimed_at across the whole feature must yield no cycle time, not a zero: {:?}",
            snap.shipped[0].cycle_time
        );
        // must not silently contribute a fabricated day/rate either
        assert!(snap.velocity.per_day.is_empty());
        assert_eq!(snap.velocity.active_days, 0);
        assert!(snap.velocity.features_per_active_day.is_none());
        assert!(snap.velocity.features_per_week.is_none());
        assert!(snap.velocity.median_cycle_time_hours.is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_cell_store_yields_zero_shipped_and_no_division_by_zero() {
        let root = fresh_root("empty-store-velocity");
        std::fs::create_dir_all(root.join(".bee/cells")).unwrap();

        let snap = read_snapshot(&root);
        assert!(snap.shipped.is_empty());
        assert!(snap.velocity.per_day.is_empty());
        assert_eq!(snap.velocity.active_days, 0);
        assert!(snap.velocity.features_per_active_day.is_none());
        assert!(snap.velocity.features_per_week.is_none());
        assert!(snap.velocity.median_cycle_time_hours.is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn d7_buckets_and_d8_active_unchanged_by_feature_view() {
        // Regression: adding the feature/shipped view must not perturb the
        // bee-cockpit-1 bucket/active behavior it builds on top of.
        let root = fresh_root("regression-buckets");
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));
        write(&root, ".bee/cells/c-claimed.json", &cell_json("c-claimed", "claimed"));
        write(&root, ".bee/cells/c-blocked.json", &cell_json("c-blocked", "blocked"));
        write(&root, ".bee/cells/c-capped.json", &cell_json("c-capped", "capped"));
        write(&root, ".bee/cells/c-dropped.json", &cell_json("c-dropped", "dropped"));

        let snap = read_snapshot(&root);
        assert_eq!(snap.buckets.doing.len(), 1);
        assert_eq!(snap.buckets.waiting.len(), 1);
        assert_eq!(snap.buckets.stuck.len(), 1);
        assert_eq!(snap.buckets.done.len(), 1);
        assert!(snap.active, "an open and a claimed cell must still flip active (D8)");

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bee-cockpit-5: backlog, sessions, lanes, workspaces, decisions ---

    #[test]
    fn pbi_folds_to_last_status_not_first() {
        let root = fresh_root("pbi-fold");
        let lines = [
            r#"{"kind":"pbi","id":"P1","title":"Widget","status":"proposed","feature":"demo"}"#,
            r#"{"kind":"pbi","id":"P1","title":"Widget","status":"in-flight","feature":"demo"}"#,
            r#"{"kind":"pbi","id":"P1","title":"Widget","status":"done","feature":"demo"}"#,
        ];
        write(&root, ".bee/backlog.jsonl", &lines.join("\n"));

        let snap = read_snapshot(&root);
        assert_eq!(
            snap.backlog.pbis.len(),
            1,
            "repeated events for one id must fold to a single PBI: {:?}",
            snap.backlog.pbis
        );
        assert_eq!(snap.backlog.pbis[0].status, "done", "must fold to the LAST status, not the first");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn findings_grouped_by_severity_with_correct_counts() {
        let root = fresh_root("findings-severity");
        let lines = [
            r#"{"ts":"2026-08-01T00:00:00.000Z","type":"finding","title":"a","detail":"d","severity":"P1","layer":"l","feature":"f"}"#,
            r#"{"ts":"2026-08-01T00:00:01.000Z","type":"finding","title":"b","detail":"d","severity":"P2","layer":"l","feature":"f"}"#,
            r#"{"ts":"2026-08-01T00:00:02.000Z","type":"finding","title":"c","detail":"d","severity":"P2","layer":"l","feature":"f"}"#,
            r#"{"ts":"2026-08-01T00:00:03.000Z","type":"finding","title":"e","detail":"d","severity":"P3","layer":"l","feature":"f"}"#,
        ];
        write(&root, ".bee/backlog.jsonl", &lines.join("\n"));

        let snap = read_snapshot(&root);
        assert_eq!(snap.backlog.findings.total, 4);
        assert_eq!(snap.backlog.findings.by_severity.p1, 1);
        assert_eq!(snap.backlog.findings.by_severity.p2, 2);
        assert_eq!(snap.backlog.findings.by_severity.p3, 1);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn session_heartbeat_recent_is_live_hour_old_is_stale() {
        let root = fresh_root("session-liveness");
        let now = time::OffsetDateTime::now_utc();
        let fmt = &time::format_description::well_known::Rfc3339;
        let recent = (now - time::Duration::minutes(5)).format(fmt).unwrap();
        let old = (now - time::Duration::hours(1)).format(fmt).unwrap();

        write(
            &root,
            ".bee/sessions/live.json",
            &format!(
                r#"{{"id":"live","started_at":"{recent}","last_heartbeat":"{recent}","transcript_path":"/home/someone/.claude/x.jsonl","workspace_id":"main","source":"startup"}}"#
            ),
        );
        write(
            &root,
            ".bee/sessions/stale.json",
            &format!(
                r#"{{"id":"stale","started_at":"{old}","last_heartbeat":"{old}","transcript_path":"/home/someone/.claude/y.jsonl","workspace_id":"main","source":"clear"}}"#
            ),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.sessions.len(), 2);
        let live = snap.sessions.iter().find(|s| s.id == "live").unwrap();
        let stale = snap.sessions.iter().find(|s| s.id == "stale").unwrap();
        assert!(live.live, "a 5-minute-old heartbeat must be live: age={}", live.heartbeat_age_minutes);
        assert!(live.heartbeat_age_minutes < 30.0);
        assert!(!stale.live, "a 1-hour-old heartbeat must be stale: age={}", stale.heartbeat_age_minutes);
        assert!(stale.heartbeat_age_minutes > 30.0);

        std::fs::remove_dir_all(&root).ok();
    }

    // --- running_workers: the in-flight view joined from state.json's
    // workers[], live cells and live sessions ---

    fn session_json_with_age(id: &str, minutes_ago: i64) -> String {
        let now = time::OffsetDateTime::now_utc();
        let hb = (now - time::Duration::minutes(minutes_ago))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        format!(r#"{{"id":"{id}","started_at":"{hb}","last_heartbeat":"{hb}","workspace_id":"main","source":"startup"}}"#)
    }

    #[test]
    fn running_worker_with_live_session_and_claimed_cell_has_no_discrepancy() {
        let root = fresh_root("running-happy");
        write(&root, ".bee/cells/kf-1.json", &cell_json("kf-1", "claimed"));
        write(
            &root,
            ".bee/state.json",
            r#"{"phase":"exploring","workers":[{"nickname":"kf1-worker","cell":"kf-1","tier":"generation","status":"running"}]}"#,
        );
        write(&root, ".bee/sessions/kf1-worker.json", &session_json_with_age("kf1-worker", 1));

        let snap = read_snapshot(&root);
        assert_eq!(snap.running_workers.len(), 1, "{:?}", snap.running_workers);
        let w = &snap.running_workers[0];
        assert_eq!(w.nickname, "kf1-worker");
        assert_eq!(w.cell.as_deref(), Some("kf-1"));
        assert!(w.cell_found);
        assert_eq!(w.cell_status.as_deref(), Some("claimed"));
        assert!(!w.discrepancy, "a claimed cell backing a live worker must not be a discrepancy");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn running_worker_named_cell_the_store_still_calls_open_is_a_discrepancy() {
        // The exact shape reported live: a worker names a cell, a session
        // shares its nickname and is live, yet the cell file itself is
        // still "open" — the store and the running process disagree.
        let root = fresh_root("running-discrepancy");
        write(&root, ".bee/cells/kf-1.json", &cell_json("kf-1", "open"));
        write(
            &root,
            ".bee/state.json",
            r#"{"workers":[{"nickname":"kf1-worker","cell":"kf-1","tier":null,"status":null}]}"#,
        );
        write(&root, ".bee/sessions/kf1-worker.json", &session_json_with_age("kf1-worker", 1));

        let snap = read_snapshot(&root);
        assert_eq!(snap.running_workers.len(), 1, "{:?}", snap.running_workers);
        let w = &snap.running_workers[0];
        assert!(w.cell_found);
        assert_eq!(w.cell_status.as_deref(), Some("open"));
        assert!(w.discrepancy, "a worker naming a still-open cell must be flagged");

        // D7: the cell must still land in Waiting, never moved to Doing by
        // the presence of a worker naming it.
        assert_eq!(snap.buckets.waiting.len(), 1);
        assert_eq!(snap.buckets.doing.len(), 0);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn running_worker_naming_nonexistent_cell_is_flagged_not_dropped() {
        let root = fresh_root("running-no-cell");
        write(
            &root,
            ".bee/state.json",
            r#"{"workers":[{"nickname":"ghost-worker","cell":"does-not-exist","tier":"generation","status":"running"}]}"#,
        );
        write(&root, ".bee/sessions/ghost-worker.json", &session_json_with_age("ghost-worker", 1));

        let snap = read_snapshot(&root);
        assert_eq!(snap.running_workers.len(), 1, "a worker naming an unknown cell must not be dropped: {:?}", snap.running_workers);
        let w = &snap.running_workers[0];
        assert!(!w.cell_found);
        assert!(w.cell_status.is_none());
        assert!(w.discrepancy, "a worker naming a nonexistent cell must be a discrepancy");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn running_worker_with_stale_session_is_absent() {
        let root = fresh_root("running-stale");
        write(&root, ".bee/cells/kl-1.json", &cell_json("kl-1", "claimed"));
        write(
            &root,
            ".bee/state.json",
            r#"{"workers":[{"nickname":"kl1-worker","cell":"kl-1","tier":"generation","status":"running"}]}"#,
        );
        // 1 hour old: stale per SESSION_LIVE_MINUTES (30).
        write(&root, ".bee/sessions/kl1-worker.json", &session_json_with_age("kl1-worker", 60));

        let snap = read_snapshot(&root);
        assert!(
            snap.running_workers.is_empty(),
            "a worker backed only by a stale session must not be presented as running: {:?}",
            snap.running_workers
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn worker_with_no_matching_session_is_absent_from_running() {
        let root = fresh_root("running-no-session");
        write(&root, ".bee/cells/kl-2.json", &cell_json("kl-2", "claimed"));
        write(
            &root,
            ".bee/state.json",
            r#"{"workers":[{"nickname":"kl2-worker","cell":"kl-2","tier":"generation","status":"running"}]}"#,
        );
        // No .bee/sessions/kl2-worker.json at all.

        let snap = read_snapshot(&root);
        assert!(
            snap.running_workers.is_empty(),
            "a worker with no backing session must not be presented as running: {:?}",
            snap.running_workers
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn backlog_absent_is_empty_not_error() {
        let root = fresh_root("backlog-absent");
        let snap = read_snapshot(&root);
        assert!(snap.backlog.pbis.is_empty());
        assert_eq!(snap.backlog.findings.total, 0);
        assert!(snap.read_errors.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn backlog_empty_file_is_empty_not_error() {
        let root = fresh_root("backlog-empty");
        write(&root, ".bee/backlog.jsonl", "");
        let snap = read_snapshot(&root);
        assert!(snap.backlog.pbis.is_empty());
        assert_eq!(snap.backlog.findings.total, 0);
        assert!(snap.read_errors.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn backlog_one_malformed_line_degrades_without_losing_good_rows() {
        let root = fresh_root("backlog-malformed");
        let lines = [
            r#"{"kind":"pbi","id":"P1","title":"Good one","status":"in-flight","feature":"demo"}"#.to_string(),
            "{ this is not valid json".to_string(),
            r#"{"ts":"2026-08-01T00:00:00.000Z","type":"finding","title":"Also good","detail":"d","severity":"P1","layer":"l","feature":"demo"}"#
                .to_string(),
        ];
        write(&root, ".bee/backlog.jsonl", &lines.join("\n"));

        let snap = read_snapshot(&root);
        assert_eq!(snap.backlog.pbis.len(), 1, "the good pbi row must survive: {:?}", snap.backlog.pbis);
        assert_eq!(snap.backlog.findings.total, 1, "the good finding row must survive");
        assert!(
            snap.read_errors.iter().any(|e| e.contains("backlog.jsonl")),
            "the malformed line must be noted: {:?}",
            snap.read_errors
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn backlog_findings_recent_capped_but_total_reports_all() {
        let root = fresh_root("backlog-cap");
        let n = RECENT_DETAIL_CAP + 7;
        let lines: Vec<String> = (0..n)
            .map(|i| {
                format!(
                    r#"{{"ts":"2026-08-01T{:02}:00:{:02}.000Z","type":"finding","title":"f{i}","detail":"d","severity":"P2","layer":"l","feature":"demo"}}"#,
                    (i / 60) % 24,
                    i % 60
                )
            })
            .collect();
        write(&root, ".bee/backlog.jsonl", &lines.join("\n"));

        let snap = read_snapshot(&root);
        assert_eq!(snap.backlog.findings.total, n, "the true total must be reported: {}", snap.backlog.findings.total);
        assert_eq!(snap.backlog.findings.recent.len(), RECENT_DETAIL_CAP, "recent findings must be capped");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn decisions_recent_capped_but_total_reports_all_events() {
        let root = fresh_root("decisions-cap");
        let n = RECENT_DETAIL_CAP + 5;
        let mut lines: Vec<String> = (0..n)
            .map(|i| {
                format!(
                    r#"{{"id":"d{i}","type":"decide","date":"2026-08-01T00:{:02}:00.000Z","decision":"Decision {i}","rationale":null,"alternatives":null,"scope":"repo","source":"user","confidence":0}}"#,
                    i % 60
                )
            })
            .collect();
        // A non-decide event must still count toward the true total.
        lines.push(r#"{"id":"tag-1","type":"tag","date":"2026-08-01T00:01:00.000Z"}"#.to_string());
        write(&root, ".bee/decisions.jsonl", &lines.join("\n"));

        let snap = read_snapshot(&root);
        assert_eq!(snap.decisions.total, n + 1, "the true total (every event type) must be reported");
        assert_eq!(snap.decisions.recent.len(), RECENT_DETAIL_CAP, "recent decide events must be capped");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn lanes_dir_absent_yields_empty_list_not_error() {
        let root = fresh_root("lanes-absent");
        write(&root, ".bee/state.json", r#"{"phase":"swarming"}"#);
        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert!(snap.lanes.is_empty());
        assert!(
            snap.read_errors.iter().all(|e| !e.contains("lanes")),
            "an absent .bee/lanes/ must not be a read error: {:?}",
            snap.read_errors
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn lane_record_is_read_when_present() {
        let root = fresh_root("lanes-present");
        write(
            &root,
            ".bee/lanes/demo.json",
            r#"{"schema_version":"1.0","feature":"demo","mode":"standard","phase":"swarming","next_action":"Execute c-1."}"#,
        );
        let snap = read_snapshot(&root);
        assert_eq!(snap.lanes.len(), 1);
        assert_eq!(snap.lanes[0].feature, "demo");
        assert_eq!(snap.lanes[0].phase.as_deref(), Some("swarming"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workspace_root_is_relativized_or_reduced_never_absolute() {
        let root = fresh_root("workspace-abs");
        let sibling = std::env::temp_dir()
            .join("mdview-bee-workspace-outside-root")
            .to_string_lossy()
            .into_owned();
        write(
            &root,
            ".bee/runtime/workspaces/demo.json",
            &format!(
                r#"{{"id":"demo--wt--demo","type":"worktree","root":"{}","branch":"wt/demo","base_sha":"abc","write_owner_session":null,"fence_epoch":0,"attached_sessions":["s1","s2"],"created_at":"2026-08-01T00:00:00.000Z"}}"#,
                sibling.replace('\\', "\\\\")
            ),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.workspaces.len(), 1);
        let w = &snap.workspaces[0];
        assert!(!Path::new(&w.root).is_absolute(), "workspace root leaked absolute: {}", w.root);
        assert!(
            !w.root.contains(&root.to_string_lossy().into_owned()),
            "workspace root leaked the fixture root: {}",
            w.root
        );
        assert_eq!(w.attached_sessions, 2);
        assert_eq!(w.branch.as_deref(), Some("wt/demo"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_transcript_path_and_no_absolute_workspace_root_survive_into_snapshot() {
        let root = fresh_root("security-slice2");
        let root_str = root.to_string_lossy().into_owned();
        let transcript = root.join("transcripts/should-not-leak.jsonl").to_string_lossy().into_owned();
        write(
            &root,
            ".bee/sessions/s1.json",
            &format!(
                r#"{{"id":"s1","started_at":"2026-08-01T00:00:00.000Z","last_heartbeat":"2026-08-01T00:00:00.000Z","transcript_path":"{}","workspace_id":"main","source":"startup"}}"#,
                transcript.replace('\\', "\\\\")
            ),
        );
        let outside_abs = std::env::temp_dir()
            .join("mdview-bee-slice2-outside-workspace")
            .to_string_lossy()
            .into_owned();
        write(
            &root,
            ".bee/runtime/workspaces/w1.json",
            &format!(
                r#"{{"id":"w1","type":"worktree","root":"{}","branch":"wt/x","attached_sessions":[],"created_at":"2026-08-01T00:00:00.000Z"}}"#,
                outside_abs.replace('\\', "\\\\")
            ),
        );

        let snap = read_snapshot(&root);
        let serialized = serde_json::to_string(&snap).unwrap();

        assert!(!serialized.contains(&transcript), "the session's own transcript_path leaked into the snapshot");
        assert!(
            !serialized.contains("transcript_path"),
            "the field name itself must not appear - BeeSession never carries it"
        );
        assert!(!serialized.contains(&root_str), "the fixture root leaked into the snapshot");
        assert!(
            !serialized.contains(&outside_abs),
            "the outside-root absolute workspace path leaked into the snapshot"
        );
        for w in &snap.workspaces {
            assert!(!Path::new(&w.root).is_absolute(), "workspace.root must never be absolute: {}", w.root);
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reading_never_writes_the_slice2_files() {
        let root = fresh_root("read-only-slice2");
        write(
            &root,
            ".bee/backlog.jsonl",
            r#"{"kind":"pbi","id":"P1","title":"t","status":"open","feature":"demo"}"#,
        );
        write(
            &root,
            ".bee/decisions.jsonl",
            r#"{"id":"d1","type":"decide","date":"2026-08-01T00:00:00.000Z","decision":"x","scope":"repo"}"#,
        );
        write(
            &root,
            ".bee/sessions/s1.json",
            r#"{"id":"s1","started_at":"2026-08-01T00:00:00.000Z","last_heartbeat":"2026-08-01T00:00:00.000Z","transcript_path":"/x","workspace_id":"main","source":"startup"}"#,
        );
        write(&root, ".bee/lanes/demo.json", r#"{"feature":"demo","phase":"swarming"}"#);
        write(
            &root,
            ".bee/runtime/workspaces/w1.json",
            r#"{"id":"w1","type":"worktree","root":"/x","attached_sessions":[]}"#,
        );

        let before = snapshot_tree(&root);
        let _ = read_snapshot(&root);
        let after = snapshot_tree(&root);

        assert_eq!(before, after, ".bee/ tree changed after reading the Slice 2 files");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn slice2_data_does_not_perturb_buckets_shipped_or_velocity() {
        // Regression: cells 1/3 behavior (buckets, shipped, velocity) must
        // be unaffected by backlog/session/lane/workspace data coexisting
        // in the same store.
        let root = fresh_root("regression-slice2-mix");
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));
        write(
            &root,
            ".bee/cells/f-1.json",
            &feature_cell_json(
                "f-1",
                "feat-a",
                "capped",
                Some("2026-08-01T00:00:00.000Z"),
                Some("2026-08-01T01:00:00.000Z"),
            ),
        );
        write(
            &root,
            ".bee/backlog.jsonl",
            r#"{"kind":"pbi","id":"P1","title":"t","status":"open","feature":"feat-a"}"#,
        );
        write(
            &root,
            ".bee/sessions/s1.json",
            r#"{"id":"s1","started_at":"2026-08-01T00:00:00.000Z","last_heartbeat":"2026-08-01T00:00:00.000Z","transcript_path":"/x","workspace_id":"main","source":"startup"}"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.buckets.waiting.len(), 1);
        assert_eq!(snap.shipped.len(), 1);
        assert_eq!(snap.shipped[0].feature, "feat-a");
        assert_eq!(snap.velocity.per_day.len(), 1);
        // and the new data is present too, proving both coexist.
        assert_eq!(snap.backlog.pbis.len(), 1);
        assert_eq!(snap.sessions.len(), 1);

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bee-board-ux-4: each granted worktree, its own lifecycle record ---

    fn worktree_sibling_root(id: &str) -> PathBuf {
        std::env::temp_dir().join(id)
    }

    /// Create (or refresh) a sibling worktree directory beside `fresh_root`'s
    /// temp parent — the exact shape `resolve_worktree` expects: `<parent of
    /// project root>/<id>/.bee/...`. Cleaned up by the caller like every
    /// other fixture in this module.
    fn make_worktree_sibling(id: &str) -> PathBuf {
        let dir = worktree_sibling_root(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn grants_json(ids: &[&str]) -> String {
        let entries: String = ids.iter().map(|id| format!("\"{id}\": true")).collect::<Vec<_>>().join(",");
        format!("{{{entries}}}")
    }

    fn workspace_json(id: &str, root_abs: &Path, branch: &str, created_at: &str) -> String {
        format!(
            r#"{{"id":"{id}","type":"worktree","root":"{root}","branch":"{branch}","attached_sessions":[],"created_at":"{created_at}"}}"#,
            root = root_abs.to_string_lossy().replace('\\', "\\\\"),
        )
    }

    #[test]
    fn each_granted_worktree_renders_own_feature_phase_branch() {
        let root = fresh_root("worktree-two");
        let alpha = make_worktree_sibling("bee-board-ux-4-wt-alpha");
        let beta = make_worktree_sibling("bee-board-ux-4-wt-beta");
        write(&alpha, ".bee/state.json", r#"{"phase":"swarming","feature":"feat-alpha","mode":"standard"}"#);
        write(&beta, ".bee/state.json", r#"{"phase":"planning","feature":"feat-beta","mode":"small"}"#);

        write(
            &root,
            ".bee/runtime/worktree-grants.json",
            &grants_json(&["bee-board-ux-4-wt-alpha", "bee-board-ux-4-wt-beta"]),
        );
        write(
            &root,
            ".bee/runtime/workspaces/alpha.json",
            &workspace_json("bee-board-ux-4-wt-alpha", &alpha, "wt/alpha", "2026-08-01T00:00:00.000Z"),
        );
        write(
            &root,
            ".bee/runtime/workspaces/beta.json",
            &workspace_json("bee-board-ux-4-wt-beta", &beta, "wt/beta", "2026-08-02T00:00:00.000Z"),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.worktrees.len(), 2, "{:?}", snap.worktrees);
        let a = snap.worktrees.iter().find(|w| w.id == "bee-board-ux-4-wt-alpha").unwrap();
        assert!(a.resolved);
        assert_eq!(a.feature.as_deref(), Some("feat-alpha"));
        assert_eq!(a.phase.as_deref(), Some("swarming"));
        assert_eq!(a.branch.as_deref(), Some("wt/alpha"));
        let b = snap.worktrees.iter().find(|w| w.id == "bee-board-ux-4-wt-beta").unwrap();
        assert!(b.resolved);
        assert_eq!(b.feature.as_deref(), Some("feat-beta"));
        assert_eq!(b.phase.as_deref(), Some("planning"));
        assert_eq!(b.branch.as_deref(), Some("wt/beta"));

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&alpha).ok();
        std::fs::remove_dir_all(&beta).ok();
    }

    #[test]
    fn live_worktree_sorts_ahead_of_quiet_one_with_relative_heartbeat_age() {
        let root = fresh_root("worktree-liveness");
        let live = make_worktree_sibling("bee-board-ux-4-wt-live");
        let quiet = make_worktree_sibling("bee-board-ux-4-wt-quiet");
        write(&live, ".bee/state.json", r#"{"phase":"swarming","feature":"feat-live","mode":"standard"}"#);
        write(&quiet, ".bee/state.json", r#"{"phase":"idle","feature":"feat-quiet","mode":"standard"}"#);
        write(&live, ".bee/sessions/s1.json", &session_json_with_age("s1", 2));

        write(
            &root,
            ".bee/runtime/worktree-grants.json",
            // "quiet" listed first in the source file — the sort must still
            // put the live one ahead regardless of grant order.
            &grants_json(&["bee-board-ux-4-wt-quiet", "bee-board-ux-4-wt-live"]),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.worktrees.len(), 2, "{:?}", snap.worktrees);
        assert_eq!(snap.worktrees[0].id, "bee-board-ux-4-wt-live", "live worktree must sort first: {:?}", snap.worktrees);
        assert!(snap.worktrees[0].live);
        let age = snap.worktrees[0]
            .heartbeat_age_minutes
            .expect("a live worktree must carry a heartbeat age");
        assert!(age < 30.0, "age={age}");
        assert_eq!(snap.worktrees[1].id, "bee-board-ux-4-wt-quiet");
        assert!(!snap.worktrees[1].live);
        assert!(snap.worktrees[1].heartbeat_age_minutes.is_none());

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&live).ok();
        std::fs::remove_dir_all(&quiet).ok();
    }

    /// Regression: a granted worktree's own `.bee/cells/` must never be read
    /// into this project's buckets or shipped set — see the module doc
    /// comment. A "claimed" cell sitting only in the worktree's own store,
    /// naming a feature this project's own store has never heard of, must
    /// leave the Doing bucket and the shipped set exactly as the main
    /// store's own cells computed them.
    #[test]
    fn worktree_cell_files_never_perturb_buckets_or_shipped_set() {
        let root = fresh_root("worktree-no-cell-merge");
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));
        write(
            &root,
            ".bee/cells/f-1.json",
            &feature_cell_json(
                "f-1",
                "feat-a",
                "capped",
                Some("2026-08-01T00:00:00.000Z"),
                Some("2026-08-01T01:00:00.000Z"),
            ),
        );

        let sibling = make_worktree_sibling("bee-board-ux-4-wt-cells");
        write(&sibling, ".bee/state.json", r#"{"phase":"swarming","feature":"ghost-feature","mode":"standard"}"#);
        // A "claimed" cell for a feature this project's own store has never
        // heard of. If this ever got merged into the main snapshot it would
        // show up in `buckets.doing` and possibly `shipped` — neither may
        // happen.
        write(
            &sibling,
            ".bee/cells/ghost.json",
            &feature_cell_json("ghost-1", "ghost-feature", "claimed", Some("2026-08-01T00:00:00.000Z"), None),
        );

        write(&root, ".bee/runtime/worktree-grants.json", &grants_json(&["bee-board-ux-4-wt-cells"]));

        let snap = read_snapshot(&root);
        assert_eq!(snap.buckets.waiting.len(), 1, "{:?}", snap.buckets.waiting);
        assert_eq!(snap.buckets.doing.len(), 0, "a worktree's own claimed cell must never enter this project's Doing bucket: {:?}", snap.buckets.doing);
        assert_eq!(snap.shipped.len(), 1);
        assert_eq!(snap.shipped[0].feature, "feat-a");
        assert!(
            snap.shipped.iter().all(|f| f.feature != "ghost-feature"),
            "a worktree-only feature must never appear in this project's shipped set: {:?}",
            snap.shipped
        );
        // The worktree itself is still visible, just not cell-merged.
        assert_eq!(snap.worktrees.len(), 1);
        assert_eq!(snap.worktrees[0].feature.as_deref(), Some("ghost-feature"));

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&sibling).ok();
    }

    #[test]
    fn worktree_directory_missing_is_reported_unresolved_not_dropped() {
        let root = fresh_root("worktree-dir-missing");
        // No sibling directory is ever created for this id.
        std::fs::remove_dir_all(worktree_sibling_root("bee-board-ux-4-wt-ghost-dir")).ok();
        write(&root, ".bee/runtime/worktree-grants.json", &grants_json(&["bee-board-ux-4-wt-ghost-dir"]));

        let snap = read_snapshot(&root);
        assert_eq!(snap.worktrees.len(), 1, "a dangling grant must still be reported: {:?}", snap.worktrees);
        let w = &snap.worktrees[0];
        assert!(!w.resolved);
        assert!(w.unresolved_reason.is_some(), "an unresolved worktree must name what could not be read");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn worktree_state_json_malformed_is_reported_unresolved_not_fatal() {
        let root = fresh_root("worktree-state-malformed");
        let sibling = make_worktree_sibling("bee-board-ux-4-wt-malformed");
        write(&sibling, ".bee/state.json", "{ not valid json");
        write(&root, ".bee/runtime/worktree-grants.json", &grants_json(&["bee-board-ux-4-wt-malformed"]));

        let snap = read_snapshot(&root);
        assert!(snap.present, "a malformed worktree state.json must not take down the whole read");
        assert_eq!(snap.worktrees.len(), 1);
        let w = &snap.worktrees[0];
        assert!(!w.resolved);
        assert!(w.unresolved_reason.is_some());

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&sibling).ok();
    }

    #[test]
    fn no_grants_file_yields_empty_worktrees_no_read_error() {
        let root = fresh_root("worktree-no-grants");
        write(&root, ".bee/state.json", r#"{"phase":"swarming"}"#);

        let snap = read_snapshot(&root);
        assert!(snap.worktrees.is_empty());
        assert!(
            snap.read_errors.iter().all(|e| !e.contains("worktree-grants")),
            "an absent grants file must not be a read error: {:?}",
            snap.read_errors
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_absolute_worktree_root_survives_into_snapshot() {
        let root = fresh_root("worktree-security");
        let root_str = root.to_string_lossy().into_owned();
        let sibling = make_worktree_sibling("bee-board-ux-4-wt-security");
        let sibling_str = sibling.to_string_lossy().into_owned();
        write(&sibling, ".bee/state.json", r#"{"phase":"swarming","feature":"feat-sec","mode":"standard"}"#);

        write(&root, ".bee/runtime/worktree-grants.json", &grants_json(&["bee-board-ux-4-wt-security"]));
        write(
            &root,
            ".bee/runtime/workspaces/w1.json",
            &workspace_json("bee-board-ux-4-wt-security", &sibling, "wt/security", "2026-08-01T00:00:00.000Z"),
        );

        let snap = read_snapshot(&root);
        let serialized = serde_json::to_string(&snap).unwrap();

        assert!(!serialized.contains(&root_str), "the fixture root leaked into the snapshot");
        assert!(!serialized.contains(&sibling_str), "the worktree's own absolute sibling root leaked into the snapshot");
        // BeeWorktree carries no `root` field at all - id (a safe name) is
        // the only identifier - so this also holds by construction; assert
        // the general shape too, not just the fixture-specific literal.
        for w in &snap.worktrees {
            assert!(!Path::new(&w.id).is_absolute(), "worktree id must never be an absolute path: {}", w.id);
        }

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&sibling).ok();
    }

    #[test]
    fn worktree_read_never_writes_the_project_or_sibling_bee_tree() {
        let root = fresh_root("worktree-read-only");
        let sibling = make_worktree_sibling("bee-board-ux-4-wt-read-only");
        write(&sibling, ".bee/state.json", r#"{"phase":"swarming","feature":"feat-ro","mode":"standard"}"#);
        write(&sibling, ".bee/sessions/s1.json", &session_json_with_age("s1", 2));
        write(&root, ".bee/runtime/worktree-grants.json", &grants_json(&["bee-board-ux-4-wt-read-only"]));

        let before_root = snapshot_tree(&root);
        let before_sibling = snapshot_tree(&sibling);
        let _ = read_snapshot(&root);
        let after_root = snapshot_tree(&root);
        let after_sibling = snapshot_tree(&sibling);

        assert_eq!(before_root, after_root, "reading worktrees must not write the project's own .bee/ tree");
        assert_eq!(before_sibling, after_sibling, "reading a worktree's own .bee/ must never write to it either");

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&sibling).ok();
    }
}
