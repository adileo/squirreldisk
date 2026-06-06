export const formatScannedAt = (timestamp?: number | null) => {
  if (!timestamp) {
    return null;
  }

  const scannedAt = new Date(timestamp);
  if (Number.isNaN(scannedAt.getTime())) {
    return null;
  }

  const elapsedMs = Math.max(0, Date.now() - timestamp);
  const minute = 60 * 1000;
  const hour = 60 * minute;
  const day = 24 * hour;

  if (elapsedMs < minute) {
    return "Scanned just now";
  }

  if (elapsedMs < hour) {
    const minutes = Math.floor(elapsedMs / minute);
    return `Scanned ${minutes}m ago`;
  }

  if (elapsedMs < day) {
    const hours = Math.floor(elapsedMs / hour);
    return `Scanned ${hours}h ago`;
  }

  const sameYear = scannedAt.getFullYear() === new Date().getFullYear();
  const formatted = scannedAt.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    ...(sameYear ? {} : { year: "numeric" }),
    hour: "numeric",
    minute: "2-digit",
  });

  return `Scanned ${formatted}`;
};
