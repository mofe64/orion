import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { resetAcknowledgementDrafts } from "./lib/drafts";
import "./styles.css";

// Run before React reads or autosaves a scene so old drafts cannot be restored.
try { resetAcknowledgementDrafts(); }
catch (error) { console.error("Could not reset the acknowledgement scene drafts.", error); }

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
