import { type Accessor, Index, Show } from "solid-js";
import { createMarkdown } from "../md/client";

/** Markdown rendered in the worker; plain text stands in until the first result arrives. */
export function Markdown(props: { id: string; text: Accessor<string>; final: Accessor<boolean> }) {
  const html = createMarkdown(() => props.id, props.text, props.final);
  return (
    <div class="md">
      <Show when={html()} fallback={<div class="md-plain">{props.text()}</div>}>
        {(blocks) => <Index each={blocks()}>{(block) => <div class="md-block" innerHTML={block()} />}</Index>}
      </Show>
    </div>
  );
}
