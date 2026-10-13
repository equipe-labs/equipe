import { runInDurableObject } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearPushCaches } from "../src/push";
import { devicePath, open, setup } from "./helpers";

const APNS_TOKEN = "ab".repeat(32);
const PUSH = { type: "push", title: "frontend", body: "frontend asks to run a command", category: "approval", data: "sealed" };

type Handler = (request: Request) => Response | Promise<Response>;

/** An outgoing request, read where it was made (bodies can't cross Durable Objects). */
interface Call {
  url: string;
  method: string;
  headers: Headers;
  body: string;
}
let calls: Call[] = [];

function answer(handler: Handler) {
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    const request = new Request(input, init);
    const body = new TextDecoder().decode(await request.clone().arrayBuffer());
    calls.push({ url: request.url, method: request.method, headers: request.headers, body });
    return handler(request);
  });
}

beforeEach(() => {
  calls = [];
  clearPushCaches();
});

afterEach(() => {
  vi.restoreAllMocks();
});

/** A daemon with one device that has handed the relay a push token. */
async function deviceWithToken(platform: "apns" | "fcm", token: string, sandbox?: boolean) {
  const ctx = await setup();
  const phone = await open(devicePath(ctx.id, ctx.device), ctx.deviceToken);
  phone.send({ type: "push_token", platform, token, sandbox });
  await phone.flush();
  phone.close();
  await ctx.daemon.next("presence");
  return ctx;
}

async function pushOnce(ctx: Awaited<ReturnType<typeof setup>>, extra: object = {}) {
  ctx.daemon.send({ ...PUSH, device: ctx.device, ...extra });
  return ctx.daemon.next("push_result");
}

const storedPush = (id: string, device: string) =>
  runInDurableObject(env.DAEMONS.get(env.DAEMONS.idFromName(id)), async (_, state) => {
    const record = await state.storage.get<{ push?: unknown }>(`device:${device}`);
    return record?.push;
  });

function fromB64url(text: string): Uint8Array {
  const b64 = text.replace(/-/g, "+").replace(/_/g, "/");
  return Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
}

const decodeJson = (part: string) => JSON.parse(new TextDecoder().decode(fromB64url(part)));

async function verifyJwt(jwt: string, publicPem: string, algorithm: { name: string; namedCurve?: string; hash?: string }) {
  const [header, claims, signature] = jwt.split(".") as [string, string, string];
  const der = fromB64url(publicPem.replace(/-----[^-]+-----|\s/g, "").replace(/\+/g, "-").replace(/\//g, "_"));
  const key = await crypto.subtle.importKey("spki", der, algorithm, false, ["verify"]);
  const verifyAlgorithm = algorithm.name === "ECDSA" ? { name: "ECDSA", hash: "SHA-256" } : algorithm;
  const valid = await crypto.subtle.verify(
    verifyAlgorithm,
    key,
    fromB64url(signature),
    new TextEncoder().encode(`${header}.${claims}`),
  );
  return { header: decodeJson(header), claims: decodeJson(claims), valid };
}

describe("APNs", () => {
  it("sends an alert with the sealed data, signed with the relay's key", async () => {
    answer(() => new Response(null, { status: 200 }));
    const ctx = await deviceWithToken("apns", APNS_TOKEN);

    expect(await pushOnce(ctx, { collapse: "ticket-7" })).toEqual({ type: "push_result", device: ctx.device, ok: true });

    expect(calls).toHaveLength(1);
    const call = calls[0]!;
    expect(call.method).toBe("POST");
    expect(call.url).toBe(`https://api.push.apple.com/3/device/${APNS_TOKEN}`);
    expect(call.headers.get("apns-push-type")).toBe("alert");
    expect(call.headers.get("apns-topic")).toBe("im.equipe.app");
    expect(call.headers.get("apns-collapse-id")).toBe("ticket-7");
    expect(JSON.parse(call.body)).toEqual({
      aps: {
        alert: { title: PUSH.title, body: PUSH.body },
        "mutable-content": 1,
        category: "approval",
        sound: "default",
      },
      data: "sealed",
    });

    const jwt = call.headers.get("authorization")!.replace(/^bearer /, "");
    const { header, claims, valid } = await verifyJwt(jwt, env.TEST_APNS_PUBLIC_KEY!, {
      name: "ECDSA",
      namedCurve: "P-256",
    });
    expect(valid).toBe(true);
    expect(header).toEqual({ alg: "ES256", kid: "TESTKEY123" });
    expect(claims.iss).toBe("TESTTEAM12");
    expect(Math.abs(claims.iat - Date.now() / 1000)).toBeLessThan(60);
  });

  it("uses the sandbox for development builds and reuses its JWT", async () => {
    answer(() => new Response(null, { status: 200 }));
    const ctx = await deviceWithToken("apns", APNS_TOKEN, true);

    expect((await pushOnce(ctx)).ok).toBe(true);
    expect((await pushOnce(ctx)).ok).toBe(true);

    expect(calls.map((c) => c.url)).toEqual([
      `https://api.sandbox.push.apple.com/3/device/${APNS_TOKEN}`,
      `https://api.sandbox.push.apple.com/3/device/${APNS_TOKEN}`,
    ]);
    expect(calls[0]!.headers.has("apns-collapse-id")).toBe(false);
    expect(calls[1]!.headers.get("authorization")).toBe(calls[0]!.headers.get("authorization"));
  });

  it("forgets a token APNs says is gone", async () => {
    answer(() => Response.json({ reason: "Unregistered" }, { status: 410 }));
    const ctx = await deviceWithToken("apns", APNS_TOKEN);

    const result = await pushOnce(ctx);
    expect(result).toMatchObject({ ok: false, error: "APNs 410 Unregistered" });
    expect(await storedPush(ctx.id, ctx.device)).toBeUndefined();

    expect(await pushOnce(ctx)).toEqual({
      type: "push_result",
      device: ctx.device,
      ok: false,
      error: "device has no push token",
    });
    expect(calls).toHaveLength(1);
  });

  it("keeps the token on other errors", async () => {
    answer(() => Response.json({ reason: "TooManyRequests" }, { status: 429 }));
    const ctx = await deviceWithToken("apns", APNS_TOKEN);
    expect(await pushOnce(ctx)).toMatchObject({ ok: false, error: "APNs 429 TooManyRequests" });
    expect(await storedPush(ctx.id, ctx.device)).toBeDefined();
  });

  it("answers ok:false when the network fails", async () => {
    answer(() => {
      throw new Error("connection reset");
    });
    const ctx = await deviceWithToken("apns", APNS_TOKEN);
    expect(await pushOnce(ctx)).toMatchObject({ ok: false, error: "push failed: connection reset" });
  });
});

describe("FCM", () => {
  const fcm: Handler = (request) => {
    if (request.url === "https://oauth2.googleapis.com/token") {
      return Response.json({ access_token: "ya29.test", expires_in: 3599, token_type: "Bearer" });
    }
    return Response.json({ name: "projects/equipe-test/messages/1" });
  };

  it("trades a signed service-account JWT for an access token and sends a data message", async () => {
    answer(fcm);
    const ctx = await deviceWithToken("fcm", "fcm-token-1");

    expect(await pushOnce(ctx, { collapse: "ticket-7" })).toEqual({ type: "push_result", device: ctx.device, ok: true });
    expect(calls.map((c) => c.url)).toEqual([
      "https://oauth2.googleapis.com/token",
      "https://fcm.googleapis.com/v1/projects/equipe-test/messages:send",
    ]);

    const [oauth, send] = calls as [Call, Call];
    const form = new URLSearchParams(oauth.body);
    expect(form.get("grant_type")).toBe("urn:ietf:params:oauth:grant-type:jwt-bearer");
    const { header, claims, valid } = await verifyJwt(form.get("assertion")!, env.TEST_FCM_PUBLIC_KEY!, {
      name: "RSASSA-PKCS1-v1_5",
      hash: "SHA-256",
    });
    expect(valid).toBe(true);
    expect(header).toEqual({ alg: "RS256", typ: "JWT" });
    expect(claims).toMatchObject({
      iss: "relay@equipe-test.iam.gserviceaccount.com",
      scope: "https://www.googleapis.com/auth/firebase.messaging",
      aud: "https://oauth2.googleapis.com/token",
    });
    expect(claims.exp - claims.iat).toBe(3600);

    expect(send.headers.get("authorization")).toBe("Bearer ya29.test");
    expect(JSON.parse(send.body)).toEqual({
      message: {
        token: "fcm-token-1",
        data: { title: PUSH.title, body: PUSH.body, category: "approval", data: "sealed", collapse: "ticket-7" },
        android: { priority: "HIGH" },
      },
    });
  });

  it("reuses the access token", async () => {
    answer(fcm);
    const ctx = await deviceWithToken("fcm", "fcm-token-1");
    await pushOnce(ctx);
    await pushOnce(ctx);
    expect(calls.filter((c) => c.url.startsWith("https://oauth2")).length).toBe(1);
    expect(calls.filter((c) => c.url.startsWith("https://fcm")).length).toBe(2);
    const { message } = JSON.parse(calls[2]!.body);
    expect(message.data.title).toBe(PUSH.title);
    expect(message.data).not.toHaveProperty("collapse");
  });

  it("forgets a token FCM says is unregistered", async () => {
    answer((request) =>
      request.url.startsWith("https://oauth2")
        ? fcm(request)
        : Response.json(
            {
              error: {
                code: 404,
                status: "NOT_FOUND",
                details: [{ "@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError", errorCode: "UNREGISTERED" }],
              },
            },
            { status: 404 },
          ),
    );
    const ctx = await deviceWithToken("fcm", "fcm-token-1");
    expect(await pushOnce(ctx)).toMatchObject({ ok: false, error: "FCM 404 UNREGISTERED" });
    expect(await storedPush(ctx.id, ctx.device)).toBeUndefined();
  });

  it("answers ok:false when Google won't give an access token", async () => {
    answer(() => new Response("nope", { status: 400 }));
    const ctx = await deviceWithToken("fcm", "fcm-token-1");
    expect(await pushOnce(ctx)).toMatchObject({ ok: false, error: "push failed: Google OAuth 400" });
  });
});

describe("push without a token", () => {
  it("answers ok:false for a device that never sent one", async () => {
    answer(() => new Response(null, { status: 200 }));
    const ctx = await setup();
    expect(await pushOnce(ctx)).toEqual({
      type: "push_result",
      device: ctx.device,
      ok: false,
      error: "device has no push token",
    });
    expect(calls).toHaveLength(0);
  });

  it("forgets the push token with the device", async () => {
    const ctx = await deviceWithToken("apns", APNS_TOKEN);
    ctx.daemon.send({ type: "device_remove", device: ctx.device });
    await ctx.daemon.flush();
    expect(await storedPush(ctx.id, ctx.device)).toBeUndefined();
    expect(await pushOnce(ctx)).toMatchObject({ ok: false });
  });
});
