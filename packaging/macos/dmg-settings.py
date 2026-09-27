# dmgbuild settings for the drag-and-drop DMG.
# Usage: dmgbuild -s packaging/macos/dmg-settings.py -D app=path/to/SquirrelDisk.app SquirrelDisk SquirrelDisk-macOS.dmg
import os.path

app = defines.get("app", "dist/SquirrelDisk.app")  # noqa: F821 (provided by dmgbuild)
app_name = os.path.basename(app)

format = "UDZO"
files = [app, ("packaging/macos/If-SquirrelDisk-wont-open.txt", "If SquirrelDisk won't open.txt")]
symlinks = {"Applications": "/Applications"}
icon = "assets/icon/SquirrelDisk.icns"
background = "packaging/macos/dmg-background.png"  # @2x picked up automatically

default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
window_rect = ((200, 120), (660, 460))
icon_size = 112
text_size = 13
icon_locations = {app_name: (165, 175), "Applications": (495, 175), "If SquirrelDisk won't open.txt": (330, 385)}
hide_extensions = [app_name]
