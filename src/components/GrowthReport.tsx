import prettyBytes from "pretty-bytes";

interface GrowthReportProps {
  report: ScanHistoryReport | null;
}

const formatDelta = (value: number) =>
  `${value >= 0 ? "+" : "-"}${prettyBytes(Math.abs(value))}`;

const formatDateTime = (timestamp: number) =>
  new Date(timestamp).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });

const compactPath = (rootPath: string, path: string) => {
  if (path === rootPath) {
    return path;
  }

  const prefix = rootPath.endsWith("/") ? rootPath : `${rootPath}/`;
  if (path.startsWith(prefix)) {
    return path.slice(prefix.length);
  }

  return path;
};

const GrowthReport = ({ report }: GrowthReportProps) => {
  if (!report) {
    return null;
  }

  return (
    <div className="mb-2 rounded-md bg-gray-800/90 p-3 text-white">
      <div className="mb-2 flex items-start justify-between gap-3">
        <div>
          <div className="text-xs font-semibold uppercase tracking-wide text-gray-300">
            Growth since last scan
          </div>
          <div className="mt-1 text-[11px] text-gray-400">
            {report.previousTimestamp != null
              ? `${formatDateTime(report.previousTimestamp)} → ${formatDateTime(
                  report.currentTimestamp
                )}`
              : "First snapshot saved for this path"}
          </div>
        </div>
        <div className="shrink-0 rounded bg-gray-700 px-2 py-1 text-right text-xs">
          {report.totalDelta == null
            ? `${report.snapshotCount} snapshot`
            : formatDelta(report.totalDelta)}
        </div>
      </div>

      {report.topGrowth.length === 0 && (
        <div className="text-xs text-gray-400">
          Scan this same path again later to see the largest growth contributors.
        </div>
      )}

      {report.topGrowth.length > 0 && (
        <div className="space-y-1">
          {report.topGrowth.map((entry) => (
            <div
              key={entry.path}
              title={entry.path}
              className="grid grid-cols-[1fr_auto] gap-3 rounded bg-gray-900/70 p-2"
            >
              <div className="min-w-0">
                <div className="truncate text-xs">
                  {compactPath(report.rootPath, entry.path)}
                </div>
                <div className="mt-0.5 text-[11px] text-gray-500">
                  now {prettyBytes(entry.size)}
                  {entry.previousSize > 0 &&
                    ` · was ${prettyBytes(entry.previousSize)}`}
                </div>
              </div>
              <div className="text-right text-xs text-green-300">
                {formatDelta(entry.delta)}
                {entry.dailyRate != null && (
                  <div className="text-[11px] text-gray-500">
                    {prettyBytes(entry.dailyRate)}/day
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

export default GrowthReport;
