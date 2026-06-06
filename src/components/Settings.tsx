import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { getVersion } from '@tauri-apps/api/app'
import {
  type DownloadEvent,
  type Update,
  checkForUpdate,
  downloadAndInstall,
  loadPrefs,
  patchPrefs,
} from '../lib/updater'

type Status =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'available'; update: Update }
  | { kind: 'upToDate' }
  | { kind: 'installing'; pct: number | null }
  | { kind: 'installed'; version: string }
  | { kind: 'error'; message: string }

export default function Settings() {
  const navigate = useNavigate()
  const [version, setVersion] = useState<string>('')
  const [prefs, setPrefs] = useState(loadPrefs())
  const [status, setStatus] = useState<Status>({ kind: 'idle' })

  useEffect(() => {
    void getVersion().then(setVersion)
  }, [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') navigate('/')
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [navigate])

  const close = () => navigate('/')

  const apply = (patch: Parameters<typeof patchPrefs>[0]) => {
    setPrefs(patchPrefs(patch))
  }

  async function manualCheck() {
    setStatus({ kind: 'checking' })
    try {
      const update = await checkForUpdate()
      apply({ lastCheckMs: Date.now() })
      if (update && update.updateAvailable) {
        setStatus({ kind: 'available', update })
      } else {
        setStatus({ kind: 'upToDate' })
      }
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) })
    }
  }

  async function install(update: Update) {
    setStatus({ kind: 'installing', pct: null })
    try {
      await downloadAndInstall(update, (event: DownloadEvent) => {
        if (event.event === 'started') {
          setStatus({ kind: 'installing', pct: event.data.contentLength ? 0 : null })
        } else if (event.event === 'progress' && event.data) {
          setStatus((prev) => {
            if (prev.kind !== 'installing') return prev
            return { kind: 'installing', pct: prev.pct === null ? null : 50 }
          })
        }
      })
      setStatus({ kind: 'installed', version: update.version })
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) })
    }
  }

  return (
    <>
      <div
        className="fixed inset-0 z-40"
        style={{ background: 'rgba(0,0,0,0.45)' }}
        onClick={close}
        aria-hidden
      />
      <div
        role="dialog"
        aria-label="Settings"
        className="fixed z-50 left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 w-[min(560px,90vw)] max-h-[80vh] overflow-y-auto rounded-lg shadow-2xl text-white"
        style={{ background: '#0F1831', border: '1px solid rgba(255,255,255,0.08)' }}
      >
        <div className="flex items-center justify-between px-5 py-3 border-b border-white/10">
          <h1 className="text-base font-semibold">Settings</h1>
          <button
            aria-label="Close Settings"
            className="text-gray-400 hover:text-white p-1"
            onClick={close}
          >
            <svg xmlns="http://www.w3.org/2000/svg" className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="p-5">
        <section className="mb-2">
        <h2 className="text-sm uppercase tracking-wide text-gray-400 mb-2">Updates</h2>
        <div className="text-sm text-gray-300 mb-3">Current version: {version || '—'}</div>

        <div className="mb-3">
          <button
            className="px-3 py-1.5 rounded bg-white/10 hover:bg-white/20 text-sm"
            onClick={manualCheck}
            disabled={status.kind === 'checking' || status.kind === 'installing'}
          >
            {status.kind === 'checking' ? 'Checking…' : 'Check for updates'}
          </button>

          {status.kind === 'available' && (
            <div className="mt-2 p-3 rounded bg-white/5 text-sm">
              <div className="mb-2">
                Version <strong>{status.update.version}</strong> is available.
              </div>
              {status.update.notes && (
                <div className="text-gray-400 mb-2 whitespace-pre-line">{status.update.notes}</div>
              )}
              <button
                className="px-3 py-1.5 rounded bg-white/10 hover:bg-white/20 text-sm"
                onClick={() => install(status.update)}
              >
                Install now
              </button>
            </div>
          )}
          {status.kind === 'upToDate' && (
            <div className="mt-2 text-sm text-green-300">You're up to date.</div>
          )}
          {status.kind === 'installing' && (
            <div className="mt-2 text-sm text-gray-300">
              Installing{status.pct !== null ? ` — ${status.pct}%` : '…'}
            </div>
          )}
          {status.kind === 'installed' && (
            <div className="mt-2 text-sm text-green-300">
              Installed {status.version}. Restart SquirrelDisk to apply.
            </div>
          )}
          {status.kind === 'error' && (
            <div className="mt-2 text-sm text-red-300">{status.message}</div>
          )}

          <div className="text-xs text-gray-500 mt-2 h-4">
            {prefs.lastCheckMs > 0
              ? `Last checked ${new Date(prefs.lastCheckMs).toLocaleString()}`
              : ''}
          </div>
        </div>

        <label className="flex items-center gap-2 mb-2 text-sm">
          <input
            type="checkbox"
            checked={prefs.autoCheck}
            disabled={prefs.autoInstall}
            onChange={(e) => apply({ autoCheck: e.target.checked })}
          />
          Check for updates automatically
        </label>
        <label className="flex items-center gap-2 mb-3 text-sm">
          <input
            type="checkbox"
            checked={prefs.autoInstall}
            onChange={(e) => apply({ autoInstall: e.target.checked })}
          />
          Install updates automatically <span className="text-xs text-gray-500">(applies on next start)</span>
        </label>
      </section>
        </div>
      </div>
    </>
  )
}
