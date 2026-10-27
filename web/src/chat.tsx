import { render } from "solid-js/web";
import "katex/dist/katex.min.css";
import { App } from "./App";

export function mountChat(root: HTMLElement) {
  const bench = import.meta.env.DEV && new URLSearchParams(location.search).has("bench");
  render(() => <App offline={bench} />, root);
  if (bench) void import("./bench").then((module) => module.runBench());
}
