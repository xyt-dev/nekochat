// SPDX-License-Identifier: Apache-2.0 OR MIT
import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
import type { SvgNode } from "./types";
import { useGraphics } from "./context";
import { ColorControl, ColorSettings } from "./controls";
import { resolvedColor } from "./theme";
import { stateKey } from "./state";

/** Validated XML only ever enters a detached XML document and an isolated image. */
export default function SvgImage(props: { node: SvgNode; nodeKey: string }) {
  const context = useGraphics();
  let host!: HTMLDivElement;
  const [url, setUrl] = createSignal("");
  const [error, setError] = createSignal("");
  let previous = "";
  let currentUrl = "";
  const documentView = createMemo(() => new DOMParser().parseFromString(props.node.code, "image/svg+xml"));
  createEffect(() => {
    context.themeRevision();
    const colorKey = `${props.nodeKey}:svg-color`;
    const override = context.state().colors[stateKey(colorKey)];
    const followTheme = context.followsTheme(colorKey);
    const original = documentView();
    if (!host) return;
    try {
      if (original.querySelector("parsererror") || original.documentElement.localName !== "svg") throw new Error("无法读取已校验的 SVG");
      const doc = original.cloneNode(true) as XMLDocument;
      const root = doc.documentElement;
      const colors = new Map<string, string>();
      const resolve = (value: string): string => {
        const cached = colors.get(value);
        if (cached) return cached;
        const resolved = resolvedColor(host, value);
        colors.set(value, resolved);
        return resolved;
      };
      const text = resolve("var(--nk-text)");
      const accent = resolve("var(--nk-accent)");
      if (override || !root.hasAttribute("color")) root.setAttribute("color", override ? resolve(override) : text);
      if (!root.hasAttribute("fill")) root.setAttribute("fill", override ? resolve(override) : "currentColor");
      for (const element of [root, ...Array.from(root.querySelectorAll("*"))]) {
        for (const name of ["fill", "stroke", "color", "stop-color"]) {
          const value = element.getAttribute(name);
          if (value === null || /^(?:none|transparent|url\s*\()/i.test(value.trim())) continue;
          const hasTheme = /var\(|currentColor/i.test(value);
          const replacement = override ? resolve(override) : followTheme && !hasTheme ? accent : value.trim().startsWith("var(") ? resolve(value) : value;
          element.setAttribute(name, replacement);
        }
      }
      const xml = new XMLSerializer().serializeToString(doc);
      if (xml === previous) return;
      const nextUrl = URL.createObjectURL(new Blob([xml], { type: "image/svg+xml" }));
      const oldUrl = currentUrl;
      currentUrl = nextUrl;
      previous = xml;
      setUrl(nextUrl);
      setError("");
      if (oldUrl) URL.revokeObjectURL(oldUrl);
    } catch (failure) {
      setError(String(failure));
      setUrl("");
      previous = "";
      if (currentUrl) URL.revokeObjectURL(currentUrl);
      currentUrl = "";
    }
  });
  onCleanup(() => { if (currentUrl) URL.revokeObjectURL(currentUrl); currentUrl = previous = ""; });
  return <div ref={host} class="nk-graphic-svg">
    <Show when={props.node.palette}><div class="nk-graphic-widget-tools"><ColorSettings><ColorControl colorKey={`${props.nodeKey}:svg-color`} label="图形主色" fallback="accent" /></ColorSettings></div></Show>
    <Show when={url()}><img class="nk-graphic-isolated-svg" src={url()} alt={props.node.title ?? "SVG 图形"} style={{ height: props.node.height ? `${props.node.height}px` : undefined }} onError={() => setError("浏览器无法显示此 SVG；可查看源规范。")} /></Show>
    <Show when={error()}><p class="nk-graphic-error" role="alert">{error()}</p></Show>
  </div>;
}
