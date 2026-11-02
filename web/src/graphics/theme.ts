// SPDX-License-Identifier: Apache-2.0 OR MIT
const subscribers = new Set<() => void>();
let rootObserver: MutationObserver | undefined;
let headObserver: MutationObserver | undefined;
let media: MediaQueryList | undefined;
let frame = 0;
function schedule(): void {
  if (!frame) frame = requestAnimationFrame(() => {
    frame = 0;
    for (const subscriber of subscribers) subscriber();
  });
}
function loaded(event: Event): void {
  if (event.target instanceof HTMLLinkElement && event.target.rel === "stylesheet") schedule();
}
/** Shared observers include custom stylesheet insertion, text edits and asynchronous load. */
export function subscribeTheme(subscriber: () => void): () => void {
  subscribers.add(subscriber);
  if (subscribers.size === 1) {
    rootObserver = new MutationObserver(schedule);
    rootObserver.observe(document.documentElement, { attributes: true, subtree: true, attributeFilter: ["data-theme", "style", "class"] });
    headObserver = new MutationObserver(schedule);
    headObserver.observe(document.head, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ["href", "media", "disabled", "rel"] });
    document.addEventListener("load", loaded, true);
    window.addEventListener("resize", schedule);
    media = matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", schedule);
  }
  return () => {
    subscribers.delete(subscriber);
    if (!subscribers.size) {
      rootObserver?.disconnect();
      headObserver?.disconnect();
      rootObserver = headObserver = undefined;
      document.removeEventListener("load", loaded, true);
      window.removeEventListener("resize", schedule);
      media?.removeEventListener("change", schedule);
      media = undefined;
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
    }
  };
}
/** Snapshot custom properties without coercing lengths, opacity, or CSS expressions to colors. */
export function svgThemeProperties(host: HTMLElement): Record<string, string> {
  const computed = getComputedStyle(host);
  const properties: Record<string, string> = Object.create(null);
  for (let index = 0; index < computed.length; index++) {
    const name = computed.item(index);
    if (name.startsWith("--nk-")) properties[name] = computed.getPropertyValue(name).trim();
  }
  properties["--nk-muted"] = computed.getPropertyValue("--nk-muted").trim() || computed.getPropertyValue("--nk-text-muted").trim();
  return properties;
}

/** Browser color resolution is reserved for palette controls and explicit paint overrides. */
export function resolvedColor(host: HTMLElement, color: string): string {
  const probe = document.createElement("span");
  probe.style.cssText = "position:absolute;visibility:hidden;pointer-events:none";
  const computed = getComputedStyle(host);
  if (!computed.getPropertyValue("--nk-muted").trim()) probe.style.setProperty("--nk-muted", computed.getPropertyValue("--nk-text-muted"));
  probe.style.color = color;
  host.appendChild(probe);
  const resolved = getComputedStyle(probe).color;
  probe.remove();
  return resolved;
}
export function colorInputValue(host: HTMLElement, color: string): string {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1;
  const context = canvas.getContext("2d");
  if (!context) return "#000000";
  context.fillStyle = resolvedColor(host, color);
  context.fillRect(0, 0, 1, 1);
  const [r, g, b] = context.getImageData(0, 0, 1, 1).data;
  return `#${[r, g, b].map(value => value.toString(16).padStart(2, "0")).join("")}`;
}
