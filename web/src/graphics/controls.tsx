// SPDX-License-Identifier: Apache-2.0 OR MIT
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { Color, SemanticColor } from "./types";
import { cssColor, useGraphics } from "./context";
import { colorInputValue } from "./theme";
import { registerGraphicsReset } from "./state";

const palette: { color: SemanticColor; label: string }[] = [
  { color: "accent", label: "主题强调" }, { color: "red", label: "红色" },
  { color: "orange", label: "橙色" }, { color: "yellow", label: "黄色" },
  { color: "green", label: "绿色" }, { color: "teal", label: "青色" },
  { color: "blue", label: "蓝色" }, { color: "purple", label: "紫色" },
  { color: "pink", label: "粉色" }, { color: "text", label: "正文色" },
  { color: "muted", label: "次要文字色" }, { color: "surface", label: "表面色" },
  { color: "border", label: "边框色" }
];

export function frameAction<T>(apply: (value: T) => void): (value: T) => void {
  let frame = 0;
  let latest: T;
  const cancel = () => { if (frame) cancelAnimationFrame(frame); frame = 0; };
  const unregister = registerGraphicsReset(cancel);
  onCleanup(() => { cancel(); unregister(); });
  return value => {
    latest = value;
    if (!frame) frame = requestAnimationFrame(() => { frame = 0; apply(latest); });
  };
}
export function ColorSettings(props: { children: JSX.Element }) {
  const [open, setOpen] = createSignal(false);
  return <details class="nk-graphic-colors" name="nk-graphic-palette" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary aria-label="调整配色" title="调整配色"><svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.4"><path d="M10 2.5a7.5 7.5 0 1 0 0 15h1a2 2 0 0 0 1.4-3.4.8.8 0 0 1 .6-1.4h1.3a3.2 3.2 0 0 0 3.2-3.2A7.1 7.1 0 0 0 10 2.5Z"/><circle cx="6" cy="7" r=".8" fill="currentColor" stroke="none"/><circle cx="10" cy="5.5" r=".8" fill="currentColor" stroke="none"/><circle cx="14" cy="7" r=".8" fill="currentColor" stroke="none"/><circle cx="5" cy="11" r=".8" fill="currentColor" stroke="none"/></svg></summary><Show when={open()}><div class="nk-graphic-color-list">{props.children}</div></Show>
  </details>;
}
export function ColorControl(props: { colorKey: string; label: string; declared?: Color; fallback?: Color }) {
  const context = useGraphics();
  let host!: HTMLDivElement;
  const current = createMemo(() => context.color(props.colorKey, props.declared, props.fallback));
  const [value, setValue] = createSignal("#000000");
  const [draft, setDraft] = createSignal("#000000");
  const [swatchValues, setSwatchValues] = createSignal<string[]>([]);
  createEffect(() => {
    context.themeRevision();
    if (host) setSwatchValues(palette.map(swatch => colorInputValue(host, cssColor(swatch.color))));
  });
  createEffect(() => {
    context.themeRevision();
    const color = current();
    if (host) { const hex = colorInputValue(host, color); setValue(hex); setDraft(hex); }
  });
  return <div ref={host} class="nk-graphic-color-control">
    <div class="nk-graphic-color-label"><span>{props.label}</span><button type="button" onClick={() => context.themeColor(props.colorKey)} title="忽略显式配色，使用当前主题">跟随主题</button></div>
    <div class="nk-graphic-color-swatches" role="group" aria-label={`${props.label}预设颜色`}><For each={palette}>{(swatch, index) => <button type="button" class="nk-graphic-color-swatch" style={{ "background-color": cssColor(swatch.color) }} aria-label={`${props.label}：${swatch.label}`} aria-pressed={value() === swatchValues()[index()]} title={`${swatch.label} ${swatchValues()[index()] ?? ""}`} onClick={() => { const color = swatchValues()[index()]; if (color) context.setColor(props.colorKey, color); }} />}</For></div>
    <label class="nk-graphic-color-hex"><span>HEX</span><input type="text" value={draft()} aria-label={`${props.label}颜色`} placeholder="#RRGGBB" maxlength={7} pattern="#[0-9a-fA-F]{6}" spellcheck={false} autocapitalize="off" autocomplete="off" onInput={event => {
      const next = event.currentTarget.value;
      setDraft(next);
      if (/^#[0-9a-f]{6}$/i.test(next)) context.setColor(props.colorKey, next.toLowerCase());
    }} onBlur={() => setDraft(value())} /><span class="nk-graphic-color-preview" style={{ "background-color": value() }} aria-hidden="true" /></label>
  </div>;
}
export function useCanvasWidth() {
  const [width, setWidth] = createSignal(640);
  let host: HTMLElement | undefined;
  let observer: ResizeObserver | undefined;
  const update = frameAction((value: number) => setWidth(Math.max(260, Math.min(1600, Math.round(value)))));
  onMount(() => {
    if (!host) return;
    update(host.clientWidth);
    observer = new ResizeObserver(entries => { if (entries[0]) update(entries[0].contentRect.width); });
    observer.observe(host);
  });
  onCleanup(() => observer?.disconnect());
  return { width, ref: (element: HTMLElement) => { host = element; } };
}
export function formatNumber(value: number): string {
  const magnitude = Math.abs(value);
  if (!Number.isFinite(value)) return "超出数值显示范围";
  if (magnitude !== 0 && (magnitude >= 100000 || magnitude < 0.001)) return value.toExponential(1);
  return Number(value.toPrecision(4)).toLocaleString(undefined, { maximumFractionDigits: 6 });
}
export function ratio(value: number, min: number, max: number): number {
  if (min === max) return 0.5;
  const magnitude = Math.max(Math.abs(min), Math.abs(max), Math.abs(value), Number.MIN_VALUE);
  return (value / magnitude - min / magnitude) / (max / magnitude - min / magnitude);
}
export function interpolate(min: number, max: number, fraction: number): number { return min * (1 - fraction) + max * fraction; }
export function linePath(points: [number, number | null][], x: (value: number) => number, y: (value: number) => number): string {
  let path = "", connected = false;
  for (const [px, py] of points) {
    if (py === null || !Number.isFinite(py)) { connected = false; continue; }
    const dx = x(px), dy = y(py);
    if (!Number.isFinite(dx) || !Number.isFinite(dy)) { connected = false; continue; }
    path += `${connected ? "L" : "M"}${dx.toFixed(2)},${dy.toFixed(2)}`;
    connected = true;
  }
  return path;
}
