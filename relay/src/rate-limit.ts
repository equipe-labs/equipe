/** Registrations one address may make in an hour. Not exported from the
 * entry module: workerd takes every export there for an entrypoint. */
export const REGISTRATIONS_PER_HOUR = 20;

import { DurableObject } from "cloudflare:workers";
import type { Env } from "./env";

interface Window {
  start: number;
  count: number;
}

/** A fixed-window counter; one instance per client IP. */
export class RateLimit extends DurableObject<Env> {
  /** Counts one hit; false when the window's `limit` is already used up. */
  async hit(limit: number, windowMs: number): Promise<boolean> {
    const now = Date.now();
    let window = await this.ctx.storage.get<Window>("window");
    if (!window || now - window.start >= windowMs) {
      window = { start: now, count: 0 };
      // Nothing to keep once the window is over.
      await this.ctx.storage.setAlarm(now + windowMs);
    }
    if (window.count >= limit) return false;
    window.count += 1;
    await this.ctx.storage.put("window", window);
    return true;
  }

  async alarm(): Promise<void> {
    await this.ctx.storage.deleteAll();
  }
}
