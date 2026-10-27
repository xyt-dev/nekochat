import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { ModelStats, Stats, Totals } from "./api";
export type TokenMetric = "total" | "cached_in" | "uncached_in" | "reasoning" | "output";
export const metricLabels: Record<TokenMetric, string> = { total: "总 token", cached_in: "缓存输入", uncached_in: "无缓存输入", reasoning: "思考", output: "输出（不含思考）" };
export const tokens = (value: Totals, metric: TokenMetric) => metric === "total" ? value.cached_in + value.uncached_in + value.reasoning + value.output : value[metric];
export function modelColor(key: string) {
  let hash = 2166136261;
  for (let index = 0; index < key.length; index++) hash = Math.imul(hash ^ key.charCodeAt(index), 16777619);
  return `hsl(${(hash >>> 0) % 360} 67% 52%)`;
}

export function TokenChart(props: { stats: Stats; models: ModelStats[]; metric: TokenMetric }) {
  let canvas!: HTMLCanvasElement;
  let width = 800;
  const height = 300;
  const padding = { left: 68, right: 20, top: 25, bottom: 48 };
  const [hover, setHover] = createSignal<number | null>(null);
  const [revision, setRevision] = createSignal(0);
  const buckets = createMemo(() => {
    const start = Math.floor(props.stats.from_ms / props.stats.bucket_ms) * props.stats.bucket_ms;
    const count = Math.ceil((props.stats.to_ms - start) / props.stats.bucket_ms);
    return { start, count };
  });
  const series = createMemo(() => props.models.map((model) => {
    const values = new Array<number>(buckets().count).fill(0);
    for (const point of model.points) {
      const index = Math.round((point.at_ms - buckets().start) / props.stats.bucket_ms);
      if (index >= 0 && index < values.length) values[index] = tokens(point, props.metric);
    }
    return { model, values };
  }));
  const time = (index: number) => new Date(buckets().start + index * props.stats.bucket_ms);
  const shortTime = (date: Date) => date.toLocaleString(undefined, { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
  function draw() {
    const context = canvas?.getContext("2d"); if (!context) return;
    const scale = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * scale); canvas.height = Math.round(height * scale);
    context.setTransform(scale, 0, 0, scale, 0, 0);
    const css = getComputedStyle(canvas);
    const text = css.getPropertyValue("--nk-text-muted").trim();
    const border = css.getPropertyValue("--nk-border").trim();
    const plotWidth = Math.max(1, width - padding.left - padding.right);
    const plotHeight = height - padding.top - padding.bottom;
    let maximum = 1;
    for (const line of series()) for (const value of line.values) maximum = Math.max(maximum, value);
    context.font = "11px system-ui"; context.textBaseline = "middle";
    context.lineWidth = 1;
    for (let tick = 0; tick <= 4; tick++) {
      const y = padding.top + plotHeight * tick / 4;
      context.strokeStyle = border; context.beginPath(); context.moveTo(padding.left, y); context.lineTo(width - padding.right, y); context.stroke();
      context.fillStyle = text; context.textAlign = "right"; context.fillText(Math.round(maximum * (1 - tick / 4)).toLocaleString(), padding.left - 8, y);
    }
    context.fillStyle = text; context.textAlign = "left"; context.fillText(`${metricLabels[props.metric]} / 桶`, padding.left, 10);
    const x = (index: number) => padding.left + index / Math.max(1, buckets().count - 1) * plotWidth;
    const ticks = width < 520 ? 1 : 3;
    for (let tick = 0; tick <= ticks; tick++) {
      const index = Math.round(Math.max(0, buckets().count - 1) * tick / ticks);
      context.textAlign = tick === 0 ? "left" : tick === ticks ? "right" : "center";
      context.fillText(shortTime(time(index)), x(index), height - 22);
    }
    for (const line of series()) {
      context.strokeStyle = modelColor(line.model.key); context.lineWidth = 2; context.beginPath();
      for (let index = 0; index < line.values.length; index++) {
        const y = padding.top + plotHeight * (1 - line.values[index] / maximum);
        if (index === 0) context.moveTo(x(index), y); else context.lineTo(x(index), y);
      }
      context.stroke();
      if (line.values.length === 1) { context.fillStyle = modelColor(line.model.key); context.beginPath(); context.arc(x(0), padding.top + plotHeight * (1 - line.values[0] / maximum), 3, 0, Math.PI * 2); context.fill(); }
    }
    if (hover() !== null) { context.strokeStyle = text; context.lineWidth = 1; context.setLineDash([4, 4]); context.beginPath(); context.moveTo(x(hover()!), padding.top); context.lineTo(x(hover()!), padding.top + plotHeight); context.stroke(); context.setLineDash([]); }
  }
  onMount(() => {
    const resize = new ResizeObserver((entries) => { width = Math.max(1, entries[0]?.contentRect.width ?? 800); setRevision((value) => value + 1); });
    resize.observe(canvas);
    const theme = new MutationObserver(() => setRevision((value) => value + 1));
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    onCleanup(() => { resize.disconnect(); theme.disconnect(); });
  });
  createEffect(() => { revision(); series(); hover(); draw(); });
  createEffect(() => { props.stats; props.metric; setHover(null); });
  return <figure class="admin-chart">
    <figcaption>时间 × {metricLabels[props.metric]} · 每桶 {Math.round(props.stats.bucket_ms / 60000)} 分钟</figcaption>
    <p id="chart-help" class="admin-note">悬停或轻触查看数值；键盘聚焦图表后用 ← → 切换时间桶，Home / End 跳至首尾。无调用的桶补零，横轴为本地时间。</p>
    <canvas ref={canvas} class="admin-token-canvas" tabIndex={0} role="img" aria-label={`各模型${metricLabels[props.metric]}时间图`} aria-describedby="chart-help" onPointerMove={(event) => { const rect = canvas.getBoundingClientRect(); const position = (event.clientX - rect.left - padding.left) / Math.max(1, width - padding.left - padding.right); setHover(Math.max(0, Math.min(buckets().count - 1, Math.round(position * (buckets().count - 1))))); }} onPointerLeave={() => { if (document.activeElement !== canvas) setHover(null); }} onFocus={() => setHover((value) => value ?? 0)} onKeyDown={(event) => { if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return; event.preventDefault(); setHover((value) => event.key === "Home" ? 0 : event.key === "End" ? buckets().count - 1 : Math.max(0, Math.min(buckets().count - 1, (value ?? 0) + (event.key === "ArrowRight" ? 1 : -1)))); }} />
    <ul class="admin-legend"><For each={props.models}>{(model) => <li><span class="admin-color-dot" style={{ background: modelColor(model.key) }} />{model.model_label} <small>({model.provider} / {model.upstream_id})</small></li>}</For></ul>
    <Show when={hover() !== null}><div class="admin-chart-values" aria-live="polite"><strong>{shortTime(time(hover()!))} — {shortTime(new Date(Math.min(props.stats.to_ms, time(hover()!).getTime() + props.stats.bucket_ms)))}</strong><ul class="admin-legend"><For each={series()}>{(line) => <li><span class="admin-color-dot" style={{ background: modelColor(line.model.key) }} />{line.model.model_label}: <b>{(line.values[hover()!] ?? 0).toLocaleString()}</b></li>}</For></ul></div></Show>
  </figure>;
}
