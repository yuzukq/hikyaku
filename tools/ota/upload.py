#!/usr/bin/env python3
"""BLE 経由で hikyaku のファームを更新する（OTA）。

使い方: upload.py <image.bin> <key-hex> [--name hikyaku-OTA]

事前に、キーボードで Ctrl+Alt+Shift+Gui+Enter+U を押して OTA モードにしておく
（hikyaku がホストとの接続を切り、`hikyaku-OTA` として待つ。180 秒で自動的に通常に戻る）。
プロトコルは firmware/hikyaku-s3/src/bin/ota/mod.rs の冒頭を参照。
"""
import argparse
import asyncio
import hashlib
import hmac
import struct
import sys
import time

from bleak import BleakClient, BleakScanner

CTRL = "6b9a0002-6f1e-4b8e-9d57-7f0d0a5b0001"
DATA = "6b9a0003-6f1e-4b8e-9d57-7f0d0a5b0001"
ACK_EVERY = 16 * 1024
STATUS = {0: "OK", 1: "bad state", 2: "authentication failed", 3: "too big / bad length", 4: "flash error",
          5: "hash mismatch", 6: "no OTA partitions (flash once over USB with the new partition table)",
          7: "bad image", 8: "bad request"}


async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("image")
    ap.add_argument("key")
    ap.add_argument("--name", default="hikyaku-OTA")
    ap.add_argument("--timeout", type=float, default=600)
    a = ap.parse_args()

    image = open(a.image, "rb").read()
    key = bytes.fromhex(a.key)
    sha = hashlib.sha256(image).digest()
    print(f"image: {len(image)} bytes, sha256 {sha.hex()[:16]}…")

    print(f"scanning for '{a.name}' (up to {a.timeout:.0f}s)… press Ctrl+Alt+Shift+Gui+Enter+U on the keyboard")
    dev = await BleakScanner.find_device_by_filter(
        lambda d, ad: (ad.local_name or d.name) == a.name, timeout=a.timeout)
    if dev is None:
        print("not found (is the keyboard in OTA mode?)")
        return 1
    print(f"found {dev.address}")

    q: asyncio.Queue[bytes] = asyncio.Queue()

    async def expect(op: int, timeout: float = 15) -> bytes:
        end = time.time() + timeout
        while True:
            left = end - time.time()
            if left <= 0:
                raise TimeoutError(f"no reply 0x{op:02x}")
            msg = await asyncio.wait_for(q.get(), left)
            if msg[0] == op:
                return msg

    async with BleakClient(dev, timeout=20) as c:
        await c.start_notify(CTRL, lambda _, d: q.put_nowait(bytes(d)))
        chunk = min(240, c.mtu_size - 3)
        chunk -= chunk % 4
        print(f"connected, mtu {c.mtu_size}, chunk {chunk}")

        await c.write_gatt_char(CTRL, b"\x01", response=True)
        nonce = (await expect(0x81))[1:17]
        size = struct.pack("<I", len(image))
        mac = hmac.new(key, nonce + size + sha, hashlib.sha256).digest()
        await c.write_gatt_char(CTRL, b"\x02" + size + sha + mac, response=True)
        st = (await expect(0x82))[1]
        if st != 0:
            print("START failed:", STATUS.get(st, st))
            return 2

        t0 = time.time()
        sent = 0
        last_ack = 0
        for i in range(0, len(image), chunk):
            if sent - last_ack >= ACK_EVERY:
                last_ack = struct.unpack("<I", (await expect(0x90))[1:5])[0]
                rate = sent / max(time.time() - t0, 1e-3) / 1024
                print(f"\r  {sent * 100 // len(image):3d}%  {rate:5.1f} KiB/s", end="", flush=True)
            await c.write_gatt_char(DATA, image[i:i + chunk], response=False)
            sent += len(image[i:i + chunk])
        print(f"\r  100%  sent {sent} bytes in {time.time() - t0:.1f}s")

        await c.write_gatt_char(CTRL, b"\x03", response=True)
        st = (await expect(0x83, timeout=30))[1]
        if st != 0:
            print("FINISH failed:", STATUS.get(st, st))
            return 3
        print("update accepted; the keyboard is rebooting")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
