// Typed blocks exchanged with the Markdown worker. Graphics are validated by core/WASM.
import type { GraphicSpec } from "../graphics/types";

export type MdBlock =
  | { kind: "html"; html: string }
  | { kind: "graphic"; spec: GraphicSpec; source: string }
  | { kind: "graphic-pending" }
  | { kind: "graphic-error"; error: string; source: string };

export interface MdRequest {
  key: string;
  text: string;
  final: boolean;
  /** Blocks the main thread holds; unchanged leading blocks are not resent. */
  base: number;
  /** Text nodes render Markdown/code without opening nested interactive graphics. */
  graphics?: boolean;
  drop?: boolean;
}

export interface MdResponse {
  key: string;
  total: number;
  start: number;
  blocks: MdBlock[];
  dropped?: boolean;
}
