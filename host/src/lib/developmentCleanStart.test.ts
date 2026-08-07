import { beforeEach, expect, it } from "vitest";

import { applyDevelopmentCleanStart } from "$lib/developmentCleanStart";

beforeEach(() => localStorage.clear());

it("removes browser-local Kestral state once for each clean launch", () => {
  localStorage.setItem("host-theme-preference", "dark");
  localStorage.setItem("host-custom-theme-profiles", "saved theme");
  localStorage.setItem("host-sidebar-layout", "saved sidebar");
  localStorage.setItem("kernel.active-chat-thread", "thread-1");
  localStorage.setItem("kernel.pending-chat-sends", "pending send");
  localStorage.setItem("unrelated", "keep");

  expect(applyDevelopmentCleanStart(localStorage, "clean-1")).toBe(true);
  expect(localStorage.getItem("host-theme-preference")).toBeNull();
  expect(localStorage.getItem("host-custom-theme-profiles")).toBeNull();
  expect(localStorage.getItem("host-sidebar-layout")).toBeNull();
  expect(localStorage.getItem("kernel.active-chat-thread")).toBeNull();
  expect(localStorage.getItem("kernel.pending-chat-sends")).toBeNull();
  expect(localStorage.getItem("unrelated")).toBe("keep");

  localStorage.setItem("host-theme-preference", "light");
  expect(applyDevelopmentCleanStart(localStorage, "clean-1")).toBe(false);
  expect(localStorage.getItem("host-theme-preference")).toBe("light");
  expect(applyDevelopmentCleanStart(localStorage, "clean-2")).toBe(true);
  expect(localStorage.getItem("host-theme-preference")).toBeNull();
});
