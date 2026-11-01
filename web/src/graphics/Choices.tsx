// SPDX-License-Identifier: Apache-2.0 OR MIT
import { For, Show } from "solid-js";
import type { ChoicesNode } from "./types";
import { useGraphics } from "./context";

export default function Choices(props: { node: ChoicesNode }) {
  const context = useGraphics();
  return <div class="nk-graphic-choices" role="group" aria-label={props.node.title ?? "建议问题"}>
    <Show when={!props.node.title}><p class="nk-graphic-choices-label">接下来可以问</p></Show>
    <div class="nk-graphic-choices-list"><For each={props.node.options}>{option =>
      <button type="button" class="nk-graphic-question" title={option.prompt} disabled={!context.canQuestion()} onClick={() => context.chooseQuestion(option.prompt)}>
        <span>{option.label}</span><svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true"><path d="M4 12 12 4M4 4h8v8" /></svg>
      </button>
    }</For></div>
    <p class="nk-graphic-choices-hint">点击填入输入框，可编辑后发送</p>
  </div>;
}
