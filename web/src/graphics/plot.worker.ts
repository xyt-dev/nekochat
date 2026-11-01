// SPDX-License-Identifier: Apache-2.0 OR MIT
/// <reference lib="webworker" />
import init, { sample_graphic_plot, type InitOutput } from "../wasm/nekochat_mdwasm.js";
import type { PlotRequest, PlotResponse, PlotSample } from "./types";

// Initialization and every expression evaluation stay off the UI thread.
let ready: Promise<InitOutput> | undefined;
self.onmessage = async (event: MessageEvent<PlotRequest>) => {
  const { client, revision, source, values } = event.data;
  let response: PlotResponse;
  try {
    await (ready ??= init().catch(error => { ready = undefined; throw error; }));
    const sample = JSON.parse(sample_graphic_plot(source, values)) as PlotSample;
    response = { client, revision, sample };
  } catch (error) {
    response = { client, revision, error: String(error).slice(0, 1000) };
  }
  self.postMessage(response);
};
