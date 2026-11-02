// SPDX-License-Identifier: Apache-2.0 OR MIT
/** Views of core-validated graphics v1; validation belongs to Rust/WASM. */
export type SemanticColor = "accent" | "text" | "muted" | "surface" | "border" | "red" | "orange" | "yellow" | "green" | "teal" | "blue" | "purple" | "pink";
export type Color = SemanticColor | `#${string}`;
export type ChartKind = "line" | "bar" | "area" | "donut";

export interface GraphicSpec {
  version: 1;
  title?: string;
  gap?: number;
  items: GraphicNode[];
}

export interface NodeBase { id?: string; title?: string; span?: number }
export interface TextNode extends NodeBase { type: "text"; content: string; size?: "body" | "h2" | "h3" | "muted"; color?: Color; palette?: boolean }
export interface StatNode extends NodeBase { type: "stat"; label: string; value: string | number; delta?: string; color?: Color; palette?: boolean }
export interface QuestionChoice { label: string; prompt: string }
export interface ChoicesNode extends NodeBase { type: "choices"; options: QuestionChoice[] }
export interface LayoutNode extends NodeBase { type: "row" | "col" | "grid" | "card"; items: GraphicNode[]; gap?: number; cols?: number; color?: Color; palette?: boolean }
export interface ChartSeries { label: string; values: number[]; color?: Color }
export interface ChartNode extends NodeBase { type: "chart"; kind: ChartKind; labels: string[]; series: ChartSeries[]; height?: number; palette?: boolean }
export interface PlotParam { name: string; label?: string; value: number; min: number; max: number; step?: number }
export interface PlotSeries { label?: string; expr: string; color?: Color }
export interface PlotNode extends NodeBase { type: "plot"; xMin: number; xMax: number; yMin?: number; yMax?: number; height?: number; params?: PlotParam[]; series: PlotSeries[]; palette?: boolean }
export interface DiagramItem { id: string; label: string; x: number; y: number; w?: number; h?: number; description?: string; color?: Color }
export interface DiagramEdge { from: string; to: string; label?: string }
export interface DiagramNode extends NodeBase { type: "diagram"; width?: number; height?: number; nodes: DiagramItem[]; edges: DiagramEdge[]; color?: Color; palette?: boolean }
export interface SvgNode extends NodeBase { type: "svg"; code: string; height?: number; palette?: boolean }
export type GraphicNode = TextNode | StatNode | ChoicesNode | LayoutNode | ChartNode | PlotNode | DiagramNode | SvgNode;

export interface PlotSample {
  xMin: number;
  xMax: number;
  yMin: number;
  yMax: number;
  series: { label: string; points: [number, number | null][] }[];
}

export interface PlotRequest { client: number; revision: number; source: string; values: string }
export type PlotResponse = { client: number; revision: number; sample: PlotSample } | { client: number; revision: number; error: string };
