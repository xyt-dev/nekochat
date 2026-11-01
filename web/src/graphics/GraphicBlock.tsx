// SPDX-License-Identifier: Apache-2.0 OR MIT
import { createEffect, createMemo, createSignal, For, Match, onCleanup, onMount, Show, Switch, type Accessor, type Setter } from "solid-js";
import type { GraphicNode, GraphicSpec, LayoutNode, TextNode, StatNode, ChoicesNode, ChartNode, PlotNode, DiagramNode, SvgNode, Color } from "./types";
import { cssColor, GraphicsContext, useGraphics, type GraphicsContextValue } from "./context";
import { readGraphicState, writeGraphicState, runtimeEpoch, stateKey, type GraphicState } from "./state";
import { subscribeTheme } from "./theme";
import { ColorControl, ColorSettings } from "./controls";
import { copyText } from "../clipboard";
import { Markdown } from "../components/Markdown";
import Chart from "./Chart";
import Plot from "./Plot";
import Diagram from "./Diagram";
import SvgImage from "./SvgImage";
import Choices from "./Choices";
import "./graphics.css";

interface Entry { key: string; node: Accessor<GraphicNode>; setNode: Setter<GraphicNode> }
function NodeList(props: { items: GraphicNode[]; path: string; columns?: number }) {
  const entries = createMemo<Entry[]>((previous) => {
    const existing = new Map(previous.map(entry => [entry.key, entry]));
    return props.items.map((node, index) => {
      const key = node.id ? `node:${encodeURIComponent(node.id)}` : `${props.path}/${index}`;
      const entry = existing.get(key);
      if (entry) { entry.setNode(node); return entry; }
      const [current, setNode] = createSignal(node);
      return { key, node: current, setNode };
    });
  }, []);
  return <For each={entries()}>{entry => <NodeView node={entry.node()} nodeKey={entry.key} columns={props.columns ?? 1} />}</For>;
}
function Text(props: { node: TextNode; nodeKey: string }) {
  const context = useGraphics();
  return <><Show when={props.node.palette}><div class="nk-graphic-widget-tools"><ColorSettings><ColorControl colorKey={`${props.nodeKey}:color`} label={props.node.title ?? "文字颜色"} declared={props.node.color} fallback={props.node.size === "muted" ? "muted" : "text"} /></ColorSettings></div></Show><div class={`nk-graphic-text nk-graphic-text-${props.node.size ?? "body"}`} role={props.node.size === "h2" || props.node.size === "h3" ? "heading" : undefined} aria-level={props.node.size === "h2" ? 2 : props.node.size === "h3" ? 3 : undefined} style={{ color: context.color(`${props.nodeKey}:color`, props.node.color, props.node.size === "muted" ? "muted" : "text") }}>
    <Markdown id={`${context.id()}:text:${props.nodeKey}`} text={() => props.node.content} final={() => true} graphics={false} />
  </div></>;
}
function Stat(props: { node: StatNode; nodeKey: string }) {
  const context = useGraphics();
  return <div class="nk-graphic-stat" classList={{ "nk-graphic-stat-palette": !!props.node.palette }}>
    <span class="nk-graphic-muted">{props.node.label}</span>
    <strong style={{ color: context.color(`${props.nodeKey}:color`, props.node.color, "accent") }}>{props.node.value}</strong>
    <Show when={props.node.delta}><span class="nk-graphic-stat-delta">{props.node.delta}</span></Show>
    <Show when={props.node.palette}><ColorSettings><ColorControl colorKey={`${props.nodeKey}:color`} label={props.node.label} declared={props.node.color} fallback="accent" /></ColorSettings></Show>
  </div>;
}
function Layout(props: { node: LayoutNode; nodeKey: string }) {
  const context = useGraphics();
  const filled = createMemo(() => props.node.type !== "card" && (props.node.color !== undefined || context.state().colors[stateKey(`${props.nodeKey}:color`)] !== undefined));
  return <><Show when={props.node.palette}><div class="nk-graphic-widget-tools"><ColorSettings><ColorControl colorKey={`${props.nodeKey}:color`} label={props.node.title ?? "背景颜色"} declared={props.node.color} fallback="surface" /></ColorSettings></div></Show><div class={`nk-graphic-layout nk-graphic-layout-${props.node.type}`} classList={{ "nk-graphic-layout-filled": filled() }} style={{ gap: `${props.node.gap ?? 12}px`, "--nk-graphic-cols": props.node.cols ?? 2, background: filled() ? context.color(`${props.nodeKey}:color`, props.node.color, "surface") : undefined }}>
    <NodeList items={props.node.items} path={props.nodeKey} columns={props.node.type === "grid" ? props.node.cols ?? 2 : 1} />
  </div></>;
}
function NodeView(props: { node: GraphicNode; nodeKey: string; columns: number }) {
  const context = useGraphics();
  return <section class={`nk-graphic-node nk-graphic-node-${props.node.type}`} style={{ "--nk-graphic-span": props.node.span ?? 1, "grid-column": `span ${Math.min(props.columns, props.node.span ?? 1)}`, background: props.node.type === "card" ? context.color(`${props.nodeKey}:color`, (props.node as LayoutNode).color, "surface") : undefined }}>
    <Show when={props.node.title}><h3 class="nk-graphic-node-title">{props.node.title}</h3></Show>
    <Switch>
      <Match when={props.node.type === "text"}><Text node={props.node as TextNode} nodeKey={props.nodeKey} /></Match>
      <Match when={props.node.type === "stat"}><Stat node={props.node as StatNode} nodeKey={props.nodeKey} /></Match>
      <Match when={props.node.type === "choices"}><Choices node={props.node as ChoicesNode} /></Match>
      <Match when={["row", "col", "grid", "card"].includes(props.node.type)}><Layout node={props.node as LayoutNode} nodeKey={props.nodeKey} /></Match>
      <Match when={props.node.type === "chart"}><Chart node={props.node as ChartNode} nodeKey={props.nodeKey} /></Match>
      <Match when={props.node.type === "plot"}><Plot node={props.node as PlotNode} nodeKey={props.nodeKey} /></Match>
      <Match when={props.node.type === "diagram"}><Diagram node={props.node as DiagramNode} nodeKey={props.nodeKey} /></Match>
      <Match when={props.node.type === "svg"}><SvgImage node={props.node as SvgNode} nodeKey={props.nodeKey} /></Match>
    </Switch>
  </section>;
}

/** A validated graphic; id must be stable across streaming and virtual-scroll remounts. */
export default function GraphicBlock(props: { id: string; spec: GraphicSpec; source: string; onQuestion?: (prompt: string) => void; questionsDisabled?: boolean }) {
  const [state, setState] = createSignal<GraphicState>(readGraphicState(props.id));
  const [themeRevision, setThemeRevision] = createSignal(0);
  const [sourceOpen, setSourceOpen] = createSignal(false);
  const [copyStatus, setCopyStatus] = createSignal("");
  const choicesOnly = createMemo(() => props.spec.items.length > 0 && props.spec.items.every(node => node.type === "choices"));
  let active = true;
  let copyRevision = 0;
  let unsubscribe: (() => void) | undefined;
  createEffect(() => {
    runtimeEpoch();
    setState(readGraphicState(props.id));
    setSourceOpen(false);
    setCopyStatus("");
    copyRevision++;
  });
  onMount(() => { unsubscribe = subscribeTheme(() => setThemeRevision(revision => revision + 1)); });
  onCleanup(() => { active = false; copyRevision++; unsubscribe?.(); });
  function update(field: keyof GraphicState, key: string, value: string | number | undefined): void {
    const current = state();
    const part = { ...current[field] };
    delete part[stateKey(key)];
    if (value !== undefined) part[stateKey(key)] = value as never;
    const next = { ...current, [field]: part };
    setState(writeGraphicState(props.id, next));
  }
  const context: GraphicsContextValue = {
    id: () => props.id, state, themeRevision,
    canQuestion: () => props.onQuestion !== undefined && !props.questionsDisabled,
    chooseQuestion(prompt) { if (context.canQuestion()) props.onQuestion?.(prompt); },
    color(key: string, declared?: Color, fallback: Color = "accent"): string {
      return state().colors[stateKey(key)] ?? cssColor(context.followsTheme(key) ? fallback : declared ?? fallback);
    },
    followsTheme: key => state().views[stateKey(`theme:${key}`)] === 1,
    setColor(key, color) { if (/^#[0-9a-f]{6}$/i.test(color)) update("colors", key, color); },
    themeColor(key) { update("colors", key, undefined); update("views", `theme:${key}`, 1); },
    param: (key, fallback) => state().params[stateKey(key)] ?? fallback,
    setParam(key, value) { if (Number.isFinite(value)) update("params", key, value); },
    view: (key, fallback) => state().views[stateKey(key)] ?? fallback,
    setView(key, value) { if (Number.isSafeInteger(value) && value >= 0) update("views", key, value); }
  };
  async function copySource(): Promise<void> {
    const revision = ++copyRevision;
    try { await copyText(props.source); if (active && revision === copyRevision) setCopyStatus("已复制"); }
    catch { if (active && revision === copyRevision) setCopyStatus("复制失败，请在源规范中手动选择复制。"); }
  }
  return <GraphicsContext.Provider value={context}><div class="nk-graphic-block" classList={{ "nk-graphic-questions-only": choicesOnly() }} role="group" aria-label={props.spec.title ?? (choicesOnly() ? "建议问题" : "交互图形")}>
    <Show when={choicesOnly() && props.spec.title}><h3 class="nk-graphic-questions-title">{props.spec.title}</h3></Show>
    <Show when={!choicesOnly() && props.spec.title}><div class="nk-graphic-heading"><h2>{props.spec.title}</h2></div></Show>
    <div class="nk-graphic-items" style={{ gap: `${props.spec.gap ?? 12}px` }}><NodeList items={props.spec.items} path="root" /></div>
    <Show when={!choicesOnly()}><details class="nk-graphic-source-details" open={sourceOpen()} onToggle={event => setSourceOpen(event.currentTarget.open)}><summary>源规范</summary><Show when={sourceOpen()}><div class="nk-graphic-source-frame"><div class="nk-graphic-source-actions"><Show when={copyStatus()}><span class="nk-graphic-status" role="status">{copyStatus()}</span></Show><button type="button" onClick={() => { void copySource(); }}>复制</button></div><pre class="nk-graphic-source" tabindex="0" role="region" aria-label="图形源规范"><code>{props.source}</code></pre></div></Show></details></Show>
  </div></GraphicsContext.Provider>;
}
