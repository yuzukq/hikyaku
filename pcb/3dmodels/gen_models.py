#!/usr/bin/env python3
"""Rough 3D placeholders (VRML) for checking placement and interference in the KiCad 3D viewer.
Run: python3 gen_models.py   -> writes *.wrl next to this script.

Coordinates are footprint coordinates in mm, origin = pin 1 (sockets) or the footprint origin (MX).
KiCad reads VRML in units of 2.54mm and flips Y, so boxes are converted below.
Sizes marked MEASURE are guesses: replace them with caliper measurements.
"""
import os

HERE = os.path.dirname(os.path.abspath(__file__))
PITCH = 2.54
SOCKET_H = 3.0  # low-profile socket (Mill-Max 310/315 class)

# Module sizes: (width along columns, length along rows, thickness) in mm
S3_BOARD = (18.0, 22.52, 1.2)   # ESP32-S3 SuperMini
S3_ROW_GAP = 15.24              # centre distance of J1/J2 rows. MEASURE
CH_BOARD = (25.0, 11.0, 1.0)    # AE-CH9102F-TYPEC-BO (spec: 25x11). Where the pins sit on it: MEASURE
FFC_BOARD = (24.0, 26.0, 1.0)   # uxcell FFC 12P adapter (spec: 26x24x11.5, 11.5 = height). Pin position: MEASURE


def box(cx, cy, z0, sx, sy, sz, rgb):
    """Box centred at (cx, cy) in footprint mm, from height z0 up by sz."""
    t = (cx / PITCH, -cy / PITCH, (z0 + sz / 2) / PITCH)
    s = (sx / PITCH, sy / PITCH, sz / PITCH)
    return (f"Transform {{ translation {t[0]:.5f} {t[1]:.5f} {t[2]:.5f} children [ Shape {{ "
            f"appearance Appearance {{ material Material {{ diffuseColor {rgb[0]} {rgb[1]} {rgb[2]} }} }} "
            f"geometry Box {{ size {s[0]:.5f} {s[1]:.5f} {s[2]:.5f} }} }} ] }}\n")


def write(name, body):
    with open(os.path.join(HERE, name + ".wrl"), "w") as f:
        f.write("#VRML V2.0 utf8\n" + "".join(body))


BLACK, GREEN, GREY, SILVER = (0.1, 0.1, 0.1), (0.0, 0.45, 0.2), (0.75, 0.75, 0.75), (0.8, 0.8, 0.85)


def socket(cols, rows):
    return box(-(cols - 1) * PITCH / 2, (rows - 1) * PITCH / 2, 0, cols * PITCH, rows * PITCH, SOCKET_H, BLACK)


def module(board, cx, cy):
    """Board lying on the socket, centred at (cx, cy), 'rows' direction along +Y."""
    return box(cx, cy, SOCKET_H, board[0], board[1], board[2], GREEN)


# In KiCad's 2xN socket footprints the odd pads are at x=0 and the even pads at x=-2.54.
# Sockets alone (for J3 / J4 the module is added on top)
write("PinSocketLow_1x09", [socket(1, 9)])

# J1 carries the whole SuperMini (J2 is the other row, 15.24mm to +X)
s3_cx, s3_cy = S3_ROW_GAP / 2, (9 - 1) * PITCH / 2
s3 = [socket(1, 9),
       module(S3_BOARD, s3_cx, s3_cy),
       box(s3_cx, s3_cy - S3_BOARD[1] / 2 + 3.2, SOCKET_H + S3_BOARD[2], 9.0, 7.4, 3.2, GREY),  # USB-C
       box(s3_cx, s3_cy + 2.5, SOCKET_H + S3_BOARD[2], 13.0, 12.0, 2.0, SILVER)]                  # RF can
write("S3_on_J1", s3)
write("PinSocketLow_1x09_J2", [socket(1, 9)])

# J3 (2x6) / J4 (2x4): socket + placeholder module; both bodies extend toward the even column (-X), checked on the real parts
write("PinSocketLow_2x06_FFC", [socket(2, 6), module(FFC_BOARD, 2.0 - FFC_BOARD[0] / 2, 5 * PITCH / 2),
                                      box(2.0 - FFC_BOARD[0] / 2, 5 * PITCH / 2, SOCKET_H + FFC_BOARD[2], 10.0, 6.0, 11.5 - FFC_BOARD[2], GREY)])  # connector block: height only a guess
write("PinSocketLow_2x04_CH9102F", [socket(2, 4), module(CH_BOARD, 2.0 - CH_BOARD[0] / 2, 3 * PITCH / 2)])  # CH9102F: pins 2mm from the end, body toward the even column (-X)

# MX switch: CherryMX_Red.wrl from usagi.kicad (see LICENSE-usagi.kicad.md); it is in mm, so scale 0.3937 in the footprint
print("done")
