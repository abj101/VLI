import { describe, expect, it } from "vitest";
import { searchAppIndexInvokeArgs } from "./appIndexInvoke";

describe("searchAppIndexInvokeArgs", () => {
  it("nests query and limit under payload for Tauri command deserialization", () => {
    expect(searchAppIndexInvokeArgs("notepad", 24)).toEqual({
      payload: { query: "notepad", limit: 24 },
    });
  });
});
