// Stands in for @tauri-apps/plugin-dialog in the preview harness.
//
// This used to return null — "the user cancelled" — which made "Choose folder…"
// a silent no-op indistinguishable from a broken button. A browser tab has no
// native picker, so say so and let App surface it as a notice. Throwing at the
// picker also gives a truer message than letting the flow reach
// `resolve_folder`, which has no fixture and would blame the wrong step.
export async function open(_opts?: unknown): Promise<string | null> {
  throw new Error(
    "The preview harness has no folder picker — it renders captured fixtures. " +
      "Run the Tauri app to choose a folder.",
  );
}
