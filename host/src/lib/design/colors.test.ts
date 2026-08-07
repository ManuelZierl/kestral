import { describe, expect, it } from "vitest";

import { themeColorGroups, themeColorTokens, themes, type ThemeColors, type ThemeId } from "$lib/design/colors";

// The token set is the design system's contract, so contrast is checked here
// rather than left to a manual pass: a token can be retuned in one theme and
// quietly drop text below the readable floor in the other.

type Rgb = [number, number, number, number];

function parse(color: string): Rgb {
  const value = color.trim();
  if (value.startsWith("#")) {
    const hex = value.length === 4
      ? [...value.slice(1)].map((digit) => digit + digit).join("")
      : value.slice(1);
    return [
      parseInt(hex.slice(0, 2), 16),
      parseInt(hex.slice(2, 4), 16),
      parseInt(hex.slice(4, 6), 16),
      1,
    ];
  }
  const parts = value
    .slice(value.indexOf("(") + 1, value.lastIndexOf(")"))
    .split(",")
    .map((part) => Number.parseFloat(part.trim()));
  return [parts[0], parts[1], parts[2], parts.length > 3 ? parts[3] : 1];
}

/** Flatten a possibly-translucent color onto an opaque backdrop. */
function over(color: string, backdrop: Rgb): Rgb {
  const [r, g, b, alpha] = parse(color);
  return [
    r * alpha + backdrop[0] * (1 - alpha),
    g * alpha + backdrop[1] * (1 - alpha),
    b * alpha + backdrop[2] * (1 - alpha),
    1,
  ];
}

function relativeLuminance([r, g, b]: Rgb): number {
  const [rl, gl, bl] = [r, g, b].map((channel) => {
    const srgb = channel / 255;
    return srgb <= 0.03928 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * rl + 0.7152 * gl + 0.0722 * bl;
}

function contrast(foreground: Rgb, background: Rgb): number {
  const a = relativeLuminance(foreground);
  const b = relativeLuminance(background);
  const [lighter, darker] = a > b ? [a, b] : [b, a];
  return (lighter + 0.05) / (darker + 0.05);
}

interface Pair {
  foreground: keyof ThemeColors;
  background: keyof ThemeColors;
  /** The zone the pair renders in; decides what translucency flattens onto. */
  zone?: "workspace" | "sidebar";
  label: string;
}

// WCAG 2.1 §1.4.3 (Contrast Minimum) — 4.5:1 for normal-size body text.
const TEXT_PAIRS: Pair[] = [
  { foreground: "text", background: "surfaceRaised", label: "body text on panels" },
  { foreground: "text", background: "surface", label: "body text on cards" },
  { foreground: "text", background: "surfaceMuted", label: "body text on muted fills" },
  { foreground: "text", background: "surfaceHover", label: "body text on hovered rows" },
  { foreground: "text", background: "bgGradientB", label: "body text on workspace" },
  { foreground: "textMuted", background: "surface", label: "secondary text on cards" },
  { foreground: "textMuted", background: "surfaceRaised", label: "secondary text on panels" },
  { foreground: "textMuted", background: "bgGradientB", label: "secondary text on workspace" },
  { foreground: "textSoft", background: "surfaceRaised", label: "chat secondary text" },
  { foreground: "textSoft", background: "surfaceMuted", label: "chat secondary on muted" },
  // Timestamps, hints and placeholders are body text for contrast purposes.
  { foreground: "textFaint", background: "surfaceRaised", label: "timestamps and placeholders" },
  { foreground: "textFaint", background: "surfaceMuted", label: "hints on muted fills" },
  { foreground: "accent", background: "surfaceRaised", label: "accent text on panels" },
  { foreground: "accentText", background: "surfaceRaised", label: "accent-colored text" },
  { foreground: "accentText", background: "accentSoft", label: "accent text in info banners" },
  { foreground: "accentContrast", background: "accent", label: "label on accent button" },
  { foreground: "accentContrast", background: "accentStrong", label: "label on accent hover" },
  { foreground: "successText", background: "successSoft", label: "success chip" },
  { foreground: "warningText", background: "warningSoft", label: "warning chip" },
  { foreground: "dangerText", background: "dangerSoft", label: "danger chip" },
  { foreground: "commentText", background: "commentSoft", label: "annotation chip" },
  { foreground: "successText", background: "surfaceRaised", label: "success text on panels" },
  { foreground: "warningText", background: "surfaceRaised", label: "warning text on panels" },
  { foreground: "dangerText", background: "surfaceRaised", label: "danger text on panels" },
  { foreground: "sidebarText", background: "sidebarBgB", zone: "sidebar", label: "sidebar label" },
  { foreground: "sidebarTextMuted", background: "sidebarBgB", zone: "sidebar", label: "sidebar muted label" },
  // Faint sidebar color is limited to redundant, aria-hidden icons.
  { foreground: "sidebarText", background: "sidebarCardBg", zone: "sidebar", label: "sidebar card text" },
  { foreground: "chromeText", background: "chromeBg", label: "trusted chrome body" },
  { foreground: "chromeTextMuted", background: "chromeBg", label: "trusted chrome secondary" },
  { foreground: "chromeText", background: "chromePanelBg", label: "chrome scope panel" },
  { foreground: "chromeTextMuted", background: "chromePanelBg", label: "chrome scope panel secondary" },
  { foreground: "chromeAccent", background: "chromeBg", label: "chrome accent text" },
  { foreground: "chromeApproveText", background: "chromeApprove", label: "Approve button label" },
  { foreground: "chromeText", background: "chromeDeny", label: "Deny button label" },
];

// WCAG 2.1 §1.4.11 (Non-text Contrast) — 3:1 for indicators that identify a
// control or its state. The focus ring is the load-bearing one: it is the only
// thing telling a keyboard user where they are.
const INDICATOR_PAIRS: Pair[] = [
  { foreground: "focusRing", background: "surfaceRaised", label: "focus ring on panels" },
  { foreground: "focusRing", background: "surfaceMuted", label: "focus ring on muted fills" },
  { foreground: "focusRing", background: "surface", label: "focus ring on cards" },
];

function ratioFor(colors: ThemeColors, pair: Pair): number {
  const page = parse(colors.bgGradientB);
  const backdrop = pair.zone === "sidebar" ? over(colors.sidebarBgB, page) : page;
  const background = over(colors[pair.background], backdrop);
  const foreground = over(colors[pair.foreground], background);
  return contrast(foreground, background);
}

describe.each(Object.keys(themes) as ThemeId[])("%s theme", (themeId) => {
  const colors = themes[themeId];

  it.each(TEXT_PAIRS)(
    "renders $label at the AA text contrast floor",
    (pair) => {
      expect(ratioFor(colors, pair)).toBeGreaterThanOrEqual(4.5);
    },
  );

  it.each(INDICATOR_PAIRS)(
    "renders $label at the non-text contrast floor",
    (pair) => {
      expect(ratioFor(colors, pair)).toBeGreaterThanOrEqual(3);
    },
  );
});

describe("theme registry", () => {
  it("gives every theme a value for every token", () => {
    const tokens = Object.keys(themes.light) as (keyof ThemeColors)[];
    for (const [themeId, colors] of Object.entries(themes)) {
      for (const token of tokens) {
        expect(colors[token], `${themeId}.${String(token)}`).toBeTruthy();
      }
      expect(Object.keys(colors).sort()).toEqual([...tokens].sort());
    }
  });

  it("lists every token exactly once in the custom profile editor", () => {
    const tokens = Object.keys(themes.light).sort();
    expect([...themeColorTokens].sort()).toEqual(tokens);
    expect(new Set(themeColorTokens).size).toBe(tokens.length);
    expect(themeColorGroups.every((group) => group.tokens.length > 0)).toBe(true);
  });

  it("keeps built-in sidebars distinct and trusted chrome shared", () => {
    const sidebar = themeColorGroups.find((group) => group.id === "sidebar");
    const chrome = themeColorGroups.find((group) => group.id === "chrome");
    if (!sidebar || !chrome) throw new Error("theme color groups are incomplete");

    for (const token of sidebar.tokens) {
      expect(themes.light[token], token).not.toBe(themes.dark[token]);
    }
    for (const token of chrome.tokens) {
      expect(themes.light[token], token).toBe(themes.dark[token]);
    }
  });
});
