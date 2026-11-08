import { createSignal, For, onCleanup, Show } from "solid-js";
import { errorMessage, newModelProtocol, type AdminApi, type ModelSpec, type Provider, type UpstreamModel } from "./api";

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
  const [levels, setLevels] = createSignal([...initial.reasoning.levels]);
  const [customLevel, setCustomLevel] = createSignal("");
  const [defaultLevel, setDefaultLevel] = createSignal(initial.reasoning.default ?? "");
  const [vision, setVision] = createSignal(initial.vision);
  const [context, setContext] = createSignal(initial.context_window?.toString() ?? "");
  const [cached, setCached] = createSignal(initial.price.cached_in?.toString() ?? "");
  const [uncached, setUncached] = createSignal(initial.price.uncached_in?.toString() ?? "");
  const [output, setOutput] = createSignal(initial.price.output?.toString() ?? "");
  const [metadata, setMetadata] = createSignal<UpstreamModel | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [notice, setNotice] = createSignal("");
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const number = (value: string, label: string, integer = false): number | null => {
    if (!value.trim()) return null;
    const capacity = integer ? value.trim().match(/^(\d+(?:\.\d+)?)\s*([km])?$/i) : null;
    const parsed = integer && capacity ? Number(capacity[1]) * (capacity[2]?.toLowerCase() === "m" ? 1_000_000 : capacity[2] ? 1_000 : 1) : Number(value);
    if (!Number.isFinite(parsed) || parsed < 0 || integer && (!Number.isSafeInteger(parsed) || parsed === 0)) throw new Error(`${label}必须是${integer ? "正整数，可使用 K / M" : "非负数"}`);
    return parsed;
  };
  function removeLevel(level: string) {
    setLevels((current) => current.filter((item) => item !== level));
    if (defaultLevel() === level) setDefaultLevel("");
  }
  async function fetchMetadata() {
    if (busy() || disposed) return;
    setBusy(true); setError(""); setNotice(""); setMetadata(null);
    try {
      const directory = await props.api.directory(provider());
      if (disposed) return;
      const found = directory.find((item) => item.upstream_id === upstream().trim());
      if (!found?.defaults) throw new Error("站点目录未返回此模型的信息，请按站点文档手动配置");
      setMetadata(found);
      setNotice(found.price_source ? "已获取站点公布的模型信息与价格，请选择采用。" : "已获取模型信息；站点未公布可识别的 USD 单价，价格保持未知。");
    } catch (failure) { if (!disposed) setError(errorMessage(failure)); }
    finally { if (!disposed) setBusy(false); }
  }
  function adoptMetadata(pricesOnly: boolean) {
    const found = metadata()?.defaults;
    if (!found) return;
    setCached(found.price.cached_in?.toString() ?? "");
    setUncached(found.price.uncached_in?.toString() ?? "");
    setOutput(found.price.output?.toString() ?? "");
    if (!pricesOnly) {
      setName(found.display_name); setStyle(protocol() !== "chat_completions" && found.reasoning.style === "deepseek" ? "effort" : found.reasoning.style);
      if (provider() === "chatgpt-subscription") setProtocol("chatgpt_responses");
      setLevels([...found.reasoning.levels]); setDefaultLevel(found.reasoning.default ?? "");
      setVision(found.vision); setContext(found.context_window?.toString() ?? "");
    }
    setNotice("已填入获取的值，点击保存模型后生效。未公布的价格已清空为未知。");
  }
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy() || disposed) return;
    setBusy(true); setError("");
    try {
      const parsedSort = Number(sort());
      if (!Number.isSafeInteger(parsedSort)) throw new Error("排序必须是整数");
      const reasoningLevels = style() === "none" ? [] : levels();
      if (style() !== "none" && !reasoningLevels.length) throw new Error("请选择或添加思考档位");
      if (style() !== "none" && defaultLevel() && !reasoningLevels.includes(defaultLevel())) throw new Error("请从当前档位选择默认思考档位");
      await props.api.saveModel(props.modelId, {
        provider: provider(), upstream_id: upstream().trim(), display_name: name().trim(), sort: parsedSort,
        protocol: protocol(), reasoning: { style: style(), levels: reasoningLevels, default: style() === "none" ? null : defaultLevel() || null },
        vision: vision(), context_window: number(context(), "上下文窗口", true),
        price: { cached_in: number(cached(), "缓存输入价格"), uncached_in: number(uncached(), "无缓存输入价格"), output: number(output(), "输出价格") },
      });
      if (!disposed) await props.onSaved();
    } catch (failure) { if (!disposed) setError(errorMessage(failure)); }
    finally { if (!disposed) setBusy(false); }
  }
  return <section class="admin-card admin-editor" aria-labelledby="model-editor-title" aria-busy={busy()}>
    <h3 id="model-editor-title">{props.modelId ? "编辑模型" : "上架模型"}</h3>
    <form onSubmit={(event) => void save(event)}>
      <fieldset disabled={busy()} class="admin-form-grid">
        <h4 class="admin-field-heading">模型标识与上架顺序</h4>
        <label>提供商<select name="model-provider" value={provider()} onChange={(event) => {
          const key = event.currentTarget.value; setProvider(key); setMetadata(null); setNotice("");
          if (key === "chatgpt-subscription") { setProtocol("chatgpt_responses"); setCached(""); setUncached(""); setOutput(""); }
          else if (!props.modelId) {
            const next = newModelProtocol(props.providers.find((site) => site.key === key), "chat_completions", upstream().trim());
            setProtocol(next);
            if (next !== "chat_completions" && style() === "deepseek") setStyle("effort");
          }
          else if (protocol() === "chatgpt_responses") setProtocol("chat_completions");
        }} required><For each={props.providers}>{(item) => <option value={item.key} disabled={item.connected === false}>{item.label} · {item.key}{item.connected === false ? "（未连接）" : ""}</option>}</For></select></label>
        <label>上游模型 ID<input name="upstream-id" required value={upstream()} onInput={(event) => { setUpstream(event.currentTarget.value); setMetadata(null); setNotice(""); }} /></label>
        <label>显示名<input name="display-name" required value={name()} onInput={(event) => setName(event.currentTarget.value)} /></label>
        <label>排序（小数值在前）<input name="model-sort" type="number" step="1" required value={sort()} onInput={(event) => setSort(event.currentTarget.value)} /></label>
        <div class="admin-actions"><button type="button" disabled={!provider() || !upstream().trim()} onClick={() => void fetchMetadata()}>获取模型信息与价格</button></div>
        <Show when={metadata()} keyed>{(item) => <div class="admin-field-heading">
          <p class="admin-note">获取时间：{item.fetched_at_ms ? new Date(item.fetched_at_ms).toLocaleString() : "刚刚"} · 价格单位：USD / 百万 token</p>
          <Show when={item.price_source}><p class="admin-note">价格来源：<a href={item.price_source!} target="_blank" rel="noopener noreferrer">{item.price_source}</a></p></Show>
          <p class="admin-note">缓存输入 {item.defaults?.price.cached_in ?? "未知"} · 无缓存输入 {item.defaults?.price.uncached_in ?? "未知"} · 输出 {item.defaults?.price.output ?? "未知"}</p>
          <div class="admin-actions"><button type="button" onClick={() => adoptMetadata(false)}>采用能力与价格</button><button type="button" onClick={() => adoptMetadata(true)}>仅采用价格</button></div>
        </div>}</Show>
        <h4 class="admin-field-heading">协议与生成能力</h4>
        <label>协议<select name="model-protocol" value={protocol()} disabled={provider() === "chatgpt-subscription"} onChange={(event) => {
          const next = event.currentTarget.value as ModelSpec["protocol"]; setProtocol(next);
          if (next !== "chat_completions" && style() === "deepseek") setStyle("effort");
        }}><option value="chat_completions">Chat Completions</option><option value="responses">Responses（API Key）</option><option value="chatgpt_responses" disabled={provider() !== "chatgpt-subscription"}>ChatGPT Responses（官方订阅）</option></select></label>
        <label>思考方式<select name="reasoning-style" value={style()} onChange={(event) => setStyle(event.currentTarget.value as ModelSpec["reasoning"]["style"])}><option value="none">无思考</option><option value="effort">reasoning_effort</option><option value="deepseek" disabled={protocol() !== "chat_completions"}>DeepSeek thinking</option></select></label>
        <Show when={style() !== "none"}>
          <div class="admin-field-heading"><span>支持的思考强度</span><div class="admin-toolbar"><For each={provider() === "chatgpt-subscription" ? [...new Set([...initial.reasoning.levels, ...levels()])] : [...new Set(["none", "minimal", "low", "medium", "high", "xhigh", "max", ...levels()])]}>{(level) => <label class="admin-check"><input name={`reasoning-level-${level}`} type="checkbox" checked={levels().includes(level)} onChange={(event) => { if (event.currentTarget.checked) setLevels((current) => [...current, level]); else removeLevel(level); }} />{level}</label>}</For></div><p class="admin-note">仅选择此站点模型实际支持的档位。</p></div>
          <Show when={provider() !== "chatgpt-subscription"}><label>添加自定义思考强度<input name="custom-reasoning-level" value={customLevel()} onInput={(event) => setCustomLevel(event.currentTarget.value)} placeholder="站点支持的档位" /></label><div class="admin-actions"><button type="button" disabled={!customLevel().trim()} onClick={() => { const level = customLevel().trim(); if (!levels().includes(level)) setLevels((current) => [...current, level]); setCustomLevel(""); }}>添加档位</button></div></Show>
          <div class="admin-field-heading admin-toolbar"><For each={levels()}>{(level) => <button type="button" aria-label={`删除思考强度 ${level}`} onClick={() => removeLevel(level)}>{level} ×</button>}</For></div>
          <label>默认思考档位<select name="reasoning-default" value={defaultLevel()} onChange={(event) => setDefaultLevel(event.currentTarget.value)}><option value="">使用上游默认</option><For each={levels()}>{(level) => <option value={level}>{level}</option>}</For></select></label>
        </Show>
        <label>上下文窗口限制（token，留空为未知）<input name="context-window" type="text" inputmode="text" list="model-context-presets" value={context()} onInput={(event) => setContext(event.currentTarget.value)} placeholder="例如 128K、1M 或 200000" /><datalist id="model-context-presets"><For each={["8K", "16K", "32K", "64K", "128K", "200K", "256K", "1M", "2M"]}>{(value) => <option value={value} />}</For></datalist></label>
        <label class="admin-check"><input name="vision" type="checkbox" checked={vision()} onChange={(event) => setVision(event.currentTarget.checked)} />支持图片输入</label>
        <Show when={provider() !== "chatgpt-subscription"} fallback={<p class="admin-note">ChatGPT 订阅费用未知，不使用 API token 单价；能力与思考档位来自实际账号目录。</p>}>
          <h4 class="admin-field-heading">API 计费单价</h4>
          <label>缓存输入（USD / 百万 token）<input name="price-cached" type="number" min="0" step="any" value={cached()} onInput={(event) => setCached(event.currentTarget.value)} /></label>
          <label>无缓存输入（USD / 百万 token）<input name="price-uncached" type="number" min="0" step="any" value={uncached()} onInput={(event) => setUncached(event.currentTarget.value)} /></label>
          <label>输出（USD / 百万 token）<input name="price-output" type="number" min="0" step="any" value={output()} onInput={(event) => setOutput(event.currentTarget.value)} /></label>
        </Show>
      </fieldset>
      <p class="admin-note">上下文窗口记录模型容量，K = 1000、M = 1000000。价格留空表示未知；获取采用站点公开的明确单价，不从模型名称推测。思考 token 按输出单价计费。</p>
      <Show when={notice()}><p role="status" class="admin-notice">{notice()}</p></Show>
      <Show when={error()}><p role="alert" class="admin-error">{error()}</p></Show>
      <div class="admin-actions"><button class="primary" type="submit" disabled={busy()}>{busy() ? "处理中…" : "保存模型"}</button><button type="button" disabled={busy()} onClick={props.onCancel}>取消</button></div>
    </form>
  </section>;
}
