## SquirrelDisk 2.6.0

- **Find duplicates.** A new tool on the home screen finds identical files across folders, disks, servers over SSH and cloud storage, all compared together. Files are matched by content, not by name, so renamed copies are found too, and results show up while the scan runs, biggest savings first. Only files that share a size are ever read; big files are compared by samples spread over the whole file (turn on "Exact comparison" to read every byte), and fingerprints are remembered so comparing again is quick.
- **Keep the right copy.** For every set, one copy is kept and the others are checked for removal: "Always keep" rules protect folders or patterns like `**/Archive/**`, otherwise the oldest, newest or shortest-path copy stays, and your own ticks always win. Files inside app libraries (Photos, Final Cut Pro…) are never removed.
- **Previews and details.** Photos, RAW files and videos show a preview (Quick Look on macOS) with dimensions, duration and camera, plus the full path of every copy.
- **Remove safely.** Move to the Trash or delete permanently. A copy of every file always stays, it's checked before anything is removed, and files changed since the scan are left alone. APFS clones and hard links are recognised, so the space you get back is real.
- Moving things to the Trash no longer plays Finder's sound once per item, and no longer asks for permission to control Finder (macOS).

### From 2.5.1


- **See what's new before you update.** When a new version is ready, a window shows what changed, with "Restart now" or "Later". SquirrelDisk now checks for updates every hour while it's open; the ribbon at the top works as before.
- **Hidden space, explained.** Click "Hidden space" to see what it is made of: on macOS the system volumes that share the disk (VM, Preboot and others), on Linux the blocks the file system keeps in reserve, and the rest that a scan can't read. It no longer offers "Open", which led nowhere. Thanks @schroef for asking!
- **The command line asks the open app.** While SquirrelDisk is open, `squirreldisk scan <folder>` answers at once from a scan the app has already finished, instead of scanning again (macOS and Linux). Add `--fresh` to scan anyway. Thanks @paymog and @vladartym!
- **macOS: cloned files count once.** Copies made with Finder's Duplicate, `cp -c` or by package managers share their space on disk. Each one was counted in full, so a disk could add up to more than its size. Thanks @paymog!
- Fixed: on macOS the chart and the list were missing in full screen. Thanks @pshirali!
- Fixed: Chinese, Japanese, Korean, Arabic, Thai and Indian text sat a few pixels above the Latin text next to it. Thanks @AoSankaku!
- The sponsor list is now refreshed every hour while the app is open, and its request says which operating system and CPU type you use. The [privacy page](https://www.squirreldisk.com/privacy) has the details.

### From 2.5

- **Scan from the terminal.** `squirreldisk <folder>` opens SquirrelDisk on that folder, and `squirreldisk scan <folder>` prints the biggest items without opening a window: as a tree (`--depth`, `--top`) or as JSON (`--json`) for your scripts. The Windows installer adds the command for you; on macOS and Linux SquirrelDisk sets it up on first launch when it can, or from Settings → Command-line tool.
- **"Scan with SquirrelDisk" on any folder.** Turn on Settings → Folder menu and right-click a folder: in Finder's Quick Actions on macOS, in Explorer on Windows (under "Show more options" on Windows 11), and in Dolphin, Nemo, Nautilus or "Open with" on Linux. It's off unless you turn it on.
- **Chinese, Japanese, Korean, Thai, Arabic and Indian file names** now show whatever the app's language, instead of empty boxes. Thanks @ryerwera!
- The window opens centred on the screen you're using, at a comfortable size.
- The update button now hangs from the top of the window, on every screen.
- Fixed: Hindi, Bengali, Tamil and Telugu text on recent Windows 11 builds; a crash in the background with some non-Latin folder names.

### From 2.4

- **Portable mode (Windows and Linux).** Create a folder named `portable` next to the `.exe`, the `.AppImage` or the `squirreldisk` binary, and SquirrelDisk keeps everything there: settings, the rclone it installs and your cloud accounts. Perfect for a USB stick. You can also pick any folder with the `SQUIRRELDISK_CONFIG_DIR` environment variable, on every platform.

### From 2.3.1

- **Linux:** the AppImage now includes the keyboard libraries (xkbcommon) it loads at startup, so it also opens on minimal systems that don't have them installed.

### From 2.3

- **Cloud accounts, built in.** Home → Cloud storage now installs rclone for you with one click (official build, checksum-verified, no admin rights needed) on macOS, Windows and Linux.
- **Add, edit and remove accounts** without the terminal: Google Drive, Dropbox, OneDrive, Box and pCloud sign in through your browser; S3 and compatible, Backblaze B2, SFTP, FTP, WebDAV/Nextcloud and MEGA take a short form. The connection is tested when you save. Anything else is one click away in `rclone config`.
- rclone installed with Homebrew, Scoop, winget or Chocolatey is now found even when SquirrelDisk is opened from the Finder or Start menu.

### From 2.2.1

- The language picker in Settings shows every language in its own script (no more empty boxes) and closes when you click outside it.
- The "Check for updates" button fits its label in every language.
- The treemap shows scan progress while scanning.

### From 2.2

- **Treemap view.** Prefer boxes to rings? Pick "Treemap" in Settings → Chart style: every folder becomes a box inside its parent, with names and sizes. Zooming, hovering, collecting and deleting work just like in the sunburst.
- **⌘, opens Settings** from anywhere (Ctrl+, on Windows and Linux).

### From 2.1.4
- **Linux AppImage is back.** Download `SquirrelDisk-x86_64.AppImage`, make it executable and run it. It works with AppImageUpdate and Gear Lever, and SquirrelDisk also updates it by itself.
- **Automatic updates.** New versions download in the background; a "Restart to update" button appears when one is ready. Turn it off in Settings → Install updates automatically. SquirrelDisk also checks again every 6 hours while it's open.
- Settings and the home screen scroll when the window is small, so nothing ends up off screen.
- Quieter finish: the pop-up at the end of a scan is gone.

### From 2.1

- **25 languages.** SquirrelDisk now speaks English, 简体中文, हिन्दी, Español, Français, العربية, বাংলা, Português, Русский, اردو, Bahasa Indonesia, Deutsch, 日本語, मराठी, తెలుగు, Türkçe, தமிழ், Tiếng Việt, 한국어, Italiano, فارسی, Polski, Українська, ไทย and Nederlands. It follows your system language; change it in Settings.
- **Secure erase.** When deleting permanently, tick "Secure erase" to overwrite every file three times (zeros, ones, random data — DoD 5220.22-M) before it's removed. On SSDs and APFS some old data can survive: full-disk encryption (FileVault, BitLocker) protects it completely.
- Flat buttons and smoother progress bars.

### Downloads
- **macOS:** `SquirrelDisk-macOS.dmg` (Apple silicon and Intel) — open it and drag SquirrelDisk to Applications.
- **Windows:** `SquirrelDisk-Windows.msi` (installer) or `squirreldisk-x86_64-pc-windows-msvc.exe` (portable).
- **Linux:** `SquirrelDisk-x86_64.AppImage` (recommended) or `SquirrelDisk-Linux-x86_64.tar.gz`.

### First launch
**macOS** — if macOS says SquirrelDisk can't be opened: open it once, then go to **System Settings → Privacy & Security** and click **Open Anyway**. If it says the app is damaged, run in Terminal:

```
xattr -dr com.apple.quarantine /Applications/SquirrelDisk.app
```

**Windows** — if you see "Windows protected your PC", click **More info → Run anyway**. For the portable `.exe` you can also right-click it → **Properties** → tick **Unblock**.

The `squirreldisk-agent-*` files are used automatically when scanning remote servers.
