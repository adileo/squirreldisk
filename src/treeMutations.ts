export const removeNodesFromTree = (
  node: DiskItem,
  idsToRemove: Set<string>
): { node: DiskItem | null; removedBytes: number } => {
  if (idsToRemove.has(node.id)) {
    return { node: null, removedBytes: node.size || 0 };
  }

  if (!node.children || node.children.length === 0) {
    return { node: { ...node }, removedBytes: 0 };
  }

  const results = node.children.map((child) =>
    removeNodesFromTree(child, idsToRemove)
  );
  const children = results
    .map((result) => result.node)
    .filter(Boolean) as Array<DiskItem>;
  const removedBytes = results.reduce(
    (sum, result) => sum + result.removedBytes,
    0
  );
  const size = Math.max((node.size || 0) - removedBytes, 0);

  return {
    node: {
      ...node,
      children,
      size,
      value: size,
    },
    removedBytes,
  };
};
