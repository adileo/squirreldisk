import { afterEach, describe, expect, it, vi } from "vitest";

import { formatScannedAt } from "./scanTime";

describe("formatScannedAt", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns nothing for missing or invalid timestamps", () => {
    expect(formatScannedAt()).toBeNull();
    expect(formatScannedAt(Number.NaN)).toBeNull();
  });

  it("formats recent scans as relative time", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-30T12:00:00Z"));

    expect(formatScannedAt(Date.parse("2026-04-30T11:59:45Z"))).toBe(
      "Scanned just now"
    );
    expect(formatScannedAt(Date.parse("2026-04-30T11:42:00Z"))).toBe(
      "Scanned 18m ago"
    );
    expect(formatScannedAt(Date.parse("2026-04-30T07:00:00Z"))).toBe(
      "Scanned 5h ago"
    );
  });

  it("formats older scans with a calendar date", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-30T12:00:00Z"));

    const formatted = formatScannedAt(Date.parse("2026-04-29T10:15:00Z"));

    expect(formatted?.startsWith("Scanned ")).toBe(true);
    expect(formatted).not.toContain("ago");
    expect(formatted).not.toBe("Scanned just now");
  });
});
