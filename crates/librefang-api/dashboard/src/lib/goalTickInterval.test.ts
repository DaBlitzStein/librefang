import { describe, expect, it } from "vitest";

import {
  DEFAULT_GOAL_TICK_INTERVAL_SECS,
  MAX_GOAL_TICK_INTERVAL_SECS,
  MIN_GOAL_TICK_INTERVAL_SECS,
  parseGoalTickInterval,
} from "./goalTickInterval";

describe("parseGoalTickInterval", () => {
  it("reads a blank field as the backend's 'use the default' signal", () => {
    expect(parseGoalTickInterval("")).toBeNull();
    expect(parseGoalTickInterval("   ")).toBeNull();
  });

  it("accepts both ends of the range the API enforces", () => {
    expect(parseGoalTickInterval(String(MIN_GOAL_TICK_INTERVAL_SECS))).toBe(
      MIN_GOAL_TICK_INTERVAL_SECS,
    );
    expect(parseGoalTickInterval(String(MAX_GOAL_TICK_INTERVAL_SECS))).toBe(
      MAX_GOAL_TICK_INTERVAL_SECS,
    );
    expect(parseGoalTickInterval(" 900 ")).toBe(900);
  });

  // Each of these reached `validate_tick_interval` and came back a 400 that
  // took the rest of the edit with it.
  it.each([
    ["one under the floor", String(MIN_GOAL_TICK_INTERVAL_SECS - 1)],
    ["one over the ceiling", String(MAX_GOAL_TICK_INTERVAL_SECS + 1)],
    ["a fraction", "1.5"],
    ["a negative", "-1"],
    ["not a number at all", "soon"],
  ])("refuses %s rather than spending a request on it", (_label, raw) => {
    expect(parseGoalTickInterval(raw)).toBeUndefined();
  });

  it("keeps a default that sits inside its own bounds", () => {
    expect(DEFAULT_GOAL_TICK_INTERVAL_SECS).toBeGreaterThanOrEqual(MIN_GOAL_TICK_INTERVAL_SECS);
    expect(DEFAULT_GOAL_TICK_INTERVAL_SECS).toBeLessThanOrEqual(MAX_GOAL_TICK_INTERVAL_SECS);
  });
});
