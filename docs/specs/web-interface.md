---
area: web-interface
updated: 2026-08-12
sources: [file-nav-ux, ui-polish-settings-sidebar, agent-terminal, terminal-open-access]
decisions: [12d62831, 99e8df73, 184c77b0]
coverage: partial
---

# Spec: Web interface navigation

The browser chrome shared across every page of the viewer: the top bar that is
always present, and the per-file sidebar used to move between files in a
project. This spec covers navigation and orientation, not the rendered document
content itself.

## Entry Points & Triggers

- Any page (project list, a rendered file, search results, settings, an error
  page) → shows the shared top bar.
- Opening `/` (or clicking the brand) → the project list.
- Opening a file's page → shows the chapter sidebar focused on that file's
  folder, a reading breadcrumb above the article, and (when the file has
  headings and/or is linked from elsewhere) a right-hand panel.
- Clicking "Settings" in the top bar → the settings page.
- Clicking the brand ("mdview") → the project list.
- Clicking a heading link in the right-hand "On this page" list, or a
  "Linked from" entry → jumps to that heading, or opens the linking file.
- Scrolling a file's content → the right-hand "On this page" list tracks
  which heading is currently in view.

## Data Dictionary

| # | Element | Meaning | Values |
|---|---|---|---|
| 1 | Brand | Always-present link back to the project list | "mdview" |
| 2 | Center slot | Page-specific orientation text in the top bar | a file's `project / path`, "· search", "Settings", or empty |
| 3 | Settings link | Always-present link to the settings page | — |
| 4 | Theme toggle | Always-present light/dark switch (behavior in the Appearance spec) | — |
| 5 | Chapter focus (file pages) | Which single folder the sidebar is currently showing | a folder within the project; starts at the viewed file's folder |
| 6 | Chapter breadcrumb | The ancestor path of the focused folder, each segment selectable | project root → … → focused folder |
| 7 | File label | How a file is named in the sidebar | its title (first H1); the file name when it has no title |
| 8 | Project row (project list) | One registered project | a row linking to the project — its name, indexed markdown file count, and when it was last seen (never the filesystem path, per R5) — with a delete control that unregisters it. A worktree of a registered project sits indented under it, labelled by its branch alone |
| 8a | Session marker (project row) | One coding session running inside that project | a small marker per session, carrying its state and the program it is running; absent entirely when the terminal switch is off (per R6) |
| 8b | Add-project field | Where the operator names a folder to register | one absolute folder path; the project's name is taken from the folder's own name |
| 8c | Suggestion | One folder holding coding agents that no registered project covers | its full path, the number of agent sessions in it, and a one-press register control (per R9) |
| 9 | Reading breadcrumb (file pages) | Orientation trail above the article, distinct from the chapter sidebar's zoom breadcrumb | project name → each path segment of the file, in order; segments are not independently clickable (orientation only) |
| 10 | "On this page" (TOC) | Right-hand list of the current file's headings (levels 1-4) | one entry per heading, indented by level, linking to that heading |
| 11 | "Linked from" (backlinks) | Right-hand list of other files that link to the one being viewed | empty when nothing links here; hidden entirely when both this and the TOC are empty |
| 12 | Project tab strip (a project's own page) | Section switcher on a registered project's landing page | Overview · Terminal · Transcript — Terminal and Transcript render unconditionally, whether or not the agent terminal has ever been switched on (opening either is gated only by the terminal's own switch, with no authentication behind it — see the Agent terminal spec) |

## Behaviors & Operations

### Project list

- **Triggers:** opening `/` or clicking the brand from anywhere.
- **What it shows:** one row per registered project, every name on the same
  left edge. A row links to the project's default file and shows its name,
  file count, and last-seen time — never the filesystem path (per R5). Each
  row carries a delete control that unregisters the project. A worktree of a
  registered project is indented under the project it branches from and
  labelled by its branch alone, so one repository with three checkouts reads
  as one project with three branches rather than four unrelated entries; a
  worktree whose parent is not registered stands on its own under its full
  name.
- **Which sessions are running where:** each row also carries one marker per
  coding session whose folder sits inside that project, each showing the
  session's state and the program it runs, and each opening that session's
  own terminal view. Every session in the project is marked, whatever its
  state — a stuck one is the one most worth seeing. A worktree row marks its
  own sessions, not its parent's. The markers are drawn when the page loads
  and do not move on their own; seeing a change means reloading. When the
  terminal switch is off, or the session host cannot be reached or does not
  answer promptly, rows render exactly as they otherwise would — no markers,
  no error, no empty space (per R6).
- **Add a project:** the list carries a single field for a folder path. Naming
  a folder registers it and returns to the refreshed list, with the project
  taking the folder's own name. Refusals — a path that is not absolute, does
  not exist, is not a folder, is already registered, sits in or around a
  protected folder, or holds too much to index — are each reported on the page
  in fixed words, and the list is never left silently unchanged (per R7).
- **Suggested projects:** under the list, one entry per folder where a coding
  agent is running that no registered project covers — its full path, how many
  agent sessions are in it, and a control that registers it in one press,
  going through the same registration as the field above and meeting the same
  refusals. Only sessions with a coding agent count (per D a302ac94): a plain
  shell session never produces a suggestion, matching how the Unassigned
  group is drawn. A folder already inside a registered project is
  never suggested, whether or not that folder still exists on disk and whether
  or not the session reports its location by a roundabout route; nor is a
  session whose location is reported as nothing at all, or by a path that
  walks up through a parent. Two sessions in one folder are one entry, and
  entries are ordered by their path. The block shows nothing beyond the path
  and count — never a session's own name or title — carries no dismissal, is
  recomputed on every page load, and follows the terminal switch alone,
  showing nothing when it is off (per R6, R9).
- **Unassigned agents card:** when both the terminal switch and the
  Unassigned group's own switch are on (see the Agent terminal spec), one
  extra card, "Unassigned agents," sits below the project rows and
  opens a page listing coding agents that fall outside every registered
  project's root. The card itself is a bare presence marker — it carries no
  agent name and no working directory. With either switch off, the card
  does not appear at all — showing it while the group itself is switched
  off would disclose that this host has a host-wide pane group configured,
  with nothing left to gate that disclosure.
- **Delete / unregister:** activating a row's delete control asks the operator
  to confirm, then removes the project from the registry and returns to the
  list. This removes only the registry entry and its index — **the files on
  disk are untouched**, and re-registering re-scans them. The endpoint is
  unauthenticated, like every route in mdview, the agent terminal family
  included (see the Agent terminal spec), so anyone who can reach it can
  unregister a project (reversible; no data loss).
- **Which file a project opens to:** a fixed, predictable rule (never "whatever
  was indexed first"): a `README.md` wins over everything, else an `index.md`,
  else the shallowest-path then alphabetically-first markdown file. Basename
  matching is case-insensitive. So a project with a README lands on it.
- **Afterwards:** the operator picks a project by name without seeing where
  it lives on disk, or removes one it no longer wants listed.

### A file the project does not have

- **Triggers:** asking for a path that the project's index does not hold —
  most often a file that exists on disk but was written while the viewer was
  not watching, or one whose address was typed or shared wrongly.
- **What it shows:** a not-found page naming what happened, and — when the
  project itself is known — a single "Refresh index" control. There is no
  such control when the project is unknown, because there would be nothing
  to reconcile.
- **Refreshing:** activating it reconciles that one project against disk and
  returns the reader to the exact address they asked for. A file that was on
  disk all along renders on arrival; a genuinely missing file returns the
  same not-found page, now truthfully. The control needs nothing but the
  page itself — no terminal, no command, and no scripting in the browser
  (per R10).
- **Afterwards:** a reader who was told "not found" about a file they can see
  in their editor fixes it from the page they are already on.

### Reading breadcrumb (file pages)

- **Triggers:** viewing any file.
- **What it shows:** the project name followed by each path segment of the
  file being viewed, for orientation above the article. This is distinct
  from the chapter sidebar's zoom breadcrumb (element 6), which is
  interactive and scoped to folders, not the file path.
- **Afterwards:** the operator can see where the current file sits in the
  project without it crowding the article title directly below it.

### Right panel — table of contents + backlinks (file pages)

- **Triggers:** viewing a file that has headings (levels 1-4) and/or is
  linked from other files in the project.
- **What it shows:** an "On this page" list of the file's headings (when any
  exist), and a "Linked from" list of files that link to this one (when any
  exist). The panel does not render at all when both are empty.
- **What it does while scrolling:** the "On this page" entry matching the
  heading currently in view is visually marked, tracking the reader's
  position down the article.
- **Afterwards:** the operator can jump to any heading or an inbound link,
  and always sees at a glance which section of the article they're in.

### Chapter sidebar search

- **Triggers:** typing in the search box above the chapter sidebar's file
  tree, then submitting.
- **What it does:** navigates to the current project's full-text search
  results page for that query (see the search results page, not covered by
  this spec).
- **Afterwards:** the search box sits with clear spacing above the file
  tree, so the two are not read as one continuous block.

### Top bar (all pages)

- **What it shows:** the brand, a page-specific center slot, the Settings link,
  and the theme toggle — on every page without exception (per R1). On a
  project's own pages the bar also carries that project's section switcher
  (element 12).
- **On a narrow screen:** everything in the bar that navigates *away* from the
  current page — the section switcher and the Settings link — collapses behind
  one menu control at the bar's right edge, opening as a panel that spans the
  full width directly under the bar, one comfortably-sized row per
  destination, the current section marked. The brand, the page label and the
  theme toggle stay on the bar itself: the toggle changes this page rather
  than leaving it, so it is never a press further away. The menu opens and
  closes on its own; it does not depend on scripting (per R8).
- **Afterwards:** from anywhere, the operator can reach Settings and the project
  list in one click, on any screen.

### Chapter sidebar (file pages) — breadcrumb zoom

- **Triggers:** viewing any file.
- **What it shows:** exactly **one** folder's contents at a time (per R2),
  presented as a book's chapter list — the files directly in the focused folder
  (each labelled by title, the currently-viewed one highlighted) are the primary
  content, under a "Chapters" label. The focused folder's immediate subfolders
  are **all collapsed into a single disclosure bar** ("Subfolders" + a count)
  above the chapter list, so however many subfolders exist they never crowd out
  the chapters. The bar is collapsed by default and remembered for the session;
  it opens automatically when the focused folder has no files of its own.
- **Default focus:** the folder containing the file being viewed.
- **Zoom out:** selecting any breadcrumb segment refocuses the sidebar on that
  ancestor folder (this is also how you go up a level).
- **Zoom in:** expanding the subfolders bar and selecting a subfolder refocuses
  on it.
- **What changes:** refocusing changes only what the sidebar lists — it does not
  navigate or reload. Selecting a *file* opens that file's page normally.
- **Afterwards:** the operator sees a short, folder-scoped list instead of the
  project's entire file list, and can move up or down the folder hierarchy
  without ever seeing the whole tree at once.

### Fuzzy file-jump palette (file pages)

- **Triggers:** pressing the jump shortcut (Cmd+K on macOS, Ctrl+K elsewhere)
  on any file page opens a centered overlay with a single text input; pressing
  it again, or Escape, or clicking outside the box, closes it.
- **What it does:** as the operator types, the project's files are ranked by a
  fuzzy match of the query against each file's **name and path** (not its
  content) and the top matches are listed live, each showing its title and its
  path. This is distinct from full-text search, which matches file *content*.
- **Navigation:** Arrow keys move the highlighted match; Enter opens the
  highlighted file; clicking a match opens it. An empty query shows no matches.
- **Afterwards:** the operator jumps directly to a file by approximate name
  without browsing the sidebar or running a content search.

### Copy as markdown (file pages)

- **Whole page:** a Copy button in the top bar (beside Settings and the theme
  toggle) copies the **entire page's raw markdown source** to the clipboard in
  one click, with brief "Copied" feedback. Because the top bar is sticky it
  stays reachable while scrolling; on narrow screens the button is icon-only.
- **Triggers (selection):** selecting text inside a rendered file and copying it
  (the normal copy gesture).
- **What it does:** instead of the rendered HTML/plain text, the clipboard
  receives the **raw markdown** of the source lines the selection spans. The
  granularity is whole source lines of the blocks the selection touches — a
  partial selection inside a block still yields that block's full source lines.
- **Fallback:** copying from outside the rendered article, or from a region that
  maps to no source, behaves as an ordinary copy.
- **Afterwards:** the operator (often an agent) pastes back authorable markdown,
  not rendered output — round-tripping documentation without de-rendering by hand.

### Mermaid diagram zoom / pan / fullscreen (file pages)

- **Triggers:** a rendered file containing a Mermaid diagram; the diagram is
  drawn client-side, then gains interactive controls.
- **What it offers:** hovering a diagram reveals a small toolbar — zoom in, zoom
  out, reset, and fullscreen. The mouse wheel zooms toward the cursor; dragging
  pans; reset restores the original view; fullscreen expands the diagram to fill
  the screen (Escape/toggle exits).
- **Afterwards:** the operator can read a large or dense diagram that would
  otherwise overflow its box, without leaving the page.

## Actors & Access

Not applicable in the role sense — a single local operator in a browser; no
authentication and no distinct roles for anything this spec covers, or
anywhere else in mdview, the agent terminal family included (it is gated
only by its own switch — see the Agent terminal spec). A file page's
sidebar data is the project's file list (paths + titles); no other actor
consumes it.

## Business Rules

- **R1 (per D 12d62831).** The Settings link (and the theme toggle) appear on
  every page via one shared top bar; no page renders its own divergent header.
- **R2 (per D 99e8df73).** The file-page sidebar shows exactly one folder at a
  time (breadcrumb-zoom), never the project's full flat file list; files are
  labelled by title, and moving between folders is done by zooming the
  breadcrumb in and out, not by scrolling one long list.
- **R3.** The fuzzy file-jump palette ranks files by name/path, never by
  content; it is the "jump to a file I can half-name" affordance and is kept
  distinct from full-text (content) search, which stays a separate results page.
- **R4.** Copying a selection from a rendered file yields the raw markdown of the
  spanned source lines, not the rendered output; the mapping is by source line
  range (block granularity), and a selection that maps to nothing copies normally.
- **R5 (per D 184c77b0, narrowed by D 8f21c4ab).** A project's filesystem root path is never shown on
  the project list page — only its name, indexed file count, and last-seen
  time. The project list page carries no authentication (per settings.md,
  outside the agent terminal family) and a wildcard/LAN-reachable bind is a
  supported mode (settings.md R3), so the operator's local path is treated
  the same way as any other local-only detail: never exposed to whoever can
  reach the page.
- **R6 (per D d356af5d, D bc3bf3bb, D 7810e5ee).** The session markers on a
  project row obey the terminal switch and nothing else: switch off, the page
  behaves as though the feature did not exist, and it does not ask the session
  host anything. Switch on, a marker names the session's state and the program
  it runs — never the agent's own generated identifier, and never a folder
  path, so R5 still holds. The markers are read once per page load; the page
  never polls, so there is nothing to keep open. The reading is bounded: a
  session host that is down, or that accepts and then does not answer, leaves
  the rows plain rather than delaying the page.
- **R7 (per D 6c41879e, D 4fcbe3fb).** Registering a folder from the page is
  open to whoever can reach it, exactly as unregistering is (R5's reasoning) —
  there is no list of permitted locations and no restriction to the local
  machine. Three refusals bound it. A folder that is, sits inside, or contains
  one of the system's protected locations — credential and configuration
  folders, and the machine-wide roots that hold them — is refused, so the home
  folder itself and the filesystem root are both refused while an ordinary
  folder inside the home folder registers normally. A folder holding more
  markdown than a registration is meant to take on, or taking too long to
  survey, is refused before anything is indexed. A folder already registered is
  refused as a duplicate, recognised through the folder's real location, so a
  shortcut to it or a trailing separator is caught too. Every refusal names its
  reason in fixed words and never repeats back what the operator typed.
- **R8 (per D a5f4f0c6, D 0f2c8d7a).** The narrow-screen menu's open state
  belongs to the page itself, not to a script: the menu opens, closes and
  navigates on a page whose scripting never ran. Scripting adds only two
  conveniences on top — closing on Escape, and closing when the reader
  presses outside the panel. Above the narrow width the menu is not a menu at
  all: its control is not shown and its contents sit in the bar exactly as
  they did before it existed.

- **R9 (per D 4d0e77a1, D 8f21c4ab).** The suggestion block is the one place
  on this page that prints a filesystem path, and it prints only folders no
  registered project covers — never a registered project's own location, which
  R5 still protects. It follows the terminal switch alone, not the Unassigned
  group's separate switch, even though it reads the same set of sessions: the
  consequence, accepted deliberately, is that with the terminal switch on
  anyone who can reach the page learns where on this machine coding sessions
  are running outside the registered projects. A folder is suppressed whenever
  it lies within a registered project by plain path containment, independently
  of whether that project's folder still exists; and a location reported by a
  path that walks up through a parent is never suggested at all, since
  registering such a path would be refused anyway.
- **R10.** The refresh control on a not-found page returns the reader only to
  an address within this viewer. The address to return to travels with the
  request and is therefore whatever the sender put there, so anything that
  would leave this site — another host, or an address that borrows the
  current one's protocol — is discarded in favour of the project's own home
  page. Reconciling a project is open to whoever can reach the page, exactly
  as registering and unregistering are (R7's reasoning): it costs a re-scan
  and changes no file on disk.


## Edge Cases Settled

- A file at the project root → the sidebar focus is the root; the breadcrumb is
  just the project name and there is no "up" affordance.
- A file whose title is empty or the same as its file name → the file name is
  used as the label.
- A folder containing both subfolders and files → subfolders are listed first,
  then files.
- Without client scripting, the file page still shows the current folder's files
  by title (a reduced, non-zoomable fallback), so navigation is never blank.
- A not-found page for an unknown project → the plain message, with no refresh
  control: there is no project to reconcile.
- Refreshing from a not-found page whose file really is absent → the reader
  lands back on the same not-found page, which now reflects a checked answer
  rather than a stale one.

## Open Gaps

- The interactive zoom (breadcrumb/subfolder selection) is delivered by client
  scripting; its behavior with scripting disabled is limited to the static
  current-folder fallback above — full parity is not a goal.
- Sort order of files within a folder (currently by label) and of subfolders is
  not a settled product rule, just current behavior.
- Whether search results and the project list should also adopt any of this
  folder-scoped navigation is not decided.
- The "On this page" current-heading marker's behavior before the reader has
  scrolled past the first heading, or when no heading is currently within the
  tracked viewport band, was not exercised this session — unverified.

## Visuals

No settled screenshot captured yet — the top bar, chapter sidebar, project
list, reading breadcrumb, and right panel have all changed across sessions; a
snapshot under `docs/specs/visuals/web-interface/` is an open item.

## Pointers (implementation)

- `crates/mdview/src/views.rs` — `topbar()` (shared header), `file_tree`
  (chapter sidebar: ships the file list as JSON + focus data), `project_list_page`,
  `breadcrumb()` (reading breadcrumb), `right_panel()` (TOC + backlinks), page
  functions.
- `crates/mdview/src/server.rs` — `index_page` (one timeout-bounded herdr
  snapshot behind the terminal switch, matched per project through
  `paths_boundary::Boundary` + `project_panes`), `register_project` and its
  `validate_register_path` guard chain (`paths_boundary::is_denied_root`, the
  canonical duplicate lookup, `indexer::bounded_scan_markdown_files`), all of
  it inside one `spawn_blocking`.
- `crates/mdview/assets/app.js` — chapter renderer (breadcrumb zoom in/out,
  files by title), TOC scrollspy (`IntersectionObserver` over the article's
  headings, toggles the matching TOC link's active state).
- `crates/mdview/assets/app.css` — `.chapter` / `.chap-*` styles, `.toc` /
  `.backlinks`, `.breadcrumb`, `.fg-sidebar-search`.
- `crates/mdview/assets/atelier/components.css` — `.fg-input` / `.fg-select`
  (shared form-field skeleton used by the sidebar search box too).
