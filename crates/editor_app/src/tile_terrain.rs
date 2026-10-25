//! 程序化瓦片地形：把 `TileTerrain` 组件描述的字符网格，按原项目的
//! **双网格自动瓦片**规格合成为一张纹理。
//!
//! # 这不是"把地图烘焙成图片"
//!
//! 场景里存的是**地形数据与图集参数**，不是一张预先拼好的 PNG。每次加载
//! （或参数变化）都重新走一遍：
//!
//! ```text
//! 字符网格 ──► 逻辑格材质 ──► 每张视觉瓦片的四角掩码 ──► 图集取 16px 瓦片
//!          ──► 按材质分层合成（后画的抢像素） ──► 最近邻放大到每格像素
//! ```
//!
//! 规则逐条对应原项目：
//!
//! | 规则 | 出处 |
//! | --- | --- |
//! | 图集是 4×4 张 16px 双网格瓦片，索引乱序 | `src/mapgen/atlas.rs` 的 `POS` |
//! | 四角位权 `[TL, TR, BR, BL] = [8, 4, 1, 2]` | `CORNER_BIT` |
//! | 地表绘制顺序 | `DRAW_SURFACES` |
//! | 整格手绘变体与 0.35 概率 | `VARIANT_FILE` / `balance.ron` |
//! | 街道、耕地在全部地表之后 | `bake.rs` 的 `Layer::ALL` |
//!
//! 这就是"程序化渲染"与"换一张图"的分界：图集是素材，地图是数据，
//! 像素在运行时按规则算出来。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use editor_core::terrain::地形网格;
use editor_core::tilemap::瓦片地图;
use editor_core::tileset::{变体表, 图集源, 瓦片集 as 瓦片集定义};
use image::{Rgba, RgbaImage, imageops};
use scene_doc::property::属性值;

/// 逻辑格边长（原项目 `dualgrid::TILE`）。
const 瓦片边长: u32 = 16;

/// 图集里"第几张"才是掩码 `mask` 那张（原项目 `atlas::POS`）。
const POS: [usize; 16] = [12, 13, 0, 3, 8, 1, 14, 5, 15, 4, 11, 2, 9, 10, 7, 6];

/// 四角位权，顺序 `[TL, TR, BR, BL]`（原项目 `atlas::CORNER_BIT`）。
const CORNER_BIT: [u8; 4] = [8, 4, 1, 2];

/// 变体表尺寸（7 列 × 6 行 × 16px）。
const 变体列: u32 = 7;
const 变体行: u32 = 6;

/// 一张 64×64 的四角图集，切成 16 张 16px 瓦片，`[mask]` 直接取。
#[derive(Debug, Clone)]
struct 图集 {
    瓦片: Vec<RgbaImage>,
}

impl 图集 {
    fn 取(&self, mask: u8) -> &RgbaImage {
        &self.瓦片[mask as usize]
    }
}

/// 地表材质。文件名对应原项目 `assets/art/瓦片/透明到X.png`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum 材质 {
    湿土,
    深草,
    浅草,
    干土,
    沙,
    丘,
    山,
    水,
    道路,
    耕地,
}

impl 材质 {
    /// 全部地表材质，**按绘制顺序**（原项目 `DRAW_SURFACES`）。
    const 地表顺序: [材质; 8] = [
        材质::湿土,
        材质::深草,
        材质::浅草,
        材质::干土,
        材质::沙,
        材质::丘,
        材质::山,
        材质::水,
    ];

    fn 文件名(self) -> &'static str {
        match self {
            Self::湿土 => "透明到湿润的土.png",
            Self::深草 => "透明到深色草地.png",
            Self::浅草 => "透明到浅草地.png",
            Self::干土 => "透明到干燥土地.png",
            Self::沙 => "透明到沙滩.png",
            Self::丘 => "透明到山丘.png",
            Self::山 => "透明到山地.png",
            Self::水 => "透明到海水.png",
            Self::道路 => "透明到道路.png",
            Self::耕地 => "透明到耕地.png",
        }
    }

    /// 整格手绘变体表里的行号（原项目 `VARIANT_ROWS`）。没有变体的返回 `None`。
    fn 变体行(self) -> Option<usize> {
        match self {
            Self::沙 => Some(0),
            Self::干土 => Some(1),
            Self::浅草 => Some(2),
            Self::深草 => Some(3),
            Self::湿土 => Some(4),
            Self::山 => Some(5),
            _ => None,
        }
    }

    /// 地表字符 → 材质。字符选择见 `assets/**/*.terrain.ron` 头部注释。
    fn 由地表字符(字符: char) -> Option<Self> {
        match 字符 {
            '草' => Some(Self::浅草),
            '林' => Some(Self::深草),
            '水' => Some(Self::水),
            '沙' => Some(Self::沙),
            '山' => Some(Self::山),
            '丘' => Some(Self::丘),
            '土' => Some(Self::湿土),
            '旱' => Some(Self::干土),
            _ => None,
        }
    }
}

/// 覆盖层（道路/耕地）。它们不是"地形"，而是压在表面材质上的特征。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum 覆盖 {
    无,
    道路,
    耕地,
}

impl 覆盖 {
    fn 由字符(字符: char) -> Option<Self> {
        match 字符 {
            '.' | '·' => Some(Self::无),
            '路' => Some(Self::道路),
            '田' => Some(Self::耕地),
            _ => None,
        }
    }
}

/// 一张地形图的数据：地表与覆盖两层材质网格。
///
/// 磁盘格式与校验由 [`editor_core::terrain`] 负责；这里只把字符翻译成
/// 渲染用的材质枚举。两处共用同一份解析器，因此面板存下的文件
/// 预览一定读得懂。
#[derive(Debug, Clone)]
struct 地形数据 {
    宽: usize,
    高: usize,
    地表: Vec<Vec<材质>>,
    覆盖: Vec<Vec<覆盖>>,
}

impl 地形数据 {
    fn 读取(路径: &Path) -> Result<Self, String> {
        let 文本 = std::fs::read_to_string(路径)
            .map_err(|错误| format!("读取地形文件 {} 失败：{错误}", 路径.display()))?;
        let 网格 = 地形网格::解析(&文本)
            .map_err(|错误| format!("解析地形文件 {} 失败：{错误}", 路径.display()))?;

        let mut 地表 = Vec::with_capacity(网格.高);
        for (行号, 行) in 网格.地表.iter().enumerate() {
            let mut 材质行 = Vec::with_capacity(网格.宽);
            for (列号, 字) in 行.iter().enumerate() {
                材质行.push(
                    材质::由地表字符(*字)
                        .ok_or_else(|| format!("地表第 {行号} 行第 {列号} 列有未知字符 `{字}`"))?,
                );
            }
            地表.push(材质行);
        }

        let mut 覆盖层 = Vec::with_capacity(网格.高);
        for (行号, 行) in 网格.覆盖.iter().enumerate() {
            let mut 本行 = Vec::with_capacity(网格.宽);
            for (列号, 字) in 行.iter().enumerate() {
                本行.push(
                    覆盖::由字符(*字)
                        .ok_or_else(|| format!("覆盖第 {行号} 行第 {列号} 列有未知字符 `{字}`"))?,
                );
            }
            覆盖层.push(本行);
        }

        Ok(Self {
            宽: 网格.宽,
            高: 网格.高,
            地表,
            覆盖: 覆盖层,
        })
    }
}

/// 从 `TileTerrain` 组件属性解析出的渲染参数。
#[derive(Debug, Clone, PartialEq)]
pub struct 地形参数 {
    /// 旧格式地形数据文件（项目相对路径）。
    ///
    /// 为空且 `地图文件` 也为空时没有内容可画；两者都设置时优先用 `地图文件`。
    pub 地形文件: String,
    /// 旧格式图集目录（项目相对路径）。
    pub 图集目录: String,
    /// 通用瓦片地图文件（`.tilemap.ron`，项目相对路径）。
    ///
    /// 非空时走**资产驱动**的渲染路径：瓦片集由开发者定义，
    /// 不再依赖固定文件名与固定材质表。
    pub 地图文件: String,
    /// 通用瓦片集文件（`.tileset.ron`，项目相对路径）。
    ///
    /// 为空时使用地图文件里记录的 `瓦片集` 路径。
    pub 瓦片集: String,
    /// 世界显示尺寸。
    pub 显示尺寸: (f32, f32),
    /// 每逻辑格烘焙成多少像素。
    pub 每格像素: u32,
    /// 整格变体概率。
    pub 变体概率: f32,
}

impl Default for 地形参数 {
    fn default() -> Self {
        Self {
            地形文件: String::new(),
            图集目录: "assets/terrain/tiles".to_string(),
            地图文件: String::new(),
            瓦片集: String::new(),
            显示尺寸: (1024.0, 1024.0),
            每格像素: 64,
            变体概率: 0.35,
        }
    }
}

impl 地形参数 {
    /// 从场景文档的组件属性解析。缺失字段走默认值，不让旧场景直接坏掉。
    pub fn 解析(属性: &std::collections::BTreeMap<String, 属性值>) -> Self {
        let mut 参数 = Self::default();
        if let Some(值) = 属性.get("地形文件").and_then(属性值::取文本) {
            参数.地形文件 = 值.to_string();
        }
        if let Some(值) = 属性.get("图集目录").and_then(属性值::取文本) {
            参数.图集目录 = 值.to_string();
        }
        if let Some(值) = 属性.get("地图文件").and_then(属性值::取文本) {
            参数.地图文件 = 值.to_string();
        }
        if let Some(值) = 属性.get("瓦片集").and_then(属性值::取文本) {
            参数.瓦片集 = 值.to_string();
        }
        if let Some(值) = 属性.get("显示尺寸").and_then(属性值::取二维) {
            参数.显示尺寸 = (值.0.abs().max(1.0), 值.1.abs().max(1.0));
        }
        if let Some(值) = 属性.get("每格像素").and_then(属性值::取整数) {
            参数.每格像素 = 值.clamp(8, 128) as u32;
        }
        if let Some(值) = 属性.get("变体概率").and_then(属性值::取浮点) {
            参数.变体概率 = 值.clamp(0.0, 1.0);
        }
        参数
    }
}

/// 烘焙结果。
pub struct 烘焙结果 {
    /// 可直接交给 `Assets<Image>` 的图片。
    pub 图片: Image,
    /// 纹理像素尺寸。
    pub 纹理尺寸: (u32, u32),
    /// 逻辑格数。
    pub 格数: (u32, u32),
}

/// 把项目相对路径转成本机路径。
fn 项目路径(项目根: &Path, 相对: &str) -> PathBuf {
    项目根.join(相对.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// 载入一张 64×64 四角图集，按 `POS` 搬成 `[mask]` 下标。
fn 载入图集(路径: &Path) -> Result<图集, String> {
    let 图 = image::open(路径)
        .map_err(|错误| format!("读取图集 {} 失败：{错误}", 路径.display()))?
        .to_rgba8();
    if 图.dimensions() != (64, 64) {
        return Err(format!(
            "图集 {} 尺寸 {:?}，期望 64×64",
            路径.display(),
            图.dimensions()
        ));
    }

    let mut 瓦片 = Vec::with_capacity(16);
    for mask in 0..16 {
        let 索引 = POS[mask as usize];
        let x = (索引 % 4) as u32 * 瓦片边长;
        let y = (索引 / 4) as u32 * 瓦片边长;
        瓦片.push(imageops::crop_imm(&图, x, y, 瓦片边长, 瓦片边长).to_image());
    }
    Ok(图集 { 瓦片 })
}

/// 载入 112×96 的整格变体表（7 列 × 6 行）。
fn 载入变体表(路径: &Path) -> Result<Vec<Vec<RgbaImage>>, String> {
    let 图 = image::open(路径)
        .map_err(|错误| format!("读取变体表 {} 失败：{错误}", 路径.display()))?
        .to_rgba8();
    if 图.dimensions() != (变体列 * 瓦片边长, 变体行 * 瓦片边长) {
        return Err(format!(
            "变体表 {} 尺寸 {:?}，期望 {}×{}",
            路径.display(),
            图.dimensions(),
            变体列 * 瓦片边长,
            变体行 * 瓦片边长
        ));
    }
    let mut 行表 = Vec::with_capacity(变体行 as usize);
    for 行 in 0..变体行 {
        let mut 本行 = Vec::with_capacity(变体列 as usize);
        for 列 in 0..变体列 {
            本行.push(
                imageops::crop_imm(&图, 列 * 瓦片边长, 行 * 瓦片边长, 瓦片边长, 瓦片边长)
                    .to_image(),
            );
        }
        行表.push(本行);
    }
    Ok(行表)
}

/// 一组图集与变体表。
struct 瓦片集 {
    图集: HashMap<材质, 图集>,
    变体: Vec<Vec<RgbaImage>>,
}

impl 瓦片集 {
    fn 载入(目录: &Path) -> Result<Self, String> {
        let mut 图集 = HashMap::new();
        for 材质 in [
            材质::湿土,
            材质::深草,
            材质::浅草,
            材质::干土,
            材质::沙,
            材质::丘,
            材质::山,
            材质::水,
            材质::道路,
            材质::耕地,
        ] {
            图集.insert(材质, 载入图集(&目录.join(材质.文件名()))?);
        }
        let 变体 = 载入变体表(&目录.join("瓦片地图变体_丰富地图细节.png"))?;
        Ok(Self { 图集, 变体 })
    }

    fn 取(&self, 材质: 材质, mask: u8) -> &RgbaImage {
        self.图集
            .get(&材质)
            .expect("瓦片集在载入时已覆盖全部材质")
            .取(mask)
    }
}

/// 计算一张视觉瓦片的四角掩码。
///
/// 屏幕坐标 y 向下，所以四角就是图像里的左上、右上、右下、左下。
/// 位权顺序必须与图集的 `CORNER_BIT` 一致。
fn 四角掩码(
    视觉x: i32,
    视觉y: i32,
    宽: usize,
    高: usize,
    属于: impl Fn(usize, usize) -> bool,
) -> u8 {
    四角掩码带位权(视觉x, 视觉y, 宽, 高, CORNER_BIT, 属于)
}

/// 通用四角掩码：角位权由瓦片集给出。
fn 四角掩码带位权(
    视觉x: i32,
    视觉y: i32,
    宽: usize,
    高: usize,
    角位权: [u8; 4],
    属于: impl Fn(usize, usize) -> bool,
) -> u8 {
    let 四角 = [
        (视觉x - 1, 视觉y - 1),
        (视觉x, 视觉y - 1),
        (视觉x, 视觉y),
        (视觉x - 1, 视觉y),
    ];
    let mut mask = 0u8;
    for (位, (x, y)) in 角位权.iter().zip(四角) {
        // 地图边缘按 clamp 处理，视觉上像地形continue到图外，而不是透明缺口。
        let x = x.clamp(0, 宽 as i32 - 1) as usize;
        let y = y.clamp(0, 高 as i32 - 1) as usize;
        if 属于(x, y) {
            mask |= 位;
        }
    }
    mask
}

/// 把一张 16px 瓦片按 alpha 覆盖到画布上（后画的赢）。
fn 贴瓦片(画布: &mut RgbaImage, 瓦片: &RgbaImage, x: i32, y: i32) {
    贴瓦片尺寸(画布, 瓦片, x, y, 瓦片边长, 瓦片边长);
}

/// 把任意尺寸的瓦片按 alpha 覆盖到画布上（后画的赢）。
fn 贴瓦片尺寸(
    画布: &mut RgbaImage,
    瓦片: &RgbaImage,
    x: i32,
    y: i32,
    瓦片宽: u32,
    瓦片高: u32,
) {
    let (宽, 高) = 画布.dimensions();
    for py in 0..瓦片高 as i32 {
        for px in 0..瓦片宽 as i32 {
            let dx = x + px;
            let dy = y + py;
            if dx < 0 || dy < 0 || dx >= 宽 as i32 || dy >= 高 as i32 {
                continue;
            }
            let 像素 = 瓦片.get_pixel(px as u32, py as u32);
            if 像素[3] > 0 {
                画布.put_pixel(dx as u32, dy as u32, *像素);
            }
        }
    }
}

/// 整格瓦片：按概率抽手绘变体，否则用图集自带的"全 1"格。
fn 选整格(
    瓦片集: &瓦片集, 材质: 材质, 视觉x: i32, 视觉y: i32, 概率: f32
) -> &RgbaImage {
    let Some(行) = 材质.变体行() else {
        return 瓦片集.取(材质, 15);
    };
    let mut 散列 = (视觉x as u64).wrapping_mul(73856093)
        ^ (视觉y as u64).wrapping_mul(19349663)
        ^ (行 as u64).wrapping_mul(83492791);
    散列 &= 0x7fff_ffff;
    let 抽中 = (散列 % 1000) as f32 / 1000.0 < 概率;
    if 抽中 {
        &瓦片集.变体[行][((散列 / 1000) as usize) % 变体列 as usize]
    } else {
        瓦片集.取(材质, 15)
    }
}

// ---------------------------------------------------------------------------
// 通用瓦片集 / 瓦片地图（资产驱动）
// ---------------------------------------------------------------------------

/// 载入好的一张图集源：瓦片按行优先展开。
struct 载入源 {
    瓦片: Vec<RgbaImage>,
}

/// 载入好的变体表。
struct 载入变体 {
    瓦片: Vec<RgbaImage>,
    列: u32,
    行: u32,
}

impl 载入变体 {
    fn 取(&self, 行: u32, 列: u32) -> Option<&RgbaImage> {
        if 行 >= self.行 || 列 >= self.列 {
            return None;
        }
        self.瓦片.get((行 * self.列 + 列) as usize)
    }
}

/// 按瓦片集的尺寸参数切一张图集源。
fn 载入源图片(
    项目根: &Path, 集: &瓦片集定义, 源: &图集源
) -> Result<载入源, String> {
    let 路径 = 项目路径(项目根, &源.图片);
    let 图 = image::open(&路径)
        .map_err(|错误| format!("读取图集 {} 失败：{错误}", 路径.display()))?
        .to_rgba8();
    let (图宽, 图高) = 图.dimensions();
    let 列 = 源.计算列数(图宽, 集.瓦片宽, 集.边距, 集.间距);
    let 行 = 源.计算行数(图高, 集.瓦片高, 集.边距, 集.间距);
    if 列 == 0 || 行 == 0 {
        return Err(format!(
            "图集源「{}」算出的行列数为 0，请检查图片尺寸与瓦片尺寸",
            源.显示名()
        ));
    }

    let 步宽 = 集.瓦片宽 + 集.间距;
    let 步高 = 集.瓦片高 + 集.间距;
    let mut 瓦片 = Vec::with_capacity((列 * 行) as usize);
    for 行号 in 0..行 {
        for 列号 in 0..列 {
            let x = 集.边距 + 列号 * 步宽;
            let y = 集.边距 + 行号 * 步高;
            if x + 集.瓦片宽 > 图宽 || y + 集.瓦片高 > 图高 {
                return Err(format!(
                    "图集源「{}」第 {行号} 行第 {列号} 列超出图片 {图宽}×{图高}",
                    源.显示名()
                ));
            }
            瓦片.push(imageops::crop_imm(&图, x, y, 集.瓦片宽, 集.瓦片高).to_image());
        }
    }
    Ok(载入源 { 瓦片 })
}

/// 按瓦片集的尺寸参数切变体表。
fn 载入变体图片(
    项目根: &Path,
    集: &瓦片集定义,
    变体: &变体表,
) -> Result<载入变体, String> {
    let 路径 = 项目路径(项目根, &变体.图片);
    let 图 = image::open(&路径)
        .map_err(|错误| format!("读取变体表 {} 失败：{错误}", 路径.display()))?
        .to_rgba8();
    if 变体.列 == 0 || 变体.行 == 0 {
        return Err("变体表的行列数不能为 0".to_string());
    }
    let mut 瓦片 = Vec::with_capacity((变体.列 * 变体.行) as usize);
    for 行 in 0..变体.行 {
        for 列 in 0..变体.列 {
            瓦片.push(
                imageops::crop_imm(&图, 列 * 集.瓦片宽, 行 * 集.瓦片高, 集.瓦片宽, 集.瓦片高)
                    .to_image(),
            );
        }
    }
    Ok(载入变体 {
        瓦片,
        列: 变体.列,
        行: 变体.行,
    })
}

/// 变体抽签：同一格每次烘焙都抽到同一张，避免重新载入时地面闪变。
fn 变体抽签(视觉x: i32, 视觉y: i32, 行: u32, 列数: u32, 概率: f32) -> Option<u32> {
    if 列数 == 0 || 概率 <= 0.0 {
        return None;
    }
    let mut 散列 = (视觉x as u64).wrapping_mul(73856093)
        ^ (视觉y as u64).wrapping_mul(19349663)
        ^ (行 as u64).wrapping_mul(83492791);
    散列 &= 0x7fff_ffff;
    let 抽中 = (散列 % 1000) as f32 / 1000.0 < 概率;
    if 抽中 {
        Some(((散列 / 1000) % 列数 as u64) as u32)
    } else {
        None
    }
}

/// 资产驱动的烘焙：瓦片集与地图都来自开发者定义的文件。
fn 烘焙通用(项目根: &Path, 参数: &地形参数) -> Result<烘焙结果, String> {
    let 地图路径 = 项目路径(项目根, &参数.地图文件);
    let 地图文本 = std::fs::read_to_string(&地图路径)
        .map_err(|错误| format!("读取瓦片地图 {} 失败：{错误}", 地图路径.display()))?;
    let 地图 = 瓦片地图::解析(&地图文本)
        .map_err(|错误| format!("解析瓦片地图 {} 失败：{错误}", 地图路径.display()))?;

    let 集相对 = if 参数.瓦片集.trim().is_empty() {
        地图.瓦片集.trim().to_string()
    } else {
        参数.瓦片集.trim().to_string()
    };
    if 集相对.is_empty() {
        return Err("瓦片地图没有设置 `瓦片集` 路径".to_string());
    }
    let 集路径 = 项目路径(项目根, &集相对);
    let 集文本 = std::fs::read_to_string(&集路径)
        .map_err(|错误| format!("读取瓦片集 {} 失败：{错误}", 集路径.display()))?;
    let 集 = 瓦片集定义::解析(&集文本)
        .map_err(|错误| format!("解析瓦片集 {} 失败：{错误}", 集路径.display()))?;

    let 载入: Vec<载入源> = 集
        .图集源
        .iter()
        .map(|源| 载入源图片(项目根, &集, 源))
        .collect::<Result<_, _>>()?;
    let 变体 = match &集.双网格.变体 {
        Some(定义) => Some(载入变体图片(项目根, &集, 定义)?),
        None => None,
    };

    let 瓦片宽 = 集.瓦片宽;
    let 瓦片高 = 集.瓦片高;
    let 每格 = 参数.每格像素.clamp(8, 128);
    let 纹理宽 = 地图.宽 as u32 * 每格;
    let 纹理高 = 地图.高 as u32 * 每格;
    if 纹理宽 > 8192 || 纹理高 > 8192 {
        return Err(format!(
            "瓦片地图纹理 {纹理宽}×{纹理高} 超过 8192 上限（调小 `每格像素`）"
        ));
    }

    let mut 画布 = RgbaImage::new(地图.宽 as u32 * 瓦片宽, 地图.高 as u32 * 瓦片高);

    // 双网格图层里，源的绘制顺序由瓦片集给出；同序时按声明顺序。
    let mut 双网格源: Vec<usize> = (0..载入.len())
        .filter(|序号| 集.图集源[*序号].双网格)
        .collect();
    双网格源.sort_by_key(|序号| (集.图集源[*序号].绘制顺序, *序号));

    for 层 in &地图.图层 {
        if 层.双网格 && 集.双网格.启用 {
            for vy in 0..=地图.高 as i32 {
                for vx in 0..=地图.宽 as i32 {
                    for &源序号 in &双网格源 {
                        let mask = 四角掩码带位权(
                            vx,
                            vy,
                            地图.宽,
                            地图.高,
                            集.双网格.角位权,
                            |x, y| 层.取格(x, y).map(|格| 格.源 as usize) == Some(源序号),
                        );
                        if mask == 0 {
                            continue;
                        }

                        let 源 = &载入[源序号];
                        let 瓦片序号 = 集.源的掩码表(源序号)[mask as usize] as usize;
                        let mut 选中 = 源.瓦片.get(瓦片序号).ok_or_else(|| {
                            format!(
                                "图层「{}」的源「{}」掩码 {mask} 指向第 {瓦片序号} 张瓦片，\
                                 但该源只有 {} 张",
                                层.名称,
                                集.图集源[源序号].显示名(),
                                源.瓦片.len()
                            )
                        })?;

                        // 整格（四角全属于）时按概率换成手绘变体。
                        if mask == 15
                            && let (Some(行), Some(变体)) = (集.图集源[源序号].变体行, &变体)
                            && let Some(列) = 变体抽签(vx, vy, 行, 变体.列, 参数.变体概率)
                            && let Some(替换) = 变体.取(行, 列)
                        {
                            选中 = 替换;
                        }

                        贴瓦片尺寸(
                            &mut 画布,
                            选中,
                            vx * 瓦片宽 as i32 - 瓦片宽 as i32 / 2,
                            vy * 瓦片高 as i32 - 瓦片高 as i32 / 2,
                            瓦片宽,
                            瓦片高,
                        );
                    }
                }
            }
        } else {
            for y in 0..地图.高 {
                for x in 0..地图.宽 {
                    let Some(格) = 层.取格(x, y) else {
                        continue;
                    };
                    let 源 = 载入.get(格.源 as usize).ok_or_else(|| {
                        format!(
                            "图层「{}」({x},{y}) 引用第 {} 个图集源，但瓦片集只有 {} 个",
                            层.名称,
                            格.源,
                            载入.len()
                        )
                    })?;
                    let 瓦片 = 源.瓦片.get(格.瓦片 as usize).ok_or_else(|| {
                        format!(
                            "图层「{}」({x},{y}) 引用源「{}」的第 {} 张瓦片，但该源只有 {} 张",
                            层.名称,
                            集.图集源[格.源 as usize].显示名(),
                            格.瓦片,
                            源.瓦片.len()
                        )
                    })?;
                    贴瓦片尺寸(
                        &mut 画布,
                        瓦片,
                        x as i32 * 瓦片宽 as i32,
                        y as i32 * 瓦片高 as i32,
                        瓦片宽,
                        瓦片高,
                    );
                }
            }
        }
    }

    let 最终 = if 每格 == 瓦片宽 && 瓦片宽 == 瓦片高 {
        画布
    } else {
        imageops::resize(&画布, 纹理宽, 纹理高, imageops::FilterType::Nearest)
    };

    let mut 图片 = Image::new(
        Extent3d {
            width: 纹理宽,
            height: 纹理高,
            ..default()
        },
        TextureDimension::D2,
        最终.into_raw(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    图片.sampler = ImageSampler::linear();

    info!(
        "[瓦片地图] 合成完成：{}×{} 像素，{}×{} 格，{} 个图层",
        纹理宽,
        纹理高,
        地图.宽,
        地图.高,
        地图.图层.len()
    );

    Ok(烘焙结果 {
        图片,
        纹理尺寸: (纹理宽, 纹理高),
        格数: (地图.宽 as u32, 地图.高 as u32),
    })
}

/// 按原项目规格烘焙地形。
pub fn 烘焙(项目根: &Path, 参数: &地形参数) -> Result<烘焙结果, String> {
    // 新路径优先：只要设置了地图文件就走资产驱动渲染，
    // 旧字段仍可留在场景里，不强迫用户迁移。
    if !参数.地图文件.trim().is_empty() {
        return 烘焙通用(项目根, 参数);
    }
    if 参数.地形文件.trim().is_empty() {
        return Err("TileTerrain 未设置 `地形文件`".to_string());
    }
    if 参数.图集目录.trim().is_empty() {
        return Err("TileTerrain 未设置 `图集目录`".to_string());
    }

    let 地形路径 = 项目路径(项目根, &参数.地形文件);
    let 图集目录 = 项目路径(项目根, &参数.图集目录);
    let 数据 = 地形数据::读取(&地形路径)?;
    let 瓦片集 = 瓦片集::载入(&图集目录)?;

    let 每格 = 参数.每格像素.clamp(8, 128);
    let 纹理宽 = 数据.宽 as u32 * 每格;
    let 纹理高 = 数据.高 as u32 * 每格;
    if 纹理宽 > 8192 || 纹理高 > 8192 {
        return Err(format!(
            "地形纹理 {纹理宽}×{纹理高} 超过 8192 上限（调小 `每格像素`）"
        ));
    }

    let mut 画布 = RgbaImage::new(数据.宽 as u32 * 瓦片边长, 数据.高 as u32 * 瓦片边长);

    for vy in 0..=数据.高 as i32 {
        for vx in 0..=数据.宽 as i32 {
            let px = vx * 瓦片边长 as i32 - 瓦片边长 as i32 / 2;
            let py = vy * 瓦片边长 as i32 - 瓦片边长 as i32 / 2;

            for 材质 in 材质::地表顺序 {
                let mask = 四角掩码(vx, vy, 数据.宽, 数据.高, |x, y| {
                    数据.地表[y][x] == 材质
                });
                if mask == 0 {
                    continue;
                }
                let 瓦片 = if mask == 15 {
                    选整格(&瓦片集, 材质, vx, vy, 参数.变体概率)
                } else {
                    瓦片集.取(材质, mask)
                };
                贴瓦片(&mut 画布, 瓦片, px, py);
            }

            let 道路 = 四角掩码(vx, vy, 数据.宽, 数据.高, |x, y| {
                数据.覆盖[y][x] == 覆盖::道路
            });
            if 道路 != 0 {
                贴瓦片(&mut 画布, 瓦片集.取(材质::道路, 道路), px, py);
            }

            let 耕地 = 四角掩码(vx, vy, 数据.宽, 数据.高, |x, y| {
                数据.覆盖[y][x] == 覆盖::耕地
            });
            if 耕地 != 0 {
                // 原项目按生长阶段给耕地乘 tint；这里固定一档稍亮的土色，
                // 并把田埂的纯黑描边换成深棕 —— 64px/格下纯黑边太抢眼。
                let mut 瓦片 = 瓦片集.取(材质::耕地, 耕地).clone();
                for 像素 in 瓦片.pixels_mut() {
                    if 像素[3] == 0 {
                        continue;
                    }
                    if 像素[0] < 26 && 像素[1] < 26 && 像素[2] < 26 {
                        *像素 = Rgba([58, 40, 30, 像素[3]]);
                    } else {
                        像素[0] = (像素[0] as f32 * 1.30).min(255.0) as u8;
                        像素[1] = (像素[1] as f32 * 1.30).min(255.0) as u8;
                        像素[2] = (像素[2] as f32 * 1.30).min(255.0) as u8;
                    }
                }
                贴瓦片(&mut 画布, &瓦片, px, py);
            }
        }
    }

    let 最终 = if 每格 == 瓦片边长 {
        画布
    } else {
        imageops::resize(&画布, 纹理宽, 纹理高, imageops::FilterType::Nearest)
    };

    let mut 图片 = Image::new(
        Extent3d {
            width: 纹理宽,
            height: 纹理高,
            ..default()
        },
        TextureDimension::D2,
        最终.into_raw(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    // 地形是一张大拼图，视口常常以 <1 的比例整图预览；线性采样在缩小时
    // 不会产生最近邻的闪烁。放大到 1:1 时两者没有差别。
    图片.sampler = ImageSampler::linear();

    info!(
        "[地形] 合成完成：{}×{} 像素，{}×{} 格，每格 {}px",
        纹理宽, 纹理高, 数据.宽, 数据.高, 每格
    );

    Ok(烘焙结果 {
        图片,
        纹理尺寸: (纹理宽, 纹理高),
        格数: (数据.宽 as u32, 数据.高 as u32),
    })
}

// ---------------------------------------------------------------------------
// 缓存
// ---------------------------------------------------------------------------

/// 一次烘焙的缓存项。
struct 缓存项 {
    地形时间: Option<SystemTime>,
    图集时间: Option<SystemTime>,
    每格像素: u32,
    变体概率: f32,
    句柄: Handle<Image>,
}

/// 地形纹理缓存。
///
/// 键只到"地形文件 + 图集目录"这一层，不包含烘焙参数：拖动
/// `每格像素` 滑条时**替换**同一项并释放旧纹理，而不是每变一次就往
/// `Assets<Image>` 里塞一张 3000×2000 的新图。
#[derive(Resource, Default)]
pub struct 地形缓存 {
    条目: HashMap<String, 缓存项>,
    占位: Option<Handle<Image>>,
}

impl 地形缓存 {
    /// 一组地形参数的缓存键。
    ///
    /// 必须包含烘焙参数：同一个地形文件挂两次、参数不同时，
    /// 两张纹理不能互相覆盖。
    pub fn 键(项目根: &Path, 参数: &地形参数) -> String {
        format!(
            "{}|{}|{}|{}|{}|{:.4}",
            项目路径(项目根, &参数.地形文件).display(),
            项目路径(项目根, &参数.图集目录).display(),
            项目路径(项目根, &参数.地图文件).display(),
            项目路径(项目根, &参数.瓦片集).display(),
            参数.每格像素,
            参数.变体概率
        )
    }

    /// 取缓存或重新烘焙。失败时返回洋红占位图，并把原因写进日志。
    pub fn 取或烘焙(
        &mut self,
        项目根: &Path,
        参数: &地形参数,
        图片资源: &mut Assets<Image>,
    ) -> Handle<Image> {
        let 地形路径 = 项目路径(项目根, &参数.地形文件);
        let 图集目录 = 项目路径(项目根, &参数.图集目录);
        let 键 = Self::键(项目根, 参数);

        // 通用路径要盯住地图、瓦片集与全部图集图片：任何一张改了都重烘。
        let (地形时间, 图集时间) = if !参数.地图文件.trim().is_empty() {
            (
                修改时间(&项目路径(项目根, &参数.地图文件)),
                通用依赖时间(项目根, 参数),
            )
        } else {
            (修改时间(&地形路径), 目录修改时间(&图集目录))
        };

        if let Some(项) = self.条目.get(&键)
            && 项.地形时间 == 地形时间
            && 项.图集时间 == 图集时间
            && 项.每格像素 == 参数.每格像素
            && (项.变体概率 - 参数.变体概率).abs() < f32::EPSILON
        {
            return 项.句柄.clone();
        }

        let 句柄 = match 烘焙(项目根, 参数) {
            Ok(结果) => 图片资源.add(结果.图片),
            Err(原因) => {
                warn!("[地形] {原因}");
                self.占位句柄(图片资源)
            }
        };

        // 替换同一地形时释放旧纹理；占位图是全局共享的，不能删。
        if let Some(旧) = self.条目.insert(
            键,
            缓存项 {
                地形时间,
                图集时间,
                每格像素: 参数.每格像素,
                变体概率: 参数.变体概率,
                句柄: 句柄.clone(),
            },
        ) && Some(&旧.句柄) != self.占位.as_ref()
        {
            let _ = 图片资源.remove(旧.句柄.id());
        }
        句柄
    }

    /// 清掉这次同步没有用到的地形纹理。
    ///
    /// 场景里删掉一个 `TileTerrain`、或把 `地形文件` 改成另一个路径时，
    /// 旧条目不会自动消失；一张 3000×2000 的纹理是 25MB，不清理会一直涨。
    pub fn 清理(&mut self, 使用的: &[String], 图片资源: &mut Assets<Image>) {
        let 占位 = self.占位.clone();
        self.条目.retain(|键, 项| {
            if 使用的.contains(键) {
                return true;
            }
            if Some(&项.句柄) != 占位.as_ref() {
                let _ = 图片资源.remove(项.句柄.id());
            }
            false
        });
    }

    /// 取（或创建）1×1 洋红占位图。
    fn 占位句柄(&mut self, 图片资源: &mut Assets<Image>) -> Handle<Image> {
        if let Some(句柄) = &self.占位 {
            return 句柄.clone();
        }
        let 图片 = Image::new(
            Extent3d {
                width: 1,
                height: 1,
                ..default()
            },
            TextureDimension::D2,
            vec![255, 0, 255, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        let 句柄 = 图片资源.add(图片);
        self.占位 = Some(句柄.clone());
        句柄
    }
}

fn 修改时间(路径: &Path) -> Option<SystemTime> {
    std::fs::metadata(路径)
        .ok()
        .and_then(|元| 元.modified().ok())
}

/// 目录里全部图集文件的最近修改时间。任一文件缺失时返回 `None`，
/// 这样"先缺图、后补图"会触发一次重烘焙。
fn 目录修改时间(目录: &Path) -> Option<SystemTime> {
    let mut 最近 = None;
    for 文件名 in [
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
    ] {
        let 时间 = 修改时间(&目录.join(文件名))?;
        最近 = Some(match 最近 {
            Some(旧) if 旧 > 时间 => 旧,
            _ => 时间,
        });
    }
    最近
}

/// 通用路径的依赖时间：瓦片集文件 + 全部图集图片 + 变体表。
///
/// 任一文件缺失或解析失败都返回 `None`，让缓存失效并触发一次重烘 ——
/// 重烘会把真正的错误写进日志，比在缓存里悄悄用旧纹理好。
fn 通用依赖时间(项目根: &Path, 参数: &地形参数) -> Option<SystemTime> {
    let 地图文本 = std::fs::read_to_string(项目路径(项目根, &参数.地图文件)).ok()?;
    let 地图 = 瓦片地图::解析(&地图文本).ok()?;
    let 集相对 = if 参数.瓦片集.trim().is_empty() {
        地图.瓦片集
    } else {
        参数.瓦片集.clone()
    };
    let 集路径 = 项目路径(项目根, &集相对);
    let 集文本 = std::fs::read_to_string(&集路径).ok()?;
    let 集 = 瓦片集定义::解析(&集文本).ok()?;

    let mut 最近 = 修改时间(&集路径)?;
    for 源 in &集.图集源 {
        最近 = 较晚(最近, 修改时间(&项目路径(项目根, &源.图片))?);
    }
    if let Some(变体) = &集.双网格.变体 {
        最近 = 较晚(最近, 修改时间(&项目路径(项目根, &变体.图片))?);
    }
    Some(最近)
}

/// 取两个时间里较晚的一个。
fn 较晚(左: SystemTime, 右: SystemTime) -> SystemTime {
    if 右 > 左 { 右 } else { 左 }
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 四角掩码按位权组合() {
        // 2×2 全属于 → 15。
        assert_eq!(
            四角掩码(1, 1, 2, 2, |_, _| true),
            15,
            "全属于时四角掩码应为 15"
        );
        // 左半属于、右半不属于：TL + BL = 8 + 2 = 10。
        assert_eq!(
            四角掩码(1, 1, 2, 2, |x, _| x == 0),
            10,
            "左半属于时掩码应为 10"
        );
        // 只有右上角属于：TR = 4。
        assert_eq!(
            四角掩码(1, 1, 2, 2, |x, y| x == 1 && y == 0),
            4,
            "只有右上角属于时掩码应为 4"
        );
    }

    #[test]
    fn 边缘采样被_clamp_到图内() {
        // 视觉瓦片 (0,0) 的四角会落在 -1，clamp 后应等价于 (0,0)。
        assert_eq!(四角掩码(0, 0, 1, 1, |_, _| true), 15);
        assert_eq!(四角掩码(0, 0, 1, 1, |_, _| false), 0);
    }

    #[test]
    fn 地表字符有唯一映射() {
        for (字符, 材质) in [
            ('草', 材质::浅草),
            ('林', 材质::深草),
            ('水', 材质::水),
            ('沙', 材质::沙),
            ('山', 材质::山),
            ('丘', 材质::丘),
            ('土', 材质::湿土),
            ('旱', 材质::干土),
        ] {
            assert_eq!(材质::由地表字符(字符), Some(材质));
        }
        assert_eq!(材质::由地表字符('?'), None);
    }

    #[test]
    fn 参数解析会收拢越界值() {
        let mut 属性 = std::collections::BTreeMap::new();
        属性.insert("每格像素".to_string(), 属性值::整数(9999));
        属性.insert("变体概率".to_string(), 属性值::浮点(7.0));
        属性.insert("显示尺寸".to_string(), 属性值::二维(-3.0, 0.0));
        let 参数 = 地形参数::解析(&属性);
        assert_eq!(参数.每格像素, 128);
        assert!((参数.变体概率 - 1.0).abs() < f32::EPSILON);
        assert_eq!(参数.显示尺寸, (3.0, 1.0));
    }

    /// 用仓库里的真实示例数据跑一遍完整烘焙。
    ///
    /// 守住的是"数据 / 图集 / 渲染规格三者对得上"：字符网格宽高、
    /// 图集文件名、`POS` 映射、纹理尺寸，以及最重要的**没有透明像素**
    /// （少一层材质、掩码写反都会露底）。
    #[test]
    fn 示例地形能烘焙出完整纹理() {
        let 仓库根 = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let 参数 = 地形参数 {
            地形文件: "examples/2d_demo/assets/xiuxian/地形/qinglan_valley.terrain.ron"
                .to_string(),
            图集目录: "examples/2d_demo/assets/xiuxian/瓦片".to_string(),
            地图文件: String::new(),
            瓦片集: String::new(),
            显示尺寸: (3072.0, 2048.0),
            每格像素: 64,
            变体概率: 0.35,
        };
        let 结果 = 烘焙(&仓库根, &参数).expect("示例地形应当能烘焙");
        assert_eq!(结果.格数, (48, 32));
        assert_eq!(结果.纹理尺寸, (3072, 2048));

        let 数据 = 结果.图片.data.as_ref().expect("烘焙结果应带像素数据");
        assert_eq!(数据.len(), 3072 * 2048 * 4);
        let 透明像素 = 数据.chunks(4).filter(|像素| 像素[3] == 0).count();
        assert_eq!(透明像素, 0, "双网格合成后不应留下透明缺口");
    }

    /// 通用路径：瓦片集与地图都是开发者定义的文件，不依赖固定文件名。
    ///
    /// 与上一个测试画的是同一张地图，因此除了"能读回自己写出的格式"，
    /// 还顺带守住旧格式与新格式的转换结果一致。
    #[test]
    fn 通用瓦片集地图能烘焙出完整纹理() {
        let 项目根 =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/2d_demo");
        let 参数 = 地形参数 {
            地形文件: String::new(),
            图集目录: String::new(),
            地图文件: "assets/xiuxian/地形/青岚山谷.tilemap.ron".to_string(),
            // 留空：使用地图文件里记录的瓦片集路径。
            瓦片集: String::new(),
            显示尺寸: (3072.0, 2048.0),
            每格像素: 64,
            变体概率: 0.35,
        };
        let 结果 = 烘焙(&项目根, &参数).expect("通用瓦片地图应当能烘焙");
        assert_eq!(结果.格数, (48, 32));
        assert_eq!(结果.纹理尺寸, (3072, 2048));

        let 数据 = 结果.图片.data.as_ref().expect("烘焙结果应带像素数据");
        assert_eq!(数据.len(), 3072 * 2048 * 4);
        let 透明像素 = 数据.chunks(4).filter(|像素| 像素[3] == 0).count();
        assert_eq!(透明像素, 0, "通用双网格合成后不应留下透明缺口");
    }
}
