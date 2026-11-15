import { tick } from "svelte";

const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type=hidden])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[contenteditable=true]",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

function shown(el) {
  return !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
}

export function modalFocus(node, options = {}) {
  let opts = options || {};
  const opener = typeof document !== "undefined" ? document.activeElement : null;

  const focusable = () =>
    [...node.querySelectorAll(FOCUSABLE)].filter((el) => shown(el) && !el.closest("[inert]"));

  function first() {
    if (opts.initial) {
      const wanted = node.querySelector(opts.initial);
      if (wanted && shown(wanted)) return wanted;
    }
    return focusable().find((el) => el !== node) || node;
  }

  tick().then(() => {
    if (!node.isConnected) return;
    const el = first();
    el?.focus?.({ preventScroll: true });
  });

  function onKey(e) {
    if (e.key !== "Tab") return;
    const list = focusable();
    if (!list.length) {
      e.preventDefault();
      node.focus?.();
      return;
    }
    const head = list[0];
    const tail = list[list.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === head || active === node)) {
      e.preventDefault();
      tail.focus();
    } else if (!e.shiftKey && active === tail) {
      e.preventDefault();
      head.focus();
    } else if (!node.contains(active)) {
      e.preventDefault();
      head.focus();
    }
  }

  node.addEventListener("keydown", onKey);

  return {
    update(next) {
      opts = next || {};
    },
    destroy() {
      node.removeEventListener("keydown", onKey);
      const back = opts.returnFocus || opener;
      if (back && back.isConnected) back.focus?.({ preventScroll: true });
    },
  };
}
