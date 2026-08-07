import { describe, expect, it } from "vitest";

import {
  secretInputPlaceholder,
  secretStatusAfterClear,
  secretStatusAfterSave,
  secretStatusFromPresence,
  secretStatusLabel,
} from "$lib/settings/secretInputModel";

describe("secretInputModel", () => {
  it("maps stored presence to set and not-set states", () => {
    expect(secretStatusFromPresence(true)).toBe("set");
    expect(secretStatusFromPresence(false)).toBe("not-set");
  });

  it("keeps save and clear transitions explicit", () => {
    expect(secretStatusAfterSave(true)).toBe("updated-now");
    expect(secretStatusAfterSave(false)).toBe("error");
    expect(secretStatusAfterClear(false)).toBe("not-set");
    expect(secretStatusAfterClear(true)).toBe("error");
  });

  it("renders obvious status labels without exposing secret values", () => {
    expect(secretStatusLabel("set")).toBe("Set");
    expect(secretStatusLabel("updated-now")).toBe("Updated just now");
    expect(secretStatusLabel("not-set")).toBe("Not set");
    expect(secretStatusLabel("error")).toBe("Error");
    expect(secretInputPlaceholder("set")).toBe("Secret is set");
    expect(secretInputPlaceholder("not-set")).toBe("Enter secret");
  });
});
