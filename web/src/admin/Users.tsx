import { createSignal, For, onMount, Show } from "solid-js";
import { localImageUrl } from "../localImages";
import { errorMessage, type AdminApi, type PasswordStorage, type UserAdmin, type UserPassword, type UserSettings } from "./api";
import { AvatarInput } from "./AvatarInput";

function UserEditor(props: { api: AdminApi; initial: UserAdmin | null; onSaved: () => Promise<void>; onCancel: () => void }) {
  const initial = props.initial;
  const [username, setUsername] = createSignal(initial?.username ?? "");
  const [password, setPassword] = createSignal("");
  const [avatar, setAvatar] = createSignal(initial?.avatar ?? null);
  const [busy, setBusy] = createSignal(false);
  const [uploading, setUploading] = createSignal(false);
  const [error, setError] = createSignal("");
  async function save(event: SubmitEvent) {
    event.preventDefault(); setError("");
    if (!username().trim()) { setError("用户名不能为空"); return; }
    if (password() || !initial) {
      const bytes = new TextEncoder().encode(password()).byteLength;
      if (bytes < 8 || bytes > 1024) { setError("密码必须为 8–1024 个 UTF-8 字节"); return; }
    }
    if (initial && password() && !confirm(`确定重置「${initial.username}」的密码？该用户现有登录会话将失效。`)) return;
    const submitted = password(); setPassword(""); setBusy(true);
    try {
      if (initial) await props.api.updateUser(initial.id, { username: username().trim(), avatar: avatar(), ...(submitted ? { password: submitted } : {}) });
      else await props.api.createUser({ username: username().trim(), password: submitted });
      await props.onSaved();
    } catch (error) { setError(errorMessage(error)); }
    finally { setBusy(false); }
  }
  return <section class="admin-card admin-editor" aria-labelledby="user-editor-title">
    <h3 id="user-editor-title">{initial ? `编辑用户：${initial.username}` : "创建用户"}</h3>
    <form onSubmit={(event) => void save(event)}>
      <fieldset disabled={busy() || uploading()}>
        <div class="admin-form-grid">
          <label>用户名<input name="user-username" autocomplete="off" required value={username()} onInput={(event) => setUsername(event.currentTarget.value)} /></label>
          <label>{initial ? "重置密码（留空不修改）" : "初始密码"}<input name="user-password" type="password" autocomplete="new-password" required={!initial} value={password()} onInput={(event) => setPassword(event.currentTarget.value)} /></label>
        </div>
        <p class="admin-note">密码长度为 8–1024 个 UTF-8 字节；中文等字符按实际编码字节数计算。</p>
        <Show when={initial}><AvatarInput api={props.api} name="user-avatar" value={avatar()} onChange={setAvatar} onUploading={setUploading} disabled={busy()} /></Show>
        <p class="admin-note">{initial ? "修改用户名或头像不会分配其他账号的历史。重置密码沿用该账号原有的明文或哈希存储方式，并退出该用户的现有会话。" : "创建普通聊天账号，密码按当前新账号存储策略保存。不授予管理权限，也不会自动取得旧对话；创建后可编辑头像。"}</p>
      </fieldset>
      <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show>
      <div class="admin-actions"><button type="submit" class="primary" disabled={busy() || uploading()}>{busy() ? "保存中…" : initial ? "保存用户" : "创建用户"}</button><button type="button" disabled={busy() || uploading()} onClick={props.onCancel}>取消</button></div>
    </form>
  </section>;
}

function PasswordView(props: { api: AdminApi; user: UserAdmin; disabled: boolean }) {
  const [result, setResult] = createSignal<UserPassword | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  async function reveal() {
    setBusy(true); setError(""); setResult(null);
    try { setResult(await props.api.userPassword(props.user.id)); }
    catch (error) { setError(errorMessage(error)); }
    finally { setBusy(false); }
  }
  return <div class="admin-password-view">
    <Show when={result()} fallback={<button type="button" disabled={props.disabled || busy()} onClick={() => void reveal()}>{busy() ? "查询中…" : "查看密码"}</button>}>{(value) => <>
      <Show when={value().password !== null} fallback={<p class="admin-note">当前账号使用不可恢复的哈希，无法查看原文，也不会显示哈希。修改新账号策略、登录或重置密码均不会改变此账号的存储方式。</p>}><p class="admin-password" aria-label="用户密码">{value().password}</p></Show>
      <button type="button" onClick={() => { setResult(null); setError(""); }}>隐藏密码</button>
    </>}</Show>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show>
  </div>;
}

export function Users(props: { api: AdminApi }) {
  const [users, setUsers] = createSignal<UserAdmin[]>([]);
  const [settings, setSettings] = createSignal<UserSettings | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [ready, setReady] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [notice, setNotice] = createSignal("");
  const [editing, setEditing] = createSignal<{ initial: UserAdmin | null } | null>(null);
  const blocked = () => loading() || busy() || !!editing();
  async function reload() {
    const [nextUsers, nextSettings] = await Promise.all([props.api.users(), props.api.userSettings()]);
    setUsers(nextUsers.users); setSettings(nextSettings); setReady(true);
  }
  async function initialize() {
    setLoading(true); setError(""); setNotice("");
    try { await reload(); }
    catch (error) { setError(errorMessage(error)); }
    finally { setLoading(false); }
  }
  async function mutate(action: () => Promise<string>) {
    setBusy(true); setError(""); setNotice("");
    try { const message = await action(); await reload(); setNotice(message); }
    catch (error) { setError(errorMessage(error)); }
    finally { setBusy(false); }
  }
  function changeStorage(mode: PasswordStorage) {
    void mutate(async () => { await props.api.saveUserSettings(mode); return `新账号密码存储已设为${mode === "hash" ? "哈希" : "明文"}；已有账号及其会话不变`; });
  }
  function assignLegacy(user: UserAdmin) {
    const count = settings()?.legacy_conversations ?? 0;
    if (confirm(`确定把当前全部 ${count} 个未归属旧对话分配给「${user.username}」？这些对话及其附件将只属于该用户，其他普通用户不可访问。此处不提供撤销分配。`)) void mutate(async () => { const result = await props.api.assignLegacyConversations(user.id); return `已向「${user.username}」分配 ${result.assigned} 个旧对话`; });
  }
  onMount(() => void initialize());
  return <section aria-labelledby="users-title" aria-busy={loading() || busy()}>
    <div class="admin-section-heading"><div><h2 id="users-title">用户管理</h2><p class="admin-note">所有聊天账号独立隔离；管理密码与聊天账号无关。列表不获取密码，只有点击「查看密码」才请求该用户的可恢复原文。</p></div><div class="admin-actions"><button type="button" disabled={blocked()} onClick={() => void initialize()}>{loading() ? "加载中…" : "刷新用户"}</button><button type="button" class="primary" disabled={!ready() || blocked()} onClick={() => { setEditing({ initial: null }); setNotice(""); }}>创建用户</button></div></div>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show><Show when={notice()}><p role="status">{notice()}</p></Show>
    <Show when={loading() || busy()}><p role="status">{loading() ? "正在加载用户与存储设置…" : "正在保存更改…"}</p></Show>
    <Show when={settings()}>{(current) => <section class="admin-card" aria-labelledby="password-storage-title">
      <h3 id="password-storage-title">新账号密码存储</h3><p>新建账号使用：<strong data-password-storage={current().password_storage}>{current().password_storage === "plain" ? "明文" : "哈希"}</strong>（新安装默认明文）</p>
      <p class="admin-note">仅影响此后创建的账号，包括自行注册和管理员创建。已有账号始终保留各自实际存储方式，登录或重置密码也不会转换；修改策略不会退出已有会话。明文账号允许管理员显式查看密码，哈希账号无法恢复原文。</p>
      <div class="admin-actions" role="group" aria-label="新账号密码存储策略"><button type="button" aria-pressed={current().password_storage === "plain"} disabled={blocked() || current().password_storage === "plain"} onClick={() => changeStorage("plain")}>新账号使用明文</button><button type="button" aria-pressed={current().password_storage === "hash"} disabled={blocked() || current().password_storage === "hash"} onClick={() => changeStorage("hash")}>新账号使用哈希</button></div>
      <h4 class="admin-legacy-heading">未归属旧对话：<span data-legacy-conversations>{current().legacy_conversations}</span></h4><p class="admin-note">旧对话不会自动分给首个注册用户。请在目标用户卡片上明确分配；该操作会分配当时仍未归属的全部旧对话。</p>
    </section>}</Show>
    <Show when={editing()} keyed>{(draft) => <UserEditor api={props.api} initial={draft.initial} onCancel={() => setEditing(null)} onSaved={async () => { await reload(); setEditing(null); setNotice(draft.initial ? "用户已更新" : "用户已创建"); }} />}</Show>
    <Show when={ready()}><div class="admin-card-list"><For each={users()} fallback={<p class="admin-empty">暂无用户。新账号也可在聊天登录页自行注册。</p>}>{(user) => <article class="admin-card" data-user-id={user.id}>
      <div class="admin-section-heading"><div class="admin-identity"><Show when={user.avatar && localImageUrl(user.avatar)} fallback={<span class="admin-avatar admin-avatar-placeholder" aria-hidden="true">{user.username.slice(0, 1)}</span>}>{(url) => <img class="admin-avatar" src={url()} alt={`${user.username}的头像`} loading="lazy" decoding="async" />}</Show><div><h3>{user.username} <span class="admin-badge">{user.disabled ? "已停用" : "已启用"}</span></h3><p class="admin-note">密码实际存储：{user.password_storage === "plain" ? "明文（可显式查看）" : "哈希（原文不可恢复）"}</p></div></div></div>
      <p class="admin-note">用户 ID：{user.id}<br />创建时间：<time dateTime={new Date(user.created_at).toISOString()}>{new Date(user.created_at).toLocaleString()}</time></p>
      <div class="admin-actions"><button type="button" disabled={blocked()} onClick={() => { setEditing({ initial: user }); setNotice(""); }}>编辑用户 / 重置密码</button><button type="button" disabled={blocked()} onClick={() => { if (confirm(user.disabled ? `确定重新启用「${user.username}」？该用户可重新登录。` : `确定停用「${user.username}」？现有登录会话将失效，用户将无法登录，历史数据保留。`)) void mutate(async () => { await props.api.updateUser(user.id, { disabled: !user.disabled }); return user.disabled ? "用户已重新启用" : "用户已停用，现有会话已失效"; }); }}>{user.disabled ? "启用用户" : "停用用户"}</button><button type="button" class="danger" disabled={blocked()} onClick={() => { if (confirm(`确定永久删除「${user.username}」及其全部对话与附件？此操作不可撤销，现有登录会话也将失效。`)) void mutate(async () => { await props.api.deleteUser(user.id); return "用户及其数据已删除"; }); }}>删除用户</button></div>
      <PasswordView api={props.api} user={user} disabled={blocked()} />
      <button type="button" disabled={blocked() || !settings()?.legacy_conversations} onClick={() => assignLegacy(user)}>分配未归属旧对话（{settings()?.legacy_conversations ?? 0}）</button>
    </article>}</For></div></Show>
  </section>;
}
