import { createHash } from "node:crypto";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "@playwright/test";

/**
 * The port this checkout's dev server listens on.
 *
 * Fixed at 4173 before, and that was a trap. `reuseExistingServer` below is
 * what makes a local re-run cheap, and over one shared port it means the dev
 * server started in *another* worktree keeps serving — that worktree's code —
 * while the spec under test comes from this one. The mixed state it produces
 * (one tree's fix with another tree's locators) exists in no commit, so two
 * people measured incompatible numbers from it and neither was wrong.
 *
 * Derived from this config's own directory: a reused server is therefore always
 * this worktree's, and the value is stable across runs so the reuse still
 * works. `LIBREFANG_E2E_PORT` overrides it when a caller needs a known port.
 *
 * The range keeps clear of the other fixed ports in this repo — 4173 and 4174
 * (the `web/` app's e2e), 4545 (the daemon the dev server proxies to, which
 * this suite must never contend with) and 5173 (vite's own default).
 */
const CHECKOUT = dirname(fileURLToPath(import.meta.url));
const PORT =
  Number(process.env.LIBREFANG_E2E_PORT) ||
  5400 + (parseInt(createHash("sha256").update(CHECKOUT).digest("hex").slice(0, 8), 16) % 400);
const ORIGIN = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "./e2e",
  timeout: 30000,
  use: {
    baseURL: ORIGIN,
    trace: "on-first-retry"
  },
  webServer: {
    command: `pnpm dev --host 127.0.0.1 --port ${PORT}`,
    port: PORT,
    reuseExistingServer: !process.env.CI,
    cwd: "."
  }
});
