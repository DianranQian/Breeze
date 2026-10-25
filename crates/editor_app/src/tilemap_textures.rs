//! 中文名：瓦片图集纹理
//! 把瓦片集引用的图片解码成 egui 纹理，供瓦片地图面板的调色板与画布直接画瓦片。
//!
//! # 为什么按图片缓存，而不是按瓦片
//!
//! 一张 4×4 图集里有 16 张瓦片。若为每张瓦片单独建纹理，切图成本高、
//! 显存也多 16 倍。这里只把整张图片注册成一张 egui 纹理，面板用 UV
//! 矩形取其中一格 —— 与 Godot 的 atlas 取样方式一致。
//!
//! 图片路径来自**面板里正在编辑的瓦片集**，不是硬编码目录。文件修改时间
//! 变了才重新解码；面板关掉（瓦片集为 `None`）时纹理随之释放。
//!
//! # 为什么用 `ctx.load_texture` 而不是 Bevy 图片资源
//!
//! 面板需要的是 `TextureHandle`：它带图片尺寸，能算 UV 与调色板行列。
//! `EguiContexts::add_image` 只给 `TextureId`，还得另存尺寸与图片句柄。
//! 瓦片集图片都不大（通常几十 KB），直接交给 egui 管理生命周期更简单。

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use bevy::prelude::*;
use bevy_egui::EguiContexts;

use crate::plugin::{EditorStateHolder, UiStateHolder};

/// 瓦片集图片的 egui 纹理缓存。
#[derive(Resource, Default)]
pub struct 瓦片图集纹理 {
    /// 项目相对图片路径 → egui 纹理。
    pub 纹理: BTreeMap<String, egui::TextureHandle>,
    /// 上次载入时的文件修改时间（用于判断是否需要重新解码）。
    时间: BTreeMap<String, Option<SystemTime>>,
}

impl 瓦片图集纹理 {
    /// 取一张图集图片的纹理。
    pub fn 取(&self, 图片: &str) -> Option<&egui::TextureHandle> {
        self.纹理.get(图片)
    }
}

/// 每帧按面板里的瓦片集同步纹理。
pub fn 同步瓦片图集纹理(
    mut 上下文: EguiContexts,
    编辑器: Res<EditorStateHolder>,
    界面: Res<UiStateHolder>,
    mut 缓存: ResMut<瓦片图集纹理>,
) {
    let Ok(egui上下文) = 上下文.ctx_mut() else {
        return;
    };

    // 没有打开项目时，项目相对路径无法解析，先清空。
    let Some(项目) = 编辑器.状态.项目.as_ref() else {
        缓存.纹理.clear();
        缓存.时间.clear();
        return;
    };

    let mut 需要: Vec<String> = Vec::new();
    if let Some(集) = &界面.状态.瓦片地图.瓦片集 {
        for 源 in &集.图集源 {
            需要.push(源.图片.clone());
        }
        if let Some(变体) = &集.双网格.变体 {
            需要.push(变体.图片.clone());
        }
    }
    需要.sort();
    需要.dedup();

    for 图片 in &需要 {
        let 路径 = 根路径(项目.根目录(), 图片);
        let 时间 = 修改时间(&路径);
        if let Some(旧) = 缓存.时间.get(图片)
            && 旧 == &时间
            && 缓存.纹理.contains_key(图片)
        {
            continue;
        }

        match image::open(&路径).map(|图| 图.to_rgba8()) {
            Ok(图) => {
                let (宽, 高) = 图.dimensions();
                let 颜色 = egui::ColorImage::from_rgba_unmultiplied(
                    [宽 as usize, 高 as usize],
                    &图.into_raw(),
                );
                // 像素瓦片放大时要保持硬边，不能用线性采样。
                let 纹理 =
                    egui上下文.load_texture(图片.clone(), 颜色, egui::TextureOptions::NEAREST);
                缓存.纹理.insert(图片.clone(), 纹理);
            }
            Err(错误) => {
                warn!("[瓦片地图] 读取图集 {图片} 失败：{错误}");
                缓存.纹理.remove(图片);
            }
        }
        缓存.时间.insert(图片.clone(), 时间);
    }

    // 面板不再引用（或已经删掉源）的图片要释放。
    缓存.纹理.retain(|图片, _| 需要.contains(图片));
    缓存.时间.retain(|图片, _| 需要.contains(图片));
}

/// 项目相对路径 → 本机路径。
fn 根路径(根: &Path, 相对: &str) -> std::path::PathBuf {
    根.join(相对.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// 文件修改时间。
fn 修改时间(路径: &Path) -> Option<SystemTime> {
    std::fs::metadata(路径)
        .ok()
        .and_then(|元| 元.modified().ok())
}
