<!-- FOR AI AGENTS - Human readability is a side effect, not a goal -->
<!-- Managed by agent: keep sections and order; edit content, not structure -->
<!-- Last updated: 2026-11-23 | Last verified: 2026-11-23 -->

# AGENTS.md

**Precedence:** the **closest `AGENTS.md`** to the files you're changing wins. This is the only one in the repo.

## Project

Herdr plugin `sbx.sandbox` runs coding agents through the Docker Sandboxes CLI.
The manifest starts the Rust executable or its platform launcher. `src/main.rs` dispatches actions, bridges, panes and events.
Mappings live in `HERDR_PLUGIN_STATE_DIR/panes/`. Configuration lives in `HERDR_PLUGIN_CONFIG_DIR/config.json`.

## Commands

<!-- AGENTS-GENERATED:START commands -->
- Build a linked checkout with `cargo build --release --locked`.
- Run the suite with `cargo test --locked`, or one integration target with `cargo test --locked --test management`.
<!-- AGENTS-GENERATED:END commands -->

## Response Style

- Answer first, elaborate only if needed. No sycophantic openers.
- For yes/no or status questions, lead with the answer.
- Skip preamble. Match response length to task complexity.

## Workflow

1. Read the rules and the relevant implementation before editing.
2. Run the affected tests, then `cargo fmt --check` and `cargo test --locked` before committing.
3. Report test results. The integration suite uses the native CLI fixture in `tests/fixtures/sbx.rs`.

## Golden Samples

- `src/management.rs` handles confirmation, ownership checks and deletion checkpoints.
- `src/start.rs` prepares and attaches the agent. `src/sbx.rs` captures CLI output and classifies errors.
- `tests/management.rs` exercises complete actions and inspects their result lines.

## Heuristics (quick decisions)
<!-- AGENTS-GENERATED:START heuristics -->
- Register actions in the manifest and Rust dispatcher, document them in the README and cover their behavior in `tests/`.
- Update `src/config-defaults.json`, config validation, the settings editor and the README together. Built-in agents live in `src/agents.json`.
- Use argument vectors for CLI calls. Captured calls need a timeout. Interactive sessions inherit the pane terminal.
- Check bridge, shell and deletion ownership across mappings sharing a sandbox. Lock sandbox names in sorted order before the pane. Recheck ownership and the confirmed targets before deletion.
- Verify an ambiguous removal against a complete sandbox inventory. An exec probe would start a stopped VM.
- Resolve and validate the workspace root and nested workdir before mounting. Use the state module for locked, atomic mapping updates.
- Add keybindings in `src/bindings.rs`, check existing chords, and update the README and manual checklist.
- Resolve published ports through `src/ports.rs` and the manifest link handler. Query Herdr through its CLI and batch pane inventory reads.
<!-- AGENTS-GENERATED:END heuristics -->

## Boundaries

### Always Do

- Document public interfaces. Keep `HERDR_SANDBOX_RESULT:` on the first stdout line of every action and send diagnostics to stderr.
- Keep the Cargo and manifest versions, README and changelog in sync.
- Use conventional commit messages (`feat:`, `fix:`, `docs:`, `test:`, `chore:`).

### Ask First

- Changing the manifest `id`, `min_herdr_version` or `platforms`.
- Changing the mapping store format (`STATE_VERSION`) or the result line schema (`RESULT_SCHEMA_VERSION`).
- Changing default agent launch flags (they mirror the Docker template defaults).
- Modifying `.github/workflows/ci.yml`.

### Never Do

- Require a compiler on the release installation path.
- Run destructive `sbx` commands (`rm`, `prune`) without the confirmation popup flow in `src/management.rs`.
- Build shell strings from user paths without `start::quote`; the pane command is executed by the
  user's interactive shell (PowerShell, bash, zsh or fish).
- Read tokens or secrets in the plugin; credentials belong to `sbx secret`.
- Commit secrets or the contents of a real `HERDR_PLUGIN_STATE_DIR`.
- Put raw escape bytes in source; use escaped literals or crossterm.

## Contracts this code depends on

| Contract | Where verified | Notes |
| ---------- | ---------------- | ------- |
| Herdr plugin env vars and `HERDR_PLUGIN_CONTEXT_JSON` fields | herdr 0.9.0 docs `plugins.mdx` | `src/action.rs` |
| `herdr pane split` JSON `.result.pane.pane_id` | herdr 0.9.0 `cli-reference.mdx` | `src/start.rs` |
| `worktree.removed` payload `data.worktree.path` | herdr `src/api/schema/events.rs` | `src/management/events.rs` |
| `sbx create/exec/ls/stop/rm` flags | Docker docs `data/sbx_cli/*.yaml` (sbx 0.42) | `src/sbx.rs` |
| `sbx exec` starts a stopped sandbox before running the command | Docker docs `sbx exec` description (sbx 0.42) | reconnect in `src/start.rs` |
| `sbx ports NAME --json` schema | **undocumented**; `the port parser` accepts several spellings | `src/sbx.rs`, `open-port` |
| `herdr pane report-agent/release-agent`, `tab create` JSON, `[[link_handlers]]` context | herdr 0.9.0 `cli-reference.mdx`, `plugins.mdx` | `src/start.rs`, `open-port` |
| `sbx ls --json` schema, `sbx` error wording, exec exit codes | schema verified on sbx 0.42.1 (`{"sandboxes":[{name,status,...}]}` parses); error wording and exit codes still **undocumented** | `inventory`, `sbx::classify` |
| Clone-mode checkout location inside the VM | verified on sbx 0.42.1 (2026-09-12): the clone sits at the host path, so no `--workdir` is needed | `start::exec_args` |

<!-- AGENTS-GENERATED:START module-boundaries -->
Keep CLI dispatch in `main.rs`, action result formatting in `action.rs`, and shared config, state, inventory and process logic in their respective modules.

<!-- AGENTS-GENERATED:END module-boundaries -->

## Scoped AGENTS.md (MUST read when working in these directories)
<!-- AGENTS-GENERATED:START scope-index -->
- none; this root file covers the whole repository
<!-- AGENTS-GENERATED:END scope-index -->

## When instructions conflict

Explicit user prompts override this file. Where this file and `README.md` disagree, fix whichever is wrong and keep the documented behavior and Rust tests in sync.
