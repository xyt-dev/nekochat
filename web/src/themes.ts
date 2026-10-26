// Theme registry: built-in CSS themes plus custom ones served from the data directory.
import { createSignal } from "solid-js";
import { api } from "./api";
import "./themes/catppuccin-latte.css";
import "./themes/catppuccin-frappe.css";
import "./themes/catppuccin-macchiato.css";
import "./themes/catppuccin-mocha.css";
import "./themes/gruvbox-dark.css";
import "./themes/gruvbox-light.css";
import "./themes/claude.css";

export interface ThemeInfo {
  id: string;
  name: string;
  group: string;
  /** Background, accent and text: the three preview dots. */
  dots: [string, string, string];
  /** Stylesheet of a custom theme. */
  url?: string;
}

export const BUILTIN_THEMES: ThemeInfo[] = [
  { id: "catppuccin-latte", name: "Latte", group: "Catppuccin", dots: ["#eff1f5", "#8839ef", "#4c4f69"] },
  { id: "catppuccin-frappe", name: "Frappé", group: "Catppuccin", dots: ["#303446", "#ca9ee6", "#c6d0f5"] },
  { id: "catppuccin-macchiato", name: "Macchiato", group: "Catppuccin", dots: ["#24273a", "#c6a0f6", "#cad3f5"] },
  { id: "catppuccin-mocha", name: "Mocha", group: "Catppuccin", dots: ["#1e1e2e", "#cba6f7", "#cdd6f4"] },
  { id: "gruvbox-dark", name: "Dark", group: "Gruvbox", dots: ["#282828", "#fabd2f", "#ebdbb2"] },
  { id: "gruvbox-light", name: "Light", group: "Gruvbox", dots: ["#fbf1c7", "#af3a03", "#3c3836"] },
  { id: "claude", name: "Claude 娘", group: "其他", dots: ["#faf9f5", "#d97757", "#141413"] },
];

export const CUSTOM_GROUP = "自定义";
const LINK_ID = "nk-custom-theme";

const [customThemes, setCustomThemes] = createSignal<ThemeInfo[]>([]);
const [themeId, setThemeId] = createSignal(document.documentElement.dataset.theme ?? "catppuccin-mocha");
export { customThemes, themeId };

/** Reads the preview colors out of a custom stylesheet; unknown ones fall back to gray. */
function previewDots(css: string): [string, string, string] {
  const pick = (name: string) =>
    css.match(new RegExp(`--nk-${name}\\s*:\\s*([^;}]+)`))?.[1].trim() ?? "#888888";
  return [pick("bg"), pick("accent"), pick("text")];
}

export async function loadCustomThemes(): Promise<void> {
  let list;
  try {
    list = await api.themes();
  } catch {
    return;
  }
  const themes = await Promise.all(
    list.map(async (t): Promise<ThemeInfo> => {
      let dots: [string, string, string] = ["#888888", "#888888", "#888888"];
      try {
        const r = await fetch(t.url);
        if (r.ok) dots = previewDots(await r.text());
      } catch {
        // Keep gray dots; the theme can still be applied.
      }
      return { id: t.id, name: t.name, group: CUSTOM_GROUP, dots, url: t.url };
    }),
  );
  setCustomThemes(themes);
}

export function applyTheme(theme: ThemeInfo): void {
  document.getElementById(LINK_ID)?.remove();
  if (theme.url) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = theme.url;
    link.id = LINK_ID;
    document.head.appendChild(link);
    localStorage.setItem("nk.themeUrl", theme.url);
  } else {
    localStorage.removeItem("nk.themeUrl");
  }
  document.documentElement.dataset.theme = theme.id;
  localStorage.setItem("nk.theme", theme.id);
  setThemeId(theme.id);
}

export function allThemes(): ThemeInfo[] {
  return [...BUILTIN_THEMES, ...customThemes()];
}
