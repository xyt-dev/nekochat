import { createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { emptyModel, errorMessage, type AdminApi, type ListedModel, type Provider, type ProviderProtocol, type ProviderWrite, type UpstreamModel } from "./api";

export function ProviderEditor(props: {
  api: AdminApi; initial: Provider | null; models: ListedModel[];
  onChanged: () => Promise<void>; onSaved: (key: string) => Promise<void>; onCancel: () => void;
}) {
  const [key, setKey] = createSignal(props.initial?.key ?? "");
  const [label, setLabel] = createSignal(props.initial?.label ?? "");
  const [base, setBase] = createSignal(props.initial?.base_url ?? "");
  const [apiKey, setApiKey] = createSignal("");
  const [modelsUrl, setModelsUrl] = createSignal(props.initial?.models_url ?? "");
  const [sessionHeader, setSessionHeader] = createSignal(props.initial?.session_header ?? "");
  const existingProtocols = new Set(props.models.filter((model) => model.provider === props.initial?.key).map((model) => model.protocol));
  const inferredProtocol = existingProtocols.size === 1 ? [...existingProtocols][0] : null;
  const initialProtocol: ProviderProtocol = props.initial?.protocol ?? (inferredProtocol === "responses" ? "responses" : "chat_completions");
  const [protocol, setProtocol] = createSignal<ProviderProtocol>(initialProtocol);
  const [savedProtocol, setSavedProtocol] = createSignal<ProviderProtocol | null>(props.initial?.protocol ?? null);
  const [hasApiKey, setHasApiKey] = createSignal(props.initial?.has_api_key === true);
  const [savedKey, setSavedKey] = createSignal(props.initial?.key ?? "");
  const [directory, setDirectory] = createSignal<UpstreamModel[] | null>(null);
  const [selected, setSelected] = createSignal(new Set<string>());
  const [query, setQuery] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [notice, setNotice] = createSignal("");
  let disposed = false;
  onCleanup(() => { disposed = true; setApiKey(""); });
  const listed = (id: string) => props.models.some((model) => model.provider === key().trim() && model.upstream_id === id);
  const candidates = createMemo(() => (directory() ?? []).filter((model) => `${model.upstream_id} ${model.defaults?.display_name ?? ""}`.toLocaleLowerCase().includes(query().toLocaleLowerCase())));
  const protocolStatus = () => {
    if (savedProtocol() === protocol()) return "已保存";
    if (savedProtocol()) return "未保存（当前选择尚未保存）";
    if (inferredProtocol === protocol()) return "未保存（根据现有模型协议推定，保存站点后生效）";
    return existingProtocols.size > 1 ? "未保存（现有模型协议不同，请选择新模型默认协议）" : "未保存（新模型默认协议尚未保存）";
  };
  function invalidate() { setDirectory(null); setSelected(new Set<string>()); setNotice(""); }
  const body = createMemo<ProviderWrite>(() => ({
    key: key().trim(), label: label().trim() || key().trim(), base_url: base().trim(),
    models_url: modelsUrl().trim() || null, session_header: sessionHeader().trim() || null, protocol: protocol(),
    ...(apiKey().trim() ? { api_key: apiKey().trim() } : {}),
  }));
  async function discover() {
    if (busy() || disposed) return;
    setBusy(true); setError(""); setNotice(""); invalidate();
    try {
      const models = await props.api.discoverProvider(body());
      if (disposed) return;
      setDirectory(models); setQuery("");
      setNotice(models.length ? "勾选需要上架的模型；保存后可继续编辑各模型参数。" : "站点返回了空目录，保存后仍可手动上架模型。");
    } catch (failure) { if (!disposed) setError(`${errorMessage(failure)}；也可以保存站点后手动上架模型。`); }
    finally { if (!disposed) setBusy(false); }
  }
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy() || disposed) return;
    setBusy(true); setError(""); setNotice("");
    let changed = false;
    try {
      const provider = await props.api.saveProvider(savedKey() || null, body());
      changed = true;
      if (!disposed) { setSavedKey(provider.key); setHasApiKey(provider.has_api_key === true); setSavedProtocol(provider.protocol ?? null); setApiKey(""); }
      for (const item of directory() ?? []) {
        if (disposed) break;
        if (!selected().has(item.upstream_id) || !item.supported || listed(item.upstream_id)) continue;
        await props.api.saveModel(null, emptyModel(provider.key, item.upstream_id, item.defaults, provider));
        setSelected((current) => { const next = new Set(current); next.delete(item.upstream_id); return next; });
      }
      await props.onChanged();
      if (!disposed) await props.onSaved(provider.key);
      changed = false;
    } catch (failure) {
      if (!disposed) setError(`${changed ? "站点已保存，后续上架或列表刷新失败：" : ""}${errorMessage(failure)}`);
    } finally {
      if (changed) {
        try { await props.onChanged(); }
        catch (failure) { if (!disposed) setError(`配置已保存，但列表刷新失败：${errorMessage(failure)}`); }
      }
      if (!disposed) setBusy(false);
    }
  }
  return <section class="admin-card admin-editor" aria-labelledby="provider-editor-title" aria-busy={busy()}>
    <h3 id="provider-editor-title">{savedKey() ? "编辑 API 站点" : "新增 API 站点"}</h3>
    <form onSubmit={(event) => void save(event)}>
      <fieldset class="admin-form-grid" disabled={busy()}>
        <label>站点标识<input name="provider-key" required value={key()} disabled={!!savedKey()} placeholder="my-api" onInput={(event) => { setKey(event.currentTarget.value); invalidate(); }} /></label>
        <label>站点名称<input name="provider-label" required value={label()} placeholder="我的 API 站点" onInput={(event) => setLabel(event.currentTarget.value)} /></label>
        <label>API 地址<input name="provider-base-url" type="url" required value={base()} placeholder="https://api.example.com/v1" onInput={(event) => { setBase(event.currentTarget.value); invalidate(); }} /></label>
        <label>API Key<input name="provider-api-key" type="password" autocomplete="new-password" required={!hasApiKey()} value={apiKey()} placeholder={hasApiKey() ? "已配置，留空保留现有 Key" : "粘贴站点 API Key"} onInput={(event) => { setApiKey(event.currentTarget.value); invalidate(); }} /><small data-testid="provider-key-status">{apiKey().trim() ? "待替换（保存后生效）" : hasApiKey() ? "已配置（留空保留）" : "未配置"}</small></label>
        <label>新模型使用的协议<select name="provider-protocol" value={protocol()} onChange={(event) => { setProtocol(event.currentTarget.value as ProviderProtocol); setNotice(""); }}><option value="chat_completions">Chat Completions</option><option value="responses">Responses</option></select><small data-testid="provider-protocol-status">{protocolStatus()}</small></label>
        <label>模型列表地址（可选）<input name="provider-models-url" type="url" value={modelsUrl()} placeholder="默认使用 API 地址 /models" onInput={(event) => { setModelsUrl(event.currentTarget.value); invalidate(); }} /></label>
        <label>会话标识请求头（可选）<input name="provider-session-header" value={sessionHeader()} placeholder="按站点文档填写" onInput={(event) => { setSessionHeader(event.currentTarget.value); invalidate(); }} /></label>
      </fieldset>
      <p class="admin-note">此协议是新上架模型的默认值；已上架模型的协议单独配置，保存站点不会修改它们。官方站点遵循其模型要求的协议。获取列表使用当前填写的地址和 Key，不保存草稿；Key 保存后不会返回浏览器，已配置不代表连接已验证。</p>
      <div class="admin-toolbar"><button type="button" disabled={busy() || !key().trim() || !base().trim() || !hasApiKey() && !apiKey().trim()} onClick={() => void discover()}>{busy() ? "处理中…" : "获取模型列表"}</button></div>
      <Show when={directory() !== null}>
        <div class="admin-toolbar">
          <label class="admin-grow">搜索获取的模型<input name="provider-model-search" type="search" disabled={busy()} value={query()} onInput={(event) => setQuery(event.currentTarget.value)} /></label>
          <button type="button" disabled={busy()} onClick={() => setSelected((current) => new Set([...current, ...candidates().filter((item) => item.supported && !listed(item.upstream_id)).map((item) => item.upstream_id)]))}>全选搜索结果</button>
          <button type="button" disabled={busy()} onClick={() => setSelected(new Set<string>())}>取消全选</button>
        </div>
        <div class="admin-directory"><For each={candidates()} fallback={<p class="admin-empty">没有匹配的模型。</p>}>{(item) => <div class="admin-directory-row">
          <label class="admin-check"><input type="checkbox" disabled={busy() || !item.supported || listed(item.upstream_id)} checked={listed(item.upstream_id) || selected().has(item.upstream_id)} aria-label={`导入 ${item.upstream_id}`} onChange={(event) => setSelected((current) => { const next = new Set(current); if (event.currentTarget.checked) next.add(item.upstream_id); else next.delete(item.upstream_id); return next; })} /><span class="mono">{item.upstream_id}</span></label>
          <small>{listed(item.upstream_id) ? "已上架" : !item.supported ? "协议暂未支持" : `上下文 ${item.defaults?.context_window?.toLocaleString() ?? "未知"}`}{item.price_source ? " · 含站点价格" : ""}</small>
        </div>}</For></div>
      </Show>
      <Show when={notice()}><p role="status" class="admin-notice">{notice()}</p></Show>
      <Show when={error()}><p role="alert" class="admin-error">{error()}</p></Show>
      <div class="admin-actions"><button type="submit" class="primary" disabled={busy()}>{busy() ? "保存中…" : selected().size ? `保存站点并上架所选（${selected().size}）` : "保存站点"}</button><button type="button" disabled={busy()} onClick={props.onCancel}>取消</button></div>
    </form>
  </section>;
}
