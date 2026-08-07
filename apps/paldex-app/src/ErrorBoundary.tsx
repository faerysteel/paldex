import { Component, type ErrorInfo, type ReactNode } from "react";

/**
 * A render error anywhere below here would otherwise unmount the whole tree,
 * leaving an empty (black) window with no clue what happened. Show the error
 * instead — a visible message is always more useful than a blank screen.
 */
export default class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[paldex] render error:", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="panel" style={{ padding: "1rem" }}>
        <h2>Something broke while rendering</h2>
        <pre style={{ whiteSpace: "pre-wrap" }}>{String(this.state.error?.stack ?? this.state.error)}</pre>
      </div>
    );
  }
}
