import { exports } from "cloudflare:workers";
import { expect } from "vitest";
import { randomBytes, sha256Hex, toB64url, toHex } from "../src/encoding";

const BASE = "https://relay.test";

export type Frame = Record<string, any>;

export function newId(): string {
  return toHex(randomBytes(16));
}

export function newToken(): string {
  return toB64url(randomBytes(32));
}

/** A fresh client address, so rate limits don't leak between tests. */
export function newIp(): string {
  const [a, b, c] = randomBytes(3);
  return `10.${a}.${b}.${c}`;
}

export function request(path: string, init?: RequestInit): Promise<Response> {
  return exports.default.fetch(new Request(BASE + path, init));
}

export async function register(ip = newIp()): Promise<{ id: string; token: string }> {
  const response = await request("/v1/daemons", { method: "POST", headers: { "CF-Connecting-IP": ip } });
  expect(response.status).toBe(201);
  return response.json();
}

export function upgrade(path: string, token?: string): Promise<Response> {
  const headers: Record<string, string> = { Upgrade: "websocket" };
  if (token) headers.Authorization = `Bearer ${token}`;
  return request(path, { headers });
}

export async function open(path: string, token?: string): Promise<Client> {
  const response = await upgrade(path, token);
  expect(response.status).toBe(101);
  return new Client(response.webSocket!);
}

export const daemonPath = (id: string) => `/v1/daemons/${id}/connect`;
export const devicePath = (id: string, device: string) => `/v1/daemons/${id}/devices/${device}/connect`;
export const pairPath = (id: string, slot: string) => `/v1/daemons/${id}/pair/${slot}`;

/** The client end of a relay socket, collecting what the relay sends. */
export class Client {
  private inbox: Frame[] = [];
  private wake: (() => void) | undefined;
  readonly closed: Promise<{ code: number; reason: string }>;

  constructor(readonly ws: WebSocket) {
    ws.addEventListener("message", (event) => {
      this.inbox.push(JSON.parse(event.data as string));
      this.wake?.();
    });
    this.closed = new Promise((resolve) => {
      ws.addEventListener("close", (event) => resolve({ code: event.code, reason: event.reason }));
    });
    ws.accept();
  }

  send(frame: object | string): void {
    this.ws.send(typeof frame === "string" ? frame : JSON.stringify(frame));
  }

  /** The next frame of `type` (skipping none of the others), waiting for it if need be. */
  async next(type?: string, timeoutMs = 2000): Promise<Frame> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const index = this.inbox.findIndex((f) => type === undefined || f.type === type);
      if (index >= 0) return this.inbox.splice(index, 1)[0]!;
      const left = deadline - Date.now();
      if (left <= 0) throw new Error(`no ${type ?? "frame"} arrived; inbox: ${JSON.stringify(this.inbox)}`);
      await new Promise<void>((resolve) => {
        this.wake = resolve;
        setTimeout(resolve, left);
      });
    }
  }

  /** Waits a little and checks that nothing (of `type`) arrived. */
  async nothing(type?: string, waitMs = 100): Promise<void> {
    await new Promise((resolve) => setTimeout(resolve, waitMs));
    expect(this.inbox.filter((f) => type === undefined || f.type === type)).toEqual([]);
  }

  /** Returns once the relay has handled everything this client sent so far. */
  async flush(): Promise<void> {
    this.send({ type: "flush" });
    const error = await this.next("error");
    expect(error.message).toContain("flush");
  }

  close(): void {
    this.ws.close(1000, "bye");
  }
}

export async function connectDaemon(id: string, token: string): Promise<Client> {
  return open(daemonPath(id), token);
}

/** A registered daemon, connected, with one device registered (not connected). */
export async function setup() {
  const { id, token } = await register();
  const daemon = await connectDaemon(id, token);
  const device = newId();
  const deviceToken = newToken();
  daemon.send({ type: "device_add", device, token_hash: await sha256Hex(deviceToken) });
  await daemon.flush();
  return { id, token, daemon, device, deviceToken };
}
