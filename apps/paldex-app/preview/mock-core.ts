// Stands in for @tauri-apps/api/core so the real components can be driven with
// fixture data captured from the actual commands. Preview harness only.
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
    const res = await fetch(`/__fixture__/${name}.json`);
    if (res.ok) return (await res.json()) as T;
  }
  throw new Error(`no fixture for ${cmd}`);
}
