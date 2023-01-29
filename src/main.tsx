import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./style.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  // <React.StrictMode> commenting since react-beautiful-dnd crash with this issue: https://github.com/atlassian/react-beautiful-dnd/issues/2396
  <App />
  // </React.StrictMode>
);
