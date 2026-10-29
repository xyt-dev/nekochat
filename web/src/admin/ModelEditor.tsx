import { createSignal, For, onCleanup, Show } from "solid-js";
import { errorMessage, type AdminApi, type ModelSpec, type Provider } from "./api";

export function ModelEditor(props: {
  api: AdminApi; providers: Provider[]; initial: ModelSpec; modelId: string | null;
  onSaved: () => Promise<void>; onCancel: () => void;
}) {
  const initial = props.initial;
  const [provider, setProvider] = createSignal(initial.provider);
  const [upstream, setUpstream] = createSignal(initial.upstream_id);
  const [name, setName] = createSignal(initial.display_name);
  const [sort, setSort] = createSignal(String(initial.sort));
  const [protocol, setProtocol] = createSignal(initial.protocol);
  const [style, setStyle] = createSignal(initial.reasoning.style);
  const [levels, setLevels] = createSignal(initial.reasoning.levels.join(", "));
  const [defaultLevel, setDefaultLevel] = createSignal(initial.reasoning.default ?? "");
  const [vision, setVision] = createSignal(initial.vision);
  const [context, setContext] = createSignal(initial.context_window?.toString() ?? "");
  const [cached, setCached] = createSignal(initial.price.cached_in?.toString() ?? "");
  const [uncached, setUncached] = createSignal(initial.price.uncached_in?.toString() ?? "");
  const [output, setOutput] = createSignal(initial.price.output?.toString() ?? "");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const levelList = () => levels().split(/[,，\n]/).map((level) => level.trim()).filter(Boolean);
  const number = (value: string, label: string, integer = false): number | null => {
    if (!value.trim()) return null;
    const parsed = Number(value);
    if (!Number.isFinite(parsed) || parsed < 0 || integer && (!Number.isSafeInteger(parsed) || parsed === 0)) throw new Error(`${label}必须是${integer ? "正整数" : "非负数"}`);
    return parsed;
  };
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy() || disposed) return;
    setBusy(true); setError("");
    try {
      const parsedSort = Number(sort());
      if (!Number.isSafeInteger(parsedSort)) throw new Error("排序必须是整数");
      const reasoningLevels = style() === "none" ? [] : levelList();
      if (style() !== "none" && !reasoningLevels.length) throw new Error("请输入思考档位");
      if (new Set(reasoningLevels).size !== reasoningLevels.length) throw new Error("思考档位不能重复");
      if (style() !== "none" && defaultLevel() && !reasoningLevels.includes(defaultLevel())) throw new Error("请从当前档位选择默认思考档位");
      await props.api.saveModel(props.modelId, {
        provider: provider(), upstream_id: upstream().trim(), display_name: name().trim(), sort: parsedSort,
        protocol: protocol(), reasoning: { style: style(), levels: reasoningLevels, default: style() === "none" ? null : defaultLevel() || null },
        vision: vision(), context_window: number(context(), "上下文窗口", true),
        price: { cached_in: number(cached(), "缓存输入价格"), uncached_in: number(uncached(), "无缓存输入价格"), output: number(output(), "输出价格") },
      });
      await props.onSaved();
    } catch (error) { setError(errorMessage(error)); } finally { setBusy(false); }
  }
  return <section class="admin-card admin-editor" aria-labelledby="model-editor-title" aria-busy={busy()}>
    <h3 id="model-editor-title">{props.modelId ? "编辑模型" : "上架模型"}</h3>
    <form onSubmit={(event) => void save(event)}>
      <fieldset disabled={busy()} class="admin-form-grid">
        <h4 class="admin-field-heading">模型标识与上架顺序</h4>
        <label>提供商<select name="model-provider" value={provider()} onChange={(event) => { const key = event.currentTarget.value; setProvider(key); if (key === "chatgpt-subscription") { setProtocol("chatgpt_responses"); setCached(""); setUncached(""); setOutput(""); } }} required><For each={props.providers}>{(item) => <option value={item.key} disabled={item.connected === false}>{item.label} · {item.key}{item.connected === false ? "（未连接）" : ""}</option>}</For></select></label>
        <label>上游模型 ID<input name="upstream-id" required value={upstream()} onInput={(event) => setUpstream(event.currentTarget.value)} /></label>
        <label>显示名<input name="display-name" required value={name()} onInput={(event) => setName(event.currentTarget.value)} /></label>
        <label>排序（小数值在前）<input name="model-sort" type="number" step="1" required value={sort()} onInput={(event) => setSort(event.currentTarget.value)} /></label>
        <h4 class="admin-field-heading">协议与生成能力</h4>
        <label>协议<select name="model-protocol" value={protocol()} disabled={provider() === "chatgpt-subscription"} onChange={(event) => {
          const next = event.currentTarget.value as ModelSpec["protocol"];
          setProtocol(next);
          if (next !== "chat_completions" && style() === "deepseek") {
            setStyle("effort"); setLevels("low, medium, high"); setDefaultLevel("high");
          }
        }}><option value="chat_completions">Chat Completions</option><option value="responses">Responses（API key）</option><option value="chatgpt_responses" disabled={provider() !== "chatgpt-subscription"}>ChatGPT Responses（官方订阅）</option><option disabled>Messages（尚未接入）</option><option disabled>Gemini（尚未接入）</option></select></label>
        <label>思考方式<select name="reasoning-style" value={style()} onChange={(event) => setStyle(event.currentTarget.value as ModelSpec["reasoning"]["style"])}><option value="none">无思考</option><option value="effort">reasoning_effort</option><option value="deepseek" disabled={protocol() !== "chat_completions"}>DeepSeek thinking</option></select></label>
        <Show when={style() !== "none"}>
          <label>思考档位（逗号分隔）<input name="reasoning-levels" value={levels()} onInput={(event) => setLevels(event.currentTarget.value)} placeholder="low, medium, high" /></label>
          <label>默认思考档位<select name="reasoning-default" value={defaultLevel()} onChange={(event) => setDefaultLevel(event.currentTarget.value)}><option value="">无默认档位</option><For each={levelList()}>{(level) => <option value={level}>{level}</option>}</For></select></label>
        </Show>
        <label>上下文窗口（token，留空为未知）<input name="context-window" type="number" min="1" step="1" value={context()} onInput={(event) => setContext(event.currentTarget.value)} /></label>
        <label class="admin-check"><input name="vision" type="checkbox" checked={vision()} onChange={(event) => setVision(event.currentTarget.checked)} />支持图片输入</label>
        <Show when={provider() !== "chatgpt-subscription"} fallback={<p class="admin-note">ChatGPT 订阅费用未知，不使用 API token 单价；能力与思考档位必须来自实际账号模型目录。</p>}>
          <h4 class="admin-field-heading">API 计费单价</h4>
          <label>缓存输入（USD / 百万 token）<input name="price-cached" type="number" min="0" step="any" value={cached()} onInput={(event) => setCached(event.currentTarget.value)} /></label>
          <label>无缓存输入（USD / 百万 token）<input name="price-uncached" type="number" min="0" step="any" value={uncached()} onInput={(event) => setUncached(event.currentTarget.value)} /></label>
          <label>输出（USD / 百万 token）<input name="price-output" type="number" min="0" step="any" value={output()} onInput={(event) => setOutput(event.currentTarget.value)} /></label>
        </Show>
      </fieldset>
      <p class="admin-note">支持 Chat Completions 与 Responses 原生协议。未知参数请按提供商文档填写；价格留空表示未知，不等于免费。思考 token 按输出单价计费，输出计数不重复包含思考。</p>
      <Show when={error()}><p role="alert" class="admin-error">{error()}</p></Show>
      <div class="admin-actions"><button class="primary" type="submit" disabled={busy()}>{busy() ? "保存中…" : "保存模型"}</button><button type="button" disabled={busy()} onClick={props.onCancel}>取消</button></div>
    </form>
  </section>;
}
