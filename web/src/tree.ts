// Conversation tree helpers: the current path, sibling branches and the /tree outline.
import type { NodeView } from "./api";

export interface TreeIndex {
  byId: Map<string, NodeView>;
  /** Children per parent id (`null` = roots), oldest first. */
  children: Map<string | null, NodeView[]>;
}

export function indexTree(nodes: NodeView[]): TreeIndex {
  const byId = new Map<string, NodeView>();
  const children = new Map<string | null, NodeView[]>();
  for (const n of nodes) byId.set(n.id, n);
  for (const n of nodes) {
    // A node whose parent is missing is shown as a root rather than dropped.
    const parent = n.parent_id !== null && byId.has(n.parent_id) ? n.parent_id : null;
    let list = children.get(parent);
    if (!list) children.set(parent, (list = []));
    list.push(n);
  }
  for (const list of children.values()) {
    list.sort((a, b) => a.created_at - b.created_at || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  }
  return { byId, children };
}

function parentKey(idx: TreeIndex, n: NodeView): string | null {
  return n.parent_id !== null && idx.byId.has(n.parent_id) ? n.parent_id : null;
}

/** Root → leaf along `parent_id`. */
export function pathTo(idx: TreeIndex, leafId: string | null): NodeView[] {
  const path: NodeView[] = [];
  const seen = new Set<string>();
  let cur = leafId ? idx.byId.get(leafId) : undefined;
  while (cur && !seen.has(cur.id)) {
    seen.add(cur.id);
    path.push(cur);
    const p = parentKey(idx, cur);
    cur = p === null ? undefined : idx.byId.get(p);
  }
  return path.reverse();
}

/** The node's sibling group (itself included) and its position in it. */
export function siblingsOf(idx: TreeIndex, n: NodeView): { list: NodeView[]; index: number } {
  const list = idx.children.get(parentKey(idx, n)) ?? [n];
  return { list, index: Math.max(0, list.indexOf(n)) };
}

export interface OutlineRow {
  node: NodeView;
  /** Box-drawing guides drawn before the node. */
  prefix: string;
}

/**
 * Depth-first outline of the whole tree. A chain of only children stays at one indent level, so a
 * long linear conversation does not drift right; only real branch points add a level.
 */
export function outline(idx: TreeIndex): OutlineRow[] {
  const rows: OutlineRow[] = [];
  // Explicit stack: conversations can be thousands of messages deep.
  type Frame = { node: NodeView; guide: string; connector: string };
  const roots = idx.children.get(null) ?? [];
  const stack: Frame[] = [];
  const pushGroup = (list: NodeView[], guide: string) => {
    if (list.length === 1) {
      stack.push({ node: list[0], guide, connector: "" });
      return;
    }
    for (let i = list.length - 1; i >= 0; i--) {
      const last = i === list.length - 1;
      stack.push({
        node: list[i],
        guide: guide + (last ? "   " : "│  "),
        connector: guide + (last ? "└─ " : "├─ "),
      });
    }
  };
  pushGroup(roots, "");
  while (stack.length > 0) {
    const f = stack.pop()!;
    rows.push({ node: f.node, prefix: f.connector || f.guide });
    pushGroup(idx.children.get(f.node.id) ?? [], f.guide);
  }
  return rows;
}
