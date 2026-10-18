import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { Peek } from "./components/Peek";
import { SettingsWindow } from "./components/Settings";

// The same bundle serves the sidebar, its "peek" window (collapsed-strip hover card) and the
// settings window.
const view = new URLSearchParams(window.location.search).get("view");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {view === "peek" ? <Peek /> : view === "settings" ? <SettingsWindow /> : <App />}
  </React.StrictMode>,
);
