// Stands in for @tauri-apps/api/event so components that subscribe to backend
// events render in the harness. Preview harness only.
//
// Nothing ever emits here: the harness has no Rust side, so the live-sync
// listener simply stays idle. That is the point — the subscription must not be
// what breaks the screen when no events arrive.
export async function listen<T>(
  _event: string,
  _handler: (event: { payload: T }) => void,
): Promise<() => void> {
  return () => {};
}
