import React from "react";
import ReactDOM from "react-dom/client";
import "./i18n";
import "./styles/app.css";
import App from "./App";

// Disable the browser context menu outside text fields (the app has its own).
document.addEventListener("contextmenu", (e) => {
  const el = e.target as HTMLElement;
  if (!(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el.closest(".select-text"))) e.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
