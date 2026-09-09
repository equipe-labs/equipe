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

const VERSION = "0.0.1";

const HELP = `equipe v${VERSION} — a full software squad of AI agents.

USAGE
  equipe <command> [options]

COMMANDS
  login      Authenticate an engine (default: ChatGPT/Codex subscription OAuth)
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

function notImplemented(name: string): void {
  console.error(`equipe: '${name}' is scaffolded but not implemented yet.`);
  console.error("Follow along: this is an early skeleton. See docs/ARCHITECTURE.md.");
  process.exitCode = 2;
}

function main(argv: string[]): void {
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

main(process.argv.slice(2));
