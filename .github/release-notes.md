## SquirrelDisk 2.1.4

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
