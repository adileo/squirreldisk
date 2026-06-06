import { check as pluginCheck, Update, type DownloadEvent } from './updater-api'

const PREFS_KEY = 'squirreldisk.updater.prefs.v1'
const DEFAULT_AUTO_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000 // 6h

export interface UpdaterPrefs {
  autoCheck: boolean
  autoInstall: boolean
  lastCheckMs: number
  lastNotifiedVersion: string | null
  dismissedVersion: string | null
}

const DEFAULT_PREFS: UpdaterPrefs = {
  autoCheck: true,
  autoInstall: false,
  lastCheckMs: 0,
  lastNotifiedVersion: null,
  dismissedVersion: null,
}

export function loadPrefs(): UpdaterPrefs {
  try {
    const raw = localStorage.getItem(PREFS_KEY)
    if (!raw) return { ...DEFAULT_PREFS }
    return { ...DEFAULT_PREFS, ...JSON.parse(raw) }
  } catch {
    return { ...DEFAULT_PREFS }
  }
}

export function savePrefs(prefs: UpdaterPrefs): void {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(prefs))
  } catch {
    /* ignore */
  }
}

export function patchPrefs(patch: Partial<UpdaterPrefs>): UpdaterPrefs {
  const next = { ...loadPrefs(), ...patch }
  // Auto-install implies auto-check.
  if (next.autoInstall) next.autoCheck = true
  savePrefs(next)
  return next
}

export async function checkForUpdate(): Promise<Update | null> {
  return pluginCheck()
}

export async function maybeAutoCheck(
  intervalMs: number = DEFAULT_AUTO_CHECK_INTERVAL_MS
): Promise<Update | null> {
  const prefs = loadPrefs()
  if (!prefs.autoCheck) return null
  if (Date.now() - prefs.lastCheckMs < intervalMs) return null
  try {
    const update = await pluginCheck()
    patchPrefs({ lastCheckMs: Date.now() })
    return update
  } catch {
    return null
  }
}

export async function downloadAndInstall(
  update: Update,
  onEvent?: (event: DownloadEvent) => void
): Promise<void> {
  await update.downloadAndInstall(onEvent)
  patchPrefs({ lastNotifiedVersion: update.version })
}

export type { Update, DownloadEvent }
