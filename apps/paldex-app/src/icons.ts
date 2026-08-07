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
 *   the webview's per-URL image cache.
 *
 * Artwork is optional everywhere it is used: a failure logs and yields no
 * icons rather than propagating, so a missing pak never blanks a screen.
 */
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
        await Promise.all(
          Object.entries(dataUrls).map(async ([id, dataUrl]) => {
            blobUrls[id] = URL.createObjectURL(await (await fetch(dataUrl)).blob());
          }),
        );

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
