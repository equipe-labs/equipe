/**
 * OpenAI ChatGPT (Codex) subscription OAuth.
 *
 * Mechanism (identical to OpenAI's own open-source Codex CLI): authenticate the
 * user's ChatGPT Plus/Pro account with the public Codex OAuth client + PKCE, then
 * call the ChatGPT backend with the resulting token. Equipe identifies itself
 * honestly as a third-party harness via the `originator` parameter — it does not
 * impersonate the official CLI.
 *
 * Endpoints and parameters are grounded in the public Codex CLI / openclaw source.
 */

import type { AuthCredentials } from "../../core/types.js";
import { generatePkce, randomState } from "./pkce.js";

/** Public Codex OAuth client id shipped in OpenAI's open-source Codex CLI. */
export const CODEX_CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann";
export const AUTHORIZE_URL = "https://auth.openai.com/oauth/authorize";
export const TOKEN_URL = "https://auth.openai.com/oauth/token";
export const CODEX_BACKEND_URL = "https://chatgpt.com/backend-api/codex";
export const CALLBACK_PORT = 1455;
export const CALLBACK_PATH = "/auth/callback";
const SCOPE = "openid profile email offline_access";

/** Honest third-party identity — never "codex" / "codex_cli_rs". */
export const EQUIPE_ORIGINATOR = "equipe";

export interface AuthorizationRequest {
  url: string;
  verifier: string;
  state: string;
  redirectUri: string;
}

export function redirectUri(port: number = CALLBACK_PORT): string {
  return `http://localhost:${port}${CALLBACK_PATH}`;
}

/** Build the authorization URL to open in the user's browser. */
export function createAuthorizationRequest(port: number = CALLBACK_PORT): AuthorizationRequest {
  const { verifier, challenge } = generatePkce();
  const state = randomState();
  const uri = redirectUri(port);
  const url = new URL(AUTHORIZE_URL);
  url.searchParams.set("response_type", "code");
  url.searchParams.set("client_id", CODEX_CLIENT_ID);
  url.searchParams.set("redirect_uri", uri);
  url.searchParams.set("scope", SCOPE);
  url.searchParams.set("code_challenge", challenge);
  url.searchParams.set("code_challenge_method", "S256");
  url.searchParams.set("state", state);
  url.searchParams.set("id_token_add_organizations", "true");
  url.searchParams.set("codex_cli_simplified_flow", "true");
  url.searchParams.set("originator", EQUIPE_ORIGINATOR);
  return { url: url.toString(), verifier, state, redirectUri: uri };
}

interface TokenResponse {
  access_token?: string;
  refresh_token?: string;
  expires_in?: number;
}

async function postTokenForm(form: Record<string, string>): Promise<AuthCredentials> {
  const res = await fetch(TOKEN_URL, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams(form),
    signal: AbortSignal.timeout(30_000),
  });
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`token request failed (HTTP ${res.status}): ${text.slice(0, 300)}`);
  }
  let json: TokenResponse;
  try {
    json = JSON.parse(text) as TokenResponse;
  } catch {
    throw new Error("token response was not valid JSON");
  }
  if (!json.access_token) {
    throw new Error(`token response missing access_token`);
  }
  const now = Math.floor(Date.now() / 1000);
  return {
    provider: "openai",
    method: "oauth-subscription",
    access: json.access_token,
    // Prefer a rotated refresh token from the server; fall back to the one we sent.
    refresh: json.refresh_token || form.refresh_token,
    expires: typeof json.expires_in === "number" ? now + json.expires_in : undefined,
    accountId: decodeChatgptAccountId(json.access_token),
  };
}

/** Exchange an authorization code for tokens. */
export function exchangeCode(
  code: string,
  verifier: string,
  uri: string,
): Promise<AuthCredentials> {
  return postTokenForm({
    grant_type: "authorization_code",
    client_id: CODEX_CLIENT_ID,
    code,
    code_verifier: verifier,
    redirect_uri: uri,
  });
}

/** Refresh an access token from a refresh token. */
export function refreshToken(refresh: string): Promise<AuthCredentials> {
  return postTokenForm({
    grant_type: "refresh_token",
    client_id: CODEX_CLIENT_ID,
    refresh_token: refresh,
  });
}

/** Extract the `chatgpt_account_id` claim from an access-token JWT (best effort). */
export function decodeChatgptAccountId(accessToken: string): string | undefined {
  const parts = accessToken.split(".");
  if (parts.length < 2 || !parts[1]) return undefined;
  try {
    const payload = JSON.parse(Buffer.from(parts[1], "base64url").toString("utf8")) as {
      chatgpt_account_id?: unknown;
      "https://api.openai.com/auth"?: { chatgpt_account_id?: unknown };
    };
    const direct = payload.chatgpt_account_id;
    const nested = payload["https://api.openai.com/auth"]?.chatgpt_account_id;
    const id = typeof direct === "string" ? direct : nested;
    return typeof id === "string" && id.length > 0 ? id : undefined;
  } catch {
    return undefined;
  }
}

/** Identity headers for calls to the ChatGPT backend — honest, non-impersonating. */
export function codexIdentityHeaders(
  cred: AuthCredentials,
  version: string,
): Record<string, string> {
  return {
    Authorization: `Bearer ${cred.access}`,
    originator: EQUIPE_ORIGINATOR,
    "User-Agent": `equipe/${version}`,
    ...(cred.accountId ? { "ChatGPT-Account-ID": cred.accountId } : {}),
  };
}
