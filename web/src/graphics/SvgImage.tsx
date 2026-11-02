// SPDX-License-Identifier: Apache-2.0 OR MIT
import { createEffect, createMemo, createSignal, createUniqueId, onCleanup, Show } from "solid-js";
import type { SvgNode } from "./types";
import { useGraphics } from "./context";
import { ColorControl, ColorSettings } from "./controls";
import { colorInputValue, resolvedColor, svgThemeProperties } from "./theme";
import { stateKey } from "./state";
import { bindSvgAnimationVariables, svgAnimationValues } from "./svgAnimation";

const SVG_NAMESPACE = "http://www.w3.org/2000/svg";
const paintAttributes = ["fill", "stroke", "color"];
const colorProperties = [...paintAttributes, "background-color", "border-top-color", "border-right-color", "border-bottom-color", "border-left-color", "outline-color", "text-decoration-color", "column-rule-color", "caret-color", "accent-color"];

function metadata(doc: XMLDocument, attribute: string, element?: string): string {
  const root = doc.documentElement;
  const references = root.getAttribute(attribute)?.trim().split(/\s+/) ?? [];
  const referenced = references.map(id => doc.getElementById(id)?.textContent?.trim() ?? "").filter(Boolean).join(" ");
  return referenced || Array.from(root.children).find(child => child.namespaceURI === SVG_NAMESPACE && child.localName === element)?.textContent?.trim() || "";
}

/** Only palette mode edits color declarations; normal SVG values stay browser-owned. */
function recolorSvg(root: SVGSVGElement, host: HTMLElement, palette: string, followTheme: boolean, properties: Record<string, string>): void {
  const hex = colorInputValue(host, palette);
  const channels = [1, 3, 5].map(offset => Number.parseInt(hex.slice(offset, offset + 2), 16)).join(" ");
  const relativeColors = CSS.supports("color", "rgb(from red 1 2 3 / alpha)");
  const concreteColors = new Map<string, string>();
  const concreteColor = (source: string): string => {
    const cached = concreteColors.get(source);
    if (cached) return cached;
    const computed = resolvedColor(host, source);
    const alphaToken = computed.match(/\/\s*([\d.e+-]+)(%)?\s*\)$/i) ?? (/^rgba\(/i.test(computed) ? computed.match(/,\s*([\d.e+-]+)\s*\)$/i) : null);
    const alpha = alphaToken ? Number.parseFloat(alphaToken[1]) / (alphaToken[2] === "%" ? 100 : 1) : 1;
    const color = `rgba(${channels.replaceAll(" ", ", ")}, ${alpha})`;
    concreteColors.set(source, color);
    return color;
  };
  // Retarget the primary theme color so gradient stops keep their topology and alpha.
  const accent = bindSvgAnimationVariables(root.style.getPropertyValue("--nk-accent").trim() || properties["--nk-accent"] || "", properties);
  if (!followTheme && accent && !/\\|\bvar\b/i.test(accent) && CSS.supports("color", accent)) {
    properties["--nk-accent"] = concreteColor(accent);
    root.style.setProperty("--nk-accent", properties["--nk-accent"], "important");
  }
  const resources = new Set<Element>();
  for (const resource of Array.from(root.querySelectorAll("linearGradient,radialGradient,pattern,mask,clipPath,filter"))) {
    resources.add(resource);
    for (const child of Array.from(resource.querySelectorAll("*"))) resources.add(child);
  }
  const resourceElements = Array.from(resources);
  const recolor = (value: string): string => {
    const paint = value.trim();
    // Resolve only for classification: unknown document variables may be paint servers.
    const bound = bindSvgAnimationVariables(paint, properties);
    if (/^(?:none|transparent|inherit|initial|unset|revert(?:-layer)?)$/i.test(bound) || /\\|\b(?:url|currentcolor|var)\b/i.test(bound) || followTheme && bound !== paint || !CSS.supports("color", bound)) return value;
    // Relative colors keep source alpha; older engines receive equivalent concrete RGBA.
    return relativeColors ? `rgb(from ${paint} ${channels} / alpha)` : concreteColor(bound);
  };
  const declarations = (style: CSSStyleDeclaration) => {
    for (const property of colorProperties) {
      const value = style.getPropertyValue(property);
      if (!value) continue;
      const replacement = recolor(value);
      if (replacement !== value) style.setProperty(property, replacement, style.getPropertyPriority(property));
    }
  };
  const rules = (list: CSSRuleList) => {
    for (const rule of Array.from(list)) {
      // A keyframe can drive mask luminance or a filter resource as well as paint.
      // Preserve its authored values; theme-token keyframes still follow the palette.
      if (rule.type === CSSRule.KEYFRAMES_RULE) continue;
      if ("style" in rule) {
        let sharedWithResource = false;
        if ("selectorText" in rule) {
          try { sharedWithResource = resourceElements.some(element => element.matches(rule.selectorText as string)); }
          catch { sharedWithResource = true; }
        }
        if (!sharedWithResource) declarations(rule.style as CSSStyleDeclaration);
      }
      if ("cssRules" in rule) rules(rule.cssRules as CSSRuleList);
    }
  };
  for (const element of [root, ...Array.from(root.querySelectorAll("*"))]) {
    if (resources.has(element)) continue;
    if (element.namespaceURI === SVG_NAMESPACE) {
      for (const name of paintAttributes) {
        const value = element.getAttribute(name);
        if (value !== null) element.setAttribute(name, recolor(value));
      }
      if (["animate", "set"].includes(element.localName) && paintAttributes.includes(element.getAttribute("attributeName") ?? "")) {
        for (const name of ["from", "to", "by", "values"]) {
          const value = element.getAttribute(name);
          if (value !== null) element.setAttribute(name, (name === "values" ? svgAnimationValues(value) : [value]).map(recolor).join(";"));
        }
      }
    }
    const style = (element as SVGElement | HTMLElement).style;
    if (style) declarations(style);
    if (element.localName === "style") {
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(element.textContent ?? "");
      rules(sheet.cssRules);
      element.textContent = Array.from(sheet.cssRules, rule => rule.cssText).join("\n");
    }
  }
}

/** Resolve SMIL-only CSS tokens in a script-disabled, temporary native SVG document. */
async function bindNativeAnimationVariables(doc: XMLDocument, host: HTMLElement, signal: AbortSignal): Promise<void> {
  const animations = Array.from(doc.querySelectorAll("animate,animateTransform,animateMotion,set")).filter(element =>
    element.namespaceURI === SVG_NAMESPACE && ["from", "to", "by", "values"].some(name => /var\(|\\/i.test(element.getAttribute(name) ?? "")));
  if (!animations.length) return;
  if (signal.aborted) throw new DOMException("Aborted", "AbortError");
  const frame = document.createElement("iframe");
  frame.sandbox.add("allow-same-origin");
  frame.setAttribute("aria-hidden", "true");
  frame.inert = true;
  const box = (doc.documentElement as unknown as SVGSVGElement).viewBox.baseVal;
  const width = Math.max(1, host.clientWidth);
  const height = box.width > 0 ? width * box.height / box.width : 150;
  frame.style.cssText = `position:fixed;left:-100000px;top:0;width:${width}px;height:${height}px;visibility:hidden;pointer-events:none;border:0`;
  const url = URL.createObjectURL(new Blob([new XMLSerializer().serializeToString(doc)], { type: "image/svg+xml" }));
  const { promise, resolve, reject } = Promise.withResolvers<void>();
  const abort = () => reject(new DOMException("Aborted", "AbortError"));
  try {
    signal.addEventListener("abort", abort, { once: true });
    frame.onload = () => resolve();
    frame.onerror = () => reject(new Error("无法解析 SVG 动画样式"));
    frame.src = url;
    host.appendChild(frame);
    await promise;
    const native = frame.contentDocument;
    const view = frame.contentWindow;
    if (!native || !view) throw new Error("无法读取隔离 SVG 动画样式");
    const nativeAnimations = Array.from(native.querySelectorAll("animate,animateTransform,animateMotion,set")).filter(element =>
      element.namespaceURI === SVG_NAMESPACE && ["from", "to", "by", "values"].some(name => /var\(|\\/i.test(element.getAttribute(name) ?? "")));
    for (let index = 0; index < animations.length; index++) {
      const animation = nativeAnimations[index];
      const target = (animation as SVGAnimationElement).targetElement;
      if (!target) continue;
      const computed = view.getComputedStyle(target);
      const properties: Record<string, string> = Object.create(null);
      for (let property = 0; property < computed.length; property++) {
        const name = computed.item(property);
        if (name.startsWith("--")) properties[name] = computed.getPropertyValue(name).trim();
      }
      for (const name of ["from", "to", "by", "values"]) {
        const value = animations[index].getAttribute(name);
        if (value !== null) animations[index].setAttribute(name, bindSvgAnimationVariables(value, properties, true));
      }
    }
  } finally {
    signal.removeEventListener("abort", abort);
    frame.onload = frame.onerror = null;
    frame.remove();
    URL.revokeObjectURL(url);
  }
}

/** Nested SVG data images are separate documents and need their own inherited defaults. */
async function prepareDocument(doc: XMLDocument, host: HTMLElement, hostProperties: Readonly<Record<string, string>>, override: string | undefined, followTheme: boolean, signal: AbortSignal, depth = 0): Promise<string> {
  if (doc.querySelector("parsererror") || doc.documentElement.localName !== "svg" || doc.documentElement.namespaceURI !== SVG_NAMESPACE) throw new Error("无法读取已校验的 SVG");
  const root = doc.documentElement as unknown as SVGSVGElement;
  const properties = { ...hostProperties };
  const bindings = (doc.createElementNS(SVG_NAMESPACE, "g") as SVGGElement).style;
  for (const [name, value] of Object.entries(properties)) if (value) bindings.setProperty(name, value);
  // A trusted data stylesheet precedes source CSS without changing :first/nth-child.
  const defaults = `@layer { :where(:root) { ${bindings.cssText} } }`;
  doc.insertBefore(doc.createProcessingInstruction("xml-stylesheet", `type="text/css" href="data:text/css,${encodeURIComponent(defaults)}"`), root);
  if (!root.hasAttribute("color")) root.setAttribute("color", override || (followTheme ? "var(--nk-accent)" : "var(--nk-text)"));
  if (!root.hasAttribute("fill")) root.setAttribute("fill", "currentColor");
  if (override || followTheme) recolorSvg(root, host, override || "var(--nk-accent)", followTheme && !override, properties);
  for (const element of Array.from(root.querySelectorAll("*"))) {
    const svgImage = element.namespaceURI === SVG_NAMESPACE && ["image", "feImage"].includes(element.localName);
    const htmlImage = element.namespaceURI === "http://www.w3.org/1999/xhtml" && element.localName === "img";
    if (!svgImage && !htmlImage) continue;
    for (const attribute of Array.from(element.attributes)) {
      if (attribute.localName !== (svgImage ? "href" : "src")) continue;
      const data = attribute.value.match(/^data:image\/svg\+xml((?:;[^,]*)?),(.*)$/is);
      if (!data) continue;
      if (depth >= 8) throw new Error("SVG 嵌套图像超过已校验的层数限制");
      const source = /;base64(?:;|$)/i.test(data[1])
        ? new TextDecoder("utf-8", { fatal: true }).decode(Uint8Array.from(atob(data[2]), character => character.charCodeAt(0)))
        : decodeURIComponent(data[2]);
      const nested = new DOMParser().parseFromString(source, "image/svg+xml");
      const xml = await prepareDocument(nested, host, hostProperties, override, followTheme, signal, depth + 1);
      const bytes = new TextEncoder().encode(xml);
      let binary = "";
      for (let start = 0; start < bytes.length; start += 16384) binary += String.fromCharCode(...bytes.subarray(start, start + 16384));
      element.setAttributeNS(attribute.namespaceURI, attribute.name, `data:image/svg+xml;base64,${btoa(binary)}`);
    }
  }
  await bindNativeAnimationVariables(doc, host, signal);
  return new XMLSerializer().serializeToString(doc);
}

/** Validated XML is displayed only as an isolated image; native style probes are sandboxed. */
export default function SvgImage(props: { node: SvgNode; nodeKey: string }) {
  const context = useGraphics();
  let host!: HTMLDivElement;
  const [url, setUrl] = createSignal("");
  const [error, setError] = createSignal("");
  let previous = "";
  let currentUrl = "";
  const descriptionId = `nk-svg-description-${createUniqueId()}`;
  const documentView = createMemo(() => new DOMParser().parseFromString(props.node.code, "image/svg+xml"));
  const accessibleName = createMemo(() => {
    const doc = documentView();
    return props.node.title || metadata(doc, "aria-labelledby") || doc.documentElement.getAttribute("aria-label")?.trim() || metadata(doc, "", "title") || "SVG 图形";
  });
  const description = createMemo(() => {
    const doc = documentView();
    return metadata(doc, "aria-describedby") || doc.documentElement.getAttribute("aria-description")?.trim() || metadata(doc, "", "desc");
  });
  createEffect(() => {
    context.themeRevision();
    const colorKey = `${props.nodeKey}:svg-color`;
    const override = context.state().colors[stateKey(colorKey)];
    const followTheme = context.followsTheme(colorKey);
    const original = documentView();
    if (!host) return;
    const controller = new AbortController();
    onCleanup(() => controller.abort());
    const doc = original.cloneNode(true) as XMLDocument;
    void prepareDocument(doc, host, svgThemeProperties(host), override, followTheme, controller.signal).then(xml => {
      if (controller.signal.aborted || xml === previous) return;
      const nextUrl = URL.createObjectURL(new Blob([xml], { type: "image/svg+xml" }));
      const oldUrl = currentUrl;
      currentUrl = nextUrl;
      previous = xml;
      setUrl(nextUrl);
      setError("");
      if (oldUrl) URL.revokeObjectURL(oldUrl);
    }).catch(failure => {
      if (controller.signal.aborted) return;
      setError(String(failure));
      setUrl("");
      previous = "";
      if (currentUrl) URL.revokeObjectURL(currentUrl);
      currentUrl = "";
    });
  });
  onCleanup(() => { if (currentUrl) URL.revokeObjectURL(currentUrl); currentUrl = previous = ""; });
  return <div ref={host} class="nk-graphic-svg">
    <Show when={props.node.palette}><div class="nk-graphic-widget-tools"><ColorSettings><ColorControl colorKey={`${props.nodeKey}:svg-color`} label="图形主色" fallback="accent" /></ColorSettings></div></Show>
    <Show when={url()}><img class="nk-graphic-isolated-svg" src={url()} alt={accessibleName()} aria-describedby={description() ? descriptionId : undefined} style={{ height: props.node.height ? `${props.node.height}px` : undefined }} onError={() => setError("浏览器无法显示此 SVG；可查看源规范。")} /></Show>
    <Show when={description()}><span id={descriptionId} hidden>{description()}</span></Show>
    <Show when={error()}><p class="nk-graphic-error" role="alert">{error()}</p></Show>
  </div>;
}
