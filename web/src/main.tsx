import { render } from "solid-js/web";
import "katex/dist/katex.min.css";
import "./themes";
import "./styles.css";
import { App } from "./App";

const root = document.getElementById("root")!;
const bench = import.meta.env.DEV && new URLSearchParams(location.search).has("bench");

render(() => <App offline={bench} />, root);

if (bench) void import("./bench").then((m) => m.runBench());
