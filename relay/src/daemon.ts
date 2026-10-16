// One Durable Object per daemon. It holds the daemon's socket, its devices' sockets and
// any pairing sockets, and passes sealed frames between them without opening them.
//
// Sockets use the WebSocket Hibernation API, so an idle daemon costs nothing. Each socket
// is tagged so it can be found again after the object wakes up:
//   daemon socket   "daemon"
//   device socket   "device", "device:<device id>"
//   pairing socket  "pair", "pair:<slot>"
// and carries a `Peer` attachment saying which of these it is.
//
// Storage:
//   "token_hash"       hex SHA-256 of the daemon token (absent: unknown daemon)
//   "device:<id>"      DeviceRecord
//   "slot:<slot>"      Unix ms when the pairing slot expires
//   "unpaired:<id>"    a device that unpaired itself, until the daemon says `device_remove`

import { DurableObject } from "cloudflare:workers";
import { Budget } from "./budget";
import { tokenMatches } from "./encoding";
import type { Env } from "./env";
import {
  DAEMON_FRAMES,
  DEVICE_FRAMES,
  FrameError,
  MAX_FRAME_BYTES,
  PAIR_FRAMES,
  parseFrame,
  type DaemonFrame,
  type DeviceFrame,
  type PairFrame,
  type PushFrame,
} from "./frames";
import { sendPush, type PushTarget } from "./push";
import { bearer, plain, route } from "./routes";

type Peer = { role: "daemon" } | { role: "device"; device: string } | { role: "pair"; slot: string };

interface DeviceRecord {
  token_hash: string;
  push?: PushTarget;
}

export const MAX_SLOT_MS = 10 * 60 * 1000;

const CLOSE_NORMAL = 1000;
const CLOSE_REPLACED = 4000;
const CLOSE_REMOVED = 4001;

export class Daemon extends DurableObject<Env> {
  private budget = new Budget();

  /** Called once by the Worker when the relay hands out this daemon's id. */
  async register(tokenHash: string): Promise<void> {
    await this.ctx.storage.put("token_hash", tokenHash);
  }

  async fetch(request: Request): Promise<Response> {
    const tokenHash = await this.ctx.storage.get<string>("token_hash");
    if (!tokenHash) return plain(404, "unknown daemon");
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return plain(426, "expected a WebSocket upgrade");
    }

    const target = route(new URL(request.url).pathname);
    switch (target?.kind) {
      case "daemon":
        return this.connectDaemon(request, tokenHash);
      case "device":
        return this.connectDevice(request, target.device);
      case "pair":
        return this.connectPair(target.slot);
      default:
        return plain(404, "not found");
    }
  }

  // ---- connecting ----

  private async connectDaemon(request: Request, tokenHash: string): Promise<Response> {
    const token = bearer(request);
    if (!token || !(await tokenMatches(token, tokenHash))) return plain(401, "bad daemon token");

    const wasOnline = this.isOpen("daemon");
    for (const old of this.ctx.getWebSockets("daemon")) close(old, CLOSE_REPLACED, "replaced by a new connection");
    const { client, server } = this.accept({ role: "daemon" }, ["daemon"]);

    if (!wasOnline) this.toDevices({ type: "presence", online: true });
    for (const device of this.connectedDevices()) {
      send(server, { type: "presence", device, online: true });
    }
    // Phones that unpaired while the daemon was away, or before it confirmed.
    for (const key of (await this.ctx.storage.list({ prefix: "unpaired:" })).keys()) {
      send(server, { type: "device_unpaired", device: key.slice("unpaired:".length) });
    }
    return upgraded(client);
  }

  private async connectDevice(request: Request, device: string): Promise<Response> {
    const record = await this.device(device);
    const token = bearer(request);
    if (!record || !token || !(await tokenMatches(token, record.token_hash))) {
      return plain(401, "bad device token");
    }

    const tag = `device:${device}`;
    const wasOnline = this.isOpen(tag);
    for (const old of this.ctx.getWebSockets(tag)) close(old, CLOSE_REPLACED, "replaced by a new connection");
    const { client, server } = this.accept({ role: "device", device }, ["device", tag]);

    send(server, { type: "presence", online: this.isOpen("daemon") });
    if (!wasOnline) this.toDaemon({ type: "presence", device, online: true });
    return upgraded(client);
  }

  private async connectPair(slot: string): Promise<Response> {
    const expires = await this.ctx.storage.get<number>(`slot:${slot}`);
    if (expires === undefined || expires <= Date.now()) return plain(404, "no open pairing slot");
    if (this.isOpen(`pair:${slot}`)) return plain(409, "pairing slot is taken");

    const { client } = this.accept({ role: "pair", slot }, ["pair", `pair:${slot}`]);
    return upgraded(client);
  }

  private accept(peer: Peer, tags: string[]): { client: WebSocket; server: WebSocket } {
    const [client, server] = Object.values(new WebSocketPair()) as [WebSocket, WebSocket];
    this.ctx.acceptWebSocket(server, tags);
    server.serializeAttachment(peer);
    return { client, server };
  }

  // ---- hibernation handlers ----

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): Promise<void> {
    const peer = ws.deserializeAttachment() as Peer;
    try {
      switch (peer.role) {
        case "daemon":
          return await this.fromDaemon(ws, parseFrame<DaemonFrame>(message, DAEMON_FRAMES));
        case "device":
          return await this.fromDevice(ws, peer.device, parseFrame<DeviceFrame>(message, DEVICE_FRAMES));
        case "pair":
          return this.fromPair(peer.slot, parseFrame<PairFrame>(message, PAIR_FRAMES));
      }
    } catch (e) {
      if (!(e instanceof FrameError)) console.error("frame handling failed", e);
      send(ws, { type: "error", message: e instanceof FrameError ? e.message : "internal error" });
    }
  }

  async webSocketClose(ws: WebSocket, code: number, reason: string): Promise<void> {
    // Finish the closing handshake where the runtime doesn't reply on its own
    // (no-op under web_socket_auto_reply_to_close). 1005/1006 can't be sent back.
    close(ws, code === 1005 || code === 1006 ? CLOSE_NORMAL : code, reason);
    this.left(ws);
  }

  async webSocketError(ws: WebSocket): Promise<void> {
    this.left(ws);
  }

  /** Tells the other side when the last socket of a daemon or device goes. */
  private left(ws: WebSocket): void {
    const peer = ws.deserializeAttachment() as Peer;
    if (peer.role === "daemon" && !this.isOpen("daemon", ws)) {
      this.toDevices({ type: "presence", online: false });
    } else if (peer.role === "device" && !this.isOpen(`device:${peer.device}`, ws)) {
      this.toDaemon({ type: "presence", device: peer.device, online: false });
    }
  }

  async alarm(): Promise<void> {
    await this.expireSlots();
  }

  // ---- frames ----

  private async fromDaemon(ws: WebSocket, frame: DaemonFrame): Promise<void> {
    switch (frame.type) {
      case "send":
        return this.toDevice(frame.device, { type: "recv", data: frame.data });
      case "pair_open":
        return this.openSlot(frame.slot, frame.expires);
      case "pair_send":
        for (const socket of this.ctx.getWebSockets(`pair:${frame.slot}`)) {
          send(socket, { type: "recv", data: frame.data });
        }
        return this.closeSlot(frame.slot, "pairing answered");
      case "pair_close":
        return this.closeSlot(frame.slot, "pairing closed");
      case "device_add":
        return this.addDevice(frame.device, frame.token_hash);
      case "device_remove":
        return this.removeDevice(frame.device);
      case "push":
        return this.push(ws, frame);
    }
  }

  private async fromDevice(ws: WebSocket, device: string, frame: DeviceFrame): Promise<void> {
    switch (frame.type) {
      case "send": {
        const recv = { type: "recv", device, data: frame.data };
        // What the daemon gets is a little bigger than what the device sent: it must fit too.
        const size = JSON.stringify(recv).length;
        if (size > MAX_FRAME_BYTES) throw new FrameError("frame is too large");
        const spent = this.budget.spend(device, frame.data.length);
        if (!spent.ok) return send(ws, { type: "error", message: "slow down", retry_ms: spent.retryMs });
        return this.toDaemon(recv);
      }
      case "push_token": {
        const record = await this.device(device);
        if (!record) return;
        record.push = { platform: frame.platform, token: frame.token, sandbox: frame.sandbox ?? false };
        await this.ctx.storage.put(`device:${device}`, record);
        return;
      }
      case "unpair":
        return this.unpair(ws, device);
    }
  }

  private fromPair(slot: string, frame: PairFrame): void {
    this.toDaemon({ type: "pair_recv", slot, data: frame.data });
  }

  // ---- pairing slots ----

  private async openSlot(slot: string, expires: number): Promise<void> {
    const now = Date.now();
    if (expires <= now) throw new FrameError("pair_open: \"expires\" is in the past");
    // A daemon whose clock runs ahead still gets a slot, just no longer than allowed.
    const until = Math.min(expires, now + MAX_SLOT_MS);
    await this.ctx.storage.put(`slot:${slot}`, until);

    const alarm = await this.ctx.storage.getAlarm();
    if (alarm === null || until < alarm) await this.ctx.storage.setAlarm(until);
    // The daemon shows its QR code only now, so a phone quick to scan finds the slot.
    this.toDaemon({ type: "pair_opened", slot });
  }

  private async closeSlot(slot: string, reason: string): Promise<void> {
    for (const socket of this.ctx.getWebSockets(`pair:${slot}`)) close(socket, CLOSE_NORMAL, reason);
    await this.ctx.storage.delete(`slot:${slot}`);
  }

  private async expireSlots(): Promise<void> {
    const now = Date.now();
    let next = Infinity;
    for (const [key, expires] of await this.ctx.storage.list<number>({ prefix: "slot:" })) {
      if (expires <= now) await this.closeSlot(key.slice("slot:".length), "pairing slot expired");
      else next = Math.min(next, expires);
    }
    if (next !== Infinity) await this.ctx.storage.setAlarm(next);
  }

  // ---- devices ----

  private device(device: string): Promise<DeviceRecord | undefined> {
    return this.ctx.storage.get<DeviceRecord>(`device:${device}`);
  }

  private async addDevice(device: string, tokenHash: string): Promise<void> {
    const old = await this.device(device);
    if (old?.token_hash === tokenHash) return;
    // A new token for a known id: whoever holds the old one is out.
    if (old) this.closeDevice(device, "device token replaced");
    await this.ctx.storage.put(`device:${device}`, { token_hash: tokenHash } satisfies DeviceRecord);
  }

  private async removeDevice(device: string): Promise<void> {
    this.closeDevice(device, "device removed");
    await this.ctx.storage.delete([`device:${device}`, `unpaired:${device}`]);
  }

  /**
   * The phone erases itself: its token and push token go at once, and the daemon is told
   * (`device_unpaired`) now and on each connection until it answers `device_remove`.
   */
  private async unpair(ws: WebSocket, device: string): Promise<void> {
    await this.ctx.storage.delete(`device:${device}`);
    await this.ctx.storage.put(`unpaired:${device}`, Date.now());
    send(ws, { type: "unpaired" });
    for (const socket of this.ctx.getWebSockets(`device:${device}`)) close(socket, CLOSE_NORMAL, "device unpaired");
    this.toDaemon({ type: "device_unpaired", device });
  }

  private closeDevice(device: string, reason: string): void {
    for (const socket of this.ctx.getWebSockets(`device:${device}`)) close(socket, CLOSE_REMOVED, reason);
  }

  private connectedDevices(): string[] {
    const devices = new Set<string>();
    for (const socket of this.ctx.getWebSockets("device")) {
      const peer = socket.deserializeAttachment() as Peer;
      if (peer.role === "device" && socket.readyState === WebSocket.OPEN) devices.add(peer.device);
    }
    return [...devices];
  }

  // ---- push ----

  private async push(ws: WebSocket, frame: PushFrame): Promise<void> {
    const { device, title, body, category, data, collapse } = frame;
    const target = (await this.device(device))?.push;
    const outcome = target
      ? await sendPush(this.env, target, { title, body, category, data, collapse })
      : { ok: false, error: "device has no push token" };

    if (target && outcome.unregistered) await this.forgetPushToken(device, target.token);
    const result = { type: "push_result", device, ok: outcome.ok, ...(outcome.error && { error: outcome.error }) };
    // The daemon may have reconnected while the push was out.
    if (ws.readyState === WebSocket.OPEN) send(ws, result);
    else this.toDaemon(result);
  }

  private async forgetPushToken(device: string, token: string): Promise<void> {
    const record = await this.device(device);
    if (record?.push?.token !== token) return;
    delete record.push;
    await this.ctx.storage.put(`device:${device}`, record);
  }

  // ---- sending ----

  private toDaemon(frame: object): void {
    for (const socket of this.ctx.getWebSockets("daemon")) send(socket, frame);
  }

  /** Dropped when the device isn't connected: the daemon pushes instead. */
  private toDevice(device: string, frame: object): void {
    for (const socket of this.ctx.getWebSockets(`device:${device}`)) send(socket, frame);
  }

  private toDevices(frame: object): void {
    for (const socket of this.ctx.getWebSockets("device")) send(socket, frame);
  }

  /** Whether a socket with `tag` other than `except` is still open. */
  private isOpen(tag: string, except?: WebSocket): boolean {
    return this.ctx.getWebSockets(tag).some((s) => s !== except && s.readyState === WebSocket.OPEN);
  }
}

function upgraded(client: WebSocket): Response {
  return new Response(null, { status: 101, webSocket: client });
}

function send(socket: WebSocket, frame: object): void {
  if (socket.readyState !== WebSocket.OPEN) return;
  try {
    socket.send(JSON.stringify(frame));
  } catch {
    // The socket went away between the check and the send; its close handler follows.
  }
}

function close(socket: WebSocket, code: number, reason: string): void {
  try {
    socket.close(code, reason);
  } catch {
    // Already closed.
  }
}
