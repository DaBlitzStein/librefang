import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import type { RegistrySchema } from "../../api";
import { configQueries, selectKernelMode, useRegistrySchema, useRawConfigToml } from "./config";
import * as client from "../http/client";
import { registryKeys, configKeys } from "./keys";
import { createQueryClientWrapper } from "../test/query-client";

vi.mock("../http/client", () => ({
  // `getFullConfig` backs `configQueries.full()`, which the `select`-based
  // projections build on — the kernelMode key-sharing test reads the factory
  // without issuing a request, but the factory still has to resolve its fn.
  fetchRegistrySchema: vi.fn(),
  getRawConfigToml: vi.fn(),
  getFullConfig: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("selectKernelMode", () => {
  it("passes through the three spellings `KernelMode` serialises to", () => {
    expect(selectKernelMode({ mode: "stable" })).toBe("stable");
    expect(selectKernelMode({ mode: "default" })).toBe("default");
    expect(selectKernelMode({ mode: "dev" })).toBe("dev");
  });

  it("does not read the pre-#6596 `Debug` spelling as a mode", () => {
    // `manage.rs` used to emit `Debug`'s `"Default"`, which no option in the
    // Config page's dropdown matched. That spelling must not come back here as
    // a fourth value, and it must not be mistaken for `stable` either — an
    // unrecognized mode is not evidence the kernel is pinned.
    expect(selectKernelMode({ mode: "Default" })).toBeUndefined();
    expect(selectKernelMode({ mode: "Stable" })).toBeUndefined();
  });

  it("returns undefined rather than guessing when the payload carries no mode", () => {
    // The caller renders a warning off this. A `"default"` fallback would
    // suppress a real warning; an optimistic one would invent a false alert.
    expect(selectKernelMode({})).toBeUndefined();
    expect(selectKernelMode({ mode: null })).toBeUndefined();
    expect(selectKernelMode({ mode: 3 })).toBeUndefined();
    expect(selectKernelMode(undefined)).toBeUndefined();
    expect(selectKernelMode(null)).toBeUndefined();
  });

  it("is a projection of the full config, not a second subscription", () => {
    // The whole reason this lives on `full` is one cache entry and one
    // refetch; a factory with its own key would quietly double the traffic.
    expect(configQueries.kernelMode().queryKey).toEqual(configKeys.full());
  });
});

describe("useRegistrySchema", () => {
  it("should be disabled when contentType is empty string", () => {
    const { result } = renderHook(() => useRegistrySchema(""), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.fetchRegistrySchema).not.toHaveBeenCalled();
  });

  it("should be enabled when contentType is valid", async () => {
    const mockSchema: RegistrySchema = { fields: {} };
    vi.mocked(client.fetchRegistrySchema).mockResolvedValue(mockSchema);

    const { result } = renderHook(() => useRegistrySchema("application/json"), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockSchema);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.fetchRegistrySchema).toHaveBeenCalledWith("application/json");
  });

  it("should use registryKeys.schema(contentType) as queryKey", async () => {
    const mockSchema: RegistrySchema = { sections: {} };
    vi.mocked(client.fetchRegistrySchema).mockResolvedValue(mockSchema);

    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => useRegistrySchema("text/plain"), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryData(registryKeys.schema("text/plain"))).toEqual(mockSchema);
    });
  });
});

describe("useRawConfigToml", () => {
  it("keeps UI enablement out of the reusable query factory", () => {
    expect(configQueries.rawToml().enabled).toBeUndefined();
  });

  it("should not fetch when enabled is false", () => {
    const { result } = renderHook(() => useRawConfigToml(false), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getRawConfigToml).not.toHaveBeenCalled();
  });

  it("should fetch when enabled is true", async () => {
    const mockToml = "[kernel]\nlog_level = \"info\"";
    vi.mocked(client.getRawConfigToml).mockResolvedValue(mockToml);

    const { result } = renderHook(() => useRawConfigToml(true), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(result.current.isLoading).toBe(true);
    expect(result.current.fetchStatus).toBe("fetching");

    await waitFor(() => {
      expect(result.current.data).toEqual(mockToml);
    });

    expect(result.current.fetchStatus).toBe("idle");
    expect(client.getRawConfigToml).toHaveBeenCalled();
  });

  it("allows callers to disable an otherwise enabled read", () => {
    renderHook(() => useRawConfigToml(true, { enabled: false }), {
      wrapper: createQueryClientWrapper().wrapper,
    });

    expect(client.getRawConfigToml).not.toHaveBeenCalled();
  });

  it("should use configKeys.rawToml() as queryKey", async () => {
    const mockToml = "toml content";
    vi.mocked(client.getRawConfigToml).mockResolvedValue(mockToml);

    const { queryClient, wrapper } = createQueryClientWrapper();

    renderHook(() => useRawConfigToml(true), { wrapper });

    await waitFor(() => {
      expect(queryClient.getQueryData(configKeys.rawToml())).toEqual(mockToml);
    });
  });
});
