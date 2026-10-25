#!/usr/bin/env python3
"""生成 2D 示例项目的**数据与道具**。

# 这个脚本不负责"画出地面"

地面由编辑器里的 `TileTerrain` 组件在加载时程序化合成的：字符网格 →
逻辑格材质 → 双网格四角掩码 → 原项目 `透明到X.png` 图集取瓦片 → 分层合成。
Rust 侧实现在 `crates/editor_app/src/tile_terrain.rs`。

所以这里只做三件事：

1. 把原项目 `assets/art/瓦片/` 下 10 张四角图集与 1 张变体表**原样复制**
   到示例项目（素材是素材，不预先拼图）；
2. 生成地形数据 `.terrain.ron`（宽度/高度/地表/覆盖 两层字符网格）；
3. 生成道具与角色帧 PNG，以及给 Rust 生成器读取的 `layout.json`。

用法（仓库根执行）::

    python tools/build_xiuxian_demo.py \
        --source "C:/Spaces/Share/10_Code/10_Bevy/修仙冒险游戏/assets" \
        --out examples/2d_demo/assets/xiuxian \
        --layout examples/2d_demo/layout.json
"""

from __future__ import annotations

import argparse
import json
import math
import random
import shutil
from pathlib import Path

from PIL import Image, ImageDraw

# ---------------------------------------------------------------------------
# 地图规格
# ---------------------------------------------------------------------------

#: 逻辑格边长（原项目 `mapgen::dualgrid::TILE`）。
CELL = 16
#: 示例项目里每格的显示像素（编辑器按 4 倍最近邻放大烘焙结果）。
SCALE = 4
#: 地图逻辑尺寸（格）。
MAP_W = 48
MAP_H = 32

LIGHT_GRASS = "light_grass"
DARK_GRASS = "dark_grass"
WATER = "water"
SAND = "sand"
MOUNTAIN = "mountain"
HILL = "hill"
WET_SOIL = "wet_soil"

#: 地表字符：Rust `tile_terrain.rs::材质::由地表字符` 必须与这里一一对应。
GROUND_CHAR = {
    LIGHT_GRASS: "草",
    DARK_GRASS: "林",
    WATER: "水",
    SAND: "沙",
    MOUNTAIN: "山",
    HILL: "丘",
    WET_SOIL: "土",
}

#: 原项目图集文件（复制到示例项目，文件名保持不变）。
ATLAS_FILES = [
    "透明到湿润的土.png",
    "透明到深色草地.png",
    "透明到浅草地.png",
    "透明到干燥土地.png",
    "透明到沙滩.png",
    "透明到山丘.png",
    "透明到山地.png",
    "透明到海水.png",
    "透明到道路.png",
    "透明到耕地.png",
    "瓦片地图变体_丰富地图细节.png",
]


# ---------------------------------------------------------------------------
# 地形生成
# ---------------------------------------------------------------------------


def wobble(x: float, y: float, seed: float) -> float:
    """确定性的平滑扰动，用来把几何边界揉成自然曲线。"""
    return (
        0.55 * math.sin(x * 0.53 + y * 0.21 + seed)
        + 0.30 * math.sin(x * 0.19 - y * 0.47 + seed * 1.7)
        + 0.15 * math.sin(x * 0.91 + y * 0.83 + seed * 2.3)
    )


def in_ellipse(x, y, cx, cy, rx, ry, seed, edge=0.0) -> bool:
    d = ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2
    return d < 1.0 + edge + 0.16 * wobble(x, y, seed)


def build_terrain() -> tuple[list[list[str]], set[tuple[int, int]], set[tuple[int, int]]]:
    """返回 `(地表, 道路格, 耕地格)`，坐标 (col, row)，row 0 = 北。"""
    terrain = [[LIGHT_GRASS for _ in range(MAP_W)] for _ in range(MAP_H)]
    roads: set[tuple[int, int]] = set()
    farms: set[tuple[int, int]] = set()

    # ---- 西侧与西南的林地 ----
    for row in range(MAP_H):
        for col in range(MAP_W):
            if (
                in_ellipse(col, row, 6.0, 19.0, 9.5, 13.0, 1.3)
                or in_ellipse(col, row, 15.0, 28.0, 10.0, 6.5, 2.7)
                or in_ellipse(col, row, 4.0, 5.0, 5.0, 6.0, 4.1)
            ):
                terrain[row][col] = DARK_GRASS

    # ---- 湿润的土：山脚裸土与林间空地 ----
    for cx, cy, rx, ry, seed in [
        (40.0, 5.6, 4.6, 2.4, 11.3),
        (20.0, 4.6, 4.2, 2.0, 12.9),
        (3.0, 17.0, 2.8, 2.2, 14.1),
        (18.0, 21.0, 2.2, 1.6, 15.7),
    ]:
        for row in range(MAP_H):
            for col in range(MAP_W):
                if terrain[row][col] in (LIGHT_GRASS, DARK_GRASS) and in_ellipse(
                    col, row, cx, cy, rx, ry, seed
                ):
                    terrain[row][col] = WET_SOIL

    # ---- 北侧山脉与东侧山脊：山 → 山丘 → 草地 ----
    # 山丘台地做厚到 3 格左右：原项目的岩壁层依赖连续山丘掩码，
    # 太薄的台地会让岩壁碎成一排点。
    for col in range(MAP_W):
        edge = 3.0 + 2.5 * (0.5 + 0.5 * math.sin(col * 0.31 + 0.8)) + 0.8 * wobble(col, 0, 3.1)
        for row in range(MAP_H):
            if row < edge:
                terrain[row][col] = MOUNTAIN
            elif row < edge + 3.4 + 0.5 * wobble(col, row, 5.3):
                terrain[row][col] = HILL
    for row in range(MAP_H):
        edge = 2.6 + 2.2 * (0.5 + 0.5 * math.sin(row * 0.29 + 2.2)) + 0.7 * wobble(0, row, 6.7)
        for col in range(MAP_W):
            if MAP_W - 1 - col < edge:
                terrain[row][col] = MOUNTAIN
            elif MAP_W - 1 - col < edge + 3.0 + 0.5 * wobble(col, row, 8.9):
                if terrain[row][col] not in (MOUNTAIN, HILL):
                    terrain[row][col] = HILL

    # ---- 碧水潭：水 → 沙滩环 ----
    pond_cx, pond_cy = 12.5, 24.5
    for row in range(MAP_H):
        for col in range(MAP_W):
            d = ((col - pond_cx) / 6.2) ** 2 + ((row - pond_cy) / 3.6) ** 2
            d += 0.14 * wobble(col, row, 9.7)
            if d < 1.0:
                terrain[row][col] = WATER
            elif d < 1.42 and terrain[row][col] != MOUNTAIN:
                terrain[row][col] = SAND

    # ---- 南流的小溪 ----
    for row in range(26, MAP_H):
        cx = 14.0 + 1.6 * math.sin((row - 26) * 0.7)
        for col in range(MAP_W):
            if abs(col - cx) < 1.15:
                terrain[row][col] = WATER
            elif abs(col - cx) < 2.3 and terrain[row][col] != WATER:
                terrain[row][col] = SAND

    # ---- 道路：南北主道 + 东西横道，宽 1 格 ----
    def lay_road(col: float, row: int) -> None:
        c = int(round(col))
        if 0 <= c < MAP_W and 0 <= row < MAP_H and terrain[row][c] != WATER:
            roads.add((c, row))

    for row in range(MAP_H):
        base = 25.0 + 1.4 * math.sin((row - 5) * 0.34)
        if row < 6:
            base = 25.0
        lay_road(base, row)
    for col in range(1, MAP_W - 1):
        row = 14.0 + 0.7 * math.sin(col * 0.25)
        lay_road(col, int(round(row)))

    # ---- 耕地 ----
    for x0, y0, w, h in [(30, 7, 4, 3), (31, 16, 4, 3), (40, 11, 4, 5), (30, 22, 4, 3)]:
        for row in range(y0, min(y0 + h, MAP_H)):
            for col in range(x0, min(x0 + w, MAP_W)):
                if terrain[row][col] in (LIGHT_GRASS, DARK_GRASS) and (col, row) not in roads:
                    farms.add((col, row))

    return terrain, roads, farms


def write_terrain_ron(path: Path) -> None:
    """把字符网格写成 `.terrain.ron`。"""
    terrain, roads, farms = build_terrain()
    地表 = []
    覆盖 = []
    for row in range(MAP_H):
        地表行 = "".join(GROUND_CHAR[terrain[row][col]] for col in range(MAP_W))
        覆盖行 = "".join(
            "路" if (col, row) in roads else ("田" if (col, row) in farms else ".")
            for col in range(MAP_W)
        )
        地表.append(地表行)
        覆盖.append(覆盖行)

    def 行列表(rows: list[str]) -> str:
        return ",\n".join(f'        "{row}"' for row in rows)

    text = f"""// 青岚山谷 —— 双网格瓦片地形数据。
//
// 这里**没有**任何"预拼好的地面图"。编辑器加载时按原项目规格现算：
//   逻辑格 ──► 每张视觉瓦片的四角掩码 ──► 图集取 16px 瓦片 ──► 分层合成
// 渲染实现在 `crates/editor_app/src/tile_terrain.rs`。
//
// 地表字符：草=浅草  林=深草  水=海水  沙=沙滩  山=山地  丘=山丘  土=湿润的土
// 覆盖字符：. = 无     路 = 道路   田 = 耕地
//
// 说明：地图尺寸 48×32 格，与 `assets/xiuxian/瓦片/` 这套 16px 图集配套。
(
    宽度: {MAP_W},
    高度: {MAP_H},
    地表: [
{行列表(地表)},
    ],
    覆盖: [
{行列表(覆盖)},
    ],
)
"""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


# ---------------------------------------------------------------------------
# 道具像素画（1 格 = 16 逻辑像素，输出 ×4）
# ---------------------------------------------------------------------------

PALETTE = {
    "ink": (24, 22, 20, 255),
    "shadow": (0, 0, 0, 70),
    "wood": (108, 74, 48, 255),
    "wood_dark": (74, 48, 30, 255),
    "thatch": (168, 132, 74, 255),
    "thatch_dark": (124, 92, 50, 255),
    "wall": (122, 96, 68, 255),
    "window": (246, 208, 116, 255),
    "leaf_dark": (30, 74, 34, 255),
    "leaf": (48, 104, 44, 255),
    "leaf_hi": (74, 136, 58, 255),
    "pine_dark": (26, 66, 42, 255),
    "pine": (42, 92, 54, 255),
    "stone": (118, 120, 118, 255),
    "stone_dark": (84, 86, 86, 255),
    "stone_hi": (150, 152, 146, 255),
}


def new_canvas(w: int, h: int) -> Image.Image:
    return Image.new("RGBA", (w, h), (0, 0, 0, 0))


def draw_shadow(draw: ImageDraw.ImageDraw, cx: int, cy: int, w: int, h: int) -> None:
    draw.ellipse((cx - w // 2, cy - h // 2, cx + w // 2, cy + h // 2), fill=PALETTE["shadow"])


def draw_hut() -> Image.Image:
    w = h = 24
    image = new_canvas(w, h)
    d = ImageDraw.Draw(image)
    draw_shadow(d, 12, 22, 19, 4)
    d.rectangle((3, 12, 20, 21), fill=PALETTE["wood"], outline=PALETTE["ink"])
    for y in (15, 18):
        d.line((4, y, 19, y), fill=PALETTE["wood_dark"])
    d.polygon([(1, 12), (12, 4), (22, 12), (22, 13), (1, 13)], fill=PALETTE["thatch"], outline=PALETTE["ink"])
    d.line((12, 4, 12, 12), fill=PALETTE["thatch_dark"])
    for x in range(3, 21, 4):
        d.line((x, 12, x + 2, 7), fill=PALETTE["thatch_dark"])
    d.rectangle((10, 16, 14, 21), fill=(40, 28, 20, 255), outline=PALETTE["ink"])
    d.rectangle((5, 15, 8, 18), fill=PALETTE["window"], outline=PALETTE["ink"])
    d.rectangle((17, 3, 19, 7), fill=PALETTE["stone"], outline=PALETTE["ink"])
    return image


def draw_granary() -> Image.Image:
    w, h = 28, 28
    image = new_canvas(w, h)
    d = ImageDraw.Draw(image)
    draw_shadow(d, 14, 26, 23, 4)
    d.rectangle((4, 13, 23, 25), fill=PALETTE["wood"], outline=PALETTE["ink"])
    for y in (16, 20, 23):
        d.line((5, y, 22, y), fill=PALETTE["wood_dark"])
    d.polygon([(1, 13), (14, 3), (26, 13), (26, 14), (1, 14)], fill=PALETTE["thatch"], outline=PALETTE["ink"])
    for x in range(3, 25, 4):
        d.line((x, 13, x + 3, 6), fill=PALETTE["thatch_dark"])
    d.line((14, 3, 14, 13), fill=PALETTE["thatch_dark"])
    d.rectangle((10, 18, 16, 25), fill=(40, 28, 20, 255), outline=PALETTE["ink"])
    d.rectangle((6, 17, 8, 19), fill=PALETTE["window"], outline=PALETTE["ink"])
    d.rectangle((19, 17, 21, 19), fill=PALETTE["window"], outline=PALETTE["ink"])
    return image


def draw_well() -> Image.Image:
    w, h = 16, 18
    image = new_canvas(w, h)
    d = ImageDraw.Draw(image)
    draw_shadow(d, 8, 16, 13, 3)
    d.ellipse((2, 10, 13, 16), fill=PALETTE["stone"], outline=PALETTE["ink"])
    d.ellipse((4, 11, 11, 15), fill=(34, 36, 40, 255))
    d.rectangle((3, 4, 4, 12), fill=PALETTE["wood_dark"])
    d.rectangle((11, 4, 12, 12), fill=PALETTE["wood_dark"])
    d.polygon([(1, 5), (8, 1), (14, 5), (14, 6), (1, 6)], fill=PALETTE["thatch"], outline=PALETTE["ink"])
    d.line((8, 6, 8, 10), fill=PALETTE["wood"])
    return image


def draw_tree() -> Image.Image:
    w, h = 24, 32
    rng = random.Random(3)
    image = new_canvas(w, h)
    d = ImageDraw.Draw(image)
    draw_shadow(d, 12, 29, 14, 4)
    d.rectangle((10, 20, 13, 28), fill=PALETTE["wood_dark"], outline=PALETTE["ink"])
    blobs = [(12, 18, 11, 8), (10, 12, 8, 6), (13, 7, 6, 5)]
    for cx, cy, rx, ry in blobs:
        d.ellipse((cx - rx, cy - ry, cx + rx, cy + ry), fill=PALETTE["leaf"], outline=PALETTE["ink"])
    for cx, cy, rx, ry in blobs:
        d.ellipse((cx - rx + 2, cy - ry + 3, cx + rx - 1, cy + ry), fill=PALETTE["leaf_dark"])
    for _ in range(18):
        x = rng.randint(3, 20)
        y = rng.randint(2, 18)
        if image.getpixel((x, y))[3] > 0:
            d.point((x, y), fill=PALETTE["leaf_hi"])
    return image


def draw_pine() -> Image.Image:
    w, h = 20, 34
    image = new_canvas(w, h)
    d = ImageDraw.Draw(image)
    draw_shadow(d, 10, 31, 12, 4)
    d.rectangle((8, 24, 11, 30), fill=PALETTE["wood_dark"], outline=PALETTE["ink"])
    for top, bottom, half in [(3, 14, 7), (8, 21, 9), (14, 27, 10)]:
        d.polygon(
            [(10, top), (10 + half, bottom), (10 - half, bottom)],
            fill=PALETTE["pine"],
            outline=PALETTE["ink"],
        )
        d.polygon(
            [(10, top + 2), (10 + half - 2, bottom - 1), (10 - half + 2, bottom - 1)],
            fill=PALETTE["pine_dark"],
        )
        for x in range(10 - half + 2, 10 + half - 1, 3):
            d.line((x, bottom - 2, x + 1, bottom - 4), fill=PALETTE["leaf_hi"])
    return image


def draw_rock() -> Image.Image:
    w, h = 18, 14
    image = new_canvas(w, h)
    d = ImageDraw.Draw(image)
    draw_shadow(d, 9, 12, 14, 3)
    d.polygon([(2, 11), (4, 5), (9, 2), (14, 4), (16, 10), (12, 12), (6, 12)], fill=PALETTE["stone"], outline=PALETTE["ink"])
    d.polygon([(4, 6), (9, 4), (13, 5), (14, 8), (8, 8)], fill=PALETTE["stone_hi"])
    d.polygon([(6, 12), (12, 12), (14, 10), (8, 9)], fill=PALETTE["stone_dark"])
    return image


def draw_signpost() -> Image.Image:
    w, h = 14, 20
    image = new_canvas(w, h)
    d = ImageDraw.Draw(image)
    draw_shadow(d, 7, 18, 8, 3)
    d.rectangle((6, 7, 8, 18), fill=PALETTE["wood_dark"], outline=PALETTE["ink"])
    d.rectangle((2, 3, 12, 9), fill=PALETTE["wood"], outline=PALETTE["ink"])
    d.rectangle((3, 4, 11, 5), fill=PALETTE["thatch"])
    for y in (8, 11, 14):
        d.line((6, y, 8, y), fill=PALETTE["wood_dark"])
    d.line((9, 3, 11, 1), fill=PALETTE["ink"])
    return image


# ---------------------------------------------------------------------------
# 角色帧
# ---------------------------------------------------------------------------

#: 名称 → 原项目 `protagonist.png` 的帧号（见 `protagonist.anim.ron`）。
CHARACTER_FRAMES = {
    "主角": 0,  # idle 第 1 帧
    "师姐": 4,  # walk 第 3 帧
    "采药人": 20,  # cast 第 3 帧
    "村民": 10,  # run 第 3 帧
}


def slice_character(source: Path, frame: int, size: int = 160) -> Image.Image:
    atlas = Image.open(source).convert("RGBA")
    if atlas.size != (2112, 64):
        raise SystemExit(f"{source} 尺寸 {atlas.size}，期望 2112×64")
    cell = atlas.crop((frame * 64, 0, frame * 64 + 64, 64))
    canvas = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    d = ImageDraw.Draw(canvas)
    d.ellipse((20, 56, 44, 62), fill=(0, 0, 0, 60))
    canvas.alpha_composite(cell)
    return canvas.resize((size, size), Image.NEAREST)


# ---------------------------------------------------------------------------
# 实体摆放
# ---------------------------------------------------------------------------


def bottom_of(row: float) -> float:
    """某一行格子的底边世界 y（道具/角色的脚底线）。"""
    return MAP_H * CELL * SCALE / 2 - (row + 1) * CELL * SCALE


def depth_of(y: float) -> float:
    """按世界 y 算 z：越靠下越靠前。"""
    return 10.0 + (MAP_H * CELL * SCALE / 2 - y) * 0.02


def build_layout(asset_dir: Path) -> dict:
    """生成实体清单（不含地面；地面由 `TileTerrain` 组件负责）。"""
    objects: list[dict] = []

    def add(
        name: str,
        group: list[str],
        image_name: str,
        col: float,
        row: float,
        *,
        size: tuple[int, int] | None = None,
        flip_x: bool = False,
        tint: tuple[float, float, float] | None = None,
    ) -> None:
        if size is None:
            size = Image.open(asset_dir / image_name).size
        world_x = (col + 0.5) * CELL * SCALE - MAP_W * CELL * SCALE / 2
        base_y = bottom_of(row)
        item = {
            "name": name,
            "group": group,
            "x": round(world_x, 2),
            "y": round(base_y + size[1] / 2, 2),
            "w": size[0],
            "h": size[1],
            "z": round(depth_of(base_y) + (0.5 if group == ["人物"] else 0.0), 3),
            "asset": f"assets/xiuxian/{image_name}",
            "flip_x": flip_x,
        }
        if tint is not None:
            item["tint"] = [round(c, 3) for c in tint]
        objects.append(item)

    # ---- 村落：四栋房围着十字路口 ----
    add("青溪村·东屋", ["村落", "房屋"], "道具/茅屋.png", 28, 9, size=(96, 96))
    add("青溪村·西屋", ["村落", "房屋"], "道具/茅屋.png", 20, 11, size=(96, 96), flip_x=True)
    add("青溪村·南屋", ["村落", "房屋"], "道具/茅屋.png", 22.5, 18, size=(96, 96))
    add("青溪村·谷仓", ["村落", "房屋"], "道具/谷仓.png", 28, 18, size=(112, 112), flip_x=True)
    add("青溪村·水井", ["村落", "点景"], "道具/水井.png", 23.5, 12.5, size=(64, 72))
    add("青溪村·木牌", ["村落", "点景"], "道具/木牌.png", 26.5, 16.5, size=(56, 80))

    # ---- 植被 ----
    tree_spots = [
        (3, 12, False),
        (5, 16, True),
        (2, 21, False),
        (5, 23, True),
        (6, 29, False),
        (4, 30, True),
        (13, 19, False),
        (20, 28, True),
        (17, 30, False),
        (8, 7, True),
        (33, 6, False),
        (43, 20, True),
    ]
    for index, (col, row, flip) in enumerate(tree_spots, start=1):
        add(f"古樟 {index:02}", ["植被", "乔木"], "道具/阔叶树.png", col, row, size=(96, 128), flip_x=flip)
    pine_spots = [(1, 5, False), (11, 4, True), (42, 3, False), (46, 27, True)]
    for index, (col, row, flip) in enumerate(pine_spots, start=1):
        add(f"松树 {index:02}", ["植被", "乔木"], "道具/松树.png", col, row, size=(80, 136), flip_x=flip)
    bush_spots = [(8, 20, False), (19, 22, True), (31, 7, False), (38, 24, True)]
    for index, (col, row, flip) in enumerate(bush_spots, start=1):
        add(f"灌木 {index:02}", ["植被", "灌木"], "道具/灌木.png", col, row, size=(80, 60), flip_x=flip)

    # ---- 山石 ----
    rock_spots = [(6, 4, False), (16, 3, True), (36, 4, False), (43, 15, True), (45, 29, False)]
    for index, (col, row, flip) in enumerate(rock_spots, start=1):
        add(f"山石 {index:02}", ["山石"], "道具/岩石.png", col, row, size=(72, 56), flip_x=flip)

    # ---- 人物 ----
    for name, image_name, col, row, flip, tint in [
        ("主角", "人物/主角.png", 26.5, 16.5, False, None),
        ("同门师姐", "人物/师姐.png", 28.5, 15.0, True, (0.82, 1.0, 0.86)),
        ("采药人", "人物/采药人.png", 18.5, 12.5, False, (1.0, 0.92, 0.78)),
        ("村民", "人物/村民.png", 23.5, 20.5, True, (0.84, 0.90, 1.0)),
    ]:
        add(
            name,
            ["人物"],
            image_name,
            col,
            row,
            size=(160, 160),
            flip_x=flip,
            tint=tint,
        )

    return {
        "schema": 2,
        "map": {
            "cell_px": CELL * SCALE,
            "cols": MAP_W,
            "rows": MAP_H,
        },
        "terrain": {
            "name": "青岚山谷地形",
            "file": "assets/xiuxian/地形/qinglan_valley.terrain.ron",
            "atlas": "assets/xiuxian/瓦片",
            "display_size": [MAP_W * CELL * SCALE, MAP_H * CELL * SCALE],
            "cell_pixels": CELL * SCALE,
            "variant_chance": 0.35,
            "x": 0.0,
            "y": 0.0,
            "z": -20.0,
        },
        "objects": objects,
    }


# ---------------------------------------------------------------------------
# 入口
# ---------------------------------------------------------------------------


def save(image: Image.Image, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    image.save(path, optimize=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, help="修仙冒险游戏项目的 assets 目录")
    parser.add_argument("--out", required=True, help="示例项目的 assets/xiuxian 目录")
    parser.add_argument("--layout", required=True, help="输出 layout.json 的路径")
    args = parser.parse_args()

    source = Path(args.source)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    # 1) 图集原样复制。这里**不拼接、不缩放**，渲染时才按双网格取瓦片。
    tile_dir = out_dir / "瓦片"
    tile_dir.mkdir(parents=True, exist_ok=True)
    for 文件名 in ATLAS_FILES:
        源 = source / "art" / "瓦片" / 文件名
        if not 源.is_file():
            raise SystemExit(f"缺少图集：{源}")
        shutil.copy2(源, tile_dir / 文件名)

    # 2) 地形数据（字符网格）。
    write_terrain_ron(out_dir / "地形" / "qinglan_valley.terrain.ron")

    # 3) 道具与角色帧。
    props = {
        "茅屋.png": draw_hut(),
        "谷仓.png": draw_granary(),
        "水井.png": draw_well(),
        "阔叶树.png": draw_tree(),
        "松树.png": draw_pine(),
        "岩石.png": draw_rock(),
        "木牌.png": draw_signpost(),
    }
    for 文件名, image in props.items():
        save(image.resize((image.width * SCALE, image.height * SCALE), Image.NEAREST), out_dir / "道具" / 文件名)

    tree = props["阔叶树.png"]
    bush = tree.crop((2, 6, 22, 21))
    save(bush.resize((bush.width * SCALE, bush.height * SCALE), Image.NEAREST), out_dir / "道具" / "灌木.png")

    for 名称, frame in CHARACTER_FRAMES.items():
        save(slice_character(source / "sprites" / "protagonist.png", frame), out_dir / "人物" / f"{名称}.png")

    # 4) 实体清单（不含地面）。
    layout = build_layout(out_dir)
    layout_path = Path(args.layout)
    layout_path.parent.mkdir(parents=True, exist_ok=True)
    layout_path.write_text(json.dumps(layout, ensure_ascii=False, indent=2), encoding="utf-8")

    print(f"图集：{len(ATLAS_FILES)} 张 → {tile_dir}")
    print(f"地形：{MAP_W}×{MAP_H} 格 → {out_dir / '地形' / 'qinglan_valley.terrain.ron'}")
    print(f"实体：{len(layout['objects'])} 个道具/角色 → {layout_path}")


if __name__ == "__main__":
    main()
