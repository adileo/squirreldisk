#!/usr/bin/env node

import { spawn, spawnSync } from 'node:child_process'
import {
  mkdtempSync,
  mkdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(__dirname, '..')
const devServerUrl = process.env.SQD_E2E_URL || 'http://127.0.0.1:1420'

const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

const assert = (condition, message) => {
  if (!condition) {
    throw new Error(message)
  }
}

const chromePath = () => {
  if (process.env.CHROME) return process.env.CHROME
  if (process.platform === 'darwin') {
    return '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
  }
  if (process.platform === 'linux') {
    return 'google-chrome'
  }
  return 'chrome'
}

const pduPath = () => {
  const platform = process.platform
  const arch = process.arch
  if (platform === 'darwin' && arch === 'arm64') {
    return join(repoRoot, 'src-tauri', 'bin', 'pdu-aarch64-apple-darwin')
  }
  if (platform === 'darwin') {
    return join(repoRoot, 'src-tauri', 'bin', 'pdu-x86_64-apple-darwin')
  }
  if (platform === 'linux') {
    return join(repoRoot, 'src-tauri', 'bin', 'pdu-x86_64-unknown-linux-gnu')
  }
  if (platform === 'win32') {
    return join(repoRoot, 'src-tauri', 'bin', 'pdu-x86_64-pc-windows-msvc.exe')
  }
  throw new Error(`Unsupported e2e platform: ${platform}`)
}

const runPduScan = (scanRoot) => {
  const args = ['--json-output', '--progress']
  if (process.platform !== 'win32') {
    args.push(
      '--deduplicate-hardlinks',
      '--omit-json-shared-details',
      '--omit-json-shared-summary',
    )
  }
  args.push('--threads=max', '--min-ratio=0.001', scanRoot)

  const result = spawnSync(pduPath(), args, {
    cwd: repoRoot,
    encoding: 'utf8',
  })

  if (result.status !== 0) {
    throw new Error(result.stderr || `pdu exited with ${result.status}`)
  }

  return JSON.parse(result.stdout)
}

const createFixture = () => {
  const root = mkdtempSync(join(tmpdir(), 'squirreldisk-sidebar-e2e-'))
  const files = [
    ['Users/sirius/Downloads/a.bin', 'a'.repeat(513)],
    ['Users/Shared/b.bin', 'b'.repeat(1027)],
    ['Library/Caches/c.bin', 'c'.repeat(2049)],
    ['Applications/d.bin', 'd'.repeat(4097)],
  ]

  for (const [relativePath, contents] of files) {
    const fullPath = join(root, relativePath)
    mkdirSync(dirname(fullPath), { recursive: true })
    writeFileSync(fullPath, contents)
  }

  return root
}

const writeHarness = ({ scanRoot, scanPayload }) => {
  const harnessPath = join(repoRoot, '.local', 'sidebar-pdu-e2e.html')
  mkdirSync(dirname(harnessPath), { recursive: true })
  const disk = {
    availableSpace: 1_000_000_000,
    isRemovable: false,
    name: 'E2E Disk',
    sMountPoint: scanRoot,
    totalSpace: 2_000_000_000,
  }

  const html = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>SquirrelDisk PDU Sidebar E2E</title>
  </head>
  <body>
    <div id="root"></div>
    <script>
      const disk = ${JSON.stringify(disk)};
      const scanPayload = ${JSON.stringify(scanPayload)};
      let nextCallbackId = 1;
      let nextEventId = 1;
      const listeners = new Map();

      const emit = (event, payload) => {
        for (const listener of listeners.values()) {
          if (listener.event === event) {
            window.__TAURI_INTERNALS__.runCallback(listener.handler, {
              event,
              id: listener.id,
              payload,
            });
          }
        }
      };

      window.__TAURI_OS_PLUGIN_INTERNALS__ = {
        arch: "aarch64",
        eol: "\\n",
        exe_extension: "",
        family: "unix",
        os_type: "macos",
        platform: "macos",
        version: "14.0.0",
      };
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: (_event, eventId) => {
          listeners.delete(eventId);
        },
      };
      window.__TAURI_INTERNALS__ = {
        callbacks: new Map(),
        convertFileSrc: (path) => path,
        metadata: {
          currentWebview: { label: "main" },
          currentWindow: { label: "main" },
        },
        transformCallback(callback, once) {
          const id = nextCallbackId++;
          this.callbacks.set(id, (payload) => {
            if (once) this.callbacks.delete(id);
            callback(payload);
          });
          return id;
        },
        unregisterCallback(id) {
          this.callbacks.delete(id);
        },
        runCallback(id, payload) {
          this.callbacks.get(id)?.(payload);
        },
        async invoke(cmd, args) {
          if (cmd === "plugin:app|version") return "0.3.11";
          if (cmd === "plugin:event|listen") {
            const id = nextEventId++;
            listeners.set(id, { id, event: args.event, handler: args.handler });
            return id;
          }
          if (cmd === "plugin:event|unlisten") {
            listeners.delete(args.eventId);
            return null;
          }
          if (cmd === "list_scan_snapshots") return [];
          if (cmd === "get_scan_snapshot") return null;
          if (cmd === "save_scan_snapshot") return null;
          if (cmd === "delete_scan_snapshot") return null;
          if (cmd === "get_disks") return JSON.stringify([disk]);
          if (cmd === "start_scanning") {
            setTimeout(() => emit("scan_status", {
              items: 0,
              total: 0,
              errors: 0,
            }), 0);
            setTimeout(() => emit("scan_completed", JSON.stringify(scanPayload)), 20);
            return null;
          }
          if (cmd === "stop_scanning") return null;
          if (cmd === "show_in_folder") return null;
          if (cmd === "open_terminal") return null;
          return null;
        },
      };
    </script>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>`

  writeFileSync(harnessPath, html)
  return harnessPath
}

class CdpClient {
  constructor(wsUrl) {
    this.nextId = 1
    this.pending = new Map()
    this.ws = new WebSocket(wsUrl)
    this.ready = new Promise((resolve, reject) => {
      this.ws.addEventListener('open', resolve, { once: true })
      this.ws.addEventListener('error', reject, { once: true })
    })
    this.ws.addEventListener('message', (event) => {
      const message = JSON.parse(event.data)
      if (!message.id) return
      const pending = this.pending.get(message.id)
      if (!pending) return
      this.pending.delete(message.id)
      if (message.error) {
        pending.reject(new Error(message.error.message))
      } else {
        pending.resolve(message.result)
      }
    })
  }

  async send(method, params = {}) {
    await this.ready
    const id = this.nextId++
    const payload = JSON.stringify({ id, method, params })
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject })
      this.ws.send(payload)
    })
  }

  close() {
    this.ws.close()
  }
}

const waitForJson = async (url, timeoutMs = 10_000) => {
  const started = Date.now()
  let lastError
  while (Date.now() - started < timeoutMs) {
    try {
      const response = await fetch(url)
      if (response.ok) return response.json()
      lastError = new Error(`${url} returned ${response.status}`)
    } catch (error) {
      lastError = error
    }
    await wait(100)
  }
  throw lastError || new Error(`Timed out waiting for ${url}`)
}

const stopProcess = async (child) => {
  if (!child || child.exitCode !== null || child.signalCode !== null) return

  child.kill('SIGTERM')
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    wait(2_000),
  ])

  if (child.exitCode === null && child.signalCode === null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      wait(1_000),
    ])
  }
}

const waitForHttpOk = async (url, timeoutMs = 10_000) => {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    try {
      const response = await fetch(url)
      if (response.ok) return
    } catch {
      // keep waiting
    }
    await wait(100)
  }
  throw new Error(`Timed out waiting for ${url}`)
}

const evalInPage = async (cdp, expression) => {
  const result = await cdp.send('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
  })
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.text || 'Runtime.evaluate failed')
  }
  return result.result.value
}

const waitFor = async (cdp, expression, timeoutMs = 5_000) => {
  const started = Date.now()
  let value
  while (Date.now() - started < timeoutMs) {
    value = await evalInPage(cdp, expression)
    if (value) return value
    await wait(100)
  }
  throw new Error(`Timed out waiting for expression: ${expression}`)
}

const selectorFor = (selector) => JSON.stringify(selector)

const pointForSelector = async (cdp, selector) =>
  waitFor(
    cdp,
    `(() => {
      const el = document.querySelector(${selectorFor(selector)});
      if (!el) return null;
      const rect = el.getBoundingClientRect();
      if (!rect.width || !rect.height) return null;
      return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    })()`,
  )

const moveTo = async (cdp, selector) => {
  const point = await pointForSelector(cdp, selector)
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mouseMoved',
    x: point.x,
    y: point.y,
    button: 'none',
  })
}

const mouseOver = async (cdp, selector) =>
  evalInPage(
    cdp,
    `(() => {
      const el = document.querySelector(${selectorFor(selector)});
      if (!el) return false;
      el.dispatchEvent(new MouseEvent('mouseover', {
        bubbles: true,
        cancelable: true,
        view: window,
      }));
      return true;
    })()`,
  )

const mouseLeave = async (cdp, selector) =>
  evalInPage(
    cdp,
    `(() => {
      const el = document.querySelector(${selectorFor(selector)});
      if (!el) return false;
      el.dispatchEvent(new MouseEvent('mouseleave', {
        bubbles: false,
        cancelable: true,
        view: window,
      }));
      return true;
    })()`,
  )

const click = async (cdp, selector) => {
  const point = await pointForSelector(cdp, selector)
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mouseMoved',
    x: point.x,
    y: point.y,
    button: 'none',
  })
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mousePressed',
    x: point.x,
    y: point.y,
    button: 'left',
    clickCount: 1,
  })
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mouseReleased',
    x: point.x,
    y: point.y,
    button: 'left',
    clickCount: 1,
  })
}

const rightClick = async (cdp, selector) => {
  const point = await pointForSelector(cdp, selector)
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mouseMoved',
    x: point.x,
    y: point.y,
    button: 'none',
  })
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mousePressed',
    x: point.x,
    y: point.y,
    button: 'right',
    clickCount: 1,
  })
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mouseReleased',
    x: point.x,
    y: point.y,
    button: 'right',
    clickCount: 1,
  })
}

const sidebarState = (cdp) =>
  evalInPage(
    cdp,
    `(() => {
      const list = document.querySelector('[data-testid="sidebar-file-list"]');
      const header = document.querySelector('[data-testid="sidebar-directory"]');
      return {
        directoryId: list?.getAttribute('data-directory-id') || null,
        preview: list?.getAttribute('data-preview') || null,
        headerPreview: header?.getAttribute('data-preview') || null,
        entries: [...document.querySelectorAll('[data-testid="sidebar-entry"]')]
          .map((entry) => ({
            id: entry.getAttribute('data-entry-id'),
            text: entry.textContent.replace(/\\s+/g, ' ').trim(),
          })),
      };
    })()`,
  )

const breadcrumbState = (cdp) =>
  evalInPage(
    cdp,
    `(() => {
      const breadcrumb = document.querySelector('[data-testid="title-breadcrumb"]');
      const root = document.querySelector('[data-testid="title-breadcrumb-root"]');
      const current = document.querySelector('[data-testid="title-breadcrumb-current"]');
      return {
        text: breadcrumb?.textContent.replace(/\\s+/g, ' ').trim() || '',
        rootPath: root?.getAttribute('data-path') || null,
        rootTitle: root?.getAttribute('title') || null,
        rootOverflowed: root ? root.scrollWidth > root.clientWidth : false,
        currentPath: current?.getAttribute('data-path') || null,
        segmentPaths: [...document.querySelectorAll('[data-testid="title-breadcrumb-segment"]')]
          .map((segment) => segment.getAttribute('data-path')),
      };
    })()`,
  )

const assertEntryIds = (state, expectedIds, label) => {
  const actual = state.entries.map((entry) => entry.id).sort()
  const expected = [...expectedIds].sort()
  assert(
    JSON.stringify(actual) === JSON.stringify(expected),
    `${label}: expected ${expected.join(', ')}, got ${actual.join(', ')}`,
  )
}

const main = async () => {
  await waitForHttpOk(`${devServerUrl}/.local/sidebar-harness.html`)

  const scanRoot = createFixture()
  const scanPayload = runPduScan(scanRoot)
  writeHarness({ scanRoot, scanPayload })

  const chromeUserData = mkdtempSync(join(tmpdir(), 'squirreldisk-chrome-'))
  const debuggingPort = 9223 + Math.floor(Math.random() * 1_000)
  const chrome = spawn(chromePath(), [
    '--headless=new',
    '--disable-gpu',
    '--no-first-run',
    '--no-default-browser-check',
    '--remote-debugging-address=127.0.0.1',
    `--remote-debugging-port=${debuggingPort}`,
    `--user-data-dir=${chromeUserData}`,
    '--window-size=1200,900',
    `${devServerUrl}/.local/sidebar-pdu-e2e.html`,
  ], {
    stdio: ['ignore', 'ignore', 'pipe'],
  })

  let cdp
  try {
    const targets = await waitForJson(`http://127.0.0.1:${debuggingPort}/json`)
    const page = targets.find((target) => target.type === 'page')
    assert(page?.webSocketDebuggerUrl, 'Chrome did not expose a page target')
    cdp = new CdpClient(page.webSocketDebuggerUrl)
    await cdp.send('Page.enable')
    await cdp.send('Runtime.enable')

    await waitFor(cdp, `document.querySelector('[data-testid="disk-row"]') !== null`)
    await click(cdp, `[data-testid="disk-row"][data-disk-path="${scanRoot}"]`)

    const rootId = scanRoot
    const usersId = `${rootId}/Users`
    const libraryId = `${rootId}/Library`
    const applicationsId = `${rootId}/Applications`
    const siriusId = `${usersId}/sirius`
    const sharedId = `${usersId}/Shared`
    const cachesId = `${libraryId}/Caches`

    await waitFor(
      cdp,
      `document.querySelector('[data-testid="sidebar-file-list"]')?.getAttribute('data-directory-id') === ${JSON.stringify(rootId)}`,
    )
    let state = await sidebarState(cdp)
    assert(state.preview === 'false', 'root sidebar should not be preview')
    assertEntryIds(state, [usersId, libraryId, applicationsId], 'root entries')
    let breadcrumb = await breadcrumbState(cdp)
    assert(!breadcrumb.text.includes('SquirrelDisk'), 'breadcrumb should not show app name')
    assert(breadcrumb.text.includes('All Disks'), 'breadcrumb should start at All Disks')
    assert(breadcrumb.text.includes('Disk'), 'breadcrumb should include disk crumb')
    assert(breadcrumb.rootPath === rootId, 'disk crumb should point at scan root')
    assert(breadcrumb.rootTitle === rootId, 'disk crumb should expose full root path')
    assert(breadcrumb.rootOverflowed, 'long disk crumb should be visually truncated')

    await rightClick(cdp, `[data-testid="sidebar-entry"][data-entry-id="${usersId}"]`)
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="node-context-menu"]')?.getAttribute('data-node-id') === ${JSON.stringify(usersId)}`,
    )
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="context-menu-collect"]')?.disabled === false`,
    )
    await click(cdp, '[data-testid="context-menu-collect"]')
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="node-context-menu"]') === null
        && document.querySelector('[data-testid="collector-drop-zone"]')?.textContent.includes('1 selected')
        && document.querySelector('[data-testid="chart-arc"][data-node-id="${usersId}"]') === null
        && document.querySelector('[data-testid="sidebar-entry"][data-entry-id="${usersId}"]') === null`,
    )
    state = await sidebarState(cdp)
    assertEntryIds(state, [libraryId, applicationsId], 'root entries after collecting Users')

    await click(cdp, '[data-testid="collector-drop-zone"] button')
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="collector-drop-zone"]')?.textContent.includes('Drop files and folders here to collect')
        && document.querySelector('[data-testid="chart-arc"][data-node-id="${usersId}"]') !== null
        && document.querySelector('[data-testid="sidebar-entry"][data-entry-id="${usersId}"]') !== null`,
    )
    state = await sidebarState(cdp)
    assertEntryIds(state, [usersId, libraryId, applicationsId], 'root entries after clearing Collector')

    await mouseOver(cdp, `[data-testid="chart-arc"][data-node-id="${usersId}"]`)
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="sidebar-file-list"]')?.getAttribute('data-directory-id') === ${JSON.stringify(usersId)}`,
    )
    const rootHoverStyle = await evalInPage(
      cdp,
      `(() => {
        const hoveredArc = document.querySelector('[data-testid="chart-arc"][data-node-id="${usersId}"]');
        const siblingArc = document.querySelector('[data-testid="chart-arc"][data-node-id="${libraryId}"]');
        return {
          hoveredStrokeWidth: Number(hoveredArc?.getAttribute('stroke-width') || 0),
          siblingOpacity: Number(siblingArc?.getAttribute('fill-opacity') || 0),
        };
      })()`,
    )
    assert(rootHoverStyle.hoveredStrokeWidth <= 0.6, 'hovered arc should keep a thin stroke')
    assert(rootHoverStyle.siblingOpacity >= 0.95, 'hover should not dim sibling arcs')
    state = await sidebarState(cdp)
    assert(state.preview === 'true', 'hovered Users should render as preview')
    assertEntryIds(state, [siriusId, sharedId], 'Users preview entries')

    await mouseLeave(cdp, `[data-testid="chart-arc"][data-node-id="${usersId}"]`)
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="sidebar-file-list"]')?.getAttribute('data-preview') === 'false'
        && document.querySelector('[data-testid="sidebar-file-list"]')?.getAttribute('data-directory-id') === ${JSON.stringify(rootId)}`,
    )
    state = await sidebarState(cdp)
    assertEntryIds(state, [usersId, libraryId, applicationsId], 'root entries after hover reset')

    await click(cdp, `[data-testid="sidebar-entry"][data-entry-id="${usersId}"]`)
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="sidebar-file-list"]')?.getAttribute('data-preview') === 'false'
        && document.querySelector('[data-testid="sidebar-file-list"]')?.getAttribute('data-directory-id') === ${JSON.stringify(usersId)}`,
    )
    state = await sidebarState(cdp)
    assertEntryIds(state, [siriusId, sharedId], 'Users active entries')
    breadcrumb = await breadcrumbState(cdp)
    assert(breadcrumb.currentPath === usersId, 'breadcrumb should track focused Users directory')
    assert(breadcrumb.text.includes('Users'), 'breadcrumb should show focused Users segment')
    assert(!breadcrumb.text.includes('SquirrelDisk'), 'focused breadcrumb should not show app name')

    await click(cdp, `[data-testid="sidebar-entry"][data-entry-id="${siriusId}"]`)
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="sidebar-file-list"]')?.getAttribute('data-directory-id') === ${JSON.stringify(siriusId)}`,
    )
    breadcrumb = await breadcrumbState(cdp)
    assert(breadcrumb.currentPath === siriusId, 'breadcrumb should track nested focused directory')
    assert(
      breadcrumb.segmentPaths.includes(usersId),
      'breadcrumb should make ancestor path segments clickable',
    )

    await click(cdp, `[data-testid="title-breadcrumb-segment"][data-path="${usersId}"]`)
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="sidebar-file-list"]')?.getAttribute('data-directory-id') === ${JSON.stringify(usersId)}`,
    )
    state = await sidebarState(cdp)
    assertEntryIds(state, [siriusId, sharedId], 'Users entries after breadcrumb click')

    await click(cdp, '[data-testid="sidebar-directory"]')
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="sidebar-file-list"]')?.getAttribute('data-directory-id') === ${JSON.stringify(rootId)}`,
    )
    state = await sidebarState(cdp)
    assertEntryIds(state, [usersId, libraryId, applicationsId], 'root entries after back')

    await click(cdp, `[data-testid="sidebar-entry"][data-entry-id="${libraryId}"]`)
    await waitFor(
      cdp,
      `document.querySelector('[data-testid="sidebar-file-list"]')?.getAttribute('data-directory-id') === ${JSON.stringify(libraryId)}`,
    )
    state = await sidebarState(cdp)
    assert(state.preview === 'false', 'clicked Library should be active')
    assertEntryIds(state, [cachesId], 'Library active entries')

    await waitFor(
      cdp,
      `document.querySelector('[data-testid="chart-arc"][data-node-id="${cachesId}"]') !== null`,
    )
    await moveTo(cdp, `[data-testid="sidebar-entry"][data-entry-id="${cachesId}"]`)
    if (process.env.SQD_E2E_DEBUG) {
      console.log(await evalInPage(
        cdp,
        `(() => {
          const arc = document.querySelector('[data-testid="chart-arc"][data-node-id="${cachesId}"]');
          const entry = document.querySelector('[data-testid="sidebar-entry"][data-entry-id="${cachesId}"]');
          return {
            arcStroke: arc?.getAttribute('stroke'),
            arcStrokeWidth: arc?.getAttribute('stroke-width'),
            entryClass: entry?.getAttribute('class'),
          };
        })()`,
      ))
    }
    await waitFor(
      cdp,
      `Number(document.querySelector('[data-testid="chart-arc"][data-node-id="${cachesId}"]')?.getAttribute('stroke-width') || 0) <= 0.6`,
    )

    console.log('sidebar-pdu-e2e=ok')
  } finally {
    cdp?.close()
    await stopProcess(chrome)
    rmSync(scanRoot, { recursive: true, force: true })
    rmSync(chromeUserData, {
      recursive: true,
      force: true,
      maxRetries: 5,
      retryDelay: 100,
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
