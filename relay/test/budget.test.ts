import { describe, expect, it } from "vitest";
import { Budget, DEVICE_BYTES_PER_MINUTE } from "../src/budget";

describe("a device's budget", () => {
  it("takes a 25 MB file's pieces in a minute, and no flood", () => {
    const budget = new Budget();
    // 25 MB in 384 KiB pieces, each sealed and base64'd to about 700 KB.
    for (let i = 0; i < 67; i++) expect(budget.spend("phone", 700_000, 1000 + i).ok).toBe(true);
    const after = budget.spend("phone", DEVICE_BYTES_PER_MINUTE, 2000);
    expect(after).toEqual({ ok: false, retryMs: 59_000 });
  });

  it("starts over each minute, for each device apart", () => {
    const budget = new Budget(10, 1000);
    expect(budget.spend("a", 10, 0).ok).toBe(true);
    expect(budget.spend("a", 1, 500)).toEqual({ ok: false, retryMs: 500 });
    expect(budget.spend("b", 10, 500).ok).toBe(true);
    expect(budget.spend("a", 10, 1000).ok).toBe(true);
    budget.forget("a");
    expect(budget.spend("a", 10, 1001).ok).toBe(true);
  });
});
