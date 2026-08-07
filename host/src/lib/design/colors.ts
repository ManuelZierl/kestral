// The single source of truth for every color in the frontend.
//
// Components never use color literals. They reference semantic CSS custom
// properties (`var(--color-*)`) that the theme store derives from the
// `ThemeColors` values below. The built-in registry and every saved custom
// profile contain this complete shape, so a theme cannot be partial.

/** Every semantic color slot in the design system. */
export interface ThemeColors {
  // Workspace surfaces
  bgGradientA: string; // main surface background gradient, top
  bgGradientB: string; // main surface background gradient, bottom
  surface: string; // cards on the workspace background
  surfaceRaised: string; // pure panels: chat column, inputs, buttons
  surfaceMuted: string; // quiet fills: thread list, code chips, muted rows
  surfaceHover: string; // hover/active fill for list rows

  // Borders
  border: string; // card borders
  borderStrong: string; // input borders
  borderSubtle: string; // hairlines inside panels (chat, table rows)
  borderHover: string; // hovered/focused hairlines

  // Text
  text: string; // primary text
  textMuted: string; // secondary text on workspace surfaces
  textSoft: string; // secondary text on raised surfaces (chat)
  textFaint: string; // timestamps, placeholders, hints

  // Accent (interactive / informational blue)
  accent: string;
  accentStrong: string;
  accentContrast: string; // text/icon on accent fills
  accentSoft: string; // soft accent fills (info banners, chips)
  accentText: string; // readable accent-colored text
  accentBorder: string; // selected-item borders
  // Keyboard focus indicator. Held apart from `accent` because a focus ring
  // must clear WCAG 1.4.11 (3:1) against whatever it sits on, independent of
  // how the accent fill is later retuned.
  focusRing: string;

  // Status
  successText: string;
  successSoft: string;
  successBorder: string;
  warningText: string;
  warningSoft: string;
  warningBorder: string;
  dangerText: string;
  dangerSoft: string;
  dangerBorder: string;

  // Reading insights annotations
  commentText: string;
  commentSoft: string;
  commentBorder: string;

  // Decorative chip fills that are neither accent nor status
  chipPurpleSoft: string;

  // Bars framing the workspace (top bar, status bar)
  barBg: string;
  barBorder: string;

  // Shadows and overlays
  shadowSoft: string;
  shadowStrong: string;
  scrim: string;

  // App sidebar (built-in palettes follow the active Light/Dark theme)
  sidebarBgA: string;
  sidebarBgB: string;
  sidebarBorder: string;
  sidebarText: string;
  sidebarTextMuted: string;
  sidebarTextFaint: string; // redundant/decorative icons; readable labels use muted
  sidebarHover: string;
  sidebarActiveA: string;
  sidebarActiveB: string;
  sidebarActiveBorder: string;
  sidebarCardBg: string;
  sidebarCardBorder: string;
  brandGradientB: string;
  // The Kestral mark: a monochrome bird on an inverted monochrome tile —
  // near-black on near-white in the light theme, swapped in the dark theme.
  brandMark: string;
  brandMarkBg: string;

  // Trusted chrome. The amber-on-dark identity is the trust boundary's
  // visual signature and stays identical across themes on purpose.
  chromeBg: string;
  chromeText: string;
  chromeTextMuted: string;
  chromeAccent: string;
  chromeAccentContrast: string;
  chromeApprove: string;
  chromeApproveText: string;
  chromeDeny: string;
  // A raised panel inside the dark dialog (e.g. the grant-scope box). Kept
  // theme-constant like the rest of chrome, so chromeText stays readable on it
  // in both themes — unlike the badge tokens, which are a light accent.
  chromePanelBg: string;
  chromePanelBorder: string;
}

/** Ids of the immutable built-in themes. */
export type ThemeId = "light" | "dark";

const lightSidebar = {
  sidebarBgA: "rgba(246, 246, 246, 0.98)",
  sidebarBgB: "rgba(238, 239, 240, 0.98)",
  sidebarBorder: "rgba(182, 186, 191, 0.58)",
  sidebarText: "#252525",
  sidebarTextMuted: "#64696e",
  sidebarTextFaint: "#85898d",
  sidebarHover: "rgba(63, 88, 115, 0.06)",
  sidebarActiveA: "rgba(82, 111, 145, 0.12)",
  sidebarActiveB: "rgba(82, 111, 145, 0.17)",
  sidebarActiveBorder: "rgba(82, 111, 145, 0.3)",
  sidebarCardBg: "rgba(255, 255, 255, 0.58)",
  sidebarCardBorder: "rgba(164, 170, 176, 0.42)",
  brandGradientB: "#667f9b",
  brandMark: "#292929",
  brandMarkBg: "#edf1f4",
} satisfies Partial<ThemeColors>;

const darkSidebar = {
  sidebarBgA: "rgba(24, 25, 26, 0.98)",
  sidebarBgB: "rgba(18, 19, 20, 0.98)",
  sidebarBorder: "rgba(78, 82, 87, 0.68)",
  sidebarText: "#e7e7e7",
  sidebarTextMuted: "#a9adb2",
  sidebarTextFaint: "#83888d",
  sidebarHover: "rgba(142, 172, 205, 0.08)",
  sidebarActiveA: "rgba(142, 172, 205, 0.15)",
  sidebarActiveB: "rgba(142, 172, 205, 0.21)",
  sidebarActiveBorder: "rgba(142, 172, 205, 0.34)",
  sidebarCardBg: "rgba(255, 255, 255, 0.04)",
  sidebarCardBorder: "rgba(145, 151, 158, 0.3)",
  brandGradientB: "#829db9",
  brandMark: "#e5e5e5",
  brandMarkBg: "#2d3237",
} satisfies Partial<ThemeColors>;

const chrome = {
  chromeBg: "#0f1420",
  chromeText: "#edf1ff",
  chromeTextMuted: "#c4cce4",
  chromeAccent: "#ffb454",
  chromeAccentContrast: "#1a1200",
  chromeApprove: "#4db86a",
  // Dark ink on the green approve fill: near-white text sits at ~2.5:1 (below
  // the WCAG AA floor), this lands well above 4.5:1 on the primary CTA.
  chromeApproveText: "#0c1f13",
  chromeDeny: "#2f3648",
  // Slightly lighter than chromeBg (#0f1420) so panels read as raised while
  // keeping near-white chromeText well above the AA contrast floor.
  chromePanelBg: "#1b2233",
  chromePanelBorder: "#2f3a52",
} satisfies Partial<ThemeColors>;

const lightTheme: ThemeColors = {
  bgGradientA: "#f3f4f5",
  bgGradientB: "#fafafa",
  surface: "rgba(255, 255, 255, 0.9)",
  surfaceRaised: "#ffffff",
  surfaceMuted: "#f5f5f5",
  surfaceHover: "#eeeeee",

  border: "#dddddd",
  borderStrong: "#cccccc",
  borderSubtle: "#eaeaea",
  borderHover: "#bfc5cc",

  text: "#202020",
  textMuted: "#62676d",
  textSoft: "#696d72",
  textFaint: "#6d7176",

  accent: "#526f91",
  accentStrong: "#3d5877",
  accentContrast: "#ffffff",
  accentSoft: "#eaf0f6",
  accentText: "#405f80",
  accentBorder: "#a8bacd",
  focusRing: "#58789d",

  successText: "#39704b",
  successSoft: "#edf6f0",
  successBorder: "#b8d8c2",
  warningText: "#8a6222",
  warningSoft: "#fbf5e9",
  warningBorder: "#e1c991",
  dangerText: "#a33f45",
  dangerSoft: "#fbecee",
  dangerBorder: "#e6b1b5",

  commentText: "#725c32",
  commentSoft: "#f8f2e7",
  commentBorder: "#ddcca5",

  chipPurpleSoft: "#f0edf6",

  barBg: "rgba(250, 250, 250, 0.94)",
  barBorder: "rgba(175, 180, 186, 0.58)",

  shadowSoft: "rgba(24, 32, 40, 0.06)",
  shadowStrong: "rgba(20, 25, 30, 0.13)",
  scrim: "rgba(20, 22, 26, 0.58)",

  ...lightSidebar,
  ...chrome,
};

const darkTheme: ThemeColors = {
  bgGradientA: "#18191a",
  bgGradientB: "#111213",
  surface: "rgba(31, 32, 33, 0.94)",
  surfaceRaised: "#282a2c",
  surfaceMuted: "#222426",
  surfaceHover: "#303337",

  border: "#3b3e42",
  borderStrong: "#4d5156",
  borderSubtle: "#303236",
  borderHover: "#636a72",

  text: "#ededed",
  textMuted: "#b6bac0",
  textSoft: "#999ea4",
  textFaint: "#8b9197",

  accent: "#8eaccd",
  accentStrong: "#b3c9df",
  accentContrast: "#17202a",
  accentSoft: "#293542",
  accentText: "#b6cce2",
  accentBorder: "#596f85",
  focusRing: "#9ab8d8",

  successText: "#9bc7a6",
  successSoft: "#233229",
  successBorder: "#44604b",
  warningText: "#d6b774",
  warningSoft: "#372f20",
  warningBorder: "#6e5932",
  dangerText: "#e5a2a7",
  dangerSoft: "#402629",
  dangerBorder: "#75454a",

  commentText: "#d0ba85",
  commentSoft: "#353025",
  commentBorder: "#685b3d",

  chipPurpleSoft: "#332e3b",

  barBg: "rgba(25, 26, 27, 0.96)",
  barBorder: "rgba(99, 104, 110, 0.6)",

  shadowSoft: "rgba(0, 0, 0, 0.26)",
  shadowStrong: "rgba(0, 0, 0, 0.48)",
  scrim: "rgba(0, 0, 0, 0.74)",

  ...darkSidebar,
  ...chrome,
};

/** The immutable built-in theme registry. */
export const themes: Record<ThemeId, ThemeColors> = {
  light: lightTheme,
  dark: darkTheme,
};

export type ThemeColorToken = keyof ThemeColors;

export interface ThemeColorGroup {
  id: string;
  label: string;
  description: string;
  tokens: readonly ThemeColorToken[];
}

/** Editor groups mirror the semantic sections in `ThemeColors`. */
export const themeColorGroups: readonly ThemeColorGroup[] = [
  {
    id: "workspace",
    label: "Workspace surfaces",
    description: "Page backgrounds, cards, panels, and hovered rows.",
    tokens: ["bgGradientA", "bgGradientB", "surface", "surfaceRaised", "surfaceMuted", "surfaceHover"],
  },
  {
    id: "borders",
    label: "Borders",
    description: "Card, input, divider, and hover outlines.",
    tokens: ["border", "borderStrong", "borderSubtle", "borderHover"],
  },
  {
    id: "text",
    label: "Text",
    description: "Primary copy, supporting text, timestamps, and hints.",
    tokens: ["text", "textMuted", "textSoft", "textFaint"],
  },
  {
    id: "accent",
    label: "Accent and focus",
    description: "Interactive controls, information highlights, and keyboard focus.",
    tokens: ["accent", "accentStrong", "accentContrast", "accentSoft", "accentText", "accentBorder", "focusRing"],
  },
  {
    id: "status",
    label: "Status colors",
    description: "Success, warning, and danger messages and their borders.",
    tokens: [
      "successText", "successSoft", "successBorder",
      "warningText", "warningSoft", "warningBorder",
      "dangerText", "dangerSoft", "dangerBorder",
    ],
  },
  {
    id: "annotations",
    label: "Annotations and chips",
    description: "Reading annotations and decorative chip fills.",
    tokens: ["commentText", "commentSoft", "commentBorder", "chipPurpleSoft"],
  },
  {
    id: "framing",
    label: "Bars, shadows, and overlays",
    description: "Workspace framing, depth, and modal backdrops.",
    tokens: ["barBg", "barBorder", "shadowSoft", "shadowStrong", "scrim"],
  },
  {
    id: "sidebar",
    label: "App sidebar",
    description: "Navigation background, labels, active states, cards, and brand mark.",
    tokens: [
      "sidebarBgA", "sidebarBgB", "sidebarBorder", "sidebarText", "sidebarTextMuted",
      "sidebarTextFaint", "sidebarHover", "sidebarActiveA", "sidebarActiveB",
      "sidebarActiveBorder", "sidebarCardBg", "sidebarCardBorder", "brandGradientB",
      "brandMark", "brandMarkBg",
    ],
  },
  {
    id: "chrome",
    label: "Trusted chrome",
    description: "Host-owned approval dialogs and notices. Change these with extra care.",
    tokens: [
      "chromeBg", "chromeText", "chromeTextMuted", "chromeAccent", "chromeAccentContrast",
      "chromeApprove", "chromeApproveText", "chromeDeny", "chromePanelBg", "chromePanelBorder",
    ],
  },
];

export const themeColorTokens = themeColorGroups.flatMap((group) => group.tokens);

/** `bgGradientA` → `Background gradient start` */
export function themeColorLabel(token: ThemeColorToken): string {
  const words = token.replace(/([a-z])([A-Z])/g, "$1 $2").split(" ");
  const label = words.map((word) => {
    if (word.toLowerCase() === "bg") return "background";
    if (word === "A") return "start";
    if (word === "B") return "end";
    return word.toLowerCase();
  }).join(" ");
  return label[0].toUpperCase() + label.slice(1);
}

/** `surfaceRaised` → `--color-surface-raised` */
export function cssVariableName(token: keyof ThemeColors): string {
  return `--color-${token.replace(/([A-Z])/g, "-$1").toLowerCase()}`;
}

/** Flatten a theme into `{ "--color-*": value }` for style application. */
export function themeCssVariables(colors: ThemeColors): Record<string, string> {
  return Object.fromEntries(
    (Object.keys(colors) as (keyof ThemeColors)[]).map((token) => [
      cssVariableName(token),
      colors[token],
    ]),
  );
}
