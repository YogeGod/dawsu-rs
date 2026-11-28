// Pure checks for Preferences › Audio output.

export const ASIO_DRIVER_HINT = "Choose an ASIO driver first.";

// MME and DirectSound need at least 10 ms; WASAPI and ASIO take 2 ms. Empty means device default.
export function bufferError(backend, bufferMs) {
  if (bufferMs === "" || bufferMs == null) return "";
  const min = backend === "mme" || backend === "direct_sound" ? 10 : 2;
  const ms = Number(bufferMs);
  return Number.isInteger(ms) && ms >= min && ms <= 500 ? "" : `Buffer must be between ${min} and 500 ms.`;
}

export function asioDriverMissing(backend, device) {
  return backend === "asio" && !String(device ?? "").trim();
}

// An apply error is not an input: it never turns Apply off.
export function canApply({ locked, loadError, backend, device, bufferMs }) {
  return !locked && !loadError && !bufferError(backend, bufferMs) && !asioDriverMissing(backend, device);
}
