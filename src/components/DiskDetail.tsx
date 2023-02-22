import { useEffect, useRef, useState } from "react";
import { useNavigate, useLocation } from "react-router-dom";
import diskIcon from "../assets/harddisk.png";
import { getChart } from "../d3chart";
import * as d3 from "d3";
import {
  buildPath,
  getViewNode,
  getViewNodeGraph,
  buildFullPath,
  diskItemToD3Hierarchy,
  itemMap,
} from "../pruneData";
import { FileLine } from "./FileLine";
import { ParentFolder } from "./ParentFolder";
import { DragDropContext, Droppable } from "react-beautiful-dnd";
import { invoke } from "@tauri-apps/api/tauri";
import { emit, listen } from "@tauri-apps/api/event";
import { removeFile, removeDir } from "@tauri-apps/api/fs";

(window as any).LockDNDEdgeScrolling = () => true;

const Scanning = () => {
  let {
    state: { disk, used, fullscan },
  } = useLocation() as any;
  const navigate = useNavigate();

  const svgRef = useRef<SVGSVGElement | null>(null);

  // Original Data
  const baseData = useRef<DiskItem | null>(null);
  // D3 Hierarchy Data
  const baseDataD3Hierarchy = useRef<D3HierarchyDiskItem | null>(null);

  // Current Directory
  const [focusedDirectory, setFocusedDirectory] =
    useState<D3HierarchyDiskItem | null>(null);
  // Hovered Item
  const [hoveredItem, setHoveredItem] = useState<DiskItem | null>(null);

  const worker = useRef<Worker | null>(null);
  const d3Chart = useRef(null) as any;
  const [view, setView] = useState("loading");
  const [bytesProcessed, setByteProcessed] = useState(0);
  const [status, setStatus]: any = useState();
  const [deleteState, setDeleteState] = useState({
    isDeleting: false,
    total: 0,
    current: 0,
  });

  const [deleteList, setDeleteList] = useState<Array<D3HierarchyDiskItem>>([]);
  const deleteMap = useRef<Map<string, boolean>>(new Map());
  // Avvio il worker e attendo i dati
  useEffect(() => {
    if (baseData.current) {
      // Skip if already loaded data
      return;
    }
    const unlisten = listen("scan_status", (event: any) => {
      // event.event is the event name (useful if you want to use a single callback fn for multiple event types)
      // event.payload is the payload object
      setStatus(event.payload);
    });

    const unlisten2 = listen("scan_completed", (event: any) => {
      // event.event is the event name (useful if you want to use a single callback fn for multiple event types)
      // event.payload is the payload object
      baseData.current = JSON.parse(event.payload).tree;
      const mapped = itemMap(baseData.current);
      baseDataD3Hierarchy.current = diskItemToD3Hierarchy(mapped as any);
      setView("disk");
    });

    invoke("start_scanning", { path: disk, ratio: fullscan ? "0" : "0.001" });
    // worker.current = new Worker(new URL("./space.worker.js", import.meta.url), {
    //   type: "module",
    // });
    // worker.current.postMessage({ type: "start", path: disk });
    // worker.current.onmessage = function (e) {
    //   const msg = e.data;
    //   if (msg.type == "DONE") {
    //     baseData.current = msg.data;
    //     baseDataD3Hierarchy.current = diskItemToD3Hierarchy(msg.data);
    //     setView("disk");
    //   }
    //   if (msg.type == "STATUS") {
    //     setByteProcessed(msg.data);
    //   }
    // };
    return () => {
      unlisten.then((f) => f());
      unlisten2.then((f) => f());
      invoke("stop_scanning", { path: disk });
      //   worker.current!.postMessage({ type: "stop" });
    };
  }, [disk, setStatus]);

  // Appena ho i dati
  useEffect(() => {
    if (view == "disk") {
      // Remove old chart
      d3.select(svgRef.current).selectAll("*").remove();

      const rootDir = baseDataD3Hierarchy.current!;
      setFocusedDirectory(rootDir);

      const base = baseDataD3Hierarchy.current!; //getViewNode(baseData.current!);

      d3Chart.current = getChart(base, svgRef.current!, {
        centerHover: (_, p) => {
          // console.log({centerHover: p})
          setHoveredItem({ ...p.data });
        },
        arcHover: (_, p) => {
          // console.log({arcHover: p})
          setHoveredItem({ ...p.data });
        },
        arcClicked: (_, p) => {
          setFocusedDirectory(p);
          return p;
          const curNodePath = buildPath(p);
          const vn = getViewNode(baseData.current!, curNodePath);

          setFocusedDirectory(
            getViewNodeGraph(baseDataD3Hierarchy.current!, curNodePath)
          );
          return vn;
        },
      });
    }
  }, [view]);
  // Avoid progress bar going to the star due to undetectable fs hardlinks
  const cappedTotal = Math.min(status ? status.total : 0, used)
  return (
    <>
      {view == "loading" && status && (
        <div className="flex-1 flex flex-col justify-center items-center justify-items-center">
          <img src={diskIcon} className="w-16 h-16"></img>
          <div className="w-2/3">
            <div className="mt-5 mb-1 text-base text-center font-medium text-white">
              Scanning {disk} {((cappedTotal / used) * 100).toFixed(2)}
              %
              <br />
              {/* <span className="text-sm">{itemPath}</span> */}
            </div>
            <div className="mt-4 w-full bg-gray-200 rounded-full h-2.5">
              <div
                className="bg-blue-600 h-2.5 rounded-full"
                style={{
                  width: ((cappedTotal / used) * 100).toFixed(2) + "%",
                }}
              ></div>
            </div>
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
              console.log(result);
              if (result.destination?.droppableId !== "deletelist") {
                return;
              }
              const item = focusedDirectory!.children!.find(
                (i) => i.data.id === result.draggableId
              );
              setDeleteList((val) => {
                if (!val.find((e) => e.data.id === item!.data.id)) {
                  deleteMap.current.set(item!.data.id, true);

                  return [...val, item!];
                } else {
                  return val;
                }
              });
            }}
          >
            <div className="flex flex-1">
              <div className="chartpartition flex-1 flex justify-items-center	items-center">
                <svg
                  ref={svgRef}
                  width={"100%"}
                  style={{ maxHeight: "calc(100vh - 40px)" }}
                />
              </div>

              <div className="bg-gray-900 w-1/3 p-2 flex flex-col">
                {focusedDirectory && (
                  <ParentFolder
                    focusedDirectory={focusedDirectory}
                    d3Chart={d3Chart}
                  ></ParentFolder>
                )}
                <Droppable droppableId="filelist">
                  {(provided) => (
                    <div
                      className="overflow-y-auto"
                      style={{ flex: "1 1 auto", height: 100 }}
                      ref={provided.innerRef}
                      {...provided.droppableProps}
                    >
                      {focusedDirectory &&
                        focusedDirectory.children &&
                        focusedDirectory.children.map((c, index) => (
                          <FileLine
                            key={c.data.id}
                            item={c}
                            hoveredItem={hoveredItem}
                            d3Chart={d3Chart}
                            index={index}
                            deleteMap={deleteMap.current}
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
                      <div className="rounded-lg border	border-gray-500	border-dashed p-2 text-gray-500 text-center mb-0">
                        {deleteList.length == 0 && (
                          <>Drag file and folders here to delete</>
                        )}
                        {deleteList.length > 0 && (
                          <div>
                            <div>
                              {deleteList.length} files selected -{" "}
                              <a
                                href="#"
                                className="underline underline-offset-2"
                                onClick={() => {
                                  setDeleteList([]);
                                  deleteMap.current.clear();
                                }}
                              >
                                Clear Selection
                              </a>
                            </div>
                          </div>
                        )}
                        <div>{provided.placeholder}</div>
                        {deleteList.length > 0 && (
                          <button
                            onClick={async () => {
                              setDeleteState({
                                isDeleting: true,
                                total: deleteList.length,
                                current: 0,
                              });
                              // Avvio spinner
                              let successful: Array<D3HierarchyDiskItem> = [];
                              // Cancello (errori li scarto da eliminare quindi vengono tenuti)
                              for (let node of deleteList) {
                                const nodePath = buildFullPath(node)
                                  .replace("\\/", "/")
                                  .replace("\\", "/");
                                try {
                                  //   await window.electron.diskUtils.rimraf(
                                  //     nodePath
                                  //   );
                                  //   if (
                                  //     node.children &&
                                  //     node.children.length > 0
                                  //   ) {
                                  // Workaroound: Since sometimes if the tree has some trimmed leafs a folder has no children
                                  removeDir(nodePath, {
                                    recursive: true,
                                  }).catch((err) =>
                                    removeFile(nodePath).catch((err2) =>
                                      console.error(err, err2)
                                    )
                                  );
                                  //   } else {
                                  //     removeFile(nodePath).catch((err) => console.error(err));
                                  //   }
                                  successful.push(node);
                                  setDeleteState((prev) => ({
                                    ...prev,
                                    current: prev.current + 1,
                                  }));
                                } catch (e) {
                                  console.error(e);
                                }
                              }
                              // Una volta finito aggiorno il grafico
                              d3Chart.current.deleteNodes(successful);
                              setDeleteState((prev) => ({
                                isDeleting: false,
                                total: 0,
                                current: 0,
                              }));
                              setDeleteList([]);
                              deleteMap.current.clear();
                            }}
                            type="button"
                            disabled={deleteState.isDeleting}
                            className="text-white w-full mt-3 bg-gradient-to-r from-red-600 via-red-700 to-red-600 hover:bg-gradient-to-br focus:ring-4 focus:ring-red-300 focus:ring-red-800 shadow-sm shadow-red-500/50 shadow-lg shadow-red-800/80 font-medium rounded-lg text-sm px-5 py-2.5 text-center mr-2 mb-2"
                          >
                            {deleteState.isDeleting
                              ? "Deleting " +
                                deleteState.current +
                                " of " +
                                deleteState.total
                              : "Delete"}
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
    </>
  );
};

export default Scanning;
