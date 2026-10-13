// Runs against a relay with no push secrets (see vitest.config.ts).

import { afterEach, describe, expect, it, vi } from "vitest";
import { devicePath, open, setup } from "./helpers";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("push without secrets", () => {
  for (const [platform, error] of [
    ["apns", "APNs is not configured on this relay"],
    ["fcm", "FCM is not configured on this relay"],
  ] as const) {
    it(`answers ok:false for ${platform}`, async () => {
      const fetch = vi.spyOn(globalThis, "fetch");
      const { id, daemon, device, deviceToken } = await setup();
      const phone = await open(devicePath(id, device), deviceToken);
      phone.send({ type: "push_token", platform, token: "ab".repeat(32) });
      await phone.flush();

      daemon.send({ type: "push", device, title: "t", body: "b", category: "approval", data: "sealed" });
      expect(await daemon.next("push_result")).toEqual({ type: "push_result", device, ok: false, error });
      expect(fetch).not.toHaveBeenCalled();
    });
  }
});
