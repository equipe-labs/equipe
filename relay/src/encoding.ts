const encoder = new TextEncoder();

export function randomBytes(length: number): Uint8Array {
  return crypto.getRandomValues(new Uint8Array(length));
}

export function toHex(bytes: ArrayBuffer | Uint8Array): string {
  return Array.from(new Uint8Array(bytes), (b) => b.toString(16).padStart(2, "0")).join("");
}

export function fromHex(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

/** base64url without padding. */
export function toB64url(input: ArrayBuffer | Uint8Array | string): string {
  const bytes = typeof input === "string" ? encoder.encode(input) : new Uint8Array(input);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function utf8(text: string): Uint8Array {
  return encoder.encode(text);
}

export async function sha256(text: string): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", encoder.encode(text)));
}

export async function sha256Hex(text: string): Promise<string> {
  return toHex(await sha256(text));
}

/** Whether SHA-256(`token`) equals `expectedHex`, without leaking where they differ. */
export async function tokenMatches(token: string, expectedHex: string): Promise<boolean> {
  const actual = await sha256(token);
  const expected = fromHex(expectedHex);
  return actual.length === expected.length && crypto.subtle.timingSafeEqual(actual, expected);
}
