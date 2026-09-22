#!/usr/bin/env python3
"""A stand-in XDG Desktop Portal for the screenshot round-trip test.

The real portal cannot run in a headless container, so this serves the same
interface on a private session bus and answers with a generated image. Drive it
with the portal screenshot test:

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
REQUEST_PATH = PORTAL_PATH + "/request/mock/1"
SCREENSHOT_IFACE = "org.freedesktop.portal.Screenshot"
REQUEST_IFACE = "org.freedesktop.portal.Request"

# The image the round-trip test expects back.
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


class Portal(dbus.service.Object):
    def __init__(self, connection, image_path):
        super().__init__(connection, PORTAL_PATH)
        self._connection = connection
        self._image_path = image_path

    @dbus.service.method(SCREENSHOT_IFACE, in_signature="sa{sv}", out_signature="o")
    def Screenshot(self, parent_window, options):
        # The result never comes back as the method reply; the caller is told to
        # watch the request object instead.
        GLib.timeout_add(150, self._reply)
        return dbus.ObjectPath(REQUEST_PATH)

    def _reply(self):
        uri = "file://%s" % self._image_path
        Request(self._connection, REQUEST_PATH).Response(dbus.UInt32(0), {"uri": uri})
        print("mock portal: replied with %s" % uri, flush=True)
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
