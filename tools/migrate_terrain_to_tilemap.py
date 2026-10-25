import io, re, os

root = os.getcwd()
src = os.path.join(root, "examples/2d_demo/assets/xiuxian/地形/qinglan_valley.terrain.ron")
text = io.open(src, encoding="utf-8").read()


def block(name):
    m = re.search(name + r":\s*\[(.*?)\]", text, re.S)
    return re.findall(r'"([^"]*)"', m.group(1))


surf = block("地表")
over = block("覆盖")
h = len(surf)
w = len(surf[0])
assert all(len(r) == w for r in surf)
assert len(over) == h and all(len(r) == w for r in over)

# 图集源顺序 = 绘制顺序
sources = [
    ("湿润的土", "透明到湿润的土.png", 4),
    ("深色草地", "透明到深色草地.png", 3),
    ("浅草地", "透明到浅草地.png", 2),
    ("干燥土地", "透明到干燥土地.png", 1),
    ("沙滩", "透明到沙滩.png", 0),
    ("山丘", "透明到山丘.png", None),
    ("山地", "透明到山地.png", 5),
    ("海水", "透明到海水.png", None),
    ("道路", "透明到道路.png", None),
    ("耕地", "透明到耕地.png", None),
]
idx = {"土": 0, "林": 1, "草": 2, "旱": 3, "沙": 4, "丘": 5, "山": 6, "水": 7, "路": 8, "田": 9}

base = "examples/2d_demo/assets/xiuxian/地形"
tileset_path = os.path.join(root, base, "青岚山谷.tileset.ron")
tilemap_path = os.path.join(root, base, "青岚山谷.tilemap.ron")

pos = [12, 13, 0, 3, 8, 1, 14, 5, 15, 4, 11, 2, 9, 10, 7, 6]
lines = [
    "// 青岚山谷 —— 通用瓦片集（微风编辑器）。",
    "//",
    "// 由旧的 qinglan_valley.terrain.ron 转换而来：10 张 64×64 四角图集，",
    "// 掩码表与原项目 POS 一致，角位权 [左上, 右上, 右下, 左下] = [8, 4, 1, 2]。",
    "//",
    "// 编辑器不硬编码这些名字：换一套图片与掩码表就是另一套画风。",
    "(",
    '    名称: "青岚山谷",',
    "    瓦片宽: 16,",
    "    瓦片高: 16,",
    "    边距: 0,",
    "    间距: 0,",
    "    双网格: (",
    "        启用: true,",
    "        角位权: (8, 4, 1, 2),",
    "        掩码表: (" + ", ".join(str(v) for v in pos) + "),",
    "        变体: Some((",
    '            图片: "assets/xiuxian/瓦片/瓦片地图变体_丰富地图细节.png",',
    "            列: 7,",
    "            行: 6,",
    "        )),",
    "    ),",
    "    图集源: [",
]
for i, (name, file, vrow) in enumerate(sources):
    lines.extend(
        [
            "        (",
            '            名称: "%s",' % name,
            '            图片: "assets/xiuxian/瓦片/%s",' % file,
            "            列: 4,",
            "            行: 4,",
            "            双网格: true,",
            "            绘制顺序: %d," % i,
            "            变体行: %s," % ("Some(%d)" % vrow if vrow is not None else "None"),
            "            掩码表: None,",
            "        ),",
        ]
    )
lines.extend(["    ],", ")"])
io.open(tileset_path, "w", encoding="utf-8", newline="\n").write("\n".join(lines) + "\n")


def cells(rows, mapping):
    out = []
    for row in rows:
        vals = []
        for ch in row:
            s = mapping.get(ch)
            vals.append(str(((s + 1) << 16) if s is not None else 0))
        out.append("                [" + ", ".join(vals) + "],")
    return out


m = [
    "// 青岚山谷 —— 通用瓦片地图（微风编辑器）。",
    "//",
    "// 每格只记录图集源序号：双网格的四角掩码由渲染器按邻居计算。",
    "// 单元格编码：(图集源序号 + 1) << 16 | 瓦片序号；双网格图层瓦片序号为 0。",
    "(",
    '    瓦片集: "assets/xiuxian/地形/青岚山谷.tileset.ron",',
    "    宽: %d," % w,
    "    高度: %d," % h,
    "    图层: [",
    "        (",
    '            名称: "地表",',
    "            双网格: true,",
    "            单元格: [",
]
m.extend(cells(surf, idx))
m.extend(
    [
        "            ],",
        "        ),",
        "        (",
        '            名称: "覆盖",',
        "            双网格: true,",
        "            单元格: [",
    ]
)
m.extend(cells(over, idx))
m.extend(["            ],", "        ),", "    ],", ")"])
io.open(tilemap_path, "w", encoding="utf-8", newline="\n").write("\n".join(m) + "\n")
print("wrote", tileset_path)
print("wrote", tilemap_path, w, "x", h)
