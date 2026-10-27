//! 材质资源：项目内可复用的 `.material.ron` 定义。
//!
//! # 为什么是项目数据，而不是 Bevy 的材质资产
//!
//! Bevy 的 `StandardMaterial` 是运行时组件，由 `AssetServer` 按 `Handle` 加载，
//! 编辑器改一个字段就要重传一次资产；而且它没法带着「这条引用属于哪个项目文件」
//! 的信息被撤销栈记录。材质文档走的是与场景文档一样的路：项目相对路径 + RON
//! 文本，能进 Git、能 diff、能按字段撤销。投影到 `StandardMaterial` 是渲染层
//! 的事，这个结构不掺和。
//!
//! # 为什么复用 RON 而不是另立格式
//!
//! 编码配置直接沿用 [`crate::animation`] 与 `scene_doc::storage` 那套
//! （`struct_names(false)` + 一行一字段 + `\n`），材质文件因此与场景、预制件、
//! 动画长得一样，能共用同一个语法高亮与同一套「读文本 → 解码 → 版本校验」流程，
//! 不再引入第二套序列化框架。
//!
//! # 贴图为什么存 [`资源引用值`] 而不是裸路径
//!
//! 材质里的贴图槽和 Inspector 里的资源字段是同一件事：路径给人读，稳定标识
//! 让文件在外部改名/移动后还能回连。存裸路径会让材质成为引用追踪网里的一个洞
//! （`asset_refs` 与查找引用都看不到它）。
//!
//! # 版本策略
//!
//! 与动画文档一致：`格式版本` 只向前拒绝。读到比编辑器更新的版本时明确报错，
//! 而不是尽力解析出半个材质再写回去，把用户的新字段静默丢掉。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::i18n::语言;
use scene_doc::error::{场景结果, 场景错误};
use scene_doc::property::{资源引用值, 颜色值};

/// 材质文档的当前格式版本。
pub const 材质格式版本: u32 = 1;

fn 默认材质格式版本() -> u32 {
    材质格式版本
}

/// 一份材质定义。
///
/// 字段字典：
/// - `格式版本`：缺省按当前版本读取，手写的第一个文件不必带这一行；
/// - `名称`：显示名。空串由 [`材质资源::显示名`] 回退到「未命名材质」，
///   与动画文档同一口径；
/// - `着色颜色`：基础色，与贴图**相乘**，所以「不上贴图」是白而不是黑；
/// - `贴图`：贴图槽的资源引用；未引用时不写进文件；
/// - `粗糙度` / `金属度`：`0.0..=1.0` 的 PBR 参数；
/// - `自发光`：自发光色，**全黑表示不发光**。用颜色而不是「强度 + 颜色」两份
///   字段，是为了让 Inspector 只有可编辑的一栏，且默认值天然是无发光；
/// - `双面`：是否双面渲染。默认关闭，因为双面会关掉背面剔除、白付一半绘制开销。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 材质资源 {
    /// 格式版本。
    #[serde(default = "默认材质格式版本")]
    pub 格式版本: u32,
    /// 显示名。
    #[serde(default)]
    pub 名称: String,
    /// 基础着色颜色（与贴图相乘）。
    #[serde(default = "默认着色颜色")]
    pub 着色颜色: 颜色值,
    /// 贴图引用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 贴图: Option<资源引用值>,
    /// 粗糙度，`0.0..=1.0`。
    #[serde(default = "默认粗糙度")]
    pub 粗糙度: f32,
    /// 金属度，`0.0..=1.0`。
    #[serde(default = "默认金属度")]
    pub 金属度: f32,
    /// 自发光颜色；全黑表示不发光。
    #[serde(default = "默认自发光")]
    pub 自发光: 颜色值,
    /// 是否双面渲染。
    #[serde(default)]
    pub 双面: bool,
}

fn 默认着色颜色() -> 颜色值 {
    颜色值::白()
}

fn 默认粗糙度() -> f32 {
    0.5
}

fn 默认金属度() -> f32 {
    0.0
}

fn 默认自发光() -> 颜色值 {
    颜色值::srgb(0.0, 0.0, 0.0, 1.0)
}

/// 把可能是 NaN / inf 的标量收进 `0.0..=1.0`。
///
/// 不直接用 `f32::clamp`：它只要任一参数是 NaN 就返回 NaN，于是用户在
/// Inspector 里误输入一次，非法值就会一路带到编码，最终写出一个每次
/// 加载都不同的材质文件。
fn 有限收拢(值: f32) -> f32 {
    if 值.is_finite() {
        值.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

impl 材质资源 {
    /// 新建一份中性材质：白色不透明、无贴图、中等粗糙度、非金属、不发光、单面。
    ///
    /// 粗糙度取 `0.5` 而不是 `0.0`：新建的文件几乎总要被调，而 `0.0` 会先把
    /// 整个物体变成完美镜面，让人误判成渲染坏了。
    pub fn 新建(名称: impl Into<String>) -> Self {
        Self {
            格式版本: 材质格式版本,
            名称: 名称.into(),
            着色颜色: 默认着色颜色(),
            贴图: None,
            粗糙度: 默认粗糙度(),
            金属度: 默认金属度(),
            自发光: 默认自发光(),
            双面: false,
        }
    }

    /// 显示名。空名回退到「未命名材质」。
    pub fn 显示名(&self, 语言: 语言) -> String {
        if self.名称.trim().is_empty() {
            语言.取("material_untitled").to_string()
        } else {
            self.名称.clone()
        }
    }

    /// 把所有可编辑分量收拢到有效范围，供属性编辑器写回时复用。
    ///
    /// 不在解码里做：文件里越界的值应当原样呈现给用户，让他看到自己写了什么，
    /// 而不是被编辑器偷偷改掉后再报告「无变更」。
    pub fn 收拢范围(&mut self) {
        self.着色颜色.收拢分量();
        self.自发光.收拢分量();
        self.粗糙度 = 有限收拢(self.粗糙度);
        self.金属度 = 有限收拢(self.金属度);
    }

    /// 编码为 RON 文本（美化输出，便于进 Git）。
    pub fn 编码(&self) -> 场景结果<String> {
        let 配置 = ron::ser::PrettyConfig::default()
            .struct_names(false)
            .depth_limit(32)
            .new_line("\n".to_string());
        ron::ser::to_string_pretty(self, 配置)
            .map_err(|错误| 场景错误::编码失败(错误.to_string()))
    }

    /// 从文本解码。版本过新时拒绝，而不是尽力解析。
    pub fn 解码(文本: &str, 路径: &Path) -> 场景结果<Self> {
        let 文档: Self = ron::from_str(文本).map_err(|来源| 场景错误::格式解析 {
            路径: 路径.to_path_buf(),
            来源: Box::new(来源),
        })?;
        if 文档.格式版本 > 材质格式版本 {
            return Err(场景错误::版本过新 {
                文件版本: 文档.格式版本,
                支持版本: 材质格式版本,
            });
        }
        Ok(文档)
    }
}

#[cfg(test)]
mod 测试 {
    use std::path::PathBuf;

    use super::*;
    use scene_doc::path::项目相对路径;
    use uuid::Uuid;

    /// 一份手写的材质文件：字段顺序刻意与结构体声明不同，
    /// 顺便证明解码依赖字段名而不是位置。
    ///
    /// `Some(...)` 是仓库既有的 RON 写法（见 `examples/2d_demo/project.ron`
    /// 与场景文件），可选项不写成 `Some` 反而不是 RON 的合法形。
    const 示例文本: &str = r#"(
    名称: "石板路",
    双面: true,
    金属度: 0.1,
    粗糙度: 0.85,
    着色颜色: (红: 0.6, 绿: 0.62, 蓝: 0.66, 透明度: 1.0, 是_srgb: true),
    自发光: (红: 0.0, 绿: 0.0, 蓝: 0.0, 透明度: 1.0, 是_srgb: true),
    贴图: Some((
        路径: "assets/贴图/石板.png",
        唯一标识: Some("6a1f8b2c-0000-4000-8000-000000000001"),
    )),
)"#;

    #[test]
    fn 材质文件可以解码出全部字段() {
        let 材质 = 材质资源::解码(示例文本, &PathBuf::from("石板路.material.ron"))
            .expect("手写的材质文件应当能解码");
        assert_eq!(材质.名称, "石板路");
        assert_eq!(材质.格式版本, 材质格式版本, "缺省版本按当前版本读取");
        assert!((材质.粗糙度 - 0.85).abs() < 1e-6);
        assert!((材质.金属度 - 0.1).abs() < 1e-6);
        assert!(材质.双面);
        assert_eq!(材质.着色颜色.红, 0.6);
        let 贴图 = 材质.贴图.expect("贴图槽应当有引用");
        assert_eq!(
            贴图.路径,
            项目相对路径::解析("assets/贴图/石板.png").unwrap()
        );
        assert_eq!(
            贴图.唯一标识.map(|标识| 标识.to_string()).as_deref(),
            Some("6a1f8b2c-0000-4000-8000-000000000001"),
            "贴图引用必须带上稳定标识，否则材质会成为引用追踪网里的洞"
        );
    }

    #[test]
    fn 编码再解码得到同一份材质() {
        let 原 = 材质资源::解码(示例文本, &PathBuf::from("x.material.ron")).unwrap();
        let 文本 = 原.编码().expect("编码");
        let 回 = 材质资源::解码(&文本, &PathBuf::from("x.material.ron")).expect("回读");
        assert_eq!(回, 原, "编码应当无损：材质文件要能进 Git 并被手改");
        assert!(
            文本.contains("粗糙度: 0.85"),
            "RON 文本要一行一字段，方便 diff：{文本}"
        );
    }

    #[test]
    fn 缺省字段落在中性值上() {
        let 材质 = 材质资源::解码("(名称: \"最简\")", &PathBuf::from("x.material.ron")).unwrap();
        assert_eq!(材质.着色颜色, 颜色值::白(), "不上贴图时基础色是白，不是黑");
        assert!((材质.粗糙度 - 0.5).abs() < 1e-6);
        assert_eq!(材质.金属度, 0.0);
        assert_eq!(材质.自发光.红, 0.0);
        assert!(!材质.双面, "双面默认关闭：它会关掉背面剔除");
        assert!(材质.贴图.is_none());
    }

    #[test]
    fn 版本过新的材质被拒绝而不是尽力解析() {
        let 结果 = 材质资源::解码(
            "(格式版本: 99, 名称: \"未来的材质\")",
            &PathBuf::from("x.material.ron"),
        );
        let 错误 = 结果.expect_err("比编辑器新的材质文件应当拒绝");
        assert!(
            matches!(
                错误,
                场景错误::版本过新 {
                    文件版本: 99, ..
                }
            ),
            "错误应当带上文件版本号：{错误:?}"
        );
    }

    #[test]
    fn 空名回退到未命名材质显示名() {
        let mut 材质 = 材质资源::新建("unused");
        材质.名称 = "   ".to_string();
        for 语种 in 语言::全部 {
            let 名 = 材质.显示名(语种);
            assert!(!名.trim().is_empty(), "空名也要有可显示的默认名");
            assert_ne!(
                名, "material_untitled",
                "词条必须已在词表里，否则界面会把键名本身漏给用户"
            );
        }
        assert_eq!(材质资源::新建("石板路").显示名(语言::中文), "石板路");
    }

    #[test]
    fn 保存到项目后可以原样读回() {
        let 目录 = tempfile::tempdir().expect("临时目录");
        let 项目 =
            crate::project::项目描述::创建(目录.path().join("项目"), "材质读写").expect("创建项目");
        let 路径 = 项目相对路径::解析("assets/materials/石板.material.ron").unwrap();
        let 贴图路径 = 项目相对路径::解析("assets/贴图/石板.png").unwrap();

        let mut 材质 = 材质资源::新建("石板路");
        材质.贴图 = Some(资源引用值::完整(贴图路径, Uuid::nil()));
        材质.粗糙度 = 0.85;
        材质.双面 = true;

        项目
            .保存材质(&路径, &材质)
            .expect("保存材质：materials 子目录不存在时也要建齐，新建材质是常见第一步");
        assert!(
            项目
                .根目录()
                .join("assets/materials/石板.material.ron")
                .is_file(),
            "落盘路径必须是项目相对路径拼出来的那个"
        );
        assert_eq!(
            项目.材质列表(),
            vec![路径.clone()],
            "写进 assets/ 的材质要被扫描认出来"
        );

        let 读回 = 项目.读取材质(&路径).expect("读取材质");
        assert_eq!(读回, 材质, "读写必须无损：材质文件是要进 Git 被人手改的");
    }

    #[test]
    fn 收拢范围把越界分量压回有效区间() {
        let mut 材质 = 材质资源::新建("x");
        材质.粗糙度 = 4.0;
        材质.金属度 = f32::NAN;
        材质.着色颜色 = 颜色值::srgb(2.0, -1.0, 0.5, 3.0);
        材质.收拢范围();
        assert_eq!(材质.粗糙度, 1.0);
        assert_eq!(材质.金属度, 0.0, "NaN 不该被留成 NaN");
        assert_eq!((材质.着色颜色.红, 材质.着色颜色.绿), (1.0, 0.0));
    }
}
