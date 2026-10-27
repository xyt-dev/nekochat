import { createMemo, createSignal, For, Show } from "solid-js";
import { localImageUrl } from "../localImages";
import { errorMessage, type AdminApi, type Character, type CharacterParameters, type CharacterRecipe, type ListedModel, type Provider, type ResourcePack } from "./api";
import { PackPreview } from "./ResourcePacks";

const NUMBER_FIELDS = [
  { key: "temperature", label: "Temperature", min: 0, max: 2, step: "any" },
  { key: "top_p", label: "Top P", min: 0, max: 1, step: "any" },
  { key: "max_output_tokens", label: "最大输出 token", min: 1, max: Number.MAX_SAFE_INTEGER, step: "1" },
  { key: "seed", label: "Seed", min: Number.MIN_SAFE_INTEGER, max: Number.MAX_SAFE_INTEGER, step: "1" },
  { key: "frequency_penalty", label: "Frequency penalty", min: -2, max: 2, step: "any" },
  { key: "presence_penalty", label: "Presence penalty", min: -2, max: 2, step: "any" },
] as const;
type NumberKey = typeof NUMBER_FIELDS[number]["key"];

export function CharacterEditor(props: {
  api: AdminApi; initial: Character | null; models: ListedModel[]; providers: Provider[]; packs: ResourcePack[];
  onSaved: () => Promise<void>; onCancel: () => void;
}) {
  const initial = props.initial;
  const [name, setName] = createSignal(initial?.name ?? "");
  const [displayName, setDisplayName] = createSignal(initial?.display_name ?? "");
  const [prompt, setPrompt] = createSignal(initial?.system_prompt ?? "");
  const [modelId, setModelId] = createSignal(initial?.model_id ?? "");
  const [enabled, setEnabled] = createSignal(initial?.enabled ?? true);
  const [sort, setSort] = createSignal(String(initial?.sort ?? 0));
  const [effort, setEffort] = createSignal(initial?.parameters.effort ?? "");
  const [numbers, setNumbers] = createSignal<Record<NumberKey, string>>(Object.fromEntries(NUMBER_FIELDS.map(({ key }) => [key, initial?.parameters[key] == null ? "" : String(initial.parameters[key])])) as Record<NumberKey, string>);
  const [stop, setStop] = createSignal(initial?.parameters.stop?.join("\n") ?? "");
  const [background, setBackground] = createSignal(initial?.background?.url ?? "");
  const [opacity, setOpacity] = createSignal(initial?.background?.opacity ?? 0.15);
  const [packs, setPacks] = createSignal(initial?.resource_packs ?? []);
  const [identity, setIdentity] = createSignal<CharacterRecipe["sticker_identity"]>(initial?.sticker_identity ?? "neutral");
  const [frequency, setFrequency] = createSignal<CharacterRecipe["sticker_frequency"]>(initial?.sticker_frequency ?? "often");
  const [busy, setBusy] = createSignal(false);
  const [uploading, setUploading] = createSignal(false);
  const [error, setError] = createSignal("");
  const model = createMemo(() => props.models.find((item) => item.id === modelId()));
  const nativeNumberFields = createMemo(() => NUMBER_FIELDS.filter((field) =>
    model()?.protocol !== "chatgpt_responses" && (model()?.protocol !== "responses" || !["seed", "frequency_penalty", "presence_penalty"].includes(field.key))));
  const missingPacks = createMemo(() => packs().filter((id) => !props.packs.some((pack) => pack.id === id)));
  function bindModel(id: string) {
    setModelId(id);
    if (!props.models.find((item) => item.id === id)?.reasoning.levels.includes(effort())) setEffort("");
    const protocol = props.models.find((item) => item.id === id)?.protocol;
    if (protocol === "chatgpt_responses") {
      setNumbers(Object.fromEntries(NUMBER_FIELDS.map(({ key }) => [key, ""])) as Record<NumberKey, string>); setStop("");
    } else if (protocol === "responses") {
      setNumbers((current) => ({ ...current, seed: "", frequency_penalty: "", presence_penalty: "" })); setStop("");
    }
  }
  async function upload(file: File) {
    setError(""); setUploading(true);
    try {
      if (!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(file.type)) throw new Error("仅支持 PNG、JPEG、WebP、GIF 图片");
      if (file.size > 5 * 1024 * 1024) throw new Error("背景图片不能超过 5 MiB");
      const result = await props.api.uploadBackground(file);
      const url = localImageUrl(result.url);
      if (!url) throw new Error("服务器返回了无效的本地背景地址");
      setBackground(url);
    } catch (error) { setError(errorMessage(error)); }
    finally { setUploading(false); }
  }
  async function save(event: SubmitEvent) {
    event.preventDefault(); setError(""); setBusy(true);
    try {
      if (!name().trim() || !displayName().trim()) throw new Error("内部名称与公开显示名不能为空");
      if (modelId() && !model()) throw new Error("请选择实际已上架的模型，或清除模型绑定");
      const order = Number(sort());
      if (!sort().trim() || !Number.isSafeInteger(order)) throw new Error("排序必须为整数");
      const parameters: CharacterParameters = {};
      if (effort()) {
        if (!model()?.reasoning.levels.includes(effort())) throw new Error("请选择绑定模型支持的思考档位");
        parameters.effort = effort();
      }
      for (const field of NUMBER_FIELDS) {
        const raw = numbers()[field.key].trim();
        if (!raw) continue;
        const value = Number(raw);
        if (!Number.isFinite(value) || value < field.min || value > field.max || field.step === "1" && !Number.isSafeInteger(value)) throw new Error(`${field.label} 数值无效`);
        parameters[field.key] = value;
      }
      if (stop()) parameters.stop = stop().split("\n").filter((value) => value.length > 0);
      const url = background() ? localImageUrl(background()) : null;
      if (background() && !url) throw new Error("只能使用本站已上传背景或本地资源素材");
      if (!Number.isFinite(opacity()) || opacity() < 0 || opacity() > 1) throw new Error("背景透明度必须在 0 到 1 之间");
      if (missingPacks().length) throw new Error("请移除未加载的资源包后保存");
      await props.api.saveCharacter(initial?.id ?? null, {
        name: name().trim(), display_name: displayName().trim(), system_prompt: prompt(), model_id: modelId() || null,
        parameters, enabled: enabled(), sort: order, background: url ? { url, opacity: opacity() } : null,
        resource_packs: packs(), sticker_identity: identity(), sticker_frequency: frequency(),
      });
      await props.onSaved();
    } catch (error) { setError(errorMessage(error)); }
    finally { setBusy(false); }
  }
  return <section class="admin-card admin-editor" aria-labelledby="character-editor-title">
    <h3 id="character-editor-title">{initial ? "编辑角色" : "新增角色"}</h3>
    <form onSubmit={(event) => void save(event)}>
      <fieldset disabled={busy() || uploading()}>
        <div class="admin-form-grid">
          <label>内部名称<input name="character-name" required value={name()} onInput={(event) => setName(event.currentTarget.value)} /></label>
          <label>公开显示名<input name="character-display-name" required value={displayName()} onInput={(event) => setDisplayName(event.currentTarget.value)} /></label>
          <label>实际绑定模型<select name="character-model" value={modelId()} onChange={(event) => bindModel(event.currentTarget.value)}>
            <option value="">未绑定（不在聊天列表显示）</option>
            <Show when={modelId() && !model()}><option value={modelId()} disabled>原绑定模型已下架</option></Show>
            <For each={props.models}>{(item) => <option value={item.id} disabled={props.providers.find((provider) => provider.key === item.provider)?.connected === false}>{props.providers.find((provider) => provider.key === item.provider)?.label ?? item.provider} · {item.display_name} · {item.upstream_id}</option>}</For>
          </select></label>
          <label>排序<input name="character-sort" type="number" step="1" required value={sort()} onInput={(event) => setSort(event.currentTarget.value)} /></label>
          <label class="admin-check"><input name="character-enabled" type="checkbox" checked={enabled()} onChange={(event) => setEnabled(event.currentTarget.checked)} />启用角色</label>
        </div>
        <Show when={!model()}><p class="admin-note">未绑定可用模型的角色不会出现在聊天端，请先上架模型并绑定。</p></Show>
        <label>系统提示词<textarea name="system-prompt" rows="9" value={prompt()} onInput={(event) => setPrompt(event.currentTarget.value)} /></label>
        <h4>服务端生成参数（不向聊天端公开）</h4><p class="admin-note">留空使用服务端模型默认值。参数直接用于下一轮生成，不接受任意 JSON、工具或文件执行配置。</p>
        <div class="admin-form-grid">
          <label>思考档位<select name="character-effort" value={effort()} disabled={!model()?.reasoning.levels.length} onChange={(event) => setEffort(event.currentTarget.value)}><option value="">模型默认{model()?.reasoning.default ? `（${model()!.reasoning.default}）` : ""}</option><For each={model()?.reasoning.levels ?? []}>{(level) => <option value={level}>{level}</option>}</For></select></label>
          <For each={nativeNumberFields()}>{(field) => <label>{field.label}<input name={`character-${field.key}`} type="number" min={field.min} max={field.max} step={field.step} placeholder="服务端默认" value={numbers()[field.key]} onInput={(event) => { const value = event.currentTarget.value; setNumbers((current) => ({ ...current, [field.key]: value })); }} /></label>}</For>
        </div>
        <Show when={model()?.protocol === "chat_completions"} fallback={<p class="admin-note">{model()?.protocol === "chatgpt_responses" ? "ChatGPT 订阅仅接受思考档位，不支持采样、最大输出、seed、penalty、stop；切换绑定时清空这些参数。" : "Responses 不支持 seed、penalty、stop；切换绑定时清空这些参数。"}</p>}><label>Stop（每行一个停止字符串）<textarea name="character-stop" rows="3" value={stop()} onInput={(event) => setStop(event.currentTarget.value)} /></label></Show>
        <h4>可选背景</h4><p class="admin-note">默认为无背景。仅可上传本站图片或选择下方本地素材；单层透明装饰，无滤镜或动画。</p>
        <label>上传背景（≤ 5 MiB）<input name="character-background" type="file" accept="image/png,image/jpeg,image/webp,image/gif" onChange={(event) => { const file = event.currentTarget.files?.[0]; event.currentTarget.value = ""; if (file) void upload(file); }} /></label>
        <Show when={localImageUrl(background())}>{(url) => <div class="admin-background-preview"><img src={url()} alt="角色背景预览" loading="lazy" decoding="async" style={{ opacity: opacity() }} /><button type="button" onClick={() => { setBackground(""); setOpacity(0.15); }}>清除背景</button></div>}</Show>
        <label>背景透明度：{Math.round(opacity() * 100)}%<input name="character-background-opacity" type="range" min="0" max="1" step="0.01" value={opacity()} disabled={!background()} onInput={(event) => setOpacity(Number(event.currentTarget.value))} /></label>
        <h4>授权资源包</h4><p class="admin-note">只把勾选包的静态指引与目录提供给此角色。预览素材不下载到外站，不运行 SKILL 中的工具。</p>
        <For each={missingPacks()}>{(id) => <p class="admin-error">资源包 {id} 未加载 <button type="button" onClick={() => setPacks((current) => current.filter((value) => value !== id))}>移除授权</button></p>}</For>
        <For each={props.packs} fallback={<p class="admin-note">没有已加载的本地资源包。</p>}>{(pack) => <div class="admin-role-pack">
          <label class="admin-check"><input name="character-resource-pack" type="checkbox" value={pack.id} checked={packs().includes(pack.id)} onChange={(event) => { const checked = event.currentTarget.checked; setPacks((current) => checked ? [...current, pack.id] : current.filter((id) => id !== pack.id)); }} />{pack.name}（{pack.sticker_count} 张）</label><p class="admin-note">{pack.description}</p><PackPreview pack={pack} onChooseBackground={setBackground} />
        </div>}</For>
        <div class="admin-form-grid">
          <label>表情身份<select name="character-sticker-identity" value={identity()} onChange={(event) => setIdentity(event.currentTarget.value as CharacterRecipe["sticker_identity"])}><option value="neutral">中立引用（neutral）</option><option value="self">角色本人（self）</option></select></label>
          <label>表情频率<select name="character-sticker-frequency" value={frequency()} onChange={(event) => setFrequency(event.currentTarget.value as CharacterRecipe["sticker_frequency"])}><option value="rare">偶尔（rare）</option><option value="normal">适中（normal）</option><option value="often">经常（often）</option></select></label>
        </div>
      </fieldset>
      <Show when={uploading()}><p role="status">正在上传到本站…</p></Show><Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show>
      <div class="admin-actions"><button type="submit" class="primary" disabled={busy() || uploading()}>{busy() ? "保存中…" : "保存角色"}</button><button type="button" disabled={busy() || uploading()} onClick={props.onCancel}>取消</button></div>
    </form>
  </section>;
}
