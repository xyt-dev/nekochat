// SPDX-License-Identifier: Apache-2.0 OR MIT
import { registerGraphicsReset } from "./state";
import type { PlotNode, PlotRequest, PlotResponse, PlotSample } from "./types";

interface Client { revision: number; result: (sample: PlotSample) => void; error: (message: string) => void }
const clients = new Map<number, Client>();
const pending = new Map<number, PlotRequest>();
const MAX_CLIENTS = 256;
let worker: Worker | undefined;
let nextClient = 0;
let frame = 0;
let flight: PlotRequest | undefined;

function stopWorker(): void {
  worker?.terminate();
  worker = undefined;
  flight = undefined;
  pending.clear();
  if (frame) cancelAnimationFrame(frame);
  frame = 0;
}
registerGraphicsReset(() => { stopWorker(); clients.clear(); });

function failWorker(message: string): void {
  const affected = new Set(pending.keys());
  if (flight) affected.add(flight.client);
  stopWorker();
  for (const id of affected) clients.get(id)?.error(message);
}
function getWorker(): Worker {
  if (worker) return worker;
  const instance = new Worker(new URL("./plot.worker.ts", import.meta.url), { type: "module" });
  instance.onmessage = (event: MessageEvent<PlotResponse>) => {
    if (worker !== instance) return;
    const response = event.data;
    if (!flight || response.client !== flight.client || response.revision !== flight.revision) return;
    flight = undefined;
    const client = clients.get(response.client);
    if (client?.revision === response.revision) {
      if ("error" in response) client.error(response.error);
      else client.result(response.sample);
    }
    schedule();
  };
  instance.onerror = event => {
    event.preventDefault();
    if (worker === instance) failWorker("绘图计算未能启动，请重新调整参数重试。");
  };
  instance.onmessageerror = () => { if (worker === instance) failWorker("绘图计算响应无法读取。"); };
  worker = instance;
  return instance;
}
function dispatch(): void {
  frame = 0;
  if (flight || !pending.size) return;
  const [id, request] = pending.entries().next().value!;
  try {
    const instance = getWorker();
    pending.delete(id);
    flight = request;
    instance.postMessage(request);
  } catch {
    failWorker("当前浏览器无法启动绘图计算。");
  }
}
function schedule(): void {
  if (!frame && !flight && pending.size) frame = requestAnimationFrame(dispatch);
}

/** One request in flight globally and one latest request per mounted plot. */
export function createPlotSampler(result: (sample: PlotSample) => void, error: (message: string) => void) {
  const id = ++nextClient;
  let disposed = false;
  const client: Client = { revision: 0, result, error };
  if (clients.size < MAX_CLIENTS) clients.set(id, client);
  return {
    submit(node: PlotNode, values: Record<string, number>): void {
      if (disposed) return;
      if (!clients.has(id) && clients.size < MAX_CLIENTS) clients.set(id, client);
      if (!clients.has(id)) { error("当前可见函数图过多，请收起部分图形后重试。"); return; }
      client.revision++;
      pending.set(id, { client: id, revision: client.revision, source: JSON.stringify(node), values: JSON.stringify(values) });
      schedule();
    },
    dispose(): void {
      disposed = true;
      pending.delete(id);
      clients.delete(id);
      if (!clients.size) stopWorker();
    },
  };
}
