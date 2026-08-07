import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import ErrorBoundary from "./ErrorBoundary";
import "./styles.css";

window.addEventListener("error", (e) => console.error("[paldex] uncaught:", e.message, e.error));
window.addEventListener("unhandledrejection", (e) =>
  console.error("[paldex] unhandled rejection:", e.reason),
);

const root = document.getElementById("root");
if (!root) throw new Error("missing #root element");

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
