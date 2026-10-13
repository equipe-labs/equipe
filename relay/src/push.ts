// Sends pushes through APNs (token auth) and FCM HTTP v1 (service account). The relay
// never opens `data`: it rides along as an opaque string for the app to open.

import { toB64url, utf8 } from "./encoding";
import type { Env } from "./env";
import type { PushFrame } from "./frames";

export interface PushTarget {
  platform: "apns" | "fcm";
  token: string;
  sandbox: boolean;
}

export type PushMessage = Omit<PushFrame, "type" | "device">;

export interface PushOutcome {
  ok: boolean;
  error?: string;
  /** The platform says the token is dead; forget it. */
  unregistered?: boolean;
}

export async function sendPush(env: Env, target: PushTarget, message: PushMessage): Promise<PushOutcome> {
  try {
    return target.platform === "apns"
      ? await sendApns(env, target, message)
      : await sendFcm(env, target, message);
  } catch (e) {
    return { ok: false, error: `push failed: ${e instanceof Error ? e.message : String(e)}` };
  }
}

/** Forgets cached JWTs and access tokens (tests). */
export function clearPushCaches(): void {
  apnsJwtCache = undefined;
  fcmTokenCache = undefined;
}

// ---- APNs ----

// APNs wants a provider token between 20 and 60 minutes old at most.
const APNS_JWT_LIFETIME_MS = 50 * 60 * 1000;

interface ApnsConfig {
  key: string;
  keyId: string;
  teamId: string;
  topic: string;
}

let apnsJwtCache: { keyId: string; teamId: string; jwt: string; issuedAt: number } | undefined;

function apnsConfig(env: Env): ApnsConfig | null {
  const { APNS_KEY: key, APNS_KEY_ID: keyId, APNS_TEAM_ID: teamId, APNS_TOPIC: topic } = env;
  return key && keyId && teamId && topic ? { key, keyId, teamId, topic } : null;
}

async function sendApns(env: Env, target: PushTarget, message: PushMessage): Promise<PushOutcome> {
  const config = apnsConfig(env);
  if (!config) return { ok: false, error: "APNs is not configured on this relay" };

  const host = target.sandbox ? "api.sandbox.push.apple.com" : "api.push.apple.com";
  const headers: Record<string, string> = {
    authorization: `bearer ${await apnsJwt(config)}`,
    "apns-push-type": "alert",
    "apns-topic": config.topic,
    "content-type": "application/json",
  };
  if (message.collapse) headers["apns-collapse-id"] = message.collapse;

  const response = await fetch(`https://${host}/3/device/${encodeURIComponent(target.token)}`, {
    method: "POST",
    headers,
    body: JSON.stringify(apnsPayload(message)),
  });
  if (response.ok) return { ok: true };

  const reason = await jsonField(response, (body) => (body as { reason?: unknown }).reason);
  return {
    ok: false,
    error: `APNs ${response.status}${typeof reason === "string" ? ` ${reason}` : ""}`,
    unregistered: response.status === 410 || reason === "Unregistered",
  };
}

function apnsPayload(message: PushMessage) {
  return {
    aps: {
      alert: { title: message.title, body: message.body },
      "mutable-content": 1,
      category: message.category,
      sound: "default",
    },
    data: message.data,
  };
}

async function apnsJwt(config: ApnsConfig): Promise<string> {
  const now = Date.now();
  const cached = apnsJwtCache;
  if (
    cached &&
    cached.keyId === config.keyId &&
    cached.teamId === config.teamId &&
    now - cached.issuedAt < APNS_JWT_LIFETIME_MS
  ) {
    return cached.jwt;
  }
  const key = await crypto.subtle.importKey(
    "pkcs8",
    pemToDer(config.key),
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["sign"],
  );
  const jwt = await signJwt(
    { alg: "ES256", kid: config.keyId },
    { iss: config.teamId, iat: Math.floor(now / 1000) },
    key,
    { name: "ECDSA", hash: "SHA-256" },
  );
  apnsJwtCache = { keyId: config.keyId, teamId: config.teamId, jwt, issuedAt: now };
  return jwt;
}

// ---- FCM ----

const GOOGLE_TOKEN_URL = "https://oauth2.googleapis.com/token";
const FCM_SCOPE = "https://www.googleapis.com/auth/firebase.messaging";

interface ServiceAccount {
  project_id: string;
  client_email: string;
  private_key: string;
}

let fcmTokenCache: { email: string; token: string; expiresAt: number } | undefined;

function fcmConfig(env: Env): ServiceAccount | null {
  if (!env.FCM_SERVICE_ACCOUNT) return null;
  try {
    const account = JSON.parse(env.FCM_SERVICE_ACCOUNT) as Partial<ServiceAccount>;
    const { project_id, client_email, private_key } = account;
    if (typeof project_id === "string" && typeof client_email === "string" && typeof private_key === "string") {
      return { project_id, client_email, private_key };
    }
  } catch {
    // Treated as not configured below.
  }
  return null;
}

async function sendFcm(env: Env, target: PushTarget, message: PushMessage): Promise<PushOutcome> {
  const account = fcmConfig(env);
  if (!account) return { ok: false, error: "FCM is not configured on this relay" };

  const data: Record<string, string> = {
    title: message.title,
    body: message.body,
    category: message.category,
    data: message.data,
  };
  if (message.collapse) data.collapse = message.collapse;

  const response = await fetch(
    `https://fcm.googleapis.com/v1/projects/${encodeURIComponent(account.project_id)}/messages:send`,
    {
      method: "POST",
      headers: {
        authorization: `Bearer ${await fcmAccessToken(account)}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({ message: { token: target.token, data, android: { priority: "HIGH" } } }),
    },
  );
  if (response.ok) return { ok: true };
  if (response.status === 401) fcmTokenCache = undefined;

  const errorCode = await jsonField(response, fcmErrorCode);
  return {
    ok: false,
    error: `FCM ${response.status}${errorCode ? ` ${errorCode}` : ""}`,
    unregistered: errorCode === "UNREGISTERED",
  };
}

/** The `errorCode` of FCM's error details, or its `status` when there is none. */
function fcmErrorCode(body: unknown): string | undefined {
  const error = (body as { error?: { status?: string; details?: { errorCode?: string }[] } }).error;
  return error?.details?.find((d) => d.errorCode)?.errorCode ?? error?.status;
}

async function fcmAccessToken(account: ServiceAccount): Promise<string> {
  const now = Date.now();
  const cached = fcmTokenCache;
  if (cached && cached.email === account.client_email && now < cached.expiresAt) return cached.token;

  const key = await crypto.subtle.importKey(
    "pkcs8",
    pemToDer(account.private_key),
    { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const iat = Math.floor(now / 1000);
  const assertion = await signJwt(
    { alg: "RS256", typ: "JWT" },
    { iss: account.client_email, scope: FCM_SCOPE, aud: GOOGLE_TOKEN_URL, iat, exp: iat + 3600 },
    key,
    { name: "RSASSA-PKCS1-v1_5" },
  );
  const response = await fetch(GOOGLE_TOKEN_URL, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ grant_type: "urn:ietf:params:oauth:grant-type:jwt-bearer", assertion }),
  });
  if (!response.ok) throw new Error(`Google OAuth ${response.status}`);
  const { access_token, expires_in } = (await response.json()) as { access_token?: string; expires_in?: number };
  if (typeof access_token !== "string") throw new Error("Google OAuth answered without an access token");

  // Renew a minute early so a token never expires on the way to FCM.
  const lifetimeMs = (typeof expires_in === "number" ? expires_in : 3600) * 1000 - 60_000;
  fcmTokenCache = { email: account.client_email, token: access_token, expiresAt: now + lifetimeMs };
  return access_token;
}

// ---- shared ----

async function signJwt(
  header: object,
  claims: object,
  key: CryptoKey,
  algorithm: Parameters<SubtleCrypto["sign"]>[0],
): Promise<string> {
  const input = `${toB64url(JSON.stringify(header))}.${toB64url(JSON.stringify(claims))}`;
  // WebCrypto's ECDSA signature is already r || s, the form JWS wants.
  const signature = await crypto.subtle.sign(algorithm, key, utf8(input));
  return `${input}.${toB64url(signature)}`;
}

/** DER bytes of a PEM key; tolerates a secret pasted with literal `\n`s. */
function pemToDer(pem: string): Uint8Array {
  const b64 = pem.replace(/-----[^-]+-----/g, "").replace(/\\n|\s/g, "");
  return Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
}

async function jsonField(response: Response, pick: (body: unknown) => unknown): Promise<unknown> {
  try {
    return pick(await response.json());
  } catch {
    return undefined;
  }
}
