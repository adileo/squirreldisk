function formatBytes(bytes: number, decimals = 2) {
    if (bytes < 1) return '0 B'

    const div = window.OS_TYPE === "Windows_NT" ? 1024 : 1000;
    const precision = decimals < 0 ? 0 : decimals
    const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB', 'EB', 'ZB', 'YB']
    const order = Math.floor(Math.log(bytes) / Math.log(div))

    return `${parseFloat((bytes / Math.pow(div, order)).toFixed(precision))} ${units[order]}`
}