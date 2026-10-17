/** 全局 toast：极简订阅-发布，避免引入状态库 */

export type ToastKind = "info" | "success" | "error" | "warning";

export interface ToastItem {
  id: number;
  msg: string;
  kind: ToastKind;
}

type Listener = (t: ToastItem) => void;

const listeners = new Set<Listener>();
let seq = 0;

export function toast(msg: string, kind: ToastKind = "info") {
  const item: ToastItem = { id: ++seq, msg, kind };
  listeners.forEach((l) => l(item));
}

export function subscribeToast(fn: Listener): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}
