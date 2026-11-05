"""Inject one key chord into an X11 window, for headless verification.

xdotool is not available in this environment, so this uses XTEST directly. It
focuses the target window first: there is no window manager on the Xvfb display,
so focus is otherwise wherever the server left it.

Usage: python3 scripts/x11-key.py :120 0x200001 ctrl+shift+r
"""

import ctypes
import ctypes.util
import sys

MODIFIERS = {
    "ctrl": "Control_L",
    "control": "Control_L",
    "shift": "Shift_L",
    "alt": "Alt_L",
    "super": "Super_L",
}

# XSetInputFocus revert-to values.
REVERT_TO_PARENT = 2
CURRENT_TIME = 0


def main(display_name: str, window_hex: str, chord: str) -> int:
    x11_path = ctypes.util.find_library("X11")
    xtst_path = ctypes.util.find_library("Xtst")
    if x11_path is None or xtst_path is None:
        print("libX11 or libXtst not found", file=sys.stderr)
        return 2

    x11 = ctypes.CDLL(x11_path)
    xtst = ctypes.CDLL(xtst_path)

    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XStringToKeysym.restype = ctypes.c_ulong
    x11.XStringToKeysym.argtypes = [ctypes.c_char_p]
    x11.XKeysymToKeycode.restype = ctypes.c_ubyte
    x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    x11.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    x11.XFlush.argtypes = [ctypes.c_void_p]
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    xtst.XTestFakeKeyEvent.argtypes = [
        ctypes.c_void_p,
        ctypes.c_uint,
        ctypes.c_int,
        ctypes.c_ulong,
    ]

    display = x11.XOpenDisplay(display_name.encode())
    if not display:
        print(f"cannot open display {display_name}", file=sys.stderr)
        return 2

    parts = [part.strip().lower() for part in chord.split("+") if part.strip()]
    if not parts:
        print("empty key chord", file=sys.stderr)
        return 2

    names = [MODIFIERS.get(part, part) for part in parts]
    keycodes = []
    for name in names:
        keysym = x11.XStringToKeysym(name.encode())
        if not keysym:
            print(f"unknown keysym `{name}`", file=sys.stderr)
            return 2
        keycodes.append(x11.XKeysymToKeycode(display, keysym))
        if keycodes[-1] == 0:
            print(f"no keycode for `{name}` on this keymap", file=sys.stderr)
            return 2

    window = int(window_hex, 0)
    x11.XSetInputFocus(display, window, REVERT_TO_PARENT, CURRENT_TIME)
    x11.XFlush(display)

    for keycode in keycodes:
        xtst.XTestFakeKeyEvent(display, keycode, 1, 0)
        x11.XFlush(display)
    for keycode in reversed(keycodes):
        xtst.XTestFakeKeyEvent(display, keycode, 0, 0)
        x11.XFlush(display)

    x11.XCloseDisplay(display)
    print(f"sent {chord} to window {window_hex}")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 4:
        print(__doc__, file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(main(sys.argv[1], sys.argv[2], sys.argv[3]))
