import { describe, expect, it } from "vitest";

import { dataScopeLabel } from "$lib/system/dataScopeLabel";

describe("dataScopeLabel", () => {
  it("names the unscoped form without implying broad resource access", () => {
    expect(dataScopeLabel({ kind: "none" })).toBe("Not tied to a registered resource");
  });

  it("lists resource ids for scoped grants", () => {
    expect(
      dataScopeLabel({ kind: "resources", resource_ids: ["note-1", "note-2"] }),
    ).toBe("Resources: note-1, note-2");
  });

  it("makes all current and future resources explicit", () => {
    expect(dataScopeLabel({ kind: "all-resources" })).toBe(
      "All current and future resources",
    );
  });
});
