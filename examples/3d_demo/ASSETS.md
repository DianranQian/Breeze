# 素材清单（3D 示例）

| 文件 | 用途 | 场景中的实体 |
| --- | --- | --- |
| `assets/3d_demo/tile_floor_a.png` | 深色地砖 | 地砖 01/03/05/07/09（棋格） |
| `assets/3d_demo/tile_floor_b.png` | 浅色地砖 | 地砖 02/04/06/08 |
| `assets/3d_demo/prop_altar.png` | 中央祭坛 | 中心祭坛 |
| `assets/3d_demo/prop_tree.png` | 古树（红） | 左侧古树 |
| `assets/3d_demo/prop_tree_2.png` | 古树（金） | 右侧古树 |
| `assets/3d_demo/prop_statue.png` | 石碑 | 后方石碑 |
| `assets/3d_demo/unit_player.png` | 主角 | 主角 |
| `assets/3d_demo/unit_goblin.png` | 哥布林 | 哥布林 |
| `assets/3d_demo/unit_orc.png` | 兽人 | 兽人 |
| `assets/3d_demo/pickup_gold.png` | 悬浮金币 | 悬浮金币 |
| `assets/3d_demo/pickup_potion.png` | 悬浮药水 | 悬浮药水 |

## 来源

- 素材包：**Dungeon Crawl Stone Soup — 32×32 tiles**（`crawl-tiles Oct-5-2010.zip`）
- 页面：<https://opengameart.org/content/dungeon-crawl-32x32-tiles>
- 许可证：**CC0 1.0**（允许商用、修改、再分发；无需署名）
- 许可证原文：`assets/DCSS_TILESET_LICENSE.txt`

## 技术说明（当前 3D 渲染能力）

- 每个实体在 3D 模式下渲染为一个**带贴图的四边形**（`Mesh3d` + unlit 材质，
  双面可见），不使用 glTF/网格模型，也没有材质与灯光编辑；
- 地砖通过**旋转四元数** `(-√0.5, 0, 0, √0.5)`（绕 X 轴 −90°）平铺到 y=0 平面；
- 角色与道具是竖直四边形，靠位置（x,y,z）与缩放形成空间层次；
- 精灵图本身是 2D 像素画，放到 3D 空间是**有意的能力展示**，
  不代表已支持三维模型资产。

## 未采用

- glTF/GLB 模型、材质、环境贴图：当前管线不渲染，纳入会造成误导；
- 原包的图集：无切分能力，未使用。
