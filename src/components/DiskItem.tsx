import { Link } from "react-router-dom";
import diskIcon from "../assets/harddisk.png";
import removableDriver from "../assets/removable-drive.png";

import { useNavigate } from "react-router-dom";

const DiskItem = ({ disk }: any) => {
  const navigate = useNavigate();
  const x = [
    { tc: "text-green-700", bg: "bg-green-600", from: 0, to: 0.6 },
    { tc: "text-yellow-700", bg: "bg-yellow-600", from: 0.6, to: 0.7 },
    { tc: "text-red-700", bg: "bg-red-600", from: 0.7, to: 1 },
  ];

  const perc = (disk.totalSpace - disk.availableSpace) / disk.totalSpace;
  const xy: any = x.find((e) => perc > e.from && perc <= e.to);

  const icona = disk.isRemovable ? removableDriver : diskIcon;
  const mul = window.OS_TYPE === "Windows_NT" ? 1024 : 1000;

  return (
    <div
      onContextMenu={(e) => {
        e.preventDefault();
        navigate("/disk", {
          state: {
            disk: disk.sMountPoint,
            used: disk.totalSpace - disk.availableSpace,
            fullscan: true,
          },
        });
      }}
      onClick={() => {
        navigate("/disk", {
          state: {
            disk: disk.sMountPoint,
            used: disk.totalSpace - disk.availableSpace,
            fullscan: false,
          },
        });
      }}
      className="text-white p-4 flex gap-4 items-center hover:bg-gray-800 cursor-pointer"
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
            {/* <span className="opacity-60"></span> */}
          </span>
          <span className="text-sm font-medium text-right text-white">
            {(perc * 100).toFixed(0)}%<br />
            <span className="opacity-60">
              {(disk.availableSpace / mul / mul / mul).toFixed(1)} GB Free
            </span>
          </span>
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
