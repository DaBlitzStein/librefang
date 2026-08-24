import { describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import type { ChatCommand } from "../api";
import { backendCommandNames, commandLabel, menuCommands } from "./chatCommands";

// Shape of a real `GET /api/commands` payload, trimmed to the cases that
// matter: a client-resolved builtin, a WS-dispatched builtin, `/goal` (the
// command that used to be missing here — upstream #3355), a catalogued builtin
// with no dashboard execution path, and a skill entry.
const CATALOG: ChatCommand[] = [
  { cmd: "/help", desc: "Show this help", desc_key: "cmd_help", no_args: true, exec: "client" },
  { cmd: "/model", desc: "Show or switch agent model", desc_key: "cmd_model", no_args: false, args_hint: "[name]", exec: "backend" },
  { cmd: "/goal", desc: "Create and start an autonomous goal", desc_key: "cmd_goal", no_args: false, args_hint: "<description> [--loop-engineering]", exec: "backend" },
  { cmd: "/think", desc: "Toggle extended thinking", desc_key: "cmd_think", no_args: false },
  { cmd: "/weather", desc: "Show the current weather", source: "skill" },
];

describe("menuCommands", () => {
  it("offers /goal in the slash menu", () => {
    expect(menuCommands(CATALOG).map(c => c.cmd)).toContain("/goal");
  });

  it("keeps commands with no dashboard execution path out of the menu", () => {
    const offered = menuCommands(CATALOG).map(c => c.cmd);
    expect(offered).not.toContain("/think");
    expect(offered).not.toContain("/weather");
  });

  it("tolerates a catalog that has not loaded yet", () => {
    expect(menuCommands(undefined)).toEqual([]);
  });
});

describe("backendCommandNames", () => {
  it("routes /goal over the WebSocket rather than to the agent", () => {
    expect(backendCommandNames(CATALOG)).toContain("goal");
  });

  it("excludes client-resolved and non-executable commands", () => {
    const backend = backendCommandNames(CATALOG);
    expect(backend).not.toContain("help");
    expect(backend).not.toContain("think");
    expect(backend).not.toContain("weather");
  });

  it("strips the leading slash", () => {
    expect(backendCommandNames(CATALOG).every(name => !name.startsWith("/"))).toBe(true);
  });
});

describe("commandLabel", () => {
  const translate = ((key: string, opts?: { defaultValue?: string }) =>
    key === "chat.cmd_goal" ? "Crear y arrancar un objetivo autónomo" : opts?.defaultValue ?? key) as unknown as TFunction;

  it("prefers the locale string", () => {
    expect(commandLabel(translate, CATALOG[2])).toBe("Crear y arrancar un objetivo autónomo");
  });

  it("falls back to the server description when the key is untranslated", () => {
    expect(commandLabel(translate, CATALOG[1])).toBe("Show or switch agent model");
  });

  it("uses the server description when the entry carries no key (skills)", () => {
    expect(commandLabel(translate, CATALOG[4])).toBe("Show the current weather");
  });
});
