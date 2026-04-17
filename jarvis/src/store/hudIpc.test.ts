import { describe, expect, it, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const { applyIpc } = vi.hoisted(() => ({
  applyIpc: vi.fn(),
}));

vi.mock("./hudStore", () => ({
  useHudStore: {
    getState: () => ({
      applyIpc: (...args: unknown[]) => applyIpc(...args),
    }),
  },
}));

import { subscribeHudIpc } from "./hudIpc";

describe("subscribeHudIpc", () => {
  beforeEach(() => {
    vi.mocked(listen).mockImplementation(() => Promise.resolve(() => {}));
    vi.mocked(invoke).mockResolvedValue("listening");
    applyIpc.mockClear();
  });

  it("registers every HUD channel in a single parallel await", async () => {
    await subscribeHudIpc();
    expect(listen).toHaveBeenCalledTimes(8);
  });

  it("applies hud_get_phase after listeners attach so phase is not stale", async () => {
    vi.mocked(invoke).mockResolvedValue("executing");
    await subscribeHudIpc();
    expect(invoke).toHaveBeenCalledWith("hud_get_phase");
    expect(applyIpc).toHaveBeenCalledWith("hud-phase", { phase: "executing" });
  });
});
