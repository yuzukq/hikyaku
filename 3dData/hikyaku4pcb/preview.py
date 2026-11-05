#!/usr/bin/env python3
"""Fusion なしでケースの形を確かめる。

hikyaku4pcb_case/hikyaku4pcb_case.py の build() を manifold3d で実行して、
stl/ に部品ごとの STL（Z 上向き、mm）と、preview_*.png を書き出す。

    python3 -m venv .venv && .venv/bin/pip install manifold3d numpy matplotlib
    .venv/bin/python preview.py
"""
import importlib.util
import os
import sys

import numpy as np
from manifold3d import CrossSection, Manifold

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("case", os.path.join(HERE, "hikyaku4pcb_case", "hikyaku4pcb_case.py"))
case = importlib.util.module_from_spec(spec)
spec.loader.exec_module(case)

SEG_DEG = 4.0


def polygon(shape):
    if shape[0] == "circle":
        _, cx, cy, r = shape
        n = max(24, int(360 / SEG_DEG))
        a = np.linspace(0, 2 * np.pi, n, endpoint=False)
        return np.c_[cx + r * np.cos(a), cy + r * np.sin(a)]
    pts = []
    for seg in case.path_segments(shape):
        if seg[0] == "L":
            pts.append(seg[1])
        else:
            (x0, y0), (xm, ym), (x1, y1) = seg[1], seg[2], seg[3]
            # 3 点から中心を求めて分割
            d = 2 * (x0 * (ym - y1) + xm * (y1 - y0) + x1 * (y0 - ym))
            ux = ((x0**2 + y0**2) * (ym - y1) + (xm**2 + ym**2) * (y1 - y0) + (x1**2 + y1**2) * (y0 - ym)) / d
            uy = ((x0**2 + y0**2) * (x1 - xm) + (xm**2 + ym**2) * (x0 - x1) + (x1**2 + y1**2) * (xm - x0)) / d
            r = np.hypot(x0 - ux, y0 - uy)
            a0 = np.arctan2(y0 - uy, x0 - ux)
            a1 = np.arctan2(y1 - uy, x1 - ux)
            sweep = (a1 - a0) % (2 * np.pi)
            n = max(2, int(np.degrees(sweep) / SEG_DEG))
            for t in np.linspace(0, 1, n, endpoint=False):
                a = a0 + sweep * t
                pts.append((ux + r * np.cos(a), uy + r * np.sin(a)))
    return np.array(pts)


class ManifoldBackend:
    def __init__(self):
        self.parts = {}
        self.cur = None

    def component(self, name):
        self.cur = name
        self.parts[name] = []

    def prism(self, shapes, axis, a, b, sketch=None):
        cs = None
        for sh in shapes:
            c = CrossSection([polygon(sh)])
            cs = c if cs is None else cs + c
        lo, hi = min(a, b), max(a, b)
        m = Manifold.extrude(cs, hi - lo)
        if axis == "z":
            return m.translate((0, 0, lo))
        if axis == "x":   # (u, v, w) -> (w, u, v)
            return m.translate((0, 0, lo)).transform(np.array([[0, 0, 1, 0], [1, 0, 0, 0], [0, 1, 0, 0]], float))
        # y: (u, v, w) -> (u, -w, v)
        return m.translate((0, 0, -hi)).transform(np.array([[1, 0, 0, 0], [0, 0, -1, 0], [0, 1, 0, 0]], float))

    def cone(self, cx, cy, z0, r0, z1, r1):
        return Manifold.cylinder(z1 - z0, r0, r1, int(360 / SEG_DEG)).translate((cx, cy, z0))

    def cut(self, t, tool):
        return t - tool

    def join(self, t, tool):
        return t + tool

    def intersect(self, t, tool):
        return t ^ tool

    def finish(self, body, name, color=None):
        self.parts[self.cur].append((name, color, body))

    def fillet_outer_loop(self, *a):
        pass

    def fillet_circle(self, *a):
        pass


def tris(m):
    mesh = m.to_mesh()
    v = np.asarray(mesh.vert_properties)[:, :3]
    f = np.asarray(mesh.tri_verts)
    return v, f


def write_stl(path, m):
    v, f = tris(m)
    t = v[f]
    n = np.cross(t[:, 1] - t[:, 0], t[:, 2] - t[:, 0])
    n /= np.linalg.norm(n, axis=1, keepdims=True) + 1e-12
    with open(path, "wb") as fp:
        fp.write(b"hikyaku4pcb".ljust(80, b" "))
        fp.write(np.uint32(len(f)).tobytes())
        rec = np.zeros(len(f), dtype=[("n", "<f4", 3), ("v", "<f4", (3, 3)), ("a", "<u2")])
        rec["n"], rec["v"] = n, t
        fp.write(rec.tobytes())


COLORS = {"pcb": (0.15, 0.45, 0.25), "module": (0.2, 0.55, 0.35), "metal": (0.75, 0.75, 0.78),
          "switch": (0.85, 0.3, 0.3), "keycap": (0.92, 0.92, 0.9), "pad": (0.25, 0.25, 0.28),
          "case": (0.86, 0.84, 0.78), "ring": (0.22, 0.22, 0.24)}


def render(path, items, elev, azim, title, size=(1200, 900)):
    """小さな Z バッファの描画（matplotlib は面の前後を正しく並べないため）。"""
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    W, H = size
    e, a = np.radians(elev), np.radians(azim)
    view = np.array([np.cos(e) * np.cos(a), np.cos(e) * np.sin(a), np.sin(e)])     # カメラへ向かう向き
    right = np.array([-np.sin(a), np.cos(a), 0.0])
    up = np.cross(view, right)
    light = 0.7 * view + 0.5 * up + 0.3 * right
    light /= np.linalg.norm(light)
    tri_all, col_all = [], []
    for m, rgb, alpha, dz in items:
        v, f = tris(m)
        t = (v + np.array([0, 0, dz]))[f]
        tri_all.append(t)
        col_all.append(np.repeat(np.array(rgb)[None, :], len(t), 0))
    T = np.concatenate(tri_all)
    C = np.concatenate(col_all)
    n = np.cross(T[:, 1] - T[:, 0], T[:, 2] - T[:, 0])
    n /= np.linalg.norm(n, axis=1, keepdims=True) + 1e-12
    keep = n @ view > 1e-6
    T, C, n = T[keep], C[keep], n[keep]
    shade = 0.35 + 0.65 * np.clip(n @ light, 0, 1) + 0.15 * np.clip(n @ view, 0, 1) ** 8
    C = np.clip(C * shade[:, None], 0, 1)
    sx, sy, sz = T @ right, T @ up, T @ view
    lo = np.array([sx.min(), sy.min()])
    hi = np.array([sx.max(), sy.max()])
    scale = 0.9 * min(W / (hi[0] - lo[0]), H / (hi[1] - lo[1]))
    cx, cy = (lo + hi) / 2
    px = (sx - cx) * scale + W / 2
    py = H / 2 - (sy - cy) * scale
    img = np.ones((H, W, 3))
    zb = np.full((H, W), -np.inf)
    ids = np.full((H, W), -1)
    for i in range(len(T)):
        x, y, z = px[i], py[i], sz[i]
        x0, x1 = max(int(np.floor(x.min())), 0), min(int(np.ceil(x.max())), W - 1)
        y0, y1 = max(int(np.floor(y.min())), 0), min(int(np.ceil(y.max())), H - 1)
        if x1 < x0 or y1 < y0:
            continue
        gx, gy = np.meshgrid(np.arange(x0, x1 + 1) + 0.5, np.arange(y0, y1 + 1) + 0.5)
        d = (y[1] - y[2]) * (x[0] - x[2]) + (x[2] - x[1]) * (y[0] - y[2])
        if abs(d) < 1e-12:
            continue
        w0 = ((y[1] - y[2]) * (gx - x[2]) + (x[2] - x[1]) * (gy - y[2])) / d
        w1 = ((y[2] - y[0]) * (gx - x[2]) + (x[0] - x[2]) * (gy - y[2])) / d
        w2 = 1 - w0 - w1
        inside = (w0 >= -1e-6) & (w1 >= -1e-6) & (w2 >= -1e-6)
        if not inside.any():
            continue
        zz = w0 * z[0] + w1 * z[1] + w2 * z[2]
        sub = zb[y0:y1 + 1, x0:x1 + 1]
        upd = inside & (zz > sub)
        sub[upd] = zz[upd]
        img[y0:y1 + 1, x0:x1 + 1][upd] = C[i]
        ids[y0:y1 + 1, x0:x1 + 1][upd] = i
    # 輪郭: 深さの段差か、向きの大きく違う面の境目を暗くする
    nn = np.zeros((H, W, 3))
    m = ids >= 0
    nn[m] = n[ids[m]]
    edge = np.zeros((H, W), bool)
    for dy, dx in ((0, 1), (1, 0)):
        a_ = np.roll(np.roll(zb, dy, 0), dx, 1)
        b_ = np.roll(np.roll(nn, dy, 0), dx, 1)
        dz = np.abs(np.nan_to_num(zb - a_, posinf=1e9, neginf=1e9))
        edge |= (dz > 0.6) | ((nn * b_).sum(-1) < 0.85)
    edge &= m | np.roll(m, 1, 0) | np.roll(m, 1, 1)
    img[edge] *= 0.35
    fig = plt.figure(figsize=(W / 100, H / 100 + 0.4), dpi=100)
    ax = fig.add_axes([0, 0, 1, H / (H + 40)])
    ax.imshow(img)
    ax.set_axis_off()
    fig.text(0.5, 0.985, title, ha="center", va="top", fontsize=13)
    fig.savefig(path)
    plt.close(fig)


def main():
    K = ManifoldBackend()
    case.build(K)
    os.makedirs(os.path.join(HERE, "stl"), exist_ok=True)
    for comp in ("Bottom", "Cover", "TrackpadRing"):
        for name, _, body in K.parts[comp]:
            write_stl(os.path.join(HERE, "stl", name + ".stl"), body)
            bb = body.bounding_box()
            print("%-14s vol %8.1f mm3  bbox %s" % (name, body.volume(), np.round(np.array(bb), 2).tolist()))

    # 干渉の確認: 仮の箱とケースの重なり
    case_parts = [b for c in ("Bottom", "Cover", "TrackpadRing") for _, _, b in K.parts[c]]
    for name, _, ref in K.parts.get("Ref_parts", []) + K.parts["PCB_ref"]:
        for cp, cn in zip(case_parts, ("Bottom", "Cover", "TrackpadRing")):
            v = (ref ^ cp).volume()
            if v > 0.01:
                print("干渉: %s と %s が %.2f mm3 重なっている" % (name, cn, v))

    if "--no-render" in sys.argv:
        return
    def items(explode):
        out = []
        for comp, dz in (("PCB_ref", 0), ("Ref_parts", 0), ("Bottom", 0), ("Cover", 25), ("TrackpadRing", 45)):
            for _, col, b in K.parts.get(comp, []):
                d = dz if explode else 0
                if comp == "Ref_parts" and col in ("pad",):
                    d = 45 if explode else 0
                out.append((b, COLORS[col], 1.0, d))
        return out
    asm = [(b, COLORS[col], 1.0, 0) for comp in ("Bottom", "Cover", "TrackpadRing") for _, col, b in K.parts[comp]]
    render(os.path.join(HERE, "preview_assembled.png"), asm, 32, -60, "hikyaku4pcb case (assembled)")
    render(os.path.join(HERE, "preview_exploded.png"), items(True), 22, -60, "hikyaku4pcb case (exploded, with reference parts)")
    ring = [(b, COLORS[col], 1.0, 0) for _, col, b in K.parts["TrackpadRing"]]
    render(os.path.join(HERE, "preview_ring.png"), ring, 35, -70, "TrackpadRing")
    bottom = [(b, COLORS[col], 1.0, 0) for comp in ("PCB_ref", "Ref_parts", "Bottom") for _, col, b in K.parts.get(comp, [])
              if col not in ("pad",)]
    render(os.path.join(HERE, "preview_inside.png"), bottom, 50, -60, "Bottom + PCB + reference parts (cover removed)")


if __name__ == "__main__":
    main()
