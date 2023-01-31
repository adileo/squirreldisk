import { invoke } from "@tauri-apps/api/tauri";
import prettyBytes from "pretty-bytes";
import { getIconForFolder } from "vscode-icons-js";
// import { iconImages } from "./iconImages";
import { buildFullPath } from "./pruneData";
interface ParentFolderProps {
  focusedDirectory: D3HierarchyDiskItem;
  d3Chart: any;
}
export const ParentFolder = ({
  focusedDirectory,
  d3Chart,
}: ParentFolderProps) => {
  const mul = window.OS_TYPE === "Windows_NT" ? 1024 : 1000;
  return (
    <div
      className="bg-gray-800 p-2 text-white flex justify-between rounded-md cursor-pointer"
      onContextMenu={(e) => {
        e.preventDefault();
        invoke("show_in_folder", { path: buildFullPath(focusedDirectory) });
      }}
      onClick={() => {
        if (focusedDirectory.parent)
          d3Chart.current.backToParent(focusedDirectory.parent);
        /*window.electron.diskUtils.openPath(buildFullPath(focusedDirectory));*/
      }}
    >
      <div className="">
        {/* {focusedDirectory && (
          <img
            src={
              iconImages[getIconForFolder(focusedDirectory.data.name)].default
            }
            className="h-6 w-6 mr-3"
          ></img>
        )} */}
      </div>
      <div className="truncate pr-6 flex-1 text-xs">
        {focusedDirectory &&
          buildFullPath(focusedDirectory)
            .replace("\\/", "/")
            .replace("\\", "/")}
      </div>
      <div className="text-xs">
        {focusedDirectory &&
          (focusedDirectory.data.value! / mul / mul / mul).toFixed(2)}{" "}
        GB
      </div>
    </div>
  );
};
