<h1 align="center">Waggle Dance</h1>

<p align="center">
  <strong>The orchestrator's cockpit for a colony of AI coding agents.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="Built with Rust" src="https://img.shields.io/badge/built%20with-Rust-orange?logo=rust&logoColor=white">
  <img alt="Single binary" src="https://img.shields.io/badge/single-binary-brightgreen">
  <img alt="Read-only by design" src="https://img.shields.io/badge/read--only-by%20design-0aa">
  <img alt="Works with Claude Code" src="https://img.shields.io/badge/works%20with-Claude%20Code%20(MCP)-8A63D2">
</p>

<p align="center">
  When a forager bee finds nectar, it doesn't carry the whole field home — it dances the
  <em>direction and the distance</em> so the rest of the colony can go straight there.
  Waggle Dance does that for your agents: one browser tab that shows every project, every
  running agent, every thing waiting on you — and lets you answer, without ever becoming
  another editor.
</p>

<!-- ▶ HERO DEMO — add docs/assets/hero-demo.gif (or .mp4), then uncomment:
<p align="center">
  <img src="docs/assets/hero-demo.gif" alt="Waggle Dance: cross-project board, live agents, terminal reply" width="820">
</p>
-->

---

## The problem it solves

You are no longer writing the code. You are running four agents across three repositories,
each one halfway through something, each one occasionally stuck on a question only you can
answer. The state of that colony lives in a dozen scrollback buffers, a `.bee/` store, and
your memory.

Waggle Dance is the surface you watch it from. It reads every project you register — the
docs the agents write, the source they change, the workflow state they keep — rolls it up
into one board, and puts the agents' own terminals one click away. **You conduct. You don't
type code here.**

| | |
|---|---|
| 🐝 **One board, every project** | The front page rolls up features across every registered project into a single flat list — what's waiting on you, what's in progress, what shipped. |
| ✋ **"Waiting on you" is a real column** | A feature stopped at an unapproved gate, or paused mid-handoff, surfaces itself. You stop discovering blockers by scrolling. |
| 🖥️ **Talk to the agents** | Watch any agent's live screen, type a reply, send the keys that matter — arrows, Enter, Escape, Tab, Ctrl+C. Attach an image to a message. |
| 🗂️ **Every agent, one drawer** | A slide-in list of every agent across every project, grouped by status. Switch panes without switching windows. |
| 📖 **Read the docs** | Whole-project markdown: cross-folder links that never 404, full-text search (SQLite FTS5), Mermaid you can pan and zoom, live reload on save. |
| 💻 **Read the code** | A second reading surface: the project's source as it sits on disk, syntax-coloured, line-numbered. For pointing at a line, not for changing it. |
| 🤖 **Agent-native** | One MCP tool, `mdview_view_file`, hands an agent a clickable URL the moment it writes a doc. |
| 📱 **Conduct from a phone** | Responsive layout, sidebar drawer, light & dark. Over the LAN or an SSH tunnel. |
| 🦀 **One binary** | Rust. No runtime, no Node, no Docker. |

---

## See it

<!-- ▶ SCREENSHOTS — see "Media checklist" at the bottom for exact shots/sizes. -->
<table>
  <tr>
    <td width="50%"><img src="docs/assets/shot-reading.png" alt="Reading view: sidebar file tree, rendered doc, on-this-page TOC"><br><em>Reading view — file tree, rendered doc, live TOC</em></td>
    <td width="50%"><img src="docs/assets/shot-search.png" alt="Full-text search results across a project"><br><em>Project-wide full-text search</em></td>
  </tr>
  <tr>
    <td><img src="docs/assets/shot-mermaid.png" alt="Mermaid diagram with zoom controls"><br><em>Mermaid with pan / zoom / fullscreen</em></td>
    <td><img src="docs/assets/shot-mobile.png" alt="Waggle Dance on a phone with the sidebar drawer open"><br><em>Mobile — sidebar drawer, pinch-zoom diagrams</em></td>
  </tr>
</table>

---

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/vantt/mdview/main/install.sh | sh
mdview doctor --fix     # wire up Claude Code MCP integration
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/vantt/mdview/main/install.ps1 | iex
mdview doctor --fix     # wire up Claude Code MCP integration
```

Or from source (needs Rust):

```sh
cargo install --git https://github.com/vantt/mdview mdview
```

The CLI binary is `mdview`.

---

## Use in 30 seconds

```sh
mdview open docs/architecture.md
```

The daemon **auto-starts**, indexes the project, resolves the links, and prints a browser
URL. Open <http://localhost:7700> to reach the board over every project you've registered.

**Conducting from a remote machine over SSH?** Forward the port and browse locally:

```sh
ssh -L 7700:localhost:7700 user@host   # then open http://localhost:7700
```

---

## The board

Per project, top to bottom:

1. **Live** — who is running right now in this project.
2. **Feature Hub** — *Waiting on you*, *In Progress*, *Finished*, as cards.
3. **Shipped** — the finished list.
4. **Backlog & Review** — backlog items, findings, the review queue.

The front page is the cross-project view: one flat, merged feature list across every
project that carries a `.bee/` store, so you see the whole colony before you pick a hive.
Cross-project cards mark which features have a terminal session running in their checkout,
and link straight to it.

**The board never writes.** It approves no gate, claims no cell, edits no backlog item,
ends no session — and the daemon verifies that by comparing each project's store
byte-for-byte before and after every request. Approving a gate is still something you do
where the agent is, deliberately. The board only makes sure you know it's waiting.

---

## Talking to the agents

Waggle Dance never runs a terminal of its own. It talks to a running
**[herdr](https://github.com/ogulcancelik/herdr)** and shows you the panes herdr already
manages. No herdr, and the tab says so plainly rather than breaking.

- **Terminal** tab — the agent's live screen, poll-refreshed, ~200 lines of scrollback.
- **Transcript** tab — that agent's own gap-free activity log.
- **Reply** — typed text is *staged*, then sent on your word. Keys (arrows, Enter, Escape,
  Tab, Ctrl+C) send immediately. Images attach to a message; attachments live outside the
  project folder, so replying never dirties a repository.
- **Start an agent** — from a preset or a plain shell.

Two background duties exist and are **off by default**: keeping herdr alive, and notifying
you when an agent's status changes.

> **tmux as a transport is not implemented.** herdr is the only backend today. Driving
> plain tmux sessions directly is on the roadmap, not in the binary.

---

## Reading surfaces

Beside each project's name is a section switch:

- **Docs** — the whole project's markdown at any folder depth. Every internal link
  (`../`, `./sub/`, anchors) is rewritten into one URL namespace, so nothing 404s. Full-text
  search and fuzzy file-jump. A filesystem watcher pushes reloads over WebSocket.
- **Code** — the project's source files, folders before files, syntax-coloured and
  line-numbered. Same containment rule: nothing outside the project root is ever served.

Neither surface edits. Line numbers exist so a line can be pointed at.

---

## Agent integration (MCP)

```sh
mdview doctor --fix
```

Registers an MCP server for whichever of **Claude Code, Codex, and Antigravity** it detects
on your machine — it never writes config for a tool you don't have — exposing one tool:

- **`mdview_view_file(project_root, relative_path)`** → a clickable `url` to the rendered
  file, **auto-registering** the project and indexing it on first use.

Drop the snippet from [`docs/mdview-agents-template.md`](docs/mdview-agents-template.md)
into your project's `AGENTS.md` / `CLAUDE.md` and your agents will hand you a viewable URL
the moment they finish writing.

---

## CLI

```sh
mdview open <file.md>                # print the browser URL (auto-starts the daemon)
mdview register <dir> [--name ...]   # recursive scan + index a project
mdview search "query"                # full-text search (FTS5)
mdview status                        # is the daemon up?
mdview config edit                   # edit ~/.mdview/config.toml in $EDITOR
mdview restart                       # restart the daemon (apply config changes)
mdview doctor [--fix]                # diagnose & repair the integration
mdview serve [--host H] [--port P]   # optional: pre-start / bind a custom address
```

Most commands accept `--json`. Full reference, SSH workflows, and settings live in the
**[usage guide](docs/usage.md)**.

---

## How it works

One daemon owns the registry (`~/.mdview/registry.db`); browser tabs are just clients. On a
`view_file` call the server auto-creates the project, scans it recursively, indexes the
target file, resolves its links, and returns the URL. A filesystem watcher keeps the index
current and pushes reload signals over WebSocket. Board data is read from each project's
`.bee/` store; terminal data comes from herdr over its socket.

- **Rendering:** comrak (GFM) → server-side syntect highlight → ammonia sanitize. Mermaid renders client-side.
- **Search:** SQLite FTS5.
- **Containment:** only registered project roots are served; path traversal is guarded and project HTML is sanitized before it's sent.

---

## Security — read this before exposing it

**Waggle Dance has no authentication.** Not on the board, not on the docs, not on the code
view, not on the terminal.

The terminal family is **off until you switch it on** in settings, and a second switch is
needed before agents outside every registered project root become visible. Both are off by
default for a reason: an exposed daemon with the terminal switch on is *unauthenticated
remote code execution*, because replying to an agent pane is arbitrary input to a shell.

Bind it to localhost. Reach it over SSH forwarding, a VPN, or an authenticating reverse
proxy. `mdview serve --host 0.0.0.0` on a network you don't fully trust is not a
supported posture. Details in the [usage guide](docs/usage.md).

---

## Status

Actively developed. Docs viewer, code viewer, project-wide search, the cross-project board,
the agent terminal over herdr, MCP + CLI + `doctor`, and the mobile UX all work end-to-end.
A native desktop shell (Tauri) is experimental. A fleet-wide analytics rollup and a tmux
transport are known gaps, not shipped features. See [PRD.md](PRD.md) for the full design.

---

## Credits

Waggle Dance grew out of a markdown viewer and kept its ancestry. It also absorbs
[herdr-go](https://github.com/vantt/herdr-go) — the agent-terminal experiment that preceded
it, now retired. Grateful thanks to two prior open-source markdown servers whose hard-won
lessons are baked into the reading surfaces:

- **[mdserve](https://github.com/jfernandez/mdserve)** — Jose Fernandez, MIT. Watcher
  robustness across atomic editor saves, WebSocket reload-signal live reload, the
  pre-render-to-memory pipeline, path-traversal guarding, and port auto-increment on bind conflict.
- **[marky](https://github.com/GRVYDEV/marky)** — GRVYDEV, Apache-2.0. Recursive folder tree
  that respects `.gitignore`, atomic corrupt-resilient settings persistence,
  sanitize-before-serve, and nucleo-backed fuzzy search.

## License

MIT — see [LICENSE](LICENSE).
