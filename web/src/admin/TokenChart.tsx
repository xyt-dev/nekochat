import { createEffect, createMemo, createSignal, createUniqueId, For, onCleanup, onMount, Show } from "solid-js";
import type { ModelStats, Stats, Totals } from "./api";
export type TokenMetric = "total" | "cached_in" | "uncached_in" | "reasoning" | "output";
export const metricLabels: Record<TokenMetric, string> = { total: "总 token", cached_in: "缓存输入", uncached_in: "无缓存输入", reasoning: "思考", output: "输出（不含思考）" };
export const tokens = (value: Pick<Totals, "cached_in" | "uncached_in" | "reasoning" | "output">, metric: TokenMetric) => metric === "total" ? value.cached_in + value.uncached_in + value.reasoning + value.output : value[metric];
export function modelColor(key: string) {
  let hash = 2166136261;
  for (let index = 0; index < key.length; index++) hash = Math.imul(hash ^ key.charCodeAt(index), 16777619);
  return `hsl(${(hash >>> 0) % 360} 67% 52%)`;
}

export interface ChartSeries {
  key: string; label: string; detail?: string;
  points: { at_ms: number; value: number }[];
}

/** Shared renderer: model ranges are exclusive; search ledger ranges include the end timestamp. */
export function UsageLineChart(props: {
  from_ms: number; to_ms: number; bucket_ms: number; inclusiveEnd?: boolean;
  series: ChartSeries[]; metricLabel: string; label: string;
}) {
  let canvas!: HTMLCanvasElement;
  let width = 800;
  const height = 300;
  const padding = { left: 68, right: 20, top: 25, bottom: 58 };
  const helpId = createUniqueId();
  const [hover, setHover] = createSignal<number | null>(null);
  const [revision, setRevision] = createSignal(0);
  const buckets = createMemo(() => {
    const start = Math.floor(props.from_ms / props.bucket_ms) * props.bucket_ms;
    const span = (props.to_ms - start) / props.bucket_ms;
    return { start, count: Math.max(1, props.inclusiveEnd ? Math.floor(span) + 1 : Math.ceil(span)) };
  });
  const series = createMemo(() => props.series.map((line) => {
    const values = new Array<number>(buckets().count).fill(0);
    for (const point of line.points) {
      const index = Math.floor((point.at_ms - buckets().start) / props.bucket_ms);
      if (index >= 0 && index < values.length) values[index] = point.value;
    }
    return { ...line, values };
  }));
  const bucketStart = (index: number) => buckets().start + index * props.bucket_ms;
  const time = (index: number) => Math.max(props.from_ms, bucketStart(index));
  const shortTime = (at: number) => new Date(at).toLocaleString(undefined, { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
  const plotWidth = () => Math.max(1, width - padding.left - padding.right);
  const x = (index: number) => padding.left + (buckets().count === 1 ? 0.5 : (time(index) - props.from_ms) / Math.max(1, props.to_ms - props.from_ms)) * plotWidth();
  function draw() {
    const context = canvas?.getContext("2d"); if (!context) return;
    const scale = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * scale); canvas.height = Math.round(height * scale);
    context.setTransform(scale, 0, 0, scale, 0, 0);
    const css = getComputedStyle(canvas);
    const text = css.getPropertyValue("--nk-text-muted").trim();
    const border = css.getPropertyValue("--nk-border").trim();
    const plotHeight = height - padding.top - padding.bottom;
    let maximum = 1;
    for (const line of series()) for (const value of line.values) maximum = Math.max(maximum, value);
    const tickStep = Math.max(1, Math.ceil(maximum / 4));
    maximum = Math.ceil(maximum / tickStep) * tickStep;
    const tickCount = maximum / tickStep;
    context.font = "11px system-ui"; context.textBaseline = "middle";
    context.lineWidth = 1;
    for (let tick = 0; tick <= tickCount; tick++) {
      const y = padding.top + plotHeight * tick / tickCount;
      context.strokeStyle = border; context.beginPath(); context.moveTo(padding.left, y); context.lineTo(width - padding.right, y); context.stroke();
      context.fillStyle = text; context.textAlign = "right"; context.fillText((maximum - tick * tickStep).toLocaleString(), padding.left - 8, y, padding.left - 12);
    }
    context.fillStyle = text; context.textAlign = "left"; context.fillText(`${props.metricLabel} / 桶`, padding.left, 10);
    const ticks = width < 520 ? 1 : 3;
    for (let tick = 0; tick <= ticks; tick++) {
      const at = props.from_ms + (props.to_ms - props.from_ms) * tick / ticks;
      const position = padding.left + plotWidth() * tick / ticks;
      context.textAlign = tick === 0 ? "left" : tick === ticks ? "right" : "center";
      if (width < 520) {
        context.fillText(new Date(at).toLocaleDateString(undefined, { month: "2-digit", day: "2-digit" }), position, height - 35);
        context.fillText(new Date(at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }), position, height - 19);
      } else context.fillText(shortTime(at), position, height - 26);
    }
    for (const line of series()) {
      context.strokeStyle = modelColor(line.key); context.lineWidth = 2; context.beginPath();
      for (let index = 0; index < line.values.length; index++) {
        const y = padding.top + plotHeight * (1 - line.values[index] / maximum);
        if (index === 0) context.moveTo(x(index), y); else context.lineTo(x(index), y);
      }
      context.stroke();
      if (line.values.length === 1) { context.fillStyle = modelColor(line.key); context.beginPath(); context.arc(x(0), padding.top + plotHeight * (1 - line.values[0] / maximum), 3, 0, Math.PI * 2); context.fill(); }
    }
    if (hover() !== null) { context.strokeStyle = text; context.lineWidth = 1; context.setLineDash([4, 4]); context.beginPath(); context.moveTo(x(hover()!), padding.top); context.lineTo(x(hover()!), padding.top + plotHeight); context.stroke(); context.setLineDash([]); }
  }
  function pointAt(event: PointerEvent) {
    const rect = canvas.getBoundingClientRect();
    const position = Math.max(0, Math.min(1, (event.clientX - rect.left - padding.left) / plotWidth()));
    const at = props.from_ms + position * (props.to_ms - props.from_ms);
    setHover(Math.max(0, Math.min(buckets().count - 1, Math.floor((at - buckets().start) / props.bucket_ms))));
  }
  onMount(() => {
    const resize = new ResizeObserver((entries) => { width = Math.max(1, entries[0]?.contentRect.width ?? 800); setRevision((value) => value + 1); });
    resize.observe(canvas);
    const theme = new MutationObserver(() => setRevision((value) => value + 1));
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    onCleanup(() => { resize.disconnect(); theme.disconnect(); });
  });
  createEffect(() => { revision(); series(); hover(); draw(); });
  createEffect(() => { series(); props.from_ms; props.to_ms; props.metricLabel; setHover(null); });
  return <figure class="admin-chart">
    <figcaption>时间 × {props.metricLabel} · 每桶 {Math.round(props.bucket_ms / 60000)} 分钟</figcaption>
    <p id={helpId} class="admin-note">悬停或轻触查看数值；键盘聚焦图表后用 ← → 切换时间桶，Home / End 跳至首尾。无调用的桶补零，横轴为本地时间。</p>
    <canvas ref={canvas} class="admin-token-canvas" tabIndex={0} role="img" aria-label={props.label} aria-describedby={helpId}
      onPointerDown={pointAt} onPointerMove={pointAt}
      onPointerLeave={(event) => { if (event.pointerType === "mouse" && document.activeElement !== canvas) setHover(null); }}
      onFocus={() => setHover((value) => value ?? 0)} onBlur={() => setHover(null)}
      onKeyDown={(event) => {
        if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
        event.preventDefault();
        setHover((value) => event.key === "Home" ? 0 : event.key === "End" ? buckets().count - 1 : Math.max(0, Math.min(buckets().count - 1, (value ?? 0) + (event.key === "ArrowLeft" ? -1 : 1))));
      }} />
    <ul class="admin-legend"><For each={props.series}>{(line) => <li><span class="admin-color-dot" style={{ background: modelColor(line.key) }} />{line.label}<Show when={line.detail}><small>({line.detail})</small></Show></li>}</For></ul>
    <Show when={hover() !== null}><div class="admin-chart-values" aria-live="polite"><strong>{shortTime(time(hover()!))} — {shortTime(Math.min(props.to_ms, bucketStart(hover()!) + props.bucket_ms))}</strong><ul class="admin-legend"><For each={series()}>{(line) => <li><span class="admin-color-dot" style={{ background: modelColor(line.key) }} />{line.label}: <b>{(line.values[hover()!] ?? 0).toLocaleString()}</b></li>}</For></ul></div></Show>
  </figure>;
}

export function TokenChart(props: { stats: Stats; models: ModelStats[]; metric: TokenMetric }) {
  const series = createMemo(() => props.models.map((model) => ({
    key: model.key, label: model.model_label, detail: `${model.provider} / ${model.upstream_id}`,
    points: model.points.map((point) => ({ at_ms: point.at_ms, value: tokens(point, props.metric) })),
  })));
  return <UsageLineChart from_ms={props.stats.from_ms} to_ms={props.stats.to_ms} bucket_ms={props.stats.bucket_ms}
    series={series()} metricLabel={metricLabels[props.metric]} label={`各模型${metricLabels[props.metric]}时间图`} />;
}
