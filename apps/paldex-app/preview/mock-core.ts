// Stands in for @tauri-apps/api/core so the real components can be driven with
// fixture data captured from the actual commands. Preview harness only.
export async function invoke<T>(cmd: string, _args?: unknown): Promise<T> {
  const res = await fetch(`/__fixture__/${cmd}.json`);
  if (!res.ok) throw new Error(`no fixture for ${cmd}`);
  return (await res.json()) as T;
}
