import { runInDurableObject } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { MAX_SLOT_MS } from "../src/daemon";
import { newId, open, pairPath, register, connectDaemon, upgrade } from "./helpers";

async function daemonWithSlot(expiresIn = 5 * 60 * 1000) {
  const { id, token } = await register();
  const daemon = await connectDaemon(id, token);
  const slot = newId();
  daemon.send({ type: "pair_open", slot, expires: Date.now() + expiresIn });
  await daemon.flush();
  return { id, daemon, slot };
}

describe("pairing", () => {
  it("carries one request and one answer, then forgets the slot", async () => {
    const { id, daemon, slot } = await daemonWithSlot();
    const phone = await open(pairPath(id, slot));

    phone.send({ type: "send", data: "sealed-request" });
    expect(await daemon.next("pair_recv")).toEqual({ type: "pair_recv", slot, data: "sealed-request" });

    daemon.send({ type: "pair_send", slot, data: "sealed-answer" });
    expect(await phone.next("recv")).toEqual({ type: "recv", data: "sealed-answer" });
    expect((await phone.closed).code).toBe(1000);
    expect((await upgrade(pairPath(id, slot))).status).toBe(404);
  });

  it("refuses a slot that was never opened", async () => {
    const { id } = await register();
    expect((await upgrade(pairPath(id, newId()))).status).toBe(404);
  });

  it("refuses a slot of a daemon that doesn't exist", async () => {
    expect((await upgrade(pairPath(newId(), newId()))).status).toBe(404);
  });

  it("takes one connection at a time", async () => {
    const { id, slot } = await daemonWithSlot();
    await open(pairPath(id, slot));
    expect((await upgrade(pairPath(id, slot))).status).toBe(409);
  });

  it("refuses an expired slot", async () => {
    const { id, slot } = await daemonWithSlot(50);
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect((await upgrade(pairPath(id, slot))).status).toBe(404);
  });

  it("closes a slot's socket when the slot expires", async () => {
    const { id, slot } = await daemonWithSlot(300);
    const phone = await open(pairPath(id, slot));
    expect((await phone.closed).reason).toBe("pairing slot expired");
    expect((await upgrade(pairPath(id, slot))).status).toBe(404);
  });

  it("pair_close closes the slot and its socket", async () => {
    const { id, daemon, slot } = await daemonWithSlot();
    const phone = await open(pairPath(id, slot));
    daemon.send({ type: "pair_close", slot });
    expect((await phone.closed).code).toBe(1000);
    expect((await upgrade(pairPath(id, slot))).status).toBe(404);
  });

  it("keeps a slot to at most 10 minutes", async () => {
    const { id, daemon } = await daemonWithSlot();
    const slot = newId();
    daemon.send({ type: "pair_open", slot, expires: Date.now() + 60 * 60 * 1000 });
    await daemon.flush();
    const expires = await runInDurableObject(env.DAEMONS.get(env.DAEMONS.idFromName(id)), (_, state) =>
      state.storage.get<number>(`slot:${slot}`),
    );
    expect(expires).toBeLessThanOrEqual(Date.now() + MAX_SLOT_MS);
  });

  it("rejects a slot that expires in the past", async () => {
    const { daemon } = await daemonWithSlot();
    daemon.send({ type: "pair_open", slot: newId(), expires: Date.now() - 1000 });
    expect((await daemon.next("error")).message).toContain("in the past");
  });

  it("lets a pairing socket only send", async () => {
    const { id, slot } = await daemonWithSlot();
    const phone = await open(pairPath(id, slot));
    phone.send({ type: "pair_send", slot, data: "x" });
    expect((await phone.next("error")).message).toContain("unknown frame type");
  });
});
