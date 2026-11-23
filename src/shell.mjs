/**
 * Quoting helpers for the command string typed into a Herdr pane shell.
 * The string is executed by the user's interactive shell, so it must be valid
 * for PowerShell on Windows and bash, zsh or fish on Unix. `env KEY=VALUE cmd` is used instead of the
 * `KEY=VALUE cmd` prefix form because fish does not support the latter.
 * @module shell
 */
import { PluginError } from "./errors.mjs";

// A bare word may not start with `=` (zsh expands `=cmd`) or `%` (fish expands `%job`);
// `~` is never in the set, so home-directory expansion cannot happen either.
const SAFE_WORD = /^[A-Za-z0-9_/.:@+,-][A-Za-z0-9_/.:=@%+,-]*$/;

/**
 * Quotes one word for POSIX/fish or PowerShell.
 * @param {unknown} value
 * @param {boolean} powershell
 * @returns {string}
 */
export function shellQuote(value, powershell = false) {
  const text = String(value);
  for (let index = 0; index < text.length; index += 1) {
    const code = text.charCodeAt(index);
    if (code < 32 || code === 127) {
      throw new PluginError("target", "A path or argument contains control characters and cannot be typed into the pane shell.");
    }
    if (!powershell && code === 92) {
      throw new PluginError("target", `A path or argument contains a backslash (${JSON.stringify(text)}), which bash and fish quote differently. Rename it before using it with this plugin.`);
    }
  }
  if (powershell) {
    // PowerShell also recognizes typographic single quotes as delimiters.
    return "'" + text.replace(/['\u2018-\u201b]/g, (quote) => quote + quote) + "'";
  }
  if (text === "") {
    return "''";
  }
  if (SAFE_WORD.test(text)) {
    return text;
  }
  return `'${text.replaceAll("'", "'\\''")}'`;
}

/**
 * Builds the single-line command Herdr submits into a pane.
 * @param {{argv: string[], env?: Record<string, string>}} input
 * @returns {string}
 */
export function buildPaneCommand({ argv, env = {} }, platform = process.platform) {
  if (!Array.isArray(argv) || argv.length === 0) {
    throw new PluginError("startup", "buildPaneCommand needs a non-empty argv.");
  }
  const entries = Object.entries(env);
  for (const [key] of entries) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
      throw new PluginError("startup", `Invalid environment variable name for the pane command: ${key}`);
    }
  }
  if (platform === "win32") {
    const invoke = "& " + argv.map((word) => shellQuote(word, true)).join(" ");
    if (!entries.length) return invoke;
    const save = entries.map(([key]) => `$old_${key} = $env:${key}`).join("; ");
    const set = entries.map(([key, value]) => `$env:${key} = ${shellQuote(value, true)}`).join("; ");
    const restore = entries.map(([key]) => `$env:${key} = $old_${key}`).join("; ");
    return `& { ${save}; try { ${set}; ${invoke} } finally { ${restore} } }`;
  }
  const assignments = entries.map(([key, value]) => `${key}=${shellQuote(value)}`);
  const words = argv.map((word) => shellQuote(word));
  return assignments.length > 0 ? ["env", ...assignments, ...words].join(" ") : words.join(" ");
}
