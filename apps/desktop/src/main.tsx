import React from "react";
import ReactDOM from "react-dom/client";
import { AppProvider } from "./providers/AppProvider";
import "./styles.css";

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("Root element not found");
ReactDOM.createRoot(rootEl).render(
  <React.StrictMode>
    <AppProvider />
  </React.StrictMode>,
);
