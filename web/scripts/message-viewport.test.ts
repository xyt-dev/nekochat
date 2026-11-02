import assert from "node:assert/strict";
import { afterAll, test } from "bun:test";
import { createMessageViewport, messageAboveViewport } from "../src/components/messageViewport";

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
  timers = new Map<number, () => void>();
  setTimeout = (job: () => void) => { const id = ++this.sequence; this.timers.set(id, job); return id; };
  clearTimeout = (id: number) => { this.timers.delete(id); };
  quiet() { const jobs = [...this.timers.values()]; this.timers.clear(); for (const job of jobs) job(); }
  flush() { const jobs = [...this.jobs.values()]; this.jobs.clear(); for (const job of jobs) job(0); }
}
class ElementFixture extends EventTarget {
  scrollTop = 0;
  scrollHeight = 2000;
  clientHeight = 500;
  clientWidth = 700;
  clientTop = 0;
  onscrollend = null;
  visualLag = 0;
  flowTop = 0;
  offsetHeight = 500;
  parent?: ElementFixture;
  isConnected = true;
  children: ElementFixture[] = [];
  ownerDocument!: { defaultView: FrameWindow; elementFromPoint(): ElementFixture };
  get offsetParent() { return this.parent ?? null; }
  get offsetTop() { return this.flowTop; }
  getBoundingClientRect() {
    let top = this.flowTop;
    let parent = this.parent;
    while (parent?.parent) {
      top += parent.flowTop;
      parent = parent.parent;
    }
    if (parent) top -= parent.scrollTop;
    top += this.visualLag;
    return { top, bottom: top + 500, left: 0, right: 700, width: 700, height: 500 };
  }
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
function fixture(top = 500) {
  const view = new FrameWindow();
  const scroll = new ElementFixture(); scroll.scrollTop = top;
  const content = new ElementFixture(); content.parent = scroll;
  const anchor = new ElementFixture(); anchor.parent = content; anchor.flowTop = top + 20;
  content.children.push(anchor);
  scroll.ownerDocument = { defaultView: view, elementFromPoint: () => anchor };
  content.ownerDocument = scroll.ownerDocument;
  Object.assign(globalThis, { ResizeObserver: SizeObserver });
  const controller = createMessageViewport(scroll as unknown as HTMLElement, content as unknown as HTMLElement,
    () => { scroll.scrollTop = Math.max(0, scroll.scrollHeight - scroll.clientHeight); }, () => {});
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


test("a stale compositor rectangle cannot turn native scrolling into a layout correction", () => {
  const f = fixture(500);
  f.event("wheel", { deltaY: -100 });
  f.scroll.scrollTop = 420;
  f.anchor.visualLag = 80;
  SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 420);
  f.controller.dispose();
});

test("elastic bottom overscroll is left to the browser instead of being snapped back", () => {
  const f = fixture(1500);
  f.event("wheel", { deltaY: 100 });
  f.scroll.scrollTop = 1580; f.event("scroll"); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1580);
  f.scroll.scrollTop = 1500; f.event("scroll"); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1500);
  f.controller.dispose();
});

test("touch and inertia keep reading ownership through history remeasurement", () => {
  const f = fixture(1500);
  f.event("touchstart", { touches: [{ clientY: 100 }] });
  f.event("touchmove", { touches: [{ clientY: 180 }] });
  f.scroll.scrollTop = 1420; f.event("scroll");
  f.scroll.scrollHeight += 180; SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1420);
  f.event("touchend");
  f.scroll.scrollTop = 1320; f.event("scroll");
  f.scroll.scrollHeight += 120; SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1320);
  f.event("scrollend"); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1320);
  f.controller.dispose();
});

test("reaching the bottom during a touch leaves spring recovery alone and resumes later streaming", () => {
  const f = fixture(1300);
  f.event("touchstart", { touches: [{ clientY: 400 }] });
  f.event("touchmove", { touches: [{ clientY: 200 }] });
  f.scroll.scrollTop = 1540; f.event("scroll"); f.event("touchend");
  SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1540);
  f.scroll.scrollTop = 1500; f.event("scroll"); f.event("scrollend"); f.view.flush();
  f.scroll.scrollHeight += 100; SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1600);
  f.controller.dispose();
});

test("cold tall messages spanning the fold are not treated as fully above the viewport", () => {
  const f = fixture(500);
  f.anchor.flowTop = 100;
  f.anchor.offsetHeight = 1253;
  const above = () => messageAboveViewport(f.scroll as unknown as HTMLElement, f.anchor as unknown as Element);
  assert.equal(above(), false);
  f.anchor.offsetHeight = 400;
  assert.equal(above(), true);
  f.anchor.visualLag = 1000;
  assert.equal(above(), true);
  f.controller.dispose();
});

test("old WebKit waits for actual inertia to stop before resuming bottom following", () => {
  const f = fixture(1300);
  Reflect.deleteProperty(f.scroll, "onscrollend");
  f.event("touchstart", { touches: [{ clientY: 400 }] });
  f.event("touchmove", { touches: [{ clientY: 200 }] });
  f.scroll.scrollTop = 1500; f.event("scroll");
  f.event("touchend");
  f.scroll.scrollTop = 1450; f.view.quiet();
  f.scroll.scrollHeight += 100; SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1450);
  f.scroll.scrollTop = 1600; f.event("scroll"); f.view.quiet(); f.view.flush();
  f.scroll.scrollHeight += 100; SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1700);
  f.controller.dispose();
});

test("an edge swipe without offset movement cannot permanently suspend following", () => {
  const f = fixture(1500);
  f.event("touchstart", { touches: [{ clientY: 400 }] });
  f.event("touchmove", { touches: [{ clientY: 200 }] });
  f.event("touchend");
  f.scroll.scrollHeight += 100; SizeObserver.current.changed(); f.view.flush();
  assert.equal(f.scroll.scrollTop, 1600);
  f.controller.dispose();
});
