# System Overview

Technology-agnostic description of what mdview does and how its areas fit
together. First read for anyone new to the repo. (Implementation: Rust; this
spec avoids code detail — see PRD.md for design and crates/ for code.)

## What it is

mdview is a local background server that makes a project's markdown viewable in
a browser with **working cross-folder links**, live reload, full-text search,
and a one-call agent integration over MCP. One daemon owns all state; browser
tabs (and, later, a desktop window) are clients of it. mdview has absorbed
herdr-go, the standalone gateway that watched and replied to coding agents
running under [herdr](https://github.com/ogulcancelik/herdr): every registered
project now also has a Terminal tab and a Transcript tab for the agents
running under it. herdr-go is retired; mdview is its successor. See the
Agent terminal spec.

## Core invariant

**At most one daemon** owns the registry (`~/.mdview/registry.db`). Every
launcher — CLI, MCP, future desktop — coordinates through `~/.mdview/daemon.lock`
(pid + port). No second server ever writes the same registry.

## Areas

- **Registry** — the set of registered projects (id, name, root path,
  timestamps). Projects are created explicitly (`register`) or **implicitly** the
  first time a file under a new root is viewed. Persisted; survives restart.
- **Indexer** — recursively scans a project root (respecting `.gitignore` and
  exclude patterns), recording each markdown file's relative path, title (first
  H1 or filename), size, and modified time, plus its full text for search.
  Steady state is **incremental** (per file-change event); a full re-scan
  reconciles drift.
- **Link resolution** — the defining feature. When rendering a file, every
  internal link is rewritten into the app's URL namespace by resolving it
  (including `../` across folders) against the project's index. Unresolved links
  are left as-is (broken); links to other projects are out of scope.
- **Renderer** — markdown → HTML: GFM, frontmatter stripped, code highlighted
  server-side with class-based styling (theme via CSS, no re-render), mermaid
  marked for client rendering, output sanitized so untrusted agent markdown is
  safe to view.
- **Appearance** — one cohesive visual style applied to every page, with a
  Light/Dark color scheme the operator can toggle (OS-default on first load,
  remembered per browser). Scheme swaps only the color layer; the interface is
  fully self-contained (no external appearance assets). See the Appearance spec.
- **Web interface** — a project list that registers a folder, marks each
  project with the coding sessions running inside it, and links into per-file
  pages with a file tree,
  themed rendering, and live reload. Non-markdown assets (images referenced
  from a rendered file, or any other file inside a registered project) are
  served from disk only when the file's extension is on a fixed, short
  allowlist of media types (the same types the renderer already recognizes for
  content-type detection: image formats and PDF) and the file is not inside a
  directory excluded from indexing; anything else — including dotfiles,
  extensionless files, and files in an excluded directory — is refused. This
  is on top of the existing path-traversal guard (a request can never resolve
  outside the project root, symlinks included).
- **Live reload** — a filesystem watcher (debounced) updates the index on change
  and pushes a reload signal over WebSocket; the browser reloads the page.
- **Search** — full-text (keyword) across a project or all projects.
- **Code** — a second way to read a project: its files as they sit on disk,
  folders before files, each source file shown with its syntax coloured and
  its lines numbered. Bounded by the same containment rule as every other file
  surface — nothing outside the project's own root is served, links out
  included. Reached from a switch beside the project name, so prose and source
  read as one place.
- **Short file links** — every indexed file also answers at a short, opaque
  address of its own, alongside its full path-shaped URL. The short address is
  stable for a given file and is what tools hand to a person, so a link stays
  short enough to paste into a chat, a commit message, or a terminal without
  wrapping. Both addresses reach the same page; neither replaces the other.
- **Agent integration (MCP)** — a single tool, `mdview_view_file(project_root,
  relative_path)`, that ensures the project exists, indexes the file, ensures the
  daemon is up, and returns a viewable URL. It returns the short address, and
  names the file's own path beside it in plain text — the short address is
  opaque, so without the path a transcript full of them says nothing about which
  file each one was.
- **CLI** — `serve` (daemon), plus `register / open / list / search / status /
  refresh / unregister / stop`, `doctor`, and `version` (prints the single-source
  app version, same as `--version`).
- **Installation** — the install script resolves which released version it is
  about to install (a specific requested version, or the latest release) and
  echoes that resolved version to the operator before/while installing, so the
  operator always knows which version they ended up with — the same
  single-source version reported everywhere else (CLI, settings page,
  `/health`).
- **Settings** — view and change the server binding, renderer theme, indexing
  behavior, and MCP transport, from a web page or `serve` CLI overrides.
  Server/Indexing/MCP changes need a restart to take effect. An optional
  display hostname can stand in for the real host/IP in every URL handed to a
  person or an agent, without changing what address the server binds/is
  health-checked on (see the Settings spec, R1) — this is a cross-area link
  into Agent integration and CLI `open`, both of which build their returned
  URL through this substitution.
- **Doctor** — diagnoses and safely repairs setup: config presence, daemon
  health, Claude Code MCP registration, and an AGENTS.md/CLAUDE.md mention of
  mdview's agent tool (all merged idempotently, with a backup where content
  already existed).
- **Agent terminal** — a per-project Terminal tab and Transcript tab for
  watching and replying to the coding agents herdr is running under that
  project's root, plus two off-by-default background duties (keeping herdr
  alive, notifying on status change). The only mdview surface gated by
  authentication. See the Agent terminal spec.

## Boundaries (non-goals)

Not a static site generator, editor, or public host. No cross-project link
resolution, no semantic search. No authentication outside the agent terminal
family's token gate (terminal, transcript, and agent-creation routes only —
see the Agent terminal spec); every other route, including the document
viewer itself, stays open exactly as before. Read-only outside that family:
the viewer itself never writes user files.

## Status

MVP implemented and verified end-to-end (link resolution in served HTML, live
reload, MCP handshake + view_file, doctor --fix). Planned: desktop shell (Tauri),
scoped live-reload, and UX polish (backlinks, TOC, command palette). See PRD.md
§8 and docs/distillery/porting-log.md.
