# CONTEXT — waggledance-rename

Rename every occurrence of `mdview` to `waggledance` so the codebase matches the
repository's new name, Waggle Dance. 1162 occurrences across 67 files, excluding
`.git`, `.bee`, `target`, `Cargo.lock`, and `docs/history`.

## Locked decisions

| # | Decision | Rationale |
|---|---|---|
| D1 | Binary CLI is `waggledance`. MCP tool is `waggledance_view_file`. Config dir is `~/.waggledance/`. Crates are `waggledance`, `waggledance-core`, `waggledance-desktop`. | Full name over `waggle` / `wd` — matches the repo exactly; 12 characters accepted. |
| D2 | `~/.mdview/` migrates automatically, once, at startup: if the new dir is absent and the old one exists, `fs::rename` it and log one line. `registry.db`, `config.toml`, and every registered project survive. | No loss of registered state. `rename` is cheaper than a copy and naturally idempotent, because it only runs when the new dir is absent. |
| D3 | No `mdview` alias or symlink. One name only. | Clean break. Manual follow-up outside the repo: rename the systemd unit and its `ExecStart`, delete `~/.local/bin/mdview`, re-run `waggledance doctor --fix` per project. |
| D4 | `install.sh`, `install.ps1`, and `Cargo.toml` `repository` point at `github.com/thanhsmind/waggledance`. Release asset becomes `waggledance-<target>`. | Today `install.sh:12` and `install.ps1:15` point at `vantt/mdview` — they install *upstream's* binary, not this fork. The rename is the moment to correct the source. |
| D5 | `localStorage` `mdview-theme` and `sessionStorage` `mdview-folders-open` become `waggledance-*`, with a one-shot fallback: if the new key is empty, read the old key, write it to the new one, delete the old. | A clean swap silently loses the user's chosen theme and folder state. The fallback is ~10 lines of JS and disables itself after the first run. |
| D6 | `crates/mdview-desktop` renames both `productName` and `identifier`: `dev.mdview.app` → `dev.waggledance.app`. | Full consistency. Accepted cost: the OS treats it as a new app — app data, granted permissions, and install location start over, and the old `dev.mdview.app` must be removed by hand. |
| D7 | The installers move the binary **out of** the config dir: fallback becomes `$HOME/.local/bin` on unix and `%LOCALAPPDATA%\Programs\waggledance` on Windows. `~/.waggledance/` holds data only. | `install.sh:35` and `install.ps1:26` put the binary *inside* the config dir today. D2 would then rename the directory holding the running executable — on Windows, `ERROR_SHARING_VIOLATION`, so the default install could never migrate. Separating them is what makes D2 work at all. |
| D8 | The project marker accepts **both** names: `.waggledance.json` and `.mdview.json` (`cli.rs:317`). A recorded exception to D3. | The marker lives in *other people's* repositories; it is their file, not this binary's artifact. A clean break there silently loses project-root detection in repos whose owners did nothing wrong. Cost of keeping it: one array element. |
| D9 | Fix the pre-existing `doctor` JSON-writer bug in this feature: refuse to write when `~/.claude.json` does not parse, matching the TOML branch at `doctor.rs:378-383`. | W3 edits exactly that code. Today `doctor.rs:296-299` swallows the parse error into `json!({})` and rewrites the whole file, erasing every entry Claude Code keeps there. Walking past a known data-loss path while standing in it is not acceptable. |

## Derived requirement, not a separate decision

**`doctor` must remove the old MCP entry.** `doctor.rs:303,334,400` use the literal
`"mdview"` as both the lookup key and the inserted key across `~/.claude.json`,
`~/.codex/config.toml`, and the Antigravity config. Renaming the constant alone means
the already-registered check no longer sees the old entry, so `doctor --fix` would add a
*second* server and leave `mcpServers.mdview` behind — pointing at a binary D3 deletes.
A dead MCP server entry makes agents fail on startup, so removing the stale entry is part
of the rename, not an optional nicety.

## Rename surface — contract vs mechanical

**Contract** (breaks other systems or user data; needs deliberate handling):

- `~/.mdview` data dir — single resolver `crates/mdview-core/src/config.rs:182` `data_dir()`;
  a second, independent resolver at `crates/mdview/src/herdr/socket.rs:40` `herdr_config_dir()`;
  a third narrow helper at `crates/mdview/src/doctor.rs:259` `home()`.
- MCP tool name `mdview_view_file` (`crates/mdview/src/mcp.rs:59,85`) and the three config
  injection keys (`doctor.rs:303,334,400`).
- Env vars `MDVIEW_HERDR_BINARY` (`supervisor.rs:33,38`), `MDVIEW_INSTALL_DIR`,
  `MDVIEW_VERSION` (install scripts). Renamed clean under D3 — no old-name fallback.
- Install/release naming: `release.yml:62,64,70-75`, `install.sh:12-13,35,39`,
  `install.ps1:15-16,26`.
- Browser storage keys (D5).
- Skill-template markers `MDVIEW_START` / `MDVIEW_END` = `<!-- mdview:START/END -->`
  (`doctor.rs:483-484`), written into *other* files' text.

**Mechanical** (the compiler or a straight search-replace catches it): crate and package
names, 28 `use mdview_core::` imports across 12 files, `#[[bin]]` and clap `name =`,
CI `-p mdview`, 870 test declarations, prose in `PRD.md` / `README.md` / `docs/specs`.

## Non-goals

- No behavior change beyond the rename and the D2 migration.
- Nothing renames the `herdr` transport, its socket protocol, or its config dir — that is
  another project's namespace.
- No new features, no refactoring taken along for the ride.
