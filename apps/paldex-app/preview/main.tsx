import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "../src/App";
import ErrorBoundary from "../src/ErrorBoundary";
import "../src/styles.css";

// Renders the whole App, so world selection — the transition that took the UI
// down — is exercised, not just the roster in isolation.
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
