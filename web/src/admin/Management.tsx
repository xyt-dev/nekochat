import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { localImageUrl } from "../localImages";
import { emptyModel, errorMessage, type AdminApi, type Character, type ListedModel, type ModelSpec, type Provider, type ResourcePack, type Settings, type UpstreamModel } from "./api";
import { ModelEditor } from "./ModelEditor";
import { CharacterEditor } from "./CharacterEditor";
import { ChatgptConnection } from "./ChatgptConnection";
import { ProviderEditor } from "./ProviderEditor";

export interface CatalogProps {
  api: AdminApi; providers: Provider[]; models: ListedModel[]; characters: Character[]; packs: ResourcePack[];
  settings: Settings; reload: () => Promise<void>; onCommitted: (message: string, settings?: Settings) => Promise<void>;
}

export function Roles(props: CatalogProps) {
  const [editing, setEditing] = createSignal<{ initial: Character | null } | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [notice, setNotice] = createSignal("");
  const blocked = () => busy() || !!editing();
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const available = (character: Character) => character.enabled && props.models.some((model) => model.id === character.model_id && props.providers.some((provider) => provider.key === model.provider && provider.connected !== false));
  async function mutate(action: () => Promise<Settings | void>, message: string) {
    if (blocked() || disposed) return;
    setBusy(true); setError(""); setNotice("");
    try {
      const committedSettings = await action();
      if (!disposed) setNotice(message);
      await props.onCommitted(message, committedSettings ?? undefined);
    }
    catch (error) { if (!disposed) setError(errorMessage(error)); }
    finally { if (!disposed) setBusy(false); }
  }
  return <section aria-labelledby="roles-title" aria-busy={busy()}>
    <div class="admin-section-heading"><div><h2 id="roles-title">角色卡片</h2><p class="admin-note">模型、生成参数与系统提示词仅管理员可见。公开显示名、背景与授权资源组成完整角色；修改从下一轮生效。</p></div><button type="button" class="primary" disabled={blocked()} onClick={() => { setEditing({ initial: null }); setNotice(""); }}>新增角色</button></div>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show><Show when={notice()}><p class="admin-notice" role="status">{notice()}</p></Show>
    <label class="admin-default">默认角色<select name="default-character" disabled={blocked()} value={props.settings.default_character_id ?? ""} onChange={(event) => { const value = event.currentTarget.value || null; event.currentTarget.value = props.settings.default_character_id ?? ""; void mutate(() => props.api.saveSettings({ ...props.settings, default_character_id: value }), "默认角色已更新"); }}><option value="">无默认角色（使用第一个可用角色）</option><For each={props.characters}>{(character) => <option value={character.id} disabled={!available(character)}>{character.display_name} · {character.name}{available(character) ? "" : "（不可用于聊天）"}</option>}</For></select></label>
    <p class="admin-note">默认角色用于尚无可用选择的账号；用户上次选择的可用角色仍优先保留。</p>
    <Show when={editing()} keyed>{(draft) => <CharacterEditor api={props.api} initial={draft.initial} models={props.models} providers={props.providers} packs={props.packs} onCancel={() => setEditing(null)} onSaved={async () => { if (!disposed) { setBusy(true); setEditing(null); setNotice("角色已保存，从下一轮开始生效"); } try { await props.onCommitted("角色已保存，从下一轮开始生效"); } finally { if (!disposed) setBusy(false); } }} />}</Show>
    <div class="admin-card-list"><For each={props.characters} fallback={<p class="admin-empty">还没有角色，点击「新增角色」配置完整角色卡。</p>}>{(character) => <article class="admin-card" data-character-id={character.id}>
      <div class="admin-section-heading"><div class="admin-identity"><Show when={character.avatar && localImageUrl(character.avatar)} fallback={<span class="admin-avatar admin-avatar-placeholder" aria-hidden="true">{character.display_name.slice(0, 1)}</span>}>{(url) => <img class="admin-avatar" src={url()} alt={`${character.display_name}的头像`} loading="lazy" decoding="async" />}</Show><div><h3>{character.display_name} <Show when={props.settings.default_character_id === character.id}><small class="admin-badge">默认</small></Show></h3><p class="admin-note">内部名称：{character.name} · {character.enabled ? "已启用" : "已停用"} · 排序 {character.sort}</p></div></div><div class="admin-actions">
        <button type="button" disabled={blocked() || !available(character) || props.settings.default_character_id === character.id} onClick={() => void mutate(() => props.api.saveSettings({ ...props.settings, default_character_id: character.id }), "默认角色已更新；用户上次选择的可用角色仍优先")}>设为默认</button>
        <button type="button" disabled={blocked()} onClick={() => { setEditing({ initial: character }); setNotice(""); }}>编辑</button>
        <button type="button" class="danger" disabled={blocked()} onClick={() => { if (confirm(`确定删除角色「${character.display_name}」？此操作不可撤销。历史回复保留原显示名。`)) void mutate(async () => { await props.api.deleteCharacter(character.id); if (editing()?.initial?.id === character.id) setEditing(null); }, "角色已删除"); }}>删除</button>
      </div></div>
      <p class="admin-note">绑定模型：{props.models.find((model) => model.id === character.model_id)?.display_name ?? "未绑定可用模型"} <span class="admin-badge">{available(character) ? "聊天可选" : "聊天不可选"}</span></p>
      <p class="admin-note">搜索服务：{character.search_service_id ? `已绑定 · ${character.search_service_id}（可在编辑中查看可用状态）` : "未绑定"}</p>
      <p class="admin-note">授权资源包：{character.resource_packs.map((id) => props.packs.find((pack) => pack.id === id)?.name ?? id).join("、") || "无"} · 背景：{character.background ? `${Math.round(character.background.opacity * 100)}%` : "无"}</p>
      <details><summary>系统提示词（仅管理员）</summary><pre class="admin-prompt">{character.system_prompt || "（空提示词）"}</pre></details>
    </article>}</For></div>
  </section>;
}

export function Models(props: CatalogProps) {
  const [provider, setProvider] = createSignal(props.providers.find((item) => item.connected !== false)?.key ?? props.providers[0]?.key ?? "");
  const [directory, setDirectory] = createSignal<UpstreamModel[]>([]);
  const [loadedProvider, setLoadedProvider] = createSignal("");
  const [query, setQuery] = createSignal("");
  const [selected, setSelected] = createSignal(new Set<string>());
  const [loading, setLoading] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [notice, setNotice] = createSignal("");
  const [editor, setEditor] = createSignal<{ modelId: string | null; spec: ModelSpec } | null>(null);
  const [providerEditor, setProviderEditor] = createSignal<{ initial: Provider | null } | null>(null);
  let directoryVersion = 0;
  const blocked = () => busy() || !!editor() || !!providerEditor();
  let disposed = false;
  onCleanup(() => { disposed = true; directoryVersion++; });
  const visibleDirectory = createMemo(() => directory().filter((item) => item.upstream_id.toLocaleLowerCase().includes(query().toLocaleLowerCase())));
  const listed = (upstream: string) => props.models.find((model) => model.provider === provider() && model.upstream_id === upstream);
  createEffect(() => {
    if (!props.providers.some((item) => item.key === provider())) {
      setProvider(props.providers.find((item) => item.connected !== false)?.key ?? props.providers[0]?.key ?? "");
      directoryVersion++; setDirectory([]); setLoadedProvider(""); setSelected(new Set<string>()); setLoading(false);
    }
  });
  async function refreshDirectory() {
    if (busy() || disposed) return;
    const key = provider(); const version = ++directoryVersion;
    if (!key) return;
    setLoading(true); setError(""); setSelected(new Set<string>());
    try { const result = await props.api.directory(key); if (version === directoryVersion) { setDirectory(result); setLoadedProvider(key); } }
    catch (error) { if (version === directoryVersion) setError(errorMessage(error)); }
    finally { if (version === directoryVersion) setLoading(false); }
  }
  async function mutate(action: () => Promise<unknown>, message: string) {
    if (blocked() || disposed) return;
    setBusy(true); setError(""); setNotice("");
    try {
      await action();
      if (!disposed) setNotice(message);
      await props.onCommitted(message);
    }
    catch (error) {
      if (!disposed) setError(errorMessage(error));
      try { await props.reload(); }
      catch (failure) { if (!disposed) setError(`${errorMessage(error)}；列表刷新失败：${errorMessage(failure)}`); }
    }
    finally { if (!disposed) setBusy(false); }
  }
  async function listSelected() {
    await mutate(async () => {
      for (const item of directory().filter((row) => selected().has(row.upstream_id) && row.supported && !listed(row.upstream_id))) {
        if (disposed) return;
        await props.api.saveModel(null, emptyModel(provider(), item.upstream_id, item.defaults, props.providers.find((site) => site.key === provider())));
        setSelected((current) => { const next = new Set(current); next.delete(item.upstream_id); return next; });
      }
    }, "勾选模型已上架（未核实的价格与参数保持未知，可继续编辑）");
  }
  onMount(() => void refreshDirectory());
  return <section aria-labelledby="models-title" aria-busy={busy()}>
    <ChatgptConnection api={props.api} onChanged={props.reload} />
    <section aria-labelledby="providers-title">
      <div class="admin-section-heading"><div><h2 id="providers-title">API 站点</h2><p class="admin-note">添加任意兼容 API 站点，保存后即时生效。官方 ChatGPT 订阅使用上方账号授权。</p></div><button type="button" class="primary" disabled={blocked()} onClick={() => { setProviderEditor({ initial: null }); setNotice(""); }}>新增 API 站点</button></div>
      <Show when={providerEditor()} keyed>{(draft) => <ProviderEditor api={props.api} initial={draft.initial} models={props.models} onChanged={props.reload} onSaved={async (key) => { directoryVersion++; setLoading(false); setProvider(key); setProviderEditor(null); setDirectory([]); setLoadedProvider(""); setSelected(new Set<string>()); setNotice("API 站点与所选模型已保存，即时生效"); }} onCancel={() => setProviderEditor(null)} />}</Show>
      <div class="admin-card-list"><For each={props.providers.filter((item) => item.key !== "chatgpt-subscription")}>{(item) => <article class="admin-card"><div class="admin-section-heading"><div><h3>{item.label}</h3><p class="admin-note"><span class="mono">{item.key}</span> · {item.base_url} · {item.managed ? "管理端配置" : "服务器配置"}</p></div><div class="admin-actions"><button type="button" disabled={blocked()} onClick={() => setProviderEditor({ initial: item })}>编辑站点</button><button type="button" class="danger" disabled={blocked()} onClick={() => { if (confirm(`确定删除 API 站点「${item.label}」？绑定它的模型与角色将暂不可用，历史记录保留。`)) void mutate(async () => { await props.api.deleteProvider(item.key); directoryVersion++; setDirectory([]); setLoadedProvider(""); setSelected(new Set<string>()); setLoading(false); }, "API 站点已删除，历史记录保留"); }}>删除站点</button></div></div></article>}</For></div>
    </section>
    <div class="admin-section-heading"><div><h2 id="models-title">模型上架</h2><p class="admin-note">获取站点模型列表并选择上架，或手动填写上游模型 ID。</p></div><button type="button" class="primary" disabled={!props.providers.some((item) => item.connected !== false) || blocked()} onClick={() => setEditor({ modelId: null, spec: emptyModel(provider(), "", null, props.providers.find((site) => site.key === provider())) })}>手动上架</button></div>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show><Show when={notice()}><p class="admin-notice" role="status">{notice()}</p></Show>
    <Show when={editor()} keyed>{(draft) => <ModelEditor api={props.api} providers={props.providers} modelId={draft.modelId} initial={draft.spec} onSaved={async () => { if (!disposed) { setBusy(true); setEditor(null); setNotice("模型已保存"); } await props.onCommitted("模型已保存"); if (!disposed) setBusy(false); }} onCancel={() => setEditor(null)} />}</Show>
    <section class="admin-card" aria-labelledby="directory-title"><h3 id="directory-title">提供商模型目录</h3>
      <div class="admin-toolbar"><label>提供商<select name="directory-provider" disabled={blocked()} value={provider()} onChange={(event) => { setProvider(event.currentTarget.value); setDirectory([]); setSelected(new Set<string>()); setLoadedProvider(""); void refreshDirectory(); }}><For each={props.providers}>{(item) => <option value={item.key} selected={item.key === provider()}>{item.label}</option>}</For></select></label><button type="button" disabled={loading() || blocked() || !provider()} onClick={() => void refreshDirectory()}>{loading() ? "获取中…" : loadedProvider() === provider() ? "刷新模型目录" : "获取模型目录"}</button><label class="admin-grow">搜索模型<input name="directory-search" type="search" value={query()} onInput={(event) => setQuery(event.currentTarget.value)} placeholder="模型 ID" /></label><button type="button" class="primary" disabled={blocked() || selected().size === 0 || loading()} onClick={() => void listSelected()}>上架所选（{selected().size}）</button></div>
      <div class="admin-toolbar"><button type="button" disabled={blocked() || loading()} onClick={() => setSelected((current) => new Set([...current, ...visibleDirectory().filter((item) => item.supported && !listed(item.upstream_id)).map((item) => item.upstream_id)]))}>全选搜索结果</button><button type="button" disabled={blocked() || loading()} onClick={() => setSelected(new Set<string>())}>取消全选</button></div>
      <p class="admin-note">使用站点公布的能力和价格；未公布的价格与上下文保持未知，可继续编辑。新上架模型使用站点已保存的默认协议；官方站点遵循其模型要求的协议。</p>
      <Show when={loading()}><p role="status">正在从提供商获取真实目录…</p></Show>
      <Show when={!loading() && loadedProvider() === provider()}><div class="admin-directory"><For each={visibleDirectory()} fallback={<p class="admin-empty">没有匹配模型。</p>}>{(item) => <div class="admin-directory-row"><label class="admin-check"><input type="checkbox" aria-label={`选择上架 ${item.upstream_id}`} disabled={!item.supported || !!listed(item.upstream_id) || blocked()} checked={!!listed(item.upstream_id) || selected().has(item.upstream_id)} onChange={(event) => setSelected((current) => { const next = new Set(current); if (event.currentTarget.checked) next.add(item.upstream_id); else next.delete(item.upstream_id); return next; })} /><span class="mono">{item.upstream_id}</span></label><small>{listed(item.upstream_id) ? "已上架" : !item.supported ? "协议尚未支持" : item.defaults ? "有已核实默认值" : "参数未知 · 可手动编辑"}</small><Show when={item.supported && !listed(item.upstream_id)}><button type="button" disabled={blocked()} onClick={() => setEditor({ modelId: null, spec: emptyModel(provider(), item.upstream_id, item.defaults, props.providers.find((site) => site.key === provider())) })}>配置后上架</button></Show></div>}</For></div></Show>
      <Show when={!props.providers.some((item) => item.key !== "chatgpt-subscription")}><p class="admin-empty">点击上方「新增 API 站点」填写地址与 Key，无需重启。</p></Show>
    </section>
    <h3>已上架模型（{props.models.length}）</h3><div class="admin-card-list"><For each={props.models} fallback={<p class="admin-empty">暂无上架模型。</p>}>{(model) => <article class="admin-card"><div class="admin-section-heading"><div><h3>{model.display_name}</h3><p class="admin-note">{props.providers.find((item) => item.key === model.provider)?.label ?? model.provider} · <span class="mono">{model.upstream_id}</span> · 排序 {model.sort}</p></div><div class="admin-actions"><button type="button" disabled={blocked()} onClick={() => setEditor({ modelId: model.id, spec: model })}>编辑</button><button type="button" class="danger" disabled={blocked()} onClick={() => { if (confirm(`确定下架「${model.display_name}」？绑定此模型的角色将无法再聊天，历史统计保留。`)) void mutate(async () => { await props.api.deleteModel(model.id); if (editor()?.modelId === model.id) setEditor(null); }, "模型已下架"); }}>下架</button></div></div><p class="admin-note">{model.protocol === "chat_completions" ? "Chat Completions" : model.protocol === "chatgpt_responses" ? "ChatGPT Responses" : "Responses"} · 思考：{model.reasoning.style === "none" ? "无" : model.reasoning.levels.join(" / ")} · 图片：{model.vision ? "支持" : "不支持"} · 上下文：{model.context_window?.toLocaleString() ?? "未知"}</p></article>}</For></div>
  </section>;
}
