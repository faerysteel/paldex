import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import Roster from "../src/Roster";
import "../src/styles.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Roster
      summary={{ snapshotId: 1, palCount: 1955, playerCount: 2, takenAt: Date.now() }}
      onBack={() => {}}
      onSummaryChange={() => {}}
    />
  </StrictMode>,
);
