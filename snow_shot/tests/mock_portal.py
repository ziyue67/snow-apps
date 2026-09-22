#!/usr/bin/env python3
"""A stand-in XDG Desktop Portal for the screenshot and shortcut round-trips.

The real portal cannot run in a headless container, so this serves the same
interfaces on a private session bus and answers with generated results. Drive it
with the portal tests:

    dbus-run-session -- bash -c \
        "python3 snow_shot/tests/mock_portal.py & sleep 2; \
         SNOW_SHOT_TEST_PORTAL=1 ./snow-shot-portal-screenshot-tests"

The shell keeps its own well-known name alive for as long as the bus runs, which
matters: an unreferenced BusName is garbage collected and releases the name, and
the call would then be handed to the real portal instead.
"""

import os
import struct
import sys
import zlib

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

PORTAL_NAME = "org.freedesktop.portal.Desktop"
PORTAL_PATH = "/org/freedesktop/portal/desktop"
SCREENSHOT_IFACE = "org.freedesktop.portal.Screenshot"
SHORTCUTS_IFACE = "org.freedesktop.portal.GlobalShortcuts"
REQUEST_IFACE = "org.freedesktop.portal.Request"
SESSION_IFACE = "org.freedesktop.portal.Session"

SESSION_PATH = PORTAL_PATH + "/session/mock/1"

# The image the screenshot round-trip expects back.
IMAGE_WIDTH = 1
IMAGE_HEIGHT = 1


def write_png(path, width, height):
    """Write a minimal valid RGB PNG without depending on an image library."""

    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    raw = b"".join(b"\x00" + b"\xff\x00\x00" * width for _ in range(height))
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as handle:
        handle.write(png)


class Request(dbus.service.Object):
    """Lives at the request path, which is where the signal has to come from."""

    @dbus.service.signal(REQUEST_IFACE, signature="ua{sv}")
    def Response(self, response, results):
        pass


class Session(dbus.service.Object):
    """Lives at the session path, which is where activations come from."""

    @dbus.service.signal(SHORTCUTS_IFACE, signature="osta{sv}")
    def Activated(self, session_handle, shortcut_id, timestamp, options):
        pass

    @dbus.service.method(SESSION_IFACE, in_signature="", out_signature="")
    def Close(self):
        print("mock portal: session closed", flush=True)
        GLib.idle_add(lambda: (print("mock portal: exiting", flush=True), sys.exit(0)))


class Portal(dbus.service.Object):
    def __init__(self, connection, image_path):
        super().__init__(connection, PORTAL_PATH)
        self._connection = connection
        self._image_path = image_path
        # The client subscribes to whatever request path the method returned, so
        # each call gets its own and the answer cannot land on the wrong one.
        self._requests = 0

    def _next_request_path(self):
        self._requests += 1
        return "%s/request/mock/%d" % (PORTAL_PATH, self._requests)

    def _answer(self, request_path, results, response=0):
        Request(self._connection, request_path).Response(
            dbus.UInt32(response), results
        )
        print("mock portal: answering %s" % request_path, flush=True)
        return False

    @dbus.service.method(SCREENSHOT_IFACE, in_signature="sa{sv}", out_signature="o")
    def Screenshot(self, parent_window, options):
        # The result never comes back as the method reply; the caller is told to
        # watch the request object instead.
        request_path = self._next_request_path()
        uri = "file://%s" % self._image_path
        GLib.timeout_add(150, lambda: self._answer(request_path, {"uri": uri}))
        return dbus.ObjectPath(request_path)

    @dbus.service.method(SHORTCUTS_IFACE, in_signature="a{sv}", out_signature="o")
    def CreateSession(self, options):
        request_path = self._next_request_path()
        GLib.timeout_add(
            150,
            lambda: self._answer(
                request_path, {"session_handle": dbus.String(SESSION_PATH)}
            ),
        )
        return dbus.ObjectPath(request_path)

    @dbus.service.method(SHORTCUTS_IFACE, in_signature="oa(sa{sv})sa{sv}", out_signature="o")
    def BindShortcuts(self, session_handle, shortcuts, parent_window, options):
        request_path = self._next_request_path()
        bound = [str(entry[0]) for entry in shortcuts]
        print("mock portal: binding %s" % bound, flush=True)
        GLib.timeout_add(150, lambda: self._answer(request_path, {"shortcuts": shortcuts}))
        # Fire the first bound shortcut shortly after the bind is acknowledged, so
        # the caller has already attached its activation handler.
        if bound:
            GLib.timeout_add(400, lambda: self._activate(bound[0]))
        return dbus.ObjectPath(request_path)

    def _activate(self, shortcut_id):
        Session(self._connection, SESSION_PATH).Activated(
            dbus.ObjectPath(SESSION_PATH), shortcut_id, dbus.UInt64(0), {}
        )
        print("mock portal: activated %s" % shortcut_id, flush=True)
        return False


def main():
    image_path = os.path.join(os.environ.get("SHOT_DIR", "/tmp"), "shot.png")
    write_png(image_path, IMAGE_WIDTH, IMAGE_HEIGHT)

    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    connection = dbus.SessionBus()
    name = dbus.service.BusName(PORTAL_NAME, connection, do_not_queue=True)
    portal = Portal(connection, image_path)
    print("mock portal: serving %s as %s" % (PORTAL_NAME, name), flush=True)
    # Keep the references alive for the lifetime of the loop.
    assert portal is not None and name is not None
    GLib.MainLoop().run()
    return 0


if __name__ == "__main__":
    sys.exit(main())
