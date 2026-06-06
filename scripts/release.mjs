#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import os from 'node:os'
import { basename, dirname, extname, join, resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(__dirname, '..')
const tauriConfigPath = join(repoRoot, 'src-tauri', 'tauri.conf.json')
const cargoManifestPath = join(repoRoot, 'src-tauri', 'Cargo.toml')
const localDir = join(repoRoot, '.local')
const defaultReleaseTree = 'releases/squirreldisk'
const defaultReleaseOwnerNpub = 'npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm'

class SkipStepError extends Error {}

function usage() {
  console.log(`Usage: node scripts/release.mjs [options]

Build SquirrelDisk desktop release artifacts, stage a git.iris.to-compatible
hashtree release directory, and optionally publish it.

Options:
  --publish                 Publish the staged tree with htree
  --dry-run                 Print the plan without running build or publish commands
  --skip-verify            Skip npm/cargo verification
  --allow-dirty            Allow releasing from a dirty git tree
  --allow-partial          Publish even if a platform build fails
  --include-updater-artifacts
                            Also publish Tauri updater archives
  --include-extra-packages  Also publish secondary package formats such as RPM/MSI
  --tag <tag>              Release tag (defaults to src-tauri/tauri.conf.json version)
  --release-tree <name>    htree release tree name (default: releases/squirreldisk)
  --owner-npub <npub>      Owner npub for the printed git.iris.to release URL
  --stage-dir <path>       Directory for staged release metadata
  --artifacts-dir <path>   Directory for collected build artifacts
  --only <csv>             Limit steps to verify,macos,linux,windows
  --skip <csv>             Skip steps by name
  --help                   Show this help

Environment:
  SQD_RELEASE_TREE
  SQD_RELEASE_OWNER_NPUB
  SQD_RELEASE_ALLOW_DIRTY
  SQD_RELEASE_ALLOW_PARTIAL
  SQD_RELEASE_INCLUDE_UPDATER_ARTIFACTS
  SQD_RELEASE_INCLUDE_EXTRA_PACKAGES
  SQD_MACOS_TARGET         Default: host macOS architecture
  SQD_MACOS_SIGNING_IDENTITY
  SQD_MACOS_NOTARY_PROFILE Optional notarytool keychain profile
  SQD_ASC_AUTH_KEY_PATH    Defaults to ~/.appstoreconnect/private_keys/AuthKey_*.p8
  SQD_ASC_AUTH_KEY_ID      Defaults from AuthKey_<id>.p8
  SQD_ASC_AUTH_KEY_ISSUER_ID Defaults from ~/.appstoreconnect/issuer.txt
  SQD_LINUX_TARGET         Default: host Linux architecture in Docker
  SQD_LINUX_DOCKER_IMAGE   Default: squirreldisk-tauri-linux-release:<arch>
  SQD_LINUX_CARGO_JOBS     Default: 1 inside Docker for reproducible builds
  SQD_PDU_VERSION          Default: 0.23.0
  SQD_WINDOWS_SSH_HOST       SSH host running Windows (default: win11-dev)
  SQD_WINDOWS_VM_NAME        Legacy alias for SQD_WINDOWS_SSH_HOST
  SQD_WINDOWS_GUEST_REPO_PATH  Repo path on the Windows host (default: C:\\src\\squirreldisk)
`)
}

function splitCsv(value) {
  return String(value || '')
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean)
}

function normalizeTag(value) {
  const tag = String(value || '').trim()
  if (!tag) {
    throw new Error('Release tag must not be empty')
  }
  return tag.startsWith('v') ? tag : `v${tag}`
}

function envFlagEnabled(value) {
  return /^(1|true|yes|on)$/i.test(String(value ?? '').trim())
}

function parseArgs(argv) {
  const options = {
    publish: false,
    dryRun: false,
    skipVerify: false,
    allowDirty: false,
    allowPartial: false,
    includeUpdaterArtifacts: false,
    includeExtraPackages: false,
    tag: null,
    releaseTree: null,
    ownerNpub: null,
    stageDir: null,
    artifactsDir: null,
    only: null,
    skip: new Set(),
  }

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    switch (arg) {
      case '-h':
      case '--help':
      case 'help':
        usage()
        process.exit(0)
      case '--publish':
        options.publish = true
        break
      case '--dry-run':
        options.dryRun = true
        break
      case '--skip-verify':
        options.skipVerify = true
        break
      case '--allow-dirty':
        options.allowDirty = true
        break
      case '--allow-partial':
        options.allowPartial = true
        break
      case '--include-updater-artifacts':
        options.includeUpdaterArtifacts = true
        break
      case '--include-extra-packages':
        options.includeExtraPackages = true
        break
      case '--tag':
        options.tag = normalizeTag(argv[++index] ?? '')
        break
      case '--release-tree':
        options.releaseTree = argv[++index] ?? ''
        break
      case '--owner-npub':
        options.ownerNpub = argv[++index] ?? ''
        break
      case '--stage-dir':
        options.stageDir = resolve(repoRoot, argv[++index] ?? '')
        break
      case '--artifacts-dir':
        options.artifactsDir = resolve(repoRoot, argv[++index] ?? '')
        break
      case '--only':
        options.only = new Set(splitCsv(argv[++index] ?? ''))
        break
      case '--skip':
        for (const value of splitCsv(argv[++index] ?? '')) {
          options.skip.add(value)
        }
        break
      default:
        throw new Error(`Unknown argument: ${arg}`)
    }
  }

  return options
}

function quote(arg) {
  const value = String(arg)
  return /[^\w./:=@%+-]/.test(value) ? JSON.stringify(value) : value
}

function run(command, args, { cwd = repoRoot, env = process.env, capture = false, dryRun = false } = {}) {
  const rendered = [command, ...args].map(quote).join(' ')
  console.log(`$ ${rendered}`)
  if (dryRun) {
    return ''
  }

  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: 'utf8',
    stdio: capture ? 'pipe' : 'inherit',
  })

  if (result.status !== 0) {
    const stderr = capture ? result.stderr.trim() : ''
    throw new Error(stderr || `${command} exited with status ${result.status ?? 'unknown'}`)
  }

  return capture ? result.stdout.trim() : ''
}

function runWithInput(command, args, input, { cwd = repoRoot, dryRun = false } = {}) {
  const rendered = [command, ...args].map(quote).join(' ')
  console.log(`$ ${rendered}`)
  if (dryRun) {
    return ''
  }

  const result = spawnSync(command, args, {
    cwd,
    input,
    encoding: 'utf8',
    stdio: ['pipe', 'inherit', 'inherit'],
  })

  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status ?? 'unknown'}`)
  }

  return ''
}

function commandExists(command) {
  const result =
    process.platform === 'win32'
      ? spawnSync('where', [command], { stdio: 'ignore' })
      : spawnSync('sh', ['-lc', `command -v ${JSON.stringify(command)}`], { stdio: 'ignore' })

  return result.status === 0
}

function readTauriConfig() {
  return JSON.parse(readFileSync(tauriConfigPath, 'utf8'))
}

function readDefaultTag() {
  return normalizeTag(readTauriConfig().version)
}

function shouldRunStep(name, options) {
  if (options.skip.has(name)) {
    return false
  }
  return !options.only || options.only.has(name)
}

function defaultMacosTarget(env) {
  if (env.SQD_MACOS_TARGET) {
    return env.SQD_MACOS_TARGET
  }
  return process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin'
}

function archLabel(target) {
  if (target === 'universal-apple-darwin') return 'universal'
  if (target.startsWith('aarch64-')) return 'arm64'
  if (target.startsWith('x86_64-')) return 'x64'
  return target.replace(/[^a-zA-Z0-9._-]+/g, '-')
}

function cargoTargetRoot(env, fallbackRoot = repoRoot) {
  return env.CARGO_TARGET_DIR ? resolve(fallbackRoot, env.CARGO_TARGET_DIR) : join(fallbackRoot, 'src-tauri', 'target')
}

function walkFiles(root) {
  if (!existsSync(root)) {
    return []
  }

  const result = []
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name)
    if (entry.isDirectory()) {
      result.push(...walkFiles(path))
    } else if (entry.isFile()) {
      result.push(path)
    }
  }
  return result
}

function walkDirectories(root) {
  if (!existsSync(root)) {
    return []
  }

  const result = []
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name)
    if (entry.isDirectory()) {
      result.push(path)
      result.push(...walkDirectories(path))
    }
  }
  return result
}

function newestFile(paths) {
  return paths
    .filter((path) => existsSync(path))
    .sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs)[0] ?? null
}

function copyAsset(sourcePath, targetDir, targetName) {
  mkdirSync(targetDir, { recursive: true })
  const targetPath = join(targetDir, targetName)
  copyFileSync(sourcePath, targetPath)
  return targetPath
}

function fileExt(path) {
  if (path.endsWith('.app.tar.gz')) return '.app.tar.gz'
  if (path.endsWith('.tar.gz')) return '.tar.gz'
  return extname(path)
}

function collectNewestByExt({ sourceDir, artifactDir, tag, platform, arch, extensions, builtLines, dryRun = false }) {
  const files = walkFiles(sourceDir)
  const assets = []

  for (const extension of extensions) {
    const sourcePath = newestFile(files.filter((path) => path.endsWith(extension)))
    if (!sourcePath) {
      continue
    }

    const targetName = `squirreldisk-${tag}-${platform}-${arch}${extension}`
    assets.push(dryRun ? join(artifactDir, targetName) : copyAsset(sourcePath, artifactDir, targetName))
    builtLines.push(`${platform} ${arch} artifact: ${targetName}`)

    const sigPath = `${sourcePath}.sig`
    if (existsSync(sigPath)) {
      assets.push(dryRun ? join(artifactDir, `${targetName}.sig`) : copyAsset(sigPath, artifactDir, `${targetName}.sig`))
    }
  }

  return assets
}

function macosReleaseExtensions({ includeUpdaterArtifacts }) {
  return includeUpdaterArtifacts ? ['.dmg', '.app.tar.gz'] : ['.dmg']
}

function linuxReleaseExtensions({ includeExtraPackages }) {
  const extensions = ['.AppImage', '.deb']
  if (includeExtraPackages) {
    extensions.push('.rpm')
  }
  return extensions
}

function windowsReleaseExtensions({ includeUpdaterArtifacts, includeExtraPackages }) {
  const extensions = ['.exe']
  if (includeExtraPackages) {
    extensions.push('.msi')
  }
  if (includeUpdaterArtifacts) {
    extensions.push('.zip')
  }
  return extensions
}

function readTrimmedFile(path) {
  if (!existsSync(path)) {
    return ''
  }
  return readFileSync(path, 'utf8').trim()
}

function defaultAscRoot(env) {
  return env.SQD_ASC_ROOT || join(os.homedir(), '.appstoreconnect')
}

function defaultAscAuthKeyPath(env) {
  const ascRoot = defaultAscRoot(env)
  const keysDir = join(ascRoot, 'private_keys')
  if (!existsSync(keysDir)) {
    return ''
  }
  const keyName = readdirSync(keysDir)
    .filter((name) => /^AuthKey_[A-Z0-9]+\.p8$/.test(name))
    .sort()[0]
  return keyName ? join(keysDir, keyName) : ''
}

function defaultAscAuthKeyId(keyPath) {
  const match = basename(keyPath).match(/^AuthKey_([A-Z0-9]+)\.p8$/)
  return match?.[1] || ''
}

function macosNotaryAuthArgs(env) {
  const profile = String(env.SQD_MACOS_NOTARY_PROFILE || env.MACOS_NOTARY_PROFILE || '').trim()
  if (profile) {
    return ['--keychain-profile', profile]
  }

  const keyPath = String(
    env.SQD_ASC_AUTH_KEY_PATH || env.IRIS_ASC_AUTH_KEY_PATH || env.NVPN_ASC_AUTH_KEY_PATH || defaultAscAuthKeyPath(env),
  ).trim()
  const keyId = String(
    env.SQD_ASC_AUTH_KEY_ID || env.IRIS_ASC_AUTH_KEY_ID || env.NVPN_ASC_AUTH_KEY_ID || defaultAscAuthKeyId(keyPath),
  ).trim()
  const issuer = String(
    env.SQD_ASC_AUTH_KEY_ISSUER_ID
      || env.IRIS_ASC_AUTH_KEY_ISSUER_ID
      || env.NVPN_ASC_AUTH_KEY_ISSUER_ID
      || readTrimmedFile(join(defaultAscRoot(env), 'issuer.txt')),
  ).trim()

  const missing = []
  if (!keyPath) missing.push('SQD_ASC_AUTH_KEY_PATH')
  if (!keyId) missing.push('SQD_ASC_AUTH_KEY_ID')
  if (!issuer) missing.push('SQD_ASC_AUTH_KEY_ISSUER_ID')
  if (missing.length > 0) {
    throw new Error(`Missing macOS notarization credentials: ${missing.join(', ')}`)
  }
  if (!existsSync(keyPath)) {
    throw new Error(`macOS notarization key not found: ${keyPath}`)
  }

  return ['--key', keyPath, '--key-id', keyId, '--issuer', issuer]
}

function detectMacosSigningIdentity(env, { dryRun }) {
  const configured = String(env.SQD_MACOS_SIGNING_IDENTITY || env.MACOS_SIGNING_IDENTITY || '').trim()
  if (configured) {
    return configured
  }
  if (dryRun) {
    return 'Developer ID Application: Example (TEAMID)'
  }

  const identities = run('security', ['find-identity', '-v', '-p', 'codesigning'], { capture: true })
  const matches = [...identities.matchAll(/"([^"]*Developer ID Application[^"]*)"/g)].map((match) => match[1])
  if (matches.length === 1) {
    return matches[0]
  }
  if (matches.length === 0) {
    throw new Error('No Developer ID Application signing identity found in the login keychain.')
  }
  throw new Error(`Multiple Developer ID Application identities found; set SQD_MACOS_SIGNING_IDENTITY. Candidates: ${matches.join(', ')}`)
}

function signMacosApp({ appPath, identity, dryRun }) {
  run(
    'codesign',
    [
      '--force',
      '--deep',
      '--options',
      'runtime',
      '--timestamp',
      '--sign',
      identity,
      appPath,
    ],
    { dryRun },
  )
  run('codesign', ['--verify', '--deep', '--strict', '--verbose=2', appPath], { dryRun })
}

function submitNotarization({ artifactPath, authArgs, label, dryRun }) {
  const output = run(
    'xcrun',
    ['notarytool', 'submit', artifactPath, ...authArgs, '--wait', '--output-format', 'json'],
    { capture: true, dryRun },
  )
  if (dryRun) {
    return
  }

  const submission = JSON.parse(output)
  if (submission.status !== 'Accepted') {
    if (submission.id) {
      try {
        run('xcrun', ['notarytool', 'log', submission.id, ...authArgs])
      } catch {}
    }
    throw new Error(`${label} notarization status was '${submission.status}' (expected 'Accepted').`)
  }
}

function notarizeAndStapleMacosApp({ appPath, env, dryRun }) {
  const authArgs = macosNotaryAuthArgs(env)
  const tempRoot = dryRun ? join(os.tmpdir(), 'squirreldisk-notary-dry-run') : mkdtempSync(join(os.tmpdir(), 'squirreldisk-notary-'))
  const zipPath = join(tempRoot, 'SquirrelDisk.app.zip')

  try {
    run('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', appPath, zipPath], { dryRun })
    submitNotarization({ artifactPath: zipPath, authArgs, label: 'macOS app', dryRun })
    run('xcrun', ['stapler', 'staple', appPath], { dryRun })
    run('xcrun', ['stapler', 'validate', appPath], { dryRun })
    run('spctl', ['--assess', '--type', 'execute', '--verbose=4', appPath], { dryRun })
  } finally {
    if (!dryRun) {
      rmSync(tempRoot, { recursive: true, force: true })
    }
  }

  return authArgs
}

function createDmgFromApp({ appPath, dmgPath, dryRun }) {
  const stageDir = dryRun ? join(os.tmpdir(), 'squirreldisk-dmg-dry-run') : mkdtempSync(join(os.tmpdir(), 'squirreldisk-dmg-'))

  try {
    if (!dryRun) {
      rmSync(dmgPath, { force: true })
    }
    run('ditto', [appPath, join(stageDir, basename(appPath))], { dryRun })
    run('ln', ['-s', '/Applications', join(stageDir, 'Applications')], { dryRun })
    run(
      'hdiutil',
      ['create', '-volname', 'SquirrelDisk', '-srcfolder', stageDir, '-fs', 'HFS+', '-format', 'UDZO', '-ov', dmgPath],
      { dryRun },
    )
  } finally {
    if (!dryRun) {
      rmSync(stageDir, { recursive: true, force: true })
    }
  }
}

function notarizeAndStapleDmg({ dmgPath, authArgs, dryRun }) {
  submitNotarization({ artifactPath: dmgPath, authArgs, label: 'macOS DMG', dryRun })
  run('xcrun', ['stapler', 'staple', dmgPath], { dryRun })
  run('xcrun', ['stapler', 'validate', dmgPath], { dryRun })
}

function runVerify({ dryRun, builtLines }) {
  run('npm', ['test'], { dryRun })
  run('npm', ['run', 'build'], { dryRun })
  run('cargo', ['test', '--manifest-path', cargoManifestPath], { dryRun })
  builtLines.push('Ran npm test, npm run build, and cargo test for src-tauri.')
}

function buildMacosArtifacts({ env, tag, artifactDir, dryRun, builtLines, includeUpdaterArtifacts }) {
  if (process.platform !== 'darwin') {
    throw new SkipStepError('macOS artifacts are only built on Darwin hosts.')
  }

  const target = defaultMacosTarget(env)
  let buildError = null
  try {
    run('npm', ['run', 'tauri', '--', 'build', '--target', target, '--bundles', 'app,dmg', '--ci'], {
      dryRun,
    })
  } catch (error) {
    buildError = error
  }

  const targetDir = join(cargoTargetRoot(env), target, 'release', 'bundle')
  const appPath = newestFile(walkDirectories(targetDir).filter((path) => path.endsWith('.app')))
  const bundleFiles = walkFiles(targetDir)
  // Tauri only writes its own .app.tar.gz when `bundle.createUpdaterArtifacts`
  // is true, which couples to the official tauri-plugin-updater (minisign).
  // We're using tauri-plugin-hashtree-updater, so derive the tarball path
  // from the .app and produce it ourselves with `ditto` below.
  let appTarPath = newestFile(bundleFiles.filter((path) => path.endsWith('.app.tar.gz')))
  if (!appTarPath && appPath && includeUpdaterArtifacts) {
    appTarPath = `${appPath}.tar.gz`
  }
  const dmgPath = newestFile(bundleFiles.filter((path) => path.endsWith('.dmg')))

  if (!dryRun && (!appPath || !dmgPath || (includeUpdaterArtifacts && !appTarPath))) {
    if (buildError) {
      throw buildError
    }
    const expected = includeUpdaterArtifacts ? 'app, updater archive, and dmg artifacts' : 'app and dmg artifacts'
    throw new SkipStepError(`macOS build completed but did not produce ${expected} in ${targetDir}.`)
  }

  if (appPath && dmgPath) {
    const identity = detectMacosSigningIdentity(env, { dryRun })
    signMacosApp({ appPath, identity, dryRun })
    const authArgs = notarizeAndStapleMacosApp({ appPath, env, dryRun })
    if (appTarPath) {
      if (!dryRun) {
        rmSync(appTarPath, { force: true })
      }
      // hashtree-updater's AppBundle installer gunzips + untars the asset, so
      // the .app.tar.gz must be a real tar.gz. `ditto -c -k` produces a PKZip
      // regardless of the output extension, so use `tar -czf` directly.
      run('tar', ['-czf', appTarPath, '-C', dirname(appPath), basename(appPath)], { dryRun })
    }
    createDmgFromApp({ appPath, dmgPath, dryRun })
    notarizeAndStapleDmg({ dmgPath, authArgs, dryRun })
    builtLines.push(`macOS ${archLabel(target)} app and DMG signed, notarized, and stapled.`)
  }

  const assets = collectNewestByExt({
    sourceDir: targetDir,
    artifactDir,
    tag,
    platform: 'macos',
    arch: archLabel(target),
    extensions: macosReleaseExtensions({ includeUpdaterArtifacts }),
    builtLines,
    dryRun,
  })

  if (assets.length === 0) {
    if (buildError) {
      throw buildError
    }
    throw new SkipStepError(`macOS build completed but no bundle artifacts were found in ${targetDir}.`)
  }

  if (buildError) {
    builtLines.push(`macOS Tauri build exited after producing bundles: ${buildError.message}`)
  }

  return assets
}

function defaultLinuxTarget(env) {
  if (env.SQD_LINUX_TARGET) {
    return env.SQD_LINUX_TARGET
  }
  return process.arch === 'arm64' ? 'aarch64-unknown-linux-gnu' : 'x86_64-unknown-linux-gnu'
}

function linuxDockerPlatform(target) {
  if (target.startsWith('aarch64-')) return 'linux/arm64'
  return 'linux/amd64'
}

function linuxDockerfile(target) {
  return `FROM node:22-bookworm
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \\
  && apt-get install -y --no-install-recommends \\
    ca-certificates \\
    curl \\
    build-essential \\
    pkg-config \\
    libssl-dev \\
    libgtk-3-dev \\
    libwebkit2gtk-4.1-dev \\
    libayatana-appindicator3-dev \\
    librsvg2-dev \\
    patchelf \\
    file \\
    xz-utils \\
  && rm -rf /var/lib/apt/lists/*
ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:$PATH
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal --default-toolchain stable \\
  && rustup target add ${target} \\
  && chmod -R a+rwx /usr/local/cargo /usr/local/rustup
`
}

function ensureLinuxDockerImage({ env, target, platform, dryRun }) {
  const image = env.SQD_LINUX_DOCKER_IMAGE || `squirreldisk-tauri-linux-release:${archLabel(target)}`
  runWithInput('docker', ['build', '--platform', platform, '-t', image, '-'], linuxDockerfile(target), {
    cwd: repoRoot,
    dryRun,
  })
  return image
}

function createCleanWorktree({ dryRun }) {
  const worktreeParent = join(localDir, 'worktrees')
  if (!dryRun) {
    mkdirSync(worktreeParent, { recursive: true })
  }
  const tempDir = dryRun ? join(worktreeParent, 'linux-dry-run') : mkdtempSync(join(worktreeParent, 'linux-'))
  if (!dryRun) {
    rmSync(tempDir, { recursive: true, force: true })
  }
  run('git', ['worktree', 'add', '--detach', tempDir, 'HEAD'], { dryRun })
  return tempDir
}

function removeWorktree(path, { dryRun }) {
  if (!path || dryRun) {
    return
  }
  run('git', ['worktree', 'remove', '--force', path], { dryRun })
}

function buildLinuxArtifacts({ env, tag, artifactDir, dryRun, builtLines, includeExtraPackages }) {
  const target = defaultLinuxTarget(env)
  const arch = archLabel(target)

  if (process.platform === 'linux') {
    let buildError = null
    try {
      run('npm', ['run', 'tauri', '--', 'build', '--target', target, '--ci'], {
        dryRun,
      })
    } catch (error) {
      buildError = error
    }
    const targetDir = join(cargoTargetRoot(env), target, 'release', 'bundle')
    const assets = collectNewestByExt({
      sourceDir: targetDir,
      artifactDir,
      tag,
      platform: 'linux',
      arch,
      extensions: linuxReleaseExtensions({ includeExtraPackages }),
      builtLines,
      dryRun,
    })
    if (assets.length === 0) {
      if (buildError) {
        throw buildError
      }
      throw new SkipStepError(`Linux build completed but no bundle artifacts were found in ${targetDir}.`)
    }
    if (buildError) {
      builtLines.push(`Linux Tauri build exited after producing bundles: ${buildError.message}`)
    }
    return assets
  }

  if (!commandExists('docker')) {
    throw new SkipStepError('Linux artifacts require either a Linux host or Docker.')
  }

  const platform = linuxDockerPlatform(target)
  const image = ensureLinuxDockerImage({ env, target, platform, dryRun })
  const worktree = createCleanWorktree({ dryRun })
  try {
    const uid = String(process.getuid?.() ?? 1000)
    const gid = String(process.getgid?.() ?? 1000)
    const pduVersion = env.SQD_PDU_VERSION || '0.23.0'
    const cargoJobs = env.SQD_LINUX_CARGO_JOBS || '1'
    const dockerScript = [
      'set -Eeuo pipefail',
      '. /usr/local/cargo/env',
      'npm ci',
      `if [ ${JSON.stringify(target)} = "aarch64-unknown-linux-gnu" ] && [ ! -x src-tauri/bin/pdu-aarch64-unknown-linux-gnu ]; then`,
      `  cargo install parallel-disk-usage --version ${JSON.stringify(pduVersion)} --root /tmp/pdu-root`,
      '  cp /tmp/pdu-root/bin/pdu src-tauri/bin/pdu-aarch64-unknown-linux-gnu',
      'fi',
      `npm run tauri -- build --target ${target} --ci`,
    ].join('\n')

    let buildError = null
    try {
      run(
        'docker',
        [
          'run',
          '--rm',
          '--platform',
          platform,
          '--user',
          `${uid}:${gid}`,
          '-e',
          'HOME=/tmp/squirreldisk-home',
          '-e',
          'CARGO_INCREMENTAL=0',
          '-e',
          `CARGO_BUILD_JOBS=${cargoJobs}`,
          '-v',
          `${worktree}:/work`,
          '-w',
          '/work',
          image,
          'bash',
          '-lc',
          dockerScript,
        ],
        { dryRun },
      )
    } catch (error) {
      buildError = error
    }

    const targetDir = join(worktree, 'src-tauri', 'target', target, 'release', 'bundle')
    const assets = collectNewestByExt({
      sourceDir: targetDir,
      artifactDir,
      tag,
      platform: 'linux',
      arch,
      extensions: linuxReleaseExtensions({ includeExtraPackages }),
      builtLines,
      dryRun,
    })

    if (assets.length === 0) {
      if (buildError) {
        throw buildError
      }
      throw new SkipStepError(`Linux build completed but no bundle artifacts were found in ${targetDir}.`)
    }

    if (buildError) {
      builtLines.push(`Linux Tauri build exited after producing bundles: ${buildError.message}`)
    }

    return assets
  } finally {
    removeWorktree(worktree, { dryRun })
  }
}

function psQuote(value) {
  return `'${String(value).replace(/'/g, "''")}'`
}

function encodePowerShellScript(script) {
  return Buffer.from(script, 'utf16le').toString('base64')
}

function runWindowsPowerShell(host, script, { capture = false, dryRun = false } = {}) {
  const encoded = encodePowerShellScript(script)
  return run(
    'ssh',
    [host, 'powershell.exe', '-NoProfile', '-EncodedCommand', encoded],
    { capture, dryRun },
  )
}

function syncRepoToWindowsHost({ host, guestRepo, dryRun }) {
  const guestRepoForward = guestRepo.replace(/\\/g, '/')
  const tarExcludes = [
    '--exclude=./node_modules',
    '--exclude=./dist',
    '--exclude=./dist-ssr',
    '--exclude=./.git',
    '--exclude=./.local',
    '--exclude=./src-tauri/target',
  ].join(' ')

  runWindowsPowerShell(
    host,
    `New-Item -ItemType Directory -Force -Path ${psQuote(guestRepo)} | Out-Null`,
    { dryRun },
  )
  if (dryRun) {
    console.log(`[dry] tar ${tarExcludes} -cf - -C ${repoRoot} . | ssh ${host} tar -xf - -C ${guestRepoForward}`)
    return
  }
  const result = spawnSync(
    'bash',
    [
      '-c',
      `tar ${tarExcludes} -cf - -C ${quote(repoRoot)} . | ssh ${quote(host)} tar -xf - -C ${quote(guestRepoForward)}`,
    ],
    { stdio: ['ignore', 'inherit', 'inherit'] },
  )
  if (result.status !== 0) {
    throw new Error(`tar | ssh sync to ${host} failed (exit ${result.status})`)
  }
}

function pullDirFromWindowsHost({ host, guestRoot, localTargetDir, dryRun }) {
  const guestRootForward = guestRoot.replace(/\\/g, '/')
  if (dryRun) {
    console.log(`[dry] ssh ${host} tar -cf - -C ${guestRootForward} . | tar -xf - -C ${localTargetDir}`)
    return
  }
  const result = spawnSync(
    'bash',
    [
      '-c',
      `ssh ${quote(host)} tar -cf - -C ${quote(guestRootForward)} . | tar -xf - -C ${quote(localTargetDir)}`,
    ],
    { stdio: ['ignore', 'inherit', 'inherit'] },
  )
  if (result.status !== 0) {
    throw new Error(`tar pull from ${host}:${guestRoot} failed (exit ${result.status})`)
  }
}

function buildWindowsArtifacts({ env, tag, artifactDir, dryRun, builtLines, includeUpdaterArtifacts, includeExtraPackages }) {
  // Windows builds run on win11-dev — an x86_64 Windows VM reachable over the
  // Nostr VPN mesh (see ~/.claude/CLAUDE.md). Set SQD_WINDOWS_SSH_HOST or the
  // legacy SQD_WINDOWS_VM_NAME to override.
  const host = env.SQD_WINDOWS_SSH_HOST || env.SQD_WINDOWS_VM_NAME || 'win11-dev'

  if (!dryRun) {
    const probe = spawnSync(
      'ssh',
      ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10', host, 'whoami'],
      { stdio: ['ignore', 'pipe', 'pipe'] },
    )
    if (probe.status !== 0) {
      throw new SkipStepError(
        `Skipping Windows artifacts because ssh ${host} is unreachable. ` +
          'Bring up the VM and ensure VPN is connected, or set SQD_WINDOWS_SSH_HOST.',
      )
    }
  }

  const guestRepo = env.SQD_WINDOWS_GUEST_REPO_PATH || 'C:\\src\\squirreldisk'

  syncRepoToWindowsHost({ host, guestRepo, dryRun })

  const rawOutDir = join(artifactDir, 'windows-raw')
  if (!dryRun) {
    rmSync(rawOutDir, { recursive: true, force: true })
    mkdirSync(rawOutDir, { recursive: true })
  }
  const guestRawOutDir = `${guestRepo}\\.local\\release-artifacts\\${tag}\\windows-raw`

  let buildError = null
  try {
    runWindowsPowerShell(
      host,
      `
$ErrorActionPreference = 'Stop'
Set-Location ${psQuote(guestRepo)}
rustup target add x86_64-pc-windows-msvc | Out-Null
npm.cmd ci
npm.cmd run tauri -- build --target x86_64-pc-windows-msvc --ci
$bundleRoot = Join-Path ${psQuote(guestRepo)} 'src-tauri\\target\\x86_64-pc-windows-msvc\\release\\bundle'
if (!(Test-Path $bundleRoot)) { throw "Missing bundle output at $bundleRoot" }
$out = ${psQuote(guestRawOutDir)}
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force -Path $out | Out-Null
Get-ChildItem $bundleRoot -Recurse -File -Include '*.exe','*.msi','*.zip','*.sig' | ForEach-Object {
  Copy-Item $_.FullName (Join-Path $out $_.Name) -Force
}
`,
      { dryRun },
    )
    pullDirFromWindowsHost({ host, guestRoot: guestRawOutDir, localTargetDir: rawOutDir, dryRun })
  } catch (error) {
    buildError = error
  }

  const assets = collectNewestByExt({
    sourceDir: rawOutDir,
    artifactDir,
    tag,
    platform: 'windows',
    arch: 'x64',
    extensions: windowsReleaseExtensions({ includeUpdaterArtifacts, includeExtraPackages }),
    builtLines,
    dryRun,
  })

  if (!dryRun) {
    rmSync(rawOutDir, { recursive: true, force: true })
  }

  if (assets.length === 0) {
    if (buildError) {
      throw buildError
    }
    throw new SkipStepError(`Windows build completed but no bundle artifacts were found in ${rawOutDir}.`)
  }

  if (buildError) {
    builtLines.push(`Windows Tauri build exited after producing bundles: ${buildError.message}`)
  }

  return assets
}

function checkGitClean({ allowDirty, dryRun }) {
  if (allowDirty || dryRun) {
    return
  }
  const status = run('git', ['status', '--porcelain'], { capture: true })
  if (status.trim()) {
    throw new Error('Working tree is dirty. Commit changes first or pass --allow-dirty.')
  }
}

function resolveReleaseCommit(tag, { dryRun }) {
  if (dryRun) {
    return tag
  }
  return run('git', ['rev-parse', 'HEAD'], { capture: true })
}

function describeAsset(name) {
  if (name.endsWith('.dmg')) return 'macOS installer'
  if (name.endsWith('.app.tar.gz')) return 'macOS updater archive'
  if (name.endsWith('.AppImage')) return 'Linux AppImage'
  if (name.endsWith('.deb')) return 'Linux Debian package'
  if (name.endsWith('.rpm')) return 'Linux RPM package'
  if (name.endsWith('.exe')) return 'Windows installer'
  if (name.endsWith('.msi')) return 'Windows MSI installer'
  if (name.endsWith('.zip')) return 'Windows updater archive'
  return name
}

function writeReleaseNotes({ tag, commit, assetPaths, builtLines, skippedLines }) {
  const lines = [
    `# SquirrelDisk ${tag}`,
    '',
    '## Downloads',
    '',
  ]

  for (const assetPath of [...assetPaths].sort((left, right) => basename(left).localeCompare(basename(right)))) {
    const name = basename(assetPath)
    lines.push(`- ${describeAsset(name)}: \`${name}\``)
  }

  lines.push('', '## Release Build', '', `- Built from commit \`${commit}\`.`)

  for (const line of builtLines) {
    lines.push(`- ${line}`)
  }

  if (skippedLines.length > 0) {
    lines.push('', '## Skipped or Not Built', '')
    for (const line of skippedLines) {
      lines.push(`- ${line}`)
    }
  }

  return `${lines.join('\n')}\n`
}

function stageRelease({ tag, commit, stageDir, assetPaths, builtLines, skippedLines, dryRun }) {
  if (assetPaths.length === 0) {
    if (dryRun) {
      console.log(`Would stage ${tag} at ${stageDir}`)
      return
    }
    throw new Error('No assets were produced; nothing to stage.')
  }

  console.log(`Staging ${tag} at ${stageDir}`)
  if (dryRun) {
    return
  }

  rmSync(stageDir, { recursive: true, force: true })
  mkdirSync(join(stageDir, 'assets'), { recursive: true })

  const stagedAssetPaths = []
  for (const assetPath of assetPaths) {
    const targetPath = join(stageDir, 'assets', basename(assetPath))
    copyFileSync(assetPath, targetPath)
    stagedAssetPaths.push(targetPath)
  }

  const createdAt = Math.floor(Date.now() / 1000)
  const assets = stagedAssetPaths
    .map((assetPath) => ({
      name: basename(assetPath),
      path: `assets/${basename(assetPath)}`,
      size: statSync(assetPath).size,
    }))
    .sort((left, right) => left.name.localeCompare(right.name))

  writeFileSync(
    join(stageDir, 'release.json'),
    `${JSON.stringify(
      {
        id: tag,
        title: tag,
        tag,
        commit,
        created_at: createdAt,
        published_at: createdAt,
        draft: false,
        prerelease: tag.includes('-'),
        notes_file: 'notes.md',
        assets,
      },
      null,
      2,
    )}\n`,
  )
  writeFileSync(
    join(stageDir, 'notes.md'),
    writeReleaseNotes({ tag, commit, assetPaths: stagedAssetPaths, builtLines, skippedLines }),
  )
}

function publishRelease({ stageDir, releaseTree, tag, dryRun }) {
  if (dryRun) {
    console.log(`Would publish ${tag} from ${stageDir} into ${releaseTree}`)
    return 'dry-run'
  }

  const addOutput = run('htree', ['add', stageDir], { capture: true })
  console.log(addOutput)
  const match = addOutput.match(/^\s*(?:url|cid):\s*(\S+)/m)
  if (!match) {
    throw new Error('Could not parse htree add output for release CID.')
  }

  const cid = match[1]
  run('htree', ['release', 'publish', releaseTree, tag, cid])
  return cid
}

function resolveOwnerNpub(env, explicitOwner, { dryRun }) {
  if (explicitOwner) return explicitOwner
  if (env.SQD_RELEASE_OWNER_NPUB) return env.SQD_RELEASE_OWNER_NPUB
  if (!dryRun && commandExists('htree')) {
    const output = run('htree', ['user'], { capture: true })
    const match = output.match(/^(npub1[023456789acdefghjklmnpqrstuvwxyz]+)\s+\(self\)/m)
    if (match) {
      return match[1]
    }
  }
  return defaultReleaseOwnerNpub
}

function releasePageUrl(ownerNpub, repoName, tag = null) {
  const params = new URLSearchParams()
  params.set('tab', 'releases')
  if (tag) {
    params.set('id', tag)
  }
  return `https://git.iris.to/#/${ownerNpub}/${repoName}?${params.toString()}`
}

function main() {
  const options = parseArgs(process.argv.slice(2))
  const env = process.env
  const tag = options.tag || readDefaultTag()
  const releaseTree = options.releaseTree || env.SQD_RELEASE_TREE || defaultReleaseTree
  const ownerNpub = resolveOwnerNpub(env, options.ownerNpub, { dryRun: options.dryRun })
  const stageDir = options.stageDir || join(localDir, 'release', tag)
  const artifactDir = options.artifactsDir || join(localDir, 'release-artifacts', tag)
  const builtLines = []
  const skippedLines = []
  const assetPaths = []
  const allowDirty = options.allowDirty || envFlagEnabled(env.SQD_RELEASE_ALLOW_DIRTY)
  const allowPartial = options.allowPartial || envFlagEnabled(env.SQD_RELEASE_ALLOW_PARTIAL)
  const includeUpdaterArtifacts =
    options.includeUpdaterArtifacts || envFlagEnabled(env.SQD_RELEASE_INCLUDE_UPDATER_ARTIFACTS)
  const includeExtraPackages =
    options.includeExtraPackages || envFlagEnabled(env.SQD_RELEASE_INCLUDE_EXTRA_PACKAGES)

  console.log(`Release tag: ${tag}`)
  console.log(`Release tree: ${releaseTree}`)
  console.log(`Release page: ${releasePageUrl(ownerNpub, 'squirreldisk', tag)}`)
  console.log(
    `Release assets: ${[
      'macOS DMG',
      'Linux AppImage',
      'Linux DEB',
      'Windows EXE installer',
      includeUpdaterArtifacts ? 'Tauri updater archives' : null,
      includeExtraPackages ? 'extra RPM/MSI packages' : null,
    ].filter(Boolean).join(', ')}`,
  )
  if (options.dryRun) {
    console.log('Dry run mode: no build, copy, or publish commands will be executed.')
  }

  checkGitClean({ allowDirty, dryRun: options.dryRun })
  if (!options.dryRun) {
    rmSync(artifactDir, { recursive: true, force: true })
    mkdirSync(artifactDir, { recursive: true })
  }

  const steps = [
    ['verify', () => runVerify({ dryRun: options.dryRun, builtLines })],
    ['macos', () => buildMacosArtifacts({
      env,
      tag,
      artifactDir,
      dryRun: options.dryRun,
      builtLines,
      includeUpdaterArtifacts,
    })],
    ['linux', () => buildLinuxArtifacts({
      env,
      tag,
      artifactDir,
      dryRun: options.dryRun,
      builtLines,
      includeExtraPackages,
    })],
    ['windows', () => buildWindowsArtifacts({
      env,
      tag,
      artifactDir,
      dryRun: options.dryRun,
      builtLines,
      includeUpdaterArtifacts,
      includeExtraPackages,
    })],
  ]

  // Track skip/fail reasons per platform so the gate can distinguish "you
  // explicitly opted out via --skip" from "your host couldn't build this and
  // we'd silently publish a partial release".
  const platformSteps = ['macos', 'linux', 'windows']
  const cliSkippedPlatforms = []
  const envSkippedPlatforms = []
  const failedPlatforms = []

  for (const [name, fn] of steps) {
    if ((name === 'verify' && options.skipVerify) || !shouldRunStep(name, options)) {
      skippedLines.push(`${name} skipped by CLI options.`)
      if (platformSteps.includes(name)) cliSkippedPlatforms.push(name)
      continue
    }

    try {
      const stepAssets = fn()
      if (Array.isArray(stepAssets)) {
        assetPaths.push(...stepAssets)
      }
    } catch (error) {
      if (error instanceof SkipStepError) {
        skippedLines.push(error.message)
        if (platformSteps.includes(name)) envSkippedPlatforms.push({ name, reason: error.message })
        continue
      }
      if (name === 'verify') {
        throw error
      }
      skippedLines.push(`${name} build failed: ${error.message}`)
      if (platformSteps.includes(name)) failedPlatforms.push({ name, reason: error.message })
    }
  }

  if (!allowPartial) {
    const blockingLines = [
      ...failedPlatforms.map(({ name, reason }) => `${name} build failed: ${reason}`),
      ...envSkippedPlatforms.map(({ name, reason }) => `${name} skipped (host can't build): ${reason}`),
    ]
    if (blockingLines.length > 0) {
      const hint = envSkippedPlatforms.length > 0
        ? '\n\nRun on a host that can build all targets (e.g. macOS with Docker for Linux + Parallels for Windows), or pass --skip <platform> for the ones you intentionally don\'t want, or pass --allow-partial to override.'
        : ''
      throw new Error(`Refusing to publish a partial release:\n${blockingLines.join('\n')}${hint}`)
    }
  }

  const commit = resolveReleaseCommit(tag, { dryRun: options.dryRun })
  stageRelease({
    tag,
    commit,
    stageDir,
    assetPaths,
    builtLines,
    skippedLines,
    dryRun: options.dryRun,
  })

  if (options.publish) {
    if (!commandExists('htree')) {
      throw new Error('Missing htree; cannot publish release.')
    }
    const cid = publishRelease({ stageDir, releaseTree, tag, dryRun: options.dryRun })
    console.log(`Published ${tag} to ${releaseTree} via ${cid}`)
    console.log(`Release page: ${releasePageUrl(ownerNpub, 'squirreldisk', tag)}`)
  } else {
    console.log(`Staged release at ${stageDir}`)
    console.log(`Release page after publish: ${releasePageUrl(ownerNpub, 'squirreldisk', tag)}`)
  }
}

try {
  main()
} catch (error) {
  console.error(error.message)
  process.exit(1)
}
