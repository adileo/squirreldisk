import { useEffect, useState } from "react";

import DiskItem from "./DiskItem";
import { invoke } from "@tauri-apps/api/tauri";

import { getVersion } from "@tauri-apps/api/app";

declare global {
  interface Window {
    electron: any;
    analytics: any;
    configStore: any;
    licver: any;
  }
}
export const DiskList = () => {
  const [disks, setDisks] = useState([]);
  const [appVersion, setAppVersion] = useState("1.0.0");

  useEffect(() => {
    getVersion().then((v) => setAppVersion(v));
    //   window.electron.app
    // setAppVersion(window.electron.appInfo().version)
  }, []);
  useEffect(() => {
    // window.electron.diskUtils.killDiskSizeWorker();

    const syncDisks = async () => {
      const disksString: string = await invoke("get_disks");
      const disks = JSON.parse(disksString);
      setDisks(disks);
    };
    const handle = setInterval(() => {
      syncDisks();
    }, 2000);
    syncDisks();
    return () => {
      clearInterval(handle);
    };
  }, []);
  useEffect(() => {
    var config = {
      selector: ".inject_here",
      account: "xYZ8B7",
    };
    window.Headway.init(config);
  }, []);
  return (
    <div className="flex-1 flex flex-col">
      <div className="text-white flex-1">
        {disks.map((disk: any) => (
          <DiskItem key={disk.sMountPoint} disk={disk}></DiskItem>
        ))}
      </div>
      <div className="p-4 text-white justify-between opacity-20 w-full flex">
        <div>Tip: Right Click for a full disk scan (slower)</div>
        <div>
          <div className="inline-block inject_here"></div> v. {appVersion}
        </div>
      </div>
    </div>
  );
};
