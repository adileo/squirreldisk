import * as d3 from "d3";
import prettyBytes from "pretty-bytes";
import { v4 as uuidv4 } from "uuid";
var width = 600;
var radius = width / 10;
var arc = d3
  .arc<D3HierarchyDiskItemArc>()
  .startAngle((d) => d.x0)
  .endAngle((d) => d.x1)
  .padAngle((d) => Math.min((d.x1 - d.x0) / 2, 0.005))
  .padRadius(radius * 1.5)
  .innerRadius((d) => d.y0 * radius)
  .outerRadius((d) => Math.max(d.y0 * radius, d.y1 * radius - 3));

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
  // d.y1 <= 4 => Hide arcs with outer radius larger than 4
  // d.y0 >= 1 => Hide root arc (spot in the middle)
  // d.x1 > d.x0 => hide non focused arcs
  return d.y1 <= 4 && d.y0 >= 1 && d.x1 > d.x0;
};

const setTargetAngles = (
  root: D3HierarchyDiskItem,
  focusedNode: D3HierarchyDiskItem
) => {
  // Focus on current slice
  root.each((d: any) => {
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

const setTargetAnglesNew = (
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
  >
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
      arcVisible(d.target) ? (d.children ? 0.6 : 0.4) : 0
    )
    .attrTween("d", (d) => () => {
      if (!d.current) {
        debugger;
      }
      return arc(d.current)!;
    })
    .end()
    .then(() => {
      // Cut OUT
      // delayedOp()
    })
    .catch((e) => {
      // console.error(e);
    });
};
let gcolor: d3.ScaleOrdinal<string, string, never> | null;
const updateData = (
  root: D3HierarchyDiskItem,
  focused: D3HierarchyDiskItem,
  innerG: d3.Selection<SVGGElement, D3HierarchyDiskItem, null, undefined>,
  // color: d3.ScaleOrdinal<string, string, never>,
  arcClickHandler: (event: any, focusedNode: D3HierarchyDiskItem) => void,
  hoverHandler: (event: any, focusedNode: D3HierarchyDiskItem) => void
) => {
  let filtered = [...focused.ancestors().slice(-1)];
  let initialDepth = focused.depth;
  let maxDepth = initialDepth + 3;
  let overallSize = focused.value || 0;
  let accumulator: D3HierarchyDiskItem | null = null;
  let accumulatorLastParent = null;
  let skipMap: any = {};
  let colorCounter = 0;

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
      break;
    }
    if ((item.value || 0) / overallSize > 0.005) {
      // Includo item grandi
      if (item.parent === root) {
        colorCounter += 1;
      }
      filtered.push(item);
    } else {
      // Accumulo item piccoli
      if (accumulator) {
        skipMap[item.data.id] = true;
        accumulator.data.value! += item.value ? item.value : 0;
        (accumulator as any).value += item.value ? item.value : 0;

        (accumulator as any).current.x1 = item.current.x1;
        (accumulator as any).x1 = item.x1;
      } else {
        skipMap[item.data.id] = true;
        let v: DiskItem = {
          id: uuidv4(),
          isDirectory: false,
          name: "Smaller Items",
          value: item.value || 0,
          data: item.value || 0,
          children: [],
        };
        accumulator = d3.hierarchy(v) as D3HierarchyDiskItem;
        accumulator.parent = item.parent;
        (accumulator as any).value = item.value || 0;
        (accumulator as any).x0 = item.x0;
        (accumulator as any).x1 = item.x1;
        (accumulator as any).y0 = item.y0;
        (accumulator as any).y1 = item.y1;

        (accumulator as any).current = { ...item.current };
        (accumulator as any).height = item.height;
        (accumulator as any).depth = item.depth;
        accumulatorLastParent = item.parent;
      }
    }
  }
  if (!gcolor && focused === root) {
    console.log("SET COLOR", colorCounter);
    gcolor = d3.scaleOrdinal(
      d3.quantize(d3.interpolateRainbow, colorCounter + 2)
    );
  }
  setTargetAnglesNew(filtered, focused);
  // console.log({filtered})
  // console.log({filtered})
  // setTargetAngles(focused, focused);
  // console.log({filtered})
  // Data deve essere
  // console.log({fd: focused.descendants().slice(1, 50)})
  let path = innerG
    .selectAll<SVGPathElement, D3HierarchyDiskItem>("path")
    .data(filtered, (d) => d.data.id)
    .join(
      (enter) => {
        let xx = enter
          .append("path")
          .attr("fill", (d) => {
            while (d.depth > 1) d = d.parent!;
            return gcolor!(d.data.name);
          })
          .attr("fill-opacity", (d) =>
            arcVisible(d.current) ? (d.children ? 0.6 : 0.4) : 0
          )
          .attr("d", (d) => arc(d.current))
          .style("cursor", "pointer")
          .on("click", arcClickHandler)
          .on("mouseover", (e, p) => hoverHandler(e, p));
        // Add Title
        xx.append("title").text(
          (d) =>
            `${d
              .ancestors()
              .map((d) => d.data.name)
              .reverse()
              .join("/")}\n${prettyBytes(d.value || 0)}`
        );
        return xx;
      },
      (update) =>
        update
          .attr("fill", (d) => {
            while (d.depth > 1) d = d.parent!;
            return gcolor!(d.data.name);
          })
          .attr("fill-opacity", (d) =>
            arcVisible(d.current) ? (d.children ? 0.6 : 0.4) : 0
          )
          .attr("d", (d) => arc(d.current))
    );

  return path;
};

interface GetChartCallbacks {
  arcClicked: (e: any, node: D3HierarchyDiskItem) => D3HierarchyDiskItem;
  arcHover: (e: any, node: D3HierarchyDiskItem) => void;
  centerHover: (e: any, node: D3HierarchyDiskItem) => void;
}
export const getChart = (
  root: D3HierarchyDiskItem,
  svgElem: SVGSVGElement,
  { arcClicked, arcHover, centerHover }: GetChartCallbacks
) => {
  // Map a value to unique color
  let current = root;
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

  let path = updateData(root, root, innerG, arcClickHandler, arcHoverHandler);

  function centerHoverHandler(e: any, node: D3HierarchyDiskItem) {
    centerHover(e, node);
  }
  function arcHoverHandler(e: any, node: D3HierarchyDiskItem) {
    arcHover(e, node);
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
      arcHoverHandler
    );
    backElement.datum(focusedNode.parent || root);
    // setTargetAngles(root, focusedNode);
    animateToTarget(g, path);
  }

  function arcClickHandler(event: any, focusedNode: D3HierarchyDiskItem) {
    if (!focusedNode.children) {
      // TODO: Handle click on sidebar focus
      return;
    }

    current = focusedNode;
    arcClicked(event, focusedNode);
    path = updateData(
      root,
      focusedNode,
      innerG,
      arcClickHandler,
      arcHoverHandler
    );
    backElement.datum(focusedNode.parent || root);
    // setTargetAngles(focusedNode.parent || root, focusedNode);
    // console.log({aft: root})

    animateToTarget(g, path);
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
        arcHoverHandler
      );
      animateToTarget(g, path);
    },
  };
};
