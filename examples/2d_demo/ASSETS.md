# 素材清单（2D · 青岚山谷）

来源项目：`C:\Spaces\Share\10_Code\10_Bevy\修仙冒险游戏`（本地 Bevy 项目，只读引用）。

## 地形（单格 PNG，64×64，原样复制）

| 文件 | 原始路径 | 许可 | 场景用途 |
| --- | --- | --- | --- |
| `assets/xiuxian/透明到浅草地.png` | `assets/tilesets/透明到浅草地.png` | 见下方说明 | 灵草地 01~15（非山道列） |
| `assets/xiuxian/透明到深色草地.png` | `assets/tilesets/透明到深色草地.png` | 同上 | 第一行灵草地 |
| `assets/xiuxian/透明到道路.png` | `assets/tilesets/透明到道路.png` | 同上 | 山道 03/08/13 |
| `assets/xiuxian/透明到山地.png` | `assets/tilesets/透明到山地.png` | 同上 | 北崖 |
| `assets/xiuxian/透明到悬崖.png` | `assets/art/瓦片/透明到悬崖.png` | 同上 | 东崖 |
| `assets/xiuxian/透明到海水.png` | `assets/tilesets/透明到海水.png` | 同上 | 碧水潭 |
| `assets/xiuxian/透明到沙滩.png` | `assets/tilesets/透明到沙滩.png` | 同上 | 白沙岸 |
| `assets/xiuxian/透明到耕地.png` | `assets/tilesets/透明到耕地.png` | 同上 | 灵田 |
| `assets/xiuxian/瓦片地图变体_丰富地图细节.png` | `assets/tilesets/瓦片地图变体_丰富地图细节.png` | 同上 | 未使用（候选） |

> 源项目的 `README.md` 未附第三方素材授权声明，`assets/` 内也没有独立 LICENSE 文件；
> 这些地形与角色图均为该项目的自有素材（用户本地项目）。复制进示例仅用于本机演示与
> 内部验收，**再分发前请先向项目作者确认**。此处如实记录，不代填许可证。

## 主角（图集切片）

| 文件 | 来源 | 处理 | 用途 |
| --- | --- | --- | --- |
| `assets/xiuxian/protagonist_idle.png` | `assets/sprites/protagonist.png` 第 0 帧 | 用 Pillow 按 64×64 裁切 | 主角 |
| `assets/xiuxian/protagonist_walk.png` | 同图集第 7 帧 | 同上 | 同门师姐 |

原图尺寸 **2112×64**，即 33 帧 × 64px。切图脚本为一次性操作，源项目文件未改动；
如需重新生成，可按 `x ∈ [帧号×64, 帧号×64+64)`、`y ∈ [0, 64)` 裁切。

## 实际显示情况

| 资产 | 是否显示 | 证据 |
| --- | --- | --- |
| 青草地 / 山道 / 崖壁 / 水潭 / 灵田 | 是 | `docs/验收证据/16_示例_2D_青岚山谷.png` |
| 主角（idle 帧） | 是 | 同上 |
| 同门师姐（walk 帧） | 是 | 同上 |

## 未复制 / 未使用

- 水面 caustic 叠加层、`_模板导出` 中间产物：属于渲染特效与制作文件，示例不需要；
- `瓦片地图变体_丰富地图细节.png`：已复制但未放入场景（保留为后续瓦片编辑的候选素材）；
- 音频（`.ogg`）：示例不需要。
