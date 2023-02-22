import { useEffect, useState } from "react";

import DiskItem from "./DiskItem";
import { invoke } from "@tauri-apps/api/tauri";

import { getVersion } from "@tauri-apps/api/app";
import { platform } from "@tauri-apps/api/os";
import { open } from "@tauri-apps/api/dialog";
import folderIcon from "../assets/folder.png";
import { useNavigate } from "react-router-dom";

declare global {
  interface Window {
    electron: any;
    analytics: any;
    configStore: any;
    licver: any;
  }
}

const DiskList = () => {
  const [disks, setDisks] = useState([]);
  const [appVersion, setAppVersion] = useState("1.0.0");
  const navigate = useNavigate();
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
      platform().then((plat) => {
        let filtered = disks.filter((disk: any) => {
          if (
            plat === "darwin" &&
            disk.sMountPoint === "/System/Volumes/Data"
          ) {
            return false; // Since it will be used /System/Volumes/Data
          }
          if (
            plat === "linux" &&
            disk.sMountPoint === "/var/snap/firefox/common/host-hunspell"
          ) {
            return false;
          }
          if (plat === "linux" && disk.sMountPoint === "/boot/efi") {
            return false;
          }
          return true;
        });
        setDisks(filtered);
      });
    };
    const handle = setInterval(syncDisks, 2000);
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
    if (window.Headway) {
      window.Headway.init(config);
    }
  }, []);
  return (
    <div className="flex-1 flex flex-col">
      <div className="text-white flex-1">
        {disks.map((disk: any) => (
          <DiskItem key={disk.sMountPoint} disk={disk}></DiskItem>
        ))}
        <div
          className="text-white p-4 flex gap-4 items-center hover:bg-gray-800 cursor-pointer"
          onClick={() => {
            open({
              multiple: false,
              directory: true,
            }).then((directory) => {
              if (directory)
                navigate("/disk", {
                  state: {
                    disk: (directory as string).replace(/\\/g, "/"),
                    used: 0,
                    fullscan: false,
                    isDirectory: true,
                  },
                });
              console.log({ directory });
            });
          }}
        >
          <div className="w-16 h-16 flex justify-center items-center align-middle">
            <img src={folderIcon} className="w-12 h-12 opacity-70"></img>
          </div>
          <div className="flex-1">
            <div className="flex justify-between mb-1">
              <span className="font-medium  text-white text-sm">
                Select a folder to Scan
                {/* <span className="opacity-60"></span> */}
              </span>
            </div>
          </div>
        </div>
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

export default DiskList;
