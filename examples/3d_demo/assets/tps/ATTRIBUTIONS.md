# 素材来源与授权（ATTRIBUTIONS）

本项目遵循「只使用 CC0 / CC-BY / 同等自由授权素材」的约束。

实际纳入的第三方素材分两类：

- **CC0 1.0 Universal**：角色模型、环境道具模型（Quaternius「Toon Shooter Game Kit」）。
- **SIL OFL 1.1**：中文字体（Noto Sans SC）。

其余素材（全部音频、地面 / 墙面贴图、UI）均为本项目原创程序化生成，版权归本项目所有，等效 CC0。

---

## 一、第三方素材

### 1. Quaternius「Toon Shooter Game Kit」（CC0）

| 项目 | 说明 |
|---|---|
| 出处 | Quaternius（作者：Quaternius / 网站 `quaternius.com`） |
| 套装名 | **Toon Shooter Game Kit (Dec 2022)**，共 74 个模型 |
| 授权 | **CC0 1.0 Universal（公共领域贡献，无需署名）** |
| 获取方式 | 本地资源包 `Toon Shooter Game Kit - Dec 2022`（用户提供的下载副本） |
| 格式 | `.gltf`（自包含：顶点数据内嵌，纹理以 bufferView 内嵌，无外部依赖文件） |

> CC0 不要求署名，此处列出仅为尊重作者、便于追溯来源。

#### 1.1 角色模型（`assets/characters/`）

| 文件 | 用途 | 节点/动画 | 代码引用位置 |
|---|---|---|---|
| `Character_Soldier.gltf` | 玩家角色 | 见下方动画表（17 条） | `resources::玩家模型路径` |
| `Character_Enemy.gltf` | 近战冲锋兵 + 首领 | 同上 | `resources::近战敌人模型路径` / `resources::首领模型路径` |
| `Character_Hazmat.gltf` | 远程射手 | 同上 | `resources::远程敌人模型路径` |

**三个角色模型的内置动画列表完全一致（17 条，0 基）**：

```
0 Death        1 Duck         2 HitReact    3 Idle         4 Idle_Shoot
5 Jump         6 Jump_Idle    7 Jump_Land   8 No           9 Punch
10 Run         11 Run_Gun     12 Run_Shoot  13 Walk         14 Walk_Shoot
15 Wave        16 Yes
```

本项目实际使用的片段（`resources::动画片段::索引()`）：

| 游戏语义 | glTF 动画 | 序号 | 循环 | 用途 |
|---|---|---|---|---|
| 待机 | `Idle` | 3 | 是 | 站立 |
| 行走 | `Walk` | 13 | 是 | 低速移动 |
| 奔跑 | `Run` | 10 | 是 | 高速移动 / 敌人追击 |
| 跳跃 | `Jump` | 5 | 否 | 玩家起跳 |
| 攻击 | `Punch` | 9 | 是 | 近战敌人挥击 |
| 射击 | `Idle_Shoot` | 4 | 是 | 玩家开火 / 远程敌人开火 |
| 死亡 | `Death` | 0 | 否 | 阵亡 |

> **武器节点说明**：每个角色 glTF 把整套 14 把武器（`AK`、`Shotgun`、`SMG`、`Sniper`…）
> 全部挂在右手骨骼 `Index1.R` 下（建模时的「武器展示姿态」）。
> 本项目用 `组件::角色武器显示` + `asset_loader::应用角色武器显示` 按 `Name` 逐个设置
> `Visibility`，只露出当前需要的一把，因此**无需额外的武器模型文件**。

#### 1.2 环境道具模型（`assets/environment/`）

竞技场掩体全部使用真实 glTF 道具，共 23 个实例、7 种模型。
下表尺寸为**世界空间**尺寸（= 局部网格尺寸 × 场景根节点缩放），来源为运行期实测
（遍历 glTF 后代网格，把 `Aabb` 的 **8 个角点**全部变换到世界空间后取并集）：

| 文件 | 用途 | 世界尺寸（长 × 高 × 宽） | 场景根节点缩放 | 代码引用位置 |
|---|---|---|---|---|
| `Crate.gltf` | 木箱掩体 | 0.790 × 0.790 × 0.790 m | 0.3909 | `resources::木箱模型路径` |
| `Container_Long.gltf` | 长货柜掩体 | 4.359 × 2.131 × 2.090 m | 0.586 | `resources::长货柜模型路径` |
| `Container_Small.gltf` | 短货柜掩体 | 2.194 × 2.131 × 2.090 m | 0.586 | `resources::短货柜模型路径` |
| `SackTrench.gltf` | 沙袋掩体 | 3.345 × 1.277 × 0.925 m | 0.586 | `resources::沙袋模型路径` |
| `Barrier_Single.gltf` | 单人铁马 | 1.890 × 0.628 × 0.657 m | (0.7379, 0.3647, 0.3647) | `resources::铁马模型路径` |
| `ExplodingBarrel.gltf` | 油桶 | 0.785 × 1.024 × 0.785 m | 0.3645 | `resources::油桶模型路径` |
| `TrafficCone.gltf` | 交通锥 | 0.675 × 0.637 × 0.675 m | 0.2334 | `resources::交通锥模型路径` |

> ⚠️ **这些 glTF 的场景根节点带非 1 缩放**（0.2334 ~ 0.7379）。
> 直接读 mesh 的 `POSITION` 访问器得到的是**局部**尺寸，**必须乘节点缩放**才是世界尺寸；
> 本项目早期漏乘，导致整张尺寸表小了 1.7 ~ 4.3 倍，进而让碰撞盒变成「隐形墙」。
> 现已改为**运行期实测**并加了自检断言（偏差 > 5% 即失败），详见
> `docs/射击与碰撞体根因分析.md` §8.4。
>
> 每个道具都显式挂 `障碍物` + `碰撞盒`（尺寸取自上面的实测世界尺寸），
> 供玩家移动的手写 AABB 检测与射击系统的解析扫掠使用。

### 2. NotoSansSC-Regular.otf（中文字体，SIL OFL 1.1）

| 项目 | 说明 |
|---|---|
| 文件 | `assets/fonts/NotoSansSC-Regular.otf`（OpenType / CFF，约 8.0 MB，含完整简体中文字形） |
| 许可全文 | `assets/fonts/OFL.txt`（SIL Open Font License 1.1） |
| 用途 | 替换 Bevy 内建默认字体，使全部中文界面文本可以正常显示 |
| 来源 URL | `https://cdn.jsdelivr.net/gh/notofonts/noto-cjk@main/Sans/SubsetOTF/SC/NotoSansSC-Regular.otf` |
| 原始仓库 | `https://github.com/notofonts/noto-cjk`（Noto Sans CJK，Google / Adobe 合作项目） |
| 授权 | **SIL Open Font License 1.1**（允许自由使用、修改、再分发，须保留许可与版权声明） |
| 版权 | © 2014-2024 Adobe (http://www.adobe.com/), with Reserved Font Name 'Source' |
| 代码引用位置 | `src/main.rs` → `替换内建默认字体()` 与常量 `中文字体候选路径` |

> **为什么必须换字体**：Bevy 内建默认字体是 `FiraMono-subset.ttf`（只含拉丁字形），
> 中文会整段渲染成空白方块。本项目在 `app.run()` 之前把该字体覆盖为 Noto Sans SC：
> Bevy 的 `TextPlugin` 把内建字体注册在 `Assets<Font>` 的 `AssetId::default()` 这个「知名句柄」上，
> 而所有未显式指定字体的 `TextFont` 都解析到同一个句柄，因此覆盖一次即可全局生效。
>
> 若该文件缺失，程序会依次回退到本机 Windows 自带中文字体
> （`Deng.ttf` 等线 → `simhei.ttf` 黑体 → `msyh.ttc` 微软雅黑 → `simsun.ttc` 宋体），
> 因此删掉 `assets/fonts/` 在本机也不会退化成方块字。

---

## 二、本项目原创素材（程序化生成，等效 CC0）

以下素材由仓库内脚本 `tools/generate_assets.py` 用 Python 标准库
（`wave` / `struct` / `math` / `zlib`）合成，再经 `ffmpeg` 转码为 OGG Vorbis。
版权归本项目所有，可按 CC0 使用。

### 音频（`assets/audio/`，均为 OGG Vorbis 单声道 44.1 kHz）

| 文件 | 用途 | 代码引用位置 |
|---|---|---|
| `bgm.ogg` | 背景音乐（循环） | `resources::背景音乐路径` |
| `shoot_rifle.ogg` | 步枪射击 | `resources::射击音效路径` |
| `shoot_shotgun.ogg` | 霰弹枪射击 | `resources::霰弹音效路径` |
| `hit.ogg` | 命中目标 | `resources::命中音效路径` |
| `reload.ogg` | 换弹 | `resources::换弹音效路径` |
| `enemy_death.ogg` | 敌人死亡 | `resources::敌人死亡音效路径` |
| `player_hurt.ogg` | 玩家受伤 | `resources::玩家受伤音效路径` |
| `ui_click.ogg` | 界面点击 | `resources::界面点击音效路径` |
| `wave_start.ogg` | 波次开始 | `resources::波次开始音效路径` |
| `pickup.ogg` | 拾取补给 | `resources::拾取音效路径` |

> 说明：Bevy 0.19 默认 `audio` 特性仅启用 **vorbis**（不含 wav），
> 因此全部音频统一转码为 `.ogg`，否则会在运行期报「不支持的音频格式」。

### 贴图（`assets/environment/`，均为程序化生成的 256×256 PNG）

| 文件 | 用途 | 代码引用位置 |
|---|---|---|
| `floor.png` | 竞技场地面 | `resources::地面贴图路径` |
| `wall.png` | 四面围墙 | `resources::墙面贴图路径` |

### 纯代码绘制（无素材文件，运行期用 Bevy 内置网格 / UI 节点构建）

| 对象 | 实现方式 | 代码位置 |
|---|---|---|
| 准星 | UI 节点绘制的十字 + 中心点 | `plugins/hud.rs` |
| 菜单 / 面板背景 | 全屏半透明纯色 `Node` | `components::生成菜单容器` |
| 敌人血条 | `Cuboid` 底色条 + 填充条 | `plugins/enemy.rs` |
| 弹药补给箱 | 自发光 `Cuboid` | `plugins/pickup.rs` |
| 命中火花 / 血雾 / 枪口火焰 | 小 `Cuboid` 粒子 | `plugins/effects.rs` |
| 弹道残留 | 细长 `Cuboid` | `plugins/shooting.rs` |
| 受击方向指示器 | `Node` 圆形 + 方位计算 | `plugins/hud.rs` |

---

## 三、未采用的素材（已主动排除 / 清理）

| 素材 | 原因 |
|---|---|
| `RobotExpressive.glb`（three.js 示例） | 早期迭代使用的角色模型，接入 Quaternius 真实模型后**已停用**（文件仍保留在 `assets/characters/` 作为备份，代码中无任何引用） |
| `Soldier.glb`（three.js 示例目录内） | 原始来源为 **Mixamo**，授权条款不允许自由再分发，**已删除**，未纳入本项目 |
| `weapons/AK.gltf`、`weapons/Shotgun.gltf` | 独立武器模型文件；改用角色 glTF 内置武器节点后**不再被代码引用**，仅作保留 |
| `environment/Pallet.gltf`、`crate.png`、`fence.png` | 未被任何代码引用的冗余素材，**已删除** |
| KayKit 系列 GLB | 在本次开发环境中未找到可稳定直连的 CC0 直链，故未纳入 |

---

## 四、替换素材的方法

- **换角色模型**：把 CC0/CC-BY 的 `.gltf` / `.glb` 放入 `assets/characters/`，
  更新 `src/resources/mod.rs` 中的 `玩家模型路径` / `近战敌人模型路径` / `远程敌人模型路径` /
  `首领模型路径`，并按新模型的动画顺序修正 `动画片段::索引()` 与 `动画总数`。
  若新模型不自带手持武器，需要另外加载武器模型并挂到手部骨骼。
- **换环境道具**：把模型放入 `assets/environment/`，
  在 `src/resources/mod.rs` 增加路径常量，并在 `src/plugins/level.rs` 的 `掩体布局` 中登记
  `(模型函数, (x, z), 是否旋转90°, (碰撞盒半长,半宽,半高))`。
- **CC-BY 素材**：请在上表中补充「作者 / 来源 URL / 授权」三列信息。
