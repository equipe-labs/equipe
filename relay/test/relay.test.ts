import { evictDurableObject, runInDurableObject } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { sha256Hex } from "../src/encoding";
import { MAX_FRAME_BYTES } from "../src/frames";
import {
  connectDaemon,
  daemonPath,
  devicePath,
  newId,
  newToken,
  open,
  register,
  request,
  setup,
  upgrade,
} from "./helpers";

const daemonStorage = <T>(id: string, key: string) =>
  runInDurableObject(env.DAEMONS.get(env.DAEMONS.idFromName(id)), (_, state) => state.storage.get<T>(key));

describe("registration", () => {
  it("hands out a daemon id and token, keeping only the token's hash", async () => {
    const { id, token } = await register();
    expect(id).toMatch(/^[0-9a-f]{32}$/);
    expect(token).toMatch(/^[A-Za-z0-9_-]{43}$/);
    expect(await daemonStorage(id, "token_hash")).toBe(await sha256Hex(token));
  });

  it("gives every daemon its own id", async () => {
    const [a, b] = await Promise.all([register(), register()]);
    expect(a.id).not.toBe(b.id);
    expect(a.token).not.toBe(b.token);
  });

  it("only answers POST", async () => {
    expect((await request("/v1/daemons")).status).toBe(405);
  });
});

describe("auth", () => {
  it("rejects a bad or missing daemon token", async () => {
    const { id } = await register();
    expect((await upgrade(daemonPath(id), newToken())).status).toBe(401);
    expect((await upgrade(daemonPath(id))).status).toBe(401);
  });

  it("knows no daemon it didn't register", async () => {
    expect((await upgrade(daemonPath(newId()), newToken())).status).toBe(404);
    expect((await upgrade("/v1/daemons/not-hex/connect", newToken())).status).toBe(404);
  });

  it("wants a WebSocket", async () => {
    const { id, token } = await register();
    const response = await request(daemonPath(id), { headers: { Authorization: `Bearer ${token}` } });
    expect(response.status).toBe(426);
  });

  it("rejects a bad device token and unknown devices", async () => {
    const { id, device } = await setup();
    expect((await upgrade(devicePath(id, device), newToken())).status).toBe(401);
    expect((await upgrade(devicePath(id, device))).status).toBe(401);
    expect((await upgrade(devicePath(id, newId()), newToken())).status).toBe(401);
  });
});

describe("forwarding", () => {
  it("passes frames daemon → device and device → daemon", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    const phone = await open(devicePath(id, device), deviceToken);
    await daemon.next("presence");

    daemon.send({ type: "send", device, data: "sealed-for-phone", extra: "dropped" });
    expect(await phone.next("recv")).toEqual({ type: "recv", data: "sealed-for-phone" });

    phone.send({ type: "send", data: "sealed-for-daemon" });
    expect(await daemon.next("recv")).toEqual({ type: "recv", device, data: "sealed-for-daemon" });
  });

  it("only delivers to the addressed device", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    const other = newId();
    const otherToken = newToken();
    daemon.send({ type: "device_add", device: other, token_hash: await sha256Hex(otherToken) });
    await daemon.flush();
    const phone = await open(devicePath(id, device), deviceToken);
    const tablet = await open(devicePath(id, other), otherToken);

    daemon.send({ type: "send", device: other, data: "for-tablet" });
    expect(await tablet.next("recv")).toEqual({ type: "recv", data: "for-tablet" });
    await phone.nothing("recv");
  });

  it("drops frames for a device that isn't connected", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    daemon.send({ type: "send", device, data: "lost" });
    await daemon.flush();
    const phone = await open(devicePath(id, device), deviceToken);
    await phone.next("presence");
    await phone.nothing();
  });

  it("drops device frames while the daemon is away", async () => {
    const { id, token, daemon, device, deviceToken } = await setup();
    daemon.close();
    await daemon.closed;
    const phone = await open(devicePath(id, device), deviceToken);
    phone.send({ type: "send", data: "lost" });
    await phone.flush();

    const again = await connectDaemon(id, token);
    await again.next("presence");
    await again.nothing("recv");
  });
});

describe("presence", () => {
  it("tells the daemon when a device comes and goes", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    const phone = await open(devicePath(id, device), deviceToken);
    expect(await daemon.next("presence")).toEqual({ type: "presence", device, online: true });
    phone.close();
    expect(await daemon.next("presence")).toEqual({ type: "presence", device, online: false });
  });

  it("tells a device whether the daemon is there, and when that changes", async () => {
    const { id, token, daemon, device, deviceToken } = await setup();
    const phone = await open(devicePath(id, device), deviceToken);
    expect(await phone.next("presence")).toEqual({ type: "presence", online: true });

    daemon.close();
    expect(await phone.next("presence")).toEqual({ type: "presence", online: false });
    await connectDaemon(id, token);
    expect(await phone.next("presence")).toEqual({ type: "presence", online: true });
  });

  it("tells a device the daemon is away when it connects", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    daemon.close();
    await daemon.closed;
    const phone = await open(devicePath(id, device), deviceToken);
    expect(await phone.next("presence")).toEqual({ type: "presence", online: false });
  });

  it("tells a connecting daemon which devices are already there", async () => {
    const { id, token, daemon, device, deviceToken } = await setup();
    const other = newId();
    const otherToken = newToken();
    const absent = newId();
    daemon.send({ type: "device_add", device: other, token_hash: await sha256Hex(otherToken) });
    daemon.send({ type: "device_add", device: absent, token_hash: await sha256Hex(newToken()) });
    await daemon.flush();
    daemon.close();
    await daemon.closed;
    await open(devicePath(id, device), deviceToken);
    await open(devicePath(id, other), otherToken);

    const again = await connectDaemon(id, token);
    const seen = [await again.next("presence"), await again.next("presence")];
    expect(seen).toEqual(
      expect.arrayContaining([
        { type: "presence", device, online: true },
        { type: "presence", device: other, online: true },
      ]),
    );
    await again.nothing("presence");
  });
});

describe("hibernation", () => {
  it("knows its sockets again after the object sleeps", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    const phone = await open(devicePath(id, device), deviceToken);
    await daemon.next("presence");

    await evictDurableObject(env.DAEMONS.get(env.DAEMONS.idFromName(id)), { webSockets: "hibernate" });

    phone.send({ type: "send", data: "after-sleep" });
    expect(await daemon.next("recv")).toEqual({ type: "recv", device, data: "after-sleep" });
    daemon.send({ type: "send", device, data: "reply" });
    expect(await phone.next("recv")).toEqual({ type: "recv", data: "reply" });
    phone.close();
    expect(await daemon.next("presence")).toEqual({ type: "presence", device, online: false });
  });
});

describe("replacing sockets", () => {
  it("a new daemon socket closes the old one without flapping presence", async () => {
    const { id, token, daemon, device, deviceToken } = await setup();
    const phone = await open(devicePath(id, device), deviceToken);
    await phone.next("presence");

    const again = await connectDaemon(id, token);
    expect((await daemon.closed).code).toBe(4000);
    expect(await again.next("presence")).toEqual({ type: "presence", device, online: true });
    await phone.nothing("presence");

    phone.send({ type: "send", data: "to-new-daemon" });
    expect(await again.next("recv")).toEqual({ type: "recv", device, data: "to-new-daemon" });
  });

  it("a device reconnecting closes its old socket", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    const first = await open(devicePath(id, device), deviceToken);
    await daemon.next("presence");

    const second = await open(devicePath(id, device), deviceToken);
    expect((await first.closed).code).toBe(4000);
    await daemon.nothing("presence");

    daemon.send({ type: "send", device, data: "hello" });
    expect(await second.next("recv")).toEqual({ type: "recv", data: "hello" });
  });
});

describe("devices", () => {
  it("device_remove closes the socket and locks the device out", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    const phone = await open(devicePath(id, device), deviceToken);
    phone.send({ type: "push_token", platform: "apns", token: "aa".repeat(32) });
    await phone.flush();

    daemon.send({ type: "device_remove", device });
    expect((await phone.closed).code).toBe(4001);
    expect((await upgrade(devicePath(id, device), deviceToken)).status).toBe(401);
    expect(await daemonStorage(id, `device:${device}`)).toBeUndefined();
  });

  it("device_add with a new token closes sockets using the old one", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    const phone = await open(devicePath(id, device), deviceToken);
    const newDeviceToken = newToken();

    daemon.send({ type: "device_add", device, token_hash: await sha256Hex(newDeviceToken) });
    expect((await phone.closed).code).toBe(4001);
    expect((await upgrade(devicePath(id, device), deviceToken)).status).toBe(401);
    expect((await upgrade(devicePath(id, device), newDeviceToken)).status).toBe(101);
  });

  it("keeps a device's push token", async () => {
    const { id, device, deviceToken } = await setup();
    const phone = await open(devicePath(id, device), deviceToken);
    phone.send({ type: "push_token", platform: "fcm", token: "fcm-token" });
    phone.send({ type: "push_token", platform: "apns", token: "ab".repeat(32), sandbox: true });
    await phone.flush();
    expect(await daemonStorage(id, `device:${device}`)).toMatchObject({
      push: { platform: "apns", token: "ab".repeat(32), sandbox: true },
    });
  });
});

describe("invalid frames", () => {
  it("answers bad daemon frames with an error and keeps going", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    const phone = await open(devicePath(id, device), deviceToken);
    await daemon.next("presence");

    const bad: [string | ArrayBuffer, string][] = [
      ["not json", "not valid JSON"],
      ["[1,2]", "JSON object"],
      [JSON.stringify({ data: "x" }), "unknown frame type"],
      [JSON.stringify({ type: "recv", device, data: "x" }), "unknown frame type"],
      [JSON.stringify({ type: "send", device }), `"data"`],
      [JSON.stringify({ type: "send", device: "ABC", data: "x" }), `"device"`],
      [JSON.stringify({ type: "device_add", device, token_hash: "short" }), `"token_hash"`],
      [JSON.stringify({ type: "pair_open", slot: newId(), expires: "soon" }), `"expires"`],
      [new TextEncoder().encode("{}").buffer as ArrayBuffer, "text"],
      [JSON.stringify({ type: "send", device, data: "x".repeat(MAX_FRAME_BYTES) }), "too large"],
    ];
    for (const [frame, message] of bad) {
      daemon.ws.send(frame);
      expect((await daemon.next("error")).message).toContain(message);
    }

    daemon.send({ type: "send", device, data: "still-works" });
    expect(await phone.next("recv")).toEqual({ type: "recv", data: "still-works" });
  });

  it("caps push labels at 200 characters", async () => {
    const { daemon, device } = await setup();
    const push = { type: "push", device, title: "t", body: "b", category: "approval", data: "d" };
    for (const field of ["title", "body", "category"]) {
      daemon.send({ ...push, [field]: "x".repeat(201) });
      expect((await daemon.next("error")).message).toContain(`"${field}" must be a string of at most 200`);
    }
  });

  it("answers bad device frames with an error", async () => {
    const { id, daemon, device, deviceToken } = await setup();
    const phone = await open(devicePath(id, device), deviceToken);
    await phone.next("presence");

    phone.send({ type: "device_remove", device });
    expect((await phone.next("error")).message).toContain("unknown frame type");
    phone.send({ type: "push_token", platform: "webpush", token: "x" });
    expect((await phone.next("error")).message).toContain(`"platform"`);

    phone.send({ type: "send", data: "fine" });
    expect(await daemon.next("recv")).toEqual({ type: "recv", device, data: "fine" });
  });
});
