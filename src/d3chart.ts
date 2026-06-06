import * as d3 from "d3";
import prettyBytes from "pretty-bytes";
import { getChartColor } from "./chartColors";

const width = 600;
const visibleDescendantLevels = 8;
const maxVisibleRadiusUnits = visibleDescendantLevels + 1;
const radius = width / (2 * (maxVisibleRadiusUnits + 0.35));
const arcGap = 0.5;
const smallerItemsThreshold = 0.005;
const arc = d3
  .arc<D3HierarchyDiskItemArc>()
  .startAngle((d) => d.x0)
  .endAngle((d) => d.x1)
  .padAngle((d) => Math.min((d.x1 - d.x0) / 2, 0.0025))
  .padRadius(radius * 1.5)
  .innerRadius((d) => d.y0 * radius)
  .outerRadius((d) => Math.max(d.y0 * radius, d.y1 * radius - arcGap));

// export const blink = (root) => {
//   root
//     .transition()
//     .duration(1000)
//     .style('fill', 'rgb(255,255,255)')
//     .transition()
//     .duration(1000)
//     .style('fill', 'rgb(0,0,0)')
//     .on('end', blink);
// };
const arcVisible = (d: D3HierarchyDiskItemArc) => {
  // Hide arcs outside the focused subtree's visible ring budget.
  // d.y0 >= 1 => Hide root arc (spot in the middle)
  // d.x1 > d.x0 => hide non focused arcs
  return d.y1 <= maxVisibleRadiusUnits && d.y0 >= 1 && d.x1 > d.x0;
};

const formatNodePath = (node: D3HierarchyDiskItem) => {
  const names = node
    .ancestors()
    .map((ancestor) => ancestor.data.name)
    .reverse()
    .filter(Boolean);

  if (names.length === 0) {
    return "";
  }

  const [rootName, ...childNames] = names;
  const root = rootName === "/" ? "" : rootName.replace(/\/+$/, "");
  const path = [root, ...childNames.map((name) => name.replace(/^\/+/, ""))]
    .filter(Boolean)
    .join("/");

  if (rootName === "/" || rootName.startsWith("/")) {
    return path ? `/${path.replace(/^\/+/, "")}` : "/";
  }

  return path || rootName;
};

const titleText = (node: D3HierarchyDiskItem, mul: number) =>
  `${formatNodePath(node)}\nAllocated ${(
    (node.data.size || 0) /
    mul /
    mul /
    mul
  ).toFixed(2)} GB`;

const baseArcOpacity = (
  node: D3HierarchyDiskItem,
  arcData: D3HierarchyDiskItemArc = node.current
) => {
  if (!arcVisible(arcData)) {
    return 0;
  }

  return node.children ? 0.98 : 0.9;
};

const applyHoverState = (
  path: d3.Selection<
    SVGPathElement,
    D3HierarchyDiskItem,
    SVGGElement,
    D3HierarchyDiskItem
  >,
  hoveredNodeId: string | null,
  animate = true
) => {
  path.interrupt("hover").interrupt("pulse");
  const visibleHoveredNodeId =
    hoveredNodeId && !path.filter((d) => d.data.id === hoveredNodeId).empty()
      ? hoveredNodeId
      : null;

  const selection = animate
    ? (path.transition("hover").duration(160).ease(d3.easeCubicOut) as any)
    : path;

  selection
    .attr("fill-opacity", (d: D3HierarchyDiskItem) => baseArcOpacity(d))
    .attr("stroke", "#2f3746")
    .attr("stroke-width", 0.55);

  const startPulse = () => {
    if (!visibleHoveredNodeId) return;

    const hoveredPath = path.filter((d) => d.data.id === visibleHoveredNodeId);
    const pulse = () => {
      hoveredPath
        .transition("pulse")
        .duration(320)
        .ease(d3.easeSinInOut)
        .attr("fill-opacity", 0.82)
        .transition()
        .duration(320)
        .ease(d3.easeSinInOut)
        .attr("fill-opacity", (d: D3HierarchyDiskItem) => baseArcOpacity(d))
        .on("end", pulse);
    };

    pulse();
  };

  if (animate) {
    selection
      .end()
      .then(startPulse)
      .catch(() => {});
  } else {
    startPulse();
  }
};

// const setTargetAngles = (
//   root: D3HierarchyDiskItem,
//   focusedNode: D3HierarchyDiskItem
// ) => {
//   // Focus on current slice
//   root.each((d: any) => {
//     const focusedSegmentRadians = focusedNode.x1 - focusedNode.x0;
//     const arcDeltaFrom = d.x0 - focusedNode.x0;
//     const arcDeltaTo = d.x1 - focusedNode.x0;
//     const fromPercentage = arcDeltaFrom / focusedSegmentRadians;
//     const toPercentage = arcDeltaTo / focusedSegmentRadians;

//     d.target = {
//       x0: Math.max(0, Math.min(1, fromPercentage)) * 2 * Math.PI, // Start Angle, radians, 0 starting from 12 oclock
//       x1: Math.max(0, Math.min(1, toPercentage)) * 2 * Math.PI, // End Angle, radians, 0 starting from 12 oclock
//       y0: Math.max(0, d.y0 - focusedNode.depth), // Inner Radius
//       y1: Math.max(0, d.y1 - focusedNode.depth), // Outer Radius
//     };
//   });
// };

const setTargetAngles = (
  filtered: Array<D3HierarchyDiskItem>,
  focusedNode: D3HierarchyDiskItem
) => {
  // Focus on current slice
  filtered.forEach((d) => {
    const focusedSegmentRadians = focusedNode.x1 - focusedNode.x0;
    const arcDeltaFrom = d.x0 - focusedNode.x0;
    const arcDeltaTo = d.x1 - focusedNode.x0;
    const fromPercentage = arcDeltaFrom / focusedSegmentRadians;
    const toPercentage = arcDeltaTo / focusedSegmentRadians;

    d.target = {
      x0: Math.max(0, Math.min(1, fromPercentage)) * 2 * Math.PI, // Start Angle, radians, 0 starting from 12 oclock
      x1: Math.max(0, Math.min(1, toPercentage)) * 2 * Math.PI, // End Angle, radians, 0 starting from 12 oclock
      y0: Math.max(0, d.y0 - focusedNode.depth), // Inner Radius
      y1: Math.max(0, d.y1 - focusedNode.depth), // Outer Radius
    };
  });
};

const animateToTarget = (
  g: d3.Selection<SVGGElement, D3HierarchyDiskItem, null, undefined>,
  path: d3.Selection<
    SVGPathElement,
    D3HierarchyDiskItem,
    SVGGElement,
    D3HierarchyDiskItem
  >,
  getHoveredNodeId: () => string | null
) => {
  // Transition the data on all arcs, even the ones that aren’t visible,
  // so that if this transition is interrupted, entering arcs will start
  // the next transition from the desired position.
  const t = g.transition().duration(750) as any;

  path
    .transition(t)
    .tween("data", (d) => {
      const i = d3.interpolate(d.current, d.target);

      return (t) => {
        const interpol = i(t);
        if (!interpol) {
          debugger;
        }
        return (d.current = interpol);
      };
    })
    .filter(function (d: any) {
      // Hide non relevant arcs
      // console.log(d.target)
      return !!(+this.getAttribute("fill-opacity")! || arcVisible(d.target));
    })
    .attr("fill-opacity", (d: any) =>
      arcVisible(d.target) ? baseArcOpacity(d, d.target) : 0
    )
    .attrTween("d", (d) => () => {
      if (!d.current) {
        debugger;
      }
      return arc(d.current)!;
    })
    .end()
    .then(() => {
      applyHoverState(path, getHoveredNodeId());
    })
    .catch((e) => {
      // console.error(e);
    });
};

const makeSmallerItemsNode = (
  item: D3HierarchyDiskItem,
  focused: D3HierarchyDiskItem,
  index: number
) => {
  const data: DiskItem = {
    id: `${item.parent?.data.id || focused.data.id}/__smaller_items_${index}`,
    isDirectory: false,
    name: "Smaller Items",
    value: item.value || 0,
    size: item.value || 0,
    children: [],
    synthetic: true,
  };
  const node = d3.hierarchy(data) as D3HierarchyDiskItem;

  Object.assign(node as any, item);
  node.data = data;
  node.parent = item.parent;
  (node as any).children = undefined;
  (node as any).value = item.value || 0;
  node.current = { ...item.current };

  return node;
};

const updateData = (
  root: D3HierarchyDiskItem,
  focused: D3HierarchyDiskItem,
  innerG: d3.Selection<SVGGElement, D3HierarchyDiskItem, null, undefined>,
  // color: d3.ScaleOrdinal<string, string, never>,
  arcClickHandler: (event: any, focusedNode: D3HierarchyDiskItem) => void,
  hoverHandler: (event: any, focusedNode: D3HierarchyDiskItem) => void,
  clearHoverHandler: () => void,
  contextMenuHandler: (event: any, focusedNode: D3HierarchyDiskItem) => void
) => {
  let filtered = [...focused.ancestors().slice(-1)];
  let initialDepth = focused.depth;
  let maxDepth = initialDepth + visibleDescendantLevels;
  let overallSize = focused.value || 0;
  let accumulator: D3HierarchyDiskItem | null = null;
  let accumulatorLastParent: D3HierarchyDiskItem | null = null;
  let smallerItemIndex = 0;
  let skipMap: any = {};

  // Tronco sulla max depth
  for (const item of focused.descendants().slice(1)) {
    if (
      accumulator &&
      accumulatorLastParent &&
      item.parent !== accumulatorLastParent
    ) {
      filtered.push(accumulator);
      accumulator = null;
      accumulatorLastParent = null;
    }
    if (item.parent && item.parent!.data.id in skipMap) {
      skipMap[item.data.id] = true;
      continue;
    }
    // Escludo cerchi più esterni
    if (item.depth > maxDepth) {
      continue;
    }
    const sizeRatio = overallSize > 0 ? (item.value || 0) / overallSize : 1;
    if (sizeRatio > smallerItemsThreshold) {
      // Includo item grandi
      filtered.push(item);
    } else {
      // Accumulo item piccoli
      if (accumulator) {
        skipMap[item.data.id] = true;
        accumulator.data.value += item.value ?? 0;
        accumulator.data.size += item.value ?? 0;
        (accumulator as any).value += item.value ?? 0;

        (accumulator as any).current.x1 = item.current.x1;
        (accumulator as any).x1 = item.x1;
      } else {
        skipMap[item.data.id] = true;
        accumulator = makeSmallerItemsNode(item, focused, smallerItemIndex++);
        accumulatorLastParent = item.parent;
      }
    }
  }
  if (accumulator) {
    filtered.push(accumulator);
  }
  setTargetAngles(filtered, focused);
  // console.log({filtered})
  // console.log({filtered})
  // setTargetAngles(focused, focused);
  // console.log({filtered})
  // Data deve essere
  // console.log({fd: focused.descendants().slice(1, 50)})
  const mul = window.OS_TYPE === "windows" ? 1024 : 1000;

  let path = innerG
    .selectAll<SVGPathElement, D3HierarchyDiskItem>("path")
    .data(filtered, (d) => d.data.id)
    .join(
      (enter) => {
        let xx = enter
          .append("path")
          .attr("data-testid", "chart-arc")
          .attr("data-node-id", (d) => d.data.id)
          .attr("data-node-name", (d) => d.data.name)
          .attr("fill", getChartColor)
          .attr("fill-opacity", (d) => baseArcOpacity(d))
          .attr("stroke", "#2f3746")
          .attr("stroke-width", 0.55)
          .attr("stroke-linejoin", "round")
          .attr("d", (d) => arc(d.current))
          .on("click", arcClickHandler)
          .on("mouseover", (e, p) => hoverHandler(e, p))
          .on("mouseleave", () => clearHoverHandler())
          .on("contextmenu", (e, p) => contextMenuHandler(e, p));
        // Add Title
        xx.append("title").text((d) => titleText(d, mul));
        return xx;
      },
      (update) => {
        update.select("title").text((d) => titleText(d, mul));
        return update
          .attr("data-testid", "chart-arc")
          .attr("data-node-id", (d) => d.data.id)
          .attr("data-node-name", (d) => d.data.name)
          .attr("fill", getChartColor)
          .attr("fill-opacity", (d) => baseArcOpacity(d))
          .attr("stroke", "#2f3746")
          .attr("stroke-width", 0.55)
          .attr("d", (d) => arc(d.current));
      }
    );

  return path;
};

interface GetChartCallbacks {
  arcClicked: (e: any, node: D3HierarchyDiskItem) => D3HierarchyDiskItem;
  arcHover: (e: any, node: D3HierarchyDiskItem) => void;
  arcContextMenu: (e: any, node: D3HierarchyDiskItem) => void;
  hoverCleared: () => void;
  centerHover: (e: any, node: D3HierarchyDiskItem) => void;
}
export const getChart = (
  root: D3HierarchyDiskItem,
  svgElem: SVGSVGElement,
  {
    arcClicked,
    arcHover,
    arcContextMenu,
    hoverCleared,
    centerHover,
  }: GetChartCallbacks
) => {
  // Map a value to unique color
  let current = root;
  let hoveredNodeId: string | null = null;
  // let color = d3.scaleOrdinal(
  //   d3.quantize(d3.interpolateRainbow, root.children!.length + 3)
  // );

  // Set View Box And Font
  let svg = d3
    .select<SVGSVGElement, D3HierarchyDiskItem>(svgElem)
    .attr("viewBox", [0, 0, width, width])
    .style("font", "10px sans-serif");

  // Center
  let g = svg
    .append("g")
    .attr("transform", `translate(${width / 2},${width / 2})`);

  g.append("circle")
    .datum(root)
    .attr("data-testid", "chart-hover-reset")
    .attr("r", radius * maxVisibleRadiusUnits)
    .attr("fill", "transparent")
    .attr("pointer-events", "all")
    .on("mouseover", () => clearHoverHandler());

  let innerG = g.append("g");

  // Back to parent click
  let backElement = g
    .append("circle")
    .datum(root)
    .attr("r", radius)
    .attr("fill", "none")
    .attr("pointer-events", "all")
    .on("click", (e, p) => centerClickHandler(e, p))
    .on("mouseover", (e, p) => centerHoverHandler(e, p));

  let path = updateData(
    root,
    root,
    innerG,
    arcClickHandler,
    arcHoverHandler,
    clearHoverHandler,
    arcContextMenuHandler
  );

  const setHoveredNode = (node: D3HierarchyDiskItem | null) => {
    hoveredNodeId = node?.data.id ?? null;
    applyHoverState(path, hoveredNodeId);
  };

  function centerHoverHandler(e: any, node: D3HierarchyDiskItem) {
    setHoveredNode(null);
    centerHover(e, node);
  }
  function arcHoverHandler(e: any, node: D3HierarchyDiskItem) {
    setHoveredNode(node);
    arcHover(e, node);
  }
  function arcContextMenuHandler(e: any, node: D3HierarchyDiskItem) {
    setHoveredNode(node);
    arcContextMenu(e, node);
  }
  function clearHoverHandler() {
    setHoveredNode(null);
    hoverCleared();
  }
  function centerClickHandler(e: any, focusedNode: D3HierarchyDiskItem) {
    // getNodeData(e, focusedNode);
    if (current === root) {
      return;
    }
    current = focusedNode;

    arcClicked(e, focusedNode);
    path = updateData(
      root,
      focusedNode,
      innerG,
      arcClickHandler,
      arcHoverHandler,
      clearHoverHandler,
      arcContextMenuHandler
    );
    backElement.datum(focusedNode.parent || root);
    // setTargetAngles(root, focusedNode);
    animateToTarget(g, path, () => hoveredNodeId);
  }

  function arcClickHandler(event: any, focusedNode: D3HierarchyDiskItem) {
    if (!focusedNode.children && !focusedNode.data.isDirectory) {
      return;
    }

    current = focusedNode;
    arcClicked(event, focusedNode);
    path = updateData(
      root,
      focusedNode,
      innerG,
      arcClickHandler,
      arcHoverHandler,
      clearHoverHandler,
      arcContextMenuHandler
    );
    backElement.datum(focusedNode.parent || root);
    // setTargetAngles(focusedNode.parent || root, focusedNode);
    // console.log({aft: root})

    animateToTarget(g, path, () => hoveredNodeId);
    // const clickRes = getNodeData(event, focusedNode);
    // if (clickRes) {
    //   // root = clickRes;
    //   path = updateData(root, clickRes, innerG, color, arcClickHandler, arcHoverHandler);
    //   // Set Parent Node to current node or root if no parent
    //   parent.datum(focusedNode.parent || root);
    //   setTargetAngles(root, focusedNode);
    //   animateToTarget(g, path);
    // }
  }

  return {
    focusDirectory: (node: D3HierarchyDiskItem) => {
      arcClickHandler(null, node);
    },
    backToParent: (node: D3HierarchyDiskItem) => {
      centerClickHandler(null, node);
    },
    setHoveredNode,
    deleteNodes: (nodes: Array<D3HierarchyDiskItem>) => {
      nodes.forEach((node) => {
        node
          .ancestors()
          .slice(1)
          .forEach((anc) => {
            // console.log({anc, prev: anc.value, minus: node.value, node});
            (anc as any).value -= node.value || 0;
            (anc as any).data.value -= node.value || 0;
          });
        node.parent!.children = node.parent!.children!.filter(
          (i: any) => i !== node
        );
      });
      root = d3.partition<DiskItem>().size([2 * Math.PI, root.height + 1])(
        root
      ) as D3HierarchyDiskItem;
      path = updateData(
        root,
        current,
        innerG,
        arcClickHandler,
        arcHoverHandler,
        clearHoverHandler,
        arcContextMenuHandler
      );
      animateToTarget(g, path, () => hoveredNodeId);
    },
  };
};
