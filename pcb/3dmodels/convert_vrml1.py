#!/usr/bin/env python3
"""Convert a single-object VRML 1.0 file (Metasequoia) to VRML 2.0, which KiCad 10 can load.
Usage: convert_vrml1.py in.wrl out.wrl
The MatrixTransform is ignored: the raw coordinates are already Z-up (Z = height), which is what KiCad expects.
"""
import re
import sys

src = open(sys.argv[1]).read()


def numbers(name):
    body = re.search(name + r"\s*\[(.*?)\]", src, re.S).group(1)
    return [t for t in re.split(r"[,\s]+", body) if t]


points = numbers("point")  # first 'point' is Coordinate3 (TextureCoordinate2 comes after)
coord_index = numbers("coordIndex")
material_index = numbers("materialIndex")
colors = numbers("diffuseColor")
assert len(points) % 3 == 0 and len(colors) % 3 == 0

pts = ",\n".join(" ".join(points[i:i + 3]) for i in range(0, len(points), 3))
cols = ",\n".join(" ".join(colors[i:i + 3]) for i in range(0, len(colors), 3))
faces = coord_index  # includes -1 terminators
# one material index per face
n_faces = faces.count("-1")
mats = material_index[:n_faces]
assert len(mats) == n_faces

out = f"""#VRML V2.0 utf8
Shape {{
  appearance Appearance {{ material Material {{ }} }}
  geometry IndexedFaceSet {{
    ccw FALSE
    solid FALSE
    colorPerVertex FALSE
    coord Coordinate {{ point [
{pts}
    ] }}
    color Color {{ color [
{cols}
    ] }}
    coordIndex [ {", ".join(faces)} ]
    colorIndex [ {", ".join(mats)} ]
  }}
}}
"""
open(sys.argv[2], "w").write(out)
print(len(points) // 3, "points,", n_faces, "faces")
