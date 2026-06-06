import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useNavigate, useLocation } from "react-router-dom";
import diskIcon from "../assets/harddisk.png";
import { getChart } from "../d3chart";
import * as d3 from "d3";
import prettyBytes from "pretty-bytes";
import {
  buildFullPath,
  diskItemToD3Hierarchy,
  addRestrictedPathsToTree,
  groupChildrenByBasePath,
  itemMap,
} from "../pruneData";
import { FileLine } from "./FileLine";
import { ParentFolder } from "./ParentFolder";
import { DragDropContext, Droppable } from "react-beautiful-dnd";
import { invoke } from "@tauri-apps/api/core";
import { confirm } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getChartColor } from "../chartColors";
import { getCachedScan, setCachedScan } from "../scanCache";
import { formatScannedAt } from "../scanTime";
import { removeNodesFromTree } from "../treeMutations";

(window as any).LockDNDEdgeScrolling = () => true;

interface ScanStatus {
  items: number;
  total: number;
  errors: number;
}

interface ScanFailure {
  path: string;
  message: string;
}

interface PrivacyAccessStatus {
  platform: string;
  hasAccess: boolean;
  canElevate: boolean;
  label: string;
}

interface DeleteFailure {
  id: string;
  name: string;
  path: string;
  message: string;
}

interface DeleteState {
  isDeleting: boolean;
  isCountingDown: boolean;
  countdown: number | null;
  total: number;
  current: number;
  recoveredBytes: number;
  failures: Array<DeleteFailure>;
}

const emptyDeleteState: DeleteState = {
  isDeleting: false,
  isCountingDown: false,
  countdown: null,
  total: 0,
  current: 0,
  recoveredBytes: 0,
  failures: [],
};

interface DeleteOutcome {
  deletedBytes: number;
}

interface ContextMenuState {
  x: number;
  y: number;
  node: D3HierarchyDiskItem;
}

interface NodeContextMenuEvent {
  clientX: number;
  clientY: number;
  preventDefault: () => void;
  stopPropagation: () => void;
}

const DELETE_COUNTDOWN_SECONDS = 5;
const CONTEXT_MENU_WIDTH = 250;
const CONTEXT_MENU_HEIGHT = 170;

const normalizePathText = (path: string) => {
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
  return normalized || "/";
};

const normalizeNodePath = (node: D3HierarchyDiskItem) =>
  normalizePathText(buildFullPath(node));

const findNodeByFullPath = (
  root: D3HierarchyDiskItem,
  path: string
): D3HierarchyDiskItem | null => {
  const normalizedPath = normalizePathText(path);
  return (
    root
      .descendants()
      .find((node) => normalizeNodePath(node) === normalizedPath) || null
  );
};

const parentPath = (path: string) => {
  const normalized = normalizePathText(path);
  if (normalized === "/") {
    return "/";
  }

  const index = normalized.lastIndexOf("/");
  return index <= 0 ? "/" : normalized.slice(0, index);
};

const findNearestDirectoryByFullPath = (
  root: D3HierarchyDiskItem,
  path: string
): D3HierarchyDiskItem => {
  const rootPath = normalizeNodePath(root);
  let candidatePath = normalizePathText(path);

  while (true) {
    const candidate = findNodeByFullPath(root, candidatePath);
    if (isDirectoryNode(candidate)) {
      return candidate;
    }

    if (
      candidatePath === rootPath ||
      (rootPath !== "/" && !candidatePath.startsWith(`${rootPath}/`))
    ) {
      return root;
    }

    const nextPath = parentPath(candidatePath);
    if (nextPath === candidatePath) {
      return root;
    }
    candidatePath = nextPath;
  }
};

const isDirectoryNode = (
  node: D3HierarchyDiskItem | null
): node is D3HierarchyDiskItem =>
  !!node && (!!node.data.isDirectory || !!node.children);

const isSyntheticNode = (node: D3HierarchyDiskItem | null) =>
  !!node?.data.synthetic || !!node?.data.id.includes("__smaller_items_");

const isAncestorNode = (
  ancestor: D3HierarchyDiskItem,
  node: D3HierarchyDiskItem
) =>
  node
    .ancestors()
    .slice(1)
    .some((candidate) => candidate.data.id === ancestor.data.id);

const wait = (ms: number) =>
  new Promise((resolve) => window.setTimeout(resolve, ms));

const formatElapsed = (seconds: number) => {
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = Math.floor(seconds % 60);

  if (minutes === 0) {
    return `${remainingSeconds}s`;
  }

  return `${minutes}m ${remainingSeconds.toString().padStart(2, "0")}s`;
};

const formatScanRate = (bytes: number, seconds: number) => {
  if (bytes <= 0 || seconds <= 0) {
    return "0 B/s";
  }

  return `${prettyBytes(bytes / seconds)}/s`;
};

const formatProgressPercent = (percent: number) =>
  `${percent < 10 ? percent.toFixed(1) : percent.toFixed(0)}%`;

const Scanning = () => {
  const location = useLocation() as any;
  const routeState = location.state || {};
  const { disk, used, forceScan, focusedPath: requestedFocusedPath } = routeState;
  const navigate = useNavigate();

  const svgRef = useRef<SVGSVGElement | null>(null);

  // Original Data
  const baseData = useRef<DiskItem | null>(null);
  // D3 Hierarchy Data
  const baseDataD3Hierarchy = useRef<D3HierarchyDiskItem | null>(null);

  // Current Directory
  const [focusedDirectory, setFocusedDirectory] =
    useState<D3HierarchyDiskItem | null>(null);
  const [previewDirectory, setPreviewDirectory] =
    useState<D3HierarchyDiskItem | null>(null);
  // Hovered Item
  const [hoveredItem, setHoveredItem] = useState<DiskItem | null>(null);

  const d3Chart = useRef(null) as any;
  const [view, setView] = useState("loading");
  const [status, setStatus] = useState<ScanStatus>();
  const [scanError, setScanError] = useState<string | null>(null);
  const [restrictedPaths, setRestrictedPaths] = useState<Array<RestrictedPath>>(
    []
  );
  const restrictedPathsRef = useRef<Array<RestrictedPath>>([]);
  const scanStartedAt = useRef(performance.now());
  const scannedAtRef = useRef<number | null>(null);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const [scannedAt, setScannedAt] = useState<number | null>(null);
  const [deleteState, setDeleteState] =
    useState<DeleteState>(emptyDeleteState);
  const [privacyStatus, setPrivacyStatus] =
    useState<PrivacyAccessStatus | null>(null);

  const [deleteList, setDeleteList] = useState<Array<D3HierarchyDiskItem>>([]);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const deleteMap = useRef<Map<string, boolean>>(new Map());
  const deleteListRef = useRef<Array<D3HierarchyDiskItem>>([]);
  const cancelDeleteRef = useRef(false);
  const updateScannedAt = (value: number | null) => {
    scannedAtRef.current = value;
    setScannedAt(value);
  };
  const updateRouteFocusedPath = (directory: D3HierarchyDiskItem) => {
    const focusedPath = normalizeNodePath(directory);
    if (normalizePathText(routeState.focusedPath || "") === focusedPath) {
      return;
    }

    navigate("/disk", {
      replace: true,
      state: {
        ...routeState,
        disk,
        used,
        forceScan,
        focusedPath,
      },
    });
  };
  const setFocusedDirectoryNode = (
    directory: D3HierarchyDiskItem,
    updateRoute = true
  ) => {
    setFocusedDirectory(directory);
    if (updateRoute) {
      updateRouteFocusedPath(directory);
    }
  };
  const focusDirectory = (directory: D3HierarchyDiskItem) => {
    clearPreviewDirectory();
    setFocusedDirectoryNode(directory);
    d3Chart.current?.focusDirectory(directory);
  };
  const hoverListItem = (item: D3HierarchyDiskItem) => {
    setHoveredItem({ ...item.data });
    d3Chart.current?.setHoveredNode(item);
  };
  const clearPreviewDirectory = () => {
    setPreviewDirectory(null);
  };
  const clearHoveredItem = () => {
    setHoveredItem(null);
    d3Chart.current?.setHoveredNode(null);
  };
  const syncDeleteMap = (nodes: Array<D3HierarchyDiskItem>) => {
    deleteMap.current = new Map(nodes.map((node) => [node.data.id, true]));
  };
  const setCollectorNodes = (nodes: Array<D3HierarchyDiskItem>) => {
    deleteListRef.current = nodes;
    syncDeleteMap(nodes);
    setDeleteList(nodes);
  };
  const getCollectorBlockReason = (
    node: D3HierarchyDiskItem,
    selected = deleteList
  ) => {
    if (deleteState.isDeleting) {
      return "Deletion in progress";
    }
    if (isSyntheticNode(node)) {
      return "Smaller Items is a summary, not a single path";
    }
    if (node.data.restricted) {
      return "Restricted items cannot be collected";
    }
    if (!node.parent) {
      return "Top-level item cannot be collected";
    }
    if (selected.some((entry) => entry.data.id === node.data.id)) {
      return "Already in Collector";
    }
    if (selected.some((entry) => isAncestorNode(entry, node))) {
      return "Parent in Collector";
    }

    return null;
  };
  const addNodeToCollector = (node: D3HierarchyDiskItem) => {
    if (getCollectorBlockReason(node)) {
      return;
    }

    cancelDeleteRef.current = false;
    setDeleteState(emptyDeleteState);
    if (getCollectorBlockReason(node, deleteListRef.current)) {
      return;
    }

    const next = deleteListRef.current.filter(
      (entry) => !isAncestorNode(node, entry)
    );
    setCollectorNodes([...next, node]);
  };
  const showNodeInFolder = (node: D3HierarchyDiskItem) => {
    if (isSyntheticNode(node)) {
      return;
    }
    invoke("show_in_folder", { path: normalizeNodePath(node) }).catch(
      console.error
    );
  };
  const openNodeInTerminal = (node: D3HierarchyDiskItem) => {
    if (isSyntheticNode(node)) {
      return;
    }
    invoke("open_terminal", { path: normalizeNodePath(node) }).catch(
      console.error
    );
  };
  const openNodeContextMenu = (
    event: NodeContextMenuEvent,
    node: D3HierarchyDiskItem
  ) => {
    event.preventDefault();
    event.stopPropagation();
    setHoveredItem({ ...node.data });
    d3Chart.current?.setHoveredNode(node);
    setContextMenu({
      x: event.clientX + 8,
      y: event.clientY + 8,
      node,
    });
  };
  const addRestrictedPath = (restrictedPath: RestrictedPath) => {
    const normalizedPath = restrictedPath.path.replace(/\\/g, "/");
    if (
      restrictedPathsRef.current.some((entry) => entry.path === normalizedPath)
    ) {
      return;
    }

    const next = [
      ...restrictedPathsRef.current,
      { ...restrictedPath, path: normalizedPath },
    ];
    restrictedPathsRef.current = next;
    setRestrictedPaths(next);
  };

  // Avvio il worker e attendo i dati
  useEffect(() => {
    if (baseData.current) {
      // Skip if already loaded data
      return;
    }
    let cancelled = false;
    let scanStarted = false;
    let timer: number | undefined;
    const unlisteners: Array<ReturnType<typeof listen>> = [];

    const startScan = async () => {
      const cached = forceScan ? null : await getCachedScan(disk);
      if (cancelled) {
        return;
      }

      if (cached) {
        restrictedPathsRef.current = cached.restrictedPaths || [];
        setRestrictedPaths(cached.restrictedPaths || []);
        updateScannedAt(cached.scannedAt || null);
        baseData.current = cached.tree;
        baseDataD3Hierarchy.current = diskItemToD3Hierarchy(cached.tree);
        setView("disk");
        return;
      }

      restrictedPathsRef.current = [];
      setRestrictedPaths([]);
      setScanError(null);
      updateScannedAt(null);
      scanStartedAt.current = performance.now();
      timer = window.setInterval(() => {
        setElapsedSeconds((performance.now() - scanStartedAt.current) / 1000);
      }, 500);

      unlisteners.push(
        listen("scan_status", (event: any) => {
          setStatus(event.payload as ScanStatus);
          setElapsedSeconds((performance.now() - scanStartedAt.current) / 1000);
        })
      );

      unlisteners.push(
        listen("scan_restricted_path", (event: any) => {
          addRestrictedPath(event.payload as RestrictedPath);
        })
      );

      unlisteners.push(
        listen("scan_failed", (event: any) => {
          const failure = event.payload as ScanFailure;
          setScanError(failure.message);
          setElapsedSeconds((performance.now() - scanStartedAt.current) / 1000);
        })
      );

      unlisteners.push(
        listen("scan_completed", (event: any) => {
          try {
            const parsed = JSON.parse(event.payload);
            const tree =
              parsed.tree?.name === "(total)"
                ? groupChildrenByBasePath(parsed.tree, disk)
                : parsed.tree;
            const treeWithRestrictedPaths = addRestrictedPathsToTree(
              tree,
              disk,
              restrictedPathsRef.current
            );
            const mapped = itemMap(treeWithRestrictedPaths);
            const nextScannedAt = Date.now();
            updateScannedAt(nextScannedAt);
            setCachedScan({
              path: disk,
              tree: mapped,
              used,
              errors: restrictedPathsRef.current.length,
              restrictedPaths: restrictedPathsRef.current,
              scannedAt: nextScannedAt,
            }).catch(console.error);
            baseData.current = mapped;
            baseDataD3Hierarchy.current = diskItemToD3Hierarchy(mapped as any);
            setView("disk");
          } catch (e) {
            console.error(
              "[scan_completed] JSON.parse failed:",
              e,
              "payload snippet:",
              String(event.payload).slice(0, 200)
            );
          }
        })
      );

      scanStarted = true;
      invoke("start_scanning", { path: disk, ratio: "0.001" }).catch(
        (error) => {
          setScanError(String(error));
        }
      );
    };

    startScan().catch(console.error);
    return () => {
      cancelled = true;
      if (timer) {
        window.clearInterval(timer);
      }
      unlisteners.forEach((unlisten) => unlisten.then((f) => f()));
      if (scanStarted) {
        invoke("stop_scanning", { path: disk });
      }
    };
  }, [disk, forceScan, used]);

  useEffect(() => {
    deleteListRef.current = deleteList;
  }, [deleteList]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;

    getCurrentWindow()
      .onCloseRequested(async (event) => {
        const selectedCount = deleteListRef.current.length;
        if (selectedCount === 0) {
          return;
        }

        const shouldClose = await confirm(
          `The Collector still contains ${selectedCount} item${
            selectedCount === 1 ? "" : "s"
          }. Close SquirrelDisk anyway?`,
          {
            title: "Collector Not Empty",
            kind: "warning",
            okLabel: "Close",
            cancelLabel: "Keep Open",
          }
        );

        if (!shouldClose) {
          event.preventDefault();
        }
      })
      .then((handler) => {
        if (disposed) {
          handler();
          return;
        }
        unlisten = handler;
      })
      .catch(console.error);

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!contextMenu) {
      return;
    }

    const closeContextMenu = () => setContextMenu(null);
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setContextMenu(null);
      }
    };

    window.addEventListener("click", closeContextMenu);
    window.addEventListener("blur", closeContextMenu);
    window.addEventListener("resize", closeContextMenu);
    window.addEventListener("scroll", closeContextMenu, true);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("click", closeContextMenu);
      window.removeEventListener("blur", closeContextMenu);
      window.removeEventListener("resize", closeContextMenu);
      window.removeEventListener("scroll", closeContextMenu, true);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [contextMenu]);

  const renderVisibleChart = (
    collectorNodes = deleteListRef.current,
    preferredFocusedPath =
      focusedDirectory ? normalizeNodePath(focusedDirectory) : requestedFocusedPath
  ) => {
    if (view !== "disk" || !baseData.current || !svgRef.current) {
      return;
    }

    const collectedIds = new Set(
      collectorNodes.map((node) => node.data.id)
    );
    const visibleTree =
      removeNodesFromTree(baseData.current, collectedIds).node ||
      baseData.current;
    const rootDir = diskItemToD3Hierarchy(visibleTree);
    const focusedPath = preferredFocusedPath || normalizeNodePath(rootDir);
    const focusedNode = findNearestDirectoryByFullPath(rootDir, focusedPath);

    baseDataD3Hierarchy.current = rootDir;
    d3.select(svgRef.current).selectAll("*").remove();
    setPreviewDirectory(null);
    setHoveredItem(null);
    setContextMenu(null);

    d3Chart.current = getChart(rootDir, svgRef.current, {
      centerHover: (_, p) => {
        setHoveredItem({ ...p.data });
        setPreviewDirectory(null);
      },
      arcHover: (_, p) => {
        setHoveredItem({ ...p.data });
        setPreviewDirectory(isDirectoryNode(p) ? p : null);
      },
      arcContextMenu: (event, p) => {
        openNodeContextMenu(event, p);
        setPreviewDirectory(isDirectoryNode(p) ? p : null);
      },
      hoverCleared: () => {
        clearHoveredItem();
        clearPreviewDirectory();
      },
      arcClicked: (_, p) => {
        clearPreviewDirectory();
        setFocusedDirectoryNode(p);
        return p;
      },
    });

    if (focusedNode === rootDir) {
      const shouldUpdateRoute =
        normalizePathText(focusedPath) !== normalizeNodePath(rootDir);
      setFocusedDirectoryNode(rootDir, shouldUpdateRoute);
    } else {
      d3Chart.current.focusDirectory(focusedNode);
    }
  };

  useEffect(() => {
    renderVisibleChart(deleteList);
  }, [view, deleteList]);

  useEffect(() => {
    if (view !== "disk" || !requestedFocusedPath || !baseDataD3Hierarchy.current) {
      return;
    }

    const requestedNode = findNodeByFullPath(
      baseDataD3Hierarchy.current,
      requestedFocusedPath
    );

    if (!requestedNode || !isDirectoryNode(requestedNode)) {
      return;
    }

    if (
      focusedDirectory &&
      normalizeNodePath(focusedDirectory) === normalizePathText(requestedFocusedPath)
    ) {
      return;
    }

    clearPreviewDirectory();
    setFocusedDirectoryNode(requestedNode, false);
    d3Chart.current?.focusDirectory(requestedNode);
  }, [requestedFocusedPath, view, focusedDirectory?.data.id]);
  const expectedTotal = typeof used === "number" && used > 0 ? used : 0;
  const progressPercent =
    status && expectedTotal > 0
      ? Math.min((status.total / expectedTotal) * 100, 99.9)
      : null;
  const scanRate = status ? formatScanRate(status.total, elapsedSeconds) : "0 B/s";
  const scannedAtText = formatScannedAt(scannedAt);
  const listedDirectory = previewDirectory || focusedDirectory;
  const isPreviewingDirectory = !!previewDirectory;
  const listedDirectoryId = listedDirectory?.data.id || "empty";
  const listedChildren = listedDirectory?.children || [];
  const selectedDeleteBytes = deleteList.reduce(
    (sum, node) => sum + (node.data.size || 0),
    0
  );
  const deleteProgressPercent =
    deleteState.total > 0
      ? Math.min((deleteState.current / deleteState.total) * 100, 100)
      : 0;
  const contextNode = contextMenu?.node || null;
  const contextNodeName = contextNode?.data.name || "";
  const contextNodeIsSynthetic = isSyntheticNode(contextNode);
  const contextNodeCanExpand =
    isDirectoryNode(contextNode) && !contextNodeIsSynthetic;
  const contextCollectBlockReason = contextNode
    ? getCollectorBlockReason(contextNode)
    : null;
  const contextCollectorLabel =
    contextCollectBlockReason === "Already in Collector" ||
    contextCollectBlockReason === "Parent in Collector"
      ? contextCollectBlockReason
      : `Move "${contextNodeName}" to Collector`;
  const contextMenuLeft = contextMenu
    ? Math.max(
        8,
        Math.min(contextMenu.x, window.innerWidth - CONTEXT_MENU_WIDTH - 8)
      )
    : 0;
  const contextMenuTop = contextMenu
    ? Math.max(
        8,
        Math.min(contextMenu.y, window.innerHeight - CONTEXT_MENU_HEIGHT - 8)
      )
    : 0;
  const refreshPrivacyStatus = () => {
    invoke<PrivacyAccessStatus>("get_privacy_access_status")
      .then(setPrivacyStatus)
      .catch(console.error);
  };
  useEffect(() => {
    refreshPrivacyStatus();
    const onFocus = () => refreshPrivacyStatus();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);
  const errorsCount = status?.errors || restrictedPaths.length;
  const privacyPlatform = privacyStatus?.platform;
  const isPrivacyOSSupported =
    privacyPlatform === "macos" || privacyPlatform === "windows";
  const shouldOfferFullDiskAccess =
    isPrivacyOSSupported &&
    privacyStatus?.hasAccess === false &&
    privacyStatus?.canElevate !== false &&
    errorsCount >= 10;
  const showAccessGrantedBadge =
    isPrivacyOSSupported && privacyStatus?.hasAccess === true && errorsCount > 0;
  const requestPrivacyAccess = () => {
    invoke("request_privacy_access")
      .then(() => {
        if (privacyPlatform === "macos") {
          window.setTimeout(refreshPrivacyStatus, 1500);
        }
      })
      .catch(console.error);
  };
  const accessButtonLabel =
    privacyPlatform === "windows"
      ? "Relaunch as Administrator"
      : privacyStatus?.label || "Full Disk Access";
  const cancelPendingDelete = () => {
    cancelDeleteRef.current = true;
    setDeleteState(emptyDeleteState);
  };
  const deleteSelectedPermanently = async () => {
    if (deleteState.isDeleting || deleteList.length === 0) {
      return;
    }

    const selected = [...deleteList];
    const successful: Array<D3HierarchyDiskItem> = [];
    const failures: Array<DeleteFailure> = [];
    let recoveredBytes = 0;
    cancelDeleteRef.current = false;

    setDeleteState({
      ...emptyDeleteState,
      isDeleting: true,
      isCountingDown: true,
      countdown: DELETE_COUNTDOWN_SECONDS,
      total: selected.length,
    });

    for (let seconds = DELETE_COUNTDOWN_SECONDS; seconds > 0; seconds--) {
      if (cancelDeleteRef.current) {
        return;
      }

      setDeleteState({
        ...emptyDeleteState,
        isDeleting: true,
        isCountingDown: true,
        countdown: seconds,
        total: selected.length,
      });
      await wait(1000);
    }

    if (cancelDeleteRef.current) {
      return;
    }

    for (const [index, node] of selected.entries()) {
      const nodePath = normalizeNodePath(node);

      setDeleteState({
        isDeleting: true,
        isCountingDown: false,
        countdown: null,
        total: selected.length,
        current: index + 1,
        recoveredBytes,
        failures: [...failures],
      });
      await wait(0);

      try {
        const outcome = await invoke<DeleteOutcome>("delete_permanently", {
          path: nodePath,
        });
        successful.push(node);
        recoveredBytes += outcome.deletedBytes || node.data.size || 0;
      } catch (error) {
        failures.push({
          id: node.data.id,
          name: node.data.name,
          path: nodePath,
          message: error instanceof Error ? error.message : String(error),
        });
      }

      setDeleteState({
        isDeleting: true,
        isCountingDown: false,
        countdown: null,
        total: selected.length,
        current: index + 1,
        recoveredBytes,
        failures: [...failures],
      });
    }

    if (successful.length > 0) {
      const successfulIds = new Set(successful.map((node) => node.data.id));
      const updatedTree = baseData.current
        ? removeNodesFromTree(baseData.current, successfulIds).node
        : null;

      if (updatedTree) {
        baseData.current = updatedTree;
        baseDataD3Hierarchy.current = diskItemToD3Hierarchy(updatedTree);
      }

      if (baseData.current) {
        try {
          await setCachedScan({
            path: disk,
            tree: baseData.current,
            used: baseData.current.size || Math.max(used - recoveredBytes, 0),
            errors: restrictedPathsRef.current.length,
            restrictedPaths: restrictedPathsRef.current,
            scannedAt: scannedAtRef.current || undefined,
          });
        } catch (error) {
          console.error(error);
        }
      }
    }

    const failedIds = new Set(failures.map((failure) => failure.id));
    const survivingItems = selected.filter((node) => failedIds.has(node.data.id));
    setCollectorNodes(survivingItems);
    setDeleteState({
      isDeleting: false,
      isCountingDown: false,
      countdown: null,
      total: selected.length,
      current: selected.length,
      recoveredBytes,
      failures,
    });
  };
  return (
    <>
      {view == "loading" && status && (
        <div className="flex-1 flex flex-col justify-center items-center justify-items-center">
          <img src={diskIcon} className="w-16 h-16"></img>
          <div className="w-2/3 max-w-xl">
            <div className="mt-5 mb-1 text-base text-center font-medium text-white">
              Scanning {disk}
            </div>
            {progressPercent !== null && (
              <div className="mt-4">
                <div className="mb-1 flex items-center justify-between text-xs text-gray-400">
                  <span>Progress</span>
                  <span>{formatProgressPercent(progressPercent)}</span>
                </div>
                <div className="h-2 w-full rounded-full bg-gray-800">
                  <div
                    className="h-2 rounded-full bg-blue-500 transition-[width] duration-300 ease-out"
                    style={{ width: `${progressPercent}%` }}
                  />
                </div>
              </div>
            )}
            <div className="mt-4 grid grid-cols-2 gap-2 sm:grid-cols-4">
              <div className="rounded-md bg-gray-900/70 px-3 py-2">
                <div className="text-[10px] text-gray-500">
                  Items
                </div>
                <div className="mt-1 text-sm font-medium text-white">
                  {status.items.toLocaleString()}
                </div>
              </div>
              <div className="rounded-md bg-gray-900/70 px-3 py-2">
                <div className="text-[10px] text-gray-500">
                  Allocated
                </div>
                <div className="mt-1 text-sm font-medium text-white">
                  {prettyBytes(status.total)}
                </div>
              </div>
              <div className="rounded-md bg-gray-900/70 px-3 py-2">
                <div className="text-[10px] text-gray-500">
                  Time
                </div>
                <div className="mt-1 text-sm font-medium text-white">
                  {formatElapsed(elapsedSeconds)}
                </div>
              </div>
              <div className="rounded-md bg-gray-900/70 px-3 py-2">
                <div className="text-[10px] text-gray-500">
                  Rate
                </div>
                <div className="mt-1 text-sm font-medium text-white">
                  {scanRate}
                </div>
              </div>
            </div>
            <div
              className={`mt-3 text-center text-xs ${
                scanError ? "text-rose-300" : "text-gray-500"
              }`}
            >
              {scanError
                ? scanError
                : status.errors > 0
                ? `${status.errors.toLocaleString()} inaccessible items`
                : "No access errors"}
            </div>
            {shouldOfferFullDiskAccess && (
              <div className="mt-3 text-center">
                <button
                  type="button"
                  onClick={requestPrivacyAccess}
                  className="rounded bg-gray-800 px-3 py-1 text-xs font-medium text-gray-200 hover:bg-gray-700"
                >
                  {accessButtonLabel}
                </button>
              </div>
            )}
            {showAccessGrantedBadge && (
              <div className="mt-3 text-center text-[11px] text-emerald-300">
                {privacyStatus?.label} granted
              </div>
            )}
          </div>
          <button
            onClick={() => navigate("/")}
            className="mt-6 relative inline-flex items-center justify-center p-0.5 mb-2 mr-2 overflow-hidden text-sm font-medium  rounded-lg group bg-gradient-to-br from-purple-600 to-blue-500 group-hover:from-purple-600 group-hover:to-blue-500 hover:text-white text-white focus:ring-4 focus:ring-blue-300 focus:ring-blue-800"
          >
            <span className="relative px-5 py-2.5 transition-all ease-in duration-75  bg-gray-900 rounded-md group-hover:bg-opacity-0">
              Back
            </span>
          </button>
        </div>
      )}
      {view == "disk" && (
        <div className="flex-1 flex">
          <DragDropContext
            onDragEnd={(result) => {
              if (deleteState.isDeleting) {
                return;
              }
              if (result.destination?.droppableId !== "deletelist") {
                return;
              }
              const item = listedDirectory?.children?.find(
                (i) => i.data.id === result.draggableId,
              );
              if (!item) {
                return;
              }
              addNodeToCollector(item);
            }}
          >
            <div
              className="flex flex-1"
              onMouseLeave={() => {
                clearHoveredItem();
                clearPreviewDirectory();
              }}
            >
              <div
                className="chartpartition flex-1 flex justify-items-center	items-center"
              >
                <svg
                  ref={svgRef}
                  width={"100%"}
                  style={{ maxHeight: "calc(100vh - 40px)" }}
                />
              </div>

              <div className="bg-gray-900 w-1/3 p-2 flex flex-col">
                {listedDirectory && (
                  <ParentFolder
                    focusedDirectory={listedDirectory}
                    isPreview={isPreviewingDirectory}
                    onFocusDirectory={focusDirectory}
                  ></ParentFolder>
                )}
                {scannedAtText && (
                  <div className="mt-1 mb-2 px-2 text-[11px] text-gray-500">
                    {scannedAtText}
                  </div>
                )}
                {restrictedPaths.length > 0 && (
                  <div className="mb-2 flex items-center justify-between rounded-md bg-rose-950/30 px-3 py-2 text-xs text-rose-100">
                    <span>
                      {restrictedPaths.length.toLocaleString()} restricted
                    </span>
                    {shouldOfferFullDiskAccess ? (
                      <button
                        type="button"
                        onClick={requestPrivacyAccess}
                        className="rounded bg-rose-900/60 px-2 py-1 font-medium hover:bg-rose-800"
                      >
                        {accessButtonLabel}
                      </button>
                    ) : showAccessGrantedBadge ? (
                      <span className="text-emerald-300">
                        {privacyStatus?.label} granted
                      </span>
                    ) : null}
                  </div>
                )}
                <Droppable
                  key={`filelist-${listedDirectoryId}`}
                  droppableId={`filelist-${listedDirectoryId}`}
                >
                  {(provided) => (
                    <div
                      key={listedDirectoryId}
                      data-testid="sidebar-file-list"
                      data-directory-id={listedDirectoryId}
                      data-preview={isPreviewingDirectory ? "true" : "false"}
                      className="overflow-y-auto"
                      style={{ flex: "1 1 auto", height: 100 }}
                      ref={provided.innerRef}
                      {...provided.droppableProps}
                    >
                      {listedChildren.map((c, index) => (
                        <FileLine
                          key={c.data.id}
                          item={c}
                          hoveredItem={hoveredItem}
                          index={index}
                          deleteMap={deleteMap.current}
                          color={getChartColor(c)}
                          onHover={hoverListItem}
                          onHoverEnd={clearHoveredItem}
                          onOpenDirectory={focusDirectory}
                          onContextMenu={(event, item) =>
                            openNodeContextMenu(event, item)
                          }
                          isCollectDisabled={!!getCollectorBlockReason(c)}
                        ></FileLine>
                      ))}

                      {provided.placeholder}
                    </div>
                  )}
                </Droppable>
                <Droppable droppableId="deletelist">
                  {(provided) => (
                    <div
                      className="pt-1 flex-initial"
                      ref={provided.innerRef}
                      {...provided.droppableProps}
                    >
                      <div
                        data-testid="collector-drop-zone"
                        className="rounded-lg border	border-gray-500	border-dashed p-2 text-gray-500 text-center mb-0"
                      >
                        {deleteList.length == 0 && (
                          <>Drop files and folders here to collect</>
                        )}
                        {deleteList.length > 0 && (
                          <div className="text-left">
                            <div className="flex items-center justify-between gap-3 text-xs">
                              <span>
                                {deleteList.length} selected ·{" "}
                                {prettyBytes(selectedDeleteBytes)} allocated
                              </span>
                              <button
                                type="button"
                                className="text-gray-300 underline underline-offset-2 hover:text-white disabled:opacity-40"
                                disabled={deleteState.isDeleting}
                                onClick={() => {
                                  setCollectorNodes([]);
                                  cancelDeleteRef.current = false;
                                  setDeleteState(emptyDeleteState);
                                }}
                              >
                                Clear
                              </button>
                            </div>
                          </div>
                        )}
                        <div>{provided.placeholder}</div>
                        {deleteState.isDeleting && (
                          <div className="mt-3">
                            <div className="mb-1 flex justify-between text-xs text-gray-400">
                              {deleteState.isCountingDown ? (
                                <span>
                                  Deleting in {deleteState.countdown}
                                </span>
                              ) : (
                                <span>
                                  Deleting {deleteState.current} of{" "}
                                  {deleteState.total}
                                </span>
                              )}
                              <span>{prettyBytes(deleteState.recoveredBytes)}</span>
                            </div>
                            {!deleteState.isCountingDown && (
                              <div className="h-1.5 w-full rounded-full bg-gray-800">
                                <div
                                  className="h-1.5 rounded-full bg-red-500 transition-[width] duration-200"
                                  style={{ width: `${deleteProgressPercent}%` }}
                                />
                              </div>
                            )}
                          </div>
                        )}
                        {!deleteState.isDeleting &&
                          deleteState.failures.length > 0 && (
                            <div className="mt-3 rounded bg-rose-950/30 p-2 text-left text-xs text-rose-100">
                              <div className="font-semibold">
                                {deleteState.failures.length} survived
                              </div>
                              {deleteState.failures.slice(0, 3).map((failure) => (
                                <div
                                  key={failure.id}
                                  className="mt-1 truncate text-rose-100/80"
                                  title={`${failure.path}: ${failure.message}`}
                                >
                                  {failure.name}
                                </div>
                              ))}
                            </div>
                          )}
                        {deleteList.length > 0 && (
                          <button
                            onClick={
                              deleteState.isCountingDown
                                ? cancelPendingDelete
                                : deleteSelectedPermanently
                            }
                            type="button"
                            disabled={
                              deleteState.isDeleting &&
                              !deleteState.isCountingDown
                            }
                            className="text-white w-full mt-3 bg-gradient-to-r from-red-600 via-red-700 to-red-600 hover:bg-gradient-to-br focus:ring-4 focus:ring-red-300 focus:ring-red-800 shadow-sm shadow-red-500/50 shadow-lg shadow-red-800/80 font-medium rounded-lg text-sm px-5 py-2.5 text-center mr-2 mb-2"
                          >
                            {deleteState.isCountingDown
                              ? "Cancel"
                              : deleteState.isDeleting
                              ? "Deleting " +
                                deleteState.current +
                                " of " +
                                deleteState.total
                              : "Delete Permanently"}
                          </button>
                        )}
                      </div>
                    </div>
                  )}
                </Droppable>
              </div>
            </div>
          </DragDropContext>
        </div>
      )}
      {contextMenu &&
        contextNode &&
        createPortal(
          <div
            role="menu"
            data-testid="node-context-menu"
            data-node-id={contextNode.data.id}
            className="fixed z-[9999] w-[250px] overflow-hidden rounded-md border border-gray-600 py-1 text-left text-sm text-gray-100 shadow-2xl"
            style={{
              left: contextMenuLeft,
              top: contextMenuTop,
              backgroundColor: "#111827",
            }}
            onClick={(event) => event.stopPropagation()}
            onContextMenu={(event) => {
              event.preventDefault();
              event.stopPropagation();
            }}
          >
            <button
              type="button"
              role="menuitem"
              data-testid="context-menu-expand"
              disabled={!contextNodeCanExpand}
              className="block w-full truncate px-4 py-2 text-left font-semibold text-gray-100 hover:bg-gray-800 disabled:text-gray-500 disabled:hover:bg-transparent"
              onClick={() => {
                if (!contextNodeCanExpand) {
                  return;
                }
                setContextMenu(null);
                focusDirectory(contextNode);
              }}
            >
              Expand "{contextNodeName}"
            </button>
            <button
              type="button"
              role="menuitem"
              data-testid="context-menu-show-in-finder"
              disabled={contextNodeIsSynthetic}
              className="block w-full px-4 py-2 text-left font-semibold text-gray-100 hover:bg-gray-800 disabled:text-gray-500 disabled:hover:bg-transparent"
              onClick={() => {
                setContextMenu(null);
                showNodeInFolder(contextNode);
              }}
            >
              Show in Finder
            </button>
            <button
              type="button"
              role="menuitem"
              data-testid="context-menu-open-terminal"
              disabled={contextNodeIsSynthetic}
              className="block w-full px-4 py-2 text-left font-semibold text-gray-100 hover:bg-gray-800 disabled:text-gray-500 disabled:hover:bg-transparent"
              onClick={() => {
                setContextMenu(null);
                openNodeInTerminal(contextNode);
              }}
            >
              Open in Terminal
            </button>
            <div className="my-1 border-t border-gray-700" />
            <button
              type="button"
              role="menuitem"
              data-testid="context-menu-collect"
              disabled={!!contextCollectBlockReason}
              title={contextCollectBlockReason || undefined}
              className="block w-full truncate px-4 py-2 text-left font-semibold text-rose-100 hover:bg-rose-950/60 disabled:text-gray-500 disabled:hover:bg-transparent"
              onClick={() => {
                if (contextCollectBlockReason) {
                  return;
                }
                setContextMenu(null);
                addNodeToCollector(contextNode);
              }}
            >
              {contextCollectorLabel}
            </button>
          </div>,
          document.body
        )}
    </>
  );
};

export default Scanning;
