import { useEffect, useState } from 'react'
import {
  type DownloadEvent,
  type Update,
  downloadAndInstall,
  loadPrefs,
  maybeAutoCheck,
  patchPrefs,
} from '../lib/updater'

type Phase = 'idle' | 'downloading' | 'installed' | 'failed'

export default function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null)
  const [phase, setPhase] = useState<Phase>('idle')
  const [progress, setProgress] = useState<{ downloaded: number; total?: number }>({
    downloaded: 0,
  })
  const [error, setError] = useState<string | null>(null)
  const [autoInstall, setAutoInstall] = useState(loadPrefs().autoInstall)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const found = await maybeAutoCheck()
      if (cancelled || !found || !found.updateAvailable) return
      const prefs = loadPrefs()
      if (prefs.dismissedVersion === found.version) return
      setUpdate(found)
      if (prefs.autoInstall) {
        void runInstall(found)
      }
    })()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  async function runInstall(target: Update) {
    setPhase('downloading')
    setError(null)
    try {
      await downloadAndInstall(target, (event: DownloadEvent) => {
        if (event.event === 'started') {
          setProgress({ downloaded: 0, total: event.data.contentLength })
        } else if (event.event === 'progress') {
          setProgress({ downloaded: event.data.downloaded, total: progress.total })
        } else if (event.event === 'finished') {
          setProgress({ downloaded: event.data.total, total: event.data.total })
        }
      })
      setPhase('installed')
    } catch (err) {
      setPhase('failed')
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  if (!update) return null

  const dismiss = () => {
    patchPrefs({ dismissedVersion: update.version })
    setUpdate(null)
  }

  const toggleAutoInstall = (value: boolean) => {
    setAutoInstall(value)
    patchPrefs({ autoInstall: value })
  }

  const pct =
    progress.total && progress.total > 0
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : null

  return (
    <div
      className="flex items-center gap-3 px-3 py-2 text-sm text-white"
      style={{ background: '#0F1831', borderBottom: '1px solid rgba(255,255,255,0.08)' }}
    >
      <div className="flex-1">
        {phase === 'installed' ? (
          <span>Installed {update.version}. Restart SquirrelDisk to apply.</span>
        ) : phase === 'downloading' ? (
          <span>
            Downloading {update.version}
            {pct !== null ? ` — ${pct}%` : '…'}
          </span>
        ) : phase === 'failed' ? (
          <span className="text-red-300">Update failed: {error}</span>
        ) : (
          <span>
            Update available: <strong>{update.version}</strong> (you're on {update.currentVersion})
          </span>
        )}
      </div>
      <label className="flex items-center gap-1.5 text-xs text-gray-300">
        <input
          type="checkbox"
          checked={autoInstall}
          onChange={(e) => toggleAutoInstall(e.target.checked)}
        />
        Install automatically
      </label>
      {phase === 'idle' && (
        <button
          className="px-3 py-1 rounded bg-white/10 hover:bg-white/20"
          onClick={() => runInstall(update)}
        >
          Install
        </button>
      )}
      {(phase === 'idle' || phase === 'failed' || phase === 'installed') && (
        <button
          aria-label="Dismiss"
          className="px-1.5 text-gray-400 hover:text-white"
          onClick={dismiss}
        >
          ×
        </button>
      )}
    </div>
  )
}
