/** Local credential storage: ~/.equipe/credentials/<provider>.json (0600). */

import { chmodSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { AuthCredentials, ProviderId } from "../../core/types.js";

export function equipeHome(): string {
  return process.env.EQUIPE_HOME?.trim() || join(homedir(), ".equipe");
}

function credentialsDir(): string {
  return join(equipeHome(), "credentials");
}

function credentialPath(provider: ProviderId): string {
  return join(credentialsDir(), `${provider}.json`);
}

export function saveCredentials(cred: AuthCredentials): string {
  const dir = credentialsDir();
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  const path = credentialPath(cred.provider);
  writeFileSync(path, JSON.stringify(cred, null, 2), { mode: 0o600 });
  chmodSync(path, 0o600); // enforce even if the file pre-existed with looser perms
  return path;
}

export function loadCredentials(provider: ProviderId): AuthCredentials | null {
  try {
    return JSON.parse(readFileSync(credentialPath(provider), "utf8")) as AuthCredentials;
  } catch {
    return null;
  }
}
