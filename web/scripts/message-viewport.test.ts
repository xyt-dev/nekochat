import assert from "node:assert/strict";
import { afterAll, test } from "bun:test";
import { createMessageViewport } from "../src/components/messageViewport";

const resizeObserverDescriptor = Object.getOwnPropertyDescriptor(globalThis, "ResizeObserver");
afterAll(() => {
  if (resizeObserverDescriptor) Object.defineProperty(globalThis, "ResizeObserver", resizeObserverDescriptor);
  else Reflect.deleteProperty(globalThis, "ResizeObserver");
});

class FrameWindow extends EventTarget {
  jobs = new Map<number, FrameRequestCallback>();
  sequence = 0;
  requestAnimationFrame = (job: FrameRequestCallback) => { const id = ++this.sequence; this.jobs.set(id, job); return id; };
  cancelAnimationFrame = (id: number) => { this.jobs.delete(id); };
  flush() { const jobs = [...this.jobs.values()]; this.jobs.clear(); for (const job of jobs) job(0); }
}
class ElementFixture extends EventTarget {
  scrollTop = 0;
  scrollHeight = 2000;
  clientHeight = 500;
  clientWidth = 700;
  clientTop = 0;
  flowTop = 0;
  parent?: ElementFixture;
  isConnected = true;
  children: ElementFixture[] = [];
  ownerDocument!: { defaultView: FrameWindow; elementFromPoint(): ElementFixture };
  getBoundingClientRect() { const top = this.parent ? this.flowTop - this.parent.scrollTop : 0; return { top, bottom: top + 500, left: 0, right: 700, width: 700, height: 500 }; }
  closest() { return this; }
  contains(node: ElementFixture) { return this.children.includes(node); }
}
class SizeObserver {
  static current: SizeObserver;
  active = true;
  constructor(private callback: ResizeObserverCallback) { SizeObserver.current = this; }
  observe() {}
  disconnect() { this.active = false; }
  changed() { if (this.active) this.callback([], this as unknown as ResizeObserver); }
}
function fixture(top = 500, nativeAnchoring = true) {
  const view = new FrameWindow();
  const scroll = new ElementFixture(); scroll.scrollTop = top;
  const content = new ElementFixture();
  const anchor = new ElementFixture(); anchor.parent = scroll; anchor.flowTop = top + 20;
  content.children.push(anchor);
  scroll.ownerDocument = { defaultView: view, elementFromPoint: () => anchor };
  content.ownerDocument = scroll.ownerDocument;
  Object.assign(globalThis, { ResizeObserver: SizeObserver });
  const controller = createMessageViewport(scroll as unknown as HTMLElement, content as unknown as HTMLElement, nativeAnchoring);
  const event = (type: string, values = {}) => scroll.dispatchEvent(Object.assign(new Event(type), values));
  return { scroll, content, anchor, view, controller, event };
}

test("upward reading cancels an already queued follow command", () => {
  const f = fixture();
  f.controller.followLatest();
  f.event("wheel", { deltaY: -100 });
  f.scroll.scrollTop = 450; f.event("scroll");
  f.view.flush();
  assert.equal(f.scroll.scrollTop, 450);
  f.controller.dispose();
});

test("measuring a tall message spanning the viewport cannot push the reader down", () => {
  const f = fixture();
  f.event("wheel", { deltaY: -100 }); f.event("scroll");
  f.scroll.scrollHeight += 1253;
  SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 500);
  f.controller.dispose();
});


test("explicit send resumes following without retaining an old reading anchor", () => {
  const f = fixture();
  f.event("wheel", { deltaY: -100 }); f.event("scroll");
  f.controller.followLatest(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1500);
  f.scroll.scrollHeight += 300;
  SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1800);
  f.controller.dispose();
});

test("clicking at the bottom preserves automatic following", () => {
  const f = fixture(1500);
  f.controller.followLatest(); f.view.flush();
  f.event("pointerdown", { clientX: 100 });
  f.scroll.scrollHeight += 200;
  SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1700);
  f.controller.dispose();
});

test("scrolling down to the end resumes following subsequent content", () => {
  const f = fixture();
  f.event("wheel", { deltaY: -100 }); f.event("scroll");
  f.event("wheel", { deltaY: 100 });
  f.scroll.scrollTop = 1500; f.event("scroll"); f.view.flush();
  f.scroll.scrollHeight += 100;
  SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1600);
  f.controller.dispose();
});

test("disposing a viewport cancels its scroll work and disconnects observation", () => {
  const f = fixture();
  f.controller.followLatest(); f.controller.dispose();
  f.scroll.scrollHeight += 500;
  SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 500);
  assert.equal(SizeObserver.current.active, false);
});

test("an unmoved scrollbar-edge press resumes the previous bottom-follow mode", () => {
  const f = fixture(1500);
  f.controller.followLatest(); f.view.flush();
  f.event("pointerdown", { clientX: 695 });
  f.view.dispatchEvent(new Event("pointerup"));
  f.scroll.scrollHeight += 200;
  SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1700);
  f.controller.dispose();
});

test("compatible anchoring does not undo native user progress before a scroll event arrives", () => {
  const f = fixture(500, false);
  f.event("wheel", { deltaY: -60 });
  f.scroll.scrollTop = 440;
  f.scroll.scrollHeight += 1253;
  SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 440);
  f.controller.dispose();
});

test("compatible anchoring corrects actual above-content displacement exactly once", () => {
  const f = fixture(500, false);
  f.event("wheel", { deltaY: -100 });
  f.anchor.flowTop += 180;
  f.scroll.scrollHeight += 180;
  SizeObserver.current.changed(); f.view.flush();
  SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 680);
  assert.equal(f.anchor.getBoundingClientRect().top, 20);
  f.controller.dispose();
});
