// SPDX-License-Identifier: Apache-2.0 OR MIT
import { createSignal } from "solid-js";

/** Only small, opaque keys and user-controlled values may cross the storage boundary. */
export interface GraphicState {
  params: Record<string, number>;
  colors: Record<string, string>;
  views: Record<string, number>;
}
const STORAGE_KEY = "nk.graphics.v1";
const MAX_ENTRIES = 80;
const MAX_BYTES = 48 * 1024;
const MAX_FIELDS = 1024;
const states = new Map<string, GraphicState>();
const resetters = new Set<() => void>();
const [runtimeEpoch, setRuntimeEpoch] = createSignal(0);
export { runtimeEpoch };
let loaded = false;
let saveFrame = 0;

/** Hash identities so neither node labels nor message text enter persisted keys. */
export function stateKey(value: string): string {
  let a = 2166136261, b = 2246822519;
  for (let i = 0; i < value.length; i++) {
    const c = value.charCodeAt(i);
    a = Math.imul(a ^ c, 16777619);
    b = Math.imul(b ^ c, 3266489917);
  }
  return `${(a >>> 0).toString(16).padStart(8, "0")}${(b >>> 0).toString(16).padStart(8, "0")}`;
}
export function emptyGraphicState(): GraphicState { return { params: {}, colors: {}, views: {} }; }
const opaqueKey = /^[a-f0-9]{16}$/;
const hexColor = /^#[a-f0-9]{6}(?:[a-f0-9]{2})?$/i;

function cleanState(value: unknown): GraphicState {
  const result = emptyGraphicState();
  if (!value || typeof value !== "object") return result;
  const input = value as Record<string, unknown>;
  for (const field of ["params", "colors", "views"] as const) {
    const part = input[field];
    if (!part || typeof part !== "object" || Array.isArray(part)) continue;
    for (const [key, item] of Object.entries(part).slice(-MAX_FIELDS)) {
      if (!opaqueKey.test(key)) continue;
      if (field === "colors") {
        if (typeof item === "string" && hexColor.test(item)) result.colors[key] = item;
      } else if (typeof item === "number" && Number.isFinite(item) && (field !== "views" || Number.isSafeInteger(item) && item >= 0)) {
        result[field][key] = item;
      }
    }
  }
  return result;
}
function ensureLoaded(): void {
  if (loaded) return;
  loaded = true;
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY);
    if (!raw) return;
    if (raw.length > MAX_BYTES) { sessionStorage.removeItem(STORAGE_KEY); return; }
    const entries: unknown = JSON.parse(raw);
    if (!Array.isArray(entries)) return;
    for (const entry of entries.slice(-MAX_ENTRIES)) {
      if (Array.isArray(entry) && typeof entry[0] === "string" && opaqueKey.test(entry[0])) states.set(entry[0], cleanState(entry[1]));
    }
  } catch { /* Sandboxed/private sessions can disable storage; memory remains bounded. */ }
}
function boundedSnapshot(): string {
  while (states.size > MAX_ENTRIES) states.delete(states.keys().next().value!);
  let encoded = JSON.stringify([...states]);
  while (encoded.length > MAX_BYTES && states.size) {
    states.delete(states.keys().next().value!);
    encoded = JSON.stringify([...states]);
  }
  return encoded;
}
function save(): void {
  saveFrame = 0;
  const encoded = boundedSnapshot();
  try { sessionStorage.setItem(STORAGE_KEY, encoded); } catch { /* A full storage quota must not break interactions. */ }
}
export function readGraphicState(id: string): GraphicState {
  ensureLoaded();
  const key = stateKey(id);
  const value = states.get(key);
  if (value) { states.delete(key); states.set(key, value); }
  return value ? { params: { ...value.params }, colors: { ...value.colors }, views: { ...value.views } } : emptyGraphicState();
}
export function writeGraphicState(id: string, value: GraphicState): GraphicState {
  ensureLoaded();
  const key = stateKey(id);
  const bounded = cleanState(value);
  states.delete(key);
  states.set(key, bounded);
  boundedSnapshot();
  if (!saveFrame) saveFrame = requestAnimationFrame(save);
  return bounded;
}
export function registerGraphicsReset(reset: () => void): () => void {
  resetters.add(reset);
  return () => { resetters.delete(reset); };
}
/** Account boundaries clear persisted preferences, live preferences and all compute work. */
export function resetGraphicsRuntime(): void {
  if (saveFrame) cancelAnimationFrame(saveFrame);
  saveFrame = 0;
  states.clear();
  loaded = true;
  try { sessionStorage.removeItem(STORAGE_KEY); } catch { /* Storage may be unavailable. */ }
  for (const reset of resetters) reset();
  setRuntimeEpoch(epoch => epoch + 1);
}
