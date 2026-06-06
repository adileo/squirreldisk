import * as d3 from "d3";
// const pLimit = require('p-limit')

let genId = 0;

const pathParts = (name: string) =>
  name
    .replace(/\\/g, "/")
    .split("/")
    .filter(Boolean);

const cloneNode = (node: any): any => ({
  ...node,
  children: Array.isArray(node.children) ? node.children.map(cloneNode) : [],
});

const cloneWithName = (node: any, name: string) => ({
  ...cloneNode(node),
  name,
});

const makeGroupNode = (name: string) => ({
  name,
  value: 0,
  size: 0,
  isDirectory: true,
  children: [],
});

const isDirectoryLike = (node: any) =>
  !!node?.isDirectory ||
  (Array.isArray(node?.children) && node.children.length > 0);

const canMergeNodes = (left: any, right: any) =>
  left?.name === right?.name && isDirectoryLike(left) === isDirectoryLike(right);

const mergeNodeInto = (target: any, source: any) => {
  const sourceSize = source.size || 0;
  target.size = (target.size || 0) + sourceSize;
  target.value = target.size;
  target.isDirectory = isDirectoryLike(target) || isDirectoryLike(source);

  if (source.restricted) {
    target.restricted = true;
    target.restrictedPath = source.restrictedPath;
    target.restrictedReason = source.restrictedReason;
  }

  if (!Array.isArray(source.children) || source.children.length === 0) {
    return;
  }

  if (!Array.isArray(target.children)) {
    target.children = [];
  }

  source.children.forEach((sourceChild: any) => {
    const child = cloneNode(sourceChild);
    const existing = target.children.find((entry: any) =>
      canMergeNodes(entry, child)
    );

    if (existing) {
      mergeNodeInto(existing, child);
    } else {
      target.children.push(child);
    }
  });
};

const mergeChildIntoParent = (parent: any, child: any) => {
  parent.children = parent.children || [];
  const existing = parent.children.find((entry: any) =>
    canMergeNodes(entry, child)
  );

  if (existing) {
    mergeNodeInto(existing, child);
  } else {
    parent.children.push(child);
  }
};

const addSizeToParent = (parent: any, size: number) => {
  parent.size = (parent.size || 0) + size;
  parent.value = parent.size;
};

const startsWithParts = (parts: Array<string>, prefix: Array<string>) =>
  prefix.every((part, index) => parts[index] === part);

const normalizeAbsolutePath = (path: string) => {
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
  return normalized.startsWith("/") ? normalized || "/" : `/${normalized}`;
};

const insertAbsoluteChild = (parent: any, parts: Array<string>, node: any) => {
  if (parts.length === 0) {
    return;
  }

  const [part, ...rest] = parts;
  if (rest.length === 0) {
    mergeChildIntoParent(parent, cloneWithName(node, part));
    addSizeToParent(parent, node.size || 0);
    return;
  }

  let group = parent.children.find(
    (child: any) => child.name === part && isDirectoryLike(child)
  );
  if (!group) {
    group = makeGroupNode(part);
    parent.children.push(group);
  }

  insertAbsoluteChild(group, rest, node);
  addSizeToParent(parent, node.size || 0);
};

const sortTreeChildren = (node: any) => {
  if (!Array.isArray(node.children)) {
    return;
  }

  node.children.sort((a: any, b: any) => {
    if (!!a.restricted !== !!b.restricted) {
      return a.restricted ? 1 : -1;
    }

    const sizeDifference = (b.size || 0) - (a.size || 0);
    return sizeDifference || String(a.name).localeCompare(String(b.name));
  });
  node.children.forEach(sortTreeChildren);
};

const insertRestrictedPath = (
  parent: any,
  parts: Array<string>,
  restrictedPath: RestrictedPath
) => {
  if (parts.length === 0) {
    return;
  }

  const [part, ...rest] = parts;
  let child = parent.children?.find(
    (entry: any) => entry.name === part && isDirectoryLike(entry)
  );

  if (rest.length === 0) {
    if (child) {
      child.restricted = true;
      child.restrictedPath = restrictedPath.path;
      child.restrictedReason = restrictedPath.message;
      return;
    }

    parent.children = parent.children || [];
    parent.children.push({
      id: "",
      name: part,
      value: 0,
      size: 0,
      isDirectory: true,
      children: [],
      restricted: true,
      restrictedPath: restrictedPath.path,
      restrictedReason: restrictedPath.message,
    });
    return;
  }

  if (!child) {
    child = makeGroupNode(part);
    parent.children = parent.children || [];
    parent.children.push(child);
  }

  insertRestrictedPath(child, rest, restrictedPath);
};

export const addRestrictedPathsToTree = (
  root: any,
  basePath: string,
  restrictedPaths: Array<RestrictedPath> = []
) => {
  if (!root || !Array.isArray(root.children) || restrictedPaths.length === 0) {
    return root;
  }

  const baseParts = pathParts(normalizeAbsolutePath(basePath || root.name || "/"));
  const seen = new Set<string>();

  restrictedPaths.forEach((restrictedPath) => {
    const fullPath = normalizeAbsolutePath(restrictedPath.path);
    if (seen.has(fullPath)) {
      return;
    }
    seen.add(fullPath);

    const fullParts = pathParts(fullPath);
    const relativeParts = startsWithParts(fullParts, baseParts)
      ? fullParts.slice(baseParts.length)
      : fullParts;

    insertRestrictedPath(root, relativeParts, {
      ...restrictedPath,
      path: fullPath,
    });
  });

  sortTreeChildren(root);
  return root;
};

export const groupChildrenByBasePath = (root: any, basePath = "/") => {
  if (!root || !Array.isArray(root.children)) {
    return root;
  }

  const baseParts = pathParts(basePath);
  const groupedRoot: any = {
    ...root,
    name: basePath || root.name,
    size: 0,
    value: 0,
    children: [],
  };

  root.children.forEach((child: any) => {
    const childParts = pathParts(child.name || "");
    const relativeParts = startsWithParts(childParts, baseParts)
      ? childParts.slice(baseParts.length)
      : childParts;
    const parts = relativeParts.length > 0 ? relativeParts : childParts;

    if (parts.length <= 1) {
      mergeChildIntoParent(
        groupedRoot,
        cloneWithName(child, parts[0] || child.name)
      );
      addSizeToParent(groupedRoot, child.size || 0);
      return;
    }

    insertAbsoluteChild(groupedRoot, parts, child);
  });

  sortTreeChildren(groupedRoot);
  return groupedRoot;
};

export const groupRootChildrenByTopLevelPath = (root: any) =>
  groupChildrenByBasePath(root, "/");

export const itemMap = (obj: any, parent: any = null) => {
  if (obj.name === "(total)") {
    obj.id = "/";
    obj.name = "/";
  } else if (parent && parent.id === "/") {
    const hasLeadingSlash = obj.name.startsWith("/");
    obj.id = hasLeadingSlash ? obj.name : `/${obj.name}`;
    obj.name = hasLeadingSlash ? obj.name.substring(1) : obj.name;
  } else {
    obj.id = parent ? parent.id + "/" + obj.name : obj.name;
  }

  if (obj.hasOwnProperty("children")) {
    //recursive call to scan property
    if (obj["children"].length > 0) {
      obj.isDirectory = true;
      obj.value = obj.size;
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
      return !d.children || d.children.length === 0 ? d.size : 0;
    })

    // .sum(d => d.value)
    // .sum((d: DiskItem) => (d.children ? d.data : d.data))
    // .sum(d => d.data ? 0 : d.value)
    .sort((a: any, b: any) => (b.size || 0) - (a.size || 0));
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
