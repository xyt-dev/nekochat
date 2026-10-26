// One requestAnimationFrame loop for every streamed DOM update: producers queue work and it all
// lands in a single batched update per frame.
import { batch } from "solid-js";

const queue = new Set<() => void>();
let pending = false;

function flush() {
  pending = false;
  const jobs = [...queue];
  queue.clear();
  batch(() => {
    for (const job of jobs) job();
  });
}

/** Runs `job` in the next animation frame; queuing the same function twice runs it once. */
export function inNextFrame(job: () => void): void {
  queue.add(job);
  if (!pending) {
    pending = true;
    requestAnimationFrame(flush);
  }
}
