import { useState } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { MemoryRouter as Router, Route, Routes } from "react-router-dom";
import TitleBar from "./TitleBar";
import { DiskList } from "./DiskList";
import DiskDetail from "./DiskDetail";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  async function greet() {
    // Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
    setGreetMsg(await invoke("greet", { name }));
  }

  return (
    <Router>
      <div
        className="flex flex-col justify-items-stretch	items-stretch min-h-full"
        style={{ borderRadius: 20 }}
      >
        <TitleBar></TitleBar>
        <Routes>
          <Route path="/" element={<DiskList />} />
          <Route path="/disk" element={<DiskDetail />} />
        </Routes>
      </div>
    </Router>
  );
}

export default App;
