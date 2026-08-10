import { get } from "svelte/store";
import { beforeEach, describe, expect, it } from "vitest";

import {
  arrangeSidebarDestinations,
  parseStoredSidebarLayout,
  resetSidebarLayout,
  setSidebarDestinationHidden,
  setSidebarDestinationOrder,
  sidebarLayout,
  SIDEBAR_LAYOUT_STORAGE_KEY,
  type SidebarDestination,
} from "$lib/stores/sidebarLayout";

const destinations: SidebarDestination[] = [
  { id: "host:chat", label: "Chat", kind: "host" },
  { id: "host:apps", label: "Apps", kind: "host" },
  { id: "app:com.example.notes", label: "Notes", kind: "app" },
];

beforeEach(() => {
  localStorage.clear();
  resetSidebarLayout();
});

describe("sidebar layout", () => {
  it("parses the exact versioned representation", () => {
    expect(parseStoredSidebarLayout(JSON.stringify({
      version: 2,
      collapsed: true,
      order: ["app:mcp-my_server/path", "host:chat"],
      hidden: ["host:apps"],
    }))).toEqual({
      collapsed: true,
      order: ["app:mcp-my_server/path", "host:chat"],
      hidden: ["host:apps"],
    });
  });

  it("migrates v1 layouts to hide the seeded documentation tool by default", () => {
    expect(parseStoredSidebarLayout(JSON.stringify({
      version: 1,
      collapsed: true,
      order: ["host:chat"],
      hidden: ["host:apps"],
    }))).toEqual({
      collapsed: true,
      order: ["host:chat"],
      hidden: ["host:apps", "app:mcp-kestral-docs"],
    });
  });

  it.each([
    { version: 3, collapsed: false, order: [], hidden: [] },
    { version: 1, collapsed: false, order: ["host:chat", "host:chat"], hidden: [] },
    { version: 1, collapsed: false, order: ["unknown"], hidden: [] },
    { version: 1, collapsed: false, order: [], hidden: [], extra: true },
  ])("rejects malformed saved state", (value) => {
    expect(() => parseStoredSidebarLayout(JSON.stringify(value))).toThrow();
  });

  it("applies saved order and appends destinations installed later", () => {
    const layout = {
      collapsed: false,
      order: ["app:com.example.notes", "host:chat", "app:com.example.missing"],
      hidden: [],
    };

    expect(arrangeSidebarDestinations(destinations, layout).map(({ id }) => id)).toEqual([
      "app:com.example.notes",
      "host:chat",
      "host:apps",
    ]);
  });

  it("persists visibility and ordering changes", () => {
    setSidebarDestinationHidden("host:apps", true);
    setSidebarDestinationOrder(["app:com.example.notes", "host:chat", "host:apps"]);

    expect(get(sidebarLayout)).toEqual({
      collapsed: false,
      order: ["app:com.example.notes", "host:chat", "host:apps"],
      hidden: ["app:mcp-kestral-docs", "host:apps"],
    });
    expect(JSON.parse(localStorage.getItem(SIDEBAR_LAYOUT_STORAGE_KEY)!)).toEqual({
      version: 2,
      collapsed: false,
      order: ["app:com.example.notes", "host:chat", "host:apps"],
      hidden: ["app:mcp-kestral-docs", "host:apps"],
    });
  });
});
