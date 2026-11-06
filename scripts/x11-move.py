"""Move the pointer on an X11 display, for headless verification.

Usage: python3 scripts/x11-move.py :120 0x200001 100 780
"""

import ctypes
import ctypes.util
import sys

REVERT_TO_PARENT = 2
CURRENT_TIME = 0


def main(display_name: str, window_hex: str, x: int, y: int) -> int:
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

    display = x11.XOpenDisplay(display_name.encode())
    if not display:
        print(f"cannot open display {display_name}", file=sys.stderr)
        return 2

    x11.XSetInputFocus(display, int(window_hex, 0), REVERT_TO_PARENT, CURRENT_TIME)
    xtst.XTestFakeMotionEvent(display, -1, x, y, 0)
    x11.XFlush(display)
    x11.XCloseDisplay(display)
    print(f"pointer at ({x}, {y})")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 5:
        print(__doc__, file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(main(sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])))
