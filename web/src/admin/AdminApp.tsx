import { createSignal, For, Match, onMount, Show, Switch } from "solid-js";
import { render } from "solid-js/web";
import { allThemes, applyTheme, loadCustomThemes, themeId } from "../themes";
import { createAdminApi, errorMessage, type AdminApi, type Character, type ListedModel, type Provider, type Settings } from "./api";
import { Models, Roles } from "./Management";
import { Monitor } from "./Monitor";
import "./admin.css";

function ThemeSelect() {
  return <label class="admin-theme">主题<select aria-label="主题" value={themeId()} onChange={(event) => { const theme = allThemes().find((item) => item.id === event.currentTarget.value); if (theme) applyTheme(theme); }}><For each={allThemes()}>{(theme) => <option value={theme.id}>{theme.group} · {theme.name}</option>}</For></select></label>;
}

function Dashboard(props: { api: AdminApi; providers: Provider[] }) {
  const [tab, setTab] = createSignal("monitor");
  const [characters, setCharacters] = createSignal<Character[]>([]);
  const [models, setModels] = createSignal<ListedModel[]>([]);
  const [settings, setSettings] = createSignal<Settings>({ default_model_id: null, default_character_id: null });
  const [ready, setReady] = createSignal(false);
  const [error, setError] = createSignal("");
  const [loading, setLoading] = createSignal(false);
  async function reload() {
    const [nextCharacters, nextModels, nextSettings] = await Promise.all([props.api.characters(), props.api.models(), props.api.settings()]);
    setCharacters(nextCharacters); setModels(nextModels); setSettings(nextSettings); setReady(true);
  }
  async function initialize() {
    setError(""); setLoading(true);
    try { await reload(); } catch (error) { setError(errorMessage(error)); } finally { setLoading(false); }
  }
  onMount(() => void initialize());
  return <>
    <nav class="admin-nav" aria-label="管理功能"><For each={[["monitor", "用量监控"], ["models", "模型管理"], ["roles", "角色管理"]]}>{([key, label]) => <button type="button" aria-current={tab() === key ? "page" : undefined} classList={{ active: tab() === key }} onClick={() => setTab(key)}>{label}</button>}</For></nav>
    <main class="admin-main"><Show when={error()}><div class="admin-error" role="alert">{error()} <button type="button" disabled={loading()} onClick={() => void initialize()}>重试加载配置</button></div></Show>
      <Switch><Match when={tab() === "monitor"}><Monitor api={props.api} /></Match><Match when={tab() === "models" || tab() === "roles"}><Show when={ready()} fallback={<p role="status">{loading() ? "正在加载管理配置…" : "配置加载失败，请重试。"}</p>}><Show when={tab() === "models"} fallback={<Roles api={props.api} providers={props.providers} models={models()} characters={characters()} settings={settings()} reload={reload} />}><Models api={props.api} providers={props.providers} models={models()} characters={characters()} settings={settings()} reload={reload} /></Show></Show></Match></Switch>
    </main>
  </>;
}

function AdminApp() {
  const [state, setState] = createSignal<"checking" | "login" | "disabled" | "ready" | "error">("checking");
  const [providers, setProviders] = createSignal<Provider[]>([]);
  const [password, setPassword] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const api = createAdminApi((status) => {
    setProviders([]); setPassword(""); setError(""); setState(status === 404 ? "disabled" : "login");
  });
  async function checkSession() {
    setError("");
    try { const items = await api.providers(); setProviders(items); setState("ready"); }
    catch (error) { if (state() !== "login" && state() !== "disabled") { setError(errorMessage(error)); setState("error"); } }
  }
  async function login(event: SubmitEvent) {
    event.preventDefault(); setBusy(true); setError("");
    const submitted = password(); setPassword("");
    try { await api.session(submitted); await checkSession(); }
    catch (error) { setError(errorMessage(error)); }
    finally { setBusy(false); }
  }
  onMount(() => {
    document.title = "nekochat · 管理";
    void checkSession();
    // A custom-theme lookup must not trigger the ordinary chat access-password queue.
    void loadCustomThemes(async () => { const response = await fetch("/api/themes", { credentials: "same-origin" }); if (!response.ok) throw new Error("主题列表不可用"); return await response.json(); });
  });
  return <div class="admin-app"><header class="admin-header"><a href="/" class="admin-brand">nekochat <span>管理</span></a><div class="admin-actions"><a href="/">返回聊天</a><ThemeSelect /></div></header>
    <Switch><Match when={state() === "ready"}><Dashboard api={api} providers={providers()} /></Match>
      <Match when={state() === "login"}><main class="admin-gate"><h1>管理登录</h1><p class="admin-note">使用独立管理密码。此页面不会请求普通聊天访问口令，也不会保存管理密码。</p><form onSubmit={(event) => void login(event)}><label>管理密码<input name="admin-password" type="password" autocomplete="current-password" required value={password()} onInput={(event) => setPassword(event.currentTarget.value)} autofocus disabled={busy()} /></label><Show when={error()}><p role="alert" class="admin-error">{error()}</p></Show><button type="submit" class="primary" disabled={busy()}>{busy() ? "登录中…" : "登录管理端"}</button></form></main></Match>
      <Match when={state() === "disabled"}><main class="admin-gate"><h1>管理端未启用</h1><p>管理 API 返回 404。请在服务器配置中设置独立的 <code>admin_password</code>，然后重启服务。</p><p class="admin-note">管理端始终需要管理密码；普通聊天访问密码不能代替。此页面不提供配置凭证编辑。</p><button type="button" onClick={() => { setState("checking"); void checkSession(); }}>重新检查</button></main></Match>
      <Match when={state() === "error"}><main class="admin-gate"><h1>无法连接管理端</h1><p role="alert" class="admin-error">{error()}</p><button type="button" onClick={() => { setState("checking"); void checkSession(); }}>重试</button></main></Match>
      <Match when={state() === "checking"}><main class="admin-gate" role="status">正在检查管理会话…</main></Match>
    </Switch>
  </div>;
}

export function mountAdmin(root: HTMLElement) { render(() => <AdminApp />, root); }
