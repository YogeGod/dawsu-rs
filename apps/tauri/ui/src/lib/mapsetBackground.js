import { invoke } from "./ipc.js";

const MAX_CACHE = 120;
const MAX_CONCURRENT = 4;

/** @type {Map<string, string>} */
const urlCache = new Map();
/** @type {Map<string, Set<(url: string) => void>>} */
const pendingListeners = new Map();
/** @type {Array<{ key: string, osuPath: string }>} */
const queue = [];
let activeCount = 0;

/** Sniffs image MIME type from raw image bytes (PNG, JPEG, WebP, GIF, BMP). */
export function detectImageMime(bytes) {
  if (!bytes || bytes.length < 4) return "";
  const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  if (u8.length >= 8 && u8[0] === 0x89 && u8[1] === 0x50 && u8[2] === 0x4e && u8[3] === 0x47) {
    return "image/png";
  }
  if (u8[0] === 0xff && u8[1] === 0xd8 && u8[2] === 0xff) {
    return "image/jpeg";
  }
  if (
    u8.length >= 12 &&
    u8[0] === 0x52 && u8[1] === 0x49 && u8[2] === 0x46 && u8[3] === 0x46 &&
    u8[8] === 0x57 && u8[9] === 0x45 && u8[10] === 0x42 && u8[11] === 0x50
  ) {
    return "image/webp";
  }
  if (u8[0] === 0x47 && u8[1] === 0x49 && u8[2] === 0x46 && u8[3] === 0x38) {
    return "image/gif";
  }
  if (u8[0] === 0x42 && u8[1] === 0x4d) {
    return "image/bmp";
  }
  return "";
}

function storeCachedUrl(key, url) {
  if (urlCache.has(key)) {
    const old = urlCache.get(key);
    urlCache.delete(key);
    if (old && old !== url && old.startsWith("blob:")) {
      try { URL.revokeObjectURL(old); } catch {}
    }
  }
  urlCache.set(key, url);
  while (urlCache.size > MAX_CACHE) {
    const oldestKey = urlCache.keys().next().value;
    const oldestUrl = urlCache.get(oldestKey);
    urlCache.delete(oldestKey);
    if (oldestUrl && oldestUrl.startsWith("blob:")) {
      try { URL.revokeObjectURL(oldestUrl); } catch {}
    }
  }
}

export function getCachedBackground(key) {
  if (!key || !urlCache.has(key)) return undefined;
  const val = urlCache.get(key);
  // Refresh LRU recency.
  urlCache.delete(key);
  urlCache.set(key, val);
  return val;
}

function finishKey(key, url) {
  storeCachedUrl(key, url);
  const listeners = pendingListeners.get(key);
  pendingListeners.delete(key);
  if (listeners) {
    for (const cb of listeners) {
      try { cb(url); } catch {}
    }
  }
}

function drainQueue() {
  while (activeCount < MAX_CONCURRENT && queue.length > 0) {
    const job = queue.shift();
    if (!job || urlCache.has(job.key)) {
      if (job) finishKey(job.key, urlCache.get(job.key) ?? "");
      continue;
    }
    activeCount++;
    invoke("mapset_background", { osuPath: job.osuPath })
      .then((buf) => {
        const bytes = buf instanceof ArrayBuffer
          ? new Uint8Array(buf)
          : ArrayBuffer.isView(buf)
            ? new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength)
            : Array.isArray(buf)
              ? new Uint8Array(buf)
              : null;
        if (!bytes || bytes.length < 4) {
          finishKey(job.key, "");
          return;
        }
        const mime = detectImageMime(bytes);
        if (!mime || typeof URL?.createObjectURL !== "function") {
          finishKey(job.key, "");
          return;
        }
        const url = URL.createObjectURL(new Blob([bytes], { type: mime }));
        finishKey(job.key, url);
      })
      .catch(() => {
        finishKey(job.key, "");
      })
      .finally(() => {
        activeCount--;
        drainQueue();
      });
  }
}

/**
 * Fetches a mapset's background image via binary Tauri IPC (with LRU Blob caching
 * and concurrency throttling). Returns the cached URL synchronously if ready.
 */
export function requestMapsetBackground(key, osuPath, onLoaded) {
  if (!key || !osuPath) return "";
  const cached = getCachedBackground(key);
  if (cached !== undefined) {
    return cached;
  }
  if (typeof window === "undefined" || !window.__TAURI__?.core?.invoke) {
    return "";
  }
  let listeners = pendingListeners.get(key);
  if (!listeners) {
    listeners = new Set();
    pendingListeners.set(key, listeners);
    queue.push({ key, osuPath });
  }
  if (onLoaded) listeners.add(onLoaded);
  drainQueue();
  return "";
}

/**
 * Svelte action that lazily requests a mapset's background once its card enters
 * (or approaches) the viewport.
 */
export function observeMapsetBg(node, params) {
  let current = params;
  let observer = null;
  let triggered = false;

  function trigger() {
    if (triggered || !current?.key || !current?.osuPath) return;
    triggered = true;
    const immediate = requestMapsetBackground(current.key, current.osuPath, (url) => {
      if (url && current?.onLoad) current.onLoad(current.key, url);
    });
    if (immediate && current?.onLoad) {
      current.onLoad(current.key, immediate);
    }
  }

  function arm() {
    triggered = false;
    if (observer) {
      observer.disconnect();
      observer = null;
    }
    if (!current?.key || !current?.osuPath) return;
    const cached = getCachedBackground(current.key);
    if (cached !== undefined) {
      triggered = true;
      if (cached && current?.onLoad) current.onLoad(current.key, cached);
      return;
    }
    if (typeof IntersectionObserver === "undefined") {
      trigger();
      return;
    }
    observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          observer?.disconnect();
          observer = null;
          trigger();
        }
      },
      { rootMargin: "160px 0px" },
    );
    observer.observe(node);
  }

  arm();
  return {
    update(next) {
      const changed = next?.key !== current?.key || next?.osuPath !== current?.osuPath;
      current = next;
      if (changed) arm();
    },
    destroy() {
      observer?.disconnect();
      observer = null;
    },
  };
}
