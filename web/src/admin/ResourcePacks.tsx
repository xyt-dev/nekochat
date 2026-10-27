import { createMemo, createSignal, For, Show } from "solid-js";
import { localImageUrl } from "../localImages";
import type { ResourcePack } from "./api";

/** Previews mount on demand and are paged: do not decode entire GIF packs at once. */
export function PackPreview(props: { pack: ResourcePack; onChooseBackground?: (url: string) => void }) {
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [page, setPage] = createSignal(0);
  const filtered = createMemo(() => props.pack.assets.filter((asset) => asset.name.toLocaleLowerCase().includes(query().toLocaleLowerCase())));
  const visible = createMemo(() => filtered().slice(page() * 24, (page() + 1) * 24));
  return <details class="admin-pack-preview" onToggle={(event) => setOpen(event.currentTarget.open)}>
    <summary>预览本地素材（{props.pack.sticker_count} 张）</summary>
    <Show when={open()}>
      <label>搜索素材<input type="search" aria-label={`搜索 ${props.pack.name} 素材`} value={query()} onInput={(event) => { setQuery(event.currentTarget.value); setPage(0); }} /></label>
      <div class="admin-asset-grid"><For each={visible()} fallback={<p class="admin-note">没有匹配素材</p>}>{(asset) => <figure>
        <Show when={localImageUrl(asset.url)} fallback={<p class="admin-error">素材地址不在本站资源目录</p>}>{(url) => <>
          <img src={url()} alt={asset.name} loading="lazy" decoding="async" width="120" height="120" />
          <Show when={props.onChooseBackground}><button type="button" onClick={() => props.onChooseBackground?.(url())}>用作背景</button></Show>
        </>}</Show><figcaption>{asset.name}</figcaption>
      </figure>}</For></div>
      <div class="admin-actions"><button type="button" disabled={page() === 0} onClick={() => setPage((value) => value - 1)}>上一页</button><span role="status">{page() + 1} / {Math.max(1, Math.ceil(filtered().length / 24))}</span><button type="button" disabled={(page() + 1) * 24 >= filtered().length} onClick={() => setPage((value) => value + 1)}>下一页</button></div>
    </Show>
  </details>;
}

export function ResourcePacks(props: { packs: ResourcePack[] }) {
  return <section aria-labelledby="resource-packs-title">
    <div class="admin-section-heading"><div><h2 id="resource-packs-title">本地资源包</h2><p class="admin-note">{props.packs.length} 个包 · 共 {props.packs.reduce((sum, pack) => sum + pack.sticker_count, 0)} 张素材。聊天仅能使用角色已授权的包。</p></div></div>
    <p class="admin-note">内置资源随程序提供。额外资源包由管理员放入服务器 data_dir/skills 下的包目录，重启加载。此处不接收任意文件路径，不下载网络素材或执行脚本。</p>
    <div class="admin-card-list"><For each={props.packs} fallback={<p class="admin-empty">尚未加载本地资源包</p>}>{(pack) => <article class="admin-card" data-pack-id={pack.id}>
      <h3>{pack.name} <small class="admin-badge">{pack.sticker_count} 张</small></h3><p class="mono">{pack.id}</p><p>{pack.description}</p>
      <details><summary>来源、授权与署名</summary><pre class="admin-resource-text">{pack.attribution}</pre></details>
      <details><summary>静态 SKILL 指引（不执行）</summary><pre class="admin-resource-text">{pack.skill}</pre></details>
      <PackPreview pack={pack} />
    </article>}</For></div>
  </section>;
}
