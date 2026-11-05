"""hikyaku4pcb のケースを Fusion に組み立てるスクリプト。

Fusion の「ユーティリティ > スクリプトとアドイン」でこのフォルダを追加して実行する。
開いているデザイン（hikyaku4pcb）に、次のコンポーネントを作る（同じ名前のものは消して作り直す）:

  PCB_ref       基板（ガーバーの外形と穴から作ったスケッチと 1.6mm の板）
  Ref_parts     モジュール・スイッチ・トラックパッドの仮の箱（干渉の確認用。寸法は推定）
  Bottom        底のトレイ（床・壁・基板を受けるボス・USB-C の穴）
  Cover         ふた（キーの窓・FFC の穴・ねじのボス）
  TrackpadRing  トラックパッドを上から包む円形のトッププレート

座標（このファイルの中の値はすべて mm）:
  原点 = 基板の左下の角（KiCad の (54, 148)）。X は右、Y は奥（KiCad の -Y）、Z は上。
  Z = 0 が基板の下面。Fusion が Y 上向きの設定なら、自動で Z を Fusion の Y に回して作る。

寸法を変えたいときは、下の「パラメータ」を書き換えて実行し直す。
同じファイルを preview.py が読み込んで、STL とプレビュー画像を作る（Fusion なしで形を確かめる用）。
"""
import math
import traceback

try:
    import adsk.core
    import adsk.fusion
except ImportError:  # preview.py から読み込むとき
    adsk = None

# ============================================================ パラメータ（mm）
UP = "auto"            # "auto" = Fusion の設定に従う / "Y" / "Z"
MAKE_REF_PARTS = True  # 干渉確認用の仮の箱を作る

# --- 基板（ガーバー: hikyaku-Edge_Cuts.gm1 / *.drl）
PCB_W = 63.585786
PCB_H = 52.0
PCB_T = 1.6

# --- 高さの積み上げ（実物を測って直す）
SOCKET_H = 3.0         # 低背ピンソケットの高さ
MODULE_LIFT = 2.5      # ソケットの上面からモジュール基板の下面まで（ピンヘッダの樹脂 ≒ 2.5）
STANDOFF = 3.0         # 床の上面から基板の下面まで（裏に出るピン・はんだの逃げ）
INNER_H = 11.0         # 基板の上面からふたの下面まで（S3 の USB-C・FFC 変換基板のコネクタが収まる高さ）

# --- 箱
GAP = 0.5              # 基板と壁のすき間
WALL = 2.0
FLOOR_T = 2.5
COVER_T = 2.0
R_IN = 3.0             # 内側の角の丸み（外側は R_IN + WALL）
CAV_TOP_Y = 68.5       # 内側の奥の端。CH9102F が基板の奥に約 16mm はみ出すので、その USB-C の端に合わせる
LIP_H = 1.5            # ふたの裏の位置合わせの縁
LIP_T = 1.2
LIP_CLEAR = 0.2

# --- ねじ（M2）
SCREW_D = 2.4          # M2 の通し穴
HEAD_D = 4.4           # 座ぐり
INSERT_D = 3.2         # M2 のヒートセットインサートの下穴（タップ切りなら 1.7 前後）
INSERT_DEPTH = 4.0
BOSS_D = 5.5
FLOOR_CB_DEPTH = 1.8   # 床の裏の座ぐりの深さ

# --- 基板の取り付け穴（M2, 2.2mm）。最後の 1 つはキーキャップの下なので、ねじではなくピンで位置決めだけする
PCB_CLAMP_HOLES = [(3.0, 23.0), (3.0, 49.0), (60.0, 49.0)]
PCB_PEG_HOLE = (61.585786, 2.0)
# 基板の外（奥の張り出し）でふたと底をとめる柱
POSTS = [(-GAP + 3.0, CAV_TOP_Y - 3.0), (PCB_W + GAP - 3.0, CAV_TOP_Y - 3.0)]

# --- MX スイッチ（SW1, SW2）
SW = [(34.81, 10.46), (54.08, 10.46)]
KEYCAP = 18.0
KEY_CLEAR = 0.75       # キーキャップと窓のすき間（片側）

# --- USB-C の穴（プラグの樹脂のかたまりが入る大きさ）
USB_W = 12.4
USB_H = 6.6
S3_USB_Y = 10.62       # S3 SuperMini の中心（J1 と J2 の間）
CH_USB_X = 31.793      # CH9102F の中心（J4 の 2 列の中心）

# --- トラックパッド（旧ケース Top_new.stl の寸法から）
TP_CX = 31.793         # 基板の左右の中心
TP_CY = 45.5
TP_FLANGE_D = 42.0     # 下の出っ張り（ここをふたとリングで挟む）が入る穴
TP_FLANGE_T = 1.5
TP_BODY_D = 40.0       # 上に出る部分が通る窓
TP_ABOVE = 1.5         # 出っ張りの上面からパッドの表面まで
FFC_HOLE = (11.0, 21.0, 0.0, 7.0)  # FFC の穴: 幅, 長さ, パッド中心からのずれ X, Y（旧ボトムの 10×20 を少し広げた）

# --- リング（トッププレート）
RING_SQ = 48.0         # 土台の四角の一辺
RING_SQ_R = 7.0
RING_BASE_T = 2.2      # 四隅の厚さ
RING_PROUD = 1.2       # パッドの表面より上に出る高さ
RING_TOP_R = 23.5      # 上面の外の半径
RING_BAND = 0.8        # 上面の外周の垂直な帯の高さ
RING_FOOT_R = 28.5     # 斜面が土台の上面に下りる半径（四角の辺で切れて平らな面になる）
RING_CHAMFER = 0.8     # 窓の内側の面取り（45°）
RING_SCREW = 19.0      # 四隅のねじの位置（中心からのずれ）
RING_CB_FLOOR = 0.9    # 座ぐりの底（リングの下面からの高さ）
RING_BOSS_H = 4.5      # ふたの裏のボスの長さ

# ============================================================ 導出値
Z_FLOOR_TOP = -STANDOFF
Z_FLOOR_BOT = Z_FLOOR_TOP - FLOOR_T
Z_COVER_BOT = PCB_T + INNER_H
Z_COVER_TOP = Z_COVER_BOT + COVER_T
Z_MOD = PCB_T + SOCKET_H + MODULE_LIFT          # モジュール基板の下面
CAV = (-GAP, -GAP, PCB_W + GAP, CAV_TOP_Y)
OUT = (CAV[0] - WALL, CAV[1] - WALL, CAV[2] + WALL, CAV[3] + WALL)
R_OUT = R_IN + WALL
Z_RING0 = Z_COVER_TOP
Z_PAD = Z_RING0 + TP_FLANGE_T + TP_ABOVE
Z_RING_TOP = Z_PAD + RING_PROUD


# ============================================================ 形（2D）
def circle(cx, cy, r):
    return ("circle", cx, cy, r)


def rrect(x0, y0, x1, y1, r=0.0):
    """角を丸めた長方形（反時計回り）。r が辺の半分なら長円になる。"""
    if r <= 0:
        return ("path", (x0, y0), [("L", x1, y0), ("L", x1, y1), ("L", x0, y1), ("L", x0, y0)])
    return ("path", (x0 + r, y0), [
        ("L", x1 - r, y0), ("A", x1 - r, y0 + r, 90),
        ("L", x1, y1 - r), ("A", x1 - r, y1 - r, 90),
        ("L", x0 + r, y1), ("A", x0 + r, y1 - r, 90),
        ("L", x0, y0 + r), ("A", x0 + r, y0 + r, 90)])


def rrect_c(cx, cy, w, h, r=0.0):
    return rrect(cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2, r)


def pcb_outline():
    """Edge_Cuts そのまま: 奥の角 R5、手前の角 R2。"""
    W, H = PCB_W, PCB_H
    return ("path", (2.0, 0.0), [
        ("L", W - 2.0, 0.0), ("A", W - 2.0, 2.0, 90),
        ("L", W, H - 5.0), ("A", W - 5.0, H - 5.0, 90),
        ("L", 5.0, H), ("A", 5.0, H - 5.0, 90),
        ("L", 0.0, 2.0), ("A", 2.0, 2.0, 90)])


def path_segments(shape):
    """('L', p0, p1) と ('A', p0, 中間点, p1) に分ける。長さ 0 の線は飛ばす。"""
    _, start, segs = shape
    x, y = start
    out = []
    for s in segs:
        if s[0] == "L":
            nx, ny = s[1], s[2]
            if math.hypot(nx - x, ny - y) > 1e-6:
                out.append(("L", (x, y), (nx, ny)))
        else:
            cx, cy, sweep = s[1], s[2], math.radians(s[3])
            r = math.hypot(x - cx, y - cy)
            a0 = math.atan2(y - cy, x - cx)
            am, a1 = a0 + sweep / 2, a0 + sweep
            nx, ny = cx + r * math.cos(a1), cy + r * math.sin(a1)
            out.append(("A", (x, y), (cx + r * math.cos(am), cy + r * math.sin(am)), (nx, ny)))
        x, y = nx, ny
    return out


# ============================================================ 穴（ガーバーのドリルデータ、mm、この座標系）
def kicad(x, y):
    return (round(x - 54.0, 6), round(148.0 - y, 6))


def drill_holes():
    """(x, y, 直径) と長穴 (x0, x1, y, 幅) を返す。"""
    holes = []
    for x in (83.73, 93.89, 103.0, 113.16):                      # MX の足（NPTH 1.9）
        holes.append(kicad(x, 137.54) + (1.9,))
    for x, y in ((57.0, 99.0), (57.0, 125.0), (114.0, 99.0), (115.585786, 146.0)):
        holes.append(kicad(x, y) + (2.2,))                        # 取り付け穴（NPTH 2.2）
    for x in (88.81, 108.08):                                     # MX の中心（NPTH 4.0）
        holes.append(kicad(x, 137.54) + (4.0,))
    for k in range(9):                                            # J1 / J2（S3）
        for y in (129.76, 145.0):
            holes.append(kicad(55.76 + 2.54 * k, y) + (1.0,))
    for k in range(6):                                            # J3（FFC 変換基板）
        for y in (112.73, 115.27):
            holes.append(kicad(79.443 + 2.54 * k, y) + (1.0,))
    for k in range(4):                                            # J4（CH9102F）
        for y in (100.46, 103.0):
            holes.append(kicad(81.983 + 2.54 * k, y) + (1.0,))
    slots = []
    for x0, x1, y in ((84.8, 85.2, 135.0), (91.15, 91.55, 132.46), (104.07, 104.47, 135.0), (110.42, 110.82, 132.46)):
        a, b = kicad(x0, y), kicad(x1, y)
        slots.append((a[0], b[0], a[1], 0.8))                     # MX のピン（PTH 0.8 の長穴）
    return holes, slots


# ============================================================ 組み立て（どちらのバックエンドでも同じ手順）
def build(K):
    holes, slots = drill_holes()
    ring_screws = [(TP_CX + sx * RING_SCREW, TP_CY + sy * RING_SCREW) for sx in (-1, 1) for sy in (-1, 1)]

    # ---------------- PCB_ref
    K.component("PCB_ref")
    pcb = K.prism([pcb_outline()], "z", 0.0, PCB_T, sketch="PCB外形")
    shapes = [circle(x, y, d / 2) for x, y, d in holes]
    shapes += [rrect(x0 - w / 2, y - w / 2, x1 + w / 2, y + w / 2, w / 2) for x0, x1, y, w in slots]
    pcb = K.cut(pcb, K.prism(shapes, "z", -1.0, PCB_T + 1.0, sketch="PCB穴"))
    K.finish(pcb, "PCB", "pcb")

    # ---------------- Ref_parts（推定寸法の仮の箱）
    if MAKE_REF_PARTS:
        K.component("Ref_parts")
        s3x0 = 1.76 - 1.1                       # S3: 22.52 × 18、USB-C 側の端が J1-1 の 1.1mm 手前（推定）
        K.finish(K.prism([rrect(s3x0, S3_USB_Y - 9, s3x0 + 22.52, S3_USB_Y + 9)], "z", Z_MOD, Z_MOD + 1.2), "S3_SuperMini", "module")
        K.finish(K.prism([rrect(s3x0 - 0.5, S3_USB_Y - 4.5, s3x0 + 6.9, S3_USB_Y + 4.5, 1)], "z", Z_MOD + 1.2, Z_MOD + 4.4), "S3_USB_C", "metal")
        K.finish(K.prism([rrect_c(CH_USB_X, 55.5, 11.0, 25.0)], "z", Z_MOD, Z_MOD + 1.0), "CH9102F", "module")
        K.finish(K.prism([rrect(CH_USB_X - 4.5, CAV_TOP_Y - 7.9, CH_USB_X + 4.5, CAV_TOP_Y - 0.5, 1)], "z", Z_MOD + 1.0, Z_MOD + 4.2), "CH9102F_USB_C", "metal")
        K.finish(K.prism([rrect_c(CH_USB_X, 42.73, 26.0, 24.0)], "z", Z_MOD + 4.5, Z_MOD + 5.5), "FFC_adapter", "module")
        for i, (x, y) in enumerate(SW):
            K.finish(K.prism([rrect_c(x, y, 14.0, 14.0)], "z", PCB_T, PCB_T + 5.0), "MX%d_bottom" % (i + 1), "switch")
            K.finish(K.prism([rrect_c(x, y, 15.6, 15.6, 1)], "z", PCB_T + 5.0, PCB_T + 11.6), "MX%d_top" % (i + 1), "switch")
            K.finish(K.prism([rrect_c(x, y, KEYCAP, KEYCAP, 1.5)], "z", PCB_T + 10.0, PCB_T + 18.0), "Keycap%d" % (i + 1), "keycap")
        K.finish(K.prism([circle(TP_CX, TP_CY, TP_FLANGE_D / 2 - 0.3)], "z", Z_RING0, Z_RING0 + TP_FLANGE_T), "TP_flange", "pad")
        K.finish(K.prism([circle(TP_CX, TP_CY, TP_BODY_D / 2 - 0.3)], "z", Z_RING0 + TP_FLANGE_T, Z_PAD), "TP_body", "pad")

    usb_s3 = ("x", OUT[0] - 1.0, CAV[0] + 0.6, [rrect_c(S3_USB_Y, Z_MOD + 1.2 + 1.6, USB_W, USB_H, 3.0)])
    usb_ch = ("y", CAV[3] - 0.6, OUT[3] + 1.0, [rrect_c(CH_USB_X, Z_MOD + 1.0 + 1.6, USB_W, USB_H, 3.0)])

    # ---------------- Bottom
    K.component("Bottom")
    tray = K.prism([rrect(*OUT, r=R_OUT)], "z", Z_FLOOR_BOT, Z_COVER_BOT, sketch="外形")
    tray = K.cut(tray, K.prism([rrect(*CAV, r=R_IN)], "z", Z_FLOOR_TOP, Z_COVER_BOT + 1.0, sketch="内側"))
    pcb_holes = PCB_CLAMP_HOLES + [PCB_PEG_HOLE]
    tray = K.join(tray, K.prism([circle(x, y, BOSS_D / 2) for x, y in pcb_holes], "z", Z_FLOOR_TOP - 0.5, 0.0, sketch="基板のボス"))
    tray = K.join(tray, K.prism([circle(PCB_PEG_HOLE[0], PCB_PEG_HOLE[1], 0.95)], "z", -0.5, PCB_T - 0.2, sketch="位置決めピン"))
    screws = PCB_CLAMP_HOLES + POSTS
    tray = K.cut(tray, K.prism([circle(x, y, SCREW_D / 2) for x, y in screws], "z", Z_FLOOR_BOT - 1.0, 0.5, sketch="ねじ穴"))
    tray = K.cut(tray, K.prism([circle(x, y, HEAD_D / 2) for x, y in screws], "z", Z_FLOOR_BOT - 1.0, Z_FLOOR_BOT + FLOOR_CB_DEPTH, sketch="座ぐり"))
    for axis, a, b, shp in (usb_s3, usb_ch):
        tray = K.cut(tray, K.prism(shp, axis, a, b, sketch="USB-C"))
    K.fillet_outer_loop(tray, Z_FLOOR_BOT, 1.0)
    K.finish(tray, "Bottom", "case")

    # ---------------- Cover
    K.component("Cover")
    cover = K.prism([rrect(*OUT, r=R_OUT)], "z", Z_COVER_BOT, Z_COVER_TOP, sketch="外形")
    c = LIP_CLEAR
    lip = K.prism([rrect(CAV[0] + c, CAV[1] + c, CAV[2] - c, CAV[3] - c, R_IN - c)], "z", Z_COVER_BOT - LIP_H, Z_COVER_BOT + 0.5)
    c2 = LIP_CLEAR + LIP_T
    lip = K.cut(lip, K.prism([rrect(CAV[0] + c2, CAV[1] + c2, CAV[2] - c2, CAV[3] - c2, R_IN - c2)], "z", Z_COVER_BOT - LIP_H - 1.0, Z_COVER_BOT + 1.0))
    cover = K.join(cover, lip)
    bosses = [circle(x, y, BOSS_D / 2) for x, y in PCB_CLAMP_HOLES]
    cover = K.join(cover, K.prism(bosses, "z", PCB_T, Z_COVER_BOT + 0.5, sketch="基板を押さえる柱"))
    cover = K.join(cover, K.prism([circle(x, y, 3.0) for x, y in POSTS], "z", Z_FLOOR_TOP, Z_COVER_BOT + 0.5, sketch="柱"))
    cover = K.join(cover, K.prism([circle(x, y, 3.0) for x, y in ring_screws], "z", Z_COVER_BOT - RING_BOSS_H, Z_COVER_BOT + 0.5, sketch="リングのボス"))
    kx0 = SW[0][0] - KEYCAP / 2 - KEY_CLEAR
    ky1 = SW[0][1] + KEYCAP / 2 + KEY_CLEAR
    cover = K.cut(cover, K.prism([rrect(kx0, CAV[1], CAV[2], ky1, R_IN)], "z", Z_FLOOR_TOP, Z_COVER_TOP + 1.0, sketch="キーの窓"))
    fw, fl, fdx, fdy = FFC_HOLE
    cover = K.cut(cover, K.prism([rrect_c(TP_CX + fdx, TP_CY + fdy, fw, fl, 1.5)], "z", Z_COVER_BOT - 1.0, Z_COVER_TOP + 1.0, sketch="FFCの穴"))
    cover = K.cut(cover, K.prism([circle(x, y, INSERT_D / 2) for x, y in PCB_CLAMP_HOLES], "z", PCB_T - 1.0, PCB_T + INSERT_DEPTH, sketch="インサート穴"))
    cover = K.cut(cover, K.prism([circle(x, y, INSERT_D / 2) for x, y in POSTS], "z", Z_FLOOR_TOP - 1.0, Z_FLOOR_TOP + INSERT_DEPTH))
    cover = K.cut(cover, K.prism([circle(x, y, INSERT_D / 2) for x, y in ring_screws], "z", Z_COVER_TOP - INSERT_DEPTH - 1.0, Z_COVER_TOP + 1.0))
    lip_in = LIP_CLEAR + LIP_T + 0.5   # ふたの縁も USB-C の前だけ切り欠く
    cover = K.cut(cover, K.prism(usb_s3[3], "x", usb_s3[1], CAV[0] + lip_in))
    cover = K.cut(cover, K.prism(usb_ch[3], "y", CAV[3] - lip_in, usb_ch[2]))
    K.fillet_outer_loop(cover, Z_COVER_TOP, 1.0)
    K.finish(cover, "Cover", "case")

    # ---------------- TrackpadRing
    K.component("TrackpadRing")
    sq = rrect_c(TP_CX, TP_CY, RING_SQ, RING_SQ, RING_SQ_R)
    ring = K.prism([sq], "z", Z_RING0, Z_RING0 + RING_BASE_T, sketch="土台")
    z_band = Z_RING_TOP - RING_BAND
    slope = (RING_FOOT_R - RING_TOP_R) / (z_band - (Z_RING0 + RING_BASE_T))
    cone = K.cone(TP_CX, TP_CY, Z_RING0 + RING_BASE_T - 0.4, RING_FOOT_R + 0.4 * slope, z_band, RING_TOP_R)
    cone = K.join(cone, K.prism([circle(TP_CX, TP_CY, RING_TOP_R)], "z", z_band - 0.2, Z_RING_TOP, sketch="外周の帯"))
    cone = K.intersect(cone, K.prism([sq], "z", Z_RING0 - 1.0, Z_RING_TOP + 1.0))
    ring = K.join(ring, cone)
    ring = K.cut(ring, K.prism([circle(TP_CX, TP_CY, TP_BODY_D / 2)], "z", Z_RING0 - 1.0, Z_RING_TOP + 1.0, sketch="窓"))
    ring = K.cut(ring, K.prism([circle(TP_CX, TP_CY, TP_FLANGE_D / 2)], "z", Z_RING0 - 1.0, Z_RING0 + TP_FLANGE_T, sketch="出っ張りの受け"))
    rb = TP_BODY_D / 2
    ch_z0 = Z_RING_TOP - RING_CHAMFER
    ring = K.cut(ring, K.cone(TP_CX, TP_CY, ch_z0, rb, Z_RING_TOP + 0.3, rb + RING_CHAMFER + 0.3))
    ring = K.cut(ring, K.prism([circle(x, y, SCREW_D / 2) for x, y in ring_screws], "z", Z_RING0 - 1.0, Z_RING_TOP + 1.0, sketch="ねじ穴"))
    ring = K.cut(ring, K.prism([circle(x, y, HEAD_D / 2) for x, y in ring_screws], "z", Z_RING0 + RING_CB_FLOOR, Z_RING_TOP + 1.0, sketch="座ぐり"))
    K.fillet_circle(ring, TP_CX, TP_CY, Z_RING_TOP, RING_TOP_R, 0.5)
    K.finish(ring, "TrackpadRing", "ring")


# ============================================================ Fusion
class FusionBackend:
    COLORS = {}

    def __init__(self, design, up):
        self.design = design
        self.root = design.rootComponent
        self.up = up
        self.comp = None
        self.warnings = []
        self.NEW = adsk.fusion.FeatureOperations.NewBodyFeatureOperation

    # この座標系（mm）→ Fusion（cm）
    def P(self, x, y, z):
        if self.up == "Y":
            x, y, z = x, z, -y
        return adsk.core.Point3D.create(x / 10.0, y / 10.0, z / 10.0)

    def mine(self, p):
        x, y, z = p.x * 10.0, p.y * 10.0, p.z * 10.0
        if self.up == "Y":
            return x, -z, y
        return x, y, z

    def component(self, name):
        occs = self.root.occurrences
        for i in range(occs.count - 1, -1, -1):
            if occs.item(i).component.name == name:
                occs.item(i).deleteMe()
        occ = occs.addNewComponent(adsk.core.Matrix3D.create())
        try:
            occ.component.name = name
        except Exception:
            self.warnings.append("コンポーネント名 %s を付けられなかった" % name)
        self.comp = occ.component

    def plane(self, axis, c):
        e = {"x": (1, 0, 0), "y": (0, 1, 0), "z": (0, 0, 1)}[axis]
        n = self.P(*e)
        v = [abs(n.x), abs(n.y), abs(n.z)]
        k = v.index(max(v))
        base = [self.comp.yZConstructionPlane, self.comp.xZConstructionPlane, self.comp.xYConstructionPlane][k]
        p = self.P(*(c * t for t in e))
        off = [p.x, p.y, p.z][k]
        if abs(off) < 1e-9:
            return base
        planes = self.comp.constructionPlanes
        for sign in (1, -1):
            pi = planes.createInput()
            pi.setByOffset(base, adsk.core.ValueInput.createByReal(sign * off))
            pl = planes.add(pi)
            o = pl.geometry.origin
            if abs([o.x, o.y, o.z][k] - off) < 1e-6:
                pl.isLightBulbOn = False
                return pl
            pl.deleteMe()
        raise RuntimeError("作業平面を作れなかった")

    def sketch(self, axis, c, shapes, name=None):
        sk = self.comp.sketches.add(self.plane(axis, c))
        if name:
            sk.name = name
        sk.isComputeDeferred = True

        def S(u, v):
            m = {"z": (u, v, c), "x": (c, u, v), "y": (u, c, v)}[axis]
            return sk.modelToSketchSpace(self.P(*m))

        curves = sk.sketchCurves
        for sh in shapes:
            if sh[0] == "circle":
                curves.sketchCircles.addByCenterRadius(S(sh[1], sh[2]), sh[3] / 10.0)
                continue
            for seg in path_segments(sh):
                if seg[0] == "L":
                    curves.sketchLines.addByTwoPoints(S(*seg[1]), S(*seg[2]))
                else:
                    curves.sketchArcs.addByThreePoints(S(*seg[1]), S(*seg[2]), S(*seg[3]))
        sk.isComputeDeferred = False
        if not name:
            sk.isVisible = False
        return sk

    def _bodies(self, feat):
        return [feat.bodies.item(i) for i in range(feat.bodies.count)]

    def prism(self, shapes, axis, a, b, sketch=None):
        sk = self.sketch(axis, (a + b) / 2.0, shapes, sketch)
        profs = adsk.core.ObjectCollection.create()
        for i in range(sk.profiles.count):
            profs.add(sk.profiles.item(i))
        ext = self.comp.features.extrudeFeatures
        ei = ext.createInput(profs, self.NEW)
        ei.setSymmetricExtent(adsk.core.ValueInput.createByReal(abs(b - a) / 10.0), True)
        return self._bodies(ext.add(ei))

    def cone(self, cx, cy, z0, r0, z1, r1):
        s0 = self.sketch("z", z0, [circle(cx, cy, r0)])
        s1 = self.sketch("z", z1, [circle(cx, cy, r1)])
        lofts = self.comp.features.loftFeatures
        li = lofts.createInput(self.NEW)
        li.loftSections.add(s0.profiles.item(0))
        li.loftSections.add(s1.profiles.item(0))
        li.isSolid = True
        return self._bodies(lofts.add(li))

    def _combine(self, target, tool, op):
        tools = adsk.core.ObjectCollection.create()
        for b in tool:
            tools.add(b)
        cf = self.comp.features.combineFeatures
        ci = cf.createInput(target[0], tools)
        ci.operation = op
        ci.isKeepToolBodies = False
        try:
            res = self._bodies(cf.add(ci))
        except Exception as e:  # 道具の形が本体に届いていないときなど
            self.warnings.append("%s: 結合できなかったので道具の形を消した（%s）" % (self.comp.name, e))
            for b in tool:
                try:
                    self.comp.features.removeFeatures.add(b)
                except Exception:
                    pass
            return target
        return res or target

    def cut(self, t, tool):
        return self._combine(t, tool, adsk.fusion.FeatureOperations.CutFeatureOperation)

    def join(self, t, tool):
        return self._combine(t, tool, adsk.fusion.FeatureOperations.JoinFeatureOperation)

    def intersect(self, t, tool):
        return self._combine(t, tool, adsk.fusion.FeatureOperations.IntersectFeatureOperation)

    def finish(self, body, name, color=None):
        try:
            body[0].name = name
        except Exception:
            pass

    def _fillet(self, edges, radius, what):
        if edges.count == 0:
            self.warnings.append("フィレット（%s）: 対象のエッジが見つからなかった" % what)
            return
        try:
            fl = self.comp.features.filletFeatures
            fi = fl.createInput()
            r = adsk.core.ValueInput.createByReal(radius / 10.0)
            try:
                fi.edgeSetInputs.addConstantRadiusEdgeSet(edges, r, True)
            except AttributeError:
                fi.addConstantRadiusEdgeSet(edges, r, True)
            fl.add(fi)
        except Exception as e:
            self.warnings.append("フィレット（%s）を付けられなかった: %s" % (what, e))

    def fillet_outer_loop(self, body, z, radius):
        """高さ z の平らな面の外周のエッジを丸める（箱の上下の縁）。"""
        edges = adsk.core.ObjectCollection.create()
        for f in body[0].faces:
            if f.geometry.surfaceType != adsk.core.SurfaceTypes.PlaneSurfaceType:
                continue
            if abs(self.mine(f.pointOnFace)[2] - z) > 1e-4:
                continue
            for lp in f.loops:
                if lp.isOuter:
                    for e in lp.edges:
                        edges.add(e)
        self._fillet(edges, radius, "z=%.1f の外周" % z)

    def fillet_circle(self, body, cx, cy, z, r, radius):
        edges = adsk.core.ObjectCollection.create()
        for e in body[0].edges:
            g = e.geometry
            if g.curveType != adsk.core.Curve3DTypes.Circle3DCurveType:
                continue
            g = adsk.core.Circle3D.cast(g)
            x, y, zz = self.mine(g.center)
            if abs(x - cx) < 1e-3 and abs(y - cy) < 1e-3 and abs(zz - z) < 1e-3 and abs(g.radius * 10 - r) < 1e-3:
                edges.add(e)
        self._fillet(edges, radius, "リングの上面")


def detect_up(app):
    try:
        o = app.preferences.generalPreferences.defaultModelingOrientation
        if o == adsk.core.DefaultModelingOrientations.YUpModelingOrientation:
            return "Y"
    except Exception:
        pass
    return "Z"


def run(context):
    ui = None
    try:
        app = adsk.core.Application.get()
        ui = app.userInterface
        design = adsk.fusion.Design.cast(app.activeProduct)
        if not design:
            ui.messageBox("デザイン（hikyaku4pcb）を開いてから実行してください。")
            return
        doc = app.activeDocument.name
        if "hikyaku4pcb" not in doc:
            r = ui.messageBox("開いているのは「%s」です。ここに作りますか？" % doc, "hikyaku4pcb",
                              adsk.core.MessageBoxButtonTypes.YesNoButtonType)
            if r != adsk.core.DialogResults.DialogYes:
                return
        up = UP if UP in ("Y", "Z") else detect_up(app)
        K = FusionBackend(design, up)
        build(K)
        msg = "作成しました（%s 上向き）。\nPCB_ref / Ref_parts / Bottom / Cover / TrackpadRing" % up
        if K.warnings:
            msg += "\n\n注意:\n- " + "\n- ".join(K.warnings)
        ui.messageBox(msg)
    except Exception:
        if ui:
            ui.messageBox("失敗しました:\n" + traceback.format_exc())
