import { describe, expect, it } from "vitest";

import {
  addRestrictedPathsToTree,
  groupChildrenByBasePath,
  itemMap,
} from "./pruneData";

describe("scan tree path shaping", () => {
  it("groups absolute pdu children under the selected base without double slashes", () => {
    const grouped = groupChildrenByBasePath(
      {
        name: "(total)",
        size: 30,
        value: 30,
        isDirectory: true,
        children: [
          {
            name: "/Users/sirius/Library",
            size: 20,
            value: 20,
            isDirectory: true,
            children: [],
          },
          {
            name: "/Users/sirius/Downloads/file.zip",
            size: 10,
            value: 10,
            isDirectory: false,
            children: [],
          },
        ],
      },
      "/Users/sirius"
    );

    const mapped = itemMap(grouped);

    expect(mapped.id).toBe("/Users/sirius");
    expect(mapped.children.map((child: DiskItem) => child.id)).toEqual([
      "/Users/sirius/Library",
      "/Users/sirius/Downloads",
    ]);
    expect(mapped.children[1].children[0].id).toBe(
      "/Users/sirius/Downloads/file.zip"
    );
    expect(JSON.stringify(mapped)).not.toContain("//Library");
    expect(JSON.stringify(mapped)).not.toContain("//Downloads");
  });

  it("merges duplicate directories that resolve to the same displayed path", () => {
    const grouped = groupChildrenByBasePath(
      {
        name: "(total)",
        size: 18,
        value: 18,
        isDirectory: true,
        children: [
          {
            name: "/Library",
            size: 5,
            value: 5,
            isDirectory: true,
            children: [
              {
                name: "A",
                size: 5,
                value: 5,
                isDirectory: false,
                children: [],
              },
            ],
          },
          {
            name: "Library",
            size: 7,
            value: 7,
            isDirectory: true,
            children: [
              {
                name: "B",
                size: 7,
                value: 7,
                isDirectory: false,
                children: [],
              },
            ],
          },
          {
            name: "/Library/Caches",
            size: 6,
            value: 6,
            isDirectory: true,
            children: [],
          },
        ],
      },
      "/"
    );

    const mapped = itemMap(grouped);
    const libraries = mapped.children.filter(
      (child: DiskItem) => child.name === "Library"
    );

    expect(libraries).toHaveLength(1);
    expect(libraries[0].size).toBe(18);
    expect(libraries[0].children.map((child: DiskItem) => child.name)).toEqual([
      "B",
      "Caches",
      "A",
    ]);
    expect(libraries[0].children.map((child: DiskItem) => child.id)).toEqual([
      "/Library/B",
      "/Library/Caches",
      "/Library/A",
    ]);
  });

  it("nests restricted subdirs into existing pdu folders that have no isDirectory flag", () => {
    const root = {
      name: "C:/",
      size: 1000,
      value: 1000,
      children: [
        {
          name: "Program Files",
          size: 600,
          value: 600,
          children: [
            { name: "Mozilla Firefox", size: 600, value: 600, children: [] },
          ],
        },
      ],
    };

    const withRestricted = addRestrictedPathsToTree(root, "C:/", [
      {
        path: "C:\\Program Files\\WindowsApps",
        operation: "read_dir",
        message: "Access is denied.",
      },
    ]);

    const programFiles = withRestricted.children.filter(
      (child: DiskItem) => child.name === "Program Files"
    );

    expect(programFiles).toHaveLength(1);
    expect(programFiles[0].children.map((c: DiskItem) => c.name)).toEqual([
      "Mozilla Firefox",
      "WindowsApps",
    ]);
    const restrictedChild = programFiles[0].children.find(
      (c: DiskItem) => c.name === "WindowsApps"
    );
    expect(restrictedChild).toMatchObject({
      restricted: true,
      restrictedReason: "Access is denied.",
    });
  });

  it("adds inaccessible folders inside the scanned tree", () => {
    const root = itemMap({
      name: "/",
      id: "",
      size: 100,
      value: 100,
      isDirectory: true,
      children: [
        {
          name: "Users",
          id: "",
          size: 100,
          value: 100,
          isDirectory: true,
          children: [],
        },
      ],
    });

    const withRestricted = addRestrictedPathsToTree(root, "/", [
      {
        path: "/private/var/db",
        operation: "read_dir",
        message: "Permission denied",
      },
    ]);

    const privateNode = withRestricted.children.find(
      (child: DiskItem) => child.name === "private"
    );
    const dbNode = privateNode?.children[0]?.children[0];

    expect(dbNode).toMatchObject({
      name: "db",
      restricted: true,
      restrictedPath: "/private/var/db",
      restrictedReason: "Permission denied",
    });
  });
});
