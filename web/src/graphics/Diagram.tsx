// SPDX-License-Identifier: Apache-2.0 OR MIT
import { createMemo, createUniqueId, For, Show } from "solid-js";
import type { DiagramNode } from "./types";
import { useGraphics } from "./context";
import { ColorControl, ColorSettings, useCanvasWidth } from "./controls";
import { stateKey } from "./state";

export default function Diagram(props: { node: DiagramNode; nodeKey: string }) {
  const context = useGraphics();
  const canvas = useCanvasWidth();
  const marker = `nk-arrow-${createUniqueId()}`;
  const focusNodes = new Map<number, SVGGElement>();
  const selected = createMemo(() => {
    const identity = context.view(`${props.nodeKey}:selection`, 0);
    return props.node.nodes.findIndex(node => Number.parseInt(stateKey(node.id).slice(0, 13), 16) + 1 === identity);
  });
  function select(index: number): void {
    context.setView(`${props.nodeKey}:selection`, index < 0 ? 0 : Number.parseInt(stateKey(props.node.nodes[index].id).slice(0, 13), 16) + 1);
  }
  const height = () => props.node.height ?? 320;
  const geometry = createMemo(() => {
    // Normalize before adding extents so even valid extreme finite coordinates stay finite.
    let scale = Math.max(1, props.node.width ?? 640, props.node.height ?? 320);
    for (const node of props.node.nodes) scale = Math.max(scale, Math.abs(node.x), Math.abs(node.y), node.w ?? 140, node.h ?? 56);
    const nodes = props.node.nodes.map(node => ({ node, x: node.x / scale, y: node.y / scale, w: (node.w ?? 140) / scale, h: (node.h ?? 56) / scale }));
    let minX = 0, minY = 0, maxX = (props.node.width ?? 640) / scale, maxY = (props.node.height ?? 320) / scale;
    for (const node of nodes) { minX = Math.min(minX, node.x); minY = Math.min(minY, node.y); maxX = Math.max(maxX, node.x + node.w); maxY = Math.max(maxY, node.y + node.h); }
    const extentX = Math.max(Number.MIN_VALUE, maxX - minX), extentY = Math.max(Number.MIN_VALUE, maxY - minY);
    const factor = Math.min((canvas.width() - 48) / extentX, (height() - 48) / extentY);
    const left = (canvas.width() - extentX * factor) / 2, top = (height() - extentY * factor) / 2;
    return nodes.map(item => ({ ...item, x: left + (item.x - minX) * factor, y: top + (item.y - minY) * factor, w: Math.max(1, item.w * factor), h: Math.max(1, item.h * factor) }));
  });
  const edges = createMemo(() => {
    const byId = new Map(geometry().map(item => [item.node.id, item]));
    return props.node.edges.flatMap(edge => {
      const from = byId.get(edge.from), to = byId.get(edge.to);
      if (!from || !to) return [];
      const ax = from.x + from.w / 2, ay = from.y + from.h / 2, bx = to.x + to.w / 2, by = to.y + to.h / 2;
      if (from === to || ax === bx && ay === by) return [{ edge, path: `M${ax},${from.y}C${ax - 40},${from.y - 28} ${ax + 40},${from.y - 28} ${ax + Math.min(20, from.w / 3)},${from.y}`, x: ax, y: from.y - 18 }];
      const dx = bx - ax, dy = by - ay;
      const start = Math.min(dx === 0 ? Infinity : from.w / 2 / Math.abs(dx), dy === 0 ? Infinity : from.h / 2 / Math.abs(dy));
      const end = Math.min(dx === 0 ? Infinity : to.w / 2 / Math.abs(dx), dy === 0 ? Infinity : to.h / 2 / Math.abs(dy));
      return [{ edge, path: `M${ax + dx * start},${ay + dy * start}L${bx - dx * end},${by - dy * end}`, x: (ax + bx) / 2, y: (ay + by) / 2 - 6 }];
    });
  });
  const color = (index: number) => context.color(`${props.nodeKey}:node:${props.node.nodes[index].id}`, props.node.nodes[index].color, "accent");
  return <div class="nk-graphic-diagram" ref={canvas.ref}>
    <div class="nk-graphic-widget-tools"><span class="nk-graphic-status">选择节点查看详情 · 方向键切换</span><ColorSettings>
      <For each={props.node.nodes}>{item => <ColorControl colorKey={`${props.nodeKey}:node:${item.id}`} label={item.label} declared={item.color} fallback="accent" />}</For>
    </ColorSettings></div>
    <svg class="nk-graphic-canvas" width="100%" height={height()} viewBox={`0 0 ${canvas.width()} ${height()}`} role="group" aria-label={props.node.title ?? "交互关系图"}>
      <title>{props.node.title ?? "关系图"}</title>
      <defs><marker id={marker} markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto" markerUnits="strokeWidth"><path d="M0,0L8,4L0,8Z" fill="var(--nk-text-muted)" /></marker></defs>
      <For each={edges()}>{edge => <g><path d={edge.path} fill="none" stroke="var(--nk-text-muted)" stroke-width="1.5" marker-end={`url(#${marker})`} /><Show when={edge.edge.label}><text x={edge.x} y={edge.y} text-anchor="middle" class="nk-graphic-edge-label">{edge.edge.label}</text></Show></g>}</For>
      <For each={geometry()}>{(item, index) => <g ref={element => { focusNodes.set(index(), element); }} data-graphic-node="" role="button" tabindex="0" aria-label={`${item.node.label}${item.node.description ? `：${item.node.description}` : ""}`} aria-pressed={selected() === index()} class="nk-graphic-diagram-node" onClick={() => select(selected() === index() ? -1 : index())} onKeyDown={event => {
        if (event.key === "Enter" || event.key === " ") { event.preventDefault(); select(selected() === index() ? -1 : index()); }
        else if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) {
          event.preventDefault();
          const count = props.node.nodes.length;
          const next = event.key === "Home" ? 0 : event.key === "End" ? count - 1 : (index() + (["ArrowLeft", "ArrowUp"].includes(event.key) ? -1 : 1) + count) % count;
          focusNodes.get(next)?.focus();
          select(next);
        } else if (event.key === "Escape") select(-1);
      }}>
        <title>{item.node.label}</title>
        <rect x={item.x} y={item.y} width={item.w} height={item.h} rx="8" fill="var(--nk-surface)" />
        <rect x={item.x} y={item.y} width={item.w} height={item.h} rx="8" fill={color(index())} fill-opacity={selected() === index() ? 0.28 : 0.12} stroke={color(index())} stroke-width={selected() === index() ? 3 : 1.5} />
        <text x={item.x + item.w / 2} y={item.y + item.h / 2} text-anchor="middle" dominant-baseline="middle" fill="var(--nk-text)" font-size={String(Math.max(8, Math.min(14, item.h * 0.3, item.w / Math.max(3, Math.min(18, item.node.label.length)))))}>{item.node.label.length > 22 ? `${item.node.label.slice(0, 21)}…` : item.node.label}</text>
      </g>}</For>
    </svg>
    <Show when={props.node.nodes[selected()]}>{item => <aside class="nk-graphic-diagram-detail" aria-live="polite"><strong>{item().label}</strong><Show when={item().description}><p>{item().description}</p></Show></aside>}</Show>
  </div>;
}
