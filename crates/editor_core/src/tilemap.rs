//! 中文名：瓦片地图
//! 瓦片地图：`.tilemap.ron` 的解析、校验、编辑与写回。
//!
//! # 与瓦片集的分工
//!
//! 瓦片集描述"有哪些瓦片、怎么自动拼接"，瓦片地图只描述"哪一格放了什么"。
//! 地图引用瓦片集（项目相对路径），因此换一套瓦片集就能换整张地图的画风。
//!
//! # 单元格编码
//!
//! 每格存一个 `u32`：
//!
//! ```text
//! 0                                   = 空格
//! (图集源序号 + 1) << 16 | 瓦片序号   = 有瓦片
//! ```
//!
//! 用整数而不是字符串，是因为地图动辄上万格：RON 里每格一个结构体会让
//! 文件大十倍，读写也慢。图层仍然按行写，diff 时只变真正改动的行。
//!
//! 双网格图层的单元格只用到"图集源序号"——四角掩码由渲染器根据邻居算，
//! 瓦片序号不参与。普通图层的单元格则两者都用。
//!
//! # 文件格式
//!
//! ```ron
//! // 头部注释原样保留
//! (
//!     瓦片集: "assets/…/青岚山谷.tileset.ron",
//!     宽: 48,
//!     高度: 32,
//!     图层: [
//!         (名称: "地表", 双网格: true, 单元格: [
//!             [65537, 65537, 0, …],
//!         ]),
//!     ],
//! )
//! ```

use std::fmt::Write as _;

/// 单边格数上限。与 `terrain::最大边长` 一致，避免两张地图规则分叉。
pub const 最大边长: usize = 1024;

/// 一张地图最多多少图层。
pub const 最大图层数: usize = 32;

/// 新建地图时写进文件头的说明。
pub fn 默认注释() -> String {
    [
        "// 瓦片地图（微风编辑器）。",
        "//",
        "// 瓦片集：项目相对的 .tileset.ron 路径。",
        "// 单元格：0 表示空；(图集源序号 + 1) << 16 | 瓦片序号 表示有瓦片。",
        "// 双网格图层只使用图集源序号，四角掩码由渲染器按邻居计算。",
    ]
    .join("\n")
}

/// 一个瓦片引用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct 瓦片格 {
    /// 图集源序号。
    pub 源: u16,
    /// 源内瓦片序号（行优先）。
    pub 瓦片: u16,
}

impl 瓦片格 {
    /// 新建一个瓦片引用。
    pub const fn 新建(源: u16, 瓦片: u16) -> Self {
        Self { 源, 瓦片 }
    }

    /// 编码成单元格值。
    pub const fn 编码(self) -> u32 {
        (((self.源 as u32) + 1) << 16) | (self.瓦片 as u32 & 0xFFFF)
    }

    /// 从单元格值解码；`0` 返回 `None`。
    pub const fn 解码(值: u32) -> Option<Self> {
        if 值 == 0 {
            return None;
        }
        Some(Self {
            源: ((值 >> 16) as u16).wrapping_sub(1),
            瓦片: (值 & 0xFFFF) as u16,
        })
    }
}

/// 一个图层。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct 地图图层 {
    /// 图层名（图层面板里显示）。
    pub 名称: String,
    /// 是否按双网格规则自动拼接。
    pub 双网格: bool,
    /// 单元格，`单元格[y][x]`。
    pub 单元格: Vec<Vec<u32>>,
}

impl Default for 地图图层 {
    fn default() -> Self {
        Self {
            名称: String::new(),
            双网格: false,
            单元格: Vec::new(),
        }
    }
}

impl 地图图层 {
    /// 新建一个指定尺寸的空图层。
    pub fn 新建(名称: impl Into<String>, 宽: usize, 高: usize, 双网格: bool) -> Self {
        Self {
            名称: 名称.into(),
            双网格,
            单元格: vec![vec![0; 宽]; 高],
        }
    }

    /// 取一格。
    pub fn 取格(&self, x: usize, y: usize) -> Option<瓦片格> {
        瓦片格::解码(*self.单元格.get(y)?.get(x)?)
    }

    /// 设置一格；越界返回 `false`。
    pub fn 设置格(&mut self, x: usize, y: usize, 格: Option<瓦片格>) -> bool {
        let Some(行) = self.单元格.get_mut(y) else {
            return false;
        };
        let Some(槽) = 行.get_mut(x) else {
            return false;
        };
        *槽 = 格.map(瓦片格::编码).unwrap_or(0);
        true
    }

    /// 调整尺寸，保留左上角。
    pub fn 调整尺寸(&mut self, 宽: usize, 高: usize) {
        let mut 新 = vec![vec![0; 宽]; 高];
        let 复制宽 = self.单元格.first().map(Vec::len).unwrap_or(0).min(宽);
        let 复制高 = self.单元格.len().min(高);
        for y in 0..复制高 {
            for x in 0..复制宽 {
                新[y][x] = self.单元格[y][x];
            }
        }
        self.单元格 = 新;
    }
}

/// 一张瓦片地图。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct 瓦片地图 {
    /// 文件头注释（不参与 RON）。
    #[serde(skip)]
    pub 注释: String,
    /// 瓦片集路径（项目相对路径）。
    pub 瓦片集: String,
    /// 宽度（列数）。
    pub 宽: usize,
    /// 高度（行数）。文件里的键是 `高度`，与旧地形格式保持一致。
    #[serde(rename = "高度")]
    pub 高: usize,
    /// 图层，先画的在前。
    pub 图层: Vec<地图图层>,
}

impl Default for 瓦片地图 {
    fn default() -> Self {
        Self {
            注释: 默认注释(),
            瓦片集: String::new(),
            宽: 48,
            高: 32,
            图层: vec![地图图层::新建("地表", 48, 32, true)],
        }
    }
}

impl 瓦片地图 {
    /// 新建一张指定尺寸的地图（一个双网格图层）。
    pub fn 新建(宽: usize, 高: usize) -> Self {
        let 宽 = 宽.clamp(1, 最大边长);
        let 高 = 高.clamp(1, 最大边长);
        Self {
            注释: 默认注释(),
            瓦片集: String::new(),
            宽,
            高,
            图层: vec![地图图层::新建("地表", 宽, 高, true)],
        }
    }

    /// 解析 `.tilemap.ron` 文本。
    pub fn 解析(文本: &str) -> Result<Self, String> {
        let 注释 = 提取注释(文本);
        let mut 地图: Self =
            ron::from_str(文本).map_err(|错误| format!("瓦片地图 RON 解析失败：{错误}"))?;
        地图.注释 = 注释;
        地图.校验()?;
        Ok(地图)
    }

    /// 校验尺寸与每个图层的行宽。
    pub fn 校验(&self) -> Result<(), String> {
        if self.宽 == 0 || self.高 == 0 {
            return Err("地图宽高不能为 0".to_string());
        }
        if self.宽 > 最大边长 || self.高 > 最大边长 {
            return Err(format!(
                "地图尺寸 {}×{} 过大（上限 {最大边长}×{最大边长}）",
                self.宽, self.高
            ));
        }
        if self.图层.is_empty() {
            return Err("地图至少要有一个图层".to_string());
        }
        if self.图层.len() > 最大图层数 {
            return Err(format!("图层数量超过上限 {最大图层数}"));
        }
        for 图层 in &self.图层 {
            if 图层.单元格.len() != self.高 {
                return Err(format!(
                    "图层「{}」有 {} 行，与地图高度 {} 不一致",
                    图层.名称,
                    图层.单元格.len(),
                    self.高
                ));
            }
            for (行号, 行) in 图层.单元格.iter().enumerate() {
                if 行.len() != self.宽 {
                    return Err(format!(
                        "图层「{}」第 {行号} 行有 {} 格，与地图宽度 {} 不一致",
                        图层.名称,
                        行.len(),
                        self.宽
                    ));
                }
            }
        }
        Ok(())
    }

    /// 序列化为 `.tilemap.ron` 文本。
    pub fn 序列化(&self) -> String {
        let mut 文本 = String::new();
        if !self.注释.trim().is_empty() {
            文本.push_str(self.注释.trim_end());
            文本.push('\n');
        }
        let _ = writeln!(文本, "(");
        let _ = writeln!(文本, "    瓦片集: {},", 引号(&self.瓦片集));
        let _ = writeln!(文本, "    宽: {},", self.宽);
        let _ = writeln!(文本, "    高度: {},", self.高);
        文本.push_str("    图层: [\n");
        for 图层 in &self.图层 {
            文本.push_str("        (\n");
            let _ = writeln!(文本, "            名称: {},", 引号(&图层.名称));
            let _ = writeln!(文本, "            双网格: {},", 图层.双网格);
            文本.push_str("            单元格: [\n");
            for 行 in &图层.单元格 {
                文本.push_str("                [");
                for (列, 值) in 行.iter().enumerate() {
                    if 列 > 0 {
                        文本.push_str(", ");
                    }
                    let _ = write!(文本, "{值}");
                }
                文本.push_str("],\n");
            }
            文本.push_str("            ],\n");
            文本.push_str("        ),\n");
        }
        文本.push_str("    ],\n");
        文本.push_str(")\n");
        文本
    }

    /// 取一格。
    pub fn 取格(&self, 图层: usize, x: usize, y: usize) -> Option<瓦片格> {
        self.图层.get(图层)?.取格(x, y)
    }

    /// 设置一格。
    pub fn 设置格(&mut self, 图层: usize, x: usize, y: usize, 格: Option<瓦片格>) -> bool {
        let Some(层) = self.图层.get_mut(图层) else {
            return false;
        };
        层.设置格(x, y, 格)
    }

    /// 调整尺寸，所有图层一起变。
    pub fn 调整尺寸(&mut self, 宽: usize, 高: usize) -> Result<(), String> {
        if 宽 == 0 || 高 == 0 {
            return Err("地图宽高不能为 0".to_string());
        }
        if 宽 > 最大边长 || 高 > 最大边长 {
            return Err(format!(
                "地图尺寸 {宽}×{高} 过大（上限 {最大边长}×{最大边长}）"
            ));
        }
        for 层 in &mut self.图层 {
            层.调整尺寸(宽, 高);
        }
        self.宽 = 宽;
        self.高 = 高;
        Ok(())
    }

    /// 新增图层（尺寸与地图一致）。
    pub fn 新增图层(
        &mut self, 名称: impl Into<String>, 双网格: bool
    ) -> Result<usize, String> {
        if self.图层.len() >= 最大图层数 {
            return Err(format!("图层数量超过上限 {最大图层数}"));
        }
        let 名称 = 名称.into();
        self.图层
            .push(地图图层::新建(名称, self.宽, self.高, 双网格));
        Ok(self.图层.len() - 1)
    }

    /// 删除图层；至少保留一个。
    pub fn 删除图层(&mut self, 序号: usize) -> bool {
        if self.图层.len() <= 1 || 序号 >= self.图层.len() {
            return false;
        }
        self.图层.remove(序号);
        true
    }

    /// 上移/下移图层。`方向` 为负表示往前（先画）。
    pub fn 移动图层(&mut self, 序号: usize, 方向: i32) -> bool {
        let 目标 = 序号 as i32 + 方向;
        if 序号 >= self.图层.len() || 目标 < 0 || 目标 as usize >= self.图层.len() {
            return false;
        }
        self.图层.swap(序号, 目标 as usize);
        true
    }

    /// 是否全部为空。
    pub fn 是空白(&self) -> bool {
        self.图层
            .iter()
            .all(|层| 层.单元格.iter().all(|行| 行.iter().all(|值| *值 == 0)))
    }
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

/// RON 字符串字面量。
fn 引号(值: &str) -> String {
    let 转义 = 值.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{转义}\"")
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 单元格编码往返() {
        let 格 = 瓦片格::新建(3, 7);
        assert_eq!(瓦片格::解码(格.编码()), Some(格));
        assert_eq!(瓦片格::解码(0), None);
        assert_eq!(瓦片格::新建(0, 0).编码(), 1 << 16);
    }

    #[test]
    fn 新建地图可以往返() {
        let mut 地图 = 瓦片地图::新建(4, 3);
        地图.瓦片集 = "assets/tiles/demo.tileset.ron".to_string();
        地图.设置格(0, 1, 2, Some(瓦片格::新建(2, 5)));
        地图.新增图层("覆盖", false).expect("可以加图层");
        地图.设置格(1, 0, 0, Some(瓦片格::新建(1, 15)));

        let 文本 = 地图.序列化();
        let 再读 = 瓦片地图::解析(&文本).expect("自己写出的地图必须能读回");
        assert_eq!(再读, 地图, "序列化 → 解析必须无损");
        assert_eq!(再读.取格(0, 1, 2), Some(瓦片格::新建(2, 5)));
        assert_eq!(再读.取格(1, 0, 0), Some(瓦片格::新建(1, 15)));
    }

    #[test]
    fn 拒绝行宽不符与空图层() {
        let 文本 = r#"(
            宽: 2,
            高度: 2,
            图层: [(名称: "地表", 双网格: true, 单元格: [[0, 0]])],
        )"#;
        let 错误 = 瓦片地图::解析(文本).expect_err("行数不符必须被拒绝");
        assert!(错误.contains("行"), "{错误}");

        let 文本 = r#"(
            宽: 2,
            高度: 2,
            图层: [],
        )"#;
        let 错误 = 瓦片地图::解析(文本).expect_err("空图层必须被拒绝");
        assert!(错误.contains("图层"), "{错误}");
    }

    #[test]
    fn 调整尺寸保留左上角() {
        let mut 地图 = 瓦片地图::新建(2, 2);
        地图.设置格(0, 1, 1, Some(瓦片格::新建(0, 3)));
        地图.调整尺寸(4, 4).expect("调整尺寸应当成功");
        assert_eq!((地图.宽, 地图.高), (4, 4));
        assert_eq!(地图.取格(0, 1, 1), Some(瓦片格::新建(0, 3)));
        assert_eq!(地图.取格(0, 3, 3), None);
    }

    #[test]
    fn 图层不能删到空() {
        let mut 地图 = 瓦片地图::新建(2, 2);
        assert!(!地图.删除图层(0), "最后一个图层不能被删除");
        let 序号 = 地图.新增图层("覆盖", false).unwrap();
        assert!(地图.删除图层(序号));
        assert_eq!(地图.图层.len(), 1);
    }
}
