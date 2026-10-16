// How much a device may send in a minute. A phone's files go through the relay a piece at a
// time (docs/remote-protocol.md, Attachments); this keeps one device from flooding its daemon.
// Kept in memory: an object that hibernates or restarts starts each device's minute over.

export const DEVICE_BYTES_PER_MINUTE = 96 * 1024 * 1024;
export const MINUTE_MS = 60_000;

export class Budget {
  private used = new Map<string, { start: number; bytes: number }>();

  constructor(
    private readonly limit = DEVICE_BYTES_PER_MINUTE,
    private readonly windowMs = MINUTE_MS,
  ) {}

  /** Counts `bytes` for `who`; when that would pass the limit, nothing is counted and the
   * answer is how many ms until the window starts over. */
  spend(who: string, bytes: number, now = Date.now()): { ok: true } | { ok: false; retryMs: number } {
    let window = this.used.get(who);
    if (!window || now - window.start >= this.windowMs) {
      window = { start: now, bytes: 0 };
      this.used.set(who, window);
    }
    if (window.bytes + bytes > this.limit) return { ok: false, retryMs: window.start + this.windowMs - now };
    window.bytes += bytes;
    return { ok: true };
  }

  forget(who: string): void {
    this.used.delete(who);
  }
}
