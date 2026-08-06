//! Transcript live tail — reads a coding agent's own on-disk JSONL
//! transcript (`~/.claude/projects/<encoded-cwd>/<session>.jsonl`) as a
//! second, *semantic* observation source beside the terminal's polled screen
//! (`pane.read`). The file is append-only and written by the agent itself,
//! so a byte cursor over it is gap-free: everything between two polls is
//! read, regardless of poll interval — unlike the screen, where content that
//! scrolls past between ticks is unrecoverable
//! (`docs/history/agent-terminal/CONTEXT.md`, D9: the transcript is a
//! second tab beside Terminal, not a toggle inside the terminal frame,
//! because the two lose data when collapsed into one frame).
//!
//! The watch set spans the main session file and every subagent transcript
//! under `<session-id>/subagents/*.jsonl`. Opening backfills the tail (the
//! client's ring lands full — an empty view on open reads as broken); every
//! later call returns only what was appended since, merged across files by
//! record timestamp. This module keeps no per-pane state — the cursor lives
//! in the caller and comes back on each request. Nothing is ever persisted
//! server-side: the transcript is the agent's own artifact, not mdview's.
//!
//! Ported from herdr-go's `src/transcript/mod.rs`, which had zero
//! dependencies on any other herdr-go module. This port adds one guard the
//! original relied on implicitly and now states explicitly in its own
//! right: [`parse_cursor`]'s rejection of any watched-file name carrying a
//! path separator or a `..` component is the only thing standing between a
//! caller-supplied cursor and a read outside the per-cwd project directory
//! — see the module's `cursor_never_escapes_the_project_dir` test, which
//! this port keeps and treats as security-relevant, not incidental.

use std::fs;
use std::io::{Read as _, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Max complete lines of one `tool_result` rendered before eliding — a single
/// huge output must not flush the client's whole 200-line ring.
const TOOL_OUTPUT_MAX_LINES: usize = 40;
/// Max lines of one text block (user prompt / assistant message).
const TEXT_MAX_LINES: usize = 80;
/// Per-display-line char cap (clipped with an ellipsis, char-boundary safe).
const MAX_LINE_CHARS: usize = 400;
/// Max display lines returned by one poll — beyond this the *oldest* lines of
/// the poll are elided (the client trims to its own ring anyway).
const POLL_MAX_LINES: usize = 400;
/// Max transcript bytes consumed per poll. More simply waits for the next
/// tick — the cursor only advances past what was actually read, so nothing
/// is skipped.
const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;
/// How far back from the end of the session file the *opening* call reads to
/// fill the view immediately (an empty view on open reads as "broken/
/// limited" — the ring should start full, not empty). The rendered result is
/// still capped at [`OPEN_BACKFILL_MAX_LINES`].
const OPEN_BACKFILL_BYTES: u64 = 512 * 1024;
/// Display-line cap for the opening backfill — matches the client's 200-line
/// ring, so open lands with the ring exactly full.
const OPEN_BACKFILL_MAX_LINES: usize = 200;
/// How many of the most recently active subagent transcripts get a tail
/// backfill at open. The rest enter the watch set at EOF — tailed live from
/// then on, but their history doesn't compete for the opening ring.
const OPEN_ACTIVE_SUBAGENTS: usize = 4;

/// One poll's worth of freshly appended, already-rendered display lines plus
/// the cursor to hand back on the next poll.
#[derive(Debug)]
pub struct ActivityChunk {
    pub lines: Vec<String>,
    /// Opaque to the client: `<session-file-name>:<byte-offset>`.
    pub cursor: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TranscriptError {
    /// No transcript exists for this cwd — not an error condition for the
    /// caller, just "this pane has no Activity view".
    #[error("no transcript available for this pane")]
    NotAvailable,
    #[error("malformed activity cursor")]
    BadCursor,
    #[error("transcript read failed: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, TranscriptError>;

/// Claude Code's project-directory encoding of a working directory: every
/// non-ASCII-alphanumeric byte becomes `-` (e.g. `D:\a\b` → `D--a-b`,
/// `/home/dev/x` → `-home-dev-x`). Every path separator and `.` in the input
/// is folded away by this encoding, so `cwd` itself can never introduce a
/// traversal component into the resulting directory name.
pub fn encode_project_dir(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// `$CLAUDE_CONFIG_DIR/projects` when set, else `<home>/.claude/projects`
/// (`USERPROFILE` on Windows, `HOME` elsewhere — same env Claude Code uses).
/// This is Claude Code's own configuration surface, unrelated to mdview's or
/// to herdr's — mdview never reads a herdr configuration file here.
fn claude_projects_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("projects"));
    }
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(home_var).map(|h| PathBuf::from(h).join(".claude").join("projects"))
}

/// Tail the transcript for `cwd` against the default Claude Code root.
pub fn read_activity(cwd: &str, cursor: Option<&str>) -> Result<ActivityChunk> {
    let root = claude_projects_root().ok_or(TranscriptError::NotAvailable)?;
    read_activity_at(&root, cwd, cursor)
}

/// [`read_activity`] with an explicit projects root — the testable seam.
///
/// The watch set is the main session file **plus every subagent transcript**
/// under `<project>/<session-id>/subagents/*.jsonl` — since Claude Code
/// ~2.1.x, Task/teammate activity is written there, NOT as `isSidechain`
/// records in the main file, so a single-file tail shows a subagent as one
/// `⑂ task:` line and then silence. Events from all files are merged by
/// their record `timestamp` (RFC3339 — string order is time order) and
/// subagent lines carry a `⑂ ` prefix.
pub fn read_activity_at(root: &Path, cwd: &str, cursor: Option<&str>) -> Result<ActivityChunk> {
    let dir = root.join(encode_project_dir(cwd));
    if !dir.is_dir() {
        return Err(TranscriptError::NotAvailable);
    }
    let newest = newest_jsonl(&dir)?.ok_or(TranscriptError::NotAvailable)?;
    match cursor {
        None => open_tail(&dir, &newest, Vec::new()),
        Some(cursor) => poll_tail(&dir, &newest, cursor),
    }
}

/// One rendered record, ready to merge across files.
struct FileEvent {
    /// The record's own `timestamp` (RFC3339, UTC) — empty sorts first.
    ts: String,
    /// Read order across the whole call — the stable tie-break.
    order: usize,
    lines: Vec<String>,
}

/// Open (or re-open after a session switch): backfill the tail of the main
/// file and of the most recently active subagent files so the ring lands
/// full; every other subagent file enters the cursor at EOF (watched from
/// now, never dumped later).
fn open_tail(dir: &Path, main: &Path, mut lead_lines: Vec<String>) -> Result<ActivityChunk> {
    let mut order = 0usize;
    let mut events: Vec<FileEvent> = Vec::new();
    let mut entries: Vec<(String, u64)> = Vec::new();

    let (main_events, main_off) = backfill_file(main, false, &mut order)?;
    events.extend(main_events);
    entries.push((file_base_name(main), main_off));

    let subs = list_subagent_files(dir, main)?;
    for (i, path) in subs.iter().enumerate() {
        if i < OPEN_ACTIVE_SUBAGENTS {
            let (sub_events, off) = backfill_file(path, true, &mut order)?;
            events.extend(sub_events);
            entries.push((file_base_name(path), off));
        } else {
            entries.push((file_base_name(path), fs::metadata(path)?.len()));
        }
    }

    let mut lines = merge_events(events);
    if lines.len() > OPEN_BACKFILL_MAX_LINES {
        lines = lines.split_off(lines.len() - OPEN_BACKFILL_MAX_LINES);
    }
    lead_lines.extend(lines);
    Ok(ActivityChunk {
        lines: lead_lines,
        cursor: cursor_for_entries(&entries),
    })
}

/// One poll: tail every cursor entry, pick up subagent files born since the
/// cursor was minted (from byte 0 — a new subagent is watched from birth),
/// merge by timestamp.
fn poll_tail(dir: &Path, newest: &Path, cursor: &str) -> Result<ActivityChunk> {
    let entries = parse_cursor_multi(cursor)?;
    let (main_name, main_offset) = entries[0].clone();
    let main_path = dir.join(&main_name);
    if !main_path.is_file() {
        // The watched session file vanished (rotated away) — re-open on the
        // newest sibling, saying so.
        return open_tail(dir, newest, vec!["— session switched —".to_string()]);
    }

    let mut order = 0usize;
    let mut events: Vec<FileEvent> = Vec::new();
    let mut new_entries: Vec<(String, u64)> = Vec::new();

    let (raw_main, main_new_off) = tail_raw(&main_path, main_offset)?;
    // Stateless mid-watch switch: only once the watched file is fully
    // consumed AND a strictly newer sibling exists do we hop — re-opening on
    // the new session with a visible divider.
    if raw_main.is_empty() && newest != main_path && is_newer(newest, &main_path)? {
        return open_tail(dir, newest, vec!["— session switched —".to_string()]);
    }
    events.extend(render_events(&raw_main, false, &mut order));
    new_entries.push((main_name, main_new_off));

    let mut known: std::collections::HashMap<String, u64> = entries[1..].iter().cloned().collect();
    for path in list_subagent_files(dir, &main_path)? {
        let name = file_base_name(&path);
        let offset = known.remove(&name).unwrap_or(0);
        let (raw, new_off) = tail_raw(&path, offset)?;
        events.extend(render_events(&raw, true, &mut order));
        new_entries.push((name, new_off));
    }
    // Entries left in `known` are files that vanished — dropped from the
    // cursor; nothing to read from them anyway.

    Ok(ActivityChunk {
        lines: cap_poll_lines(merge_events(events)),
        cursor: cursor_for_entries(&new_entries),
    })
}

/// Tail-window backfill of one file: render up to the last
/// [`OPEN_BACKFILL_BYTES`] of complete records, dropping the torn first line
/// of a mid-file window start.
fn backfill_file(
    path: &Path,
    from_subagent: bool,
    order: &mut usize,
) -> Result<(Vec<FileEvent>, u64)> {
    let len = fs::metadata(path)?.len();
    let start = len.saturating_sub(OPEN_BACKFILL_BYTES);
    let (mut raw_lines, new_offset) = tail_raw(path, start)?;
    if start > 0 && !raw_lines.is_empty() {
        // A mid-file start almost certainly lands mid-record; the first
        // "line" is that record's tail — drop it rather than render junk.
        raw_lines.remove(0);
    }
    Ok((render_events(&raw_lines, from_subagent, order), new_offset))
}

/// Render raw records into mergeable events; subagent lines get the `⑂ `
/// prefix (unless the record already carries one from `isSidechain`).
fn render_events(raw_lines: &[String], from_subagent: bool, order: &mut usize) -> Vec<FileEvent> {
    let mut out = Vec::new();
    for raw in raw_lines {
        let mut lines = render_record(raw);
        if lines.is_empty() {
            continue;
        }
        if from_subagent {
            for line in &mut lines {
                if !line.starts_with("⑂ ") {
                    line.insert_str(0, "⑂ ");
                }
            }
        }
        *order += 1;
        out.push(FileEvent {
            ts: record_timestamp(raw),
            order: *order,
            lines,
        });
    }
    out
}

/// The record's `timestamp` field, or empty (sorts first). RFC3339 UTC
/// strings compare correctly as plain strings.
fn record_timestamp(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| v["timestamp"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Merge events from all files into one line stream ordered by record
/// timestamp (read order as the stable tie-break).
fn merge_events(mut events: Vec<FileEvent>) -> Vec<String> {
    events.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.order.cmp(&b.order)));
    events.into_iter().flat_map(|e| e.lines).collect()
}

/// The session's subagent transcripts — `<dir>/<session-stem>/subagents/
/// *.jsonl`, newest mtime first (name as tie-break). Empty when the layout
/// doesn't exist (older Claude Code, or no subagents spawned yet).
fn list_subagent_files(dir: &Path, main: &Path) -> std::io::Result<Vec<PathBuf>> {
    let Some(stem) = main.file_stem().and_then(|s| s.to_str()) else {
        return Ok(Vec::new());
    };
    let sub_dir = dir.join(stem).join("subagents");
    if !sub_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<(SystemTime, String, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&sub_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry.metadata()?.modified()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        files.push((modified, name, path));
    }
    files.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    Ok(files.into_iter().map(|(_, _, p)| p).collect())
}

fn file_base_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Newest-mtime `*.jsonl` in `dir` (name as tie-break, for deterministic
/// tests on coarse-mtime filesystems).
fn newest_jsonl(dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let mut best: Option<(SystemTime, String, PathBuf)> = None;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry.metadata()?.modified()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let candidate = (modified, name, path);
        if best
            .as_ref()
            .map(|(m, n, _)| (candidate.0, &candidate.1) > (*m, n))
            .unwrap_or(true)
        {
            best = Some(candidate);
        }
    }
    Ok(best.map(|(_, _, p)| p))
}

fn is_newer(a: &Path, b: &Path) -> std::io::Result<bool> {
    Ok(fs::metadata(a)?.modified()? > fs::metadata(b)?.modified()?)
}

/// Serialize the watch set: `<file>:<offset>` entries joined by `;`, main
/// session file always first. Still an opaque string to the client.
fn cursor_for_entries(entries: &[(String, u64)]) -> String {
    entries
        .iter()
        .map(|(name, off)| format!("{name}:{off}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// Parse a multi-file cursor. A v1 single-entry cursor (`file.jsonl:123`)
/// parses as a one-entry watch set, so clients spanning a deploy keep
/// working — their subagent files are simply picked up as "new".
fn parse_cursor_multi(cursor: &str) -> Result<Vec<(String, u64)>> {
    let entries: Vec<(String, u64)> = cursor
        .split(';')
        .map(parse_cursor)
        .collect::<Result<Vec<_>>>()?;
    if entries.is_empty() {
        return Err(TranscriptError::BadCursor);
    }
    Ok(entries)
}

/// Parse and validate `<file-name>:<offset>`. The name is the per-file
/// session identifier the caller hands back on every poll, and it is used
/// directly to rejoin the project directory (`dir.join(&name)` in
/// [`poll_tail`]) — so it must be a bare `.jsonl` file name and nothing
/// else. This is the one containment guard in this module: a caller-
/// supplied cursor is untrusted input, and without this check a name like
/// `../../etc/passwd.jsonl` or `..\secrets.jsonl` would walk the resulting
/// path straight out of the per-cwd project directory computed in
/// [`read_activity_at`]. See `cursor_never_escapes_the_project_dir` below,
/// which is written to fail if any one of these four conditions is dropped.
fn parse_cursor(cursor: &str) -> Result<(String, u64)> {
    let (name, offset) = cursor.rsplit_once(':').ok_or(TranscriptError::BadCursor)?;
    let offset: u64 = offset.parse().map_err(|_| TranscriptError::BadCursor)?;
    let safe = !name.is_empty()
        && name.ends_with(".jsonl")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..");
    if !safe {
        return Err(TranscriptError::BadCursor);
    }
    Ok((name.to_string(), offset))
}

/// Read complete lines appended after `offset`. A partial trailing line (no
/// `\n` yet — the agent is mid-append) is left for the next poll: the
/// returned offset only ever advances past the last complete line.
fn tail_raw(path: &Path, offset: u64) -> std::io::Result<(Vec<String>, u64)> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len <= offset {
        // Nothing new; `len < offset` (truncation — Claude Code never does
        // this, but stay safe) re-anchors at the current end without a dump.
        return Ok((Vec::new(), len));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = Vec::new();
    file.take(MAX_READ_BYTES).read_to_end(&mut buf)?;
    let Some(last_newline) = buf.iter().rposition(|&b| b == b'\n') else {
        return Ok((Vec::new(), offset));
    };
    let complete = last_newline + 1;
    let text = String::from_utf8_lossy(&buf[..complete]);
    let lines = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    Ok((lines, offset + complete as u64))
}

fn cap_poll_lines(mut lines: Vec<String>) -> Vec<String> {
    if lines.len() > POLL_MAX_LINES {
        let dropped = lines.len() - POLL_MAX_LINES;
        lines = lines.split_off(dropped);
        lines.insert(0, format!("… (+{dropped} earlier lines this poll)"));
    }
    lines
}

// ---------------------------------------------------------------------------
// Record rendering — one transcript JSONL record to zero-or-more display
// lines. Defensive throughout: an unparseable line or unknown shape becomes a
// clipped raw line, never an error (nothing is silently dropped).
// ---------------------------------------------------------------------------

/// Render one raw transcript line into display lines.
pub fn render_record(raw: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return vec![clip(raw.trim())];
    };
    let sidechain = v["isSidechain"].as_bool().unwrap_or(false);
    let mut lines = match v["type"].as_str().unwrap_or("") {
        // File-head index record — meta, not conversation.
        "summary" => Vec::new(),
        "assistant" => render_content(&v["message"]["content"], Role::Assistant),
        "user" => render_content(&v["message"]["content"], Role::User),
        "system" => {
            let text = v["content"]
                .as_str()
                .or_else(|| v["message"].as_str())
                .unwrap_or("");
            match text.lines().next().map(str::trim).filter(|l| !l.is_empty()) {
                Some(first) => vec![clip(&format!("[system] {first}"))],
                None => Vec::new(),
            }
        }
        "" => vec![clip(raw.trim())],
        other => vec![format!("[{other}]")],
    };
    if sidechain {
        for line in &mut lines {
            line.insert_str(0, "⑂ ");
        }
    }
    lines
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    User,
    Assistant,
}

fn render_content(content: &serde_json::Value, role: Role) -> Vec<String> {
    match content {
        serde_json::Value::String(text) => push_text(text, role),
        serde_json::Value::Array(blocks) => {
            let mut out = Vec::new();
            for block in blocks {
                match block["type"].as_str().unwrap_or("") {
                    "text" => out.extend(push_text(block["text"].as_str().unwrap_or(""), role)),
                    "thinking" => {
                        let first = block["thinking"]
                            .as_str()
                            .unwrap_or("")
                            .lines()
                            .next()
                            .unwrap_or("")
                            .trim();
                        if !first.is_empty() {
                            out.push(clip(&format!("· thinking: {first}")));
                        }
                    }
                    "tool_use" => out.push(summarize_tool_use(
                        block["name"].as_str().unwrap_or("tool"),
                        &block["input"],
                    )),
                    "tool_result" => out.extend(render_tool_result(block)),
                    "" => {}
                    other => out.push(format!("[{other}]")),
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

fn push_text(text: &str, role: Role) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().take(TEXT_MAX_LINES).enumerate() {
        let lead = if role == Role::User && i == 0 {
            "» "
        } else {
            ""
        };
        if line.trim().is_empty() && lines.len() == 1 {
            continue;
        }
        out.push(clip(&format!("{lead}{line}")));
    }
    if lines.len() > TEXT_MAX_LINES {
        out.push(format!("… (+{} lines)", lines.len() - TEXT_MAX_LINES));
    }
    out
}

fn summarize_tool_use(name: &str, input: &serde_json::Value) -> String {
    let s = |key: &str| input[key].as_str().unwrap_or("");
    let line = match name {
        "Bash" => {
            let command = s("command");
            let mut it = command.lines();
            let first = it.next().unwrap_or("");
            let more = it.count();
            if more > 0 {
                format!("> bash: {first} (+{more} more lines)")
            } else {
                format!("> bash: {first}")
            }
        }
        "Edit" | "MultiEdit" => format!("✎ edit: {}", s("file_path")),
        "Write" => format!("✎ write: {}", s("file_path")),
        "NotebookEdit" => format!("✎ edit: {}", s("notebook_path")),
        "Read" => format!("≡ read: {}", s("file_path")),
        "Glob" => format!("? glob: {}", s("pattern")),
        "Grep" => format!("? grep: {}", s("pattern")),
        "Task" => {
            let what = input["description"]
                .as_str()
                .or(input["subagent_type"].as_str())
                .unwrap_or("");
            format!("⑂ task: {what}")
        }
        "TodoWrite" => "☰ todo update".to_string(),
        "WebFetch" => format!("? fetch: {}", s("url")),
        "WebSearch" => format!("? search: {}", s("query")),
        other => {
            let compact = serde_json::to_string(input).unwrap_or_default();
            format!("> {other}: {compact}")
        }
    };
    clip(&line)
}

fn render_tool_result(block: &serde_json::Value) -> Vec<String> {
    let text = match &block["content"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|i| i["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let is_error = block["is_error"].as_bool().unwrap_or(false);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().take(TOOL_OUTPUT_MAX_LINES).enumerate() {
        let lead = if is_error && i == 0 { "  ✗ " } else { "  " };
        out.push(clip(&format!("{lead}{line}")));
    }
    if lines.len() > TOOL_OUTPUT_MAX_LINES {
        out.push(format!(
            "  … (+{} lines)",
            lines.len() - TOOL_OUTPUT_MAX_LINES
        ));
    }
    out
}

/// Clip a display line to [`MAX_LINE_CHARS`] chars (char-boundary safe) and
/// strip control noise: `\r`, and ANSI escape sequences that tool outputs can
/// carry — this view renders text into a `<pre>`, not a terminal.
fn clip(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    let mut count = 0usize;
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip a CSI/OSC-ish sequence: ESC [ params final-byte, or ESC
            // plus one following char for the simple two-byte forms.
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if f.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                chars.next();
            }
            continue;
        }
        if c == '\r' || (c.is_control() && c != '\t') {
            continue;
        }
        if count >= MAX_LINE_CHARS {
            out.push('…');
            break;
        }
        out.push(c);
        count += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn project_dir(root: &Path, cwd: &str) -> PathBuf {
        let dir = root.join(encode_project_dir(cwd));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn encoding_matches_claude_codes_scheme() {
        assert_eq!(
            encode_project_dir(r"D:\projects\tools\AI\tools"),
            "D--projects-tools-AI-tools"
        );
        assert_eq!(
            encode_project_dir("/home/dev/projects/x"),
            "-home-dev-projects-x"
        );
    }

    #[test]
    fn open_lands_with_the_tail_already_rendered() {
        let root = tempfile::tempdir().unwrap();
        let dir = project_dir(root.path(), "/w/a");
        fs::write(
            dir.join("s1.jsonl"),
            "{\"type\":\"user\",\"message\":{\"content\":\"old history\"}}\n",
        )
        .unwrap();

        let chunk = read_activity_at(root.path(), "/w/a", None).unwrap();
        assert_eq!(
            chunk.lines,
            vec!["» old history"],
            "open backfills the tail"
        );
        assert!(chunk.cursor.starts_with("s1.jsonl:"));
        // Cursor sits at the file's end, so the very next poll is empty —
        // nothing is re-delivered.
        let next = read_activity_at(root.path(), "/w/a", Some(&chunk.cursor)).unwrap();
        assert!(next.lines.is_empty());
    }

    #[test]
    fn open_backfill_is_capped_and_keeps_the_newest_lines() {
        let root = tempfile::tempdir().unwrap();
        let dir = project_dir(root.path(), "/w/cap");
        let mut body = String::new();
        for i in 1..=300 {
            body.push_str(&format!(
                "{{\"type\":\"user\",\"message\":{{\"content\":\"prompt {i}\"}}}}\n"
            ));
        }
        fs::write(dir.join("s1.jsonl"), body).unwrap();

        let chunk = read_activity_at(root.path(), "/w/cap", None).unwrap();
        assert_eq!(chunk.lines.len(), OPEN_BACKFILL_MAX_LINES);
        assert_eq!(chunk.lines.first().unwrap(), "» prompt 101");
        assert_eq!(chunk.lines.last().unwrap(), "» prompt 300");
    }

    #[test]
    fn open_backfill_window_start_never_renders_a_torn_record() {
        // File bigger than OPEN_BACKFILL_BYTES: the window starts mid-record,
        // and that torn first line must be dropped, not rendered as junk.
        let root = tempfile::tempdir().unwrap();
        let dir = project_dir(root.path(), "/w/big");
        let mut body = String::new();
        let total = (OPEN_BACKFILL_BYTES as usize / 50) + 2_000;
        for i in 1..=total {
            body.push_str(&format!(
                "{{\"type\":\"user\",\"message\":{{\"content\":\"prompt {i:07}\"}}}}\n"
            ));
        }
        fs::write(dir.join("s1.jsonl"), body).unwrap();

        let chunk = read_activity_at(root.path(), "/w/big", None).unwrap();
        assert_eq!(chunk.lines.len(), OPEN_BACKFILL_MAX_LINES);
        assert_eq!(chunk.lines.last().unwrap(), &format!("» prompt {total:07}"));
        // Every rendered line is a whole record — no torn JSON passthrough.
        assert!(chunk.lines.iter().all(|l| l.starts_with("» prompt ")));
    }

    #[test]
    fn appended_records_arrive_incrementally_and_in_order() {
        let root = tempfile::tempdir().unwrap();
        let dir = project_dir(root.path(), "/w/b");
        let path = dir.join("s1.jsonl");
        fs::write(&path, "").unwrap();

        let open = read_activity_at(root.path(), "/w/b", None).unwrap();
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"hello"}},{{"type":"tool_use","name":"Bash","input":{{"command":"cargo test"}}}}]}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","content":"ok: 12 passed"}}]}}}}"#).unwrap();

        let chunk = read_activity_at(root.path(), "/w/b", Some(&open.cursor)).unwrap();
        assert_eq!(
            chunk.lines,
            vec!["hello", "> bash: cargo test", "  ok: 12 passed"]
        );
        // Fully consumed — next poll is empty, cursor stable.
        let next = read_activity_at(root.path(), "/w/b", Some(&chunk.cursor)).unwrap();
        assert!(next.lines.is_empty());
        assert_eq!(next.cursor, chunk.cursor);
    }

    #[test]
    fn partial_trailing_line_is_left_for_the_next_poll() {
        let root = tempfile::tempdir().unwrap();
        let dir = project_dir(root.path(), "/w/c");
        let path = dir.join("s1.jsonl");
        fs::write(&path, "").unwrap();
        let open = read_activity_at(root.path(), "/w/c", None).unwrap();

        // Half a record, no newline yet.
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(f, r#"{{"type":"assistant","mess"#).unwrap();
        let mid = read_activity_at(root.path(), "/w/c", Some(&open.cursor)).unwrap();
        assert!(mid.lines.is_empty(), "incomplete line must not render");
        assert_eq!(mid.cursor, open.cursor, "cursor must not advance");

        // The rest lands.
        writeln!(
            f,
            r#"age":{{"content":[{{"type":"text","text":"done"}}]}}}}"#
        )
        .unwrap();
        let after = read_activity_at(root.path(), "/w/c", Some(&mid.cursor)).unwrap();
        assert_eq!(after.lines, vec!["done"]);
    }

    #[test]
    fn missing_project_dir_is_not_available() {
        let root = tempfile::tempdir().unwrap();
        match read_activity_at(root.path(), "/nowhere", None) {
            Err(TranscriptError::NotAvailable) => {}
            other => panic!("expected NotAvailable, got {other:?}"),
        }
    }

    #[test]
    fn vanished_session_file_switches_to_newest_with_divider() {
        let root = tempfile::tempdir().unwrap();
        let dir = project_dir(root.path(), "/w/d");
        fs::write(dir.join("s1.jsonl"), "").unwrap();
        let open = read_activity_at(root.path(), "/w/d", None).unwrap();

        fs::remove_file(dir.join("s1.jsonl")).unwrap();
        fs::write(dir.join("s2.jsonl"), "{\"type\":\"summary\"}\n").unwrap();
        let chunk = read_activity_at(root.path(), "/w/d", Some(&open.cursor)).unwrap();
        assert_eq!(chunk.lines, vec!["— session switched —"]);
        assert!(chunk.cursor.starts_with("s2.jsonl:"));
    }

    /// Security-relevant: a cursor's file name is caller-supplied data one
    /// layer up (the terminal client hands it back on every poll) and is
    /// joined directly onto the per-cwd project directory in `poll_tail`.
    /// Every one of these must be refused before that join happens — a value
    /// carrying a path separator (forward or backward slash) and a value
    /// carrying a `..` component must both fail to open anything outside the
    /// project directory for `cwd`.
    #[test]
    fn cursor_never_escapes_the_project_dir() {
        let root = tempfile::tempdir().unwrap();
        project_dir(root.path(), "/w/e");
        fs::write(
            root.path()
                .join(encode_project_dir("/w/e"))
                .join("s1.jsonl"),
            "",
        )
        .unwrap();
        for bad in [
            "../../etc/passwd.jsonl:0",
            "..\\secrets.jsonl:0",
            "a/b.jsonl:0",
            "s1.jsonl:not-a-number",
            "no-extension:0",
            ":0",
        ] {
            match read_activity_at(root.path(), "/w/e", Some(bad)) {
                Err(TranscriptError::BadCursor) => {}
                other => panic!("cursor {bad:?} must be rejected, got {other:?}"),
            }
        }
    }

    fn user_record(ts: &str, text: &str) -> String {
        format!(
            "{{\"type\":\"user\",\"timestamp\":\"{ts}\",\"message\":{{\"content\":\"{text}\"}}}}\n"
        )
    }

    #[test]
    fn subagent_transcripts_are_tailed_and_merged_by_timestamp() {
        // Claude Code ≥2.1.x writes Task/teammate activity to
        // <session-id>/subagents/agent-*.jsonl, not into the main file.
        let root = tempfile::tempdir().unwrap();
        let dir = project_dir(root.path(), "/w/sub");
        let main = dir.join("sess.jsonl");
        fs::write(&main, "").unwrap();
        let sub_dir = dir.join("sess").join("subagents");
        fs::create_dir_all(&sub_dir).unwrap();
        let sub = sub_dir.join("agent-abc.jsonl");
        fs::write(&sub, "").unwrap();

        let open = read_activity_at(root.path(), "/w/sub", None).unwrap();
        assert!(open.cursor.contains("sess.jsonl:"), "main in watch set");
        assert!(open.cursor.contains("agent-abc.jsonl:"), "sub in watch set");

        // Interleaved appends: main, sub, main — timestamps decide order.
        let mut m = fs::OpenOptions::new().append(true).open(&main).unwrap();
        let mut s = fs::OpenOptions::new().append(true).open(&sub).unwrap();
        write!(m, "{}", user_record("2026-08-05T08:00:01Z", "main one")).unwrap();
        write!(s, "{}", user_record("2026-08-05T08:00:02Z", "sub speaks")).unwrap();
        write!(m, "{}", user_record("2026-08-05T08:00:03Z", "main two")).unwrap();

        let chunk = read_activity_at(root.path(), "/w/sub", Some(&open.cursor)).unwrap();
        assert_eq!(
            chunk.lines,
            vec!["» main one", "⑂ » sub speaks", "» main two"]
        );
        // Fully consumed: next poll is empty and stable.
        let next = read_activity_at(root.path(), "/w/sub", Some(&chunk.cursor)).unwrap();
        assert!(next.lines.is_empty());
    }

    #[test]
    fn subagent_file_born_mid_watch_is_picked_up_from_birth() {
        let root = tempfile::tempdir().unwrap();
        let dir = project_dir(root.path(), "/w/newsub");
        let main = dir.join("sess.jsonl");
        fs::write(&main, "").unwrap();

        let open = read_activity_at(root.path(), "/w/newsub", None).unwrap();

        // A Task spawns after the view opened: new subagent file appears.
        let sub_dir = dir.join("sess").join("subagents");
        fs::create_dir_all(&sub_dir).unwrap();
        fs::write(
            sub_dir.join("agent-new.jsonl"),
            user_record("2026-08-05T08:00:01Z", "born and speaking"),
        )
        .unwrap();

        let chunk = read_activity_at(root.path(), "/w/newsub", Some(&open.cursor)).unwrap();
        assert_eq!(chunk.lines, vec!["⑂ » born and speaking"]);
        assert!(chunk.cursor.contains("agent-new.jsonl:"), "now watched");
    }

    #[test]
    fn open_backfill_includes_recent_subagent_tails() {
        let root = tempfile::tempdir().unwrap();
        let dir = project_dir(root.path(), "/w/subfill");
        fs::write(
            dir.join("sess.jsonl"),
            user_record("2026-08-05T08:00:01Z", "main history"),
        )
        .unwrap();
        let sub_dir = dir.join("sess").join("subagents");
        fs::create_dir_all(&sub_dir).unwrap();
        fs::write(
            sub_dir.join("agent-abc.jsonl"),
            user_record("2026-08-05T08:00:02Z", "sub history"),
        )
        .unwrap();

        let open = read_activity_at(root.path(), "/w/subfill", None).unwrap();
        assert_eq!(open.lines, vec!["» main history", "⑂ » sub history"]);
    }

    #[test]
    fn render_covers_the_mid_session_record_shapes() {
        // Thinking collapses to one line.
        assert_eq!(
            render_record(
                r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"first line\nsecond line"}]}}"#
            ),
            vec!["· thinking: first line"]
        );
        // User prompt gets the » lead.
        assert_eq!(
            render_record(r#"{"type":"user","message":{"content":"do the thing"}}"#),
            vec!["» do the thing"]
        );
        // Sidechain frames are marked.
        assert_eq!(
            render_record(
                r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"sub says hi"}]}}"#
            ),
            vec!["⑂ sub says hi"]
        );
        // Unknown tool keeps its raw input (nothing silently dropped).
        let lines = render_record(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__x__y","input":{"a":1}}]}}"#,
        );
        assert_eq!(lines, vec![r#"> mcp__x__y: {"a":1}"#]);
        // Unparseable line falls back to clipped raw passthrough.
        assert_eq!(render_record("not json at all"), vec!["not json at all"]);
        // Summary records are meta, not conversation.
        assert!(render_record(r#"{"type":"summary","summary":"t"}"#).is_empty());
    }

    #[test]
    fn huge_tool_result_is_elided_not_flushed() {
        let big = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\\n");
        let raw = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","content":"{big}"}}]}}}}"#
        );
        let lines = render_record(&raw);
        assert_eq!(lines.len(), TOOL_OUTPUT_MAX_LINES + 1);
        assert_eq!(lines.last().unwrap(), "  … (+60 lines)");
    }

    #[test]
    fn clip_strips_ansi_and_bounds_line_length() {
        assert_eq!(clip("a\u{1b}[31mred\u{1b}[0mb"), "aredb");
        let long: String = "x".repeat(MAX_LINE_CHARS + 50);
        let clipped = clip(&long);
        assert_eq!(clipped.chars().count(), MAX_LINE_CHARS + 1);
        assert!(clipped.ends_with('…'));
    }
}
