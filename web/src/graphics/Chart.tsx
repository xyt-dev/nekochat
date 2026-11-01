// SPDX-License-Identifier: Apache-2.0 OR MIT
import { createMemo, createSignal, For, Show, createUniqueId } from "solid-js";
import type { ChartKind, ChartNode } from "./types";
import { SERIES_COLORS, useGraphics } from "./context";
import { ColorControl, ColorSettings, formatNumber, interpolate, linePath, ratio, useCanvasWidth } from "./controls";

const KINDS: ChartKind[] = ["line", "bar", "area", "donut"];
const KIND_NAMES = ["折线", "柱状", "面积", "环形"];

export default function Chart(props: { node: ChartNode; nodeKey: string }) {
  const context = useGraphics();
  const canvas = useCanvasWidth();
  const clip = `nk-chart-${createUniqueId()}`;
  const [dataOpen, setDataOpen] = createSignal(false);
  const canDonut = createMemo(() => props.node.series.length === 1 && props.node.series[0].values.every(value => value >= 0) && props.node.series[0].values.some(value => value > 0));
  const kind = createMemo(() => {
    const selected = KINDS[context.view(`${props.nodeKey}:kind`, KINDS.indexOf(props.node.kind))] ?? props.node.kind;
    return selected === "donut" && !canDonut() ? props.node.kind : selected;
  });
  const height = () => props.node.height ?? 280;
  const right = () => canvas.width() - 18;
  const bottom = () => height() - 38;
  const innerWidth = () => right() - 62;
  const domain = createMemo(() => {
    let min = 0, max = 0;
    for (const series of props.node.series) for (const value of series.values) { min = Math.min(min, value); max = Math.max(max, value); }
    if (min === max) { min = -1; max = 1; }
    return { min, max };
  });
  const x = (index: number) => kind() === "bar"
    ? 62 + innerWidth() * (index + 0.5) / Math.max(1, props.node.labels.length)
    : 62 + innerWidth() * (props.node.labels.length > 1 ? index / (props.node.labels.length - 1) : 0.5);
  const y = (value: number) => bottom() - ratio(value, domain().min, domain().max) * (bottom() - 18);
  const paths = createMemo(() => props.node.series.map(series => linePath(series.values.map((value, index) => [index, value]), x, y)));
  const colorKey = (index: number) => `${props.nodeKey}:${kind() === "donut" ? "segment" : "series"}:${index}`;
  const seriesColor = (index: number) => context.color(`${props.nodeKey}:series:${index}`, props.node.series[index]?.color, SERIES_COLORS[index % SERIES_COLORS.length]);
  const segmentColor = (index: number) => context.color(`${props.nodeKey}:segment:${index % 8}`, index % 8 === 0 ? props.node.series[0]?.color : undefined, SERIES_COLORS[index % 8]);
  const ticks = createMemo(() => {
    const count = Math.max(2, Math.floor(innerWidth() / 85));
    const last = props.node.labels.length - 1;
    if (last < 0) return [];
    return [...new Set(Array.from({ length: Math.min(count, last + 1) }, (_, index) => Math.round(index * last / Math.max(1, Math.min(count, last + 1) - 1))))];
  });
  const segments = createMemo(() => {
    if (kind() !== "donut") return [];
    const values = props.node.series[0].values;
    const max = Math.max(...values);
    const total = values.reduce((sum, value) => sum + value / max, 0);
    let angle = -Math.PI / 2;
    return values.map((value, index) => {
      const fraction = value / max / total;
      const start = angle;
      angle += fraction * Math.PI * 2;
      const end = angle;
      const cx = canvas.width() / 2, cy = height() / 2;
      const outer = Math.min(canvas.width() / 2 - 16, height() / 2 - 16), inner = outer * 0.62;
      const point = (radius: number, theta: number) => `${cx + radius * Math.cos(theta)},${cy + radius * Math.sin(theta)}`;
      // Two arcs also cover a single segment that fills the complete ring.
      const middle = (start + end) / 2;
      const path = `M${point(outer, start)}A${outer},${outer} 0 0 1 ${point(outer, middle)}A${outer},${outer} 0 0 1 ${point(outer, end)}L${point(inner, end)}A${inner},${inner} 0 0 0 ${point(inner, middle)}A${inner},${inner} 0 0 0 ${point(inner, start)}Z`;
      return { index, value, fraction, path };
    });
  });
  return <div class="nk-graphic-chart" ref={canvas.ref}>
    <div class="nk-graphic-widget-tools">
      <label>图表视图 <select aria-label="图表视图" value={kind()} onChange={event => context.setView(`${props.nodeKey}:kind`, KINDS.indexOf(event.currentTarget.value as ChartKind))}>
        <For each={KINDS}>{(item, index) => <option value={item} disabled={item === "donut" && !canDonut()}>{KIND_NAMES[index()]}</option>}</For>
      </select></label>
      <ColorSettings><For each={Array.from({ length: kind() === "donut" ? Math.min(8, props.node.labels.length) : props.node.series.length }, (_, index) => index)}>{index =>
        <ColorControl colorKey={colorKey(index)} label={kind() === "donut" ? `色组 ${index + 1}` : props.node.series[index].label} declared={kind() === "donut" ? index === 0 ? props.node.series[0]?.color : undefined : props.node.series[index].color} fallback={SERIES_COLORS[index % 8]} />
      }</For></ColorSettings>
    </div>
    <svg class="nk-graphic-canvas" width="100%" height={height()} viewBox={`0 0 ${canvas.width()} ${height()}`} role="img" aria-label={props.node.title ?? `${KIND_NAMES[KINDS.indexOf(kind())]}图；下方可查看数据`}>
      <title>{props.node.title ?? "图表"}</title>
      <Show when={kind() !== "donut"} fallback={<>
        <For each={segments()}>{segment => <Show when={segment.value > 0}><path d={segment.path} fill={segmentColor(segment.index)} stroke="var(--nk-surface)" stroke-width="1"><title>{props.node.labels[segment.index]}: {formatNumber(segment.value)} ({(segment.fraction * 100).toFixed(1)}%)</title></path></Show>}</For>
        <text x={canvas.width() / 2} y={height() / 2} text-anchor="middle" dominant-baseline="middle" class="nk-graphic-total">{formatNumber(props.node.series[0].values.reduce((sum, value) => sum + value, 0))}</text>
      </>}>
        <defs><clipPath id={clip}><rect x="62" y="17" width={innerWidth() + 1} height={bottom() - 16} /></clipPath></defs>
        <For each={[0, 0.25, 0.5, 0.75, 1]}>{fraction => <>
          <line x1="62" x2={right()} y1={y(interpolate(domain().min, domain().max, fraction))} y2={y(interpolate(domain().min, domain().max, fraction))} class="nk-graphic-gridline" />
          <text x="54" y={y(interpolate(domain().min, domain().max, fraction)) + 4} text-anchor="end" class="nk-graphic-axis-label">{formatNumber(interpolate(domain().min, domain().max, fraction))}</text>
        </>}</For>
        <line x1="62" x2={right()} y1={y(0)} y2={y(0)} class="nk-graphic-axis" />
        <For each={ticks()}>{index => <text x={x(index)} y={height() - 14} text-anchor="middle" class="nk-graphic-axis-label"><title>{props.node.labels[index]}</title>{props.node.labels[index].length > 12 ? `${props.node.labels[index].slice(0, 11)}…` : props.node.labels[index]}</text>}</For>
        <g clip-path={`url(#${clip})`}>
          <For each={props.node.series}>{(series, seriesIndex) => <Show when={kind() === "bar"} fallback={<>
            <Show when={kind() === "area" && series.values.length > 0}><path d={`${paths()[seriesIndex()]}L${x(series.values.length - 1)},${y(0)}L${x(0)},${y(0)}Z`} fill={seriesColor(seriesIndex())} fill-opacity="0.2" /></Show>
            <path d={paths()[seriesIndex()]} fill="none" stroke={seriesColor(seriesIndex())} stroke-width="2" stroke-linejoin="round" vector-effect="non-scaling-stroke" />
            <Show when={series.values.length === 1}><circle cx={x(0)} cy={y(series.values[0])} r="3" fill={seriesColor(seriesIndex())} /></Show>
          </>}>
            <For each={series.values}>{(value, index) => {
              const barWidth = () => innerWidth() / Math.max(1, props.node.labels.length) * 0.8 / props.node.series.length;
              return <rect x={x(index()) - barWidth() * props.node.series.length / 2 + seriesIndex() * barWidth()} y={Math.min(y(0), y(value))} width={Math.max(0.2, barWidth() * 0.9)} height={Math.abs(y(value) - y(0))} fill={seriesColor(seriesIndex())}><title>{series.label}: {props.node.labels[index()]} = {formatNumber(value)}</title></rect>;
            }}</For>
          </Show>}</For>
        </g>
      </Show>
    </svg>
    <ul class="nk-graphic-legend" aria-label="图例"><Show when={kind() === "donut"} fallback={<For each={props.node.series}>{(series, index) => <li><span style={{ background: seriesColor(index()) }} />{series.label}</li>}</For>}>
      <For each={props.node.labels}>{(label, index) => <li><span style={{ background: segmentColor(index()) }} />{label} · {formatNumber(props.node.series[0].values[index()])}</li>}</For>
    </Show></ul>
    <details class="nk-graphic-data" onToggle={event => setDataOpen(event.currentTarget.open)}><summary>查看数据</summary><Show when={dataOpen()}><div class="nk-graphic-table-scroll" tabindex="0" role="region" aria-label="图表数据"><table><thead><tr><th scope="col">类别</th><For each={props.node.series}>{series => <th scope="col">{series.label}</th>}</For></tr></thead><tbody><For each={props.node.labels}>{(label, index) => <tr><th scope="row">{label}</th><For each={props.node.series}>{series => <td>{String(series.values[index()])}</td>}</For></tr>}</For></tbody></table></div></Show></details>
  </div>;
}
