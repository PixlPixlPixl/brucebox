"""Send WM_DELETE_WINDOW to one X11 window, so a GPUI app quits gracefully.

Used only for headless verification on Xvfb: xdotool is not available here, and
killing the process would skip the app's quit hooks (and therefore the workspace
flush this check is about).

Usage: python3 scripts/x11-close-window.py :120 0x200001
"""

import ctypes
import ctypes.util
import sys


class XClientMessageEvent(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int),
        ("serial", ctypes.c_ulong),
        ("send_event", ctypes.c_int),
        ("display", ctypes.c_void_p),
        ("window", ctypes.c_ulong),
        ("message_type", ctypes.c_ulong),
        ("format", ctypes.c_int),
        ("data", ctypes.c_long * 5),
    ]


class XEvent(ctypes.Union):
    _fields_ = [
        ("type", ctypes.c_int),
        ("xclient", XClientMessageEvent),
        ("pad", ctypes.c_long * 24),
    ]


def main(display_name: str, window_hex: str) -> int:
    path = ctypes.util.find_library("X11")
    if path is None:
        print("libX11 not found", file=sys.stderr)
        return 2

    x11 = ctypes.CDLL(path)
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XInternAtom.restype = ctypes.c_ulong
    x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    x11.XSendEvent.restype = ctypes.c_int
    x11.XSendEvent.argtypes = [
        ctypes.c_void_p,
        ctypes.c_ulong,
        ctypes.c_int,
        ctypes.c_long,
        ctypes.POINTER(XEvent),
    ]
    x11.XFlush.argtypes = [ctypes.c_void_p]
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]

    display = x11.XOpenDisplay(display_name.encode())
    if not display:
        print(f"cannot open display {display_name}", file=sys.stderr)
        return 2

    window = int(window_hex, 0)
    wm_protocols = x11.XInternAtom(display, b"WM_PROTOCOLS", 0)
    wm_delete = x11.XInternAtom(display, b"WM_DELETE_WINDOW", 0)

    event = XEvent()
    event.type = 33  # ClientMessage
    event.xclient.window = window
    event.xclient.message_type = wm_protocols
    event.xclient.format = 32
    event.xclient.data[0] = wm_delete
    event.xclient.data[1] = 0  # CurrentTime

    sent = x11.XSendEvent(display, window, 0, 0, ctypes.byref(event))
    x11.XFlush(display)
    x11.XCloseDisplay(display)

    print(f"WM_DELETE_WINDOW sent={sent} window={window_hex}")
    return 0 if sent else 1


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(main(sys.argv[1], sys.argv[2]))
