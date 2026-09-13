import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  AVAILABLE_PROVIDER_STATUSES,
  getStatusVariant,
  isProviderAvailable,
  isProviderConfigured,
  isProviderKeyRejected,
  isProviderOffline,
} from "./status";

function snakeCase(value: string): string {
  return value.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
}

describe("provider status contracts", () => {
  it("normalizes availability with the same case policy as badge variants", () => {
    expect(isProviderAvailable("Validated_Key")).toBe(true);
    expect(isProviderAvailable("CONFIGURED_CLI")).toBe(true);
    expect(getStatusVariant("RUNNING")).toBe("success");
  });

  it("mirrors the Rust AuthStatus::is_available variant set", () => {
    const source = readFileSync(
      resolve(process.cwd(), "../../librefang-types/src/model_catalog.rs"),
      "utf8",
    );
    const body = source.match(
      /pub fn is_available\(self\) -> bool \{([\s\S]*?)\n[ ]{4}\}/,
    )?.[1];
    expect(body, "AuthStatus::is_available body must be discoverable").toBeTruthy();
    const rustStatuses = [...(body ?? "").matchAll(/AuthStatus::([A-Za-z]+)/g)]
      .map((match) => snakeCase(match[1]))
      .sort();

    expect([...AVAILABLE_PROVIDER_STATUSES].sort()).toEqual(rustStatuses);
  });

  it("separates usability from configuredness for a degraded provider", () => {
    // Both stay out of the availability set — they mirror the Rust variant
    // list and neither provider can serve a request.
    expect(isProviderAvailable("invalid_key")).toBe(false);
    expect(isProviderAvailable("local_offline")).toBe(false);
    expect(isProviderKeyRejected("INVALID_KEY")).toBe(true);
    expect(isProviderOffline("LOCAL_OFFLINE")).toBe(true);
    // But both ARE configured, which is what decides whether the page shows
    // them: each is a state a working provider degrades into.
    expect(isProviderConfigured("invalid_key")).toBe(true);
    expect(isProviderConfigured("local_offline")).toBe(true);
  });

  // Classifies every `AuthStatus` variant the Rust enum declares, not a
  // hand-picked sample — so a tenth variant fails here instead of silently
  // defaulting to "hide it from the page", which is the bug this predicate
  // exists to fix.
  it("classifies every AuthStatus variant the daemon can send", () => {
    const source = readFileSync(
      resolve(process.cwd(), "../../librefang-types/src/model_catalog.rs"),
      "utf8",
    );
    const display = source.match(
      /impl fmt::Display for AuthStatus \{([\s\S]*?)\n\}/,
    )?.[1];
    expect(display, "AuthStatus Display impl must be discoverable").toBeTruthy();
    const variants = [
      ...(display ?? "").matchAll(/AuthStatus::[A-Za-z]+ => write!\(f, "([a-z_]+)"\)/g),
    ].map((match) => match[1]);
    expect(variants.length).toBeGreaterThanOrEqual(9);

    // Configured = "the operator set this up", available or degraded.
    const expectedConfigured = new Set<string>([
      ...AVAILABLE_PROVIDER_STATUSES,
      "invalid_key",
      "local_offline",
    ]);
    const actualConfigured = variants.filter((v) => isProviderConfigured(v)).sort();
    expect(actualConfigured).toEqual([...expectedConfigured].sort());

    // And the rest are genuinely untouched-by-the-operator states.
    expect(variants.filter((v) => !isProviderConfigured(v)).sort()).toEqual([
      "cli_not_installed",
      "missing",
    ]);
  });

  it("treats an absent status as unconfigured", () => {
    expect(isProviderConfigured(undefined)).toBe(false);
    expect(isProviderKeyRejected(undefined)).toBe(false);
    expect(isProviderOffline(undefined)).toBe(false);
  });
});
