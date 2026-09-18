// Step ladders for the agent manifest's numeric fields.
//
// These replace bare `<input type="number">` boxes — seventeen of them, one
// per quota, heartbeat and threshold the manifest accepts. A bare number asks
// the operator to already know the field's scale: is a token budget 10_000 or
// 10_000_000? Is `max_iterations` a handful or a few hundred? The input
// answered none of that, so the only way to fill it in was to guess and then
// find out from a bill or a stopped agent.
//
// A short ladder plus a custom rung answers it by showing the shape of the
// scale, and keeps the exception reachable. Same reasoning as
// `modelParamLadders.ts`, which did this for the sampling parameters.
//
// The ladders are deliberately coarse. A rung is an order of magnitude, not a
// precise setting: an operator who needs 45_000 types it into the custom field,
// and one who has no opinion picks from a scale they can read.

const KB = 1024;
const MB = 1024 * KB;
const GB = 1024 * MB;

/**
 * Decimal counts — "10K", "100K", "1M".
 *
 * Deliberately not `formatTokens`, which is binary and renders 1_000_000 as
 * "976K". Token *budgets* are chosen in round decimal figures, and rendering
 * the rung as something other than the number it stores makes the control look
 * wrong about its own value.
 */
export function formatCount(value: number): string {
  if (value >= 1_000_000_000) return `${value / 1_000_000_000}G`;
  if (value >= 1_000_000) return `${value / 1_000_000}M`;
  if (value >= 1_000) return `${value / 1_000}K`;
  return String(value);
}

/** Binary sizes, in the units the quota is enforced in — "256 MB", "1 GB". */
export function formatBytes(value: number): string {
  if (value >= GB) return `${value / GB} GB`;
  if (value >= MB) return `${value / MB} MB`;
  if (value >= KB) return `${value / KB} KB`;
  return `${value} B`;
}

/** Durations held in seconds — "30 s", "5 min", "2 h". */
export function formatSeconds(value: number): string {
  if (value >= 3600 && value % 3600 === 0) return `${value / 3600} h`;
  if (value >= 60 && value % 60 === 0) return `${value / 60} min`;
  return `${value} s`;
}

/** Durations held in milliseconds — "500 ms", "5 s", "2 min". */
export function formatMillis(value: number): string {
  if (value >= 60_000 && value % 60_000 === 0) return `${value / 60_000} min`;
  if (value >= 1000 && value % 1000 === 0) return `${value / 1000} s`;
  return `${value} ms`;
}

/** Dollar amounts — "$0.10", "$5", "$1K". */
export function formatUsd(value: number): string {
  if (value >= 1000 && value % 1000 === 0) return `$${formatCount(value)}`;
  return `$${value}`;
}

// ---------------------------------------------------------------- budgets

/**
 * LLM tokens per hour. Rungs are the order-of-magnitude sequence: an idle
 * agent on a small model sits near the bottom, a batch worker near the top.
 */
export const LLM_TOKENS_PER_HOUR_LADDER = [
  10_000, 100_000, 1_000_000, 10_000_000, 100_000_000,
] as const;

/** Tool calls per minute. */
export const TOOL_CALLS_PER_MINUTE_LADDER = [5, 10, 30, 60, 120, 300, 600] as const;

/**
 * Spend caps, hourly / daily / monthly.
 *
 * Three ladders rather than one because each is read against a different
 * period: $50/h is a runaway agent, $50/month is a shoestring budget.
 */
export const COST_PER_HOUR_LADDER = [0.1, 0.5, 1, 5, 10, 50, 100] as const;
export const COST_PER_DAY_LADDER = [1, 5, 10, 25, 50, 100, 500] as const;
export const COST_PER_MONTH_LADDER = [10, 50, 100, 500, 1000, 5000, 10_000] as const;

/** Bytes pulled over the network per rolling hour. */
export const NETWORK_BYTES_PER_HOUR_LADDER = [
  10 * MB, 100 * MB, 500 * MB, 1 * GB, 5 * GB, 10 * GB,
] as const;

/** WASM heap ceiling for the agent's module. */
export const MEMORY_BYTES_LADDER = [
  64 * MB, 128 * MB, 256 * MB, 512 * MB, 1 * GB, 2 * GB, 4 * GB,
] as const;

/** CPU time per invocation, in milliseconds. */
export const CPU_TIME_MS_LADDER = [1000, 5000, 10_000, 30_000, 60_000, 300_000] as const;

// ------------------------------------------------------------ autonomous

/** Iterations per invocation. */
export const MAX_ITERATIONS_LADDER = [5, 10, 25, 50, 100, 250, 500] as const;

/** Restarts before the agent is stopped for good. */
export const MAX_RESTARTS_LADDER = [0, 1, 2, 3, 5, 10, 25] as const;

/** Heartbeat cadence, in seconds. */
export const HEARTBEAT_INTERVAL_LADDER = [30, 60, 300, 900, 1800, 3600] as const;

/** How long a heartbeat may go unanswered before it counts as missed. */
export const HEARTBEAT_TIMEOUT_LADDER = [30, 60, 120, 300, 600, 1800] as const;

/** Recent heartbeats kept in the agent's context. */
export const HEARTBEAT_KEEP_RECENT_LADDER = [1, 5, 10, 20, 50, 100] as const;

// -------------------------------------------------------------- routing

/**
 * Complexity-router thresholds, in **tokens**.
 *
 * `simple_threshold` is the token count below which a turn is simple and
 * `complex_threshold` the count above which it is complex, so the rungs are
 * token counts and not an abstract score — a rung of "8" would mean nothing.
 * Both share a ladder because they are the same quantity at two cut points;
 * the invariant that simple < complex is the form's to enforce, not the
 * ladder's.
 */
export const ROUTING_THRESHOLD_LADDER = [
  1000, 2000, 4000, 8000, 16_000, 32_000, 64_000, 128_000,
] as const;

// ------------------------------------------------------------- thinking

/** Extended-thinking budget, in tokens. */
export const THINKING_BUDGET_LADDER = [
  1024, 2048, 4096, 8192, 16_384, 32_768, 65_536,
] as const;

// ------------------------------------------------------------- schedule

/** Cadence of a continuous-mode agent, in seconds. */
export const CHECK_INTERVAL_LADDER = [5, 15, 30, 60, 300, 900, 3600] as const;
