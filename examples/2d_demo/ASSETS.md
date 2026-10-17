# 素材清单（2D 示例）

| 文件 | 用途 | 场景中的实体 |
| --- | --- | --- |
| `assets/2d_demo/floor_grass.png` | 草地地砖 | 草地 01~15（非路列） |
| `assets/2d_demo/floor_dirt.png` | 土路地砖 | 土路 03/08/13 |
| `assets/2d_demo/player_human_m.png` | 主角 | 玩家 |
| `assets/2d_demo/monster_goblin.png` | 哥布林 | 哥布林斥候 |
| `assets/2d_demo/monster_orc.png` | 兽人 | 兽人战士 |
| `assets/2d_demo/monster_kobold.png` | 狗头人 | 狗头人 |
| `assets/2d_demo/decor_tree_red.png` | 红枫 | 红枫、远树 |
| `assets/2d_demo/decor_tree_yellow.png` | 金枫 | 金枫 |
| `assets/2d_demo/decor_statue.png` | 石碑 | 石碑 |
| `assets/2d_demo/decor_altar.png` | 祭坛 | 祭坛 |
| `assets/2d_demo/item_gold.png` | 金币堆 | 金币堆 |
| `assets/2d_demo/item_potion_blue.png` | 蓝色药水 | 蓝色药水 |
| `assets/2d_demo/item_potion_black.png` | 黑色药水 | 黑色药水 |

## 来源

- 素材包：**Dungeon Crawl Stone Soup — 32×32 tiles**（`crawl-tiles Oct-5-2010.zip`）
- 页面：<https://opengameart.org/content/dungeon-crawl-32x32-tiles>
- 作者/维护者：Chris Hamons（maintainer）、MedicineStorm（提交者）；
  页面明确写明所有参与者以 **CC0** 发布
- 许可证：**CC0 1.0（公共领域）**，允许商业使用、修改、再分发，无需署名
- 许可证原文：`assets/DCSS_TILESET_LICENSE.txt`

## 处理说明

- 只从原包中取出 13 个实际使用的 PNG（其余未进仓库）；
- 未做格式转换：原文件即 32×32 透明 PNG，Bevy Studio 以"自定义尺寸"缩放显示；
- 文件名改为语义化命名，原路径记录在上表来源中，便于追溯；
- 场景中的 `贴图` 字段保存的是**项目相对路径**，与生成机器无关。

## 未采用

- 原包里的图集/大尺寸背景：当前编辑器按单文件渲染，没有图集切分能力，
  采用会造成"看起来有素材、实际显示整张图"的误导，因此未使用。
