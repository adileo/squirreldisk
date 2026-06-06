import { useEffect, useState } from "react";

import DiskItem from "./DiskItem";
import { invoke } from "@tauri-apps/api/core";

import { getVersion } from "@tauri-apps/api/app";
import { platform } from "@tauri-apps/plugin-os";
import { open } from "@tauri-apps/plugin-dialog";
import folderIcon from "../assets/folder.png";
import { useNavigate } from "react-router-dom";
import {
  hasCachedScan,
  listScanSnapshots,
  type ScanSnapshotSummary,
} from "../scanCache";

declare global {
  interface Window {
    electron: any;
    configStore: any;
    licver: any;
  }
}

const DiskList = () => {
  const [disks, setDisks] = useState([]);
  const [appVersion, setAppVersion] = useState("1.0.0");
  const [snapshotByPath, setSnapshotByPath] = useState<
    Map<string, ScanSnapshotSummary>
  >(new Map());
  const [, setCacheRevision] = useState(0);
  const navigate = useNavigate();
  const handleCacheChange = () => {
    setCacheRevision((revision) => revision + 1);
    refreshSnapshotPaths();
  };
  const refreshSnapshotPaths = () => {
    listScanSnapshots()
      .then((snapshots) =>
        setSnapshotByPath(
          new Map(snapshots.map((snapshot) => [snapshot.path, snapshot]))
        )
      )
      .catch(console.error);
  };
  useEffect(() => {
    getVersion().then((v) => setAppVersion(v));
    refreshSnapshotPaths();
    //   window.electron.app
    // setAppVersion(window.electron.appInfo().version)
  }, []);

  useEffect(() => {
    // window.electron.diskUtils.killDiskSizeWorker();
    const syncDisks = async () => {
      try {
        const disksString: string = await invoke("get_disks");
        const disks = JSON.parse(disksString);
        const plat = platform();
        let filtered = disks.filter((disk: any) => {
          if (plat === "macos" && disk.sMountPoint === "/System/Volumes/Data") {
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
      } catch (error) {
        console.error("Failed to sync disks:", error);
      }
    };
    const handle = setInterval(syncDisks, 2000);
    syncDisks();
    return () => {
      clearInterval(handle);
    };
  }, []);

  return (
    <div className="flex-1 flex flex-col">
      <div className="text-white flex-1">
        {disks.map((disk: any) => {
          const snapshot = snapshotByPath.get(disk.sMountPoint);
          return (
            <DiskItem
              key={disk.sMountPoint}
              disk={disk}
              hasScan={hasCachedScan(disk.sMountPoint) || !!snapshot}
              scanSnapshot={snapshot}
              onCacheChange={handleCacheChange}
            ></DiskItem>
          );
        })}
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
      <div className="p-4 text-white justify-end opacity-20 w-full flex">
        <div>v. {appVersion}</div>
      </div>
    </div>
  );
};

export default DiskList;
