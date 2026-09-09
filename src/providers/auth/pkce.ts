import { createHash, randomBytes } from "node:crypto";

function base64url(buf: Buffer): string {
  return buf.toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

export interface Pkce {
  verifier: string;
  challenge: string;
}

/** Generate a PKCE verifier/challenge pair (S256), matching the OAuth spec. */
export function generatePkce(): Pkce {
  const verifier = base64url(randomBytes(64));
  const challenge = base64url(createHash("sha256").update(verifier).digest());
  return { verifier, challenge };
}

/** A random URL-safe state token for CSRF protection on the OAuth callback. */
export function randomState(): string {
  return randomBytes(16).toString("hex");
}
