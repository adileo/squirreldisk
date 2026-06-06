import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import { useNavigate } from "react-router-dom";

import diskIcon from "../assets/harddisk.png";
import removableDriver from "../assets/removable-drive.png";
import { clearCachedScan, type ScanSnapshotSummary } from "../scanCache";
import { formatScannedAt } from "../scanTime";

type MenuAction = {
  label: string;
  onSelect: () => void;
  separatorBefore?: boolean;
  destructive?: boolean;
};

interface DiskItemProps {
  disk: any;
  hasScan: boolean;
  scanSnapshot?: ScanSnapshotSummary;
  onCacheChange?: () => void;
}

const DiskItem = ({
  disk,
  hasScan,
  scanSnapshot,
  onCacheChange,
}: DiskItemProps) => {
  const navigate = useNavigate();
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const x = [
    { tc: "text-green-700", bg: "bg-green-600", from: 0, to: 0.6 },
    { tc: "text-yellow-700", bg: "bg-yellow-600", from: 0.6, to: 0.7 },
    { tc: "text-red-700", bg: "bg-red-600", from: 0.7, to: 1 },
  ];

  useEffect(() => {
    if (!menuOpen) return;

    const closeMenu = (event: MouseEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };

    document.addEventListener("click", closeMenu);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("click", closeMenu);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [menuOpen]);

  const used = disk.totalSpace - disk.availableSpace;
  const perc = (disk.totalSpace - disk.availableSpace) / disk.totalSpace;
  const xy: any = x.find((e) => perc > e.from && perc <= e.to);

  const icona = disk.isRemovable ? removableDriver : diskIcon;
  const mul = window.OS_TYPE === "windows" ? 1024 : 1000;
  const scannedAtText = formatScannedAt(scanSnapshot?.scannedAt);

  const openDisk = (forceScan = false) => {
    setMenuOpen(false);
    navigate("/disk", {
      state: {
        disk: disk.sMountPoint,
        used,
        forceScan,
      },
    });
  };

  const rescan = () => {
    openDisk(true);
  };

  const forgetScan = async () => {
    await clearCachedScan(disk.sMountPoint);
    onCacheChange?.();
    setMenuOpen(false);
  };

  const showInFinder = () => {
    setMenuOpen(false);
    invoke("show_in_folder", { path: disk.sMountPoint }).catch(console.error);
  };

  const stopRowClick = (event: ReactMouseEvent) => {
    event.stopPropagation();
  };

  const stopRowContextMenu = (event: ReactMouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
  };

  const actions: MenuAction[] = hasScan
    ? [
        { label: "Rescan", onSelect: rescan },
        {
          label: "Forget Scan Result",
          onSelect: forgetScan,
          separatorBefore: true,
          destructive: true,
        },
        {
          label: "Show in Finder",
          onSelect: showInFinder,
          separatorBefore: true,
        },
      ]
    : [
        {
          label: "Show in Finder",
          onSelect: showInFinder,
        },
      ];

  return (
    <div
      onContextMenu={(e) => {
        e.preventDefault();
        setMenuOpen(true);
      }}
      onClick={() => {
        openDisk();
      }}
      className="text-white p-4 flex gap-4 items-center hover:bg-gray-800 cursor-pointer"
      data-testid="disk-row"
      data-disk-path={disk.sMountPoint}
    >
      <img src={icona} className="w-16 h-16"></img>
      <div className="flex-1">
        <div className="flex justify-between mb-1">
          <span className="font-medium  text-white">
            {disk.name ? disk.name : "Local Disk"}{" "}
            <span className="text-xs">({disk.sMountPoint})</span>
            <br />
            <span className=" text-sm font-medium mr-2 px-2.5 py-0.5 rounded bg-gray-700 text-gray-300">
              {(disk.totalSpace / mul / mul / mul).toFixed(1)} GB
            </span>
            {scannedAtText && (
              <span className="mt-1 block text-xs font-normal text-gray-400">
                {scannedAtText}
              </span>
            )}
            {/* <span className="opacity-60"></span> */}
          </span>
          <div className="text-sm font-medium text-right text-white">
            <div
              ref={menuRef}
              className="relative inline-flex text-xs font-semibold text-gray-100"
              onClick={stopRowClick}
              onContextMenu={stopRowContextMenu}
            >
              <button
                type="button"
                className="rounded-l bg-gray-700 px-3 py-1 hover:bg-gray-600"
                onClick={() => openDisk()}
              >
                {hasScan ? "View" : "Scan"}
              </button>
              <button
                type="button"
                aria-label="More disk actions"
                aria-expanded={menuOpen}
                aria-haspopup="menu"
                className="rounded-r border-l border-gray-600 bg-gray-700 px-1.5 py-1 hover:bg-gray-600"
                onClick={() => setMenuOpen((open) => !open)}
              >
                <svg
                  className="h-3 w-3"
                  viewBox="0 0 12 12"
                  fill="currentColor"
                  aria-hidden="true"
                >
                  <path d="M2.2 4.2h7.6L6 8 2.2 4.2Z" />
                </svg>
              </button>
              {menuOpen && (
                <div
                  role="menu"
                  className="absolute right-0 top-full z-30 mt-2 w-44 overflow-hidden rounded-md border border-gray-700 bg-gray-900 py-1 text-left shadow-2xl"
                >
                  {actions.map((action) => (
                    <div key={action.label}>
                      {action.separatorBefore && (
                        <div className="my-1 border-t border-gray-700" />
                      )}
                      <button
                        type="button"
                        role="menuitem"
                        className={`block w-full px-3 py-2 text-left text-xs font-semibold hover:bg-gray-800 ${
                          action.destructive ? "text-rose-200" : "text-gray-100"
                        }`}
                        onClick={action.onSelect}
                      >
                        {action.label}
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <br />
            <span className="mt-2 inline-block">{(perc * 100).toFixed(0)}%</span>
            <br />
            <span className="opacity-60">
              {(disk.availableSpace / mul / mul / mul).toFixed(1)} GB Free
            </span>
          </div>
        </div>
        <div className="w-full mt-2 bg-gray-700 rounded-full h-2.5">
          {xy && (
            <div
              className={"h-2.5 rounded-full " + xy.bg}
              style={{ width: perc * 100 + "%" }}
            ></div>
          )}
        </div>
      </div>
    </div>
  );
};

export default DiskItem;
