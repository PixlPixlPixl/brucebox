"""Drag the pointer on an X11 display, for headless verification.

xdotool is not available in this environment, so this uses XTEST. It focuses the
target window first: there is no window manager on the Xvfb display.

Usage: python3 scripts/x11-drag.py :120 0x200001 200 40 320 140
       (from x,y to x,y, in small steps so the app sees a drag, not a jump)
"""

import ctypes
import ctypes.util
import sys
import time

REVERT_TO_PARENT = 2
CURRENT_TIME = 0
STEPS = 12


def main(display_name: str, window_hex: str, x0: int, y0: int, x1: int, y1: int) -> int:
    x11_path = ctypes.util.find_library("X11")
    xtst_path = ctypes.util.find_library("Xtst")
    if x11_path is None or xtst_path is None:
        print("libX11 or libXtst not found", file=sys.stderr)
        return 2

    x11 = ctypes.CDLL(x11_path)
    xtst = ctypes.CDLL(xtst_path)

    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    x11.XFlush.argtypes = [ctypes.c_void_p]
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    xtst.XTestFakeMotionEvent.argtypes = [
        ctypes.c_void_p,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_ulong,
    ]
    xtst.XTestFakeButtonEvent.argtypes = [
        ctypes.c_void_p,
        ctypes.c_uint,
        ctypes.c_int,
        ctypes.c_ulong,
    ]

    display = x11.XOpenDisplay(display_name.encode())
    if not display:
        print(f"cannot open display {display_name}", file=sys.stderr)
        return 2

    window = int(window_hex, 0)
    x11.XSetInputFocus(display, window, REVERT_TO_PARENT, CURRENT_TIME)
    x11.XFlush(display)

    xtst.XTestFakeMotionEvent(display, -1, x0, y0, 0)
    x11.XFlush(display)
    time.sleep(0.15)

    xtst.XTestFakeButtonEvent(display, 1, 1, 0)
    x11.XFlush(display)
    time.sleep(0.15)

    for step in range(1, STEPS + 1):
        x = x0 + (x1 - x0) * step // STEPS
        y = y0 + (y1 - y0) * step // STEPS
        xtst.XTestFakeMotionEvent(display, -1, x, y, 0)
        x11.XFlush(display)
        time.sleep(0.05)

    xtst.XTestFakeButtonEvent(display, 1, 0, 0)
    x11.XFlush(display)
    time.sleep(0.15)

    x11.XCloseDisplay(display)
    print(f"dragged ({x0}, {y0}) -> ({x1}, {y1}) on window {window_hex}")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 7:
        print(__doc__, file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(
        main(
            sys.argv[1],
            sys.argv[2],
            int(sys.argv[3]),
            int(sys.argv[4]),
            int(sys.argv[5]),
            int(sys.argv[6]),
        )
    )
