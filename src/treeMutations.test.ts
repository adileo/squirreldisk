import { describe, expect, it } from "vitest";

import { removeNodesFromTree } from "./treeMutations";

const makeTree = (): DiskItem => ({
  id: "/",
  name: "/",
  value: 100,
  size: 100,
  isDirectory: true,
  children: [
    {
      id: "/Users",
      name: "Users",
      value: 75,
      size: 75,
      isDirectory: true,
      children: [
        {
          id: "/Users/a.mov",
          name: "a.mov",
          value: 40,
          size: 40,
          isDirectory: false,
          children: [],
        },
        {
          id: "/Users/b.mov",
          name: "b.mov",
          value: 35,
          size: 35,
          isDirectory: false,
          children: [],
        },
      ],
    },
    {
      id: "/Applications",
      name: "Applications",
      value: 25,
      size: 25,
      isDirectory: true,
      children: [],
    },
  ],
});

describe("removeNodesFromTree", () => {
  it("removes confirmed deletions and recalculates ancestor sizes", () => {
    const original = makeTree();
    const result = removeNodesFromTree(original, new Set(["/Users/a.mov"]));

    expect(result.removedBytes).toBe(40);
    expect(result.node?.size).toBe(60);
    expect(result.node?.children[0].size).toBe(35);
    expect(result.node?.children[0].children.map((child) => child.id)).toEqual([
      "/Users/b.mov",
    ]);
  });

  it("does not mutate the cached tree when preparing the updated tree", () => {
    const original = makeTree();

    removeNodesFromTree(original, new Set(["/Users/a.mov"]));

    expect(original.size).toBe(100);
    expect(original.children[0].size).toBe(75);
    expect(original.children[0].children.map((child) => child.id)).toEqual([
      "/Users/a.mov",
      "/Users/b.mov",
    ]);
  });
});
