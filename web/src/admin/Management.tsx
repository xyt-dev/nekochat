import { createMemo, createSignal, For, onMount, Show } from "solid-js";
import { emptyModel, errorMessage, type AdminApi, type Character, type ListedModel, type ModelSpec, type Provider, type ResourcePack, type Settings, type UpstreamModel } from "./api";
import { ModelEditor } from "./ModelEditor";
import { CharacterEditor } from "./CharacterEditor";

export interface CatalogProps {
  api: AdminApi; providers: Provider[]; models: ListedModel[]; characters: Character[]; packs: ResourcePack[];
  settings: Settings; reload: () => Promise<void>;
}

export function Roles(props: CatalogProps) {
  const [editing, setEditing] = createSignal<{ initial: Character | null } | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [notice, setNotice] = createSignal("");
  const available = (character: Character) => character.enabled && props.models.some((model) => model.id === character.model_id);
  async function mutate(action: () => Promise<unknown>, message: string) {
    setBusy(true); setError(""); setNotice("");
    try { await action(); await props.reload(); setNotice(message); }
    catch (error) { setError(errorMessage(error)); }
    finally { setBusy(false); }
  }
  return <section aria-labelledby="roles-title">
    <div class="admin-section-heading"><div><h2 id="roles-title">角色卡片</h2><p class="admin-note">模型、生成参数与系统提示词仅管理员可见。公开显示名、背景与授权资源组成完整角色；修改从下一轮生效。</p></div><button type="button" class="primary" disabled={busy()} onClick={() => { setEditing({ initial: null }); setNotice(""); }}>新增角色</button></div>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show><Show when={notice()}><p role="status">{notice()}</p></Show>
    <label class="admin-default">默认角色<select name="default-character" disabled={busy()} value={props.settings.default_character_id ?? ""} onChange={(event) => { const value = event.currentTarget.value || null; void mutate(() => props.api.saveSettings({ ...props.settings, default_character_id: value }), "默认角色已更新"); }}><option value="">无默认角色（使用第一个可用角色）</option><For each={props.characters}>{(character) => <option value={character.id} disabled={!available(character)}>{character.display_name} · {character.name}{available(character) ? "" : "（不可用于聊天）"}</option>}</For></select></label>
    <Show when={editing()} keyed>{(draft) => <CharacterEditor api={props.api} initial={draft.initial} models={props.models} providers={props.providers} packs={props.packs} onCancel={() => setEditing(null)} onSaved={async () => { await props.reload(); setEditing(null); setNotice("角色已保存，从下一轮开始生效"); }} />}</Show>
    <div class="admin-card-list"><For each={props.characters} fallback={<p class="admin-empty">还没有角色，点击「新增角色」配置完整角色卡。</p>}>{(character) => <article class="admin-card" data-character-id={character.id}>
      <div class="admin-section-heading"><div><h3>{character.display_name} <Show when={props.settings.default_character_id === character.id}><small class="admin-badge">默认</small></Show></h3><p class="admin-note">内部名称：{character.name} · {character.enabled ? "已启用" : "已停用"} · 排序 {character.sort}</p></div><div class="admin-actions">
        <button type="button" disabled={busy()} onClick={() => { setEditing({ initial: character }); setNotice(""); }}>编辑</button>
        <button type="button" class="danger" disabled={busy()} onClick={() => { if (confirm(`确定删除角色「${character.display_name}」？此操作不可撤销。历史回复保留原显示名。`)) void mutate(async () => { await props.api.deleteCharacter(character.id); if (editing()?.initial?.id === character.id) setEditing(null); }, "角色已删除"); }}>删除</button>
      </div></div>
      <p class="admin-note">绑定模型：{props.models.find((model) => model.id === character.model_id)?.display_name ?? "未绑定可用模型"} · {available(character) ? "聊天可选" : "聊天不可选"}</p>
      <p class="admin-note">授权资源包：{character.resource_packs.map((id) => props.packs.find((pack) => pack.id === id)?.name ?? id).join("、") || "无"} · 背景：{character.background ? `${Math.round(character.background.opacity * 100)}%` : "无"}</p>
      <details><summary>系统提示词（仅管理员）</summary><pre class="admin-prompt">{character.system_prompt || "（空提示词）"}</pre></details>
    </article>}</For></div>
  </section>;
}

export function Models(props: CatalogProps) {
  const [provider, setProvider] = createSignal(props.providers[0]?.key ?? "");
  const [directory, setDirectory] = createSignal<UpstreamModel[]>([]);
  const [loadedProvider, setLoadedProvider] = createSignal("");
  const [query, setQuery] = createSignal("");
  const [selected, setSelected] = createSignal(new Set<string>());
  const [loading, setLoading] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [notice, setNotice] = createSignal("");
  const [editor, setEditor] = createSignal<{ modelId: string | null; spec: ModelSpec } | null>(null);
  let directoryVersion = 0;
  const visibleDirectory = createMemo(() => directory().filter((item) => item.upstream_id.toLocaleLowerCase().includes(query().toLocaleLowerCase())));
  const listed = (upstream: string) => props.models.find((model) => model.provider === provider() && model.upstream_id === upstream);
  async function refreshDirectory() {
    const key = provider(); const version = ++directoryVersion;
    if (!key) return;
    setLoading(true); setError(""); setSelected(new Set<string>());
    try { const result = await props.api.directory(key); if (version === directoryVersion) { setDirectory(result); setLoadedProvider(key); } }
    catch (error) { if (version === directoryVersion) setError(errorMessage(error)); }
    finally { if (version === directoryVersion) setLoading(false); }
  }
  async function mutate(action: () => Promise<unknown>, message: string) {
    setBusy(true); setError(""); setNotice("");
    try { await action(); await props.reload(); setNotice(message); }
    catch (error) { setError(errorMessage(error)); await props.reload().catch(() => {}); }
    finally { setBusy(false); }
  }
  async function listSelected() {
    await mutate(async () => {
      for (const item of directory().filter((row) => selected().has(row.upstream_id) && row.supported && !listed(row.upstream_id))) {
        await props.api.saveModel(null, emptyModel(provider(), item.upstream_id, item.defaults));
        setSelected((current) => { const next = new Set(current); next.delete(item.upstream_id); return next; });
      }
    }, "勾选模型已上架（未核实的价格与参数保持未知，可继续编辑）");
  }
  onMount(() => void refreshDirectory());
  return <section aria-labelledby="models-title">
    <div class="admin-section-heading"><div><h2 id="models-title">模型上架</h2><p class="admin-note">API key 仅在服务器配置中管理，此处不展示或修改凭证。</p></div><button type="button" class="primary" disabled={!props.providers.length || busy()} onClick={() => setEditor({ modelId: null, spec: emptyModel(provider()) })}>手动上架</button></div>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show><Show when={notice()}><p role="status">{notice()}</p></Show>
    <label class="admin-default">默认模型<select name="default-model" disabled={busy()} value={props.settings.default_model_id ?? ""} onChange={(event) => { const value = event.currentTarget.value || null; void mutate(() => props.api.saveSettings({ ...props.settings, default_model_id: value }), "默认模型已更新"); }}><option value="">无默认模型</option><For each={props.models}>{(model) => <option value={model.id}>{props.providers.find((item) => item.key === model.provider)?.label ?? model.provider} · {model.display_name}</option>}</For></select></label>
    <Show when={editor()} keyed>{(draft) => <ModelEditor api={props.api} providers={props.providers} modelId={draft.modelId} initial={draft.spec} onSaved={async () => { await props.reload(); setEditor(null); setNotice("模型已保存"); }} onCancel={() => setEditor(null)} />}</Show>
    <section class="admin-card" aria-labelledby="directory-title"><h3 id="directory-title">提供商模型目录</h3>
      <div class="admin-toolbar"><label>提供商<select name="directory-provider" disabled={busy()} value={provider()} onChange={(event) => { setProvider(event.currentTarget.value); setDirectory([]); setSelected(new Set<string>()); setLoadedProvider(""); void refreshDirectory(); }}><For each={props.providers}>{(item) => <option value={item.key}>{item.label}</option>}</For></select></label><button type="button" disabled={loading() || busy() || !provider()} onClick={() => void refreshDirectory()}>{loading() ? "获取中…" : loadedProvider() === provider() ? "刷新模型目录" : "获取模型目录"}</button><label class="admin-grow">搜索模型<input name="directory-search" type="search" value={query()} onInput={(event) => setQuery(event.currentTarget.value)} placeholder="模型 ID" /></label><button type="button" class="primary" disabled={busy() || selected().size === 0 || loading()} onClick={() => void listSelected()}>上架所选（{selected().size}）</button></div>
      <p class="admin-note">已核实模型自动预填；未知模型保留空价格与上下文。Responses / Messages / Gemini 尚未接入，不能勾选上架。</p>
      <Show when={loading()}><p role="status">正在从提供商获取真实目录…</p></Show>
      <Show when={!loading() && loadedProvider() === provider()}><div class="admin-directory"><For each={visibleDirectory()} fallback={<p class="admin-empty">没有匹配模型。</p>}>{(item) => <div class="admin-directory-row"><label class="admin-check"><input type="checkbox" aria-label={`选择上架 ${item.upstream_id}`} disabled={!item.supported || !!listed(item.upstream_id) || busy()} checked={!!listed(item.upstream_id) || selected().has(item.upstream_id)} onChange={(event) => setSelected((current) => { const next = new Set(current); if (event.currentTarget.checked) next.add(item.upstream_id); else next.delete(item.upstream_id); return next; })} /><span class="mono">{item.upstream_id}</span></label><small>{listed(item.upstream_id) ? "已上架" : !item.supported ? "协议尚未支持" : item.defaults ? "有已核实默认值" : "参数未知 · 可手动编辑"}</small><Show when={item.supported && !listed(item.upstream_id)}><button type="button" disabled={busy()} onClick={() => setEditor({ modelId: null, spec: emptyModel(provider(), item.upstream_id, item.defaults) })}>配置后上架</button></Show></div>}</For></div></Show>
      <Show when={!props.providers.length}><p class="admin-empty">尚未配置提供商。请在服务器配置中添加 providers 后重启。</p></Show>
    </section>
    <h3>已上架模型（{props.models.length}）</h3><div class="admin-card-list"><For each={props.models} fallback={<p class="admin-empty">暂无上架模型。</p>}>{(model) => <article class="admin-card"><div class="admin-section-heading"><div><h3>{model.display_name} <Show when={props.settings.default_model_id === model.id}><small class="admin-badge">默认</small></Show></h3><p class="admin-note">{props.providers.find((item) => item.key === model.provider)?.label ?? model.provider} · <span class="mono">{model.upstream_id}</span> · 排序 {model.sort}</p></div><div class="admin-actions"><button type="button" disabled={busy()} onClick={() => setEditor({ modelId: model.id, spec: model })}>编辑</button><button type="button" class="danger" disabled={busy()} onClick={() => { if (confirm(`确定下架「${model.display_name}」？聊天页将无法再使用此模型，历史统计保留。${props.settings.default_model_id === model.id ? "默认模型将被清除。" : ""}`)) void mutate(async () => { await props.api.deleteModel(model.id); if (editor()?.modelId === model.id) setEditor(null); }, "模型已下架"); }}>下架</button></div></div><p class="admin-note">Chat Completions · 思考：{model.reasoning.style === "none" ? "无" : model.reasoning.levels.join(" / ")} · 图片：{model.vision ? "支持" : "不支持"} · 上下文：{model.context_window?.toLocaleString() ?? "未知"}</p></article>}</For></div>
  </section>;
}
