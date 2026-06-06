import { describe, expect, it } from "vitest";

import { getChartColor } from "./chartColors";

const makeNode = (
  target: D3HierarchyDiskItemArc,
  overrides: Partial<DiskItem> = {}
) =>
  ({
    data: {
      id: "node",
      name: "node",
      value: 1,
      size: 1,
      isDirectory: true,
      children: [],
      ...overrides,
    },
    target,
    current: target,
    x0: target.x0,
    x1: target.x1,
    y0: target.y0,
    y1: target.y1,
  } as D3HierarchyDiskItem);

describe("getChartColor", () => {
  it("keeps synthetic smaller items neutral", () => {
    expect(
      getChartColor(
        makeNode({ x0: 0, x1: Math.PI, y0: 1, y1: 2 }, { synthetic: true })
      )
    ).toBe("#64748b");
  });

  it("keeps file leaves neutral", () => {
    expect(
      getChartColor(
        makeNode(
          { x0: 0, x1: Math.PI, y0: 4, y1: 5 },
          { isDirectory: false }
        )
      )
    ).toBe("#6b7280");
  });

  it("uses DaisyDisk's ring depth saturation curve", () => {
    const centeredOnCyan = {
      x0: Math.PI - 0.05,
      x1: Math.PI + 0.05,
    };

    expect(getChartColor(makeNode({ ...centeredOnCyan, y0: 1, y1: 2 }))).toBe(
      "#40ffff"
    );
    expect(getChartColor(makeNode({ ...centeredOnCyan, y0: 5, y1: 6 }))).toBe(
      "#7cffff"
    );
  });

  it("colors from the displayed target arc midpoint", () => {
    const currentOnly = makeNode({ x0: 0, x1: 0.1, y0: 1, y1: 2 });
    const focusedTarget = makeNode({
      x0: Math.PI,
      x1: Math.PI + 0.1,
      y0: 1,
      y1: 2,
    });
    focusedTarget.current = currentOnly.current;

    expect(getChartColor(focusedTarget)).toBe(
      getChartColor(makeNode(focusedTarget.target))
    );
    expect(getChartColor(focusedTarget)).not.toBe(getChartColor(currentOnly));
  });
});
