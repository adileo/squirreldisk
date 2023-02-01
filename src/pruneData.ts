import * as d3 from "d3";
// const pLimit = require('p-limit')
import { v4 as uuidv4 } from "uuid";

let genId = 0;
export const itemMap = (obj: any, parent: any = null) => {
  if (obj.name === "(total)") {
    obj.id = "/";
    obj.name = "/";
  } else if (parent && parent.id === "/") {
    obj.id = obj.name;
    obj.name = obj.name.substring(1); // remove the slash for 1st level dirs /folder
  } else {
    obj.id = parent ? parent.id + "/" + obj.name : obj.name;
  }

  if (obj.hasOwnProperty("children")) {
    //recursive call to scan property
    if (obj["children"].length > 0) {
      obj.isDirectory = true;
      obj.value = obj.data;
      obj["children"].forEach((element: any) => {
        itemMap(element, obj);
      });
    }
  }
  return obj;
};

const partition = (data: DiskItem) => {
  const hierarchy = d3
    .hierarchy(data)
    .sum(function (d) {
      return !d.children || d.children.length === 0 ? d.data : 0;
    })

    // .sum(d => d.value)
    // .sum((d: DiskItem) => (d.children ? d.data : d.data))
    // .sum(d => d.data ? 0 : d.value)
    .sort((a: any, b: any) => (b.data || 0) - (a.data || 0));
  // debugger;
  const partition = d3
    .partition<DiskItem>()
    .size([2 * Math.PI, hierarchy.height + 1])(hierarchy);
  console.log({ partition });
  // debugger;
  return partition;
};

export function diskItemToD3Hierarchy(baseData: DiskItem) {
  let root = partition(baseData) as D3HierarchyDiskItem;
  root.each(
    (d: any) => (d.current = { x0: d.x0, x1: d.x1, y0: d.y0, y1: d.y1 })
  );
  return root as D3HierarchyDiskItem;
}

// Elimino tutti i children oltre una certa depth
// Parto dal nodo rootPath

// CLONE
export function depthCut(
  node: DiskItem,
  depth: number,
  curDepth = 0
): DiskItem {
  var newNode = {
    ...node,
  };
  if (newNode.children && depth == curDepth) {
    newNode.children = [];
  } else if (newNode.children) {
    newNode.children = newNode.children.map((c) =>
      depthCut(c, depth, curDepth + 1)
    );
  }
  return newNode;
}

// EDIT
export function pruneIrrelevant(
  node: DiskItem,
  origReference: number | null = null,
  threshold = 0.004
): DiskItem | null {
  if (node === null) {
    return null;
  }
  //   const reference = origReference ? origReference : node.data;
  //   if (node.children) {
  //     let keep = node.children
  //       .filter((c) => c.data / reference > threshold)
  //       .map((keeped) => pruneIrrelevant(keeped, reference, threshold))
  //       .filter((i) => i !== null) as Array<DiskItem>;
  //     const consolidate = node.children.filter(
  //       (c) => c.data / reference <= threshold
  //     );
  //     const smallerItems = consolidate.reduce(
  //       (a, b) => ({ ...a, /*count: a.count! + 1,*/ data: a.data + b.data }),
  //       {
  //         id: uuidv4(),
  //         name: "Smaller Items",
  //         // count: 0,
  //         data: 0,
  //         isDirectory: false,
  //         children: [],
  //       }
  //     );
  //     let newChilren: Array<DiskItem>;
  //     if (smallerItems.count && smallerItems.count > 0) {
  //       newChilren = [...keep, smallerItems];
  //     } else {
  //       newChilren = keep;
  //     }
  //     node.children = newChilren;
  //   }
  return node;
}

export function getViewNode(base: DiskItem, path: Array<string> = []) {
  if (path.length == 0) {
    // const cutted = depthCut(base, 4, 0)
    // debugger
    // const pruned = pruneIrrelevant(cutted)
    // debugger
    const graph = diskItemToD3Hierarchy(base);
    debugger;
    return graph;
  } else {
    const cutted = depthCut(base, 14, 0); // 14 since it's hard to handle deepnavigation in the chart we should fix this
    const pruned = pruneIrrelevant(cutted);

    const origNode = getNode(base, path);
    // const origNodePruned = pruneIrrelevant(origNode!);

    // nodeStitch(pruned!, origNodePruned!, path);

    // return diskItemToD3Hierarchy(pruned!);
    return diskItemToD3Hierarchy(pruned!);
  }
}

export function getViewNodeGraph(
  base: D3HierarchyDiskItem,
  path: Array<string> = []
): D3HierarchyDiskItem | null {
  let newPath = [...path];
  if (path.length == 0) {
    return base;
  } else {
    const match = newPath.shift();
    if (!base.children) {
      return null;
    }
    const found = base.children.find((node) => node.data.name === match);
    if (found) {
      return getViewNodeGraph(found, newPath);
    } else {
      return null;
    }
  }
}

export function nodeStitch(
  base: DiskItem,
  node: DiskItem,
  path: Array<string> = []
) {
  var stiched = getNode(base, path);
  if (stiched) stiched.children = node.children;
  return base;
}
export function getNode(
  node: DiskItem,
  path: Array<string> = []
): DiskItem | null {
  // console.log('GetNode', node, path)
  let newPath = [...path];
  if (path.length == 0) {
    return node;
  } else {
    const match = newPath.shift();
    if (!node.children) {
      return null;
    }
    const found = node.children.find((node) => node.name === match); //FIXME: undef children case
    if (found) {
      return getNode(found, newPath);
    } else {
      return null;
    }
  }
}

export function buildPath(
  node: D3HierarchyDiskItem,
  acc: Array<string> = []
): Array<string> {
  if (node.parent) {
    return buildPath(node.parent, [node.data.name, ...acc]);
  } else {
    return acc;
  }
}

export function buildFullPath(
  node: D3HierarchyDiskItem,
  acc: Array<string> = []
): string {
  const path = node.data.id.replace("\\/", "/").replace("\\", "/");
  //   console.log({ path });
  return path;
  //   if (node.parent) {
  //     return buildFullPath(node.parent, [node.data.name, ...acc]);
  //   } else {
  //     var x = [node.data.name, ...acc];
  //     return x.join("/");
  //   }
}
