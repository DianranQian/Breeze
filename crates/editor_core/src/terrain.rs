//! 中文名：地形数据
//! 地形数据：`.terrain.ron` 的解析、校验、编辑与写回。
//!
//! # 为什么这一层住在 `editor_core`
//!
//! 地形文件的读写既要给 `editor_app`（预览烘焙）用，也要给 `editor_ui`
//! （瓦片地图编辑面板）用。两边各写一份解析器，迟早会出现"面板能存、
//! 预览读不了"这种格式分叉。把格式定义放在不依赖 Bevy 与 egui 的
//! `editor_core` 里，两边的唯一真相就只有这一份。
//!
//! # 文件格式
//!
//! ```ron
//! // 头部注释会原样保留
//! (
//!     宽度: 48,
//!     高度: 32,
//!     地表: [
//!         "草草草……",
//!         // 每行恰好 宽度 个字符
//!     ],
//!     覆盖: [
//!         "....路路……",
//!     ],
//! )
//! ```
//!
//! 用字符而不是整数数组，是因为这一层直接给人看、给人改：
//! 一眼就能读出"这里是一片水、那里是一条路"，整数得先查表。
//!
//! # 字符约定
//!
//! | 层 | 字符 | 含义 |
//! | --- | --- | --- |
//! | 地表 | 草 | 浅草 |
//! | 地表 | 林 | 深草 |
//! | 地表 | 水 | 海水 |
//! | 地表 | 沙 | 沙滩 |
//! | 地表 | 山 | 山地 |
//! | 地表 | 丘 | 山丘 |
//! | 地表 | 土 | 湿润的土 |
//! | 地表 | 旱 | 干燥土地 |
//! | 覆盖 | `.` / `·` | 无 |
//! | 覆盖 | 路 | 道路 |
//! | 覆盖 | 田 | 耕地 |

use std::fmt::Write as _;

/// 允许出现在「地表」层的字符，顺序即界面调色板顺序。
pub const 地表字符集: [char; 8] = ['草', '林', '水', '沙', '山', '丘', '土', '旱'];

/// 新建网格时的默认地表字符。
pub const 默认地表字符: char = '草';

/// 允许出现在「覆盖」层的字符。
///
/// `·` 是历史文件里用过的"空"写法，读取时接受，写回时统一成 `.`。
pub const 覆盖字符集: [char; 4] = ['.', '·', '路', '田'];

/// 新建网格时的默认覆盖字符。
pub const 默认覆盖字符: char = '.';

/// 单边长度上限。
///
/// 与 `editor_app::tile_terrain` 的烘焙上限保持一致：1024 格 × 每格 8px
/// 正好是 8192，再大就会超过 GPU 纹理尺寸。
pub const 最大边长: usize = 1024;

/// 一张地形图：地表与覆盖两层字符网格。
///
/// `注释` 保存文件 `(` 之前的全部注释行。编辑后写回时原样放在文件头，
/// 用户手写的说明不会被编辑器抹掉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct 地形网格 {
    /// 文件头注释（不含结尾换行）。
    pub 注释: String,
    /// 宽度（列数）。
    pub 宽: usize,
    /// 高度（行数）。
    pub 高: usize,
    /// 地表层，`地表[y][x]`。
    pub 地表: Vec<Vec<char>>,
    /// 覆盖层，`覆盖[y][x]`。
    pub 覆盖: Vec<Vec<char>>,
}

/// 磁盘上的结构。字段名就是文件里的中文键，不能翻译。
#[derive(Debug, serde::Deserialize)]
struct 地形文件 {
    宽度: usize,
    高度: usize,
    地表: Vec<String>,
    #[serde(default)]
    覆盖: Vec<String>,
}

/// 新建地形时写进文件头的说明。
pub fn 默认注释() -> String {
    [
        "// 瓦片地形数据（微风编辑器）。",
        "//",
        "// 地表字符：草=浅草  林=深草  水=海水  沙=沙滩  山=山地  丘=山丘  土=湿润的土  旱=干燥土地",
        "// 覆盖字符：. = 无     路 = 道路   田 = 耕地",
    ]
    .join("\n")
}

/// 是否是合法的地表字符。
pub fn 是地表字符(字符: char) -> bool {
    地表字符集.contains(&字符)
}

/// 是否是合法的覆盖字符。
pub fn 是覆盖字符(字符: char) -> bool {
    覆盖字符集.contains(&字符)
}

impl 地形网格 {
    /// 新建一张全部为默认地表、无覆盖的地形。
    ///
    /// 尺寸会被夹到 `1..=最大边长`；调用方如果希望把越界当成错误，
    /// 用 [`地形网格::调整尺寸`]。
    pub fn 新建(宽: usize, 高: usize) -> Self {
        let 宽 = 宽.clamp(1, 最大边长);
        let 高 = 高.clamp(1, 最大边长);
        Self {
            注释: 默认注释(),
            宽,
            高,
            地表: vec![vec![默认地表字符; 宽]; 高],
            覆盖: vec![vec![默认覆盖字符; 宽]; 高],
        }
    }

    /// 解析 `.terrain.ron` 文本。
    ///
    /// 错误信息面向用户（会显示在瓦片地图面板与日志里），因此写清
    /// "哪一层、第几行、第几列、哪个字符"。
    pub fn 解析(文本: &str) -> Result<Self, String> {
        let 注释 = 提取注释(文本);
        let 文件: 地形文件 = ron::from_str(文本).map_err(|错误| format!("RON 解析失败：{错误}"))?;

        if 文件.宽度 == 0 || 文件.高度 == 0 {
            return Err("地形宽高不能为 0".to_string());
        }
        if 文件.宽度 > 最大边长 || 文件.高度 > 最大边长 {
            return Err(format!(
                "地形尺寸 {}×{} 过大（上限 {最大边长}×{最大边长}）",
                文件.宽度, 文件.高度
            ));
        }
        if 文件.地表.len() != 文件.高度 {
            return Err(format!(
                "地表行数 {} 与高度 {} 不一致",
                文件.地表.len(),
                文件.高度
            ));
        }

        let mut 地表 = Vec::with_capacity(文件.高度);
        for (行号, 行) in 文件.地表.iter().enumerate() {
            let 字符: Vec<char> = 行.chars().collect();
            if 字符.len() != 文件.宽度 {
                return Err(format!(
                    "地表第 {行号} 行宽 {} 与宽度 {} 不一致",
                    字符.len(),
                    文件.宽度
                ));
            }
            for (列号, 字) in 字符.iter().enumerate() {
                if !是地表字符(*字) {
                    return Err(format!("地表第 {行号} 行第 {列号} 列有未知字符 `{字}`"));
                }
            }
            地表.push(字符);
        }

        let 覆盖 = if 文件.覆盖.is_empty() {
            vec![vec![默认覆盖字符; 文件.宽度]; 文件.高度]
        } else {
            if 文件.覆盖.len() != 文件.高度 {
                return Err(format!(
                    "覆盖行数 {} 与高度 {} 不一致",
                    文件.覆盖.len(),
                    文件.高度
                ));
            }
            let mut 行表 = Vec::with_capacity(文件.高度);
            for (行号, 行) in 文件.覆盖.iter().enumerate() {
                let 字符: Vec<char> = 行.chars().collect();
                if 字符.len() != 文件.宽度 {
                    return Err(format!(
                        "覆盖第 {行号} 行宽 {} 与宽度 {} 不一致",
                        字符.len(),
                        文件.宽度
                    ));
                }
                for (列号, 字) in 字符.iter().enumerate() {
                    if !是覆盖字符(*字) {
                        return Err(format!("覆盖第 {行号} 行第 {列号} 列有未知字符 `{字}`"));
                    }
                }
                行表.push(字符);
            }
            行表
        };

        Ok(Self {
            注释,
            宽: 文件.宽度,
            高: 文件.高度,
            地表,
            覆盖,
        })
    }

    /// 序列化为 `.terrain.ron` 文本。
    ///
    /// 每行一格串、带缩进，diff 时只变真正改动的行。
    pub fn 序列化(&self) -> String {
        let mut 文本 = String::new();
        if !self.注释.trim().is_empty() {
            文本.push_str(self.注释.trim_end());
            文本.push('\n');
        }
        let _ = writeln!(文本, "(");
        let _ = writeln!(文本, "    宽度: {},", self.宽);
        let _ = writeln!(文本, "    高度: {},", self.高);
        文本.push_str("    地表: [\n");
        for 行 in &self.地表 {
            let _ = writeln!(文本, "        \"{}\",", 行.iter().collect::<String>());
        }
        文本.push_str("    ],\n");
        文本.push_str("    覆盖: [\n");
        for 行 in &self.覆盖 {
            let 规范化: String = 行
                .iter()
                .map(|字| {
                    if *字 == '·' {
                        默认覆盖字符
                    } else {
                        *字
                    }
                })
                .collect();
            let _ = writeln!(文本, "        \"{规范化}\",");
        }
        文本.push_str("    ],\n");
        文本.push_str(")\n");
        文本
    }

    /// 读取一格地表字符（越界返回 `None`）。
    pub fn 取地表(&self, x: usize, y: usize) -> Option<char> {
        self.地表.get(y).and_then(|行| 行.get(x)).copied()
    }

    /// 读取一格覆盖字符（越界返回 `None`）。
    pub fn 取覆盖(&self, x: usize, y: usize) -> Option<char> {
        self.覆盖.get(y).and_then(|行| 行.get(x)).copied()
    }

    /// 设置一格地表；字符非法或越界返回 `false`。
    pub fn 设置地表(&mut self, x: usize, y: usize, 字符: char) -> bool {
        if !是地表字符(字符) {
            return false;
        }
        let Some(格) = self.地表.get_mut(y).and_then(|行| 行.get_mut(x)) else {
            return false;
        };
        *格 = 字符;
        true
    }

    /// 设置一格覆盖；字符非法或越界返回 `false`。
    pub fn 设置覆盖(&mut self, x: usize, y: usize, 字符: char) -> bool {
        if !是覆盖字符(字符) {
            return false;
        }
        let Some(格) = self.覆盖.get_mut(y).and_then(|行| 行.get_mut(x)) else {
            return false;
        };
        *格 = 字符;
        true
    }

    /// 调整尺寸：保留左上角内容，新增格子填 `地表填充` / 无覆盖。
    pub fn 调整尺寸(
        &mut self, 宽: usize, 高: usize, 地表填充: char
    ) -> Result<(), String> {
        if 宽 == 0 || 高 == 0 {
            return Err("地形宽高不能为 0".to_string());
        }
        if 宽 > 最大边长 || 高 > 最大边长 {
            return Err(format!(
                "地形尺寸 {宽}×{高} 过大（上限 {最大边长}×{最大边长}）"
            ));
        }
        let 填充 = if 是地表字符(地表填充) {
            地表填充
        } else {
            默认地表字符
        };

        let mut 新地表 = vec![vec![填充; 宽]; 高];
        let mut 新覆盖 = vec![vec![默认覆盖字符; 宽]; 高];
        let 复制宽 = self.宽.min(宽);
        let 复制高 = self.高.min(高);
        for y in 0..复制高 {
            for x in 0..复制宽 {
                新地表[y][x] = self.地表[y][x];
                新覆盖[y][x] = self.覆盖[y][x];
            }
        }
        self.宽 = 宽;
        self.高 = 高;
        self.地表 = 新地表;
        self.覆盖 = 新覆盖;
        Ok(())
    }

    /// 是否全部为默认地表、无覆盖（用于"空地图"提示，不参与保存）。
    pub fn 是空白(&self) -> bool {
        self.地表
            .iter()
            .all(|行| 行.iter().all(|字| *字 == 默认地表字符))
            && self
                .覆盖
                .iter()
                .all(|行| 行.iter().all(|字| *字 == 默认覆盖字符))
    }
}

/// 取出 `(` 之前的注释行。
///
/// 用"第一行以 `(` 开头的行"定位，而不是找第一个 `(` 字符：
/// 注释里可能出现括号（例如"（上限 1024）"），按字符找会截错。
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

#[cfg(test)]
mod 测试 {
    use super::*;

    /// 仓库里的真实示例地形必须能解析，并能无损往返。
    #[test]
    fn 示例地形可以解析并往返() {
        let 仓库根 = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let 路径 = 仓库根.join("examples/2d_demo/assets/xiuxian/地形/qinglan_valley.terrain.ron");
        let 文本 = std::fs::read_to_string(&路径).expect("示例地形应当存在");
        let 网格 = 地形网格::解析(&文本).expect("示例地形应当能解析");

        assert_eq!((网格.宽, 网格.高), (48, 32));
        assert!(网格.注释.contains("青岚山谷"), "文件头注释必须被保留");

        let 写回 = 网格.序列化();
        let 再读 = 地形网格::解析(&写回).expect("写回的内容必须还能读");
        assert_eq!(再读, 网格, "序列化 → 解析必须无损");
    }

    #[test]
    fn 拒绝未知字符与尺寸不符() {
        let 文本 = r#"(
            宽度: 2,
            高度: 1,
            地表: ["草?"],
        )"#;
        let 错误 = 地形网格::解析(文本).expect_err("未知字符必须被拒绝");
        assert!(错误.contains("未知字符"), "{错误}");

        let 文本 = r#"(
            宽度: 3,
            高度: 1,
            地表: ["草草"],
        )"#;
        let 错误 = 地形网格::解析(文本).expect_err("行宽不符必须被拒绝");
        assert!(错误.contains("行宽"), "{错误}");
    }

    #[test]
    fn 覆盖层可省略且会补成无() {
        let 文本 = r#"(
            宽度: 2,
            高度: 2,
            地表: ["草水", "沙山"],
        )"#;
        let 网格 = 地形网格::解析(文本).expect("覆盖层可省略");
        assert_eq!(网格.覆盖.len(), 2);
        assert_eq!(网格.取覆盖(0, 0), Some('.'));
        assert_eq!(网格.取覆盖(1, 1), Some('.'));
    }

    #[test]
    fn 调整尺寸保留左上角并填充新格() {
        let mut 网格 = 地形网格::新建(2, 2);
        网格.设置地表(0, 0, '水');
        网格.设置覆盖(1, 1, '路');

        网格.调整尺寸(4, 3, '沙').expect("调整尺寸应当成功");
        assert_eq!((网格.宽, 网格.高), (4, 3));
        assert_eq!(网格.取地表(0, 0), Some('水'), "原有内容必须保留");
        assert_eq!(网格.取覆盖(1, 1), Some('路'), "覆盖层也要保留");
        assert_eq!(网格.取地表(3, 2), Some('沙'), "新增格子用填充字符");
        assert_eq!(网格.取覆盖(3, 2), Some('.'), "新增格子无覆盖");
    }

    #[test]
    fn 越界与非法写入被拒绝() {
        let mut 网格 = 地形网格::新建(2, 2);
        assert!(!网格.设置地表(2, 0, '草'), "越界应当返回 false");
        assert!(!网格.设置地表(0, 0, '?'), "非法字符应当返回 false");
        assert!(!网格.设置覆盖(0, 0, '?'), "非法覆盖字符应当返回 false");
        assert_eq!(网格.取地表(0, 0), Some('草'), "失败的写入不能改内容");
    }

    #[test]
    fn 空覆盖写法_·_会规范化成句点() {
        let 文本 = r#"(
            宽度: 1,
            高度: 1,
            地表: ["草"],
            覆盖: ["·"],
        )"#;
        let 网格 = 地形网格::解析(文本).expect("历史写法应当能读");
        assert!(网格.序列化().contains("\".\""), "写回时应统一成 `.`");
    }
}
