// Stands in for @tauri-apps/api/core so the real components can be driven with
// fixture data captured from the real commands. Preview harness only.
//
// Most commands take no arguments and map straight onto `<cmd>.json`.
// `breeding_options` takes a target species, and the dump captures a handful
// of them as `breeding_options__<target>.json`; anything else falls back to
// the plain capture, so picking an uncaptured target in the preview shows a
// real-shaped list rather than an error. Only the harness behaves this way —
// the real command answers for whatever target it is given.
export async function invoke<T>(cmd: string, args?: unknown): Promise<T> {
  const target = (args as { target?: unknown } | undefined)?.target;
  const names = typeof target === "string" ? [`${cmd}__${target}`, cmd] : [cmd];

  for (const name of names) {
    const fixture = await load<T>(name);
    if (fixture !== null) return fixture;
  }
  throw new Error(`no fixture for ${cmd}`);
}

/**
 * A fixture, or null when there isn't one.
 *
 * The content-type check is load-bearing: Vite serves the SPA's `index.html`
 * with a **200** for any path it doesn't recognise, so a missing fixture comes
 * back as HTML rather than a 404. Testing `res.ok` alone would hand `res.json()`
 * a document and fail with "Unexpected token '<'" instead of falling through to
 * the next candidate.
 */
async function load<T>(name: string): Promise<T | null> {
  const res = await fetch(`/__fixture__/${name}.json`);
  if (!res.ok) return null;
  if (!res.headers.get("content-type")?.includes("application/json")) return null;
  return (await res.json()) as T;
}
