import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { MemoryRouter as Router, Route, Routes } from "react-router-dom";

import TitleBar from "./components/TitleBar";
import DiskList from "./components/DiskList";
import DiskDetail from "./components/DiskDetail";

import { platform } from "@tauri-apps/api/os";

function App() {
  const [isLinux, setIsLinux] = useState(false);
  useEffect(() => {
    platform().then((plat) => {
      if (plat === "linux") {
        setIsLinux(true);
      }
    });
  }, []);
  return (
    <Router>
      <div
        className={
          "flex flex-col justify-items-stretch	items-stretch min-h-full" +
          (isLinux ? " linuxbg" : "")
        }
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
