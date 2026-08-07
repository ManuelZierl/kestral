import { afterEach, describe, expect, it, vi } from "vitest";

import { scrollTargetIntoView } from "$lib/a11y/scroll";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("scrollTargetIntoView", () => {
  it("uses smooth scrolling by default", () => {
    const scrollIntoView = vi.fn();
    vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false })));

    scrollTargetIntoView({ scrollIntoView } as unknown as Element);

    expect(scrollIntoView).toHaveBeenCalledWith({ block: "center", behavior: "smooth" });
  });

  it("avoids animation when reduced motion is requested", () => {
    const scrollIntoView = vi.fn();
    vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: true })));

    scrollTargetIntoView({ scrollIntoView } as unknown as Element);

    expect(scrollIntoView).toHaveBeenCalledWith({ block: "center", behavior: "auto" });
  });
});
