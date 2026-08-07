import { describe, expect, it } from "vitest";
import { scopeLabel } from "$lib/system/scopeLabel";

describe("scopeLabel", () => {
  it("formats exact and provider scopes", () => {
    expect(scopeLabel({ kind: "exact-capability", provider: "notes", capability: "create_note" })).toBe("notes/create_note");
    expect(scopeLabel({ kind: "all-provider-capabilities", provider: "notes" })).toBe("notes/*");
  });
});
