import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Fetch species artwork, keyed by `characterId`, as blob URLs ready for `src`.
 *
 * Two things here are load-bearing and were learned the hard way:
 *
 * - **One batched call, not one per species.** A roster spans a few hundred
 *   distinct species; per-row IPC would cost far more than the decoding does.
 * - **Blob URLs, not the data URLs the backend returns.** Each icon is ~22 KB
 *   of base64 and the same species repeats across many rows, so putting data
 *   URLs straight into `src` costs tens of MB of attribute text and defeats
 *   the webview's per-URL image cache. This obliges the CSP in
 *   `tauri.conf.json` to allow `blob:` under `img-src`: without it Chromium
 *   hands back a blob URL happily and then refuses to render it, which shows
 *   up as a broken-image icon rather than any error the app can catch.
 *
 * Artwork is optional everywhere it is used: a failure logs and yields no
 * icons rather than propagating, so a missing pak never blanks a screen.
 */
/**
 * Turn a `data:` URL into a Blob without going through `fetch`.
 *
 * `fetch(dataUrl)` is the obvious way to do this and it worked on macOS, but it
 * left every Pal a `?` on Windows. `fetch` is governed by `connect-src`, which
 * this app's CSP never sets, so it falls back to `default-src 'self'` — and the
 * policy permits `data:` under `img-src` only. Chromium (WebView2) enforces
 * that and refuses the request; WebKit does not, which is why the bug was
 * invisible on the development machine. Decoding here depends on no CSP
 * directive at all, so it cannot regress the same way.
 */
function dataUrlToBlob(dataUrl: string): Blob {
  const comma = dataUrl.indexOf(",");
  const header = dataUrl.slice(0, comma);
  const binary = atob(dataUrl.slice(comma + 1));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  // `data:image/png;base64` -> `image/png`
  const mime = header.slice("data:".length).split(";")[0] || "image/png";
  return new Blob([bytes], { type: mime });
}

export function useSpeciesIcons(characterIds: string[]): Record<string, string> {
  const [icons, setIcons] = useState<Record<string, string>>({});
  const urlsRef = useRef<string[]>([]);

  // Callers build this array inline, so it is a fresh identity every render.
  // Keying the effect on the contents stops it refetching on every render.
  const key = useMemo(() => [...characterIds].sort().join(","), [characterIds]);

  useEffect(() => {
    if (key === "") return;
    let cancelled = false;

    void (async () => {
      try {
        const dataUrls = await invoke<Record<string, string>>("pal_icons", {
          characterIds: key.split(","),
        });
        const blobUrls: Record<string, string> = {};
        for (const [id, dataUrl] of Object.entries(dataUrls)) {
          blobUrls[id] = URL.createObjectURL(dataUrlToBlob(dataUrl));
        }

        // A superseded request must release what it just created, or the
        // URLs leak for the lifetime of the window.
        if (cancelled) {
          Object.values(blobUrls).forEach((url) => URL.revokeObjectURL(url));
          return;
        }
        urlsRef.current.forEach((url) => URL.revokeObjectURL(url));
        urlsRef.current = Object.values(blobUrls);
        setIcons(blobUrls);
      } catch (e) {
        console.warn("icons unavailable:", e);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [key]);

  // Release on unmount. Kept separate from the fetch effect so a change of
  // `key` doesn't revoke URLs the current render is still displaying.
  useEffect(
    () => () => {
      urlsRef.current.forEach((url) => URL.revokeObjectURL(url));
      urlsRef.current = [];
    },
    [],
  );

  return icons;
}
