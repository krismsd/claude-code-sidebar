import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { Peek } from "./components/Peek";

// The same bundle serves the sidebar and its "peek" window (collapsed-strip hover card).
const peek = new URLSearchParams(window.location.search).get("view") === "peek";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{peek ? <Peek /> : <App />}</React.StrictMode>,
);
