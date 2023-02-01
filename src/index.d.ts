declare module "mongoid-js";
declare module "shade-blend-color";

interface DiskItem {
  id: string;
  name: string;
  value: number;
  data: number;
  isDirectory: boolean;
  children: Array<DiskItem>;
}

interface D3HierarchyDiskItemArc {
  x0: number;
  x1: number;
  y0: number;
  y1: number;
}
interface D3HierarchyDiskItem extends d3.HierarchyRectangularNode<DiskItem> {
  target: D3HierarchyDiskItemArc;
  current: D3HierarchyDiskItemArc;
  parent: any;
  children: this[];
  data: DiskItem;
  each: any;
  //   value: number;
  //   height: number;
  //   depth: number;
  //   x0: number;
  //   x1: number;
  //   y0: number;
  //   y1: number;
}
