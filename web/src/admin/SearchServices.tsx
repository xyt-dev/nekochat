import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { errorMessage, type AdminApi, type SearchConnection, type SearchResponse, type SearchService, type SearchServiceWrite, type SearchUsage, type SearchUsageRecord } from "./api";

function citationUrl(value: string): string | undefined {
  try { const url = new URL(value); return ["https:", "http:"].includes(url.protocol) && !url.username && !url.password ? url.href : undefined; } catch { return undefined; }
}
function costLabel(usage: SearchUsage | null): string {
  const cost = usage?.cost;
  return cost && Number.isFinite(cost.amount) && cost.amount >= 0 ? `${cost.amount.toLocaleString(undefined, { maximumSignificantDigits: 12 })} ${cost.currency}` : "费用未知（不是 0）";
}
function UsageDetails(props: { usage: SearchUsage }) {
  return <div class="admin-search-usage-detail"><strong>{costLabel(props.usage)}</strong><span>请求 {props.usage.requests} · 缓存输入 {props.usage.cached_in} · 非缓存输入 {props.usage.uncached_in} · 输出 {props.usage.output} · 推理 {props.usage.reasoning}</span></div>;
}

function ConnectionForm(props: { api: AdminApi; onSaved: () => Promise<void> }) {
  const [connection, setConnection] = createSignal<SearchConnection | null>(null);
  const [enabled, setEnabled] = createSignal(false);
  const [baseUrl, setBaseUrl] = createSignal("");
  const [timeout, setTimeout] = createSignal("60");
  const [calls, setCalls] = createSignal("3");
  const [results, setResults] = createSignal("5");
  const [accessToken, setAccessToken] = createSignal("");
  const [adminToken, setAdminToken] = createSignal("");
  const [clearAccess, setClearAccess] = createSignal(false);
  const [clearAdmin, setClearAdmin] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [busy, setBusy] = createSignal("");
  const [error, setError] = createSignal("");
  const [notice, setNotice] = createSignal("");
  const controller = new AbortController();
  let disposed = false;
  onCleanup(() => { disposed = true; controller.abort(); setAccessToken(""); setAdminToken(""); });
  function apply(value: SearchConnection) {
    setConnection(value); setEnabled(value.enabled); setBaseUrl(value.base_url); setTimeout(String(value.timeout_secs)); setCalls(String(value.max_calls_per_turn)); setResults(String(value.max_results));
  }
  async function load() {
    if (disposed || loading() || busy()) return;
    setLoading(true); setError("");
    try { const value = await props.api.searchConnection(controller.signal); if (!disposed) apply(value); }
    catch (failure) { if (!disposed) setError(errorMessage(failure)); }
    finally { if (!disposed) setLoading(false); }
  }
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (disposed || busy() || loading()) return;
    setBusy("save"); setError(""); setNotice("");
    try {
      const bounded = (raw: string, max: number, label: string) => { const value = Number(raw); if (!raw.trim() || !Number.isInteger(value) || value < 1 || value > max) throw new Error(`${label}须为 1–${max} 的整数`); return value; };
      const value = await props.api.saveSearchConnection({ enabled: enabled(), base_url: baseUrl().trim(), timeout_secs: bounded(timeout(), 180, "超时秒数"), max_calls_per_turn: bounded(calls(), 8, "每轮工具调用预算"), max_results: bounded(results(), 10, "搜索结果预算"), ...(accessToken().trim() ? { access_token: accessToken().trim() } : {}), ...(adminToken().trim() ? { admin_token: adminToken().trim() } : {}), clear_access_token: clearAccess(), clear_admin_token: clearAdmin() }, controller.signal);
      if (disposed) return;
      apply(value); setAccessToken(""); setAdminToken(""); setClearAccess(false); setClearAdmin(false); setNotice("连接配置已保存。凭证不会回读；连接测试使用这份已保存配置。");
      await props.onSaved();
    } catch (failure) { if (!disposed) setError(errorMessage(failure)); }
    finally { if (!disposed) setBusy(""); }
  }
  async function test() {
    if (disposed || busy() || loading() || !connection()) return;
    setBusy("test"); setError(""); setNotice("");
    try { const value = await props.api.testSearchConnection(controller.signal); if (!disposed) setNotice(value.ok ? `已保存配置连接成功：运行时与管理授权已验证，${value.services} 个服务。没有发起计费搜索。` : "连接未通过验证"); }
    catch (failure) { if (!disposed) setError(errorMessage(failure)); }
    finally { if (!disposed) setBusy(""); }
  }
  onMount(() => void load());
  return <section class="admin-card" aria-labelledby="search-connection-title" aria-busy={loading() || !!busy()}><div class="admin-section-heading"><div><h3 id="search-connection-title">独立搜索引擎连接</h3><p class="admin-note">浏览器仅访问本站管理 API；引擎地址和凭证保存在服务端。搜索服务故障不会阻止其他管理页面加载。</p></div><button type="button" disabled={loading() || !!busy()} onClick={() => void load()}>重新读取</button></div>
    <Show when={loading()}><p role="status">正在读取连接配置…</p></Show>
    <Show when={connection()}><form onSubmit={(event) => void save(event)}><fieldset disabled={loading() || !!busy()}><div class="admin-form-grid">
      <label class="admin-check"><input name="search-enabled" type="checkbox" checked={enabled()} onChange={(event) => setEnabled(event.currentTarget.checked)} />启用角色搜索</label>
      <label>后端地址<input name="search-base-url" type="url" placeholder="http://127.0.0.1:10011" required={enabled()} value={baseUrl()} onInput={(event) => setBaseUrl(event.currentTarget.value)} /></label>
      <label>请求超时（秒，1–180）<input name="search-timeout" type="number" min="1" max="180" step="1" required value={timeout()} onInput={(event) => setTimeout(event.currentTarget.value)} /></label>
      <label>每轮工具调用预算（1–8）<input name="search-max-calls" type="number" min="1" max="8" step="1" required value={calls()} onInput={(event) => setCalls(event.currentTarget.value)} /></label>
      <label>每次结果上限（1–10）<input name="search-max-results" type="number" min="1" max="10" step="1" required value={results()} onInput={(event) => setResults(event.currentTarget.value)} /></label>
    </div><div class="admin-search-secrets">
      <div><label>替换运行时令牌<input name="search-access-token" type="password" autocomplete="new-password" disabled={clearAccess()} placeholder={connection()?.has_access_token ? "已配置；留空保留" : "未配置；输入新令牌"} value={accessToken()} onInput={(event) => setAccessToken(event.currentTarget.value)} /></label><label class="admin-check"><input type="checkbox" checked={clearAccess()} onChange={(event) => { setClearAccess(event.currentTarget.checked); setAccessToken(""); }} />保存时明确清除运行时令牌</label></div>
      <div><label>替换管理令牌<input name="search-admin-token" type="password" autocomplete="new-password" disabled={clearAdmin()} placeholder={connection()?.has_admin_token ? "已配置；留空保留" : "未配置；输入新令牌"} value={adminToken()} onInput={(event) => setAdminToken(event.currentTarget.value)} /></label><label class="admin-check"><input type="checkbox" checked={clearAdmin()} onChange={(event) => { setClearAdmin(event.currentTarget.checked); setAdminToken(""); }} />保存时明确清除管理令牌</label></div>
    </div><p class="admin-note">两种令牌必须不同。输入框始终为空开始；留空保留已有值。启用连接需要有效地址和两种令牌，清除令牌前请停用连接。</p></fieldset><div class="admin-actions"><button class="primary" type="submit" disabled={loading() || !!busy()}>{busy() === "save" ? "保存中…" : "保存连接"}</button><button type="button" disabled={loading() || !!busy()} onClick={() => void test()}>{busy() === "test" ? "验证中…" : "测试已保存连接（不计费）"}</button></div></form></Show>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show><Show when={notice()}><p class="admin-notice" role="status">{notice()}</p></Show>
  </section>;
}

function ServiceEditor(props: { api: AdminApi; initial: SearchService | null; onSaved: (value: SearchService) => Promise<void>; onCancel: () => void }) {
  const initial = props.initial;
  const [name, setName] = createSignal(initial?.name ?? "");
  const [kind, setKind] = createSignal<SearchServiceWrite["kind"]>(initial?.kind ?? "deepseek");
  const [auth, setAuth] = createSignal<SearchServiceWrite["auth_mode"]>(initial?.auth_mode ?? "shared_chatgpt");
  const [model, setModel] = createSignal(initial?.model ?? "");
  const [enabled, setEnabled] = createSignal(initial?.enabled ?? true);
  const [key, setKey] = createSignal("");
  const [clearKey, setClearKey] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  let disposed = false;
  onCleanup(() => { disposed = true; setKey(""); });
  async function save(event: SubmitEvent) {
    event.preventDefault(); if (busy() || disposed) return;
    setBusy(true); setError("");
    try {
      if (!name().trim()) throw new Error("服务名称不能为空");
      const keyMode = kind() === "deepseek" || auth() === "api_key";
      const value = await props.api.saveSearchService(initial?.id ?? null, { name: name().trim(), kind: kind(), enabled: enabled(), auth_mode: kind() === "deepseek" ? "api_key" : auth(), model: kind() === "gpt" ? model().trim() || null : null, ...(keyMode && key().trim() ? { api_key: key().trim() } : {}), clear_api_key: keyMode && clearKey() });
      setKey(""); setClearKey(false);
      if (!disposed) await props.onSaved(value);
    } catch (failure) { if (!disposed) setError(errorMessage(failure)); }
    finally { if (!disposed) setBusy(false); }
  }
  return <section class="admin-card admin-editor" aria-labelledby="search-service-editor-title" aria-busy={busy()}><h3 id="search-service-editor-title">{initial ? "编辑搜索服务" : "新增搜索服务"}</h3><form onSubmit={(event) => void save(event)}><fieldset disabled={busy()}><div class="admin-form-grid">
    <label>服务名称<input name="search-service-name" required value={name()} onInput={(event) => setName(event.currentTarget.value)} /></label>
    <label>提供商<select name="search-service-kind" value={kind()} disabled={!!initial} onChange={(event) => { setKind(event.currentTarget.value as SearchServiceWrite["kind"]); setAuth(event.currentTarget.value === "gpt" ? "shared_chatgpt" : "api_key"); setKey(""); setClearKey(false); setModel(""); }}><option value="deepseek">DeepSeek Search / Fetch</option><option value="gpt">GPT Web Search</option></select></label>
    <Show when={kind() === "gpt"}><label>授权方式<select name="search-service-auth" value={auth()} onChange={(event) => { setAuth(event.currentTarget.value as SearchServiceWrite["auth_mode"]); setKey(""); setClearKey(false); }}><option value="shared_chatgpt">ChatGPT 订阅 · 复用本站账号</option><option value="api_key">API key · 按 API 计费</option></select></label><label>GPT 模型<input name="search-service-model" value={model()} placeholder="留空使用引擎默认模型" onInput={(event) => setModel(event.currentTarget.value)} /></label></Show>
    <label class="admin-check"><input name="search-service-enabled" type="checkbox" checked={enabled()} onChange={(event) => setEnabled(event.currentTarget.checked)} />启用此搜索服务</label>
  </div><Show when={kind() === "deepseek" || auth() === "api_key"} fallback={<p class="admin-note">复用本站已连接的 ChatGPT 订阅，账号连接由现有「模型管理」统一管理，无需再次登录。搜索服务不保存订阅凭证，也不会回退到 API key；切换至此模式会清除该服务原有的 API key。</p>}><label>替换 API key<input name="search-service-key" type="password" autocomplete="new-password" disabled={clearKey()} value={key()} placeholder={initial?.has_api_key ? "已配置；留空保留" : "未配置；输入新的 API key"} onInput={(event) => setKey(event.currentTarget.value)} /></label>
    <label class="admin-check"><input type="checkbox" checked={clearKey()} onChange={(event) => { setClearKey(event.currentTarget.checked); setKey(""); }} />保存时明确清除已有 API key</label>
    <p class="admin-note">密钥只写不读；同一 API key 模式下留空保留已有密钥，勾选清除才会删除。</p></Show></fieldset>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show><div class="admin-actions"><button type="submit" class="primary" disabled={busy()}>{busy() ? "保存中…" : "保存服务"}</button><button type="button" disabled={busy()} onClick={props.onCancel}>取消</button></div>
  </form></section>;
}


function ServiceTest(props: { api: AdminApi; service: SearchService; onAttempted: () => void }) {
  const [query, setQuery] = createSignal("");
  const [consent, setConsent] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [result, setResult] = createSignal<SearchResponse | null>(null);
  let disposed = false;
  onCleanup(() => { disposed = true; });
  async function test(event: SubmitEvent) {
    event.preventDefault(); if (disposed || busy() || !consent()) return;
    const text = query().trim();
    if (!text || Array.from(text).length > 128) { setError("查询须为 1–128 个 Unicode 字符"); return; }
    setBusy(true); setError(""); setResult(null);
    try { const value = await props.api.testSearchService(props.service.id, text); if (!disposed) setResult(value); }
    catch (failure) { if (!disposed) setError(errorMessage(failure)); }
    finally { props.onAttempted(); if (!disposed) setBusy(false); }
  }
  return <details class="admin-search-test"><summary>真实搜索测试（会调用上游，可能计费）</summary><p class="admin-note">这不是连接测试。API 模式按提供商计费，会员模式消耗会员额度。不会自动重试或切换服务；失败也可能已产生上游用量。</p><form onSubmit={(event) => void test(event)}><fieldset disabled={busy()}><label>公开测试问题<input name={`search-query-${props.service.id}`} required value={query()} onInput={(event) => setQuery(event.currentTarget.value)} placeholder="输入不含隐私的公开问题" /></label><label class="admin-check"><input type="checkbox" checked={consent()} onChange={(event) => setConsent(event.currentTarget.checked)} />我确认发起真实搜索并承担可能的费用或会员额度消耗</label></fieldset><button type="submit" disabled={busy() || !consent() || !props.service.enabled}>{busy() ? "真实搜索中…" : "执行真实搜索"}</button></form>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show><Show when={result()}>{(value) => <section class="admin-search-result" aria-label="真实搜索结果"><p class="admin-note">查询：{value().query} · 检索时间：{new Date(value().retrieved_at_ms).toLocaleString()}</p><UsageDetails usage={value().usage} /><Show when={value().summary}><h4>提供商生成回答（非网页原文）</h4><pre class="admin-search-text">{value().summary}</pre></Show><h4>规范化来源（{value().sources.length}）</h4><For each={value().sources} fallback={<p class="admin-note">上游未返回规范化来源。不会捏造引用。</p>}>{(source) => <article class="admin-search-source"><h4>[{source.id}] {source.title || "未命名来源"}</h4><Show when={citationUrl(source.url)} fallback={<span class="admin-note">来源链接无效</span>}>{(url) => <a href={url()} target="_blank" rel="noopener noreferrer">{source.url} ↗</a>}</Show><Show when={source.published_at}><p class="admin-note">发布时间：{source.published_at}</p></Show><Show when={source.snippet}><p>{source.snippet}</p></Show><Show when={source.summary}><details><summary>提供商生成摘要（非网页原文）</summary><pre class="admin-search-text">{source.summary}</pre></details></Show><Show when={source.content} fallback={<p class="admin-note">未返回网页正文。</p>}><details><summary>上游提供的网页正文</summary><pre class="admin-search-text">{source.content}</pre></details></Show></article>}</For></section>}</Show>
  </details>;
}

function UsagePanel(props: { api: AdminApi; services: SearchService[]; revision: number }) {
  const [records, setRecords] = createSignal<SearchUsageRecord[]>([]);
  const [hours, setHours] = createSignal("24");
  const [loading, setLoading] = createSignal(false);
  const [loaded, setLoaded] = createSignal(false);
  const [error, setError] = createSignal("");
  let disposed = false;
  let controller: AbortController | undefined;
  let version = 0;
  onCleanup(() => { disposed = true; version++; controller?.abort(); });
  const totals = createMemo(() => {
    const currencies = new Map<string, number>(); let unknown = 0;
    for (const record of records()) { const cost = record.usage?.cost; if (!cost || !Number.isFinite(cost.amount) || cost.amount < 0) unknown++; else currencies.set(cost.currency, (currencies.get(cost.currency) ?? 0) + cost.amount); }
    return { currencies: Array.from(currencies), unknown };
  });
  async function load() {
    if (disposed || loading()) return;
    const current = ++version; controller?.abort(); controller = new AbortController(); setLoading(true); setError("");
    try { const to = Date.now(); const value = await props.api.searchUsage(to - Number(hours()) * 3600_000, to, controller.signal); if (!disposed && current === version) { setRecords(value); setLoaded(true); } }
    catch (failure) { if (!disposed && current === version) setError(errorMessage(failure)); }
    finally { if (!disposed && current === version) setLoading(false); }
  }
  onMount(() => void load());
  return <section class="admin-card" aria-labelledby="search-usage-title" aria-busy={loading()}><div class="admin-section-heading"><div><h3 id="search-usage-title">搜索用量 · 独立于模型账单</h3><p class="admin-note">仅记录服务、时间、状态和真实用量元数据，不记录查询、来源 URL 或用户信息。不换算汇率，未知费用不视为零。</p><Show when={props.revision > 0}><p class="admin-note">本页已执行过测试，请刷新查看最新记录。</p></Show></div><div class="admin-actions"><label>时间范围<select value={hours()} disabled={loading()} onChange={(event) => { setHours(event.currentTarget.value); void load(); }}><option value="24">近 24 小时</option><option value="168">近 7 天</option><option value="720">近 30 天</option></select></label><button type="button" disabled={loading()} onClick={() => void load()}>{loading() ? "加载中…" : "刷新用量"}</button></div></div>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show><Show when={loaded()}><div class="admin-search-costs"><For each={totals().currencies}>{([currency, amount]) => <span class="admin-badge">已知费用 {amount.toLocaleString(undefined, { maximumSignificantDigits: 12 })} {currency}</span>}</For><span class="admin-note">{records().length} 条尝试 · {totals().unknown} 条费用未知</span></div><div class="admin-search-records"><For each={records()} fallback={<p class="admin-empty">此时间范围尚无搜索用量记录。</p>}>{(record) => <article class="admin-search-record"><div><strong>{props.services.find((service) => service.id === record.service_id)?.name ?? record.service_id}</strong><span class="admin-badge">{record.status}</span><small>{new Date(record.at_ms).toLocaleString()}</small></div><Show when={record.usage} fallback={<p class="admin-note">上游未提供用量 · 费用未知（不是 0）</p>}>{(usage) => <UsageDetails usage={usage()} />}</Show></article>}</For></div></Show>
  </section>;
}

export function SearchServices(props: { api: AdminApi }) {
  const [services, setServices] = createSignal<SearchService[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [loaded, setLoaded] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [editor, setEditor] = createSignal<{ initial: SearchService | null } | null>(null);
  const [error, setError] = createSignal("");
  const [notice, setNotice] = createSignal("");
  const [revision, setRevision] = createSignal(0);
  let disposed = false;
  let version = 0;
  let controller: AbortController | undefined;
  onCleanup(() => { disposed = true; version++; controller?.abort(); });
  async function load(committed = "") {
    if (disposed) return;
    const current = ++version; controller?.abort(); controller = new AbortController(); setLoading(true); setError("");
    try { const value = await props.api.searchServices(controller.signal); if (!disposed && current === version) { setServices(value); setLoaded(true); } }
    catch (failure) { if (!disposed && current === version) setError(`${committed ? `${committed}，但服务目录刷新失败（无需重复保存）：` : "服务目录暂不可用："}${errorMessage(failure)}`); }
    finally { if (!disposed && current === version) setLoading(false); }
  }
  async function saved(value: SearchService) {
    if (disposed) return;
    setServices((items) => [...items.filter((item) => item.id !== value.id), value]); setLoaded(true); setEditor(null); setNotice("搜索服务已保存");
    await load("搜索服务已保存");
  }
  async function remove(service: SearchService) {
    if (disposed || busy() || loading() || editor()) return;
    if (!confirm(`删除搜索服务「${service.name}」？仍被角色绑定的服务不能删除。`)) return;
    setBusy(true); setError(""); setNotice("");
    try { await props.api.deleteSearchService(service.id); if (disposed) return; setServices((items) => items.filter((item) => item.id !== service.id)); setNotice("搜索服务已删除"); await load("搜索服务已删除"); }
    catch (failure) { if (!disposed) setError(errorMessage(failure)); }
    finally { if (!disposed) setBusy(false); }
  }
  onMount(() => void load());
  return <section class="admin-search" aria-labelledby="search-title"><div class="admin-section-heading"><div><h2 id="search-title">搜索服务</h2><p class="admin-note">独立搜索引擎；GPT 可复用本站 ChatGPT 订阅。在「角色管理」按角色绑定可选服务，普通聊天端不提供提供商、模型或密钥设置。</p></div></div>
    <ConnectionForm api={props.api} onSaved={() => load("连接配置已保存")} />
    <div class="admin-section-heading"><div><h3>搜索服务目录</h3><p class="admin-note">DeepSeek 支持搜索与网页获取；GPT 使用原生 Web Search。API key 在此配置，本站 ChatGPT 订阅连接在现有「模型管理」管理；连接状态由本站后端提供，账号变更后可刷新目录。</p></div><div class="admin-actions"><button type="button" disabled={loading() || busy() || !!editor()} onClick={() => void load()}>{loading() ? "加载中…" : "刷新目录"}</button><button type="button" class="primary" disabled={loading() || busy() || !!editor()} onClick={() => { setEditor({ initial: null }); setNotice(""); }}>新增搜索服务</button></div></div>
    <Show when={notice()}><p class="admin-notice" role="status">{notice()}</p></Show><Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show><Show when={editor()} keyed>{(draft) => <ServiceEditor api={props.api} initial={draft.initial} onSaved={saved} onCancel={() => setEditor(null)} />}</Show>
    <div class="admin-search-services"><For each={services()} fallback={<Show when={loaded()}><p class="admin-empty">尚未配置搜索服务。</p></Show>}>{(service) => <article class="admin-card"><div class="admin-section-heading"><div><h3>{service.name} <span class="admin-badge">{service.kind === "deepseek" ? "DeepSeek" : "GPT"}</span></h3><p class="admin-note">{service.enabled ? "已启用" : "已停用"} · {service.connected ? "已连接" : "未连接"} · {service.auth_mode === "shared_chatgpt" ? "本站 ChatGPT 订阅" : "API key"} · {service.supports_fetch ? "搜索 + 网页获取" : "搜索"}{service.model ? ` · ${service.model}` : ""}</p><Show when={service.auth_mode === "shared_chatgpt"} fallback={<p class="admin-note">API key：{service.has_api_key ? "已配置（不回读）" : "未配置"}</p>}><p class="admin-note">复用本站已连接的 ChatGPT 订阅，无需再次登录；请在现有「模型管理」管理账号连接。搜索服务不保存订阅凭证。</p></Show></div><div class="admin-actions"><button type="button" disabled={busy() || loading() || !!editor()} onClick={() => { setEditor({ initial: service }); setNotice(""); }}>编辑</button><button type="button" class="danger" disabled={busy() || loading() || !!editor()} onClick={() => void remove(service)}>删除</button></div></div><ServiceTest api={props.api} service={service} onAttempted={() => { if (!disposed) setRevision((value) => value + 1); }} /></article>}</For></div>
    <UsagePanel api={props.api} services={services()} revision={revision()} />
  </section>;
}
