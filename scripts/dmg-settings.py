# dmgbuild settings for Amgi's disk image, read by scripts/dmg.sh.
# The layout matches media/dmg-background.tiff, whose hand-drawn arrow points from the
# app to the Applications shortcut. Coordinates are icon centers, from the top left.
import os

app = defines["app"]  # noqa: F821 (dmgbuild provides `defines`)

format = "ULFO"  # LZFSE: smaller than zlib, and every supported macOS reads it
filesystem = "APFS"

files = [app]
symlinks = {"Applications": "/Applications"}
icon = os.path.join(app, "Contents", "Resources", "Amgi.icns")

# Generated in the amgi-install repo as one TIFF holding a 1x and a 2x image; it's
# copied as it is, so Finder picks the image for the screen.
background = defines["background"]  # noqa: F821

# The height counts Finder's title bar (32 pt on macOS 26) on top of the 400 pt background.
window_rect = ((200, 160), (660, 432))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False

icon_size = 128
text_size = 12  # Finder's default; dmgbuild's own is 16
icon_locations = {
    os.path.basename(app): (165, 190),
    "Applications": (495, 190),
}
