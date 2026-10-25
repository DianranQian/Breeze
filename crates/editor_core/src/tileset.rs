//! 中文名：瓦片集
//! 瓦片集：`.tileset.ron` 的解析、校验与写回。
//!
//! # 为什么瓦片集是独立文件
//!
//! Godot 把 `TileSet` 做成资源，`TileMapLayer` 只引用它。这样做的好处是
//! **同一套瓦片可以被多张地图复用**，改一次瓦片尺寸或双网格规则，
//! 所有引用它的地图一起生效。微风沿用同一个模型：
//!
//! ```text
//! 瓦片集（图集源 + 瓦片尺寸 + 双网格规则）   ← 开发者定义
//!         ▲
//!         │ 引用
//! 瓦片地图（宽高 + 图层 + 每格引用哪个源的第几张）
//! ```
//!
//! 编辑器不硬编码任何图片名、字符集或目录。图集源来自开发者导入的
//! 图片资产；双网格只是瓦片集上的一个**可选规则**，普通图块照常工作。
//!
//! # 双网格（比 Godot 多的那一个能力）
//!
//! 双网格自动瓦片把逻辑格与视觉瓦片错开半格：每个视觉格看四个逻辑角，
//! 四角属于哪种材质就查一次 16 项掩码表，得到该材质的哪一张图。
//! 因此瓦片集需要记三件事：
//!
//! | 字段 | 含义 |
//! | --- | --- |
//! | `角位权` | 左上/右上/右下/左下分别对应掩码的哪一位 |
//! | `掩码表` | 掩码 `0..16` → 图集源里的第几张瓦片 |
//! | `变体` | 可选：整格瓦片抽手绘变体的图集与行列数 |
//!
//! 角位权默认 `[8, 4, 2, 1]`；原项目素材使用 `[8, 4, 1, 2]`，
//! 直接写在示例瓦片集里，而不是写死在代码里。
//!
//! # 文件格式
//!
//! ```ron
//! // 头部注释原样保留
//! (
//!     名称: "青岚山谷",
//!     瓦片宽: 16,
//!     瓦片高: 16,
//!     边距: 0,
//!     间距: 0,
//!     双网格: (
//!         启用: true,
//!         角位权: [8, 4, 1, 2],
//!         掩码表: [12, 13, 0, 3, 8, 1, 14, 5, 15, 4, 11, 2, 9, 10, 7, 6],
//!         变体: Some((图片: "assets/…/变体.png", 列: 7, 行: 6)),
//!     ),
//!     图集源: [
//!         (名称: "浅草", 图片: "assets/…/浅草.png", 列: 4, 行: 4,
//!          双网格: true, 绘制顺序: 2, 变体行: Some(2), 掩码表: None),
//!     ],
//! )
//! ```

use std::fmt::Write as _;

/// 单边瓦片像素上限。再大就不是瓦片了。
pub const 最大瓦片边长: u32 = 1024;

/// 瓦片集名字为空时，界面显示的名字。
pub const 未命名瓦片集: &str = "未命名瓦片集";

/// 新建瓦片集时写进文件头的说明。
pub fn 默认注释() -> String {
    [
        "// 瓦片集（微风编辑器）。",
        "//",
        "// 图集源：开发者自己导入的图片。列/行为 0 时按图片尺寸与瓦片尺寸自动计算。",
        "// 双网格：可选的四角自动瓦片；掩码表把 0..16 的四角掩码映射到源内瓦片序号。",
        "// 角位权顺序为 [左上, 右上, 右下, 左下]，默认 [8, 4, 2, 1]。",
    ]
    .join("\n")
}

/// 一张图集源：一张图片 + 瓦片网格参数。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct 图集源 {
    /// 显示名（调色板与图层里用）。
    pub 名称: String,
    /// 图片路径（项目相对路径）。
    pub 图片: String,
    /// 列数；0 表示按图片宽度自动计算。
    pub 列: u32,
    /// 行数；0 表示按图片高度自动计算。
    pub 行: u32,
    /// 该源是否参与四角自动瓦片。
    pub 双网格: bool,
    /// 同一图层内的绘制顺序（小的先画）。
    pub 绘制顺序: i32,
    /// 整格手绘变体在变体表里的行号；`None` 表示没有变体。
    pub 变体行: Option<u32>,
    /// 该源专用的掩码表；`None` 表示用瓦片集全局掩码表。
    pub 掩码表: Option<[u16; 16]>,
}

impl Default for 图集源 {
    fn default() -> Self {
        Self {
            名称: String::new(),
            图片: String::new(),
            列: 0,
            行: 0,
            双网格: false,
            绘制顺序: 0,
            变体行: None,
            掩码表: None,
        }
    }
}

impl 图集源 {
    /// 调色板与列表里显示的名字：没有名称时退回图片文件名。
    pub fn 显示名(&self) -> String {
        if !self.名称.trim().is_empty() {
            return self.名称.trim().to_string();
        }
        let 文件 = self.图片.rsplit('/').next().unwrap_or(&self.图片);
        文件.trim_end_matches(".png").to_string()
    }

    /// 按图片实际尺寸计算列数（`列` 为 0 时自动）。
    pub fn 计算列数(&self, 图片宽: u32, 瓦片宽: u32, 边距: u32, 间距: u32) -> u32 {
        if self.列 > 0 {
            return self.列;
        }
        自动数量(图片宽, 瓦片宽, 边距, 间距)
    }

    /// 按图片实际尺寸计算行数（`行` 为 0 时自动）。
    pub fn 计算行数(&self, 图片高: u32, 瓦片高: u32, 边距: u32, 间距: u32) -> u32 {
        if self.行 > 0 {
            return self.行;
        }
        自动数量(图片高, 瓦片高, 边距, 间距)
    }
}

/// 变体表：一张图片 + 网格尺寸。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct 变体表 {
    /// 图片路径（项目相对路径）。
    pub 图片: String,
    /// 列数。
    pub 列: u32,
    /// 行数。
    pub 行: u32,
}

impl Default for 变体表 {
    fn default() -> Self {
        Self {
            图片: String::new(),
            列: 1,
            行: 1,
        }
    }
}

/// 双网格规则。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct 双网格规则 {
    /// 是否启用。
    pub 启用: bool,
    /// 四角位权，顺序 `[左上, 右上, 右下, 左下]`。
    pub 角位权: [u8; 4],
    /// 掩码 `0..16` → 图集源内瓦片序号（行优先）。
    pub 掩码表: [u16; 16],
    /// 可选的手绘变体表。
    pub 变体: Option<变体表>,
}

impl Default for 双网格规则 {
    fn default() -> Self {
        Self {
            启用: true,
            角位权: [8, 4, 2, 1],
            掩码表: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
            变体: None,
        }
    }
}

/// 一套瓦片集。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct 瓦片集 {
    /// 文件头注释（不参与 RON）。
    #[serde(skip)]
    pub 注释: String,
    /// 显示名。
    pub 名称: String,
    /// 单张瓦片宽度（像素）。
    pub 瓦片宽: u32,
    /// 单张瓦片高度（像素）。
    pub 瓦片高: u32,
    /// 图片四周的边距（像素）。
    pub 边距: u32,
    /// 瓦片之间的间距（像素）。
    pub 间距: u32,
    /// 双网格规则。
    pub 双网格: 双网格规则,
    /// 图集源列表。
    pub 图集源: Vec<图集源>,
}

impl Default for 瓦片集 {
    fn default() -> Self {
        Self {
            注释: 默认注释(),
            名称: String::new(),
            瓦片宽: 16,
            瓦片高: 16,
            边距: 0,
            间距: 0,
            双网格: 双网格规则::default(),
            图集源: Vec::new(),
        }
    }
}

impl 瓦片集 {
    /// 新建一套空瓦片集。
    pub fn 新建(名称: impl Into<String>) -> Self {
        Self {
            名称: 名称.into(),
            ..Default::default()
        }
    }

    /// 显示名（空名时给一个稳定的占位）。
    pub fn 显示名(&self) -> String {
        if self.名称.trim().is_empty() {
            未命名瓦片集.to_string()
        } else {
            self.名称.trim().to_string()
        }
    }

    /// 取一个图集源。
    pub fn 取源(&self, 序号: usize) -> Option<&图集源> {
        self.图集源.get(序号)
    }

    /// 某个源生效的掩码表：源专用优先，否则用全局。
    pub fn 源的掩码表(&self, 序号: usize) -> &[u16; 16] {
        self.图集源
            .get(序号)
            .and_then(|源| 源.掩码表.as_ref())
            .unwrap_or(&self.双网格.掩码表)
    }

    /// 解析 `.tileset.ron` 文本。
    pub fn 解析(文本: &str) -> Result<Self, String> {
        let 注释 = 提取注释(文本);
        let mut 集: Self =
            ron::from_str(文本).map_err(|错误| format!("瓦片集 RON 解析失败：{错误}"))?;
        集.注释 = 注释;
        集.校验()?;
        Ok(集)
    }

    /// 校验并收拢明显越界的值。
    ///
    /// 返回的错误面向用户，会显示在瓦片地图面板里。
    pub fn 校验(&mut self) -> Result<(), String> {
        if self.瓦片宽 == 0 || self.瓦片高 == 0 {
            return Err("瓦片宽高不能为 0".to_string());
        }
        if self.瓦片宽 > 最大瓦片边长 || self.瓦片高 > 最大瓦片边长 {
            return Err(format!(
                "瓦片尺寸 {}×{} 过大（上限 {最大瓦片边长}）",
                self.瓦片宽, self.瓦片高
            ));
        }
        if self.图集源.len() > u16::MAX as usize {
            return Err("图集源数量超过 65535".to_string());
        }

        for (序号, 源) in self.图集源.iter_mut().enumerate() {
            if 源.图片.trim().is_empty() {
                return Err(format!("第 {序号} 个图集源没有设置图片"));
            }
            if 源.列 > 4096 || 源.行 > 4096 {
                return Err(format!("图集源「{}」的列/行过大", 源.显示名()));
            }
            if let Some(表) = &源.掩码表
                && let Some(越界) = 越界掩码(表, 源.列, 源.行)
            {
                return Err(format!(
                    "图集源「{}」的掩码表指向第 {越界} 张瓦片，超出 {}×{}",
                    源.显示名(),
                    源.列,
                    源.行
                ));
            }
            if 源.双网格 && !self.双网格.启用 {
                return Err(format!(
                    "图集源「{}」勾了双网格，但瓦片集没有启用双网格",
                    源.显示名()
                ));
            }
        }

        if self.双网格.启用 {
            if let Some(变体) = &self.双网格.变体 {
                if 变体.图片.trim().is_empty() {
                    return Err("变体表没有设置图片".to_string());
                }
                if 变体.列 == 0 || 变体.行 == 0 {
                    return Err("变体表的行列数不能为 0".to_string());
                }
                for 源 in &self.图集源 {
                    if let Some(行) = 源.变体行
                        && 行 >= 变体.行
                    {
                        return Err(format!(
                            "图集源「{}」的变体行 {行} 超出变体表行数 {}",
                            源.显示名(),
                            变体.行
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    /// 序列化为 `.tileset.ron` 文本。
    pub fn 序列化(&self) -> String {
        let mut 文本 = String::new();
        if !self.注释.trim().is_empty() {
            文本.push_str(self.注释.trim_end());
            文本.push('\n');
        }
        let _ = writeln!(文本, "(");
        let _ = writeln!(文本, "    名称: {},", 引号(&self.名称));
        let _ = writeln!(文本, "    瓦片宽: {},", self.瓦片宽);
        let _ = writeln!(文本, "    瓦片高: {},", self.瓦片高);
        let _ = writeln!(文本, "    边距: {},", self.边距);
        let _ = writeln!(文本, "    间距: {},", self.间距);
        文本.push_str("    双网格: (\n");
        let _ = writeln!(文本, "        启用: {},", self.双网格.启用);
        let _ = writeln!(文本, "        角位权: {},", 元组(&self.双网格.角位权));
        let _ = writeln!(文本, "        掩码表: {},", 元组(&self.双网格.掩码表));
        match &self.双网格.变体 {
            Some(变体) => {
                文本.push_str("        变体: Some((\n");
                let _ = writeln!(文本, "            图片: {},", 引号(&变体.图片));
                let _ = writeln!(文本, "            列: {},", 变体.列);
                let _ = writeln!(文本, "            行: {},", 变体.行);
                文本.push_str("        )),\n");
            }
            None => 文本.push_str("        变体: None,\n"),
        }
        文本.push_str("    ),\n");
        文本.push_str("    图集源: [\n");
        for 源 in &self.图集源 {
            文本.push_str("        (\n");
            let _ = writeln!(文本, "            名称: {},", 引号(&源.名称));
            let _ = writeln!(文本, "            图片: {},", 引号(&源.图片));
            let _ = writeln!(文本, "            列: {},", 源.列);
            let _ = writeln!(文本, "            行: {},", 源.行);
            let _ = writeln!(文本, "            双网格: {},", 源.双网格);
            let _ = writeln!(文本, "            绘制顺序: {},", 源.绘制顺序);
            match 源.变体行 {
                Some(行) => {
                    let _ = writeln!(文本, "            变体行: Some({行}),");
                }
                None => 文本.push_str("            变体行: None,\n"),
            }
            match &源.掩码表 {
                Some(表) => {
                    let _ = writeln!(文本, "            掩码表: Some({}),", 元组(表));
                }
                None => 文本.push_str("            掩码表: None,\n"),
            }
            文本.push_str("        ),\n");
        }
        文本.push_str("    ],\n");
        文本.push_str(")\n");
        文本
    }
}

/// 按图片尺寸自动算列/行：先减两边边距，再按「瓦片 + 间距」切。
fn 自动数量(图片边: u32, 瓦片边: u32, 边距: u32, 间距: u32) -> u32 {
    if 瓦片边 == 0 {
        return 1;
    }
    let 可用 = 图片边.saturating_sub(边距.saturating_mul(2));
    if 可用 < 瓦片边 {
        return 1;
    }
    (可用 + 间距) / (瓦片边 + 间距)
}

/// 掩码表里是否有越界序号。`列/行` 都为 0 表示只检查非负（无意义），
/// 返回第一个大于等于 `列×行` 的序号。
fn 越界掩码(表: &[u16; 16], 列: u32, 行: u32) -> Option<u16> {
    if 列 == 0 || 行 == 0 {
        return None;
    }
    let 总数 = 列.saturating_mul(行);
    if 总数 == 0 || 总数 > u16::MAX as u32 {
        return None;
    }
    表.iter().copied().find(|序号| *序号 as u32 >= 总数)
}

/// 取出 `(` 之前的注释行。
fn 提取注释(文本: &str) -> String {
    let 注释: Vec<&str> = 文本
        .lines()
        .take_while(|行| !行.trim_start().starts_with('('))
        .collect();
    let 合并 = 注释.join("\n");
    if 合并.trim().is_empty() {
        默认注释()
    } else {
        合并.trim_end().to_string()
    }
}

/// 固定长度数组在 RON 里写成元组 `(a, b, …)`，不是 `[a, b, …]`。
fn 元组<T: std::fmt::Display>(表: &[T]) -> String {
    let 项: Vec<String> = 表.iter().map(ToString::to_string).collect();
    format!("({})", 项.join(", "))
}

/// RON 字符串字面量：转义反斜杠与引号。
fn 引号(值: &str) -> String {
    let 转义 = 值.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{转义}\"")
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 默认瓦片集可以往返() {
        let mut 集 = 瓦片集::新建("测试集");
        集.图集源.push(图集源 {
            名称: "草地".to_string(),
            图片: "assets/tiles/grass.png".to_string(),
            列: 4,
            行: 4,
            双网格: true,
            绘制顺序: 1,
            变体行: Some(0),
            掩码表: None,
        });
        集.双网格.变体 = Some(变体表 {
            图片: "assets/tiles/variants.png".to_string(),
            列: 7,
            行: 6,
        });

        let 文本 = 集.序列化();
        let 再读 = 瓦片集::解析(&文本).expect("自己写出的瓦片集必须能读回");
        assert_eq!(再读, 集, "序列化 → 解析必须无损");
        assert!(文本.contains("// 瓦片集"), "文件头注释必须保留");
    }

    #[test]
    fn 空名字回退到图片名() {
        let 源 = 图集源 {
            图片: "assets/tiles/深色草地.png".to_string(),
            ..Default::default()
        };
        assert_eq!(源.显示名(), "深色草地");
        assert_eq!(瓦片集::default().显示名(), 未命名瓦片集);
    }

    #[test]
    fn 自动行列按边距与间距计算() {
        let 源 = 图集源::default();
        assert_eq!(源.计算列数(64, 16, 0, 0), 4);
        assert_eq!(源.计算行数(64, 16, 0, 0), 4);
        assert_eq!(源.计算列数(70, 16, 1, 1), 4, "边距各 1、间距 1");
        assert_eq!(源.计算列数(15, 16, 0, 0), 1, "不足一张也要返回 1");
    }

    #[test]
    fn 拒绝非法瓦片尺寸与越界掩码() {
        let mut 集 = 瓦片集::default();
        集.瓦片宽 = 0;
        assert!(集.校验().unwrap_err().contains("不能为 0"));

        let mut 集 = 瓦片集::default();
        集.图集源.push(图集源 {
            名称: "越界".to_string(),
            图片: "a.png".to_string(),
            列: 4,
            行: 4,
            双网格: true,
            掩码表: Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 99]),
            ..Default::default()
        });
        assert!(集.校验().unwrap_err().contains("超出"));
    }

    #[test]
    fn 源掩码表优先于全局() {
        let mut 集 = 瓦片集::default();
        集.双网格.掩码表 = [1; 16];
        集.图集源.push(图集源::default());
        assert_eq!(集.源的掩码表(0), &[1u16; 16]);

        集.图集源[0].掩码表 = Some([2; 16]);
        assert_eq!(集.源的掩码表(0), &[2u16; 16]);
    }
}
