import { invoke } from "@tauri-apps/api/core";
import { buildFullPath } from "../pruneData";
interface ParentFolderProps {
  focusedDirectory: D3HierarchyDiskItem;
  isPreview?: boolean;
  onFocusDirectory: (node: D3HierarchyDiskItem) => void;
}
export const ParentFolder = ({
  focusedDirectory,
  isPreview = false,
  onFocusDirectory,
}: ParentFolderProps) => {
  const mul = window.OS_TYPE === "windows" ? 1024 : 1000;
  return (
    <div
      className="bg-gray-800 p-2 text-white flex justify-between rounded-md cursor-pointer"
      onContextMenu={(e) => {
        e.preventDefault();
        invoke("show_in_folder", { path: buildFullPath(focusedDirectory) });
      }}
      onClick={() => {
        if (isPreview) {
          onFocusDirectory(focusedDirectory);
        } else if (focusedDirectory.parent) {
          onFocusDirectory(focusedDirectory.parent);
        }
        /*window.electron.diskUtils.openPath(buildFullPath(focusedDirectory));*/
      }}
      data-testid="sidebar-directory"
      data-directory-id={focusedDirectory.data.id}
      data-preview={isPreview ? "true" : "false"}
    >
      <div className="truncate pr-6 flex-1 text-xs">
        {focusedDirectory &&
          buildFullPath(focusedDirectory)
            .replace("\\/", "/")
            .replace("\\", "/")}
      </div>
      <div className="shrink-0 text-right text-xs">
        <div className="text-[10px] uppercase tracking-wide text-gray-500">
          Allocated
        </div>
        <div>
          {focusedDirectory &&
            (focusedDirectory.data.value! / mul / mul / mul).toFixed(2)}{" "}
          GB
        </div>
      </div>
    </div>
  );
};
