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
//! bee-board-pm (bbp-4) adds [`BeeSnapshot::attention`] — D6's generated,
//! severity-ordered "needs attention" list, computed in
//! [`compute_attention_items`] purely from `buckets.stuck` and
//! `read_errors`, exactly as this module already computes
//! `running_workers` from data it has already read.
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
    /// Free text, not a path field — scrubbed of any embedded absolute path
    /// before it reaches this struct; see [`scrub_paths`].
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
    /// The cell's own `behavior_change` flag (bbp-13): whether this cell's
    /// work changes observable behavior, as opposed to pure refactor/docs/
    /// process work. `false` when the key is absent — no cell in this
    /// module's fixtures has ever omitted it, but a missing key reading as
    /// "not a behavior change" is the safer default for
    /// [`compute_scribing_debt`], which only ever counts `true`.
    pub behavior_change: bool,
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
    /// `state.json`'s `approved_gates`. `None` when the file never carried
    /// the key — never fabricated as "all false".
    pub approved_gates: Option<BeeApprovedGates>,
    /// `state.json`'s `gate_revoked_at` — bee's append-style historical
    /// anchor for advisor staleness, not a current-state flag: a gate
    /// revoked and then re-approved still carries its old revocation
    /// timestamp here even though `approved_gates.<gate> == Some(true)`
    /// now. A caller must never let this field override a currently-true
    /// `approved_gates` entry; its only job is to tell an *undone* gate's
    /// two histories apart — "revoked" (was approved, then taken away)
    /// from "never reached" (no revocation on record).
    pub gate_revoked_at: Option<BeeGateRevocations>,
    /// `state.json`'s `route`. `None` when the file never carried the key.
    pub route: Option<BeeRoute>,
    /// `state.json`'s `next_action`. Free text, not a path field — scrubbed
    /// of any embedded absolute path before it reaches this struct; see
    /// [`scrub_paths`].
    pub next_action: Option<String>,
    /// `state.json`'s `last_scribing_run`, when present (bbp-13). `None`
    /// means this feature has never been through a scribe pass while it was
    /// the active feature — see [`compute_scribing_debt`].
    pub last_scribing_run: Option<BeeLastScribingRun>,
}

/// `.bee/state.json`'s or a `.bee/lanes/<feature>.json`'s
/// `last_scribing_run` object (bbp-13) — bee's own record of the last
/// scribe (capture) pass, keyed by the feature it captured. Only `feature`
/// is read: `date`, `at` and `areas_synced` have no consumer here (the
/// scribing-debt derivation this exists for only ever needs to ask "did
/// the most recent scribe pass name THIS feature?"), so they are left
/// unread rather than carried for no reason. This repo's own
/// `last_scribing_run.feature` and its `state.feature` can legitimately
/// differ (a scribe pass for one feature, followed by routing a new one) —
/// that mismatch is exactly the signal [`compute_scribing_debt`] looks
/// for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeLastScribingRun {
    pub feature: Option<String>,
}

/// `.bee/state.json`'s `approved_gates` — the four gates a feature's work
/// passes through (context, shape, execution, review), each approved
/// independently. A gate name entirely absent from the file reads as
/// `None`, never fabricated as `false`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeApprovedGates {
    pub context: Option<bool>,
    pub shape: Option<bool>,
    pub execution: Option<bool>,
    pub review: Option<bool>,
}

/// `.bee/state.json`'s `gate_revoked_at` — the timestamp a gate was revoked
/// after having been approved, keyed by the same four gate names as
/// [`BeeApprovedGates`]. A gate name absent here was never revoked (or was
/// never approved in the first place); this struct alone does not say
/// which — see [`BeeState::approved_gates`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeGateRevocations {
    pub context: Option<String>,
    pub shape: Option<String>,
    pub execution: Option<String>,
    pub review: Option<String>,
}

/// `.bee/state.json`'s `route` — the lane classification recorded when the
/// feature was routed. Only the six keys this snapshot ever reads; a
/// version-specific extra (`route.feature` on bee 2.2.2, `route.demoted_at`
/// on this repo's 2.1.15) is ignored rather than refused — neither bee
/// version's `route` object is a superset of the other's.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeRoute {
    pub class: Option<String>,
    pub lane: Option<String>,
    pub flags: Vec<String>,
    pub product_files: Option<u64>,
    /// Free text, not a path field — scrubbed of any embedded absolute path
    /// before it reaches this struct; see [`scrub_paths`].
    pub rationale: Option<String>,
    pub updated_at: Option<String>,
}

/// `.bee/HANDOFF.json` — the note a session writes when it stops and hands
/// the work back to a human, `{written_at, next_action, kind}`. Presence
/// alone means work is parked; the store carries no consumed-marker, so a
/// note whose work has since finished still sits here — the board dates it
/// rather than judging whether it is stale (bbp-8, D6). A `kind` of
/// `"pause"`, or the key absent entirely, reads as a pause (the same
/// "a kindless record reads as pause" convention bee's own workflow docs
/// state for this exact file); `"planned-next"` is a different thing
/// entirely — a clean stop with the next claim already owned — and must
/// never be reported as a pause (see [`compute_attention_items`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeHandoff {
    pub written_at: Option<String>,
    /// Free text, not a path field — scrubbed of any embedded absolute path
    /// before it reaches this struct; see [`scrub_paths`]. This repo's own
    /// handoff is a five-sentence paragraph naming filesystem paths.
    pub next_action: Option<String>,
    pub kind: Option<String>,
}

/// `.bee/config.json` — read for one field only, `gate_bypass` (bbp-9): bee
/// can be configured to auto-approve its own approval gates, and a project
/// running that way is something a manager should see.
///
/// `gate_bypass`'s value type is not stable across stores (this repo
/// records the boolean `false`; the beehive store records the string
/// `"total"`), so it is normalized defensively rather than typed as one
/// JSON shape: a boolean `false` and a missing key both collapse to `None`
/// ("off"); a string value bee itself writes (e.g. `"total"`) is carried
/// through verbatim; any other JSON shape is carried through as its own
/// text rather than guessed at or coerced to off — see
/// [`normalize_gate_bypass`].
///
/// **This is the recorded setting, never the effective one.**
/// `.bee/config.local.json` exists on disk as a machine-local overlay that
/// bee itself resolves on top of this file — this reader does not open
/// `config.local.json` and does not attempt to reproduce bee's resolution
/// order, because doing so here would be silently wrong on any machine
/// that overlays it, inside the one panel whose whole job is to be
/// trustworthy. Every consumer of this field (see
/// [`compute_attention_items`]) must word its output as the recorded
/// setting, not a claim about what is actually enforced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeConfig {
    pub gate_bypass: Option<String>,
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
    /// Free text, not a path field — scrubbed of any embedded absolute path
    /// before it reaches this struct; see [`scrub_paths`].
    pub title: String,
    /// `proposed`, `in-flight`, `parked`, `done`, `declined`, or anything
    /// else a future bee version introduces — folded from the LAST event
    /// carrying this `id`, never the first.
    pub status: String,
    pub feature: String,
    /// Condition-of-satisfaction detail text. Free text, not a path field —
    /// scrubbed of any embedded absolute path before it reaches this
    /// struct, exactly like `title`; see [`scrub_paths`]. A missing `cos`
    /// field folds to an empty string, same as a missing `title`.
    pub cos: String,
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
    /// Free text, not a path field — scrubbed of any embedded absolute path
    /// before it reaches this struct; see [`scrub_paths`].
    pub title: String,
    /// Free text, not a path field — scrubbed of any embedded absolute path
    /// before it reaches this struct; see [`scrub_paths`].
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
/// subset of `.bee/state.json` the cockpit already shows. A lane record is a
/// full parallel copy of its feature's state, not a stub — it carries its
/// own `approved_gates` and `created_at` exactly as `.bee/state.json` does
/// for the globally active feature, so the by-phase board (bbp-10) can place
/// a feature's gates and age from its own record rather than borrowing the
/// active feature's.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeLane {
    pub feature: String,
    pub phase: Option<String>,
    pub mode: Option<String>,
    /// Free text, not a path field — scrubbed of any embedded absolute path
    /// before it reaches this struct; see [`scrub_paths`].
    pub next_action: Option<String>,
    /// This lane's own `approved_gates`. `None` when the file never carried
    /// the key — never fabricated as "all false". See
    /// [`BeeState::approved_gates`] for the same shape on the active
    /// feature.
    pub approved_gates: Option<BeeApprovedGates>,
    /// This lane's own `created_at`, verbatim. `.bee/state.json` carries no
    /// equivalent field for the active feature, so a placement built from
    /// `state.json` alone (bbp-10) always reports `None` here.
    pub created_at: Option<String>,
    /// This lane's own `last_scribing_run`, when present (bbp-13) — the
    /// same shape [`BeeState::last_scribing_run`] carries for the active
    /// feature; see [`compute_scribing_debt`].
    pub last_scribing_run: Option<BeeLastScribingRun>,
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
    /// Free text, not a path field — scrubbed of any embedded absolute path
    /// before it reaches this struct; see [`scrub_paths`].
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

/// Severity of one generated "needs attention" item (D6). Variants are
/// declared lightest-first so the derived [`Ord`] ranks `Critical` highest;
/// [`compute_attention_items`] sorts descending by this so the heaviest
/// items lead. Ranked to match the source spec's three tiers
/// (`docs/history/bee-board-pm/pm-dashboard-spec.md` §5: 🟡 warning / 🟠
/// serious / 🔴 critical) so a rule added later slots into the tier it
/// already names, without moving the tiers this slice uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeeAttentionSeverity {
    Warning,
    Serious,
    Critical,
}

/// One generated "needs attention" finding (D6). Not a bee concept — it
/// exists only on the board, produced by [`compute_attention_items`] over
/// data this snapshot already read. Every field is a plain, non-optional
/// string or [`BeeAttentionSeverity`], so an item can never be constructed
/// missing one of the four the board promises.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeeAttentionItem {
    pub severity: BeeAttentionSeverity,
    /// Short label naming what fired. Built only from fields already
    /// scrubbed/relativized before they reached this snapshot (`BeeCell`,
    /// `read_errors`) — see [`scrub_paths`] and [`parse_cell`].
    pub title: String,
    /// The specifics behind `title` — which cells, which files.
    pub detail: String,
    /// What a human should do about it.
    pub suggested_action: String,
}

/// Per-feature cell counts by D7 status, for one [`BeeFeaturePhase`].
/// `dropped` cells and any unrecognized status count toward none of these
/// fields and never toward `total` (D8) — a feature whose cells are all
/// dropped reports zero everywhere here, honestly, rather than reading as
/// complete or dividing by an empty denominator.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BeeFeatureCellCounts {
    /// `status == "claimed"`.
    pub doing: usize,
    /// `status == "open"`.
    pub waiting: usize,
    /// `status == "blocked"`.
    pub stuck: usize,
    /// `status == "capped"`.
    pub done: usize,
    /// `doing + waiting + stuck + done` — the only denominator this
    /// feature's counts are ever divided by. Never includes `dropped` or an
    /// unrecognized status (D8).
    pub total: usize,
    /// `done / total`, when `total > 0`. `None` — never a guessed `0.0` —
    /// when there is nothing to measure, whether because the feature has no
    /// cells at all or because every one of its cells is `dropped` (D8).
    pub done_fraction: Option<f64>,
}

/// One feature placed on its phase (bbp-10, D5's by-phase board), replacing
/// "what cell state" with "what feature, how far along". Produced by
/// [`compute_phase_board`] over data this snapshot already read — see that
/// function for the union rule that decides which features appear here and
/// where each one's `phase`/`approved_gates`/`next_action`/`created_at` come
/// from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeFeaturePhase {
    pub feature: String,
    pub phase: Option<String>,
    pub mode: Option<String>,
    pub approved_gates: Option<BeeApprovedGates>,
    /// Free text, not a path field — already scrubbed of any embedded
    /// absolute path by the source it was read from ([`BeeLane::next_action`]
    /// or [`BeeState::next_action`], both already scrubbed at their own read
    /// site).
    pub next_action: Option<String>,
    /// `None` when this placement's phase/gates were sourced from
    /// `.bee/state.json` rather than a lane record — `state.json` carries no
    /// equivalent field for the globally active feature.
    pub created_at: Option<String>,
    pub cell_counts: BeeFeatureCellCounts,
}

/// Review status for one `.bee/review-candidates.jsonl` row (bbp-13),
/// derived entirely by joining it against `.bee/reviews/*.json` sessions —
/// the candidates file itself carries no status field of any kind. See
/// [`compute_review`] for the exact join rule. Independent review is
/// user-invoked (D7); nothing this status produces is ever worded as
/// automatic pending work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeeReviewStatus {
    /// None of this candidate's cells appear in any session's
    /// `included[]`. A candidate naming zero cells — the shape live in
    /// this repo's own store, one candidate with a null baseline and no
    /// cells — can never match anything and is always `Unreviewed`, pinned
    /// here rather than left to fall out of the join by accident.
    Unreviewed,
    /// At least one of this candidate's cells appears in a session that is
    /// not settled: its `decision.status` is `pending`, or the session
    /// carries no `decision` key at all (an in-progress review with no
    /// verdict yet is still in review, never silently unreviewed). Checked
    /// before `Settled` so a candidate re-opened for a fresh review after
    /// an earlier session settled it still reads as needing attention.
    InReview,
    /// Every session naming one of this candidate's cells has a
    /// `decision.status` of `approved` or `blocked`.
    Settled,
}

/// One `.bee/review-candidates.jsonl` row (bbp-13), joined against every
/// `.bee/reviews/*.json` session this snapshot read — see
/// [`BeeReviewStatus`] and [`compute_review`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeReviewCandidate {
    pub id: String,
    pub feature: String,
    /// `high-risk`, `standard`, or whatever mode string the candidate row
    /// itself carries; `None` when the row has no `mode` key.
    pub mode: Option<String>,
    pub status: BeeReviewStatus,
}

/// `.bee/review-candidates.jsonl` joined against `.bee/reviews/*.json`
/// (bbp-13, D6, D7) — see [`compute_review`]. Independent review is
/// user-invoked; the board never presents anything here as automatic
/// pending work.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeReview {
    pub candidates: Vec<BeeReviewCandidate>,
    /// Count of `P1` findings across every session whose own
    /// `decision.status` is NOT `approved` or `blocked` (i.e. `pending` or
    /// no decision at all) — an open P1 is stronger signal than a count of
    /// unreviewed candidates. A finding with no `severity` key, or a
    /// `severity` of `info`, is never counted here; only an exact `"P1"`
    /// match is.
    pub open_p1_findings: usize,
}

/// `.bee/capture-queue.jsonl`, summarized (bbp-13) — see
/// [`read_capture_queue`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeCaptureQueue {
    /// Distinct `stub` ids with no matching `flush` id — net of flushes,
    /// never a raw stub count.
    pub waiting: usize,
}

/// One entry from `.bee/reservations.json`'s `reservations` array (bbp-15)
/// — a file or glob currently locked by one agent while parallel work
/// runs. Neither live store this reader was verified against holds a
/// non-empty array (both are `{"reservations": []}`), so every field is
/// read defensively and carried exactly as the store spells it — no
/// renaming, no reshaping, and a field the store omits is simply `None`
/// here rather than guessed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeReservation {
    pub agent: Option<String>,
    pub cell: Option<String>,
    pub path: Option<String>,
    pub kind: Option<String>,
    pub session: Option<String>,
    pub reserved_at: Option<String>,
    pub released_at: Option<String>,
}

/// D5's process-health "tier mix" (bbp-15) — how the project's cells are
/// spread across bee's model-tier dispatch rubric (`extraction` <
/// `generation` < `ceiling`, cheapest to most expensive; see the
/// bee-swarming skill's tier rubric). See [`compute_tier_mix`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeTierMix {
    /// Count per tier value, exactly as cells spell it — not limited to
    /// the three named tiers above, so a value this reader does not
    /// recognize is still counted rather than silently dropped.
    pub counts: std::collections::BTreeMap<String, usize>,
    /// Cells with no `tier` key at all (or an explicit `null`) — counted
    /// here, never dropped and never guessed into one of the tier buckets.
    pub untiered: usize,
    /// Share (0.0-1.0) of *tiered* cells (`untiered` excluded) whose tier
    /// is `"ceiling"`, the most expensive tier. `None` when there are zero
    /// tiered cells to take a share of — a zero-tiered store reports no
    /// share, never a zero and never a division by zero.
    pub expensive_tier_share: Option<f64>,
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
    /// D5's by-phase board (bbp-10) — every feature the store knows on its
    /// phase, with its own gates and its own cell counts. See
    /// [`compute_phase_board`] for the union rule over `lanes` and the
    /// globally active feature.
    pub phase_board: Vec<BeeFeaturePhase>,
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
    /// D6's generated "needs attention" list — see
    /// [`compute_attention_items`]. Heaviest severity first, empty when
    /// nothing this slice's rules cover is wrong.
    pub attention: Vec<BeeAttentionItem>,
    /// `.bee/HANDOFF.json`, when present — see [`BeeHandoff`]. `None` is the
    /// normal, expected shape for most stores; it is never a read error.
    pub handoff: Option<BeeHandoff>,
    /// `.bee/config.json`, when present — see [`BeeConfig`]. `None` here
    /// means only that the file itself is absent or unparseable; it does
    /// NOT mean `gate_bypass` is on — see `BeeConfig::gate_bypass` and
    /// [`normalize_gate_bypass`] for how "off" is actually decided.
    pub config: Option<BeeConfig>,
    /// Presence-only read of `docs/history/<feature>/promote-proposals.md`
    /// (bbp-12), never its contents, keyed by every distinct feature name
    /// this snapshot read (the active feature in `state.json`, every lane,
    /// every cell). A feature name that fails [`validate_feature_name`] —
    /// the only gate a name passes through before touching the filesystem,
    /// see [`promote_proposals_path`] — is never looked up and is simply
    /// absent as a key here, never a false `false`.
    pub promote_proposals: std::collections::BTreeMap<String, bool>,
    /// `.bee/review-candidates.jsonl` joined against `.bee/reviews/*.json`
    /// (bbp-13) — see [`BeeReview`] and [`compute_review`].
    pub review: BeeReview,
    /// `.bee/capture-queue.jsonl`, summarized (bbp-13) — see
    /// [`BeeCaptureQueue`].
    pub capture_queue: BeeCaptureQueue,
    /// Feature names with scribing debt (bbp-13, Terms: "Knowledge debt") —
    /// a feature with at least one `capped`, `behavior_change` cell whose
    /// own `last_scribing_run` (state.json for the active feature, its own
    /// lane record for a lane feature) does not name it. See
    /// [`compute_scribing_debt`]. Only ever populated for a feature this
    /// snapshot can place at all — the same `lanes ∪ {active feature}`
    /// union [`compute_phase_board`] already establishes.
    pub scribing_debt: Vec<String>,
    /// `.bee/reservations.json`'s `reservations` array (bbp-15) — which
    /// files are locked by which agent right now, a process-health signal:
    /// work colliding on the same files is why a project slows down. Empty
    /// is both the normal shape (neither live store this reader was
    /// verified against holds one) and the fallback for a missing file or
    /// an unexpected shape — see [`read_reservations`].
    pub reservations: Vec<BeeReservation>,
    /// D5's process-health "tier mix" (bbp-15) — see [`BeeTierMix`] and
    /// [`compute_tier_mix`]. `None` only when this snapshot has no cells
    /// at all to measure.
    pub tier_mix: Option<BeeTierMix>,
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
            phase_board: Vec::new(),
            workspaces: Vec::new(),
            decisions: BeeDecisions::default(),
            worktrees: Vec::new(),
            running_workers: Vec::new(),
            attention: Vec::new(),
            handoff: None,
            config: None,
            promote_proposals: std::collections::BTreeMap::new(),
            review: BeeReview::default(),
            capture_queue: BeeCaptureQueue::default(),
            scribing_debt: Vec::new(),
            reservations: Vec::new(),
            tier_mix: None,
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
    let phase_board = compute_phase_board(&lanes, state.as_ref(), &all_cells);
    let workspaces = read_workspaces(&bee_dir, root, &mut read_errors);
    let decisions = read_decisions(&bee_dir, root, &mut read_errors);
    let worktrees = read_worktrees(root, &workspaces, now, &mut read_errors);

    let running_workers = state
        .as_ref()
        .map(|s| compute_running_workers(&s.workers, &all_cells, &sessions))
        .unwrap_or_default();

    let handoff = read_handoff(&bee_dir, root, &mut read_errors);
    let config = read_config(&bee_dir, root, &mut read_errors);
    let gate_bypass = config.as_ref().and_then(|c| c.gate_bypass.as_deref());

    // bbp-12: every distinct feature name this read has seen, from every
    // place a feature name is read — state.json's active feature, each
    // lane, each cell — deduplicated via the set before any of them is
    // ever joined onto a filesystem path (see `validate_feature_name`).
    let mut feature_names: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    if let Some(f) = state.as_ref().and_then(|s| s.feature.as_deref()) {
        feature_names.insert(f);
    }
    for lane in &lanes {
        feature_names.insert(lane.feature.as_str());
    }
    for cell in &all_cells {
        feature_names.insert(cell.feature.as_str());
    }
    let promote_proposals = read_promote_proposals(root, feature_names.into_iter());

    // bbp-13: review join, capture queue, scribing debt — see the
    // module doc comment.
    let review_candidates = read_review_candidates(&bee_dir, root, &mut read_errors);
    let review_sessions = read_review_sessions(&bee_dir, root, &mut read_errors);
    let review = compute_review(&review_candidates, &review_sessions);
    let capture_queue = read_capture_queue(&bee_dir, root, &mut read_errors);
    let scribing_debt = compute_scribing_debt(&phase_board, &lanes, state.as_ref(), &all_cells);

    // bbp-15: the two process-health signals a manager reads to see whether
    // parallel work is colliding (reservations) and where token spend is
    // going (tier mix). Neither reads anything the rest of this function
    // does not already open, except reservations.json itself.
    let reservations = read_reservations(&bee_dir, root, &mut read_errors);
    let tier_mix = compute_tier_mix(&all_cells);

    let attention = compute_attention_items(
        &buckets.stuck,
        &read_errors,
        handoff.as_ref(),
        gate_bypass,
        &review,
        &scribing_debt,
        &capture_queue,
        &promote_proposals,
    );

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
        phase_board,
        workspaces,
        decisions,
        worktrees,
        running_workers,
        attention,
        handoff,
        config,
        promote_proposals,
        review,
        capture_queue,
        scribing_debt,
        reservations,
        tier_mix,
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

/// Place every feature the store knows on its phase (bbp-10, D5's by-phase
/// board), in the shape [`compute_running_workers`] establishes: pure, no
/// I/O, no clock of its own, over data this snapshot already read.
///
/// **The feature set is the UNION of `lanes` and the globally active
/// feature named in `state.feature`** — never the lane list alone. bee's own
/// active feature commonly has no lane file of its own (a lane record is
/// only written once a feature is routed onto a lane other than the default
/// pipeline), so a board built from `.bee/lanes/` alone would silently omit
/// the one feature actually being worked on. Equally, a store with no
/// `.bee/lanes/` directory at all — every project until it grows a second
/// concurrent feature — must still place its one active feature correctly:
/// `lanes` is simply empty then, and the union degrades to the singleton
/// `{state.feature}`.
///
/// A feature present in both `lanes` and as the active feature is placed
/// **exactly once**: its own lane record wins for `phase`/`approved_gates`/
/// `next_action`/`created_at`, because the lane record is the fuller,
/// feature-specific source — `state.json`'s view of that same feature is
/// used only as a fallback, for the active feature when it has no lane
/// record of its own. `created_at` has no equivalent field in `state.json`,
/// so a placement sourced from `state.json` alone always reports `None`
/// there.
///
/// `cell_counts` is a pure function of `all_cells` filtered to the
/// placement's own feature name, bucketed exactly as D7 buckets the live
/// cell set: `dropped` and any unrecognized status count toward no bucket,
/// no `total`, and no `done_fraction` denominator (D8) — a feature whose
/// cells are all dropped reports zero counts and a `None` fraction, never a
/// completed-looking `1.0` and never a division by zero. A feature with no
/// cells at all reports the same honest zero.
fn compute_phase_board(
    lanes: &[BeeLane],
    state: Option<&BeeState>,
    all_cells: &[BeeCell],
) -> Vec<BeeFeaturePhase> {
    let mut order: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for lane in lanes {
        if seen.insert(lane.feature.as_str()) {
            order.push(lane.feature.clone());
        }
    }
    if let Some(active) = state.and_then(|s| s.feature.as_deref()) {
        if seen.insert(active) {
            order.push(active.to_string());
        }
    }

    order
        .into_iter()
        .map(|feature| {
            let lane = lanes.iter().find(|l| l.feature == feature);
            let (phase, mode, approved_gates, next_action, created_at) = match lane {
                Some(l) => (
                    l.phase.clone(),
                    l.mode.clone(),
                    l.approved_gates.clone(),
                    l.next_action.clone(),
                    l.created_at.clone(),
                ),
                None => {
                    // Only reached for the active feature when it carries no
                    // lane record of its own.
                    let s = state.filter(|s| s.feature.as_deref() == Some(feature.as_str()));
                    (
                        s.and_then(|s| s.phase.clone()),
                        s.and_then(|s| s.mode.clone()),
                        s.and_then(|s| s.approved_gates.clone()),
                        s.and_then(|s| s.next_action.clone()),
                        None,
                    )
                }
            };
            let cell_counts = compute_feature_cell_counts(&feature, all_cells);
            BeeFeaturePhase { feature, phase, mode, approved_gates, next_action, created_at, cell_counts }
        })
        .collect()
}

/// D7-style cell counts for one feature (bbp-10): every non-dropped,
/// recognized-status cell whose `feature` matches, bucketed exactly as
/// [`read_snapshot`]'s own D7 buckets are. `dropped` and any unrecognized
/// status contribute to nothing here, including `total` (D8).
fn compute_feature_cell_counts(feature: &str, all_cells: &[BeeCell]) -> BeeFeatureCellCounts {
    let mut counts = BeeFeatureCellCounts::default();
    for cell in all_cells.iter().filter(|c| c.feature == feature) {
        match cell.status.as_str() {
            "claimed" => counts.doing += 1,
            "open" => counts.waiting += 1,
            "blocked" => counts.stuck += 1,
            "capped" => counts.done += 1,
            // "dropped" and any unrecognized status: no bucket, no total (D8).
            _ => {}
        }
    }
    counts.total = counts.doing + counts.waiting + counts.stuck + counts.done;
    counts.done_fraction = if counts.total > 0 { Some(counts.done as f64 / counts.total as f64) } else { None };
    counts
}

/// Generate D6's "needs attention" list over data this snapshot already
/// read (D4 — no additional I/O, no clock of its own). Each rule below
/// fires independently of the others and, when it fires, contributes
/// exactly one item; the result is sorted heaviest severity first, and a
/// stable sort keeps items of equal severity in the fixed order the rules
/// are written in below, so the list never reshuffles between requests on
/// unchanged data.
///
/// This slice's rules are the only ones existing snapshot data already
/// supports:
/// - `blocked_cells` non-empty (source spec A6, 🔴 critical: "every red
///   cell is a fix-first cell").
/// - `read_errors` non-empty (🔴 critical too: a file this board could not
///   parse might just as easily be hiding a blocked cell as showing one,
///   so every other number on the page should be read as provisional
///   until it is fixed).
///
/// bbp-8 adds a third, independent rule: `handoff` present and its `kind`
/// reads as a pause (source spec A1, 🔴 critical: "{feature} is paused" —
/// this board has no per-note feature name to name, so the title stays
/// generic). It fires on the note's own text and its own `written_at`,
/// never on a judgement of whether the note is still relevant (D6) — a
/// `"planned-next"` handoff is a different thing (a clean stop with the
/// next claim already owned) and never fires this rule.
///
/// bbp-9 adds a fourth, independent rule: a recorded `gate_bypass` that is
/// not off (source spec §5c: `gate_bypass_level` off is "stopping by the
/// rules"; anything else is "cảnh báo" — a warning). It fires on the value
/// [`read_config`]/[`normalize_gate_bypass`] already normalized, and its
/// wording names that value as the **recorded** setting only — never a
/// claim about what is actually being enforced, because
/// `.bee/config.local.json` can override it on a given machine and this
/// reader never opens that file (see [`BeeConfig`]).
///
/// bbp-13 adds three more, independent rules, each worded as user-invoked
/// work (D7) so nothing here reads as review already running on its own:
/// - `review.open_p1_findings` non-empty (🔴 critical — an open P1 in a
///   review that has not yet been settled is stronger signal than a count
///   of candidates nobody has looked at).
/// - An unreviewed candidate whose own `mode` is `high-risk` (🟠 serious —
///   `review.candidates` filtered to [`BeeReviewStatus::Unreviewed`] with
///   `mode == Some("high-risk")`).
/// - Knowledge debt (🟡 warning — Terms: "Capped work whose learnings were
///   never recorded", folded into the one number a human can act on):
///   `scribing_debt.len()` plus `capture_queue.waiting` plus the count of
///   features carrying an unapplied `promote_proposals` entry (bbp-12).
///
/// Adding a rule later means adding another `if` below and letting it fall
/// into the sort — the ones here are never touched to make room for it.
fn compute_attention_items(
    blocked_cells: &[BeeCell],
    read_errors: &[String],
    handoff: Option<&BeeHandoff>,
    gate_bypass: Option<&str>,
    review: &BeeReview,
    scribing_debt: &[String],
    capture_queue: &BeeCaptureQueue,
    promote_proposals: &std::collections::BTreeMap<String, bool>,
) -> Vec<BeeAttentionItem> {
    let mut items = Vec::new();

    if !blocked_cells.is_empty() {
        let n = blocked_cells.len();
        let noun = if n == 1 { "cell" } else { "cells" };
        let detail = blocked_cells
            .iter()
            .map(|c| format!("{} ({})", c.title, c.id))
            .collect::<Vec<_>>()
            .join("; ");
        items.push(BeeAttentionItem {
            severity: BeeAttentionSeverity::Critical,
            title: format!("{n} {noun} blocked"),
            detail,
            suggested_action: "Every blocked cell is a fix-first cell — clear it before starting new work.".to_string(),
        });
    }

    if !read_errors.is_empty() {
        let n = read_errors.len();
        let noun = if n == 1 { "file" } else { "files" };
        items.push(BeeAttentionItem {
            severity: BeeAttentionSeverity::Critical,
            title: format!("{n} {noun} could not be read"),
            detail: read_errors.join("; "),
            suggested_action: "Repair or regenerate the file(s) named above — until they parse, every other number on this page may be incomplete.".to_string(),
        });
    }

    if let Some(h) = handoff {
        // A kindless record and an explicit "pause" both read as a pause;
        // "planned-next" (a clean stop, next claim already owned) is not.
        let is_pause = !matches!(h.kind.as_deref(), Some("planned-next"));
        if is_pause {
            let when = h.written_at.as_deref().unwrap_or("an unknown time");
            let note = h.next_action.as_deref().unwrap_or("(no note text was recorded)");
            items.push(BeeAttentionItem {
                severity: BeeAttentionSeverity::Critical,
                title: "Work is parked, waiting on a person".to_string(),
                detail: format!("Written {when}: {note}"),
                suggested_action: "Read the handoff note and decide whether to resume the work or set it aside — the store never marks a note as consumed, so date it yourself.".to_string(),
            });
        }
    }

    if let Some(level) = gate_bypass {
        items.push(BeeAttentionItem {
            severity: BeeAttentionSeverity::Warning,
            title: format!("Gate bypass recorded as \"{level}\""),
            detail: format!(
                "`.bee/config.json` records `gate_bypass: \"{level}\"` — this project's own approval gates are recorded as being auto-approved. This is the recorded setting, not the effective one: a machine-local `.bee/config.local.json` overlay is never read by this board."
            ),
            suggested_action: "Confirm this bypass is still intended, and check the machine's own config.local.json for what is actually being enforced there.".to_string(),
        });
    }

    if review.open_p1_findings > 0 {
        let n = review.open_p1_findings;
        let noun = if n == 1 { "finding" } else { "findings" };
        items.push(BeeAttentionItem {
            severity: BeeAttentionSeverity::Critical,
            title: format!("{n} open P1 review {noun}"),
            detail: format!(
                "{n} P1 {noun} in a review session that has not yet been settled (approved or blocked)."
            ),
            suggested_action: "Independent review is user-invoked — when you next run it, resolve these P1s before anything else in scope.".to_string(),
        });
    }

    let unreviewed_high_risk = review
        .candidates
        .iter()
        .filter(|c| c.status == BeeReviewStatus::Unreviewed && c.mode.as_deref() == Some("high-risk"))
        .count();
    if unreviewed_high_risk > 0 {
        let n = unreviewed_high_risk;
        let noun = if n == 1 { "candidate" } else { "candidates" };
        items.push(BeeAttentionItem {
            severity: BeeAttentionSeverity::Serious,
            title: format!("{n} high-risk review {noun} never reviewed"),
            detail: format!(
                "{n} high-risk review {noun} named in `.bee/review-candidates.jsonl` do not appear in any `.bee/reviews/*.json` session yet."
            ),
            suggested_action: "Independent review is user-invoked, never automatic — run it for these candidates when you are ready.".to_string(),
        });
    }

    let promote_unapplied = promote_proposals.values().filter(|present| **present).count();
    let knowledge_debt = scribing_debt.len() + capture_queue.waiting + promote_unapplied;
    if knowledge_debt > 0 {
        let noun = if knowledge_debt == 1 { "item" } else { "items" };
        items.push(BeeAttentionItem {
            severity: BeeAttentionSeverity::Warning,
            title: format!("{knowledge_debt} knowledge-debt {noun}"),
            detail: format!(
                "{} feature(s) with capped work never scribed, {} capture-queue stub(s) still waiting, {} feature(s) with an unapplied promote-proposals.md.",
                scribing_debt.len(),
                capture_queue.waiting,
                promote_unapplied
            ),
            suggested_action: "Run a scribing pass to fold this work into docs/knowledge before it is lost.".to_string(),
        });
    }

    items.sort_by(|a, b| b.severity.cmp(&a.severity));
    items
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
            approved_gates: parse_approved_gates(&v),
            gate_revoked_at: v.get("gate_revoked_at").and_then(Value::as_object).map(|g| BeeGateRevocations {
                context: g.get("context").and_then(Value::as_str).map(String::from),
                shape: g.get("shape").and_then(Value::as_str).map(String::from),
                execution: g.get("execution").and_then(Value::as_str).map(String::from),
                review: g.get("review").and_then(Value::as_str).map(String::from),
            }),
            route: v.get("route").and_then(Value::as_object).map(|r| BeeRoute {
                class: r.get("class").and_then(Value::as_str).map(String::from),
                lane: r.get("lane").and_then(Value::as_str).map(String::from),
                flags: r
                    .get("flags")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(|f| f.as_str().map(String::from)).collect())
                    .unwrap_or_default(),
                product_files: r.get("product_files").and_then(Value::as_u64),
                rationale: r.get("rationale").and_then(Value::as_str).map(|s| scrub_paths(s, root)),
                updated_at: r.get("updated_at").and_then(Value::as_str).map(String::from),
            }),
            next_action: v.get("next_action").and_then(Value::as_str).map(|s| scrub_paths(s, root)),
            last_scribing_run: parse_last_scribing_run(&v),
        }),
        Err(e) => {
            read_errors.push(format!("{}: could not parse ({e})", rel_str(&path, root)));
            None
        }
    }
}

/// Parse an `approved_gates` object shared by `.bee/state.json` and every
/// `.bee/lanes/<feature>.json` record (bbp-10) — the same four gate names,
/// each independently optional, never fabricated as `false` when the key is
/// absent. Factored out so `read_state` and `parse_lane` never drift apart
/// on this shape.
fn parse_approved_gates(v: &Value) -> Option<BeeApprovedGates> {
    v.get("approved_gates").and_then(Value::as_object).map(|g| BeeApprovedGates {
        context: g.get("context").and_then(Value::as_bool),
        shape: g.get("shape").and_then(Value::as_bool),
        execution: g.get("execution").and_then(Value::as_bool),
        review: g.get("review").and_then(Value::as_bool),
    })
}

/// Parse a `last_scribing_run` object shared by `.bee/state.json` and every
/// `.bee/lanes/<feature>.json` record (bbp-13) — only `feature` is read,
/// see [`BeeLastScribingRun`]. Factored out so `read_state` and
/// `parse_lane` never drift apart on this shape.
fn parse_last_scribing_run(v: &Value) -> Option<BeeLastScribingRun> {
    v.get("last_scribing_run")
        .and_then(Value::as_object)
        .map(|l| BeeLastScribingRun { feature: l.get("feature").and_then(Value::as_str).map(String::from) })
}

/// Read `.bee/HANDOFF.json` (bbp-8), following [`read_state`]'s convention
/// exactly: a missing file is silent and normal (most stores have none),
/// while a read or parse error pushes one line onto `read_errors` and the
/// rest of the snapshot still reads.
fn read_handoff(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> Option<BeeHandoff> {
    let path = bee_dir.join("HANDOFF.json");
    if !path.is_file() {
        // No HANDOFF.json is the normal, expected shape — silent, not a
        // read error.
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
        Ok(v) => Some(BeeHandoff {
            written_at: v.get("written_at").and_then(Value::as_str).map(String::from),
            next_action: v.get("next_action").and_then(Value::as_str).map(|s| scrub_paths(s, root)),
            kind: v.get("kind").and_then(Value::as_str).map(String::from),
        }),
        Err(e) => {
            read_errors.push(format!("{}: could not parse ({e})", rel_str(&path, root)));
            None
        }
    }
}

/// Read `.bee/config.json` (bbp-9), following [`read_state`]'s convention
/// exactly: a missing file is silent and normal, while a read or parse
/// error pushes one line onto `read_errors` and the rest of the snapshot
/// still reads.
///
/// This reader opens `config.json` only — never `config.local.json`, the
/// machine-local overlay bee itself resolves on top of it. See
/// [`BeeConfig`] for why: reproducing that resolution here would be
/// silently wrong on any machine that overlays it.
fn read_config(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> Option<BeeConfig> {
    let path = bee_dir.join("config.json");
    if !path.is_file() {
        // No config.json is a normal, expected shape — silent, not a read
        // error.
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
        Ok(v) => Some(BeeConfig {
            gate_bypass: normalize_gate_bypass(v.get("gate_bypass")),
        }),
        Err(e) => {
            read_errors.push(format!("{}: could not parse ({e})", rel_str(&path, root)));
            None
        }
    }
}

/// Normalize `.bee/config.json`'s `gate_bypass` key defensively, because its
/// value type is not stable across stores (`false` here, `"total"` in the
/// beehive store). A missing key (`None`, the caller passes
/// `v.get("gate_bypass")` straight through) and an explicit JSON `false`
/// both mean off (`None`); a JSON string is carried through verbatim,
/// whatever bee itself wrote into it; anything else (a bool `true`, a
/// number, an object, `null`) is carried through as its own JSON text
/// rather than guessed at or coerced to off — a value this reader does not
/// recognize is exactly the case where guessing would be the wrong call.
fn normalize_gate_bypass(v: Option<&Value>) -> Option<String> {
    match v {
        None => None,
        Some(Value::Bool(false)) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    }
}

/// Read `.bee/reservations.json` (bbp-15), following [`read_state`]'s
/// convention exactly: a missing file is silent and normal, a read or
/// parse error pushes one line onto `read_errors` and the rest of the
/// snapshot still reads. The file's own top-level shape is
/// `{"reservations": [...]}`; when that key is absent or is not itself a
/// JSON array — a shape this reader has not observed on either live store
/// it was verified against — the reservation list reads as empty rather
/// than as an error, exactly like [`read_state`]'s handling of
/// `workers[]`.
fn read_reservations(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> Vec<BeeReservation> {
    let path = bee_dir.join("reservations.json");
    if !path.is_file() {
        // No reservations.json is the normal, expected shape (both live
        // stores this reader was verified against hold none) — silent,
        // not a read error.
        return Vec::new();
    }
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) => {
            read_errors.push(format!("{}: could not read ({e})", rel_str(&path, root)));
            return Vec::new();
        }
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(v) => v
            .get("reservations")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_reservation).collect())
            .unwrap_or_default(),
        Err(e) => {
            read_errors.push(format!("{}: could not parse ({e})", rel_str(&path, root)));
            Vec::new()
        }
    }
}

/// Parse one `.bee/reservations.json` `reservations[]` entry. Every field
/// is carried exactly as the store spells it (no renaming, no reshaping) —
/// see [`BeeReservation`]. An array element that is not a JSON object
/// carries nothing to read and is skipped rather than turned into a row of
/// all-`None` fields.
fn parse_reservation(v: &Value) -> Option<BeeReservation> {
    v.as_object()?;
    Some(BeeReservation {
        agent: v.get("agent").and_then(Value::as_str).map(String::from),
        cell: v.get("cell").and_then(Value::as_str).map(String::from),
        path: v.get("path").and_then(Value::as_str).map(String::from),
        kind: v.get("kind").and_then(Value::as_str).map(String::from),
        session: v.get("session").and_then(Value::as_str).map(String::from),
        reserved_at: v.get("reserved_at").and_then(Value::as_str).map(String::from),
        released_at: v.get("released_at").and_then(Value::as_str).map(String::from),
    })
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
        .unwrap_or_default();
    let title = scrub_paths(title, root);
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
    let behavior_change = v.get("behavior_change").and_then(Value::as_bool).unwrap_or(false);

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
        behavior_change,
    })
}

/// Read every cell archived under `.bee/cells/archive/<feature>/` — moved
/// there when the feature closes (D9) — into typed [`BeeCell`]s, reusing
/// the same `parse_cell` parsing and path-scrubbing a live cell gets.
/// Read-only, and deliberately a second, narrower read the two detail
/// routes (feature, cell) call directly: [`read_snapshot`] above never
/// descends into `archive/` (`archived_cells_contribute_to_no_count`), and
/// this function does not change that — the main board's snapshot-wide
/// buckets and KPIs stay archive-free.
pub fn read_archived_cells(root: &Path, feature: &str) -> Vec<BeeCell> {
    let dir = root.join(".bee").join("cells").join("archive").join(feature);
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
        Err(_) => Vec::new(),
    };
    entries.sort();
    entries
        .into_iter()
        .filter_map(|path| parse_cell(&path, root).ok())
        .collect()
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
            let title = v.get("title").and_then(Value::as_str).unwrap_or_default();
            let title = scrub_paths(title, root);
            let status = v.get("status").and_then(Value::as_str).unwrap_or_default().to_string();
            let feature = v.get("feature").and_then(Value::as_str).unwrap_or_default().to_string();
            let cos = v.get("cos").and_then(Value::as_str).unwrap_or_default();
            let cos = scrub_paths(cos, root);
            pbis.insert(id.clone(), BeePbi { id, title, status, feature, cos });
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
                title: scrub_paths(v.get("title").and_then(Value::as_str).unwrap_or_default(), root),
                detail: scrub_paths(v.get("detail").and_then(Value::as_str).unwrap_or_default(), root),
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
        match parse_lane(&path, root) {
            Ok(l) => lanes.push(l),
            Err(e) => read_errors.push(format!("{}: {e}", rel_str(&path, root))),
        }
    }
    lanes
}

fn parse_lane(path: &Path, root: &Path) -> Result<BeeLane, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("could not read ({e})"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("could not parse ({e})"))?;

    let feature = v
        .get("feature")
        .and_then(Value::as_str)
        .ok_or("missing \"feature\"")?
        .to_string();
    let phase = v.get("phase").and_then(Value::as_str).map(String::from);
    let mode = v.get("mode").and_then(Value::as_str).map(String::from);
    let next_action = v
        .get("next_action")
        .and_then(Value::as_str)
        .map(|s| scrub_paths(s, root));
    let approved_gates = parse_approved_gates(&v);
    let created_at = v.get("created_at").and_then(Value::as_str).map(String::from);
    let last_scribing_run = parse_last_scribing_run(&v);

    Ok(BeeLane { feature, phase, mode, next_action, approved_gates, created_at, last_scribing_run })
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
                decision: scrub_paths(v.get("decision").and_then(Value::as_str).unwrap_or_default(), root),
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

/// Shared redaction text for an absolute path that cannot be made relative
/// to the project root — used by [`relativize`], [`rel_str`] and
/// [`scrub_paths`] so the three never invent a second wording.
const ABSOLUTE_PATH_REDACTED: &str = "(absolute path redacted)";

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
            .unwrap_or_else(|| ABSOLUTE_PATH_REDACTED.to_string()),
    }
}

/// Render a path known to be a descendant of `root` relative to it.
fn rel_str(path: &Path, root: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) => to_forward_slashes(rel),
        Err(_) => path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| ABSOLUTE_PATH_REDACTED.to_string()),
    }
}

fn to_forward_slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Scrub every absolute path embedded inside free text — as opposed to
/// [`relativize`], which only handles a string that IS a path in full.
/// Every field this feature renders (`next_action`, `route.rationale`, a
/// handoff paragraph, review-finding prose, a cell or PBI title) is prose
/// that may merely *contain* a path mid-sentence, so `relativize`'s own
/// `is_absolute()` guard is a no-op on it — this is D9's actual gap.
///
/// A string that is wholly an absolute path — no whitespace anywhere in it,
/// so it is a single token — delegates straight to [`relativize`], so the
/// two agree exactly on that case, whole-path reduction included. (A naive
/// `Path::new(s).is_absolute()` check on the *whole* `s` is not enough: it
/// only inspects the leading component, so it would also fire on a
/// sentence that merely *starts* with a path — the same gap this function
/// exists to close, just moved to the front of the string.) Otherwise,
/// every maximal non-whitespace run in `s` is inspected: an absolute path
/// found either as the whole run or wrapped inside it — in parentheses,
/// quotes, backticks, square brackets, angle brackets, or trailed by a
/// comma, period or semicolon, since review-finding prose and ordinary
/// sentences wrap and punctuate paths exactly that way — is reduced in
/// place, with whatever wrapping was trimmed off re-emitted around the
/// reduced result so the prose still reads correctly. A path is reduced —
/// stripped relative to `root` when it falls under `root`, replaced with
/// [`ABSOLUTE_PATH_REDACTED`] when it does not (never reduced to a bare
/// filename in this shape: a filename alone, dropped into a sentence with
/// no path context, reads as a plausible original word rather than a
/// redaction). A run with no absolute path inside it — a relative path, a
/// bare filename, an ordinary word — is carried through byte-for-byte, wrap
/// and all. Everything else in `s` — surrounding words, punctuation,
/// whitespace — is likewise carried through byte-for-byte.
///
/// Absoluteness is judged by [`is_absolute_path_str`], not the platform-only
/// `Path::is_absolute`, so a Windows-shaped path (`C:\Users\...`) is caught
/// on a POSIX host too — a snapshot can be read on either platform.
///
/// Applied at the reader (every free-text field `read_snapshot` produces
/// passes through this before it reaches a public field), never at the
/// view, so the snapshot itself can never carry an absolute path and no
/// future render site can forget to call it.
fn scrub_paths(s: &str, root: &Path) -> String {
    if s.is_empty() {
        return String::new();
    }
    if !s.chars().any(char::is_whitespace) && Path::new(s).is_absolute() {
        return relativize(s, root);
    }

    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let Some(token_start) = rest.find(|c: char| !c.is_whitespace()) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..token_start]);
        let after_ws = &rest[token_start..];
        let token_end = after_ws.find(char::is_whitespace).unwrap_or(after_ws.len());
        let token = &after_ws[..token_end];

        match reduce_wrapped_token(token, root) {
            Some(reduced) => out.push_str(&reduced),
            None => out.push_str(token),
        }

        rest = &after_ws[token_end..];
        if rest.is_empty() {
            break;
        }
    }
    out
}

/// Characters that can open a wrapper around an embedded path in prose:
/// `(quoted)`, `"quoted"`, `` `quoted` ``, `[quoted]`, `<quoted>`.
const PATH_WRAP_OPENERS: &[char] = &['(', '"', '\'', '`', '[', '<'];

/// Characters that can close a wrapper, or trail a path as sentence
/// punctuation: the mirrors of [`PATH_WRAP_OPENERS`] plus a trailing comma,
/// period or semicolon.
const PATH_WRAP_CLOSERS: &[char] = &[')', '"', '\'', '`', ']', '>', ',', '.', ';'];

/// Whether `s` is an absolute path — POSIX (`Path::is_absolute`, which also
/// covers a native Windows target) or Windows-shaped (a drive letter,
/// a colon, then `\` or `/`) so a Windows path is still recognised when
/// scrubbing runs on a POSIX host, as a snapshot committed on Windows can
/// be read on either.
fn is_absolute_path_str(s: &str) -> bool {
    Path::new(s).is_absolute() || is_windows_drive_absolute(s)
}

fn is_windows_drive_absolute(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// Reduce one whitespace-delimited token from [`scrub_paths`] when it
/// contains an absolute path — either as the whole token or wrapped inside
/// leading/trailing delimiter characters (see [`PATH_WRAP_OPENERS`] and
/// [`PATH_WRAP_CLOSERS`]) — by trimming the wrap off, testing the reduced
/// middle for absoluteness, and re-emitting the trimmed wrap around the
/// path's reduction. Returns `None` when the token — wrap trimmed or not —
/// is not an absolute path, so the caller leaves the token byte-identical:
/// a relative path in backticks, a bare filename in parentheses and
/// ordinary prose all fall through here untouched.
fn reduce_wrapped_token(token: &str, root: &Path) -> Option<String> {
    let after_open = token.trim_start_matches(PATH_WRAP_OPENERS);
    let leading = &token[..token.len() - after_open.len()];
    let inner = after_open.trim_end_matches(PATH_WRAP_CLOSERS);
    let trailing = &after_open[inner.len()..];

    if inner.is_empty() || !is_absolute_path_str(inner) {
        return None;
    }

    let mut out = String::with_capacity(leading.len() + trailing.len() + inner.len());
    out.push_str(leading);
    out.push_str(&reduce_embedded_path(inner, root));
    out.push_str(trailing);
    Some(out)
}

/// Reduce one absolute-path string found embedded in free text (see
/// [`scrub_paths`]): stripped relative to `root` when under it, otherwise
/// [`ABSOLUTE_PATH_REDACTED`] — deliberately not the bare-filename fallback
/// `relativize` uses for a whole-string path, since a lone filename dropped
/// into a sentence reads as ordinary prose rather than a redaction.
fn reduce_embedded_path(path: &str, root: &Path) -> String {
    match Path::new(path).strip_prefix(root) {
        Ok(rel) => to_forward_slashes(rel),
        Err(_) => ABSOLUTE_PATH_REDACTED.to_string(),
    }
}

/// The only gate a `feature` name passes through before it is ever joined
/// onto a filesystem path — see [`promote_proposals_path`], the sole call
/// site. `feature` is unvalidated free text everywhere this module reads
/// it — `.bee/state.json`'s active feature, a `.bee/lanes/*.json` record,
/// a `.bee/cells/*.json` record — and none of those three sources is under
/// this code's control, so this check runs the same way regardless of
/// which one a name came from.
///
/// Rejected: an empty string; a leading `.` (this alone covers a bare `.`
/// or `..` component, since both start with `.`, as well as any
/// dotfile-shaped name); either platform's path separator, `/` or `\`,
/// checked unconditionally rather than only on its native platform, since
/// a store written on one OS can be read on another; an absolute-path
/// shape, POSIX or Windows-drive-prefixed (via [`is_windows_drive_absolute`],
/// the same check [`scrub_paths`] already trusts elsewhere in this file);
/// and any control character, NUL included. Anything else — an ordinary
/// slug, hyphens and digits included — passes.
fn validate_feature_name(feature: &str) -> bool {
    if feature.is_empty() {
        return false;
    }
    if feature.starts_with('.') {
        return false;
    }
    if feature.contains('/') || feature.contains('\\') {
        return false;
    }
    if Path::new(feature).is_absolute() || is_windows_drive_absolute(feature) {
        return false;
    }
    if feature.chars().any(|c| c.is_control()) {
        return false;
    }
    true
}

/// Join `feature` onto its promote-proposals path under `root` — the ONLY
/// place in this module a `feature` string is joined onto a filesystem
/// path for this read (bbp-12). Returns `None`, building no [`PathBuf`] at
/// all, when [`validate_feature_name`] rejects `feature`: a rejected name
/// is never looked up, so this function's return value is itself the
/// proof a test needs — call it directly and assert `None`, rather than
/// inferring "no lookup happened" from a rendered page that might simply
/// have hidden the result.
fn promote_proposals_path(root: &Path, feature: &str) -> Option<PathBuf> {
    if !validate_feature_name(feature) {
        return None;
    }
    Some(root.join("docs").join("history").join(feature).join("promote-proposals.md"))
}

/// Whether `feature`'s `docs/history/<feature>/promote-proposals.md`
/// exists — presence only; its contents are never read. `None` when
/// [`promote_proposals_path`] built no path for `feature` (an invalid
/// name): the lookup was never attempted, so the caller reports nothing
/// about that feature's proposals rather than a false "does not exist".
/// Never pushes to `read_errors` — a strange feature name is not a store
/// error (see [`read_snapshot`]).
fn has_promote_proposals(root: &Path, feature: &str) -> Option<bool> {
    promote_proposals_path(root, feature).map(|p| p.is_file())
}

/// Presence-only promote-proposals read (bbp-12) for every distinct
/// `feature` name in `features` — the union [`read_snapshot`] builds from
/// every place this module reads a feature name: `.bee/state.json`'s
/// active feature, every `.bee/lanes/*.json` record, and every
/// `.bee/cells/*.json` record. A name [`has_promote_proposals`] cannot
/// check (fails validation) is simply absent from the returned map — the
/// map's key set is exactly the set of features a path was actually built
/// and checked for, which is what makes it the observable proof that a
/// rejected name built none.
fn read_promote_proposals<'a>(
    root: &Path,
    features: impl Iterator<Item = &'a str>,
) -> std::collections::BTreeMap<String, bool> {
    let mut out = std::collections::BTreeMap::new();
    for feature in features {
        if let Some(present) = has_promote_proposals(root, feature) {
            out.entry(feature.to_string()).or_insert(present);
        }
    }
    out
}

// --- bbp-13: review join, capture queue, scribing debt ---

/// One raw `.bee/review-candidates.jsonl` row, before it is joined against
/// review sessions to derive a status — see [`BeeReviewCandidate`] and
/// [`compute_review`]. Kept private: only the joined, public shape crosses
/// into [`BeeSnapshot`].
struct RawReviewCandidate {
    id: String,
    feature: String,
    mode: Option<String>,
    cells: Vec<String>,
}

/// Read `.bee/review-candidates.jsonl` (bbp-13, D4). A missing file is
/// silent and normal, matching every optional-file precedent in this
/// module. A malformed line, or one missing its own `id`, costs one
/// `read_errors` note and the read continues with whatever else could be
/// parsed.
fn read_review_candidates(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> Vec<RawReviewCandidate> {
    let path = bee_dir.join("review-candidates.jsonl");
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

    let mut out = Vec::new();
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
        let Some(id) = v.get("id").and_then(Value::as_str) else {
            read_errors.push(format!("{}: line {} candidate missing \"id\"", rel_str(&path, root), i + 1));
            continue;
        };
        let feature = v.get("feature").and_then(Value::as_str).unwrap_or_default().to_string();
        let mode = v.get("mode").and_then(Value::as_str).map(String::from);
        // A candidate naming zero cells (`cells: []`) is the shape live in
        // this repo's own store — read exactly like any other array, empty
        // or not; see the module-level pin on what an empty set means in
        // `BeeReviewStatus::Unreviewed`'s own doc comment.
        let cells = v
            .get("cells")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(|c| c.as_str().map(String::from)).collect())
            .unwrap_or_default();
        out.push(RawReviewCandidate { id: id.to_string(), feature, mode, cells });
    }
    out
}

/// One `.bee/reviews/<...>.json` session, trimmed to what [`compute_review`]
/// needs: which ids it covers, how many `P1` findings it carries, and its
/// decision status (`None` when the file has no `decision` key at all).
struct RawReviewSession {
    /// Every id named in `included[]`, regardless of that entry's own
    /// `type` (`cell`, `commit`, `feature`, ...) — a candidate's `cells[]`
    /// only ever holds cell ids, so a commit or feature entry's id simply
    /// never matches one; filtering by `type` first would add a second
    /// place this code could drift from bee's own shape for no benefit.
    included: std::collections::HashSet<String>,
    /// Count of `findings[]` entries whose own `severity` is exactly
    /// `"P1"` — a finding with no `severity` key, or `severity: "info"`,
    /// is never counted here.
    p1_findings: usize,
    /// `decision.status`, when the session carries a `decision` object at
    /// all. `None` for a session with no `decision` key — a different
    /// shape from `Some("pending")`, but both are treated as "not settled"
    /// by [`compute_review`].
    decision_status: Option<String>,
}

/// Read `.bee/reviews/*.json` (bbp-13, D4), in the directory-listing shape
/// [`read_lanes`] already establishes: absent directory yields an empty
/// list, not an error; a malformed file costs one `read_errors` note and
/// the read continues with whatever else could be parsed.
fn read_review_sessions(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> Vec<RawReviewSession> {
    let dir = bee_dir.join("reviews");
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
            read_errors.push(format!(".bee/reviews: could not list ({e})"));
            Vec::new()
        }
    };
    entries.sort();

    let mut sessions = Vec::new();
    for path in entries {
        match parse_review_session(&path) {
            Ok(s) => sessions.push(s),
            Err(e) => read_errors.push(format!("{}: {e}", rel_str(&path, root))),
        }
    }
    sessions
}

/// Parse one `.bee/reviews/<...>.json` file. `included[]` entries are the
/// real shape observed on disk — `{"type": "cell"|"commit"|..., "id": ...}`
/// — but a bare string entry is accepted too, defensively, since the join
/// only ever tests string membership either way. A session naming a cell
/// that no longer exists in this snapshot's own `.bee/cells/` is not
/// validated here at all — `included` is simply the set of ids the session
/// itself claims, exactly as written; the join in [`compute_review`] never
/// needs to know whether any of them still exist.
fn parse_review_session(path: &Path) -> Result<RawReviewSession, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("could not read ({e})"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("could not parse ({e})"))?;

    let included: std::collections::HashSet<String> = v
        .get("included")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| match entry {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(_) => entry.get("id").and_then(Value::as_str).map(String::from),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    let p1_findings = v
        .get("findings")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter(|f| f.get("severity").and_then(Value::as_str) == Some("P1")).count())
        .unwrap_or(0);

    let decision_status = v.get("decision").and_then(|d| d.get("status")).and_then(Value::as_str).map(String::from);

    Ok(RawReviewSession { included, p1_findings, decision_status })
}

/// Join `.bee/review-candidates.jsonl` rows against `.bee/reviews/*.json`
/// sessions (bbp-13, D4 — no I/O, over data already read) — see
/// [`BeeReviewStatus`] for the exact status rule this applies per
/// candidate, and [`BeeReview::open_p1_findings`] for the independent P1
/// count.
fn compute_review(candidates: &[RawReviewCandidate], sessions: &[RawReviewSession]) -> BeeReview {
    let is_settled = |s: &&RawReviewSession| matches!(s.decision_status.as_deref(), Some("approved") | Some("blocked"));

    let out_candidates = candidates
        .iter()
        .map(|c| {
            let matching: Vec<&RawReviewSession> =
                sessions.iter().filter(|s| c.cells.iter().any(|cell| s.included.contains(cell))).collect();
            let status = if matching.is_empty() {
                BeeReviewStatus::Unreviewed
            } else if matching.iter().any(|s| !is_settled(s)) {
                BeeReviewStatus::InReview
            } else {
                BeeReviewStatus::Settled
            };
            BeeReviewCandidate { id: c.id.clone(), feature: c.feature.clone(), mode: c.mode.clone(), status }
        })
        .collect();

    let open_p1_findings = sessions.iter().filter(|s| !is_settled(s)).map(|s| s.p1_findings).sum();

    BeeReview { candidates: out_candidates, open_p1_findings }
}

/// Read `.bee/capture-queue.jsonl` (bbp-13, D4). Lines of `kind: "stub"` (a
/// note waiting to be written into documentation) and `kind: "flush"` (a
/// record of a specific stub — named by its own `id` — having been
/// written). `waiting` is net of flushes: a stub whose own `id` has a
/// matching flush id is no longer waiting, never a raw stub count. A
/// missing file is silent and normal (most stores have none); a malformed
/// line, or a `stub`/`flush` row missing its own `id`, costs one
/// `read_errors` note and the read continues with whatever else could be
/// parsed. A row of any other `kind` (or none at all) is ignored, matching
/// this module's unknown-status convention elsewhere — an unrecognised
/// kind is not a store error.
fn read_capture_queue(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> BeeCaptureQueue {
    let path = bee_dir.join("capture-queue.jsonl");
    if !path.is_file() {
        return BeeCaptureQueue::default();
    }
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) => {
            read_errors.push(format!("{}: could not read ({e})", rel_str(&path, root)));
            return BeeCaptureQueue::default();
        }
    };

    let mut stub_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut flushed_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

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
        let kind = v.get("kind").and_then(Value::as_str);
        if kind != Some("stub") && kind != Some("flush") {
            // an unrecognised kind, or a row with none at all: ignored, not
            // a store error.
            continue;
        }
        let Some(id) = v.get("id").and_then(Value::as_str) else {
            read_errors.push(format!(
                "{}: line {} {} row missing \"id\"",
                rel_str(&path, root),
                i + 1,
                kind.unwrap()
            ));
            continue;
        };
        if kind == Some("stub") {
            stub_ids.insert(id.to_string());
        } else {
            flushed_ids.insert(id.to_string());
        }
    }

    BeeCaptureQueue { waiting: stub_ids.difference(&flushed_ids).count() }
}

/// Per-feature scribing debt (bbp-13, Terms: "Knowledge debt"): a feature
/// has debt when at least one of its cells is `capped` with
/// `behavior_change: true` and its own `last_scribing_run` — read from
/// `.bee/state.json` for the active feature, from its own
/// `.bee/lanes/<feature>.json` record for a lane feature, exactly the same
/// lane-wins-over-state precedence [`compute_phase_board`] already
/// establishes for `phase`/`approved_gates` — does not name it. No cell
/// carries a "was this captured" flag; this is the only place that signal
/// can come from, and only for a feature this snapshot can place at all
/// (the `lanes ∪ {active feature}` union `phase_board` already is) — a
/// feature with capped behavior_change work but neither a lane record nor
/// the active slot cannot be checked here and is silently absent from the
/// result, never guessed at either way.
fn compute_scribing_debt(
    phase_board: &[BeeFeaturePhase],
    lanes: &[BeeLane],
    state: Option<&BeeState>,
    all_cells: &[BeeCell],
) -> Vec<String> {
    let mut debt = Vec::new();
    for placement in phase_board {
        let feature = placement.feature.as_str();
        let has_capped_behavior_change =
            all_cells.iter().any(|c| c.feature == feature && c.status == "capped" && c.behavior_change);
        if !has_capped_behavior_change {
            continue;
        }
        let last_scribing_run: Option<&BeeLastScribingRun> = match lanes.iter().find(|l| l.feature == feature) {
            Some(l) => l.last_scribing_run.as_ref(),
            None => state
                .filter(|s| s.feature.as_deref() == Some(feature))
                .and_then(|s| s.last_scribing_run.as_ref()),
        };
        let names_it = last_scribing_run.and_then(|l| l.feature.as_deref()) == Some(feature);
        if !names_it {
            debt.push(feature.to_string());
        }
    }
    debt
}

/// Pure derivation over already-read cells (bbp-15) — see [`BeeTierMix`].
/// `None` only when there are no cells at all to measure; an empty store
/// reports absence, never a zeroed-out mix presented as a measurement.
fn compute_tier_mix(all_cells: &[BeeCell]) -> Option<BeeTierMix> {
    if all_cells.is_empty() {
        return None;
    }
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut untiered = 0usize;
    for cell in all_cells {
        match cell.tier.as_deref() {
            Some(t) => *counts.entry(t.to_string()).or_insert(0) += 1,
            None => untiered += 1,
        }
    }
    let tiered_total: usize = counts.values().sum();
    let expensive_tier_share = if tiered_total == 0 {
        None
    } else {
        let expensive = counts.get("ceiling").copied().unwrap_or(0);
        Some(expensive as f64 / tiered_total as f64)
    };
    Some(BeeTierMix { counts, untiered, expensive_tier_share })
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
            r#"{"kind":"pbi","id":"P1","title":"Widget","status":"proposed","feature":"demo","cos":"first cut"}"#,
            r#"{"kind":"pbi","id":"P1","title":"Widget","status":"in-flight","feature":"demo","cos":"second cut"}"#,
            r#"{"kind":"pbi","id":"P1","title":"Widget","status":"done","feature":"demo","cos":"final cut"}"#,
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
        assert_eq!(snap.backlog.pbis[0].cos, "final cut", "cos must fold to the LAST event too, not the first");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn pbi_missing_cos_folds_to_empty_string() {
        let root = fresh_root("pbi-cos-missing");
        let lines = [r#"{"kind":"pbi","id":"P1","title":"Widget","status":"proposed","feature":"demo"}"#];
        write(&root, ".bee/backlog.jsonl", &lines.join("\n"));

        let snap = read_snapshot(&root);
        assert_eq!(snap.backlog.pbis.len(), 1);
        assert_eq!(snap.backlog.pbis[0].cos, "", "a missing cos field must fold to an empty string, like title");

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

    // --- bbp-1: scrub_paths, the free-text absolute-path scrubber (D9) ---

    #[test]
    fn scrub_paths_reduces_a_path_embedded_mid_sentence_words_survive() {
        let root = fresh_root("scrub-mid-sentence");
        let file = root.join("src").join("bee.rs").to_string_lossy().into_owned();
        let text = format!("Please look at {file} before you continue.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&file), "absolute path survived scrubbing: {scrubbed}");
        assert!(scrubbed.starts_with("Please look at "), "leading words dropped: {scrubbed}");
        assert!(scrubbed.ends_with(" before you continue."), "trailing words dropped: {scrubbed}");
        assert!(scrubbed.contains("src/bee.rs"), "path was not reduced relative to root: {scrubbed}");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_reduces_several_paths_in_one_string() {
        let root = fresh_root("scrub-several");
        let a = root.join("a.md").to_string_lossy().into_owned();
        let b = root.join("sub").join("b.md").to_string_lossy().into_owned();
        let text = format!("{a} and also {b} both matter.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&a), "first path survived: {scrubbed}");
        assert!(!scrubbed.contains(&b), "second path survived: {scrubbed}");
        assert!(scrubbed.contains("a.md"), "first path was not reduced: {scrubbed}");
        assert!(scrubbed.contains("sub/b.md"), "second path was not reduced: {scrubbed}");
        assert!(scrubbed.contains(" and also "), "middle words dropped: {scrubbed}");
        assert!(scrubbed.ends_with(" both matter."), "trailing words dropped: {scrubbed}");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_leaves_a_path_free_string_byte_identical() {
        let root = fresh_root("scrub-no-path");
        let text = "Execute c-1, then cap the cell and move on.";

        assert_eq!(scrub_paths(text, &root), text);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_leaves_an_empty_string_empty() {
        let root = fresh_root("scrub-empty");

        assert_eq!(scrub_paths("", &root), "");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_on_a_string_that_is_wholly_a_path_matches_relativize_exactly() {
        let root = fresh_root("scrub-wholly-path");

        let inside = root.join("crates").join("bee.rs").to_string_lossy().into_owned();
        assert_eq!(scrub_paths(&inside, &root), relativize(&inside, &root));

        let outside = std::env::temp_dir()
            .join("mdview-bee-scrub-wholly-outside")
            .to_string_lossy()
            .into_owned();
        assert_eq!(scrub_paths(&outside, &root), relativize(&outside, &root));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_redacts_an_embedded_path_outside_the_root_rather_than_relativizing_it() {
        let root = fresh_root("scrub-outside-embedded");
        let outside = std::env::temp_dir()
            .join("mdview-bee-scrub-outside-embedded-target")
            .join("secret.txt")
            .to_string_lossy()
            .into_owned();
        let text = format!("See {outside} for details.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&outside), "absolute path outside root survived: {scrubbed}");
        assert!(
            !scrubbed.contains("secret.txt"),
            "an outside-root path must be redacted, not reduced to a bare filename: {scrubbed}"
        );
        assert!(scrubbed.contains(ABSOLUTE_PATH_REDACTED), "expected the shared redaction text: {scrubbed}");
        assert!(scrubbed.starts_with("See "), "leading words dropped: {scrubbed}");
        assert!(scrubbed.ends_with(" for details."), "trailing words dropped: {scrubbed}");

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-6: scrub_paths finds a path wrapped in a delimiter (D9) ---

    #[test]
    fn scrub_paths_reduces_a_path_wrapped_in_parentheses_wrap_survives() {
        let root = fresh_root("scrub-wrap-parens");
        let file = root.join("crates").join("bee.rs").to_string_lossy().into_owned();
        let text = format!("see ({file}) for details.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&file), "absolute path survived scrubbing: {scrubbed}");
        assert!(scrubbed.contains("(crates/bee.rs)"), "wrapping parens were not preserved: {scrubbed}");
        assert_eq!(scrubbed, "see (crates/bee.rs) for details.");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_reduces_a_path_wrapped_in_double_quotes_wrap_survives() {
        let root = fresh_root("scrub-wrap-dquote");
        let file = root.join("crates").join("bee.rs").to_string_lossy().into_owned();
        let text = format!("path was \"{file}\" at the time.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&file), "absolute path survived scrubbing: {scrubbed}");
        assert_eq!(scrubbed, "path was \"crates/bee.rs\" at the time.");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_reduces_a_path_wrapped_in_backticks_wrap_survives() {
        let root = fresh_root("scrub-wrap-backtick");
        let file = root.join("crates").join("bee.rs").to_string_lossy().into_owned();
        let text = format!("review found `{file}` reads the whole tree.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&file), "absolute path survived scrubbing: {scrubbed}");
        assert_eq!(scrubbed, "review found `crates/bee.rs` reads the whole tree.");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_reduces_a_path_wrapped_in_square_brackets_wrap_survives() {
        let root = fresh_root("scrub-wrap-brackets");
        let file = root.join("crates").join("bee.rs").to_string_lossy().into_owned();
        let text = format!("see [{file}] for the source.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&file), "absolute path survived scrubbing: {scrubbed}");
        assert_eq!(scrubbed, "see [crates/bee.rs] for the source.");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_reduces_a_path_wrapped_in_angle_brackets_wrap_survives() {
        let root = fresh_root("scrub-wrap-angle");
        let file = root.join("crates").join("bee.rs").to_string_lossy().into_owned();
        let text = format!("open <{file}> in the editor.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&file), "absolute path survived scrubbing: {scrubbed}");
        assert_eq!(scrubbed, "open <crates/bee.rs> in the editor.");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_reduces_a_path_trailed_by_a_comma_punctuation_survives() {
        let root = fresh_root("scrub-trail-comma");
        let file = root.join("crates").join("bee.rs").to_string_lossy().into_owned();
        let text = format!("{file}, then re-run the tests.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&file), "absolute path survived scrubbing: {scrubbed}");
        assert_eq!(scrubbed, "crates/bee.rs, then re-run the tests.");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_reduces_a_path_trailed_by_a_period_punctuation_survives() {
        let root = fresh_root("scrub-trail-period");
        let file = root.join("crates").join("bee.rs").to_string_lossy().into_owned();
        let text = format!("Read {file}.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&file), "absolute path survived scrubbing: {scrubbed}");
        assert_eq!(scrubbed, "Read crates/bee.rs.");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_reduces_a_path_trailed_by_a_semicolon_punctuation_survives() {
        let root = fresh_root("scrub-trail-semicolon");
        let file = root.join("crates").join("bee.rs").to_string_lossy().into_owned();
        let text = format!("{file}; check it next.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&file), "absolute path survived scrubbing: {scrubbed}");
        assert_eq!(scrubbed, "crates/bee.rs; check it next.");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_reduces_a_wrapped_path_outside_the_root_to_the_shared_redaction_text() {
        let root = fresh_root("scrub-wrap-outside");
        let outside = std::env::temp_dir()
            .join("mdview-bee-scrub-wrap-outside-target")
            .join("secret.txt")
            .to_string_lossy()
            .into_owned();
        let text = format!("see (`{outside}`) for the leak.");

        let scrubbed = scrub_paths(&text, &root);

        assert!(!scrubbed.contains(&outside), "absolute path outside root survived: {scrubbed}");
        assert!(
            !scrubbed.contains("secret.txt"),
            "an outside-root path must be redacted, not reduced to a bare filename: {scrubbed}"
        );
        assert_eq!(scrubbed, format!("see (`{ABSOLUTE_PATH_REDACTED}`) for the leak."));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_handles_a_windows_shaped_absolute_path_embedded_mid_sentence() {
        let root = fresh_root("scrub-windows-path");
        let text = "see `C:\\Users\\alice\\.ssh\\id_rsa` before you continue.";

        let scrubbed = scrub_paths(text, &root);

        assert!(!scrubbed.contains("C:\\Users\\alice"), "windows-shaped absolute path survived: {scrubbed}");
        assert_eq!(scrubbed, format!("see `{ABSOLUTE_PATH_REDACTED}` before you continue."));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_leaves_a_relative_path_in_backticks_unchanged() {
        let root = fresh_root("scrub-wrap-relative");
        let text = "see `docs/history/bee-board-pm/CONTEXT.md` for the decision.";

        assert_eq!(scrub_paths(text, &root), text);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_leaves_a_bare_filename_in_parentheses_unchanged() {
        let root = fresh_root("scrub-wrap-bare-filename");
        let text = "see (bee.rs) for the reader.";

        assert_eq!(scrub_paths(text, &root), text);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scrub_paths_leaves_ordinary_prose_byte_identical() {
        let root = fresh_root("scrub-wrap-prose");
        let text = "Execute (c-1), then cap the cell; move on, review next.";

        assert_eq!(scrub_paths(text, &root), text);

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-3: state.json gates, route and next_action ---

    #[test]
    fn state_full_gates_route_and_next_action_are_read() {
        let root = fresh_root("state-full");
        write(
            &root,
            ".bee/state.json",
            r#"{
                "phase": "swarming",
                "feature": "demo",
                "mode": "standard",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": false},
                "route": {
                    "class": "feature",
                    "lane": "standard",
                    "flags": ["cross-platform"],
                    "product_files": 3,
                    "rationale": "Small, well-scoped change.",
                    "updated_at": "2026-08-01T00:00:00.000Z"
                },
                "next_action": "Invoke bee-swarming."
            }"#,
        );

        let snap = read_snapshot(&root);
        let state = snap.state.as_ref().expect("state.json should have been read");

        let gates = state.approved_gates.as_ref().expect("approved_gates should be Some");
        assert_eq!(gates.context, Some(true));
        assert_eq!(gates.shape, Some(true));
        assert_eq!(gates.execution, Some(true));
        assert_eq!(gates.review, Some(false));

        let route = state.route.as_ref().expect("route should be Some");
        assert_eq!(route.class.as_deref(), Some("feature"));
        assert_eq!(route.lane.as_deref(), Some("standard"));
        assert_eq!(route.flags, vec!["cross-platform".to_string()]);
        assert_eq!(route.product_files, Some(3));
        assert_eq!(route.rationale.as_deref(), Some("Small, well-scoped change."));
        assert_eq!(route.updated_at.as_deref(), Some("2026-08-01T00:00:00.000Z"));

        assert_eq!(state.next_action.as_deref(), Some("Invoke bee-swarming."));
        assert!(snap.read_errors.is_empty(), "no read error expected: {:?}", snap.read_errors);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn state_missing_gates_route_and_next_action_degrades_silently() {
        let root = fresh_root("state-missing-new-fields");
        write(&root, ".bee/state.json", r#"{"phase": "swarming", "feature": "demo", "mode": "standard"}"#);

        let snap = read_snapshot(&root);
        let state = snap.state.as_ref().expect("state.json should have been read");

        assert!(state.approved_gates.is_none(), "approved_gates must be None, never fabricated");
        assert!(state.gate_revoked_at.is_none(), "gate_revoked_at must be None, never fabricated");
        assert!(state.route.is_none(), "route must be None, never fabricated");
        assert!(state.next_action.is_none(), "next_action must be None, never fabricated");
        assert!(
            snap.read_errors.is_empty(),
            "missing optional keys must not be a read error: {:?}",
            snap.read_errors
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn state_route_with_unknown_extra_key_is_read_without_complaint() {
        let root = fresh_root("state-route-drift");
        // Both a bee-2.2.2-shaped `feature` key and this repo's `demoted_at`
        // key, neither of which this reader ever looks at.
        write(
            &root,
            ".bee/state.json",
            r#"{
                "route": {
                    "class": "feature",
                    "lane": "small",
                    "flags": [],
                    "product_files": 1,
                    "rationale": "Trivial.",
                    "updated_at": "2026-08-01T00:00:00.000Z",
                    "feature": "windows-shell-doctrine",
                    "demoted_at": "2026-08-05T06:42:54.494Z"
                }
            }"#,
        );

        let snap = read_snapshot(&root);
        let state = snap.state.as_ref().expect("state.json should have been read");
        let route = state.route.as_ref().expect("route should be Some despite the unknown keys");
        assert_eq!(route.class.as_deref(), Some("feature"));
        assert_eq!(route.lane.as_deref(), Some("small"));
        assert!(snap.read_errors.is_empty(), "an unknown route key must not be a read error: {:?}", snap.read_errors);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn state_gate_revoked_at_is_carried_alongside_a_still_true_approved_gate() {
        let root = fresh_root("state-gate-revoked");
        write(
            &root,
            ".bee/state.json",
            r#"{
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": false},
                "gate_revoked_at": {"execution": "2026-08-05T09:51:47.038Z"}
            }"#,
        );

        let snap = read_snapshot(&root);
        let state = snap.state.as_ref().expect("state.json should have been read");

        let gates = state.approved_gates.as_ref().expect("approved_gates should be Some");
        assert_eq!(gates.execution, Some(true), "the store still marks execution approved");

        let revoked = state.gate_revoked_at.as_ref().expect("gate_revoked_at should be Some");
        assert_eq!(
            revoked.execution.as_deref(),
            Some("2026-08-05T09:51:47.038Z"),
            "the revocation timestamp must be readable alongside the still-true approval"
        );
        assert!(revoked.context.is_none(), "context was never revoked");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn state_absolute_path_embedded_in_rationale_and_next_action_does_not_survive() {
        let root = fresh_root("state-security-scrub");
        let secret = root.join("src").join("bee.rs").to_string_lossy().into_owned();
        let body = format!(
            r#"{{
                "route": {{
                    "class": "feature",
                    "lane": "standard",
                    "flags": [],
                    "product_files": 1,
                    "rationale": "See {rationale_path} before merging.",
                    "updated_at": "2026-08-01T00:00:00.000Z"
                }},
                "next_action": "Read {next_action_path} then continue."
            }}"#,
            rationale_path = secret.replace('\\', "\\\\"),
            next_action_path = secret.replace('\\', "\\\\"),
        );
        write(&root, ".bee/state.json", &body);

        let snap = read_snapshot(&root);
        let serialized = serde_json::to_string(&snap).unwrap();

        assert!(!serialized.contains(&secret), "an absolute path embedded in free text leaked into the snapshot");

        let state = snap.state.as_ref().expect("state.json should have been read");
        let route = state.route.as_ref().expect("route should be Some");
        assert!(
            route.rationale.as_deref().unwrap_or_default().contains("src/bee.rs"),
            "rationale should still carry the reduced relative path: {:?}",
            route.rationale
        );
        assert!(
            state.next_action.as_deref().unwrap_or_default().contains("src/bee.rs"),
            "next_action should still carry the reduced relative path: {:?}",
            state.next_action
        );

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-8: the HANDOFF.json reader (D6/D9) ---

    #[test]
    fn full_handoff_json_is_carried_onto_the_snapshot_with_all_three_fields() {
        let root = fresh_root("handoff-full");
        write(
            &root,
            ".bee/HANDOFF.json",
            r#"{
                "written_at": "2026-08-06T12:45:21.418Z",
                "next_action": "Resume the next slice.",
                "kind": "pause"
            }"#,
        );

        let snap = read_snapshot(&root);
        assert!(snap.read_errors.is_empty(), "no read error expected: {:?}", snap.read_errors);
        let handoff = snap.handoff.as_ref().expect("handoff should have been read");
        assert_eq!(handoff.written_at.as_deref(), Some("2026-08-06T12:45:21.418Z"));
        assert_eq!(handoff.next_action.as_deref(), Some("Resume the next slice."));
        assert_eq!(handoff.kind.as_deref(), Some("pause"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_handoff_json_at_all_is_silent_no_read_error() {
        let root = fresh_root("handoff-absent");
        // No .bee/HANDOFF.json written — most stores have none.
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));

        let snap = read_snapshot(&root);
        assert!(snap.handoff.is_none(), "no handoff file should mean no handoff on the snapshot");
        assert!(snap.read_errors.is_empty(), "an absent handoff must never be a read error: {:?}", snap.read_errors);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_handoff_json_pushes_exactly_one_read_error_rest_still_reads() {
        let root = fresh_root("handoff-malformed");
        write(&root, ".bee/HANDOFF.json", "{ this is not valid json");
        write(&root, ".bee/cells/good.json", &cell_json("good", "open"));

        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert!(snap.handoff.is_none());
        assert_eq!(snap.buckets.waiting.len(), 1, "the well-formed cell must still parse");
        assert_eq!(snap.read_errors.len(), 1, "expected exactly one read error: {:?}", snap.read_errors);
        assert!(snap.read_errors.iter().any(|e| e.contains("HANDOFF.json")));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn handoff_with_no_kind_key_is_still_carried_with_kind_none() {
        let root = fresh_root("handoff-no-kind");
        write(
            &root,
            ".bee/HANDOFF.json",
            r#"{"written_at": "2026-08-06T00:00:00.000Z", "next_action": "Do the thing."}"#,
        );

        let snap = read_snapshot(&root);
        assert!(snap.read_errors.is_empty(), "a missing key is not a parse error: {:?}", snap.read_errors);
        let handoff = snap.handoff.as_ref().expect("handoff should have been read");
        assert!(handoff.kind.is_none());
        assert_eq!(handoff.next_action.as_deref(), Some("Do the thing."));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn handoff_absolute_path_embedded_mid_sentence_and_backtick_wrapped_does_not_survive() {
        let root = fresh_root("handoff-security-scrub");
        let bare = root.join("src").join("bee.rs").to_string_lossy().into_owned();
        let wrapped = root.join("crates").join("bee.rs").to_string_lossy().into_owned();
        let body = format!(
            r#"{{
                "written_at": "2026-08-06T00:00:00.000Z",
                "next_action": "See {bare_path} then `{wrapped_path}` before resuming.",
                "kind": "pause"
            }}"#,
            bare_path = bare.replace('\\', "\\\\"),
            wrapped_path = wrapped.replace('\\', "\\\\"),
        );
        write(&root, ".bee/HANDOFF.json", &body);

        let snap = read_snapshot(&root);
        let serialized = serde_json::to_string(&snap).unwrap();
        assert!(!serialized.contains(&bare), "a bare absolute path embedded mid-sentence leaked onto the snapshot");
        assert!(!serialized.contains(&wrapped), "a backtick-wrapped absolute path leaked onto the snapshot");

        let handoff = snap.handoff.as_ref().expect("handoff should have been read");
        let note = handoff.next_action.as_deref().unwrap_or_default();
        assert!(note.contains("src/bee.rs"), "bare path should still carry its reduced relative form: {note}");
        assert!(note.contains("`crates/bee.rs`"), "backtick wrap should survive around the reduced path: {note}");

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-4: the attention list (D6) ---

    #[test]
    fn blocked_cells_yield_one_critical_attention_item_naming_a_suggested_action() {
        let root = fresh_root("attention-blocked");
        write(&root, ".bee/cells/c-blocked-1.json", &cell_json("c-blocked-1", "blocked"));
        write(&root, ".bee/cells/c-blocked-2.json", &cell_json("c-blocked-2", "blocked"));
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));

        let snap = read_snapshot(&root);
        assert_eq!(snap.buckets.stuck.len(), 2, "both blocked cells should be in the stuck bucket");
        assert_eq!(snap.attention.len(), 1, "one rule fired, exactly one item: {:?}", snap.attention);

        let item = &snap.attention[0];
        assert_eq!(item.severity, BeeAttentionSeverity::Critical);
        assert!(item.title.contains('2') && item.title.contains("cells"), "title: {}", item.title);
        assert!(item.detail.contains("c-blocked-1"), "detail: {}", item.detail);
        assert!(item.detail.contains("c-blocked-2"), "detail: {}", item.detail);
        assert!(!item.suggested_action.trim().is_empty());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_errors_yield_one_critical_attention_item_naming_a_suggested_action() {
        let root = fresh_root("attention-read-errors");
        write(&root, ".bee/cells/good.json", &cell_json("good", "open"));
        // A genuinely truncated file, parsed through the real code path —
        // the same shape `malformed_state_and_truncated_cell_degrade_to_partial_snapshot`
        // already proves produces one `read_errors` entry.
        write(&root, ".bee/cells/bad.json", "{\"id\": \"bad\", \"status\": \"open\"");

        let snap = read_snapshot(&root);
        assert!(snap.buckets.stuck.is_empty(), "no blocked cell in this fixture");
        assert_eq!(snap.read_errors.len(), 1, "expected one read error: {:?}", snap.read_errors);
        assert_eq!(snap.attention.len(), 1, "one rule fired, exactly one item: {:?}", snap.attention);

        let item = &snap.attention[0];
        assert_eq!(item.severity, BeeAttentionSeverity::Critical);
        assert!(item.title.contains('1') && item.title.contains("file"), "title: {}", item.title);
        assert!(item.detail.contains("bad.json"), "detail: {}", item.detail);
        assert!(!item.suggested_action.trim().is_empty());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn blocked_cells_and_read_errors_together_yield_both_items_heaviest_first_in_a_stable_order() {
        let root = fresh_root("attention-both");
        write(&root, ".bee/cells/c-blocked.json", &cell_json("c-blocked", "blocked"));
        write(&root, ".bee/cells/bad.json", "{\"id\": \"bad\", \"status\": \"open\"");

        let snap = read_snapshot(&root);
        assert_eq!(snap.attention.len(), 2, "both rules should fire: {:?}", snap.attention);

        // Both rules in this slice carry equal (Critical) severity — the
        // real-store shape that proves the stable, rule-registration order
        // a plain severity sort alone would not guarantee.
        assert_eq!(snap.attention[0].severity, BeeAttentionSeverity::Critical);
        assert_eq!(snap.attention[1].severity, BeeAttentionSeverity::Critical);
        assert!(
            snap.attention[0].title.contains("blocked"),
            "blocked-cells item should sort first: {:?}",
            snap.attention
        );
        assert!(
            snap.attention[1].title.contains("could not be read"),
            "read-errors item should sort second: {:?}",
            snap.attention
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn clean_snapshot_yields_no_attention_items() {
        let root = fresh_root("attention-clean");
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));
        write(&root, ".bee/cells/c-done.json", &cell_json("c-done", "capped"));

        let snap = read_snapshot(&root);
        assert!(snap.read_errors.is_empty(), "no read error expected: {:?}", snap.read_errors);
        assert!(
            snap.attention.is_empty(),
            "nothing wrong should yield an empty list, not a placeholder item: {:?}",
            snap.attention
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn same_severity_items_return_in_a_deterministic_order_across_repeated_calls() {
        let root = fresh_root("attention-determinism");
        write(&root, ".bee/cells/c-blocked.json", &cell_json("c-blocked", "blocked"));
        write(&root, ".bee/cells/bad.json", "{\"id\": \"bad\", \"status\": \"open\"");

        // Real-shaped input, read once; the pure computation is then
        // called twice over that same input, standing in for two
        // independent requests against unchanged data.
        let snap = read_snapshot(&root);
        let gate_bypass = snap.config.as_ref().and_then(|c| c.gate_bypass.as_deref());
        let first = compute_attention_items(
            &snap.buckets.stuck,
            &snap.read_errors,
            snap.handoff.as_ref(),
            gate_bypass,
            &snap.review,
            &snap.scribing_debt,
            &snap.capture_queue,
            &snap.promote_proposals,
        );
        let second = compute_attention_items(
            &snap.buckets.stuck,
            &snap.read_errors,
            snap.handoff.as_ref(),
            gate_bypass,
            &snap.review,
            &snap.scribing_debt,
            &snap.capture_queue,
            &snap.promote_proposals,
        );

        assert_eq!(first.len(), 2);
        assert_eq!(first, second, "repeated calls over unchanged data must return the same order");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn computing_attention_perturbs_no_other_snapshot_field() {
        let root = fresh_root("attention-no-side-effect");
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));
        write(&root, ".bee/cells/c-claimed.json", &cell_json("c-claimed", "claimed"));
        write(&root, ".bee/cells/c-blocked.json", &cell_json("c-blocked", "blocked"));
        write(&root, ".bee/cells/c-capped.json", &cell_json("c-capped", "capped"));
        write(&root, ".bee/cells/c-dropped.json", &cell_json("c-dropped", "dropped"));
        write(&root, ".bee/cells/bad.json", "{\"id\": \"bad\", \"status\": \"open\"");

        let snap = read_snapshot(&root);

        // The attention rules fired (both, since a blocked cell and a read
        // error are both present) without changing anything they read from.
        assert_eq!(snap.attention.len(), 2, "both rules should fire: {:?}", snap.attention);
        assert_eq!(snap.buckets.doing.len(), 1, "claimed bucket unperturbed");
        assert_eq!(snap.buckets.waiting.len(), 1, "open bucket unperturbed");
        assert_eq!(snap.buckets.stuck.len(), 1, "blocked bucket unperturbed");
        assert_eq!(snap.buckets.done.len(), 1, "capped bucket unperturbed");
        assert!(snap.active, "an open/claimed cell should still mark the snapshot active");
        assert_eq!(snap.read_errors.len(), 1, "read_errors unperturbed: {:?}", snap.read_errors);
        assert!(
            snap.read_errors.iter().any(|e| e.contains("bad.json")),
            "the original read error should still be present: {:?}",
            snap.read_errors
        );

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-8: a pause handoff is one attention item (D6) ---

    #[test]
    fn pause_handoff_yields_exactly_one_item_carrying_the_note_text_and_written_at() {
        let root = fresh_root("attention-handoff-pause");
        write(
            &root,
            ".bee/HANDOFF.json",
            r#"{
                "written_at": "2026-08-06T12:45:21.418Z",
                "next_action": "Resume the next slice.",
                "kind": "pause"
            }"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.attention.len(), 1, "one rule fired, exactly one item: {:?}", snap.attention);

        let item = &snap.attention[0];
        assert_eq!(item.severity, BeeAttentionSeverity::Critical);
        assert!(item.detail.contains("Resume the next slice."), "detail should carry the note's own text: {}", item.detail);
        assert!(
            item.detail.contains("2026-08-06T12:45:21.418Z"),
            "detail should carry the note's written_at: {}",
            item.detail
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn planned_next_handoff_yields_no_pause_item() {
        let root = fresh_root("attention-handoff-planned-next");
        write(
            &root,
            ".bee/HANDOFF.json",
            r#"{
                "written_at": "2026-08-06T12:45:21.418Z",
                "next_action": "Cell bbp-9 is already claimed by this session.",
                "kind": "planned-next"
            }"#,
        );

        let snap = read_snapshot(&root);
        assert!(
            snap.attention.is_empty(),
            "a planned-next handoff is not a pause and must not be reported as one: {:?}",
            snap.attention
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_handoff_yields_no_pause_item() {
        let root = fresh_root("attention-handoff-absent");
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));

        let snap = read_snapshot(&root);
        assert!(snap.attention.is_empty(), "no handoff, nothing to report: {:?}", snap.attention);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn handoff_alongside_blocked_cells_yields_both_ordered_heaviest_first() {
        let root = fresh_root("attention-handoff-and-blocked");
        write(&root, ".bee/cells/c-blocked.json", &cell_json("c-blocked", "blocked"));
        write(
            &root,
            ".bee/HANDOFF.json",
            r#"{
                "written_at": "2026-08-06T12:45:21.418Z",
                "next_action": "Resume the next slice.",
                "kind": "pause"
            }"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.attention.len(), 2, "both rules should fire: {:?}", snap.attention);
        // Both rules in this slice carry Critical severity — a stable sort
        // keeps the fixed rule-registration order, blocked cells first.
        assert_eq!(snap.attention[0].severity, BeeAttentionSeverity::Critical);
        assert_eq!(snap.attention[1].severity, BeeAttentionSeverity::Critical);
        assert!(
            snap.attention.iter().any(|i| i.title.contains("blocked")),
            "blocked-cells item missing: {:?}",
            snap.attention
        );
        assert!(
            snap.attention.iter().any(|i| i.title.contains("parked")),
            "handoff item missing: {:?}",
            snap.attention
        );

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-9: the config.json reader and the gate-bypass rule (D4/D6) ---

    #[test]
    fn gate_bypass_string_value_is_carried_through_as_itself() {
        let root = fresh_root("config-bypass-total");
        write(&root, ".bee/config.json", r#"{"gate_bypass": "total"}"#);

        let snap = read_snapshot(&root);
        assert!(snap.read_errors.is_empty(), "no read error expected: {:?}", snap.read_errors);
        let config = snap.config.as_ref().expect("config should have been read");
        assert_eq!(config.gate_bypass.as_deref(), Some("total"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn gate_bypass_false_is_carried_as_off() {
        let root = fresh_root("config-bypass-false");
        write(&root, ".bee/config.json", r#"{"gate_bypass": false}"#);

        let snap = read_snapshot(&root);
        assert!(snap.read_errors.is_empty(), "no read error expected: {:?}", snap.read_errors);
        let config = snap.config.as_ref().expect("config should have been read");
        assert!(config.gate_bypass.is_none(), "false must normalize to off: {:?}", config.gate_bypass);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_gate_bypass_key_at_all_reads_as_off() {
        let root = fresh_root("config-bypass-no-key");
        write(&root, ".bee/config.json", r#"{"some_other_key": true}"#);

        let snap = read_snapshot(&root);
        assert!(snap.read_errors.is_empty(), "no read error expected: {:?}", snap.read_errors);
        let config = snap.config.as_ref().expect("config should have been read");
        assert!(config.gate_bypass.is_none(), "a missing key must normalize to off: {:?}", config.gate_bypass);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_config_json_at_all_reads_as_off_and_pushes_no_read_error() {
        let root = fresh_root("config-absent");
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));
        // No .bee/config.json written at all.

        let snap = read_snapshot(&root);
        assert!(snap.config.is_none(), "no config file should mean no config on the snapshot");
        assert!(snap.read_errors.is_empty(), "an absent config.json must never be a read error: {:?}", snap.read_errors);
        assert!(
            snap.attention.is_empty(),
            "an absent config.json must read as off, no attention item: {:?}",
            snap.attention
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_config_json_pushes_exactly_one_read_error_rest_still_reads() {
        let root = fresh_root("config-malformed");
        write(&root, ".bee/config.json", "{ this is not valid json");
        write(&root, ".bee/cells/good.json", &cell_json("good", "open"));

        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert!(snap.config.is_none());
        assert_eq!(snap.buckets.waiting.len(), 1, "the well-formed cell must still parse");
        assert_eq!(snap.read_errors.len(), 1, "expected exactly one read error: {:?}", snap.read_errors);
        assert!(snap.read_errors.iter().any(|e| e.contains("config.json")));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unrecognised_gate_bypass_value_is_carried_through_not_coerced_to_off() {
        let root = fresh_root("config-bypass-unrecognised");
        write(&root, ".bee/config.json", r#"{"gate_bypass": true}"#);

        let snap = read_snapshot(&root);
        assert!(snap.read_errors.is_empty(), "no read error expected: {:?}", snap.read_errors);
        let config = snap.config.as_ref().expect("config should have been read");
        assert!(
            config.gate_bypass.is_some(),
            "an unrecognised value must be carried through, not coerced to off: {:?}",
            config.gate_bypass
        );
        assert_ne!(config.gate_bypass.as_deref(), Some("false"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn recorded_bypass_not_off_yields_exactly_one_item_naming_the_recorded_level() {
        let root = fresh_root("attention-bypass-total");
        write(&root, ".bee/config.json", r#"{"gate_bypass": "total"}"#);

        let snap = read_snapshot(&root);
        assert_eq!(snap.attention.len(), 1, "one rule fired, exactly one item: {:?}", snap.attention);

        let item = &snap.attention[0];
        assert!(
            item.title.contains("total") || item.detail.contains("total"),
            "the item should name the recorded level: {:?}",
            item
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn recorded_bypass_off_yields_no_item() {
        let root = fresh_root("attention-bypass-off");
        write(&root, ".bee/config.json", r#"{"gate_bypass": false}"#);
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));

        let snap = read_snapshot(&root);
        assert!(
            snap.attention.is_empty(),
            "an off bypass must yield no attention item: {:?}",
            snap.attention
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn recorded_bypass_item_wording_marks_it_as_recorded_never_as_effective() {
        let root = fresh_root("attention-bypass-wording");
        write(&root, ".bee/config.json", r#"{"gate_bypass": "total"}"#);

        let snap = read_snapshot(&root);
        assert_eq!(snap.attention.len(), 1, "one rule fired, exactly one item: {:?}", snap.attention);

        let item = &snap.attention[0];
        let rendered = format!("{} {} {}", item.title, item.detail, item.suggested_action);
        assert!(
            rendered.contains("recorded"),
            "the wording must mark the value as the recorded setting: {rendered}"
        );
        // The wording is allowed to name "effective" only to disclaim it
        // (this reader's own wording says "not the effective one"); it must
        // never assert the recorded value as the effective level outright.
        assert!(
            !rendered.contains("is the effective level")
                && !rendered.contains("this is the effective"),
            "the wording must never assert the recorded value as the effective level: {rendered}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-10: the by-phase board — union of lanes and the active
    // feature, per-feature gates/created_at, per-feature cell counts (D8) ---

    #[test]
    fn lane_carries_its_own_approved_gates_and_created_at() {
        let root = fresh_root("phase-board-lane-gates");
        write(
            &root,
            ".bee/lanes/demo.json",
            r#"{"feature":"demo","phase":"swarming","mode":"standard","next_action":"go",
                "approved_gates":{"context":true,"shape":true,"execution":false,"review":false},
                "created_at":"2026-08-01T00:00:00.000Z"}"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.lanes.len(), 1);
        let lane = &snap.lanes[0];
        let gates = lane.approved_gates.as_ref().expect("a lane's own approved_gates should be Some");
        assert_eq!(gates.context, Some(true));
        assert_eq!(gates.shape, Some(true));
        assert_eq!(gates.execution, Some(false));
        assert_eq!(gates.review, Some(false));
        assert_eq!(lane.created_at.as_deref(), Some("2026-08-01T00:00:00.000Z"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn lane_with_no_gates_or_created_at_reads_as_none_not_fabricated() {
        let root = fresh_root("phase-board-lane-no-gates");
        write(&root, ".bee/lanes/demo.json", r#"{"feature":"demo","phase":"swarming","mode":"standard"}"#);

        let snap = read_snapshot(&root);
        let lane = &snap.lanes[0];
        assert!(lane.approved_gates.is_none(), "absent approved_gates must be None, never fabricated");
        assert!(lane.created_at.is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn phase_board_is_the_union_of_lanes_and_the_distinct_active_feature() {
        let root = fresh_root("phase-board-union");
        write(&root, ".bee/state.json", r#"{"phase":"exploring","feature":"active-feature","mode":"standard"}"#);
        write(&root, ".bee/lanes/alpha.json", r#"{"feature":"alpha","phase":"swarming","mode":"standard"}"#);
        write(
            &root,
            ".bee/lanes/beta.json",
            r#"{"feature":"beta","phase":"compounding-complete","mode":"small"}"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(
            snap.phase_board.len(),
            3,
            "the union of two lanes plus a distinct active feature is three: {:?}",
            snap.phase_board
        );
        let active_count = snap.phase_board.iter().filter(|p| p.feature == "active-feature").count();
        assert_eq!(active_count, 1, "the active feature must appear exactly once: {:?}", snap.phase_board);
        let active = snap.phase_board.iter().find(|p| p.feature == "active-feature").unwrap();
        assert_eq!(
            active.phase.as_deref(),
            Some("exploring"),
            "the active feature with no lane record must take its phase from state.json"
        );
        assert!(snap.phase_board.iter().any(|p| p.feature == "alpha" && p.phase.as_deref() == Some("swarming")));
        assert!(
            snap.phase_board
                .iter()
                .any(|p| p.feature == "beta" && p.phase.as_deref() == Some("compounding-complete"))
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn phase_board_active_feature_with_its_own_lane_appears_once_lane_wins() {
        let root = fresh_root("phase-board-dedup");
        write(
            &root,
            ".bee/state.json",
            r#"{"phase":"exploring","feature":"demo","mode":"standard",
                "approved_gates":{"context":true,"shape":false,"execution":false,"review":false}}"#,
        );
        write(
            &root,
            ".bee/lanes/demo.json",
            r#"{"feature":"demo","phase":"swarming","mode":"standard",
                "approved_gates":{"context":true,"shape":true,"execution":true,"review":false},
                "created_at":"2026-08-01T00:00:00.000Z"}"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(
            snap.phase_board.len(),
            1,
            "the active feature that also has a lane record must appear exactly once: {:?}",
            snap.phase_board
        );
        let entry = &snap.phase_board[0];
        assert_eq!(
            entry.phase.as_deref(),
            Some("swarming"),
            "the lane record must win for phase, not state.json's \"exploring\""
        );
        let gates = entry.approved_gates.as_ref().expect("gates should be Some");
        assert_eq!(gates.shape, Some(true), "the lane record must win for its own gates too");
        assert_eq!(entry.created_at.as_deref(), Some("2026-08-01T00:00:00.000Z"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn phase_board_with_no_lanes_directory_places_the_one_active_feature() {
        let root = fresh_root("phase-board-no-lanes-dir");
        write(&root, ".bee/state.json", r#"{"phase":"swarming","feature":"solo","mode":"standard"}"#);

        let snap = read_snapshot(&root);
        assert!(snap.lanes.is_empty(), "no .bee/lanes/ directory at all: {:?}", snap.lanes);
        assert_eq!(
            snap.phase_board.len(),
            1,
            "a store with no lanes at all must still place its one active feature: {:?}",
            snap.phase_board
        );
        assert_eq!(snap.phase_board[0].feature, "solo");
        assert_eq!(snap.phase_board[0].phase.as_deref(), Some("swarming"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn phase_board_lane_with_no_cells_reports_zero_counts_no_division_by_zero() {
        let root = fresh_root("phase-board-empty-cells");
        write(&root, ".bee/lanes/idle.json", r#"{"feature":"idle","phase":"swarming","mode":"standard"}"#);

        let snap = read_snapshot(&root);
        let entry = snap.phase_board.iter().find(|p| p.feature == "idle").unwrap();
        assert_eq!(entry.cell_counts.total, 0);
        assert_eq!(entry.cell_counts.done, 0);
        assert!(
            entry.cell_counts.done_fraction.is_none(),
            "no cells means no measurement, never a guessed 0.0: {:?}",
            entry.cell_counts
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn phase_board_feature_with_only_dropped_cells_reports_no_completion_no_division_artifact() {
        let root = fresh_root("phase-board-all-dropped");
        write(&root, ".bee/lanes/gone.json", r#"{"feature":"gone","phase":"swarming","mode":"standard"}"#);
        write(&root, ".bee/cells/c1.json", &feature_cell_json("c1", "gone", "dropped", None, None));
        write(&root, ".bee/cells/c2.json", &feature_cell_json("c2", "gone", "dropped", None, None));

        let snap = read_snapshot(&root);
        let entry = snap.phase_board.iter().find(|p| p.feature == "gone").unwrap();
        assert_eq!(entry.cell_counts.total, 0, "all-dropped cells count toward no total (D8): {:?}", entry.cell_counts);
        assert_eq!(entry.cell_counts.done, 0, "all-dropped must not read as completed work: {:?}", entry.cell_counts);
        assert!(
            entry.cell_counts.done_fraction.is_none(),
            "an all-dropped feature must not read as complete, and must not divide by zero (D8): {:?}",
            entry.cell_counts
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn phase_board_counts_mixed_statuses_correctly_excluding_dropped() {
        let root = fresh_root("phase-board-mixed");
        write(&root, ".bee/lanes/mixed.json", r#"{"feature":"mixed","phase":"swarming","mode":"standard"}"#);
        write(&root, ".bee/cells/c1.json", &feature_cell_json("c1", "mixed", "claimed", None, None));
        write(&root, ".bee/cells/c2.json", &feature_cell_json("c2", "mixed", "open", None, None));
        write(&root, ".bee/cells/c3.json", &feature_cell_json("c3", "mixed", "blocked", None, None));
        write(
            &root,
            ".bee/cells/c4.json",
            &feature_cell_json("c4", "mixed", "capped", Some("2026-08-01T00:00:00.000Z"), Some("2026-08-01T01:00:00.000Z")),
        );
        write(&root, ".bee/cells/c5.json", &feature_cell_json("c5", "mixed", "dropped", None, None));

        let snap = read_snapshot(&root);
        let entry = snap.phase_board.iter().find(|p| p.feature == "mixed").unwrap();
        assert_eq!(entry.cell_counts.doing, 1);
        assert_eq!(entry.cell_counts.waiting, 1);
        assert_eq!(entry.cell_counts.stuck, 1);
        assert_eq!(entry.cell_counts.done, 1);
        assert_eq!(entry.cell_counts.total, 4, "the dropped cell must not count toward the total (D8)");
        let frac = entry.cell_counts.done_fraction.expect("total > 0 should yield Some fraction");
        assert!((frac - 0.25).abs() < 1e-9, "done_fraction: {frac}");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn phase_board_malformed_lane_pushes_one_read_error_other_lanes_still_read() {
        let root = fresh_root("phase-board-malformed-lane");
        write(&root, ".bee/lanes/bad.json", "{ this is not valid json");
        write(&root, ".bee/lanes/good.json", r#"{"feature":"good","phase":"swarming","mode":"standard"}"#);

        let snap = read_snapshot(&root);
        assert_eq!(
            snap.lanes.len(),
            1,
            "the malformed lane must not stop the good one from reading: {:?}",
            snap.lanes
        );
        assert_eq!(snap.lanes[0].feature, "good");
        assert_eq!(snap.read_errors.len(), 1, "expected exactly one read error: {:?}", snap.read_errors);
        assert!(snap.read_errors.iter().any(|e| e.contains("bad.json")));
        let features: Vec<&str> = snap.phase_board.iter().map(|p| p.feature.as_str()).collect();
        assert_eq!(
            features,
            vec!["good"],
            "the phase board must reflect only the successfully-read lane: {:?}",
            features
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn phase_board_lane_next_action_absolute_path_does_not_survive_onto_snapshot() {
        let root = fresh_root("phase-board-scrub");
        let root_str = root.to_string_lossy().into_owned();
        let secret = root.join("secret.txt").to_string_lossy().into_owned();
        let next_action = format!("Check {secret} for the next step.").replace('\\', "\\\\");
        write(
            &root,
            ".bee/lanes/leaky.json",
            &format!(r#"{{"feature":"leaky","phase":"swarming","mode":"standard","next_action":"{next_action}"}}"#),
        );

        let snap = read_snapshot(&root);
        let entry = snap.phase_board.iter().find(|p| p.feature == "leaky").unwrap();
        let rendered_next_action = entry.next_action.as_deref().unwrap_or("");
        assert!(
            !rendered_next_action.contains(&root_str),
            "absolute path leaked into the phase board's next_action: {rendered_next_action}"
        );
        assert!(rendered_next_action.contains("next step"), "surrounding words must survive: {rendered_next_action}");

        let serialized = serde_json::to_string(&snap).unwrap();
        assert!(!serialized.contains(&root_str), "absolute path leaked into the snapshot: {serialized}");

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-12: a feature name is validated before it is ever joined onto
    // a filesystem path (D4, D9). Every rejection case below is asserted
    // through `promote_proposals_path` returning `None` directly — no path
    // built at all — rather than through any rendered output, per the
    // guard this cell exists to prove: a test on rendered output alone
    // would pass even if the lookup happened and only its result was
    // hidden. ---

    #[test]
    fn validate_feature_name_rejects_dot_dot_traversal_shapes() {
        let root = fresh_root("promote-guard-traversal");
        assert!(promote_proposals_path(&root, "../../etc").is_none(), "a `../../etc` feature must build no path");
        assert!(promote_proposals_path(&root, "..").is_none(), "a bare `..` feature must build no path");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn validate_feature_name_rejects_a_forward_slash() {
        let root = fresh_root("promote-guard-fslash");
        assert!(promote_proposals_path(&root, "foo/bar").is_none(), "a feature containing `/` must build no path");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn validate_feature_name_rejects_a_backslash() {
        let root = fresh_root("promote-guard-bslash");
        assert!(promote_proposals_path(&root, "foo\\bar").is_none(), "a feature containing `\\` must build no path");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn validate_feature_name_rejects_an_absolute_posix_path() {
        let root = fresh_root("promote-guard-posix-abs");
        assert!(promote_proposals_path(&root, "/etc/passwd").is_none(), "an absolute POSIX-shaped feature must build no path");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn validate_feature_name_rejects_a_windows_drive_prefixed_path() {
        let root = fresh_root("promote-guard-win-abs");
        assert!(
            promote_proposals_path(&root, "C:\\Users\\someone\\.ssh").is_none(),
            "a Windows drive-prefixed feature (backslash form) must build no path"
        );
        assert!(
            promote_proposals_path(&root, "C:/Users/someone/.ssh").is_none(),
            "a Windows drive-prefixed feature (forward-slash form) must build no path"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn validate_feature_name_rejects_a_leading_dot_name() {
        let root = fresh_root("promote-guard-dotfile");
        assert!(promote_proposals_path(&root, ".hidden").is_none(), "a dotfile-shaped feature must build no path");
        assert!(promote_proposals_path(&root, ".").is_none(), "a bare `.` feature must build no path");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn validate_feature_name_rejects_an_empty_string() {
        let root = fresh_root("promote-guard-empty");
        assert!(promote_proposals_path(&root, "").is_none(), "an empty feature name must build no path");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn validate_feature_name_rejects_a_control_character() {
        let root = fresh_root("promote-guard-control");
        let embedded_nul = "feat\u{0}ure";
        assert!(promote_proposals_path(&root, embedded_nul).is_none(), "an embedded NUL must build no path");
        let bare_control = "\u{7}"; // BEL — a control character with no separator or dot shape of its own
        assert!(promote_proposals_path(&root, bare_control).is_none(), "a bare control character must build no path");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn validate_feature_name_accepts_an_ordinary_slug() {
        let root = fresh_root("promote-guard-ok-slug");
        let path = promote_proposals_path(&root, "demo").expect("an ordinary slug must build a path");
        assert_eq!(path, root.join("docs").join("history").join("demo").join("promote-proposals.md"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn validate_feature_name_accepts_a_slug_with_hyphens_and_digits() {
        let root = fresh_root("promote-guard-ok-slug2");
        let path =
            promote_proposals_path(&root, "bee-board-pm-12").expect("a hyphenated, digited slug must build a path");
        assert_eq!(
            path,
            root.join("docs").join("history").join("bee-board-pm-12").join("promote-proposals.md")
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn has_promote_proposals_reports_presence_and_absence_never_a_rejected_lookup() {
        let root = fresh_root("promote-presence");
        write(&root, "docs/history/demo/promote-proposals.md", "a proposal body that must never be read");
        assert_eq!(has_promote_proposals(&root, "demo"), Some(true));
        assert_eq!(has_promote_proposals(&root, "no-such-feature"), Some(false));
        assert_eq!(
            has_promote_proposals(&root, "../../etc"),
            None,
            "a rejected name must never be looked up, not even reported as absent"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn promote_proposals_present_and_absent_reported_correctly_end_to_end() {
        let root = fresh_root("promote-e2e");
        write(&root, "docs/history/has-proposals/promote-proposals.md", "pending proposal");
        write(&root, ".bee/state.json", r#"{"feature":"has-proposals","phase":"swarming"}"#);
        write(
            &root,
            ".bee/cells/a.json",
            &feature_cell_json("a", "no-proposals", "capped", Some("2026-01-01T00:00:00Z"), Some("2026-01-02T00:00:00Z")),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.promote_proposals.get("has-proposals"), Some(&true), "{:?}", snap.promote_proposals);
        assert_eq!(snap.promote_proposals.get("no-proposals"), Some(&false), "{:?}", snap.promote_proposals);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn traversal_shaped_cell_feature_builds_no_path_outside_the_project() {
        let root = fresh_root("promote-security-cell");
        write(&root, ".bee/cells/a.json", &feature_cell_json("a", "../../etc", "open", None, None));

        let snap = read_snapshot(&root);
        assert!(
            !snap.promote_proposals.contains_key("../../etc"),
            "a traversal-shaped cell feature must build and check no path — the map's key set IS the built path set: {:?}",
            snap.promote_proposals
        );
        assert!(
            snap.read_errors.is_empty(),
            "a strange feature name is not a store error: {:?}",
            snap.read_errors
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn traversal_shaped_state_feature_builds_no_path_outside_the_project() {
        let root = fresh_root("promote-security-state");
        write(&root, ".bee/state.json", r#"{"feature":"../../etc","phase":"swarming"}"#);

        let snap = read_snapshot(&root);
        assert!(
            !snap.promote_proposals.contains_key("../../etc"),
            "a traversal-shaped state.json feature must build and check no path — the map's key set IS the built path set: {:?}",
            snap.promote_proposals
        );
        assert!(
            snap.read_errors.is_empty(),
            "a strange feature name is not a store error: {:?}",
            snap.read_errors
        );
        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-13: review join, capture queue, scribing debt, and the three
    // attention rules they feed (D4, D6, D7) ---

    fn candidate_json(id: &str, feature: &str, mode: &str, cells: &[&str]) -> String {
        let cells_json = cells.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(",");
        format!(
            r#"{{"id":"{id}","type":"candidate","date":"2026-08-01T00:00:00.000Z","feature":"{feature}","head":"abc123","mode":"{mode}","baseline":null,"cells":[{cells_json}]}}"#
        )
    }

    /// A `.bee/reviews/<id>.json` session fixture, in the real on-disk
    /// shape: `included[]` is an array of `{"type": "cell", "id": ...}`
    /// objects, never bare strings. `decision_status: None` omits the
    /// `decision` key entirely, matching a session with no decision at all.
    fn review_session_json(id: &str, included_cells: &[&str], p1_count: usize, decision_status: Option<&str>) -> String {
        let included_json =
            included_cells.iter().map(|c| format!(r#"{{"type":"cell","id":"{c}"}}"#)).collect::<Vec<_>>().join(",");
        let findings_json: String =
            (0..p1_count).map(|i| format!(r#"{{"id":"f{i}","severity":"P1","title":"x"}}"#)).collect::<Vec<_>>().join(",");
        let decision_json = decision_status.map(|s| format!(r#","decision":{{"status":"{s}"}}"#)).unwrap_or_default();
        format!(r#"{{"id":"{id}","included":[{included_json}],"findings":[{findings_json}]{decision_json}}}"#)
    }

    /// Like `cell_json`, but `behavior_change: true` — the shape
    /// `compute_scribing_debt` looks for.
    fn behavior_change_cell_json(id: &str, feature: &str, status: &str) -> String {
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
                "behavior_change": true,
                "change_class": "behavior",
                "pbi": null,
                "status": "{status}",
                "tier": "generation",
                "trace": {{"worker": "w1"}}
            }}"#
        )
    }

    #[test]
    fn candidate_in_no_session_is_unreviewed() {
        let root = fresh_root("review-unreviewed");
        write(&root, ".bee/review-candidates.jsonl", &candidate_json("c1", "demo", "standard", &["cell-1"]));

        let snap = read_snapshot(&root);
        assert_eq!(snap.review.candidates.len(), 1);
        assert_eq!(snap.review.candidates[0].status, BeeReviewStatus::Unreviewed);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn candidate_whose_cell_is_in_a_pending_session_is_in_review() {
        let root = fresh_root("review-pending");
        write(&root, ".bee/review-candidates.jsonl", &candidate_json("c1", "demo", "high-risk", &["cell-1"]));
        write(&root, ".bee/reviews/r1.json", &review_session_json("r1", &["cell-1"], 0, Some("pending")));

        let snap = read_snapshot(&root);
        assert_eq!(snap.review.candidates[0].status, BeeReviewStatus::InReview);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn candidate_whose_cell_is_in_an_approved_session_is_settled() {
        let root = fresh_root("review-approved");
        write(&root, ".bee/review-candidates.jsonl", &candidate_json("c1", "demo", "standard", &["cell-1"]));
        write(&root, ".bee/reviews/r1.json", &review_session_json("r1", &["cell-1"], 0, Some("approved")));

        let snap = read_snapshot(&root);
        assert_eq!(snap.review.candidates[0].status, BeeReviewStatus::Settled);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn candidate_whose_cell_is_in_a_blocked_session_is_settled() {
        let root = fresh_root("review-blocked");
        write(&root, ".bee/review-candidates.jsonl", &candidate_json("c1", "demo", "standard", &["cell-1"]));
        write(&root, ".bee/reviews/r1.json", &review_session_json("r1", &["cell-1"], 0, Some("blocked")));

        let snap = read_snapshot(&root);
        assert_eq!(
            snap.review.candidates[0].status,
            BeeReviewStatus::Settled,
            "blocked is settled too, not merely approved"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn session_naming_a_nonexistent_cell_does_not_crash_and_other_cells_still_join() {
        let root = fresh_root("review-ghost-cell");
        write(
            &root,
            ".bee/review-candidates.jsonl",
            &format!(
                "{}\n{}",
                candidate_json("c1", "demo", "standard", &["cell-real"]),
                candidate_json("c2", "demo", "standard", &["cell-other"]),
            ),
        );
        write(
            &root,
            ".bee/reviews/r1.json",
            &review_session_json("r1", &["cell-ghost", "cell-real"], 0, Some("approved")),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.review.candidates.len(), 2);
        let c1 = snap.review.candidates.iter().find(|c| c.id == "c1").unwrap();
        let c2 = snap.review.candidates.iter().find(|c| c.id == "c2").unwrap();
        assert_eq!(c1.status, BeeReviewStatus::Settled, "the real cell must still join and settle");
        assert_eq!(c2.status, BeeReviewStatus::Unreviewed, "an unrelated candidate stays unaffected");
        assert!(snap.read_errors.is_empty(), "a ghost cell id in included[] is not a read error: {:?}", snap.read_errors);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn candidate_naming_zero_cells_is_always_unreviewed_pinned() {
        // A candidate naming zero cells is the shape live in this repo's
        // own store (one candidate, no cells, a null baseline). It can
        // never appear in any session's included[], so it is pinned here
        // to always read as Unreviewed — a choice, not an accident of the
        // join, even when a session exists that would otherwise settle it.
        let root = fresh_root("review-zero-cells");
        write(&root, ".bee/review-candidates.jsonl", &candidate_json("c1", "demo", "standard", &[]));
        write(&root, ".bee/reviews/r1.json", &review_session_json("r1", &["some-other-cell"], 0, Some("approved")));

        let snap = read_snapshot(&root);
        assert_eq!(snap.review.candidates[0].status, BeeReviewStatus::Unreviewed);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn finding_with_no_severity_or_info_severity_never_counted_as_p1() {
        let root = fresh_root("review-severity-honest");
        write(
            &root,
            ".bee/reviews/r1.json",
            r#"{"id":"r1","included":[],"findings":[{"id":"f1","title":"no severity key"},{"id":"f2","severity":"info","title":"informational"}],"decision":{"status":"pending"}}"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.review.open_p1_findings, 0, "neither a missing severity nor \"info\" counts as P1");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn session_with_no_decision_at_all_counts_as_not_settled() {
        let root = fresh_root("review-no-decision");
        write(&root, ".bee/review-candidates.jsonl", &candidate_json("c1", "demo", "standard", &["cell-1"]));
        write(
            &root,
            ".bee/reviews/r1.json",
            r#"{"id":"r1","included":[{"type":"cell","id":"cell-1"}],"findings":[{"id":"f1","severity":"P1"}]}"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(
            snap.review.candidates[0].status,
            BeeReviewStatus::InReview,
            "a session with no decision key at all is not settled"
        );
        assert_eq!(snap.review.open_p1_findings, 1, "the P1 in a decision-less session is open");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_review_candidate_line_costs_one_read_error_not_the_good_rows() {
        let root = fresh_root("review-malformed-candidate");
        write(
            &root,
            ".bee/review-candidates.jsonl",
            &format!("{{ not valid json\n{}", candidate_json("good", "demo", "standard", &["cell-1"])),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.review.candidates.len(), 1, "the good row must still parse: {:?}", snap.review.candidates);
        assert_eq!(snap.review.candidates[0].id, "good");
        assert_eq!(snap.read_errors.len(), 1, "expected exactly one read error: {:?}", snap.read_errors);
        assert!(snap.read_errors.iter().any(|e| e.contains("review-candidates.jsonl")));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_review_session_pushes_one_read_error_other_sessions_still_read() {
        let root = fresh_root("review-malformed-session");
        write(&root, ".bee/reviews/bad.json", "{ this is not valid json");
        write(&root, ".bee/reviews/good.json", &review_session_json("good", &["cell-1"], 1, Some("pending")));

        let snap = read_snapshot(&root);
        assert_eq!(snap.review.open_p1_findings, 1, "the good session must still be read");
        assert_eq!(snap.read_errors.len(), 1, "expected exactly one read error: {:?}", snap.read_errors);
        assert!(snap.read_errors.iter().any(|e| e.contains("bad.json")));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn capture_queue_stubs_and_flushes_counted_correctly() {
        let root = fresh_root("capture-queue-net");
        write(
            &root,
            ".bee/capture-queue.jsonl",
            concat!(
                r#"{"kind":"stub","id":"s1","at":"2026-08-01T00:00:00.000Z","outcome":"x"}"#,
                "\n",
                r#"{"kind":"stub","id":"s2","at":"2026-08-01T00:00:00.000Z","outcome":"y"}"#,
                "\n",
                r#"{"kind":"flush","id":"s1","at":"2026-08-01T01:00:00.000Z","into":"docs/x.md"}"#,
            ),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.capture_queue.waiting, 1, "s1 was flushed, s2 is still waiting");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn capture_queue_absent_file_is_silent_no_read_error() {
        let root = fresh_root("capture-queue-absent");
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));

        let snap = read_snapshot(&root);
        assert_eq!(snap.capture_queue.waiting, 0);
        assert!(
            snap.read_errors.is_empty(),
            "an absent capture-queue.jsonl must never be a read error: {:?}",
            snap.read_errors
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn capture_queue_malformed_line_costs_one_read_error_not_the_good_rows() {
        let root = fresh_root("capture-queue-malformed");
        write(
            &root,
            ".bee/capture-queue.jsonl",
            concat!(
                "{ not valid json\n",
                r#"{"kind":"stub","id":"s1","at":"2026-08-01T00:00:00.000Z","outcome":"x"}"#,
            ),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.capture_queue.waiting, 1, "the good stub row must still count: {:?}", snap.capture_queue);
        assert_eq!(snap.read_errors.len(), 1, "expected exactly one read error: {:?}", snap.read_errors);
        assert!(snap.read_errors.iter().any(|e| e.contains("capture-queue.jsonl")));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn feature_with_capped_behavior_change_cells_and_no_matching_last_scribing_run_has_debt() {
        let root = fresh_root("scribing-debt-active");
        write(&root, ".bee/state.json", r#"{"feature":"demo","phase":"swarming"}"#);
        write(&root, ".bee/cells/c1.json", &behavior_change_cell_json("c1", "demo", "capped"));

        let snap = read_snapshot(&root);
        assert_eq!(snap.scribing_debt, vec!["demo".to_string()]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn feature_whose_last_scribing_run_names_it_has_no_debt() {
        let root = fresh_root("scribing-debt-clean");
        write(
            &root,
            ".bee/state.json",
            r#"{"feature":"demo","phase":"swarming","last_scribing_run":{"feature":"demo","date":"2026-08-01","at":"2026-08-01T00:00:00.000Z"}}"#,
        );
        write(&root, ".bee/cells/c1.json", &behavior_change_cell_json("c1", "demo", "capped"));

        let snap = read_snapshot(&root);
        assert!(
            snap.scribing_debt.is_empty(),
            "a matching last_scribing_run must clear the debt: {:?}",
            snap.scribing_debt
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn lane_feature_whose_last_scribing_run_names_a_different_feature_still_has_debt() {
        let root = fresh_root("scribing-debt-lane-mismatch");
        write(
            &root,
            ".bee/lanes/demo.json",
            r#"{"feature":"demo","phase":"swarming","last_scribing_run":{"feature":"some-other-feature","date":"2026-08-01","at":"2026-08-01T00:00:00.000Z"}}"#,
        );
        write(&root, ".bee/cells/c1.json", &behavior_change_cell_json("c1", "demo", "capped"));

        let snap = read_snapshot(&root);
        assert_eq!(
            snap.scribing_debt,
            vec!["demo".to_string()],
            "a last_scribing_run naming a DIFFERENT feature still counts as debt: {:?}",
            snap.scribing_debt
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn feature_with_no_behavior_change_cells_never_has_debt() {
        let root = fresh_root("scribing-debt-none");
        write(&root, ".bee/state.json", r#"{"feature":"demo","phase":"swarming"}"#);
        write(&root, ".bee/cells/c1.json", &cell_json("c1", "capped"));

        let snap = read_snapshot(&root);
        assert!(
            snap.scribing_debt.is_empty(),
            "no behavior_change cells means no debt, capped or not: {:?}",
            snap.scribing_debt
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn open_p1_review_findings_alone_yields_one_critical_item() {
        let root = fresh_root("attention-open-p1");
        write(
            &root,
            ".bee/reviews/r1.json",
            r#"{"id":"r1","included":[],"findings":[{"id":"f1","severity":"P1","title":"x"}],"decision":{"status":"pending"}}"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.attention.len(), 1, "one rule fired, exactly one item: {:?}", snap.attention);
        let item = &snap.attention[0];
        assert_eq!(item.severity, BeeAttentionSeverity::Critical);
        assert!(item.title.contains('1') && item.title.to_lowercase().contains("p1"), "title: {}", item.title);
        assert!(
            item.suggested_action.to_lowercase().contains("user-invoked"),
            "the action must read as user-invoked work, never automatic pending review: {}",
            item.suggested_action
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unreviewed_high_risk_candidate_alone_yields_one_serious_item() {
        let root = fresh_root("attention-unreviewed-high-risk");
        write(&root, ".bee/review-candidates.jsonl", &candidate_json("c1", "demo", "high-risk", &["cell-1"]));

        let snap = read_snapshot(&root);
        assert_eq!(snap.attention.len(), 1, "one rule fired, exactly one item: {:?}", snap.attention);
        let item = &snap.attention[0];
        assert_eq!(item.severity, BeeAttentionSeverity::Serious);
        assert!(item.title.to_lowercase().contains("high-risk"), "title: {}", item.title);
        assert!(
            item.suggested_action.to_lowercase().contains("user-invoked"),
            "must be worded as user-invoked, never automatic: {}",
            item.suggested_action
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unreviewed_standard_mode_candidate_does_not_fire_the_high_risk_rule() {
        let root = fresh_root("attention-unreviewed-standard");
        write(&root, ".bee/review-candidates.jsonl", &candidate_json("c1", "demo", "standard", &["cell-1"]));

        let snap = read_snapshot(&root);
        assert!(
            snap.attention.is_empty(),
            "a standard-mode unreviewed candidate must not fire the high-risk rule: {:?}",
            snap.attention
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn knowledge_debt_alone_yields_one_warning_item() {
        let root = fresh_root("attention-knowledge-debt");
        write(
            &root,
            ".bee/capture-queue.jsonl",
            r#"{"kind":"stub","id":"s1","at":"2026-08-01T00:00:00.000Z","outcome":"x"}"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.attention.len(), 1, "one rule fired, exactly one item: {:?}", snap.attention);
        let item = &snap.attention[0];
        assert_eq!(item.severity, BeeAttentionSeverity::Warning);
        assert!(item.title.contains("knowledge-debt"), "title: {}", item.title);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn all_three_bbp13_rules_together_ordered_heaviest_first() {
        let root = fresh_root("attention-all-three-bbp13");
        write(
            &root,
            ".bee/reviews/r1.json",
            r#"{"id":"r1","included":[],"findings":[{"id":"f1","severity":"P1","title":"x"}],"decision":{"status":"pending"}}"#,
        );
        write(&root, ".bee/review-candidates.jsonl", &candidate_json("c1", "demo", "high-risk", &["cell-1"]));
        write(
            &root,
            ".bee/capture-queue.jsonl",
            r#"{"kind":"stub","id":"s1","at":"2026-08-01T00:00:00.000Z","outcome":"x"}"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.attention.len(), 3, "all three rules should fire: {:?}", snap.attention);
        assert_eq!(snap.attention[0].severity, BeeAttentionSeverity::Critical);
        assert_eq!(snap.attention[1].severity, BeeAttentionSeverity::Serious);
        assert_eq!(snap.attention[2].severity, BeeAttentionSeverity::Warning);
        assert!(snap.attention[0].title.to_lowercase().contains("p1"), "{:?}", snap.attention);
        assert!(snap.attention[1].title.to_lowercase().contains("high-risk"), "{:?}", snap.attention);
        assert!(snap.attention[2].title.contains("knowledge-debt"), "{:?}", snap.attention);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn none_of_the_bbp13_rules_fire_on_a_clean_store() {
        let root = fresh_root("attention-bbp13-clean");
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));

        let snap = read_snapshot(&root);
        assert!(snap.attention.is_empty(), "a clean store must yield no attention items: {:?}", snap.attention);

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-15: reservations (D4) ---

    #[test]
    fn no_reservations_json_at_all_is_silent_no_read_error() {
        let root = fresh_root("reservations-absent");
        // No .bee/reservations.json written — both live stores this reader
        // was verified against hold no such non-empty file either way.
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));

        let snap = read_snapshot(&root);
        assert!(snap.reservations.is_empty(), "no reservations file should mean no reservations on the snapshot");
        assert!(
            snap.read_errors.is_empty(),
            "an absent reservations.json must never be a read error: {:?}",
            snap.read_errors
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_reservations_array_reads_as_empty() {
        let root = fresh_root("reservations-empty");
        write(&root, ".bee/reservations.json", r#"{"reservations": []}"#);

        let snap = read_snapshot(&root);
        assert!(snap.reservations.is_empty());
        assert!(snap.read_errors.is_empty(), "an empty array is not a read error: {:?}", snap.read_errors);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn populated_reservations_array_carries_every_entry() {
        let root = fresh_root("reservations-populated");
        write(
            &root,
            ".bee/reservations.json",
            r#"{"reservations": [
                {"agent": "healthread", "cell": "bbp-15", "path": "crates/mdview-core/src/bee.rs", "kind": "lease", "session": "s1", "reserved_at": "2026-08-06T17:29:44.227Z", "released_at": null},
                {"agent": "otheragent", "cell": "bbp-14", "path": "crates/mdview/src/server.rs", "kind": "intent", "session": "s2", "reserved_at": "2026-08-06T10:00:00.000Z", "released_at": "2026-08-06T11:00:00.000Z"}
            ]}"#,
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.reservations.len(), 2, "both entries should be carried: {:?}", snap.reservations);
        let first = &snap.reservations[0];
        assert_eq!(first.agent.as_deref(), Some("healthread"));
        assert_eq!(first.cell.as_deref(), Some("bbp-15"));
        assert_eq!(first.path.as_deref(), Some("crates/mdview-core/src/bee.rs"));
        assert_eq!(first.kind.as_deref(), Some("lease"));
        assert_eq!(first.session.as_deref(), Some("s1"));
        assert_eq!(first.reserved_at.as_deref(), Some("2026-08-06T17:29:44.227Z"));
        assert!(first.released_at.is_none());
        let second = &snap.reservations[1];
        assert_eq!(second.agent.as_deref(), Some("otheragent"));
        assert_eq!(second.released_at.as_deref(), Some("2026-08-06T11:00:00.000Z"));
        assert!(snap.read_errors.is_empty());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_reservations_json_pushes_exactly_one_read_error_rest_still_reads() {
        let root = fresh_root("reservations-malformed");
        write(&root, ".bee/reservations.json", "{ this is not valid json");
        write(&root, ".bee/cells/good.json", &cell_json("good", "open"));

        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert!(snap.reservations.is_empty());
        assert_eq!(snap.buckets.waiting.len(), 1, "the well-formed cell must still parse");
        assert_eq!(snap.read_errors.len(), 1, "expected exactly one read error: {:?}", snap.read_errors);
        assert!(snap.read_errors.iter().any(|e| e.contains("reservations.json")));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reservations_unexpected_shape_reads_as_absent_not_an_error() {
        let root = fresh_root("reservations-unexpected-shape");
        // An object where an array belongs — valid JSON, wrong shape.
        write(&root, ".bee/reservations.json", r#"{"reservations": {"not": "an array"}}"#);
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));

        let snap = read_snapshot(&root);
        assert!(snap.reservations.is_empty(), "an unexpected shape should read as absent");
        assert!(
            snap.read_errors.is_empty(),
            "an unexpected shape must never be a read error: {:?}",
            snap.read_errors
        );
        assert_eq!(snap.buckets.waiting.len(), 1, "the rest of the snapshot still reads");

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-15: tier mix ---

    fn cell_json_with_tier(id: &str, status: &str, tier: Option<&str>) -> String {
        let tier_field = match tier {
            Some(t) => format!(r#""{t}""#),
            None => "null".to_string(),
        };
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
                "tier": {tier_field},
                "trace": {{"worker": "w1"}}
            }}"#
        )
    }

    #[test]
    fn tier_mix_across_tiers_reports_counts_and_expensive_share() {
        let root = fresh_root("tier-mix-across");
        write(&root, ".bee/cells/c1.json", &cell_json_with_tier("c1", "capped", Some("extraction")));
        write(&root, ".bee/cells/c2.json", &cell_json_with_tier("c2", "capped", Some("generation")));
        write(&root, ".bee/cells/c3.json", &cell_json_with_tier("c3", "capped", Some("generation")));
        write(&root, ".bee/cells/c4.json", &cell_json_with_tier("c4", "capped", Some("ceiling")));

        let snap = read_snapshot(&root);
        let mix = snap.tier_mix.as_ref().expect("four cells should yield a tier mix");
        assert_eq!(mix.counts.get("extraction").copied(), Some(1));
        assert_eq!(mix.counts.get("generation").copied(), Some(2));
        assert_eq!(mix.counts.get("ceiling").copied(), Some(1));
        assert_eq!(mix.untiered, 0);
        assert_eq!(
            mix.expensive_tier_share,
            Some(0.25),
            "1 of 4 tiered cells ran at the most expensive tier: {:?}",
            mix.expensive_tier_share
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn tier_mix_counts_cells_with_no_tier_as_untiered_never_dropped_never_guessed() {
        let root = fresh_root("tier-mix-untiered");
        write(&root, ".bee/cells/c1.json", &cell_json_with_tier("c1", "capped", Some("generation")));
        write(&root, ".bee/cells/c2.json", &cell_json_with_tier("c2", "capped", None));

        let snap = read_snapshot(&root);
        let mix = snap.tier_mix.as_ref().expect("two cells should yield a tier mix");
        assert_eq!(mix.untiered, 1, "the tier-less cell must be counted, not dropped: {mix:?}");
        assert_eq!(mix.counts.values().sum::<usize>(), 1, "the untiered cell must never be guessed into a bucket");
        assert_eq!(mix.expensive_tier_share, Some(0.0), "0 of 1 tiered cells ran at the most expensive tier");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn tier_mix_zero_tiered_cells_reports_no_share() {
        let root = fresh_root("tier-mix-zero-tiered");
        write(&root, ".bee/cells/c1.json", &cell_json_with_tier("c1", "capped", None));
        write(&root, ".bee/cells/c2.json", &cell_json_with_tier("c2", "capped", None));

        let snap = read_snapshot(&root);
        let mix = snap.tier_mix.as_ref().expect("two untiered cells should still yield a tier mix");
        assert_eq!(mix.untiered, 2);
        assert!(mix.counts.is_empty());
        assert!(
            mix.expensive_tier_share.is_none(),
            "zero tiered cells must report no share, never a zero or a division by zero: {:?}",
            mix.expensive_tier_share
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn tier_mix_empty_cell_store_reports_nothing() {
        let root = fresh_root("tier-mix-empty-store");
        write(&root, ".bee/state.json", r#"{"phase":"exploring"}"#);
        // No cells at all.

        let snap = read_snapshot(&root);
        assert!(
            snap.tier_mix.is_none(),
            "an empty cell store must report nothing rather than zeros: {:?}",
            snap.tier_mix
        );

        std::fs::remove_dir_all(&root).ok();
    }
}
