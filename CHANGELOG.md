# Changelog

## 1.0.0

- Ship the plugin as a Rust executable on Windows, Linux and macOS. Run actions, panes and the worktree hook directly on all three platforms.

- Add a keyboard-driven settings popup bound to prefix+shift+c.

- Use the same action IDs and keybindings on Windows, Linux and macOS.

- Run worktree removal cleanup through the native Rust binary, including the confirmation popup on Windows.

- Port reconnect, stop, clone fetch, key binding installation, replacement, deletion and orphan pruning to Rust. Add an opt-in smoke test that creates and removes a real sandbox.

- Port interactive sandbox login shells to Rust, including Windows pane launching and session ownership tracking.

- Launch a new sandbox and its configured agent with the native Rust action and pane bridge. Windows release installations use PowerShell.

- Port mapping details, opening published ports, and the overlay action to Rust.

- Port the read-only `list-sandboxes` action to Rust.

- Port the live sandboxes overlay to Rust.

- Port the doctor action to Rust, including config validation and bounded sbx calls.
- Begin the Rust port with the deletion-confirmation pane, verified release installers,
  and a cross-platform binary release workflow.

- Experimental Windows support with native PowerShell panes and Git for Windows.

## 0.1.0 - 2026-09-12

Initial release.

- Actions: `doctor`, `install-keybindings`, `start-agent`, `reconnect`, `open-shell`, `fetch-changes`,
  `stop`, `info`, `list-sandboxes`, `prune-mappings`, `sandboxes`, `open-port`, `replace-sandbox`,
  `forget-mapping`.
- `worktree.removed` hook that offers to delete the sandboxes of a removed worktree.
- Built-in adapters for Claude Code, Codex, Gemini CLI, OpenCode, Copilot CLI
  and Cursor Agent, plus custom agents with setup scripts.
- Mount and clone workspace modes.
- A `deletion-confirmation` popup pane that guards every deletion.
- Result marker line and stable error kinds for orchestration.
- `HERDR_SBX_BIN` and `HERDR_SBX_CONFIRMATION_TIMEOUT_MS` overrides.
- A manual test plan for real hosts in `docs/manual-testing.md`.
- A `prune-mappings` action for mappings whose pane and sandbox are both gone.
- A `sandboxes` overlay pane, `sbx://<sandbox>/<port>` links with an `open-port`
  action and link handler, `openIn` tab mode, agent reporting to Herdr for
  agents without a screen manifest, and a node shim recorded at install.
- An `install-keybindings` action and `scripts/install-keybindings.sh` that add
  the plugin's key bindings on chords Herdr 0.9 leaves free and refuse to touch
  a binding that sits on one of Herdr's own chords.
