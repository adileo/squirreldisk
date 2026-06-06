import Logo from "../assets/squirrel.png";
import Close from "../assets/Close.svg";
import { Link, useLocation } from "react-router-dom";
import { Platform, platform } from "@tauri-apps/plugin-os";
import { useEffect, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
const appWindow = getCurrentWebviewWindow();

const normalizePath = (path: string) => {
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
  return normalized || "/";
};

const pathParts = (path: string) =>
  normalizePath(path)
    .split("/")
    .filter(Boolean);

const relativePathParts = (rootPath: string, focusedPath: string) => {
  const root = normalizePath(rootPath);
  const focused = normalizePath(focusedPath);

  if (root === focused) {
    return [];
  }

  if (root === "/") {
    return pathParts(focused);
  }

  if (focused.startsWith(`${root}/`)) {
    return focused.slice(root.length + 1).split("/").filter(Boolean);
  }

  return pathParts(focused);
};

const breadcrumbPaths = (rootPath: string, focusedPath: string) => {
  const root = normalizePath(rootPath);
  const parts = relativePathParts(root, focusedPath);

  return parts.map((label, index) => ({
    label,
    path:
      root === "/"
        ? `/${parts.slice(0, index + 1).join("/")}`
        : `${root}/${parts.slice(0, index + 1).join("/")}`,
  }));
};

const Separator = () => (
  <span
    data-tauri-drag-region
    className="px-1 text-sm font-medium text-gray-600"
    aria-hidden="true"
  >
    /
  </span>
);

const breadcrumbLinkClass =
  "block truncate text-sm font-medium text-gray-400 hover:text-white";
const breadcrumbCurrentClass =
  "block truncate text-sm font-medium text-gray-300";

const CloseButton = () => {
  return (
    <button
      onClick={() => {
        appWindow.close();
      }}
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        className="h-6 w-6"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M6 18L18 6M6 6l12 12"
        />
      </svg>
    </button>
  );
};
const TitleBar = () => {
  let { state, pathname } = useLocation() as any;
  const routeState = state || {};
  const diskPath = routeState.disk ? normalizePath(routeState.disk) : "";
  const focusedPath = normalizePath(routeState.focusedPath || diskPath || "/");
  const pathCrumbs =
    pathname === "/disk" && diskPath ? breadcrumbPaths(diskPath, focusedPath) : [];
  const [plf, setPlf] = useState<Platform | undefined>();

  useEffect(() => {
    setPlf(platform());
  }, []);
  return (
    <div
      data-tauri-drag-region
      className="flex bg-darkBlue h-70 justify-between w-full items-center pl-3 pr-3 titlebar bg-cyan-800 p-2 text-white"
      style={{ background: "#0F1831" }}
    >
      {plf !== "macos" ? (
        <Link to="/" aria-label="Home">
          <img src={Logo} className="h-6 w-6" />
        </Link>
      ) : (
        <CloseButton></CloseButton>
      )}
      <div
        data-tauri-drag-region
        className="min-w-0 max-w-[calc(100%-7rem)] overflow-hidden px-3 font-bold"
      >
        <nav
          data-tauri-drag-region
          className="min-w-0 overflow-hidden navi"
          aria-label="Breadcrumb"
          data-testid="title-breadcrumb"
        >
          <ol
            data-tauri-drag-region
            className="flex min-w-0 items-center overflow-hidden"
          >
            <li data-tauri-drag-region className="inline-flex items-center">
              <Link
                to="/"
                data-testid="title-breadcrumb-all-disks"
                className="text-sm font-medium text-gray-400 hover:text-white"
              >
                All Disks
              </Link>
            </li>

            {pathname == "/disk" && diskPath && (
              <li data-tauri-drag-region>
                <div data-tauri-drag-region className="flex min-w-0 items-center">
                  <Separator />
                  <Link
                    to="/disk"
                    state={{
                      ...routeState,
                      focusedPath: diskPath,
                    }}
                    replace
                    data-testid="title-breadcrumb-root"
                    data-path={diskPath}
                    className={`${breadcrumbLinkClass} max-w-[14rem]`}
                    title={diskPath}
                  >
                    {routeState.isDirectory ? "Folder" : "Disk"} ({diskPath})
                  </Link>
                </div>
              </li>
            )}
            {pathCrumbs.map((crumb, index) => {
              const isCurrent = index === pathCrumbs.length - 1;
              return (
                <li
                  key={crumb.path}
                  data-tauri-drag-region
                  className="flex min-w-0 items-center"
                  aria-current={isCurrent ? "page" : undefined}
                >
                  <Separator />
                  {isCurrent ? (
                    <span
                      data-tauri-drag-region
                      data-testid="title-breadcrumb-current"
                      data-path={crumb.path}
                      className={`${breadcrumbCurrentClass} max-w-[10rem]`}
                      title={crumb.path}
                    >
                      {crumb.label}
                    </span>
                  ) : (
                    <Link
                      to="/disk"
                      state={{
                        ...routeState,
                        focusedPath: crumb.path,
                      }}
                      replace
                      data-testid="title-breadcrumb-segment"
                      data-path={crumb.path}
                      className={`${breadcrumbLinkClass} max-w-[10rem]`}
                      title={crumb.path}
                    >
                      {crumb.label}
                    </Link>
                  )}
                </li>
              );
            })}
            {pathname == "/settings" && (
              <li
                data-tauri-drag-region
                className="flex min-w-0 items-center"
                aria-current="page"
              >
                <Separator />
                <span data-tauri-drag-region className={breadcrumbCurrentClass}>
                  Settings
                </span>
              </li>
            )}
          </ol>
        </nav>
      </div>
      <div className="flex items-center gap-2">
        <Link
          to="/settings"
          aria-label="Settings"
          className="text-gray-400 hover:text-white"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            className="h-5 w-5"
            fill="currentColor"
            viewBox="0 0 24 24"
          >
            <path d="M19.14 12.94a7.92 7.92 0 0 0 .05-.94 7.92 7.92 0 0 0-.05-.94l2.03-1.58a.5.5 0 0 0 .12-.64l-1.92-3.32a.5.5 0 0 0-.61-.22l-2.39.96a7.78 7.78 0 0 0-1.62-.94l-.36-2.54a.5.5 0 0 0-.5-.42h-3.84a.5.5 0 0 0-.5.42l-.36 2.54a7.78 7.78 0 0 0-1.62.94l-2.39-.96a.5.5 0 0 0-.61.22L2.65 8.84a.5.5 0 0 0 .12.64l2.03 1.58a7.92 7.92 0 0 0 0 1.88l-2.03 1.58a.5.5 0 0 0-.12.64l1.92 3.32a.5.5 0 0 0 .61.22l2.39-.96a7.78 7.78 0 0 0 1.62.94l.36 2.54a.5.5 0 0 0 .5.42h3.84a.5.5 0 0 0 .5-.42l.36-2.54a7.78 7.78 0 0 0 1.62-.94l2.39.96a.5.5 0 0 0 .61-.22l1.92-3.32a.5.5 0 0 0-.12-.64l-2.03-1.58ZM12 15.5A3.5 3.5 0 1 1 12 8.5a3.5 3.5 0 0 1 0 7Z" />
          </svg>
        </Link>
        {plf !== "macos" ? (
          <CloseButton></CloseButton>
        ) : (
          <Link to="/" aria-label="Home">
            <img src={Logo} className="h-6 w-6" />
          </Link>
        )}
      </div>
    </div>
  );
};

export default TitleBar;
