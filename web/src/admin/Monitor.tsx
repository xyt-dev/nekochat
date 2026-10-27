import { createMemo, createSignal, For, onMount, Show } from "solid-js";
import { errorMessage, type AdminApi, type ModelStats, type Quotas, type Stats, type Totals } from "./api";
import { metricLabels, modelColor, TokenChart, type TokenMetric } from "./TokenChart";

const usd = (value: number | null) => value === null ? "未知" : `$${value.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 8 })}`;
function sum(models: ModelStats[]): Totals {
  const result: Totals = { requests: 0, cached_in: 0, uncached_in: 0, reasoning: 0, output: 0, cost_usd: null, unpriced_requests: 0 };
  for (const { totals } of models) {
    result.requests += totals.requests; result.cached_in += totals.cached_in; result.uncached_in += totals.uncached_in;
    result.reasoning += totals.reasoning; result.output += totals.output; result.unpriced_requests += totals.unpriced_requests;
    if (totals.cost_usd !== null) result.cost_usd = (result.cost_usd ?? 0) + totals.cost_usd;
  }
  return result;
}
function localInput(time: number) { const date = new Date(time); return new Date(time - date.getTimezoneOffset() * 60000).toISOString().slice(0, 16); }

export function Monitor(props: { api: AdminApi }) {
  const [preset, setPreset] = createSignal("7d");
  const [from, setFrom] = createSignal(localInput(Date.now() - 7 * 86400000));
  const [to, setTo] = createSignal(localInput(Date.now()));
  const [provider, setProvider] = createSignal("");
  const [model, setModel] = createSignal("");
  const [metric, setMetric] = createSignal<TokenMetric>("total");
  const [stats, setStats] = createSignal<Stats | null>(null);
  const [quotas, setQuotas] = createSignal<Quotas | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [quotaLoading, setQuotaLoading] = createSignal(false);
  const [error, setError] = createSignal("");
  const [quotaError, setQuotaError] = createSignal("");
  let statsVersion = 0;
  let quotaVersion = 0;
  const providers = createMemo(() => {
    const items = new Map<string, string>();
    for (const item of stats()?.models ?? []) items.set(item.provider, item.provider_label);
    for (const item of quotas()?.providers ?? []) items.set(item.key, item.label);
    return [...items.entries()];
  });
  const modelOptions = createMemo(() => (stats()?.models ?? []).filter((item) => !provider() || item.provider === provider()));
  const filtered = createMemo(() => modelOptions().filter((item) => !model() || item.key === model()));
  const totals = createMemo(() => sum(filtered()));
  const providerTotals = createMemo(() => {
    const groups = new Map<string, ModelStats[]>();
    for (const item of filtered()) { const values = groups.get(item.provider) ?? []; values.push(item); groups.set(item.provider, values); }
    return [...groups.entries()].map(([key, values]) => ({ key, label: values[0].provider_label, totals: sum(values) }));
  });
  const selectedModel = createMemo(() => stats()?.models.find((item) => item.key === model()));
  const filteredQuotas = createMemo(() => (quotas()?.providers ?? []).filter((item) => (!provider() || item.key === provider()) && (!selectedModel() || selectedModel()!.provider === item.key)));
  async function loadStats() {
    const version = ++statsVersion;
    setLoading(true); setError(""); setStats(null);
    try {
      const end = preset() === "custom" ? new Date(to()).getTime() : Date.now();
      const start = preset() === "custom" ? new Date(from()).getTime() : end - (preset() === "24h" ? 1 : preset() === "30d" ? 30 : 7) * 86400000;
      if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start >= end) throw new Error("请选择有效的起止时间（开始必须早于结束）");
      if (end - start > 366 * 86400000) throw new Error("统计区间最长为 366 天");
      const bucket = preset() === "24h" ? 900000 : preset() === "7d" ? 3600000 : preset() === "30d" ? 21600000 : Math.max(60000, Math.ceil((end - start) / 1200 / 60000) * 60000);
      const result = await props.api.stats(start, end, bucket);
      if (version === statsVersion) { setStats(result); if (!result.models.some((item) => item.key === model())) setModel(""); }
    } catch (error) { if (version === statsVersion) setError(errorMessage(error)); }
    finally { if (version === statsVersion) setLoading(false); }
  }
  async function loadQuotas() {
    const version = ++quotaVersion; setQuotaLoading(true); setQuotaError("");
    try { const result = await props.api.quotas(); if (version === quotaVersion) setQuotas(result); }
    catch (error) { if (version === quotaVersion) { setQuotaError(errorMessage(error)); setQuotas(null); } }
    finally { if (version === quotaVersion) setQuotaLoading(false); }
  }
  onMount(() => { void loadStats(); void loadQuotas(); });
  const totalCells = (value: Totals) => <><td>{value.requests.toLocaleString()}</td><td>{value.cached_in.toLocaleString()}</td><td>{value.uncached_in.toLocaleString()}</td><td>{value.reasoning.toLocaleString()}</td><td>{value.output.toLocaleString()}</td><td>{usd(value.cost_usd)}{value.cost_usd !== null && value.unpriced_requests > 0 ? "（已知部分）" : ""}</td><td>{value.unpriced_requests.toLocaleString()}</td></>;
  return <section aria-labelledby="monitor-title"><div class="admin-section-heading"><div><h2 id="monitor-title">用量与额度</h2><p class="admin-note">按真实调用时记录聚合，含已下架、删除或改名模型的历史用量。</p></div></div>
    <form class="admin-card admin-toolbar" onSubmit={(event) => { event.preventDefault(); void loadStats(); }}>
      <label>统计区间<select name="stats-range" value={preset()} onChange={(event) => { setPreset(event.currentTarget.value); if (event.currentTarget.value !== "custom") void loadStats(); }}><option value="24h">最近 24 小时</option><option value="7d">最近 7 天</option><option value="30d">最近 30 天</option><option value="custom">自定义</option></select></label>
      <Show when={preset() === "custom"}><label>开始（本地时间）<input name="stats-from" type="datetime-local" required value={from()} onInput={(event) => setFrom(event.currentTarget.value)} /></label><label>结束（不含，本地时间）<input name="stats-to" type="datetime-local" required value={to()} onInput={(event) => setTo(event.currentTarget.value)} /></label></Show>
      <button type="submit" disabled={loading()}>{loading() ? "加载中…" : "刷新统计"}</button>
      <label>提供商过滤<select name="stats-provider" value={provider()} onChange={(event) => { setProvider(event.currentTarget.value); setModel(""); }}><option value="" selected={provider() === ""}>全部提供商</option><For each={providers()}>{([key, label]) => <option value={key} selected={provider() === key}>{label} · {key}</option>}</For></select></label>
      <label>模型过滤<select name="stats-model" value={model()} onChange={(event) => setModel(event.currentTarget.value)}><option value="" selected={model() === ""}>全部模型</option><For each={modelOptions()}>{(item) => <option value={item.key} selected={model() === item.key}>{item.model_label} ({item.provider} / {item.upstream_id})</option>}</For></select></label>
      <label>图表 token 指标<select name="token-metric" value={metric()} onChange={(event) => setMetric(event.currentTarget.value as TokenMetric)}><For each={Object.entries(metricLabels)}>{([key, label]) => <option value={key}>{label}</option>}</For></select></label>
    </form>
    <Show when={error()}><div class="admin-error" role="alert">{error()} <button type="button" onClick={() => void loadStats()}>重试统计</button></div></Show><Show when={loading()}><p role="status">正在加载统计…</p></Show>
    <Show when={stats()} keyed>{(data) => <>
      <p class="admin-note">{new Date(data.from_ms).toLocaleString()} ≤ 调用时间 &lt; {new Date(data.to_ms).toLocaleString()}</p>
      <div class="admin-summary"><article class="admin-card"><small>请求数</small><strong>{totals().requests.toLocaleString()}</strong></article><article class="admin-card"><small>总 token（四类相加）</small><strong>{(totals().cached_in + totals().uncached_in + totals().reasoning + totals().output).toLocaleString()}</strong></article><article class="admin-card"><small>{totals().cost_usd === null ? "花费未知" : totals().unpriced_requests ? "已知花费 · 非完整总额" : "已记录花费"}</small><strong>{usd(totals().cost_usd)}</strong><small>未定价请求：{totals().unpriced_requests}</small></article></div>
      <Show when={filtered().length > 0} fallback={<p class="admin-empty">当前区间或过滤条件下没有调用记录。没有数据不代表花费为零。</p>}>
        <div class="admin-card"><TokenChart stats={data} models={filtered()} metric={metric()} /></div>
        <section class="admin-card"><h3>Token 分类与实际花费</h3><p class="admin-note">输出不包含思考；总 token = 缓存输入 + 无缓存输入 + 思考 + 输出。USD 花费使用调用时保存值，不按当前价格重算；接口未提供分类金额，不按比例虚构分摊。未知不等于零，未定价请求不计入已知花费。</p><div class="admin-table-scroll"><table class="admin-table"><caption>各模型和各提供商账务汇总（USD）</caption><thead><tr><th scope="col">提供商 / 模型</th><th scope="col">请求</th><th scope="col">缓存输入 token</th><th scope="col">无缓存输入 token</th><th scope="col">思考 token</th><th scope="col">输出 token（不含思考）</th><th scope="col">已记录 USD 花费</th><th scope="col">未定价请求</th></tr></thead><tbody><For each={filtered()}>{(item) => <tr><th scope="row"><span class="admin-color-dot" style={{ background: modelColor(item.key) }} />{item.model_label}<small class="admin-table-sub">{item.provider} / {item.upstream_id}</small></th>{totalCells(item.totals)}</tr>}</For><For each={providerTotals()}>{(item) => <tr class="admin-subtotal"><th scope="row">{item.label} 小计</th>{totalCells(item.totals)}</tr>}</For></tbody><tfoot><tr><th scope="row">过滤后合计</th>{totalCells(totals())}</tr></tfoot></table></div></section>
      </Show>
    </>}</Show>
    <section aria-labelledby="quota-title"><div class="admin-section-heading"><div><h3 id="quota-title">余额与滚动额度</h3><p class="admin-note">额度独立于统计区间，使用提供商 / 模型过滤。不同币种保留原值，不折算或相加。</p></div><button type="button" disabled={quotaLoading()} onClick={() => void loadQuotas()}>{quotaLoading() ? "查询额度中…" : "刷新额度"}</button></div>
      <Show when={quotaError()}><p role="alert" class="admin-error">{quotaError()}</p></Show><Show when={quotas()} keyed>{(data) => <><p class="admin-note">查询时间：{new Date(data.as_of_ms).toLocaleString()}</p><div class="admin-card-list"><For each={filteredQuotas()} fallback={<p class="admin-empty">当前过滤条件下暂无提供商额度数据。</p>}>{(item) => <article class="admin-card"><h3>{item.label} <small class="admin-badge">{item.status === "ok" ? "已获取" : item.status === "error" ? "获取失败" : "不可用"}</small></h3><Show when={item.note}><p class="admin-note">{item.note}</p></Show>
        <Show when={item.kind === "opencode_go"}><p class="admin-note">本机滚动窗口估算，仅含本机调用记录，不包含其他客户端用量，不是账户权威余额。有未知请求时「剩余」仅为已知用量估算上界，不可视为确定余额。</p></Show>
        <Show when={item.balance} keyed>{(balance) => <><p>账户可用状态：{balance.available ? "可用" : "不可用"}</p><div class="admin-table-scroll"><table class="admin-table"><caption>{item.label} 原币种余额</caption><thead><tr><th>币种</th><th>总余额</th><th>赠送余额</th><th>充值余额</th></tr></thead><tbody><For each={balance.balances} fallback={<tr><td colspan="4">余额未知 / 无可用余额数据</td></tr>}>{(row) => <tr><th scope="row">{row.currency}</th><td>{row.total}</td><td>{row.granted}</td><td>{row.topped_up}</td></tr>}</For></tbody></table></div></>}</Show>
        <For each={item.models.filter((entry) => !selectedModel() || entry.upstream_id === selectedModel()!.upstream_id)}>{(entry) => <div class="admin-quota-model"><h4>{entry.model_label} <small class="mono">{entry.upstream_id}</small></h4><div class="admin-table-scroll"><table class="admin-table"><caption>{entry.model_label} 本机 USD 额度估算</caption><thead><tr><th>滚动窗口</th><th>限额 USD</th><th>已知已用 USD</th><th>估算剩余 USD</th><th>未知请求</th></tr></thead><tbody><For each={entry.windows}>{(window) => <tr><th scope="row">{window.name === "5h" ? "最近 5 小时" : window.name === "7d" ? "最近 7 天" : "最近 30 天"}</th><td>{usd(window.limit_usd)}</td><td>{usd(window.used_usd)}{window.unknown_requests > 0 && window.used_usd !== null ? "（部分）" : ""}</td><td>{usd(window.remaining_usd)}{window.unknown_requests > 0 && window.remaining_usd !== null ? "（不确定，上界）" : ""}</td><td>{window.unknown_requests}</td></tr>}</For></tbody></table></div></div>}</For>
        <Show when={!item.balance && item.models.length === 0}><p class="admin-note">无法取得余额或额度，当前值未知。</p></Show>
      </article>}</For></div></>}</Show>
    </section>
  </section>;
}
