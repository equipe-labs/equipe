/**
 * Minimal client for the ChatGPT (Codex) backend.
 *
 * Phase 1a: prove the subscription token reaches the backend. This owns token
 * freshness (refresh on expiry) and a lightweight `listModels` probe. Model
 * completions (the Responses API path) build on top of this later.
 */

import type { AuthCredentials } from "../core/types.js";
import { CODEX_BACKEND_URL, codexIdentityHeaders, refreshToken } from "./auth/codex-oauth.js";
import { saveCredentials } from "./auth/store.js";

/** Refresh with this much headroom before the token actually expires. */
const REFRESH_SKEW_SECONDS = 60;

/** Return credentials with a valid access token, refreshing + persisting if stale. */
export async function ensureFreshCredentials(cred: AuthCredentials): Promise<AuthCredentials> {
  const now = Math.floor(Date.now() / 1000);
  const stale = cred.expires !== undefined && cred.expires - now <= REFRESH_SKEW_SECONDS;
  if (!stale || !cred.refresh) return cred;
  const refreshed = await refreshToken(cred.refresh);
  saveCredentials(refreshed);
  return refreshed;
}

export interface ModelsResult {
  models: string[];
  raw: unknown;
}

function extractModelIds(json: unknown): string[] {
  const list = Array.isArray(json)
    ? json
    : Array.isArray((json as { data?: unknown })?.data)
      ? (json as { data: unknown[] }).data
      : Array.isArray((json as { models?: unknown })?.models)
        ? (json as { models: unknown[] }).models
        : [];
  const ids: string[] = [];
  for (const item of list) {
    if (typeof item === "string") {
      ids.push(item);
    } else if (item && typeof item === "object") {
      const rec = item as Record<string, unknown>;
      const id = rec.id ?? rec.slug ?? rec.name;
      if (typeof id === "string") ids.push(id);
    }
  }
  return ids;
}

/** Probe the backend for the account's available models. Proves the token works. */
export async function listModels(
  cred: AuthCredentials,
  clientVersion: string,
): Promise<ModelsResult> {
  const url = `${CODEX_BACKEND_URL}/models?client_version=${encodeURIComponent(clientVersion)}`;
  const res = await fetch(url, {
    method: "GET",
    headers: codexIdentityHeaders(cred, clientVersion),
    signal: AbortSignal.timeout(30_000),
  });
  if (res.status === 401 || res.status === 403) {
    throw new Error(
      `backend rejected the token (HTTP ${res.status}). Re-run \`equipe login\`. ` +
        `A subscription (ChatGPT Plus/Pro) is required.`,
    );
  }
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`models request failed (HTTP ${res.status}): ${text.slice(0, 200)}`);
  }
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    throw new Error("models response was not valid JSON");
  }
  return { models: extractModelIds(raw), raw };
}
