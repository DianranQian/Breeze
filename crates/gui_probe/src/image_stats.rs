//! 截图解码与像素统计。
//!
//! 验收要回答的问题不是"这张图好不好看"，而是三个可以用数字回答的问题：
//!
//! 1. **这块区域真的被画了东西吗？** —— 唯一色数、主导色占比；
//! 2. **背景是不是当前主题该有的颜色？** —— 与证据里 `theme.panel_bg` 比对；
//! 3. **文字真的渲染出来了吗？** —— 区域内与主导亮度相差较大的像素数量。
//!
//! 第 3 条尤其重要：面板被画成一块纯色底、但文字因为字体/裁剪/尺寸问题
//! 完全没有出现，在第 1、2 条上都是"通过"的。实测中正是这种缺陷会让
//! 截图看起来"正常"而界面其实不可用。

use std::collections::HashMap;

/// 解码后的截图（RGBA8）。
pub struct 图像 {
    /// 宽（像素）。
    pub 宽: u32,
    /// 高（像素）。
    pub 高: u32,
    像素: Vec<[u8; 4]>,
}

/// 像素区域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct 矩形 {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl 矩形 {
    /// 右下角。
    pub fn 右(&self) -> i32 {
        self.x + self.w
    }
    /// 下边。
    pub fn 下(&self) -> i32 {
        self.y + self.h
    }
}

/// 一块区域的统计结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct 区域统计 {
    /// 有效像素数（落在图像内的部分）。
    pub 像素数: usize,
    /// 不同颜色数。
    pub 唯一色数: usize,
    /// 出现最多的颜色。
    pub 主导色: [u8; 4],
    /// 主导色占比（0..=1）。
    pub 主导占比: f32,
    /// 平均亮度（0..=255）。
    pub 均亮度: f32,
    /// 与主导亮度相差超过阈值的像素数（文字/图标/线条的代理指标）。
    pub 对比像素数: usize,
}

impl 区域统计 {
    /// 对比像素占有效像素的比例。
    pub fn 对比占比(&self) -> f32 {
        if self.像素数 == 0 {
            0.0
        } else {
            self.对比像素数 as f32 / self.像素数 as f32
        }
    }
}

impl 图像 {
    /// 读取一张 PNG/JPEG 等图片。
    pub fn 读取(路径: &std::path::Path) -> Result<Self, String> {
        let 图片 = image::ImageReader::open(路径)
            .map_err(|错误| format!("打不开图片 {}：{错误}", 路径.display()))?
            .decode()
            .map_err(|错误| format!("图片解码失败 {}：{错误}", 路径.display()))?;
        let rgba = 图片.to_rgba8();
        let (宽, 高) = rgba.dimensions();
        if 宽 == 0 || 高 == 0 {
            return Err(format!("图片尺寸退化：{宽}×{高}"));
        }
        let 像素 = rgba
            .pixels()
            .map(|像素| [像素[0], 像素[1], 像素[2], 像素[3]])
            .collect();
        Ok(Self { 宽, 高, 像素 })
    }

    /// 取一个像素。越界返回 `None`。
    pub fn 取(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x >= self.宽 as i32 || y >= self.高 as i32 {
            return None;
        }
        // 越界已在上面挡掉，索引必然有效。
        self.像素
            .get(y as usize * self.宽 as usize + x as usize)
            .copied()
    }

    /// 区域是否完整落在图像内。
    pub fn 包含(&self, 区域: 矩形) -> bool {
        区域.x >= 0
            && 区域.y >= 0
            && 区域.w > 0
            && 区域.h > 0
            && 区域.右() <= self.宽 as i32
            && 区域.下() <= self.高 as i32
    }

    /// 统计全图。
    pub fn 全图统计(&self) -> 区域统计 {
        self.区域统计(矩形 {
            x: 0,
            y: 0,
            w: self.宽 as i32,
            h: self.高 as i32,
        })
    }

    /// 统计一块区域。
    ///
    /// 越界部分会被裁掉，因此"区域贴到窗口边缘"时不会 panic，
    /// 但调用方应当先用 [`图像::包含`] 判断是否该判为证据不一致。
    pub fn 区域统计(&self, 区域: 矩形) -> 区域统计 {
        let x0 = 区域.x.max(0);
        let y0 = 区域.y.max(0);
        let x1 = 区域.右().min(self.宽 as i32);
        let y1 = 区域.下().min(self.高 as i32);
        if x1 <= x0 || y1 <= y0 {
            return 区域统计 {
                像素数: 0,
                唯一色数: 0,
                主导色: [0, 0, 0, 0],
                主导占比: 0.0,
                均亮度: 0.0,
                对比像素数: 0,
            };
        }

        let mut 直方图: HashMap<u32, usize> = HashMap::new();
        let mut 亮度和 = 0.0f64;
        let mut 像素数 = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let Some(像素) = self.取(x, y) else {
                    continue;
                };
                let 键 = (像素[0] as u32) << 16 | (像素[1] as u32) << 8 | 像素[2] as u32;
                *直方图.entry(键).or_insert(0) += 1;
                亮度和 += 亮度(像素) as f64;
                像素数 += 1;
            }
        }

        let 主导键 = 直方图
            .iter()
            .max_by_key(|(_, 数量)| **数量)
            .map(|(键, _)| *键)
            .unwrap_or(0);
        let 主导数量 = *直方图.get(&主导键).unwrap_or(&0);
        let 主导色 = [
            ((主导键 >> 16) & 0xFF) as u8,
            ((主导键 >> 8) & 0xFF) as u8,
            (主导键 & 0xFF) as u8,
            255,
        ];
        let 主导亮度 = 亮度(主导色);

        // 第二遍：数"与主导亮度差得够远"的像素。
        //
        // 阈值取 40：深色主题里文字是浅色（差 100+），
        // 浅色主题里文字是深色（差 100+），而分隔线/悬停底色这类
        // 细微差异只有 5~20，不会被误算成文字。
        let mut 对比像素数 = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let Some(像素) = self.取(x, y) else {
                    continue;
                };
                if (亮度(像素) - 主导亮度).abs() > 40.0 {
                    对比像素数 += 1;
                }
            }
        }

        区域统计 {
            像素数,
            唯一色数: 直方图.len(),
            主导色,
            主导占比: if 像素数 == 0 {
                0.0
            } else {
                主导数量 as f32 / 像素数 as f32
            },
            均亮度: if 像素数 == 0 {
                0.0
            } else {
                (亮度和 / 像素数 as f64) as f32
            },
            对比像素数,
        }
    }

    /// 数一块区域里"接近目标色"的像素个数（分量差最大值 ≤ 容差）。
    ///
    /// # 为什么需要它
    ///
    /// 证据里的矩形与配色是**应用自己报的**：它能证明"这一帧排版了这行字、
    /// 用的是这个颜色"，但证明不了"截图上真的出现了这个颜色的墨迹"。
    /// 徽标这类小字最容易出的正是后者 —— 被裁掉、被上一行压住、颜色被
    /// 主题覆盖掉，应用在证据里仍然一切正常。
    /// 拿截图数一次命中像素，才算把"渲染出了蓝色的嵌套徽标"钉成像素事实。
    ///
    /// 容差而不是精确匹配：文字走抗锯齿，笔画芯部才接近目标色，边缘是
    /// 与背景的混合色。调用方因此要判断的是"命中数够不够构成字形"，
    /// 而不是"每一像素都相等"。
    pub fn 接近色像素数(&self, 区域: 矩形, 目标色: [u8; 3], 容差: u8) -> usize {
        let 目标 = [目标色[0], 目标色[1], 目标色[2], 255];
        let x0 = 区域.x.max(0);
        let y0 = 区域.y.max(0);
        let x1 = 区域.右().min(self.宽 as i32);
        let y1 = 区域.下().min(self.高 as i32);
        let mut 命中 = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                if let Some(像素) = self.取(x, y)
                    && 颜色差(像素, 目标) <= 容差
                {
                    命中 += 1;
                }
            }
        }
        命中
    }
}

/// 感知亮度（Rec. 709）。
pub fn 亮度(像素: [u8; 4]) -> f32 {
    0.2126 * 像素[0] as f32 + 0.7152 * 像素[1] as f32 + 0.0722 * 像素[2] as f32
}

/// 两个颜色的分量差最大值。
pub fn 颜色差(甲: [u8; 4], 乙: [u8; 4]) -> u8 {
    let dr = 甲[0].abs_diff(乙[0]);
    let dg = 甲[1].abs_diff(乙[1]);
    let db = 甲[2].abs_diff(乙[2]);
    dr.max(dg).max(db)
}

#[cfg(test)]
mod 测试 {
    use super::*;

    #[test]
    fn 亮度对黑白正确() {
        assert!((亮度([0, 0, 0, 255]) - 0.0).abs() < 0.01);
        assert!((亮度([255, 255, 255, 255]) - 255.0).abs() < 0.01);
    }

    #[test]
    fn 颜色差取最大分量差() {
        assert_eq!(颜色差([10, 20, 30, 255], [10, 20, 30, 255]), 0);
        assert_eq!(颜色差([10, 20, 30, 255], [11, 25, 30, 255]), 5);
    }

    #[test]
    fn 区域越界时统计为零而不是崩溃() {
        let 图 = 构造纯色图(4, 4, [1, 2, 3, 255]);
        let 统计 = 图.区域统计(矩形 {
            x: -10,
            y: -10,
            w: 3,
            h: 3,
        });
        assert_eq!(统计.像素数, 0);
        assert_eq!(统计.唯一色数, 0);

        let 统计 = 图.区域统计(矩形 {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        });
        assert_eq!(统计.像素数, 16, "越界部分应被裁掉，只统计有效像素");
    }

    fn 构造纯色图(宽: u32, 高: u32, 色: [u8; 4]) -> 图像 {
        图像 {
            宽,
            高,
            像素: vec![色; (宽 * 高) as usize],
        }
    }
}
