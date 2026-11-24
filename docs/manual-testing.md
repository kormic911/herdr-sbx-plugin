# Manual test plan

The automated tests exercise the plugin against fake `sbx` and `herdr`
executables. This checklist covers what only a real host can verify: the exact
`sbx` command syntax, the shape of `sbx ls --json`, Herdr's pane handling, and
the agent's behaviour inside the VM. Work through it top to bottom; each step
says what to run, what should happen, and what to write down when it does not.

## Automated native tests

Run `cargo test --release --locked` for the native actions, confirmation popup and bridges.
To test deletion against Docker Sandboxes on a signed-in host, run:

```sh
cargo test --release --locked --test management live_create_stop_delete -- --ignored --exact --nocapture
```

This creates a uniquely named sandbox in a temporary workspace, stops it, confirms deletion
through the native popup, and checks the daemon inventory. The test drives the pane and popup
using a fixture Herdr command. It prints the sandbox name before creation for recovery after interruption.

To exercise the worktree removal hook against a disposable sandbox, run
`cargo test --release --locked --test management live_create_worktree_cleanup -- --ignored --exact --nocapture`.
The harness sends a worktree event through the platform manifest command and verifies the removal in the real daemon.

## Windows smoke test

From a native PowerShell pane, check the Windows prerequisites in the README.
From this fork’s checkout run:

```powershell
cargo build --release --locked
herdr plugin link .
.\scripts\run-action.ps1 doctor
```

Check the result for `"ok":true` and an `executable` path ending in `target\release\herdr-sbx.exe`. For the Rust launch check,
focus a PowerShell pane in a project workspace and invoke
`herdr plugin action invoke start-agent --plugin sbx.sandbox`.
Wait for the agent prompt, then invoke `info` with that pane focused.
The mapping should be ready. Invoke `open-shell` from this workspace. In the new pane, run `pwd` and then `exit`.
Open a project with a space
and an apostrophe in its name. Start an agent from a nested directory, then use
`open-shell` and run `pwd` inside the guest. Verify that both sessions see the
nested directory. While the shell is open, verify that deletion is refused.
Exit the sessions, reconnect, and check that cancelling the deletion popup
keeps the sandbox. In clone mode, make a commit and verify that `fetch-changes`
makes it available on the host. Check that `open-port` opens the published URL
in the Windows browser and `install-keybindings` reports the active Herdr config.

## Helper

Actions return as soon as Herdr has started them; the outcome lands in the
plugin log. `scripts/run-action.sh` invokes an action and waits for its result
line. Define a shorthand in every pane you test from, or add it to your shell
rc file:

```bash
sbxrun() { sh /path/to/herdr-sbx-plugin/scripts/run-action.sh "$@"; }
```

Pane-scoped actions use the focused pane, or the workspace's only sandbox when
the focused pane has none. With several sandboxes in a workspace, run the
action from the pane of the sandbox you mean.

## 0. Prerequisites

Run these on the host, not inside a sandbox.

```bash
sw_vers || cat /etc/os-release          # macOS 14+ on Apple silicon, or Ubuntu 24.04+ with /dev/kvm
herdr --version                         # 0.9.0 or newer
sbx version --json                      # 0.42 or newer
sbx daemon status                       # running; otherwise: sbx daemon start
sbx secret ls                           # an entry for your agent's API (anthropic, openai, ...)
sbx policy ls                           # a preset must be chosen; deny-all needs: sbx policy allow network api.anthropic.com
```

If `sbx policy ls` shows no preset yet, choose one now; the plugin creates
sandboxes from a process that cannot answer the interactive prompt:

```bash
sbx policy init balanced
```

If `sbx secret ls` is empty:

```bash
sbx secret set anthropic                          # Claude Code with an API key; a subscription uses /login inside Claude Code
sbx secret set github --command 'gh auth token'  # optional, for git pushes from the VM
```

Create a throwaway repository to use as the workspace:

```bash
mkdir -p ~/tmp/sbx-plugin-demo && cd ~/tmp/sbx-plugin-demo
git init -q && printf 'demo\n' > README.md && git add README.md && git commit -q -m init
```

## 1. Link the plugin and check the manifest

```bash
cargo build --release --locked --manifest-path /path/to/herdr-sbx-plugin/Cargo.toml
herdr plugin link /path/to/herdr-sbx-plugin
herdr plugin list
herdr plugin action list --plugin sbx.sandbox
```

Check that `sbx.sandbox` is enabled and lists fifteen actions. The examples below
use the default Claude Code agent in mount mode. If you use another agent, select
it in `edit-config` and substitute its name and command in the checks.

## 2. doctor

```bash
sbxrun doctor
```

Check the `HERDR_SANDBOX_RESULT:` line for `"ok":true`, the native
`executable` path, and the `version` and `daemon` fields.

If `ok` is false: `errorKind: "startup"` means `sbx` is not on the `PATH` Herdr
started with (set `sbxBin` in `config.json`); `"daemon"` means `sandboxd` is not
running.

## 3. start-agent from the TUI

Herdr has no menu for plugin actions; they run from a key binding or from the
CLI. Add the plugin's five bindings and reload:

```bash
sbxrun install-keybindings   # check added, existing and warnings for the five shortcuts
```

Press prefix then Shift+C, edit a field, save with S and reopen to verify the value.

The same thing by hand, for one binding, is a block in Herdr's `config.toml`
followed by a reload:

```toml
[[keys.command]]
key = "prefix+shift+a"
type = "plugin_action"
command = "sbx.sandbox.start-agent"
description = "start an agent in a Docker Sandbox"
```

```bash
herdr config check && herdr server reload-config
```

Start Herdr and give it a workspace rooted at the demo repository. A workspace
rooted at your home directory is refused by the plugin.

```bash
cd ~/tmp/sbx-plugin-demo && herdr
# in a second terminal, if the first pane's directory is not the demo repo:
herdr workspace create --cwd ~/tmp/sbx-plugin-demo --label demo --focus
```

Press `ctrl+b`, release, then `shift+a` in the demo workspace (`prefix+?`
shows the binding). Or trigger it from the second terminal, which uses the
focused workspace as context:

```bash
herdr plugin action invoke start-agent --plugin sbx.sandbox
```

With the default split and agent settings, a pane opens to the right titled
`sbx claude-code <short id>`. On first creation, the bridge prints the sandbox
name and workspace path. Once the image is ready, the pane shows the launch
message and the agent interface. Check that Herdr detects the agent.

From another terminal:

```bash
sbx ls
sbx ls --json | head -c 600; echo
```

Write down the `sbx ls --json` output. The plugin's parser accepts a bare
array, `{"sandboxes": [...]}` and a map keyed by name; anything else makes
`list-sandboxes` report an error and this is the single most likely mismatch.

What to record if it fails:

- The pane shows an `unknown flag` or `unknown shorthand flag` error from
  `sbx exec`: copy the full command line. The bridge separates agent flags
  from `sbx` flags with `--`; this would mean the real CLI rejects that.
- The bridge reports that the agent command is unavailable: the template does not put
  the agent on `PATH` for a login shell. Run step 7 and check
  `command -v claude` by hand.
- Claude Code starts but Herdr's status never changes: run `info` (step 5)
  and check `herdrDetectionKind` is `claude`; then confirm the pane's
  foreground process carries `HERDR_AGENT` (`herdr agent explain <pane_id>`).

## 4. Work inside the sandbox

Ask the agent to create a file, for example "create hello.txt containing hi".
On the host:

```bash
ls ~/tmp/sbx-plugin-demo          # hello.txt is there immediately (mount mode)
git -C ~/tmp/sbx-plugin-demo status
```

Exit the agent and check that the pane returns to its host shell prompt.

## 5. info and list-sandboxes

With the agent pane focused, run the `info` action, then `list-sandboxes`.

Check that `info` reports the sandbox name, `localPath`, `workdir`, the resolved
`agent.launchArgv`, and the live sandbox status. Find the same sandbox in the
`mappings` array from `list-sandboxes`. A missing sandbox has `exists: false`.
If Docker inventory could not be read, `exists` is null and `sandboxError`
explains the failure.

## 5b. The overlay and port links

Open `sandboxes` (prefix then Shift+O). Select the demo sandbox with ↑/↓.
Compare the Docker status with `sbx ls` and the mapping status with the open
Herdr pane. Check the workspace and pane labels in the details panel. In a
narrow terminal, press `i` to switch views. Press `q` to close the overlay.

On the host, substitute the demo sandbox name and an available host port:

```sh
sbx port publish <sandbox-name> 127.0.0.1:8080:3000/tcp
sbxrun info
```

Check that `ports` maps guest port 3000 to host port 8080. Reopen the overlay
and Ctrl-click the port link in the details panel. The browser should open
`http://localhost:8080`. A connection error is expected if nothing is listening
on guest port 3000. Windows opens the HTTP link directly. On Linux and macOS,
Herdr handles the `sbx://` link through `open-port`.

## 6. reconnect

Focus the agent pane (agent exited) and run `reconnect`. Expect the agent to
start again in the same sandbox with `hello.txt` still present. Running
`reconnect` while the agent is still up must be refused with
`errorKind: "conflict"`. Exit the agent before the shell checks.

## 7. open-shell

Run `open-shell` from the agent pane. Expect a pane below titled
`sbx shell herdr-claude-code-...` with a bash login shell inside the VM.

```bash
pwd                    # the demo repository path
command -v claude      # the agent binary
env | grep -c SANDBOX  # sandbox environment variables are present
```

Leave this shell open for the next check.

## 7b. Ownership guards

While the shell pane from step 7 is still open, run `forget-mapping` from the
agent pane. Expect `errorKind: "conflict"` mentioning an open-shell session
and no popup. With just the sandbox shell attached, reconnect the agent. While
the agent is running, `forget-mapping`, `replace-sandbox` and a second
`reconnect` must all return `conflict`. Exit both sessions, then invoke
`forget-mapping` from the mapped pane and
cancel the popup. Run `sbxrun doctor` to find `stateDir`. Its `panes/`
directory should contain one JSON file per mapping.

## 8. stop, then reconnect

Reconnect the agent and invoke `stop` while it is running. Expect a
`conflict`. Exit the agent and run `stop` again. Check that `sbx ls` shows
the sandbox stopped. Run `reconnect` and check that the same sandbox starts.

## 9. replace-sandbox

Exit the agent, then run `replace-sandbox`. A popup asks you to type `DELETE`.

- Type anything else first: the popup prints `Cancelled`, the action result
  has `errorKind: "cancelled"`, and `sbx ls` is unchanged.
- Run it again and press Ctrl-C in the popup: same outcome.
- Run it again and type `DELETE`: the old sandbox disappears from `sbx ls`, a
  new name appears, and the agent starts in the same pane.

## 10. forget-mapping

Exit the agent and run `forget-mapping`, confirm with `DELETE`. Expect the
sandbox gone from `sbx ls` and its mapping absent from `list-sandboxes`.

## 11. The worktree.removed hook

```bash
herdr worktree create --cwd ~/tmp/sbx-plugin-demo --branch hook-test --focus
```

In the new worktree workspace run `start-agent`, wait for the agent, exit it,
then remove the worktree:

```bash
herdr workspace list                      # find the worktree workspace id
herdr worktree remove --workspace <id> --force
#   a popup asks to type DELETE; type it
sbx ls                                    # the hook deleted the sandbox
herdr plugin log list --plugin sbx.sandbox --limit 1
```

Expect a toast "Docker Sandboxes removed" and a log entry from the `worktree.removed` hook
with `Forgot mapping for pane ...`. To keep sandboxes on worktree removal, set
`"cleanupOnWorktreeRemoved": false` and repeat: the log then says
`leaving sandboxes alone`.

## 11b. Tab mode

In `edit-config`, set `openIn` to `tab` and save.

Run `start-agent`. Expect a new tab in the workspace, labelled like the pane,
with the agent inside; `reconnect` from that tab works as before.

## 12. Clone mode and fetch-changes

In `edit-config`, set `workspaceMode` to `clone` and save.

Run `start-agent` in the demo repository. In the agent pane, note the
directory the agent reports as its working directory, then ask the agent to
commit a change on a new branch called `agent-work`. Exit the agent and run
`fetch-changes` from that pane.

```bash
git -C ~/tmp/sbx-plugin-demo branch -r    # sandbox-herdr-claude-code-.../agent-work
```

Record the working directory the agent started in. The plugin passes no
`--workdir` in clone mode; if the agent did not start inside the clone, that
assumption needs a fix. Then run `stop` and `fetch-changes` again: the result
must report `"transport": "bundle"`, because the git remote `sbx` registers
only lives while an `sbx run` session is attached, and the branches must still
arrive. Each `keep` entry names a local branch that would preserve commits the
host does not reach yet.

## 13. Driving the plugin from a script

With a Herdr workspace focused on the demo repository:

```bash
sbxrun start-agent
```

Expect the marker line with `paneId`, `sandboxName`, `localPath` and
`workdir`. The action returns as soon as the bridge is typed into the pane;
the agent itself starts a little later.

## 14. Failure classification

Exit the agent from step 13. Use its disposable sandbox to check how the plugin handles a sandbox removed outside Herdr.

```bash
sbx rm --force <sandbox-name-from-step-13>
```

Run `stop` from that pane: expect `errorKind: "not-found"` and
`lifecycleState: "missing"` in `info`. If the kind is `unknown`, copy the
`output` field from the result line; the classifier needs that wording.

```bash
sbx daemon stop
```

Run `doctor`: expect `errorKind: "daemon"`. Start the daemon again afterwards.

## 15. Cleanup

```bash
sbx ls
```

In the overlay, select each sandbox created by this checklist, exit any active
agent or shell, then press `d`. Verify the listed name and type `DELETE`.
For a stale mapping, `d` removes the mapping. Restore any settings changed
during the checks, then unlink the checkout with `herdr plugin unlink sbx.sandbox`.
Remove the throwaway repository when you are finished with it.

## What to report

For every step that deviates: the step number, the `HERDR_SANDBOX_RESULT`
line, stderr from `herdr plugin log list`, the raw `sbx` output, and the
versions from step 0. The three facts that matter most are the `sbx ls --json`
shape (step 3), whether `sbx exec` accepted the `--` separator (step 3), and
the clone-mode working directory (step 12).
