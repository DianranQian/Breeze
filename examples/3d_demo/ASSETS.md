# 素材清单（3D · TPS 竞技场）

来源项目：`C:\Spaces\Share\10_Code\10_Bevy\tps-demo\tps_arena`（本地 Bevy 项目，只读引用）。
源项目授权说明见同目录 [`ATTRIBUTIONS.md`](./ATTRIBUTIONS.md)。

## 模型（glTF，CC0）

全部为**自包含 glTF**：顶点数据在 bufferView 中、材质贴图内嵌，无外部 `.bin`/纹理依赖。
按原样复制，未转换格式。

### 角色

| 文件 | 原始路径 | 场景用途 |
| --- | --- | --- |
| `characters/Character_Soldier.gltf` | `assets/characters/Character_Soldier.gltf` | 玩家士兵 |
| `characters/Character_Enemy.gltf` | `assets/characters/Character_Enemy.gltf` | 近战敌人 |
| `characters/Character_Hazmat.gltf` | `assets/characters/Character_Hazmat.gltf` | 远程敌人、重装敌人 |

### 掩体（23 件）

| 文件 | 原始路径 | 场景数量 | 用途 |
| --- | --- | --- | --- |
| `environment/Container_Long.gltf` | `assets/environment/Container_Long.gltf` | 3 | 长货柜：西北 / 东南 / 中央 |
| `environment/Container_Small.gltf` | `assets/environment/Container_Small.gltf` | 2 | 短货柜：东北 / 西南 |
| `environment/SackTrench.gltf` | `assets/environment/SackTrench.gltf` | 2 | 东西沙袋墙 |
| `environment/Crate.gltf` | `assets/environment/Crate.gltf` | 6 | 木箱 |
| `environment/ExplodingBarrel.gltf` | `assets/environment/ExplodingBarrel.gltf` | 4 | 油桶 |
| `environment/Barrier_Single.gltf` | `assets/environment/Barrier_Single.gltf` | 2 | 南北铁马 |
| `environment/TrafficCone.gltf` | `assets/environment/TrafficCone.gltf` | 4 | 交通锥 |

位置与是否绕 Y 轴 90° 逐条对应原项目 `src/plugins/level.rs::掩体布局`。
原表里的视觉包围盒/碰撞盒两列是给碰撞与子弹用的；示例只渲染视觉，不复制尺寸列。

## 环境贴图（PNG）

| 文件 | 原始路径 | 用途 |
| --- | --- | --- |
| `environment/floor.png` | `assets/environment/floor.png` | 地面 `Primitive3d` 基色，UV 6×6 |
| `environment/wall.png` | `assets/environment/wall.png` | 四面围墙 `Primitive3d` 基色，UV 6×1 |

**没有烘焙好的 48m 地面/墙面图。** 地面与围墙是 48×48×0.6 / 48×8×0.6 的
长方体几何体，256px 原始纹理通过 `uv_transform` 重复，受光并接收阴影。

## 未复制的部分

- 音效（`.ogg`）：示例不需要音频；
- HUD / UI 贴图、武器模型（AK/Shotgun）：示例不运行玩法；
- `RobotExpressive.glb`：源项目已复制但本示例未使用，未纳入仓库。

## 许可说明

- Quaternius 模型：**CC0 1.0 Universal**，允许商业使用、修改与再分发，无署名要求
  （源项目 `ATTRIBUTIONS.md` 记录了获取方式与套装版本）；
- 地面/墙面贴图由源项目程序化生成并声明等效 CC0；
- 本项目未修改模型文件；如后续修改，仍受 CC0 约束，可自由分发。

## 实际验证方式

- `示例项目::三维示例使用受光几何体与真实模型`：场景里必须有 5 个
  `Primitive3d`（地面 + 四面墙）、27 个 Sprite（23 掩体 + 4 角色），
  且 `受光: true`、含 `贴图重复`；
- 所有 PNG 必须被场景引用；模型路径必须是项目相对路径。
