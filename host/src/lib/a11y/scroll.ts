const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

export function scrollTargetIntoView(target: Element | null): void {
  if (!target) return;

  const reduceMotion = typeof window.matchMedia === "function"
    && window.matchMedia(REDUCED_MOTION_QUERY).matches;
  target.scrollIntoView?.({
    block: "center",
    behavior: reduceMotion ? "auto" : "smooth",
  });
}
