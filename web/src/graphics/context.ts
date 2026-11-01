// SPDX-License-Identifier: Apache-2.0 OR MIT
import { createContext, useContext, type Accessor } from "solid-js";
import type { Color } from "./types";
import type { GraphicState } from "./state";

export interface GraphicsContextValue {
  id: Accessor<string>;
  state: Accessor<GraphicState>;
  themeRevision: Accessor<number>;
  color: (key: string, declared?: Color, fallback?: Color) => string;
  setColor: (key: string, color: string) => void;
  themeColor: (key: string) => void;
  param: (key: string, fallback: number) => number;
  setParam: (key: string, value: number) => void;
  view: (key: string, fallback: number) => number;
  setView: (key: string, value: number) => void;
  followsTheme: (key: string) => boolean;
  canQuestion: Accessor<boolean>;
  chooseQuestion: (prompt: string) => void;
}
export const GraphicsContext = createContext<GraphicsContextValue>();
export function useGraphics(): GraphicsContextValue {
  const context = useContext(GraphicsContext);
  if (!context) throw new Error("Graphic node rendered outside GraphicBlock");
  return context;
}
export const SERIES_COLORS: Color[] = ["accent", "blue", "green", "orange", "purple", "teal", "pink", "red"];
export function cssColor(color: Color = "accent"): string {
  if (color.startsWith("#")) return color;
  return `var(--nk-${color === "muted" ? "text-muted" : color})`;
}
