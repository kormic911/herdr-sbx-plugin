import assert from "node:assert/strict";
import { test } from "node:test";
import { workspaceExecArgs } from "../src/lifecycle.mjs";
import { openerCommand } from "../src/open.mjs";
import { buildPaneCommand } from "../src/shell.mjs";

test("PowerShell pane arguments stay literal and the agent override is restored", () => {
  const argv = [String.raw`C:\Program Files\node.exe`, String.raw`D:\O'Brien\$work & tools`];
  const command = buildPaneCommand({ argv, env: { HERDR_AGENT: "codex" } }, "win32");
  assert.equal(command, String.raw`& { $old_HERDR_AGENT = $env:HERDR_AGENT; try { $env:HERDR_AGENT = 'codex'; & 'C:\Program Files\node.exe' 'D:\O''Brien\$work & tools' } finally { $env:HERDR_AGENT = $old_HERDR_AGENT } }`);
  assert.throws(() => buildPaneCommand({ argv: ["a\nb"] }, "win32"), /control characters/);
});

test("Windows guest commands start relative to the primary workspace", () => {
  const entry = { localPath: String.raw`D:\My Project`, sandboxName: "example" };
  const options = { argv: ["bash", "-l"], tty: true };
  assert.deepEqual(workspaceExecArgs(entry, options, "win32"), ["exec", "--tty", "example", "--", "bash", "-l"]);
  entry.workdir = String.raw`D:\My Project\sub dir\$files`;
  assert.deepEqual(workspaceExecArgs(entry, options, "win32"), ["exec", "--tty", "example", "--", "sh", "-c", 'cd "./$1" && shift && exec "$@"', "sh", "sub dir/$files", "bash", "-l"]);
  assert.deepEqual(workspaceExecArgs({ ...entry, workspaceMode: "clone" }, options, "win32"), ["exec", "--tty", "example", "--", "bash", "-l"]);
});

test("Windows port links use the registered protocol handler", () => {
  assert.equal(openerCommand({}, "win32"), "rundll32.exe");
  assert.equal(openerCommand({ HERDR_SBX_OPENER: "browser.exe" }, "win32"), "browser.exe");
});
