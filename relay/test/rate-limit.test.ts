import { runDurableObjectAlarm } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { REGISTRATIONS_PER_HOUR } from "../src/rate-limit";
import { newIp, request } from "./helpers";

const registerFrom = (ip: string) =>
  request("/v1/daemons", { method: "POST", headers: { "CF-Connecting-IP": ip } });

describe("registration rate limit", () => {
  it("allows a few registrations per address per hour", async () => {
    const ip = newIp();
    for (let i = 0; i < REGISTRATIONS_PER_HOUR; i++) {
      expect((await registerFrom(ip)).status).toBe(201);
    }
    expect((await registerFrom(ip)).status).toBe(429);
    expect((await registerFrom(newIp())).status).toBe(201);

    // The window's end clears the count.
    expect(await runDurableObjectAlarm(env.RATE_LIMITS.get(env.RATE_LIMITS.idFromName(ip)))).toBe(true);
    expect((await registerFrom(ip)).status).toBe(201);
  });
});
