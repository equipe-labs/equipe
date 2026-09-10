#!/usr/bin/env node
/**
 * equipe — CLI entry point.
 *
 * Usage:
 *   equipe login [--provider openai]   Authenticate an engine (default: ChatGPT/Codex OAuth)
 *   equipe init                        Scaffold an equipe.config.jsonc for a repo
 *   equipe roles                       List the built-in squad roles
 *   equipe run [--ticket <id>]         Run the squad orchestrator against the board
 *   equipe board                       Show the current board
 */

import { parseArgs } from "node:util";
import { ROLES } from "./core/types.js";
import { ensureFreshCredentials, listModels } from "./providers/codex-client.js";
import { runCodexLogin } from "./providers/auth/login.js";
import { loadCredentials } from "./providers/auth/store.js";

const VERSION = "0.0.1";

const HELP = `equipe v${VERSION} — a full software squad of AI agents.

USAGE
  equipe <command> [options]

COMMANDS
  login      Authenticate an engine (default: ChatGPT/Codex subscription OAuth)
  status     Verify the login and list models the backend offers
  init       Create an equipe.config.jsonc in the current repo
  roles      List the built-in squad roles
  run        Run the orchestrator against the board
  board      Print the current board state

OPTIONS
  -h, --help       Show this help
  -v, --version    Show version

Docs: https://github.com/<owner>/equipe   (ACP-first · OpenAI OAuth by default)
`;

function cmdRoles(): void {
  console.log("Built-in squad roles:\n");
  for (const r of ROLES) {
    console.log(`  ${r}`);
  }
}

async function cmdLogin(provider: string | undefined): Promise<void> {
  const p = (provider ?? "openai").toLowerCase();
  if (p !== "openai") {
    console.error(`login: provider '${p}' is not supported yet (only 'openai' for now).`);
    process.exitCode = 2;
    return;
  }

  const existing = loadCredentials("openai");
  if (existing?.method === "oauth-subscription") {
    console.log("Already signed in to OpenAI (subscription). Re-authenticating…\n");
  }

  console.log("Sign in with your ChatGPT (Codex) subscription — this uses YOUR account.");
  console.log("Equipe identifies itself honestly as a third-party harness (originator: equipe).\n");

  try {
    const cred = await runCodexLogin({
      onUrl: (url) => console.log(`Opening your browser… if it didn't open, visit:\n  ${url}\n`),
    });
    const acct = cred.accountId ? ` (account ${cred.accountId})` : "";
    const exp = cred.expires ? new Date(cred.expires * 1000).toISOString() : "unknown";
    console.log(`\n✓ Signed in to OpenAI via subscription${acct}.`);
    console.log(`  Token stored at ~/.equipe/credentials/openai.json (expires ${exp}).`);
  } catch (err) {
    console.error(`\n✗ Login failed: ${err instanceof Error ? err.message : String(err)}`);
    process.exitCode = 1;
  }
}

async function cmdStatus(): Promise<void> {
  const cred = loadCredentials("openai");
  if (!cred || cred.method !== "oauth-subscription") {
    console.log("Not signed in. Run `equipe login`.");
    process.exitCode = 1;
    return;
  }
  const now = Math.floor(Date.now() / 1000);
  const tokenState =
    cred.expires === undefined
      ? "unknown"
      : cred.expires - now > 0
        ? `valid (${Math.floor((cred.expires - now) / 60)}m left)`
        : "expired (will refresh)";

  console.log("Provider : openai — ChatGPT/Codex subscription");
  if (cred.accountId) console.log(`Account  : ${cred.accountId}`);
  console.log(`Token    : ${tokenState}`);

  try {
    const fresh = await ensureFreshCredentials(cred);
    const { models } = await listModels(fresh, VERSION);
    console.log(`Backend  : ✓ reachable — ${models.length} model(s) available`);
    if (models.length > 0) {
      const shown = models.slice(0, 8).join(", ");
      console.log(`Models   : ${shown}${models.length > 8 ? ", …" : ""}`);
    }
  } catch (err) {
    console.error(`Backend  : ✗ ${err instanceof Error ? err.message : String(err)}`);
    process.exitCode = 1;
  }
}

function notImplemented(name: string): void {
  console.error(`equipe: '${name}' is scaffolded but not implemented yet.`);
  console.error("Follow along: this is an early skeleton. See docs/ARCHITECTURE.md.");
  process.exitCode = 2;
}

async function main(argv: string[]): Promise<void> {
  const { values, positionals } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      help: { type: "boolean", short: "h" },
      version: { type: "boolean", short: "v" },
      provider: { type: "string" },
      ticket: { type: "string" },
    },
  });

  const command = positionals[0];

  if (values.version) {
    console.log(VERSION);
    return;
  }
  if (!command || values.help) {
    console.log(HELP);
    return;
  }

  switch (command) {
    case "roles":
      cmdRoles();
      return;
    case "login":
      await cmdLogin(values.provider);
      return;
    case "status":
      await cmdStatus();
      return;
    case "init":
    case "run":
    case "board":
      notImplemented(command);
      return;
    default:
      console.error(`equipe: unknown command '${command}'\n`);
      console.log(HELP);
      process.exitCode = 1;
  }
}

main(process.argv.slice(2)).catch((err: unknown) => {
  console.error(err instanceof Error ? err.stack ?? err.message : String(err));
  process.exitCode = 1;
});
