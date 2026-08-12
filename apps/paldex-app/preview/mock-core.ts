// Stands in for @tauri-apps/api/core so the real components can be driven with
// fixture data captured from the real commands. Preview harness only.
//
// Most commands take no arguments and map straight onto `<cmd>.json`.
// `breeding_options` takes a target species and is captured per target, as
// `breeding_options__<target>.json`. A target with no file is one the owned
// roster cannot breed, so its absence resolves to the empty list the real
// command returns — deliberately *not* a fallback to some other target's
// capture, which would show one species' pairs under another species' name.
// A selected player adds a `__player_<uid>` suffix, captured only when the
// dump ran with PALDEX_FIXTURE_PLAYERS=1. Without it `world_players` is empty,
// so the selector never renders and nothing ever asks for a scoped fixture.
export async function invoke<T>(cmd: string, args?: unknown): Promise<T> {
  const { target, playerUid } = (args ?? {}) as {
    target?: unknown;
    playerUid?: unknown;
  };
  const suffix = typeof playerUid === "string" ? `__player_${playerUid}` : "";

  if (typeof target === "string") {
    const pairs = await load<T>(`${cmd}__${target}${suffix}`);
    return pairs ?? ([] as unknown as T);
  }

  const fixture = await load<T>(`${cmd}${suffix}`);
  if (fixture === null) throw new Error(`no fixture for ${cmd}${suffix}`);
  return fixture;
}

/**
 * A fixture, or null when there isn't one.
 *
 * The content-type check is load-bearing: Vite serves the SPA's `index.html`
 * with a **200** for any path it doesn't recognise, so a missing fixture comes
 * back as HTML rather than a 404. Testing `res.ok` alone would hand `res.json()`
 * a document and fail with "Unexpected token '<'".
 */
async function load<T>(name: string): Promise<T | null> {
  const res = await fetch(`/__fixture__/${name}.json`);
  if (!res.ok) return null;
  if (!res.headers.get("content-type")?.includes("application/json")) return null;
  return (await res.json()) as T;
}
