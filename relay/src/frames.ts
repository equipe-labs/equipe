// The JSON frames each kind of socket may send, and their validation. Anything that
// doesn't match is answered with an `error` frame and dropped.

export const MAX_FRAME_BYTES = 1024 * 1024;
export const MAX_LABEL_CHARS = 200;

export type DaemonFrame =
  | { type: "send"; device: string; data: string }
  | { type: "pair_open"; slot: string; expires: number }
  | { type: "pair_send"; slot: string; data: string }
  | { type: "pair_close"; slot: string }
  | { type: "device_add"; device: string; token_hash: string }
  | { type: "device_remove"; device: string }
  | PushFrame;

export interface PushFrame {
  type: "push";
  device: string;
  title: string;
  body: string;
  category: string;
  data: string;
  collapse?: string;
}

export type DeviceFrame =
  | { type: "send"; data: string }
  | { type: "push_token"; platform: "apns" | "fcm"; token: string; sandbox?: boolean };

export type PairFrame = { type: "send"; data: string };

export class FrameError extends Error {}

interface Check {
  ok: (value: unknown) => boolean;
  want: string;
}

const isString = (v: unknown): v is string => typeof v === "string";
const string: Check = { ok: isString, want: "a string" };
const hex = (bytes: number): Check => ({
  ok: (v) => isString(v) && new RegExp(`^[0-9a-f]{${bytes * 2}}$`).test(v),
  want: `${bytes * 2} lowercase hex characters`,
});
const shortString = (max: number): Check => ({
  ok: (v) => isString(v) && v.length <= max,
  want: `a string of at most ${max} characters`,
});
const unixMs: Check = { ok: (v) => Number.isSafeInteger(v), want: "a Unix time in milliseconds" };
const pushToken: Check = {
  ok: (v) => isString(v) && v.length > 0 && v.length <= 4096,
  want: "a non-empty string",
};
const platform: Check = { ok: (v) => v === "apns" || v === "fcm", want: `"apns" or "fcm"` };
const optional = (check: Check): Check => ({
  ok: (v) => v === undefined || check.ok(v),
  want: check.want,
});
const boolean: Check = { ok: (v) => typeof v === "boolean", want: "a boolean" };

type Schemas = Record<string, Record<string, Check>>;

const id = hex(16);

export const DAEMON_FRAMES: Schemas = {
  send: { device: id, data: string },
  pair_open: { slot: id, expires: unixMs },
  pair_send: { slot: id, data: string },
  pair_close: { slot: id },
  device_add: { device: id, token_hash: hex(32) },
  device_remove: { device: id },
  push: {
    device: id,
    title: shortString(MAX_LABEL_CHARS),
    body: shortString(MAX_LABEL_CHARS),
    category: shortString(MAX_LABEL_CHARS),
    data: string,
    // apns-collapse-id is at most 64 bytes.
    collapse: optional(shortString(64)),
  },
};

export const DEVICE_FRAMES: Schemas = {
  send: { data: string },
  push_token: { platform, token: pushToken, sandbox: optional(boolean) },
};

export const PAIR_FRAMES: Schemas = {
  send: { data: string },
};

export function parseFrame<T>(message: string | ArrayBuffer, schemas: Schemas): T {
  if (typeof message !== "string") throw new FrameError("frames must be text");
  // Frames are JSON around base64url, so characters and bytes are nearly the same.
  if (message.length > MAX_FRAME_BYTES) throw new FrameError("frame is too large");

  let frame: unknown;
  try {
    frame = JSON.parse(message);
  } catch {
    throw new FrameError("frame is not valid JSON");
  }
  if (typeof frame !== "object" || frame === null || Array.isArray(frame)) {
    throw new FrameError("frame must be a JSON object");
  }
  const fields = frame as Record<string, unknown>;
  const type = fields.type;
  if (!isString(type) || !Object.hasOwn(schemas, type)) {
    throw new FrameError(`unknown frame type ${JSON.stringify(type)}`);
  }
  for (const [name, check] of Object.entries(schemas[type]!)) {
    if (!check.ok(fields[name])) throw new FrameError(`${type}: "${name}" must be ${check.want}`);
  }
  return frame as T;
}
