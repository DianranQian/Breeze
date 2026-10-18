# 图标资产说明

本目录是编辑器图标资产的**唯一来源**。运行时不会读取这里的文件——
PNG 由 `include_bytes!` 编进二进制（见 `crates/editor_ui/src/icons.rs`），
所以"干净构建"和"发布构建"的加载路径完全一致。

## 来源与许可

| 项目 | 值 |
| --- | --- |
| 名称 | [Tabler Icons](https://github.com/tabler/tabler-icons)（outline 风格） |
| 版本 | **v3.49.0**（GitHub Releases `latest`，2026-10-05 发布） |
| 上游路径 | `https://raw.githubusercontent.com/tabler/tabler-icons/v3.49.0/icons/outline/<name>.svg` |
| 许可证 | **MIT**，Copyright (c) 2020-2026 Paweł Kuna |
| 许可证全文 | 本目录下的 [`LICENSE`](./LICENSE)（从同一 tag 下载，未修改） |

选择理由：统一 24px 网格、2px 描边、`stroke-linecap: round` 的细线风格，
与桌面编辑器的视觉语言一致；覆盖面足够（面板、播放控制、文件类型、状态提示都有）。
比较过的替代品：Kenney UI Pack（位图、偏游戏化）、Lucide（ISC，覆盖面略窄）。

本项目**没有**复制任何其他产品的品牌资产或专有界面素材；
图标仅按 MIT 条款使用上游开源图形。

## 目录结构

```text
assets/icons/
├── LICENSE          上游 MIT 许可证原文
├── README.md        本文件
├── svg/             上游原始 SVG（57 个：48 个原有 + 9 个场景树节点图标）
└── png/             烘焙产物（57 个，32×32，白色描边）
```

## 烘焙流程

```bash
# 重新生成全部 PNG（改了图标清单或上游版本后执行）
cargo run -p icon_bake

# 只校验：PNG 是否齐全、是否都是 32×32（CI 用）
cargo run -p icon_bake -- --check
```

烘焙时做两件事：

1. `currentColor` → `#FFFFFF`：描边染成纯白，运行时用 `Image::tint(主题色)` 上色，
   **一套文件覆盖深色/浅色/高对比度主题**；
2. 按 24 → 32 的比例缩放渲染（2 倍 DPI 余量）。

## 新增一个图标

1. 把上游 `outline/<name>.svg` 放进 `svg/`（保持原样，便于核对版本）；
2. 在 `crates/editor_ui/src/icons.rs` 的 `图标` 枚举、`名称()`、`字节()` 里各加一行；
3. `cargo run -p icon_bake && cargo test -p editor_ui icons`；
4. 提交 SVG、PNG 与代码三处改动。

若上游版本升级，请**同时**更新本文件的版本号与 `LICENSE`，并在工作日志里记录。

## 体积

57 个 32×32 PNG 合计约 **48 KB**，全部编进二进制；
解码后约 57 × 32 × 32 × 4 ≈ 234 KB 常驻内存（纹理缓存，只在首次使用时上传）。

## 场景树专用图标（第二批次，9 个）

这 9 个图标是为场景树节点类型系统加的（见 crates/editor_ui/src/node_icons.rs）。
它们此前用字符 ▸ / ▾ / 👁 / 🔒 表示，而内置中文字体子集里
**没有这些字形**，于是界面上被渲染成空心方框（tofu）—— 用户看到的
所有节点都是空心正方形就是这么来的。

| 文件名 | 用途 |
| --- | --- |
| chevron-right.svg | 折叠指示器：有子节点且已折叠 |
| chevron-down.svg | 折叠指示器：有子节点且已展开 |
| camera.svg | 节点类型：Camera2d |
| map.svg | 节点类型：TileTerrain |
| cube.svg | 节点类型：Primitive3d，以及带 模型 资源的 Sprite |
| photo.svg（复用） | 节点类型：二维 Sprite |
| point.svg | 节点类型：普通实体（无可识别组件、无子实体） |
| hierarchy.svg | 节点类型：容器/分组（无可识别组件、有子实体） |
| eye-off.svg | 实体在编辑器中隐藏 |
| lock-open.svg | 实体未锁定 |

新增这 9 个文件时，SVG 头部保留了来源 URL 与许可证行，便于日后核对版本；
svg/ 里另有一个上游历史的 circle-x.svg 目前没有任何枚举使用
（icons.rs 的自检测试会把它列为未使用源文件，不算错误）。
