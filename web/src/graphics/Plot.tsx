// SPDX-License-Identifier: Apache-2.0 OR MIT
import { batch, createEffect, createMemo, createSignal, createUniqueId, For, onCleanup, Show } from "solid-js";
import type { PlotNode, PlotSample } from "./types";
import { SERIES_COLORS, useGraphics } from "./context";
import { ColorControl, ColorSettings, formatNumber, frameAction, interpolate, linePath, ratio, useCanvasWidth } from "./controls";
import { createPlotSampler } from "./compute";
import { runtimeEpoch } from "./state";

export default function Plot(props: { node: PlotNode; nodeKey: string }) {
  const context = useGraphics();
  const canvas = useCanvasWidth();
  const clip = `nk-plot-${createUniqueId()}`;
  const [sample, setSample] = createSignal<PlotSample>();
  const [pending, setPending] = createSignal(true);
  const [error, setError] = createSignal("");
  const values = createMemo(() => {
    const result: Record<string, number> = Object.create(null);
    for (const param of props.node.params ?? []) result[param.name] = Math.max(param.min, Math.min(param.max, context.param(`${props.nodeKey}:param:${param.name}`, param.value)));
    return JSON.stringify(result);
  });
  createEffect(() => {
    runtimeEpoch();
    setSample(undefined);
    const sampler = createPlotSampler(result => { setSample(result); setPending(false); setError(""); }, message => { setError(message); setPending(false); });
    createEffect(() => {
      const node = props.node;
      const params = JSON.parse(values()) as Record<string, number>;
      setError("");
      setPending(true);
      sampler.submit(node, params);
    });
    onCleanup(() => sampler.dispose());
  });
  const height = () => props.node.height ?? 280;
  const right = () => canvas.width() - 18;
  const bottom = () => height() - 36;
  const x = (value: number) => 62 + ratio(value, sample()!.xMin, sample()!.xMax) * (right() - 62);
  const y = (value: number) => bottom() - ratio(value, sample()!.yMin, sample()!.yMax) * (bottom() - 18);
  const paths = createMemo(() => sample()?.series.map(series => linePath(series.points, x, y)) ?? []);
  const color = (index: number) => context.color(`${props.nodeKey}:series:${index}`, props.node.series[index]?.color, SERIES_COLORS[index % 8]);
  return <div class="nk-graphic-plot" ref={canvas.ref}>
    <div class="nk-graphic-widget-tools">
      <span class="nk-graphic-status" role="status">{pending() ? "正在计算…" : error() ? "计算失败" : "本地函数图"}</span>
      <ColorSettings><For each={props.node.series}>{(series, index) => <ColorControl colorKey={`${props.nodeKey}:series:${index()}`} label={series.label ?? `函数 ${index() + 1}`} declared={series.color} fallback={SERIES_COLORS[index() % 8]} />}</For></ColorSettings>
    </div>
    <Show when={sample()} fallback={<div class="nk-graphic-compute-placeholder" style={{ height: `${height()}px` }} role="status">{error() || "正在准备函数图…"}</div>}>
      {result => <svg class="nk-graphic-canvas" classList={{ "nk-graphic-updating": pending() }} width="100%" height={height()} viewBox={`0 0 ${canvas.width()} ${height()}`} role="img" aria-label={props.node.title ?? "交互函数图；下方可调整参数"} aria-busy={pending()}>
        <title>{props.node.title ?? "函数图"}</title>
        <defs><clipPath id={clip}><rect x="62" y="17" width={right() - 61} height={bottom() - 16} /></clipPath></defs>
        <For each={[0, 0.25, 0.5, 0.75, 1]}>{fraction => <>
          <line x1="62" x2={right()} y1={y(interpolate(result().yMin, result().yMax, fraction))} y2={y(interpolate(result().yMin, result().yMax, fraction))} class="nk-graphic-gridline" />
          <text x="54" y={y(interpolate(result().yMin, result().yMax, fraction)) + 4} text-anchor="end" class="nk-graphic-axis-label">{formatNumber(interpolate(result().yMin, result().yMax, fraction))}</text>
          <text x={x(interpolate(result().xMin, result().xMax, fraction))} y={height() - 12} text-anchor={fraction === 0 ? "start" : fraction === 1 ? "end" : "middle"} class="nk-graphic-axis-label">{formatNumber(interpolate(result().xMin, result().xMax, fraction))}</text>
        </>}</For>
        <Show when={result().yMin <= 0 && result().yMax >= 0}><line x1="62" x2={right()} y1={y(0)} y2={y(0)} class="nk-graphic-axis" /></Show>
        <Show when={result().xMin <= 0 && result().xMax >= 0}><line x1={x(0)} x2={x(0)} y1="18" y2={bottom()} class="nk-graphic-axis" /></Show>
        <g clip-path={`url(#${clip})`}><For each={paths()}>{(path, index) => <path d={path} fill="none" stroke={color(index())} stroke-width="2" stroke-linejoin="round" vector-effect="non-scaling-stroke" />}</For></g>
      </svg>}
    </Show>
    <Show when={error() && sample()}><p class="nk-graphic-error" role="alert">{error()}</p></Show>
    <ul class="nk-graphic-legend" aria-label="函数图例"><For each={props.node.series}>{(series, index) => <li><span style={{ background: color(index()) }} />{series.label ?? `函数 ${index() + 1}`}</li>}</For></ul>
    <Show when={props.node.params?.length}>
      <div class="nk-graphic-params"><For each={props.node.params}>{param => {
        const value = () => Math.max(param.min, Math.min(param.max, context.param(`${props.nodeKey}:param:${param.name}`, param.value)));
        const changeParam = frameAction((next: number) => context.setParam(`${props.nodeKey}:param:${param.name}`, next));
        return <label class="nk-graphic-param"><span>{param.label ?? param.name}<output>{formatNumber(value())}</output></span>
          <input type="range" min={param.min} max={param.max} step={param.step ?? Math.max(Number.MIN_VALUE, param.max / 100 - param.min / 100)} value={value()} aria-label={param.label ?? param.name} aria-valuetext={String(value())} onInput={event => changeParam(event.currentTarget.valueAsNumber)} />
          <span class="nk-graphic-range"><span>{formatNumber(param.min)}</span><span>{formatNumber(param.max)}</span></span>
        </label>;
      }}</For></div>
      <button class="nk-graphic-small-button" type="button" onClick={() => batch(() => { for (const param of props.node.params ?? []) context.setParam(`${props.nodeKey}:param:${param.name}`, param.value); })}>重置参数</button>
    </Show>
  </div>;
}
