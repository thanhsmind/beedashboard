# Product Backlog

<!--
GENERATED FILE — do not hand-edit.
Rendered by `bee backlog render` from event-sourced PBI records in .bee/backlog.jsonl (backlog-unification D1/D3).
Regenerate: `bee backlog render --write`. Check freshness: `bee backlog render --check`.
Deterministic: byte-identical for the same backlog.jsonl contents — status-grouped, id-sorted entries, LF endings,
never a generation timestamp or any other wall-clock value.
-->

| ID | Story | CoS | Status | Feature |
|----|-------|-----|--------|---------|
| p-498dd298 | doctor reorders every key in the user's ~/.claude.json | serde_json is declared without preserve_order, so its Map is a BTreeMap and each doctor write alphabetically reorders the whole file. Content survives; ordering and formatting do not. Fix by enabling preserve_order, with a test asserting an unrelated key block keeps its original order across a fix. | proposed | — |
| p-732028ad | docs/backlog.md line 3 still names the old command | The header reads PBI rows cho mdview and is live prose, unlike the 17 other mentions which sit inside done rows and are history. Note: bee backlog render --write is NOT the fix — it replaces the hand-written ledger with a generated stub and drops 24 lines of detail. | proposed | — |
| p-cd0eadfc | bee backlog add refuses every argument shape | bee backlog add is unusable in this build: even the minimal form (--title alone) is refused with unsupported argument shape, while bee backlog pbi add works. Either the dispatcher shape or the help text is wrong. | proposed | — |
| p-e5b770fe | Parse install.ps1 in CI — no pwsh on the dev machine | A windows-latest job at minimum runs [ScriptBlock]::Create over install.ps1's contents. The script was rewritten for the rename (repo, binary name, and the move to %LOCALAPPDATA%\Programs\waggledance) but has never been syntax-checked: pwsh is absent locally, so it is review-verified only. | proposed | — |
| p-e62c967c | Prove the desktop crate builds — it never has | cargo build --manifest-path crates/waggledance-desktop/Cargo.toml completes in CI or on a machine with pkg-config, libwebkit2gtk and gtk3. Today it fails inside libdbus-sys before reaching the crate's own code, and the same failure occurs against the old manifest path, so it is not a rename defect — but the crate is unproven. It is excluded from the workspace, so cargo test --workspace can never catch it. | proposed | — |

## Done / Declined

- [p-097cf752] Agent-facing MCP query surface: waggledance answers questions instead of agents re-reading files — done
