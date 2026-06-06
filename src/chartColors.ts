const tau = Math.PI * 2;
const fileColor = "#6b7280";
const smallerItemsColor = "#64748b";

const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

const normalizeUnit = (value: number) => ((value % 1) + 1) % 1;

const channelToHex = (value: number) =>
  Math.round(clamp(value, 0, 1) * 255)
    .toString(16)
    .padStart(2, "0");

const hsvToHex = (hue: number, saturation: number, value: number) => {
  const h = normalizeUnit(hue) * 6;
  const i = Math.floor(h);
  const f = h - i;
  const p = value * (1 - saturation);
  const q = value * (1 - f * saturation);
  const t = value * (1 - (1 - f) * saturation);

  let r = value;
  let g = t;
  let b = p;

  switch (i % 6) {
    case 1:
      r = q;
      g = value;
      b = p;
      break;
    case 2:
      r = p;
      g = value;
      b = t;
      break;
    case 3:
      r = p;
      g = q;
      b = value;
      break;
    case 4:
      r = t;
      g = p;
      b = value;
      break;
    case 5:
      r = value;
      g = p;
      b = q;
      break;
  }

  return `#${channelToHex(r)}${channelToHex(g)}${channelToHex(b)}`;
};

const displayedArcOf = (node: D3HierarchyDiskItem) => {
  const arc = node.target || node.current || node;

  if (
    Number.isFinite(arc.x0) &&
    Number.isFinite(arc.x1) &&
    Number.isFinite(arc.y0) &&
    arc.x1 > arc.x0
  ) {
    return arc;
  }

  return node;
};

export const getChartColor = (node: D3HierarchyDiskItem) => {
  if (node.data.synthetic || node.data.name === "Smaller Items") {
    return smallerItemsColor;
  }

  if (!node.data.isDirectory && !node.children) {
    return fileColor;
  }

  const arc = displayedArcOf(node);
  const hue = normalizeUnit((arc.x0 + arc.x1) / (2 * tau));
  const ringDepth = Math.max(1, Math.round(arc.y0));
  const saturation = clamp(0.5 + 0.5 * Math.pow(2, -ringDepth), 0.5, 0.75);

  return hsvToHex(hue, saturation, 1);
};
