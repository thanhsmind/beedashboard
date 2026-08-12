//! `PaneScroller` — answers "give me this pane's older content" without the
//! caller needing to know which of the two mechanisms actually served it:
//!
//! - **Native scrollback** — herdr's own `recent` read source, free (no
//!   bytes sent) when it already holds more than `visible`.
//! - **Escape injection** — for an alt-screen pane that holds no native
//!   scrollback at all (VT100/xterm semantics: the alternate screen buffer
//!   never accumulates scrollback), replay the agent's own scroll
//!   keybinding via raw bytes through [`Herdr::send_text`] instead.
//!
//! Ported from herdr-go's `PaneScroller` (measured live against a real
//! herdr 0.7.3/0.7.4 + Claude Code pane, 2026-07-28: two idle alt-screen
//! panes with no native scrollback each revealed previously-unseen lines --
//! 17 and 14 -- in response to a raw PageUp).
//!
//! scroll-keep-position: this module now offers two entry points, not one --
//! [`PaneScroller::read_to_depth`] is the primary, *stateful* one, moving
//! only the delta between a caller-supplied `from_depth` and `to_depth`
//! (one `PAGE_UP`/`PAGE_DOWN` per page, never a full replay), so the
//! gateway's own per-pane depth record (`server.rs`'s `PaneScrollTracker`)
//! can make "one more page back" cost one hop instead of re-walking every
//! page from live each time. [`PaneScroller::read_history`] stays exactly
//! as it always was -- self-contained, always ends restored to live, no
//! memory of a previous call -- and is now used only as the gateway's
//! content-mismatch recovery path (`server.rs`'s `scroll_aware_read`): when
//! the caller's remembered depth can no longer be trusted, replaying the
//! whole journey from a known-good start is safer than trying to delta from
//! a position that might not be real anymore.

use std::time::Duration;

use super::{Herdr, ReadSource, Result, ScreenRead};

/// Raw PageUp — the VT escape sequence a live Claude Code pane was verified
/// to interpret as "scroll its own transcript up".
pub(crate) const PAGE_UP: &str = "\x1b[5~";
/// Raw PageDown — the mirror of [`PAGE_UP`], used by
/// [`PaneScroller::read_to_depth`] to move a page back *toward* live without
/// going all the way (a full [`RESTORE_BOTTOM`] would overshoot a caller
/// asking for depth 1 when the pane is currently at depth 3). Not
/// independently live-verified the way `PAGE_UP`/`RESTORE_BOTTOM` were
/// (2026-07-28's live pane testing only exercised scrolling up and
/// restoring to bottom) -- accepted on the same VT100/xterm pager-keybinding
/// symmetry `PAGE_UP` itself relies on, same low-cost-if-wrong reasoning as
/// `read_history_with_strategy`'s own harmless-no-op acceptance for a short
/// primary-screen pane.
pub(crate) const PAGE_DOWN: &str = "\x1b[6~";
/// Raw Ctrl+End — verified to restore the live bottom view afterward.
pub(crate) const RESTORE_BOTTOM: &str = "\x1b[1;5F";

/// Herdr's own hard cap on a `recent` read.
const RECENT_LINES_CAP: usize = 1000;

/// How many re-reads of `Visible` to try, waiting for two consecutive ones
/// to come back identical, before giving up and using whatever the last
/// read returned (see [`PaneScroller::wait_until_progressed_and_stable`]).
/// `send_text` only confirms bytes reached the pty, not that the agent's TUI
/// has finished redrawing in response -- live testing across multiple real
/// Claude Code panes (2026-07-28) found this settle time varies a lot
/// session to session (some landed within 40ms, others needed several
/// hundred), so a single guessed fixed delay was never reliable; waiting for
/// stability (not a specific expected value) is. A pane that never responds
/// at all (the harmless no-op case) or whose content keeps genuinely
/// changing simply exhausts every attempt and falls back to its last read.
const STABILITY_READ_ATTEMPTS: usize = 10;
/// Delay between stability re-reads (see `STABILITY_READ_ATTEMPTS`).
const STABILITY_READ_INTERVAL: Duration = Duration::from_millis(50);

/// Which mechanism actually served a `read_history` result -- not part of
/// `read_history`'s public return (a caller only ever needs the bare
/// [`ScreenRead`]), exposed only via `read_history_with_strategy` so this
/// module's own tests can assert which path fired without inferring it from
/// timing or `send_text` call logs alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollStrategy {
    NativeScrollback,
    EscapeInjection,
}

/// Answers "give me this pane's older content" over a [`Herdr`] -- the
/// try-native-scrollback-then-escalate logic runs internally, so a caller
/// never branches on strategy itself.
pub struct PaneScroller<'a> {
    herdr: &'a dyn Herdr,
}

impl<'a> PaneScroller<'a> {
    pub fn new(herdr: &'a dyn Herdr) -> Self {
        PaneScroller { herdr }
    }

    /// Read `pane_id`'s older content, `pages` PageUp-hops back from the live
    /// bottom (clamped to at least 1). Every call is a fresh, self-contained
    /// round trip that always ends restored to live, never a continuation of
    /// a previous call's state -- there is no durable per-pane scroll state
    /// kept *by this type* between requests (the gateway keeps its own now,
    /// see below). The whole `pages`-hop journey happens inside this one
    /// call before restoring. Always returns the existing [`ScreenRead`]
    /// wire type (`text` + `revision`, never a bare `Vec<String>`/`String`),
    /// and always carries the read's own `revision` through unchanged, never
    /// fabricated.
    ///
    /// scroll-keep-position: no longer the gateway's default path for a
    /// `?history=` request -- [`PaneScroller::read_to_depth`] is, moving
    /// only the delta from the caller's own remembered depth. This method
    /// remains exactly as self-contained as it always was, which is now
    /// exactly what makes it the right *recovery* path: when the gateway's
    /// remembered depth can no longer be trusted (`server.rs`'s
    /// content-mismatch rail), replaying the full journey from scratch and
    /// always landing back at live is safer than deltaing from a position
    /// that might not be real.
    pub async fn read_history(&self, pane_id: &str, pages: usize) -> Result<ScreenRead> {
        self.read_history_with_strategy(pane_id, pages)
            .await
            .map(|(read, _strategy)| read)
    }

    /// Same as [`PaneScroller::read_history`], but also names which strategy
    /// served the result -- used by this module's own tests to assert
    /// `NativeScrollback` vs. `EscapeInjection` directly instead of inferring
    /// it from `send_text` call logs alone.
    async fn read_history_with_strategy(
        &self,
        pane_id: &str,
        pages: usize,
    ) -> Result<(ScreenRead, ScrollStrategy)> {
        let pages = pages.max(1);
        let visible = self
            .herdr
            .read_pane(pane_id, ReadSource::Visible, 0)
            .await?;
        let recent = self
            .herdr
            .read_pane(pane_id, ReadSource::Recent, RECENT_LINES_CAP)
            .await?;

        // WHY, not WHAT: alt-screen panes run in the terminal's alternate
        // screen buffer, which by VT100/xterm semantics never accumulates
        // scrollback at all -- so herdr's own `recent` read comes back no
        // richer than `visible` for them, not merely smaller. Selection
        // therefore compares the two ACTUAL reads (richer or not), never a
        // separately-fetched scroll-offset field (the pane.read response
        // carries no scroll field at all, and it would read the same
        // 0/absent for both a permanently-alt-screen pane AND a genuinely
        // empty fresh primary-screen pane -- indistinguishable by that field
        // alone). Also never the pane's `agent` name: a plain shell pane
        // carries none, an unknown/future agent would be silently
        // mishandled by a hardcoded list, and an agent's own alt-screen
        // usage can change release to release while its name stays the
        // same. "Not richer than" is deliberately ONE case covering exact
        // equality and any other non-richer outcome identically -- there is
        // no partial-credit branch, and no at-capacity/viewport-dimension
        // gate before escalating: the gateway's wire types carry no viewport
        // data to gate on, and the risk it would guard against -- a short
        // primary-screen pane escalating once, harmlessly -- is accepted
        // below, not solved by inventing missing data.
        if is_richer(&recent, &visible) {
            return Ok((recent, ScrollStrategy::NativeScrollback));
        }

        // Escalate: replay the agent's own scroll keybinding via raw bytes
        // -- never send_keys/send_input, see Herdr::send_text's own doc for
        // why. For a genuinely short primary-screen pane this fires once,
        // harmlessly: an ASSUMED-but-not-independently-verified no-op (only
        // Claude Code/alt-screen was live-tested with these exact bytes)
        // accepted as a low-cost round trip, not a correctness bug.
        // Each hop's completion is judged against the PREVIOUS hop's own
        // content (starting from the pre-scroll `visible`), never a fixed
        // "original" baseline -- that's what lets `pages > 1` reveal
        // successively older content instead of re-landing on the same
        // page.
        // A hop's own completion is judged by simple difference from the
        // page before it, never by is_richer/length: live testing (2026-07-
        // 28) found a real Claude Code pane's successive PageUp hops behave
        // like a pager advancing by a fixed viewport, not a strictly-growing
        // buffer -- an older page can easily have FEWER visible characters
        // than the one before it (shorter lines, different wrapping) while
        // still being genuinely different, further-back content. Judging
        // hops by length silently discarded such pages as "not richer" and
        // gave up (multi-page scroll-back plateauing after 1-2 hops even
        // though PageUp kept doing something real every time). is_richer
        // itself is intentionally kept only for the NativeScrollback-vs-
        // Visible pre-check above: that comparison IS a genuine
        // superset/tail-match by construction, unlike hop-to-hop paging.
        let mut escalated = visible.clone();
        for _ in 0..pages {
            self.herdr.send_text(pane_id, PAGE_UP).await?;
            let before = escalated.clone();
            let progressed = self
                .wait_until_progressed_and_stable(pane_id, |r| r.text != before.text)
                .await?;
            if progressed.text == before.text {
                // PageUp genuinely did nothing further within the wait
                // budget (already at the agent's own scrollback limit, or a
                // pane that ignores the sequence entirely) -- keep the
                // previous content and stop; later hops would be no-ops too.
                break;
            }
            escalated = progressed;
        }
        // ALWAYS restore the live bottom afterward, regardless of what the
        // escalated read returned -- an operator's "load older" swipe must
        // never leave the pane scrolled away from its live tail. Waits for
        // the exit-transition to actually land for the same reason as the
        // escalation wait above: returning too early left the pane still
        // mid-transition when the very next real keystroke arrived (e.g. a
        // Reply-sheet Send tapped right after "load older") -- live-
        // reproduced as the agent swallowing that Enter as "dismiss scroll
        // view" instead of "submit", so typed text landed with no Enter.
        // "Progressed" here means differs from the just-scrolled `escalated`
        // view, not an exact match against the original pre-scroll
        // `visible` baseline: the agent's own footer (elapsed-time/token
        // counters) keeps ticking regardless, so an exact-match wait could
        // spin out its whole budget over nothing.
        self.herdr.send_text(pane_id, RESTORE_BOTTOM).await?;
        self.wait_until_progressed_and_stable(pane_id, |r| r.text != escalated.text)
            .await?;

        Ok((escalated, ScrollStrategy::EscapeInjection))
    }

    /// Re-reads `Visible` until a read satisfies `has_progressed` at all,
    /// then keeps re-reading until it stabilizes -- two consecutive reads
    /// come back identical, meaning the agent's TUI has stopped changing --
    /// or the shared attempt budget (`STABILITY_READ_ATTEMPTS`) runs out,
    /// whichever comes first. The two-phase shape matters: accepting mere
    /// stability alone would falsely treat "hasn't started transitioning
    /// yet" (the same stale value read twice in a row before the agent's
    /// redraw has even begun) as "transition finished". `send_text` only
    /// confirms bytes reached the pty, not that the agent has finished
    /// redrawing in response -- live testing across multiple real Claude
    /// Code panes (2026-07-28) found this settle time varies a lot session
    /// to session (some landed within tens of ms, others needed several
    /// hundred), so a single guessed fixed delay was never reliable.
    async fn wait_until_progressed_and_stable(
        &self,
        pane_id: &str,
        has_progressed: impl Fn(&ScreenRead) -> bool,
    ) -> Result<ScreenRead> {
        let mut candidate = self
            .herdr
            .read_pane(pane_id, ReadSource::Visible, 0)
            .await?;
        let mut attempts_left = STABILITY_READ_ATTEMPTS;
        while !has_progressed(&candidate) && attempts_left > 1 {
            tokio::time::sleep(STABILITY_READ_INTERVAL).await;
            candidate = self
                .herdr
                .read_pane(pane_id, ReadSource::Visible, 0)
                .await?;
            attempts_left -= 1;
        }
        if !has_progressed(&candidate) {
            return Ok(candidate); // never progressed within budget -- caller decides what to do
        }
        let mut last = candidate;
        while attempts_left > 1 {
            tokio::time::sleep(STABILITY_READ_INTERVAL).await;
            let next = self
                .herdr
                .read_pane(pane_id, ReadSource::Visible, 0)
                .await?;
            attempts_left -= 1;
            if next.text == last.text {
                return Ok(next);
            }
            last = next;
        }
        Ok(last)
    }

    /// scroll-keep-position: the stateful entry point -- moves `pane_id`
    /// from `from_depth` (the gateway's own remembered depth, `0` meaning
    /// live) to `to_depth`, sending only the difference: one [`PAGE_UP`] per
    /// extra page going deeper, one [`PAGE_DOWN`] per page coming back
    /// toward live, and [`RESTORE_BOTTOM`] only when `to_depth` is `0`. The
    /// caller (`server.rs`'s `scroll_aware_read`) owns `from_depth` --  this
    /// method trusts it completely and never re-derives it, which is why
    /// the gateway's own safety rails (idle-TTL, content-mismatch) run
    /// *before* calling this, not inside it.
    ///
    /// `from_depth == 0` is the one case that also gets the free
    /// native-scrollback shortcut (see `read_history_with_strategy`'s own
    /// WHY-comment for why that check only ever compares the two live
    /// reads): a fresh request restarting from live may still be answerable
    /// without escape injection at all. Once a pane is already escape-
    /// injection-scrolled (`from_depth > 0`), every further move is a pure
    /// delta -- the native-scrollback comparison is deliberately not
    /// repeated there, since it answers a different question (does herdr's
    /// own scrollback beat the live screen) than the one an already-
    /// escalated pane is asking (move N pages from here).
    ///
    /// Returns the strategy that served the request alongside the read, the
    /// same shape `read_history_with_strategy` returns, so a caller can
    /// decide whether the move is worth remembering: only an
    /// [`ScrollStrategy::EscapeInjection`] result changes the pane's own
    /// escape-injection position, so only that case is worth recording as
    /// the gateway's new depth.
    ///
    /// Review fix (D): also returns the depth ACTUALLY reached -- not
    /// necessarily `to_depth`. The hop loops below can break early (the
    /// agent's own scrollback ran out), and the caller
    /// (`server.rs`'s `scroll_aware_read`) records whatever this returns as
    /// its new per-pane depth, never the depth it merely asked for; claiming
    /// `to_depth` was reached when it was not is exactly the kind of silent
    /// depth drift an independent review found.
    pub async fn read_to_depth(
        &self,
        pane_id: &str,
        from_depth: usize,
        to_depth: usize,
    ) -> Result<(ScreenRead, ScrollStrategy, usize)> {
        if from_depth == to_depth {
            // Nothing to move -- just hand back what's on screen right now.
            // `to_depth == 0` here still reports `NativeScrollback` (no
            // escape-injection position to speak of), matching what a
            // brand-new pane's first depth-0 request would report.
            let read = self
                .herdr
                .read_pane(pane_id, ReadSource::Visible, 0)
                .await?;
            let strategy = if to_depth == 0 {
                ScrollStrategy::NativeScrollback
            } else {
                ScrollStrategy::EscapeInjection
            };
            return Ok((read, strategy, to_depth));
        }

        if to_depth == 0 {
            // Always ends at live regardless of how far back `from_depth`
            // was -- one Ctrl+End is exactly as cheap as one PageDown here,
            // and it is the one move this module already verified live
            // (2026-07-28) actually lands the agent's TUI back at its own
            // bottom, so it stays the depth-0 exit for every from_depth,
            // never a chain of PAGE_DOWNs that would only be trusted on the
            // same unverified symmetry PAGE_DOWN itself relies on.
            let before = self
                .herdr
                .read_pane(pane_id, ReadSource::Visible, 0)
                .await?;
            self.herdr.send_text(pane_id, RESTORE_BOTTOM).await?;
            let restored = self
                .wait_until_progressed_and_stable(pane_id, |r| r.text != before.text)
                .await?;
            return Ok((restored, ScrollStrategy::EscapeInjection, 0));
        }

        if from_depth == 0 {
            // Fresh request restarting from live: the free native-
            // scrollback shortcut applies here and only here (see this
            // method's own doc comment for why it is not repeated once
            // already escalated).
            let visible = self
                .herdr
                .read_pane(pane_id, ReadSource::Visible, 0)
                .await?;
            let recent = self
                .herdr
                .read_pane(pane_id, ReadSource::Recent, RECENT_LINES_CAP)
                .await?;
            if is_richer(&recent, &visible) {
                return Ok((recent, ScrollStrategy::NativeScrollback, 0));
            }
            let mut current = visible;
            let mut reached = 0;
            for _ in 0..to_depth {
                self.herdr.send_text(pane_id, PAGE_UP).await?;
                let before = current.clone();
                let progressed = self
                    .wait_until_progressed_and_stable(pane_id, |r| r.text != before.text)
                    .await?;
                if progressed.text == before.text {
                    break; // agent's own scrollback limit reached -- later hops would be no-ops too
                }
                current = progressed;
                reached += 1;
            }
            return Ok((current, ScrollStrategy::EscapeInjection, reached));
        }

        // Both from_depth and to_depth are non-zero and unequal: a pure
        // delta move, no restore either way -- the whole reason this method
        // exists instead of always replaying through `read_history`.
        let (bytes, hops) = if to_depth > from_depth {
            (PAGE_UP, to_depth - from_depth)
        } else {
            (PAGE_DOWN, from_depth - to_depth)
        };
        let mut current = self
            .herdr
            .read_pane(pane_id, ReadSource::Visible, 0)
            .await?;
        let mut reached = from_depth;
        for _ in 0..hops {
            self.herdr.send_text(pane_id, bytes).await?;
            let before = current.clone();
            let progressed = self
                .wait_until_progressed_and_stable(pane_id, |r| r.text != before.text)
                .await?;
            if progressed.text == before.text {
                break;
            }
            current = progressed;
            reached = if bytes == PAGE_UP {
                reached + 1
            } else {
                reached - 1
            };
        }
        Ok((current, ScrollStrategy::EscapeInjection, reached))
    }

    /// Review fix (B): always sends [`RESTORE_BOTTOM`], regardless of what
    /// the caller believes this pane's current escape-injection depth to
    /// be -- the one entry point an EXPLICIT depth-0 request
    /// (`server.rs`'s Live button / `?history=0`) must go through.
    /// [`PaneScroller::read_to_depth`]'s own `from_depth == to_depth`
    /// shortcut sends nothing at all when the caller (wrongly) assumes
    /// `from_depth == 0`, which is exactly what left a Live press a no-op
    /// after a daemon restart dropped the gateway's own per-pane record
    /// even though the real pane might still genuinely be scrolled from
    /// before the restart. Sending `RESTORE_BOTTOM` to a pane that is
    /// already at live is the same accepted harmless no-op every other
    /// unconditional restore in this module already relies on (see
    /// `read_history_with_strategy`'s own doc comment).
    ///
    /// Independent re-review fix (4): waits via [`Self::wait_until_settled`],
    /// not [`Self::wait_until_progressed_and_stable`] -- see that method's
    /// own doc comment for why a pane that is ALREADY live (the majority of
    /// both Live presses and idle-sweep restores) needs a different settle
    /// rule than a hop that must confirm real movement.
    pub async fn restore_to_live(&self, pane_id: &str) -> Result<ScreenRead> {
        self.herdr.send_text(pane_id, RESTORE_BOTTOM).await?;
        self.wait_until_settled(pane_id).await
    }

    /// Re-reads `Visible` until two consecutive reads land on the same
    /// value -- the pane has genuinely stopped changing -- or the shared
    /// attempt budget (`STABILITY_READ_ATTEMPTS`) runs out, whichever comes
    /// first. [`Self::restore_to_live`] is the one caller.
    ///
    /// Independent re-review fix (4): unlike
    /// [`Self::wait_until_progressed_and_stable`], this does NOT first
    /// require a difference from some pre-send baseline.
    /// [`Self::restore_to_live`] unconditionally sends [`RESTORE_BOTTOM`]
    /// even to a pane that is already live -- the accepted harmless no-op
    /// every unconditional restore in this module already relies on (see
    /// `read_history_with_strategy`'s own doc comment) -- and for that
    /// overwhelmingly common call shape (every idle-sweep restore, every
    /// Live press on a pane that never actually scrolled) a baseline-diff
    /// gate can NEVER fire, so it burned the full
    /// `STABILITY_READ_ATTEMPTS x STABILITY_READ_INTERVAL` budget (~450ms)
    /// every single time, waiting for a change that structurally cannot
    /// come. "Two reads in a row came back the same" is the only question
    /// `restore_to_live` actually needs answered -- has the pane stopped
    /// moving -- and it is trivially true on the very first pair of reads
    /// for an already-live pane, cutting that case down to one
    /// `STABILITY_READ_INTERVAL` (~50ms). A pane that IS genuinely
    /// transitioning still gets its real end state here: successive reads
    /// keep differing from each other while the transition is under way, so
    /// two-in-a-row still can't fire until the redraw has actually stopped
    /// -- the rule is symmetric, not merely "give up early".
    ///
    /// Traded off, not eliminated: a transition whose own async-redraw lag
    /// happens to echo the exact same stale frame across two reads in a row
    /// before it starts moving would settle one read early here -- the same
    /// low-cost-if-wrong acceptance this module already relies on elsewhere
    /// (see `read_history_with_strategy`'s own doc comment), traded for
    /// cutting the overwhelmingly common already-live case from ~450ms to
    /// ~50ms.
    async fn wait_until_settled(&self, pane_id: &str) -> Result<ScreenRead> {
        let mut last = self
            .herdr
            .read_pane(pane_id, ReadSource::Visible, 0)
            .await?;
        let mut attempts_left = STABILITY_READ_ATTEMPTS;
        while attempts_left > 1 {
            tokio::time::sleep(STABILITY_READ_INTERVAL).await;
            let next = self
                .herdr
                .read_pane(pane_id, ReadSource::Visible, 0)
                .await?;
            attempts_left -= 1;
            if next.text == last.text {
                return Ok(next);
            }
            last = next;
        }
        Ok(last)
    }
}

/// `recent` is "richer" than `visible` when it strictly contains more text --
/// a length comparison, not equality: live evidence showed a primary-screen
/// pane's `recent` read always tails-match `visible` exactly and adds
/// content before it, so a strict length win is exactly "has more to show".
/// Equal length (even with different bytes) is deliberately NOT richer --
/// see the WHY-comment in `read_history_with_strategy` for why "not richer"
/// must stay one case, not just exact equality. Compares `visible_len`, not
/// raw byte length: live testing against multiple real Claude Code panes
/// found its own footer/status bar (elapsed-time and token-percentage
/// counters) re-renders on every single read regardless of scroll position,
/// and ANSI color codes plus right-padding to the terminal's column width
/// both inflate raw length independent of actual content -- together enough
/// noise on some panes to make a strictly-longer escalation register as
/// "not richer" (multi-page scroll-back plateaued after the first hop on
/// one real pane even though PageUp was doing something every time).
fn is_richer(recent: &ScreenRead, visible: &ScreenRead) -> bool {
    visible_len(&recent.text) > visible_len(&visible.text)
}

/// Strips ANSI CSI escape sequences from `text`, leaving every other
/// character -- including newlines -- untouched. The shared first half of
/// both `visible_len` (a length-only heuristic) and `normalized_lines` (a
/// full line-by-line normalisation `server.rs`'s content-mismatch rail
/// uses, review fix A).
fn strip_ansi(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            stripped.push(c);
            continue;
        }
        // CSI sequence: ESC '[' ... final byte in 0x40..=0x7E. A bare ESC
        // (no following '[') is simply dropped -- there is nothing else in
        // practice that `format: ansi` reads emit.
        if chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&next) {
                    break;
                }
            }
        }
    }
    stripped
}

/// A read's length with ANSI CSI escape sequences stripped and each line's
/// trailing whitespace trimmed -- both are rendering noise (color codes,
/// padding to the terminal's column width) that a raw byte-length
/// comparison would otherwise count as "more content" (see `is_richer`).
fn visible_len(text: &str) -> usize {
    strip_ansi(text)
        .lines()
        .map(|line| line.trim_end().len())
        .sum()
}

/// `text` normalised the same way `visible_len` measures it -- ANSI CSI
/// sequences stripped, each line's trailing whitespace trimmed -- but
/// returned as the actual lines rather than a bare length, so a caller can
/// compare WHAT changed, not just how much. Used by
/// `normalized_content_diverges`.
pub(crate) fn normalized_lines(text: &str) -> Vec<String> {
    strip_ansi(text)
        .lines()
        .map(|line| line.trim_end().to_string())
        .collect()
}

/// Review fix (A): whether `current` and `recorded` -- both run through
/// `normalized_lines` first -- differ MATERIALLY, meaning anywhere other
/// than their own last line. `server.rs`'s content-mismatch rail
/// (`scroll_aware_read`) uses this, not a raw whole-text hash, to decide
/// whether a remembered scroll depth can still be trusted for a delta move.
///
/// WHY the last line is exempt rather than compared: a live Claude Code
/// pane's footer (elapsed-time / token-percentage counters -- see
/// `is_richer`'s own doc comment) re-renders on every single read,
/// including while the pane sits escape-injection-scrolled away from live,
/// so even a fully ANSI/padding-normalised byte-for-byte comparison fired
/// on nearly every "Older" click even though nothing else about the pane
/// had moved -- collapsing the whole point of remembering a depth back
/// into a full replay-and-restore on almost every request (the independent
/// review's headline finding). The rail exists to catch a REAL change
/// under the pane (a reply sent, another operator scrolling, the agent
/// producing genuinely new output) -- every one of those changes more than
/// just the trailing status line -- so tolerating drift confined to the
/// last line is a deliberate, narrow exception, not a general fuzzy match:
/// a different LINE COUNT (the screen reflowed) or any difference in an
/// earlier line still counts as material and still trips the rail. A
/// single-line pane has no separate footer line to exempt, so that case
/// compares in full rather than silently accepting any change at all.
pub(crate) fn normalized_content_diverges(current: &str, recorded: &str) -> bool {
    let current_lines = normalized_lines(current);
    let recorded_lines = normalized_lines(recorded);
    if current_lines.len() != recorded_lines.len() {
        return true;
    }
    if current_lines.len() <= 1 {
        return current_lines != recorded_lines;
    }
    let last = current_lines.len() - 1;
    current_lines[..last] != recorded_lines[..last]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::fake::FakeHerdr;

    #[tokio::test(start_paused = true)]
    async fn shortpane_escalates_harmlessly_and_returns_carried_revision() {
        // Recent(1000) == Visible (no extra native scrollback) and the pane
        // does not respond to PageUp (escape_reveal: None) -- the harmless
        // no-op case: PaneScroller still escalates (the "not richer" branch
        // covers this identically to an alt-screen pane), but the content
        // comes back unchanged, and the revision is the pane's own, carried
        // through, never fabricated.
        let herdr = FakeHerdr::new();
        herdr.seed_scroll_pane("w1:p1", "❯ ", "❯ ", None);

        let scroller = PaneScroller::new(&herdr);
        let (read, strategy) = scroller
            .read_history_with_strategy("w1:p1", 1)
            .await
            .unwrap();

        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert_eq!(read.text, "❯ ", "short pane's content is unchanged");
        assert_eq!(
            read.revision, 1,
            "revision is the pane's own, carried through"
        );

        // Escalation still ran the full send_text sequence (PageUp then
        // Ctrl+End) even though it changed nothing -- proving the restore
        // step is unconditional, not skipped when the pane didn't respond.
        assert_eq!(
            herdr.sent_text_log("w1:p1").await,
            vec![PAGE_UP.to_string(), RESTORE_BOTTOM.to_string()]
        );
    }

    #[tokio::test]
    async fn primaryscreen_pane_returns_native_scrollback_without_escalating() {
        // Recent(1000) is strictly richer than Visible -- herdr already
        // holds real scrollback for this pane, so PaneScroller must use it
        // directly and never touch send_text at all.
        let herdr = FakeHerdr::new();
        let history = "line 1\nline 2\nline 3\n❯ ";
        let visible = "line 3\n❯ ";
        herdr.seed_scroll_pane("w1:p1", visible, history, None);

        let scroller = PaneScroller::new(&herdr);
        let (read, strategy) = scroller
            .read_history_with_strategy("w1:p1", 1)
            .await
            .unwrap();

        assert_eq!(strategy, ScrollStrategy::NativeScrollback);
        assert_eq!(read.text, history);
        assert!(
            herdr.sent_text_log("w1:p1").await.is_empty(),
            "NativeScrollback must never call send_text"
        );
    }

    #[tokio::test]
    async fn altscreen_pane_escalates_and_always_restores_the_bottom() {
        // Recent(1000) == Visible (alt-screen VT semantics never accumulate
        // scrollback) but the pane DOES respond to PageUp -- PaneScroller
        // must escalate, return the revealed older content, and always send
        // the restore-to-bottom Ctrl+End afterward so a later Visible read
        // is back to the live tail.
        let herdr = FakeHerdr::new();
        let live_bottom = "Jump to bottom (ctrl+End)\n❯ ";
        let revealed = "...earlier transcript...\nJump to bottom (ctrl+End)\n❯ ";
        herdr.seed_scroll_pane("w1:p1", live_bottom, live_bottom, Some(revealed));

        let scroller = PaneScroller::new(&herdr);
        let (read, strategy) = scroller
            .read_history_with_strategy("w1:p1", 1)
            .await
            .unwrap();

        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert_eq!(read.text, revealed, "the escalated read is what's returned");
        assert_eq!(
            herdr.sent_text_log("w1:p1").await,
            vec![PAGE_UP.to_string(), RESTORE_BOTTOM.to_string()],
            "PageUp then Ctrl+End, in that order"
        );

        // The restore actually took effect: a fresh Visible read afterward
        // is back to the live bottom, not stuck showing the escalated read.
        let after = herdr
            .read_pane("w1:p1", ReadSource::Visible, 0)
            .await
            .unwrap();
        assert_eq!(after.text, live_bottom);
    }

    #[tokio::test(start_paused = true)]
    async fn equal_length_different_content_is_not_richer() {
        // "Not richer than" must stay ONE case covering exact equality AND
        // any other non-richer outcome identically -- same length, different
        // bytes, still escalates rather than trusting recent as-is.
        let herdr = FakeHerdr::new();
        herdr.seed_scroll_pane("w1:p1", "abc", "xyz", None);

        let scroller = PaneScroller::new(&herdr);
        let (_read, strategy) = scroller
            .read_history_with_strategy("w1:p1", 1)
            .await
            .unwrap();

        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
    }

    #[tokio::test(start_paused = true)]
    async fn escalated_reveal_that_lags_behind_the_redraw_is_still_returned() {
        // Regression, reproduced live against a real Claude Code pane
        // (herdr 0.7.4, Claude Code 2.1.220): the agent's TUI redraw is not
        // synchronous with the send_text() call that triggers it. A single
        // immediate read right after PageUp raced ahead of the redraw and
        // returned the stale, pre-scroll screen every time -- root cause of
        // "load older" appearing to do nothing. The retry loop must wait out
        // a short redraw lag instead of giving up after one read.
        let herdr = FakeHerdr::new();
        let live_bottom = "Jump to bottom (ctrl+End)\n❯ ";
        let revealed = "...earlier transcript...\nJump to bottom (ctrl+End)\n❯ ";
        herdr.seed_scroll_pane("w1:p1", live_bottom, live_bottom, Some(revealed));
        herdr.set_reveal_delay("w1:p1", 2); // stale for the first 2 reads

        let scroller = PaneScroller::new(&herdr);
        let (read, strategy) = scroller
            .read_history_with_strategy("w1:p1", 1)
            .await
            .unwrap();

        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert_eq!(
            read.text, revealed,
            "retries wait out the redraw lag instead of giving up after one read"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn escalated_reveal_gives_up_after_exhausting_all_read_attempts() {
        // If the redraw never lands within the retry budget, the last
        // (stale) read is used rather than looping forever or erroring --
        // restore-to-bottom still always runs regardless.
        let herdr = FakeHerdr::new();
        let live_bottom = "Jump to bottom (ctrl+End)\n❯ ";
        let revealed = "...earlier transcript...\nJump to bottom (ctrl+End)\n❯ ";
        herdr.seed_scroll_pane("w1:p1", live_bottom, live_bottom, Some(revealed));
        herdr.set_reveal_delay("w1:p1", 999); // never lands within the budget

        let scroller = PaneScroller::new(&herdr);
        let (read, strategy) = scroller
            .read_history_with_strategy("w1:p1", 1)
            .await
            .unwrap();

        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert_eq!(
            read.text, live_bottom,
            "exhausts attempts and returns the last (stale) read, not an error"
        );
        assert_eq!(
            herdr.sent_text_log("w1:p1").await,
            vec![PAGE_UP.to_string(), RESTORE_BOTTOM.to_string()],
            "restore-to-bottom still runs even when the redraw never landed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn restore_wait_lets_a_delayed_exit_from_scroll_view_land_before_returning() {
        // Regression, reproduced live: a real keystroke (e.g. a Reply-sheet
        // Send) arriving right after read_history returns got swallowed by
        // Claude Code as "dismiss scroll view" instead of reaching the
        // composer, because the agent's own exit from its scroll view had
        // not actually landed yet even though Ctrl+End's bytes had. The
        // restore-wait must consume that lag internally so a caller reading
        // Visible immediately afterward already sees the real, settled
        // state, not a still-scrolled one.
        let herdr = FakeHerdr::new();
        let live_bottom = "Jump to bottom (ctrl+End)\n❯ ";
        let revealed = "...earlier transcript...\nJump to bottom (ctrl+End)\n❯ ";
        herdr.seed_scroll_pane("w1:p1", live_bottom, live_bottom, Some(revealed));
        herdr.set_restore_delay("w1:p1", 2); // still looks scrolled for 2 reads after Ctrl+End

        let scroller = PaneScroller::new(&herdr);
        let (_read, strategy) = scroller
            .read_history_with_strategy("w1:p1", 1)
            .await
            .unwrap();
        assert_eq!(strategy, ScrollStrategy::EscapeInjection);

        let after = herdr
            .read_pane("w1:p1", ReadSource::Visible, 0)
            .await
            .unwrap();
        assert_eq!(
            after.text, live_bottom,
            "restore-wait consumed the lag internally -- an immediate read afterward is already settled"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn restore_wait_gives_up_after_exhausting_all_read_attempts() {
        // If the exit-from-scroll-view lag never resolves within the retry
        // budget, read_history_with_strategy must still return (not hang)
        // -- the escalated read itself is unaffected by the restore-side
        // wait either way.
        let herdr = FakeHerdr::new();
        let live_bottom = "Jump to bottom (ctrl+End)\n❯ ";
        let revealed = "...earlier transcript...\nJump to bottom (ctrl+End)\n❯ ";
        herdr.seed_scroll_pane("w1:p1", live_bottom, live_bottom, Some(revealed));
        herdr.set_restore_delay("w1:p1", 999); // never lands within the budget

        let scroller = PaneScroller::new(&herdr);
        let (read, strategy) = scroller
            .read_history_with_strategy("w1:p1", 1)
            .await
            .unwrap();

        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert_eq!(
            read.text, revealed,
            "the escalated read itself is unaffected by the restore-side wait"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn restore_to_live_settles_immediately_on_an_already_live_pane() {
        // Independent re-review fix (4): a pane that never scrolled makes
        // RESTORE_BOTTOM a structural no-op -- `wait_until_settled` must not
        // burn the full stability budget (~450ms) waiting for a change that
        // can never come. It should settle on the very first pair of
        // identical reads (one `STABILITY_READ_INTERVAL`, ~50ms).
        let herdr = FakeHerdr::new();
        herdr.seed_scroll_pane("w1:p1", "❯ ", "❯ ", None);

        let scroller = PaneScroller::new(&herdr);
        let start = tokio::time::Instant::now();
        let read = scroller.restore_to_live("w1:p1").await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(read.text, "❯ ");
        assert!(
            elapsed <= STABILITY_READ_INTERVAL,
            "an already-live restore must settle within one stability interval, not the full \
             budget -- took {elapsed:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn restore_to_live_still_waits_out_a_genuine_transition() {
        // Independent re-review fix (4): the short-circuit above must not
        // cut a REAL transition off early. The pane is genuinely scrolled,
        // and its exit lands one read late (mirrors
        // `restore_wait_lets_a_delayed_exit_from_scroll_view_land_before_
        // returning`'s own asynchronous-redraw-lag fixture) -- the first
        // post-send read still echoes the scrolled frame, and only the read
        // after that shows the real live content twice in a row.
        // `wait_until_settled` must keep polling through the stale echo and
        // land on the real settled value, not the stale one.
        let herdr = FakeHerdr::new();
        let live_bottom = "Jump to bottom (ctrl+End)\n❯ ";
        let revealed = "...earlier transcript...\nJump to bottom (ctrl+End)\n❯ ";
        herdr.seed_scroll_pane("w1:p1", live_bottom, live_bottom, Some(revealed));
        herdr.set_restore_delay("w1:p1", 1); // still looks scrolled for 1 read after Ctrl+End

        let scroller = PaneScroller::new(&herdr);
        // Genuinely scroll the pane first -- the state restore_to_live is
        // meant to exit.
        herdr.send_text("w1:p1", PAGE_UP).await.unwrap();

        let read = scroller.restore_to_live("w1:p1").await.unwrap();

        assert_eq!(
            read.text, live_bottom,
            "restore_to_live must land on the real live content, not the stale frame the \
             redraw lag echoed first"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn pages_gt_1_hops_back_further_than_a_single_pageup() {
        // User field-test finding (2026-07-28): a single "load older" always
        // revealed the same one page back, no matter how many times it was
        // repeated, because every request restored to live before the next
        // one began -- there was no way to ask for "further back than last
        // time". `pages` lets one call hop back N times before restoring.
        let herdr = FakeHerdr::new();
        let live_bottom = "page0 (live)\n❯ ";
        herdr.seed_scroll_pane(
            "w1:p1",
            live_bottom,
            live_bottom,
            Some("page1 further back\npage0 (live)\n❯ "),
        );
        herdr.push_escape_page(
            "w1:p1",
            "page2 even further back\npage1 further back\npage0 (live)\n❯ ",
        );
        herdr.push_escape_page(
            "w1:p1",
            "page3 furthest back\npage2 even further back\npage1 further back\npage0 (live)\n❯ ",
        );

        let scroller = PaneScroller::new(&herdr);
        let (read, strategy) = scroller
            .read_history_with_strategy("w1:p1", 3)
            .await
            .unwrap();

        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert!(
            read.text.contains("page3 furthest back"),
            "3 pages requested in one call should land on the 3rd page, not just the 1st: got {:?}",
            read.text
        );

        // Always restores afterward regardless of how many pages were hopped.
        let after = herdr
            .read_pane("w1:p1", ReadSource::Visible, 0)
            .await
            .unwrap();
        assert_eq!(after.text, live_bottom);
    }

    #[tokio::test(start_paused = true)]
    async fn pages_beyond_what_the_agent_has_stops_early_without_erroring() {
        // Only 1 page exists; asking for 5 must not error or loop forever --
        // it lands on the 1 available page and stops (later hops aren't
        // richer than the one before, so they're skipped).
        let herdr = FakeHerdr::new();
        let live_bottom = "page0 (live)\n❯ ";
        let only_page = "page1 further back\npage0 (live)\n❯ ";
        herdr.seed_scroll_pane("w1:p1", live_bottom, live_bottom, Some(only_page));

        let scroller = PaneScroller::new(&herdr);
        let (read, strategy) = scroller
            .read_history_with_strategy("w1:p1", 5)
            .await
            .unwrap();

        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert_eq!(read.text, only_page);
    }

    #[tokio::test(start_paused = true)]
    async fn read_to_depth_from_1_to_2_sends_exactly_one_pageup_and_no_restore() {
        // scroll-keep-position: the whole point of `read_to_depth` over
        // `read_history` -- a caller already at depth 1 asking for depth 2
        // must send only the ONE additional PageUp the delta actually
        // needs, never a restore, and never a replay of the first hop.
        let herdr = FakeHerdr::new();
        let live_bottom = "page0 (live)\n❯ ";
        herdr.seed_scroll_pane(
            "w1:p1",
            live_bottom,
            live_bottom,
            Some("page1 further back\npage0 (live)\n❯ "),
        );
        herdr.push_escape_page(
            "w1:p1",
            "page2 even further back\npage1 further back\npage0 (live)\n❯ ",
        );

        let scroller = PaneScroller::new(&herdr);
        let (first, strategy, reached) = scroller.read_to_depth("w1:p1", 0, 1).await.unwrap();
        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert!(first.text.contains("page1 further back"));
        assert_eq!(reached, 1, "the depth actually reached is 1");
        assert_eq!(
            herdr.sent_text_log("w1:p1").await,
            vec![PAGE_UP.to_string()],
            "reaching depth 1 from a fresh pane sends exactly one PageUp"
        );

        let (second, strategy, reached) = scroller.read_to_depth("w1:p1", 1, 2).await.unwrap();
        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert!(
            second.text.contains("page2 even further back"),
            "depth 1 -> 2 should land on page 2: {:?}",
            second.text
        );
        assert_eq!(reached, 2, "the depth actually reached is 2");
        assert_eq!(
            herdr.sent_text_log("w1:p1").await,
            vec![PAGE_UP.to_string(), PAGE_UP.to_string()],
            "depth 1 -> 2 sends exactly one MORE PageUp, and no restore"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn read_to_depth_step_back_toward_live_sends_exactly_one_pagedown() {
        // scroll-keep-position review fix (E): the delta branch's PAGE_DOWN
        // side (moving toward live but not all the way, both depths
        // non-zero) had no coverage at all before this test. From depth 2
        // to depth 1 is a single hop: exactly one PageDown, never a
        // restore and never a PageUp.
        let herdr = FakeHerdr::new();
        let live_bottom = "page0 (live)\n❯ ";
        herdr.seed_scroll_pane(
            "w1:p1",
            live_bottom,
            live_bottom,
            Some("page1 further back\npage0 (live)\n❯ "),
        );
        herdr.push_escape_page(
            "w1:p1",
            "page2 even further back\npage1 further back\npage0 (live)\n❯ ",
        );

        let scroller = PaneScroller::new(&herdr);
        scroller.read_to_depth("w1:p1", 0, 2).await.unwrap();
        assert_eq!(
            herdr.sent_text_log("w1:p1").await,
            vec![PAGE_UP.to_string(), PAGE_UP.to_string()]
        );

        let (_read, strategy, reached) = scroller.read_to_depth("w1:p1", 2, 1).await.unwrap();
        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert_eq!(
            herdr.sent_text_log("w1:p1").await,
            vec![
                PAGE_UP.to_string(),
                PAGE_UP.to_string(),
                PAGE_DOWN.to_string(),
            ],
            "depth 2 -> 1 sends exactly one PageDown, never a restore or a PageUp"
        );
        // The fake never modeled PAGE_DOWN moving the pane (this module's
        // own doc comment on `PAGE_DOWN`: unlike PAGE_UP/RESTORE_BOTTOM,
        // its effect was never independently live-verified) -- so the hop
        // is a no-op and the loop breaks after the one attempt. The
        // ACTUALLY reached depth must reflect that reality (still 2, the
        // hop never really moved anything), not blindly report the
        // requested depth 1 -- exactly the drift review fix (D) exists to
        // prevent.
        assert_eq!(
            reached, 2,
            "a no-op hop must not be reported as having reached the requested depth"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn read_to_depth_returns_the_depth_actually_reached_when_the_hop_loop_breaks_early() {
        // scroll-keep-position review fix (D): only 1 page of scrollback
        // exists; asking to go from depth 0 to depth 5 must not claim depth
        // 5 was reached when the agent's own scrollback ran out after 1 hop
        // -- the caller (`server.rs`) records whatever this returns.
        let herdr = FakeHerdr::new();
        let live_bottom = "page0 (live)\n❯ ";
        let only_page = "page1 further back\npage0 (live)\n❯ ";
        herdr.seed_scroll_pane("w1:p1", live_bottom, live_bottom, Some(only_page));

        let scroller = PaneScroller::new(&herdr);
        let (read, strategy, reached) = scroller.read_to_depth("w1:p1", 0, 5).await.unwrap();

        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert_eq!(read.text, only_page);
        assert_eq!(
            reached, 1,
            "only 1 page exists -- the actual depth reached is 1, not the requested 5"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn read_to_depth_from_2_to_0_restores() {
        // Asking for depth 0 from anywhere non-zero always restores to the
        // live bottom, regardless of how many pages back the caller was.
        let herdr = FakeHerdr::new();
        let live_bottom = "page0 (live)\n❯ ";
        herdr.seed_scroll_pane(
            "w1:p1",
            live_bottom,
            live_bottom,
            Some("page1 further back\npage0 (live)\n❯ "),
        );
        herdr.push_escape_page(
            "w1:p1",
            "page2 even further back\npage1 further back\npage0 (live)\n❯ ",
        );

        let scroller = PaneScroller::new(&herdr);
        scroller.read_to_depth("w1:p1", 0, 2).await.unwrap();
        assert_eq!(
            herdr.sent_text_log("w1:p1").await,
            vec![PAGE_UP.to_string(), PAGE_UP.to_string()]
        );

        let (restored, strategy, reached) = scroller.read_to_depth("w1:p1", 2, 0).await.unwrap();
        assert_eq!(strategy, ScrollStrategy::EscapeInjection);
        assert_eq!(reached, 0, "depth 0 is always the depth actually reached");
        assert_eq!(
            restored.text, live_bottom,
            "depth 0 always lands back at live"
        );
        assert_eq!(
            herdr.sent_text_log("w1:p1").await,
            vec![
                PAGE_UP.to_string(),
                PAGE_UP.to_string(),
                RESTORE_BOTTOM.to_string(),
            ],
            "depth 2 -> 0 sends exactly one restore, no PageDowns"
        );
    }

    #[test]
    fn visible_len_strips_ansi_color_codes() {
        let plain = "hi";
        let colored = "\x1b[38;2;80;80;80mhi\x1b[0m";
        assert_eq!(visible_len(colored), visible_len(plain));
    }

    #[test]
    fn visible_len_ignores_trailing_padding_on_each_line() {
        // `format: ansi` reads right-pad every line to the terminal's column
        // width -- that padding is layout, not content, and must not count.
        let padded = "line one                    \nline two          \n";
        let unpadded = "line one\nline two\n";
        assert_eq!(visible_len(padded), visible_len(unpadded));
    }

    #[test]
    fn is_richer_ignores_ansi_and_padding_noise_that_a_raw_length_check_would_not() {
        // Live finding (2026-07-28): a real Claude Code pane's footer
        // (elapsed-time/token-percentage counters) re-renders on every read
        // regardless of scroll position, and its ANSI color codes plus
        // right-padding could make a genuinely richer (more actual lines)
        // read register as "not richer" under a naive `.text.len()`
        // comparison -- multi-page scroll-back plateaued after one hop as a
        // result. Heavier noise on the SHORTER-in-content read must not
        // outweigh genuinely more content on the other side.
        let heavily_padded_but_less_content =
            "\x1b[38;2;80;80;80m❯ \x1b[0m                                                  \n";
        let plain_but_more_content = "older line revealed\n❯ \n";
        let a = ScreenRead {
            text: plain_but_more_content.to_string(),
            revision: 1,
        };
        let b = ScreenRead {
            text: heavily_padded_but_less_content.to_string(),
            revision: 1,
        };
        assert!(is_richer(&a, &b));
        assert!(!is_richer(&b, &a));
    }

    #[test]
    fn normalized_content_diverges_ignores_a_footer_only_change() {
        // scroll-keep-position review fix (A): a live Claude Code pane's
        // footer (elapsed-time / token-percentage counters) keeps
        // re-rendering regardless of scroll position -- a fixture that
        // changes ONLY that trailing line must not read as a material
        // divergence, or `server.rs`'s content-mismatch rail fires on
        // nearly every "Older" click even though nothing else moved.
        let recorded = "page1 further back\npage0 (live)\nfooter 00:12 \u{b7} 45%";
        let current = "page1 further back\npage0 (live)\nfooter 00:59 \u{b7} 47%";
        assert!(
            !normalized_content_diverges(current, recorded),
            "a footer-only change must not be treated as a material divergence"
        );
    }

    #[test]
    fn normalized_content_diverges_still_flags_a_real_content_change() {
        let recorded = "page1 further back\npage0 (live)\nfooter 00:12 \u{b7} 45%";
        let current = "a completely different frame\npage0 (live)\nfooter 00:59 \u{b7} 47%";
        assert!(
            normalized_content_diverges(current, recorded),
            "a change earlier than the last line must still count as material"
        );
    }

    #[test]
    fn normalized_content_diverges_flags_a_line_count_change() {
        let recorded = "page1 further back\npage0 (live)\n\u{2771} ";
        let current = "page1 further back\nan extra line appeared\npage0 (live)\n\u{2771} ";
        assert!(
            normalized_content_diverges(current, recorded),
            "a different line count is a structural change, always material"
        );
    }

    #[test]
    fn normalized_content_diverges_ignores_ansi_and_padding_noise_that_carries_no_content_change() {
        // Same content, wrapped in different ANSI colour codes and
        // right-padding to the terminal's column width -- rendering noise
        // that must never register as a divergence, single line or not
        // (the same normalisation `is_richer` already relies on for its own
        // length comparison, `is_richer_ignores_ansi_and_padding_noise...`
        // above).
        let recorded = "\x1b[38;2;80;80;80m\u{2771} \x1b[0m                    ";
        let current = "\x1b[38;2;40;40;40m\u{2771} \x1b[0m";
        assert!(
            !normalized_content_diverges(current, recorded),
            "ANSI colour codes and trailing padding differing must not register as content \
             diverging when the actual text is identical"
        );
    }
}
