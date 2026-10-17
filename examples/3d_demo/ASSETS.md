# 素材清单（3D · TPS 竞技场）

来源项目：`C:\Spaces\Share\10_Code\10_Bevy\tps-demo\tps_arena`（本地 Bevy 项目，只读引用）。
源项目的授权说明见同目录 [`ATTRIBUTIONS.md`](./ATTRIBUTIONS.md)。

## 模型（glTF，CC0）

| 文件 | 原始路径 | 类型 | 许可 | 场景用途 |
| --- | --- | --- | --- | --- |
| `characters/Character_Soldier.gltf` | `assets/characters/Character_Soldier.gltf` | 角色网格（含内嵌材质/贴图） | CC0 1.0（Quaternius） | 玩家士兵 |
| `characters/Character_Enemy.gltf` | `assets/characters/Character_Enemy.gltf` | 角色网格 | CC0 1.0（Quaternius） | 近战敌人 |
| `characters/Character_Hazmat.gltf` | `assets/characters/Character_Hazmat.gltf` | 角色网格 | CC0 1.0（Quaternius） | 远程敌人 |
| `environment/Container_Long.gltf` | `assets/environment/Container_Long.gltf` | 道具网格 | CC0 1.0（Quaternius） | 长货柜 |
| `environment/Crate.gltf` | `assets/environment/Crate.gltf` | 道具网格 | CC0 1.0（Quaternius） | 木箱 |
| `environment/ExplodingBarrel.gltf` | `assets/environment/ExplodingBarrel.gltf` | 道具网格 | CC0 1.0（Quaternius） | 爆炸桶 |
| `environment/Barrier_Single.gltf` | `assets/environment/Barrier_Single.gltf` | 道具网格 | CC0 1.0（Quaternius） | 路障 |
| `environment/TrafficCone.gltf` | `assets/environment/TrafficCone.gltf` | 道具网格 | CC0 1.0（Quaternius） | 锥桶 |
| `environment/SackTrench.gltf` | `assets/environment/SackTrench.gltf` | 道具网格 | CC0 1.0（Quaternius） | 沙袋 |

模型均为**自包含 glTF**：顶点数据在 bufferView 中、材质贴图内嵌，无外部 `.bin`/纹理依赖。
文件按原样复制，未转换格式。

## 环境贴图（PNG）

| 文件 | 原始路径 | 类型 | 许可 | 场景用途 |
| --- | --- | --- | --- | --- |
| `environment/floor.png` | `assets/environment/floor.png` | 地面贴图 | 源项目原创，等效 CC0 | 9 块场地砖（四边形，平铺） |
| `environment/wall.png` | `assets/environment/wall.png` | 墙面贴图 | 源项目原创，等效 CC0 | 北侧/西侧围墙 |

## 实际显示情况

| 资产 | 是否显示 | 证据 |
| --- | --- | --- |
| 三名角色模型 | 是 | `docs/验收证据/18_示例_3D_TPS_模型修复.png` |
| 六件道具模型 | 是 | 同上（货柜/木箱/爆炸桶/路障/锥桶/沙袋） |
| 地面 / 墙面贴图 | 是 | 同上 |

## 未复制的部分

- 音效（`.ogg`）：示例不需要音频；
- HUD / UI 贴图：编辑器不运行游戏逻辑；
- `RobotExpressive.glb`：源项目已复制但本示例未使用，未纳入仓库；
- 武器模型（AK/Shotgun）：当前示例未放置武器节点，未复制。

## 许可说明

- Quaternius 模型：**CC0 1.0 Universal**，允许商业使用、修改与再分发，无署名要求
  （源项目 `ATTRIBUTIONS.md` 记录了获取方式与套装版本）；
- 地面/墙面贴图由源项目程序化生成并声明等效 CC0；
- 本项目未修改模型文件；如后续修改，仍受 CC0 约束，可自由分发。
