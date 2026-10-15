//! 场景文档：编辑器的持久化数据结构。
//!
//! # 数据模型的选择：为什么是"扁平实体表 + 父指针"
//!
//! 直觉上场景是棵树，于是很容易写成嵌套的 `children: Vec<场景实体>`。
//! 但编辑器里最常见的三种操作在嵌套结构上都是**最坏情况**：
//! 拖放改变层级要跨树搬移整个子树、重命名要定位并重建路径、
//! 撤销要整份深拷贝两棵树做 diff。嵌套结构还让"按 ID 找实体"退化成 O(n) 遍历。
//!
//! 这里改用 **扁平存储 + 父指针 + 顺序表**，也就是 Godot 内部实际采用的思路：
//!
//! ```text
//! entities: { 实体ID -> 场景实体 { 父实体: Option<实体ID>, 次序: u32, ... } }
//! roots:    [实体ID, ...]            // 顶层实体的顺序
//! ```
//!
//! 代价是文档里的层级是**隐式**的，必须靠 [`场景文档::校验`] 保证一致性
//! （父实体存在、无环、`次序` 在兄弟间唯一）。这个代价是值得的：
//! 校验只需跑一次 O(n) 遍历，而换取的是所有编辑操作都变成常数或线性时间。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{场景错误, 场景结果};
use crate::path::{项目相对路径, 资源唯一标识};
use crate::property::属性值;

/// 本编辑器支持的最高场景格式版本。
///
/// 版本号策略：
/// * **加字段**（新增可选组件字段、新增元数据）→ 保持版本号，靠 `serde(default)` 兼容；
/// * **改语义**（字段单位变化、层级表示变化）→ 递增版本号并写升级函数；
/// * 读到**更高**版本 → 直接拒绝加载（见 [`场景错误::版本过新`]），
///   而不是尽力解析，否则保存时会静默丢掉新版数据。
pub const 当前格式版本: u32 = 1;

/// 实体在场景中的稳定身份。
///
/// 用 UUID 而不是"场景内序号"：序号在删除中间实体后会整体位移，
/// 导致撤销栈、资源引用、外部工具里的引用全部错位。
/// UUID 让实体的身份与它在层级中的位置彻底解耦。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct 实体标识(pub Uuid);

impl 实体标识 {
    /// 生成一个新的实体标识。
    pub fn 新建() -> Self {
        Self(Uuid::new_v4())
    }

    /// 由已有 UUID 构造。
    pub fn 从(值: Uuid) -> Self {
        Self(值)
    }
}

impl std::fmt::Display for 实体标识 {
    fn fmt(&self, 格式: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, 格式)
    }
}

/// 变换数据：位置、旋转（四元数）、缩放。
///
/// 旋转存四元数而不是欧拉角：欧拉角存盘后，不同解析顺序会得到不同姿态，
/// 而且无法表示万向节锁以外的连续插值。Inspector 里仍以角度显示，
/// 转换发生在 UI 层，文档里始终是唯一的规范表示。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct 变换数据 {
    /// 平移，单位与项目约定一致（2D 用 x/y，z 用于图层顺序）。
    pub 平移: (f32, f32, f32),
    /// 旋转四元数 `(x, y, z, w)`。
    pub 旋转: (f32, f32, f32, f32),
    /// 缩放。
    pub 缩放: (f32, f32, f32),
}

impl Default for 变换数据 {
    fn default() -> Self {
        Self::单位()
    }
}

impl 变换数据 {
    /// 单位变换。
    pub const fn 单位() -> Self {
        Self {
            平移: (0.0, 0.0, 0.0),
            旋转: (0.0, 0.0, 0.0, 1.0),
            缩放: (1.0, 1.0, 1.0),
        }
    }

    /// 是否为单位变换。
    pub fn 是单位(&self) -> bool {
        self == &Self::单位()
    }

    /// 把四元数归一化。零四元数会退化为单位四元数。
    ///
    /// 这一步是必须的：用户通过 Inspector 输入角度、或将来从外部导入时，
    /// 累积误差会让四元数逐渐偏离单位长度，最终造成可见的形变扭曲。
    pub fn 归一化旋转(&mut self) -> bool {
        let (x, y, z, w) = self.旋转;
        let 模长平方 = x * x + y * y + z * z + w * w;
        if !模长平方.is_finite() || 模长平方 <= f32::EPSILON {
            self.旋转 = (0.0, 0.0, 0.0, 1.0);
            return true;
        }
        let 模长 = 模长平方.sqrt();
        if (模长 - 1.0).abs() < 1e-6 {
            return false;
        }
        self.旋转 = (x / 模长, y / 模长, z / 模长, w / 模长);
        true
    }
}

/// 一个组件在文档中的存在形式。
///
/// `类型名` + `属性` 的组合刻意保持"弱类型"：
/// 文档不需要知道 Rust 类型，只需要知道组件怎么称呼、字段有哪些值。
/// 真正的类型含义由 [`crate::registry::组件注册表`] 在加载时赋予。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 组件数据 {
    /// 组件类型名，与 [`crate::registry::组件类型::组件名`] 一致。
    pub 类型名: String,
    /// 属性名到属性值的映射。用 `BTreeMap` 保证保存顺序稳定。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub 属性: BTreeMap<String, 属性值>,
}

impl 组件数据 {
    /// 由类型名与属性表构造。
    pub fn 新建(类型名: impl Into<String>, 属性: BTreeMap<String, 属性值>) -> Self {
        Self {
            类型名: 类型名.into(),
            属性,
        }
    }

    /// 只有类型名、无属性的占位组件（例如标记组件）。
    pub fn 标记(类型名: impl Into<String>) -> Self {
        Self {
            类型名: 类型名.into(),
            属性: BTreeMap::new(),
        }
    }
}

/// 场景中的资源引用记录。
///
/// 集中登记所有被引用的资源，好处有三：
/// 1. 资源被改名/删除时，可以 O(1) 找出所有受影响实体并给出提示；
/// 2. 加载场景时可以一次性校验引用是否存在，而不是等运行时 asset 加载失败；
/// 3. 打包时可以据此裁剪出真正需要的资源集合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct 资源引用 {
    /// 资源路径。
    pub 路径: 项目相对路径,
    /// 资源唯一标识（若已生成）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 唯一标识: Option<资源唯一标识>,
}

/// 场景文档的元数据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 场景元数据 {
    /// 场景标识，用于项目内的场景互引用。
    #[serde(default = "实体标识::新建")]
    pub 场景标识: 实体标识,
    /// 场景显示名。
    pub 场景名: String,
    /// 保存时间（Unix 秒）。缺失时不报错，只表示"非本编辑器保存"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 保存时间: Option<u64>,
    /// 编辑器写入的自定义键值，供插件使用。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub 自定义: BTreeMap<String, 属性值>,
}

impl 场景元数据 {
    /// 以一个名字构造元数据。
    pub fn 新建(场景名: impl Into<String>) -> Self {
        Self {
            场景标识: 实体标识::新建(),
            场景名: 场景名.into(),
            保存时间: None,
            自定义: BTreeMap::new(),
        }
    }
}

impl Default for 场景元数据 {
    fn default() -> Self {
        Self::新建("未命名场景")
    }
}

/// 场景中的一个实体。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 场景实体 {
    /// 稳定身份。
    pub 标识: 实体标识,
    /// 显示名，编辑器里允许重名（唯一性由标识保证）。
    pub 名称: String,
    /// 父实体。`None` 表示顶层实体。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 父实体: Option<实体标识>,
    /// 在兄弟中的顺序。必须唯一，否则渲染顺序不确定。
    #[serde(default)]
    pub 次序: u32,
    /// 本地变换。`None` 表示该实体不参与变换（纯逻辑实体）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub 变换: Option<变换数据>,
    /// 组件列表。用 `Vec` 而非 `Map` 以保留用户添加顺序。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub 组件: Vec<组件数据>,

    // ---- 以下为编辑器专有元数据，运行时不需要 ----
    /// 在场景树中是否隐藏（仅影响编辑器显示，不影响游戏运行）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub 编辑器隐藏: bool,
    /// 是否锁定，锁定后不可在视口中被选中或拖动。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub 编辑器锁定: bool,
    /// 场景树展开状态。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub 编辑器展开: bool,
}

impl 场景实体 {
    /// 创建一个顶层、无组件的空实体。
    pub fn 新建(名称: impl Into<String>) -> Self {
        Self {
            标识: 实体标识::新建(),
            名称: 名称.into(),
            父实体: None,
            次序: 0,
            变换: Some(变换数据::单位()),
            组件: Vec::new(),
            编辑器隐藏: false,
            编辑器锁定: false,
            编辑器展开: false,
        }
    }

    /// 查找一个组件。
    pub fn 取组件(&self, 类型名: &str) -> Option<&组件数据> {
        self.组件.iter().find(|组件| 组件.类型名 == 类型名)
    }

    /// 查找一个组件（可变）。
    pub fn 取组件可变(&mut self, 类型名: &str) -> Option<&mut 组件数据> {
        self.组件.iter_mut().find(|组件| 组件.类型名 == 类型名)
    }

    /// 该实体是否包含指定组件。
    pub fn 有组件(&self, 类型名: &str) -> bool {
        self.取组件(类型名).is_some()
    }

    /// 添加组件。若已存在同类型组件则返回 `false`（不覆盖）。
    pub fn 加组件(&mut self, 组件: 组件数据) -> bool {
        if self.有组件(&组件.类型名) {
            return false;
        }
        self.组件.push(组件);
        true
    }

    /// 移除组件，返回是否存在。
    pub fn 删组件(&mut self, 类型名: &str) -> bool {
        let 原长度 = self.组件.len();
        self.组件.retain(|组件| 组件.类型名 != 类型名);
        self.组件.len() != 原长度
    }
}

/// 文档校验报告。
///
/// 校验**不拒绝加载**，只报告问题并允许修复：一个层级轻微损坏的场景，
/// 让用户能打开、看到问题、手工修好，比直接拒绝打开有用得多。
/// 唯一的例外是格式版本过新，那属于"语义未知"，必须拒绝。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct 文档校验报告 {
    /// 引用了不存在父实体的实体。
    pub 悬空父实体: Vec<实体标识>,
    /// 位于环中的实体。
    pub 循环层级: Vec<实体标识>,
    /// 同一父实体下 `次序` 重复的实体。
    pub 次序冲突: Vec<实体标识>,
    /// 引用了未注册组件的实体与组件类型名。
    pub 未注册组件: Vec<(实体标识, String)>,
    /// 未登记在资源表中的资源引用。
    pub 未登记资源: Vec<项目相对路径>,
}

impl 文档校验报告 {
    /// 是否完全没有任何问题。
    pub fn 无问题(&self) -> bool {
        self.悬空父实体.is_empty()
            && self.循环层级.is_empty()
            && self.次序冲突.is_empty()
            && self.未注册组件.is_empty()
            && self.未登记资源.is_empty()
    }

    /// 面向用户的问题摘要。
    pub fn 摘要(&self) -> String {
        if self.无问题() {
            return "场景校验通过".to_string();
        }
        let mut 条目 = Vec::new();
        if !self.悬空父实体.is_empty() {
            条目.push(format!("{} 个实体的父实体不存在", self.悬空父实体.len()));
        }
        if !self.循环层级.is_empty() {
            条目.push(format!("{} 个实体处于循环层级中", self.循环层级.len()));
        }
        if !self.次序冲突.is_empty() {
            条目.push(format!("{} 个实体的次序重复", self.次序冲突.len()));
        }
        if !self.未注册组件.is_empty() {
            条目.push(format!("{} 个组件未注册", self.未注册组件.len()));
        }
        if !self.未登记资源.is_empty() {
            条目.push(format!("{} 个资源引用未登记", self.未登记资源.len()));
        }
        条目.join("；")
    }
}

/// 场景文档。
///
/// 这是编辑器与磁盘之间的唯一权威表示。游戏运行时数据**不会**写回这里，
/// 除非用户显式执行"把运行时修改应用到场景"操作。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct 场景文档 {
    /// 格式版本。放在文件最前面，便于工具快速判断能否处理。
    pub 格式版本: u32,
    /// 元数据。
    pub 元数据: 场景元数据,
    /// 实体表。
    #[serde(default)]
    pub 实体表: BTreeMap<实体标识, 场景实体>,
    /// 顶层实体的顺序。
    #[serde(default)]
    pub 顶层顺序: Vec<实体标识>,
    /// 被引用的资源表。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub 资源表: BTreeMap<项目相对路径, 资源引用>,
}

impl Default for 场景文档 {
    fn default() -> Self {
        Self::新建("未命名场景")
    }
}

impl 场景文档 {
    /// 创建一个空场景。
    pub fn 新建(场景名: impl Into<String>) -> Self {
        Self {
            格式版本: 当前格式版本,
            元数据: 场景元数据::新建(场景名),
            实体表: BTreeMap::new(),
            顶层顺序: Vec::new(),
            资源表: BTreeMap::new(),
        }
    }

    /// 实体数量。
    pub fn 实体数量(&self) -> usize {
        self.实体表.len()
    }

    /// 查询实体。
    pub fn 取(&self, 标识: 实体标识) -> Option<&场景实体> {
        self.实体表.get(&标识)
    }

    /// 查询实体（可变）。
    pub fn 取可变(&mut self, 标识: 实体标识) -> Option<&mut 场景实体> {
        self.实体表.get_mut(&标识)
    }

    /// 判断实体是否存在。
    pub fn 存在(&self, 标识: 实体标识) -> bool {
        self.实体表.contains_key(&标识)
    }

    /// 直接插入一个实体。
    ///
    /// 这是**低层**接口：它不做层级一致性维护。调用方应优先使用
    /// [`场景文档::生成实体`] / [`场景文档::设为子实体`] 等高层操作。
    pub fn 插入实体(&mut self, 实体: 场景实体) {
        let 标识 = 实体.标识;
        let 父 = 实体.父实体;
        self.实体表.insert(标识, 实体);
        match 父 {
            None => {
                if !self.顶层顺序.contains(&标识) {
                    self.顶层顺序.push(标识);
                }
            }
            Some(父标识) => {
                // 若该实体同时出现在顶层顺序里，需要清理掉，避免双重归属。
                self.顶层顺序.retain(|项| 项 != &标识);
                self.修正次序(Some(父标识));
            }
        }
    }

    /// 在指定父实体下生成一个新实体，自动分配 `次序`。
    ///
    /// 返回新实体的标识。
    pub fn 生成实体(&mut self, 名称: impl Into<String>, 父实体: Option<实体标识>) -> 实体标识 {
        let mut 实体 = 场景实体::新建(名称);
        实体.父实体 = 父实体;
        实体.次序 = self.下一个次序(父实体);
        let 标识 = 实体.标识;
        self.插入实体(实体);
        标识
    }

    /// 计算某个父实体下可用的下一个 `次序`。
    pub fn 下一个次序(&self, 父实体: Option<实体标识>) -> u32 {
        self.子实体列表(父实体)
            .into_iter()
            .filter_map(|标识| self.取(标识).map(|实体| 实体.次序))
            .max()
            .map_or(0, |最大| 最大.saturating_add(1))
    }

    /// 某个父实体的直接子实体列表，**按 `次序` 排序**。
    pub fn 子实体列表(&self, 父实体: Option<实体标识>) -> Vec<实体标识> {
        let mut 列表: Vec<(u32, 实体标识)> = match 父实体 {
            None => self
                .顶层顺序
                .iter()
                .filter_map(|标识| self.取(*标识).map(|实体| (实体.次序, *标识)))
                .collect(),
            Some(父标识) => self
                .实体表
                .values()
                .filter(|实体| 实体.父实体 == Some(父标识))
                .map(|实体| (实体.次序, 实体.标识))
                .collect(),
        };
        列表.sort();
        列表.into_iter().map(|(_, 标识)| 标识).collect()
    }

    /// 把某个父实体下的 `次序` 重新编号为连续的 `0..n`。
    ///
    /// 删除实体后会留下序号空洞，虽然不影响正确性，但会让"上移/下移"
    /// 这类基于序号的操作用户体验变差，所以在结构变更后统一整理。
    pub fn 修正次序(&mut self, 父实体: Option<实体标识>) {
        for (新次序, 标识) in self.子实体列表(父实体).into_iter().enumerate() {
            if let Some(实体) = self.实体表.get_mut(&标识) {
                实体.次序 = 新次序 as u32;
            }
        }
    }

    /// 删除一个实体及其整个子树。返回被删除的所有实体标识。
    ///
    /// 删除子树而不是"把子实体提升到父级"：后者会让用户在删除一个容器时
    /// 意外得到一堆散落在场景根的实体，且撤销后层级无法复原。
    pub fn 删除实体(&mut self, 标识: 实体标识) -> Vec<实体标识> {
        let 待删 = self.收集子树(标识);
        let 父 = self.取(标识).and_then(|实体| 实体.父实体);
        for 项 in &待删 {
            self.实体表.remove(项);
        }
        self.顶层顺序.retain(|项| !待删.contains(项));
        if let Some(父标识) = 父 {
            self.修正次序(Some(父标识));
        } else {
            self.修正次序(None);
        }
        self.清理资源表();
        待删
    }

    /// 收集某个实体及其所有后代的标识（含自身）。
    ///
    /// 使用显式栈而不是递归，避免深场景导致栈溢出。
    pub fn 收集子树(&self, 根: 实体标识) -> Vec<实体标识> {
        let mut 结果 = Vec::new();
        let mut 已访问 = HashSet::new();
        let mut 栈 = vec![根];
        while let Some(当前) = 栈.pop() {
            if !已访问.insert(当前) {
                continue;
            }
            结果.push(当前);
            for 子 in self.子实体列表(Some(当前)) {
                栈.push(子);
            }
        }
        结果
    }

    /// 判断 `可能的后代` 是否位于 `祖先` 的子树中（用于阻止成环的重设父实体）。
    pub fn 是后代(&self, 祖先: 实体标识, 可能的后代: 实体标识) -> bool {
        let mut 当前 = self.取(可能的后代).and_then(|实体| 实体.父实体);
        let mut 已访问 = HashSet::new();
        while let Some(标识) = 当前 {
            if 标识 == 祖先 {
                return true;
            }
            if !已访问.insert(标识) {
                // 文档已损坏（存在环），停止向上走以免死循环。
                return false;
            }
            当前 = self.取(标识).and_then(|实体| 实体.父实体);
        }
        false
    }

    /// 改变实体的父实体与次序。
    ///
    /// 会拒绝把实体挂到自己的后代下（那会造出环并使整棵子树脱离场景）。
    pub fn 设为子实体(
        &mut self,
        标识: 实体标识,
        新父: Option<实体标识>,
        新次序: u32,
    ) -> 场景结果<()> {
        if !self.存在(标识) {
            return Err(场景错误::非法(format!("实体 {标识} 不存在")));
        }
        if let Some(父) = 新父 {
            if 父 == 标识 {
                return Err(场景错误::非法("不能把实体设为自己的子实体"));
            }
            if !self.存在(父) {
                return Err(场景错误::非法(format!("目标父实体 {父} 不存在")));
            }
            if self.是后代(标识, 父) {
                return Err(场景错误::非法(
                    "不能把实体挂到自己的后代下，那会造成循环层级",
                ));
            }
        }

        let 旧父 = self.取(标识).and_then(|实体| 实体.父实体);

        // 先摘出来，避免在中间状态下出现"同时属于两个父级"。
        self.顶层顺序.retain(|项| 项 != &标识);
        if let Some(实体) = self.实体表.get_mut(&标识) {
            实体.父实体 = 新父;
            实体.次序 = 新次序;
        }

        // 在新父下腾出位置，再整理两侧的次序。
        let 新父子列表 = self.子实体列表(新父);
        if let Some(当前下标) = 新父子列表.iter().position(|项| 项 == &标识) {
            self.把子实体移到(新父, 当前下标, 新次序 as usize);
        }
        if let Some(旧父标识) = 旧父
            && Some(旧父标识) != 新父 {
                self.修正次序(Some(旧父标识));
            }
        if 新父.is_none() && 旧父.is_some() {
            self.修正次序(None);
        }
        Ok(())
    }

    /// 在某个父实体下，把第 `从` 个位置的子实体移动到第 `到` 个位置。
    fn 把子实体移到(&mut self, 父实体: Option<实体标识>, 从: usize, 到: usize) {
        let mut 列表 = self.子实体列表(父实体);
        if 从 >= 列表.len() {
            return;
        }
        let 项 = 列表.remove(从);
        let 目标 = 到.min(列表.len());
        列表.insert(目标, 项);
        for (次序, 标识) in 列表.into_iter().enumerate() {
            if let Some(实体) = self.实体表.get_mut(&标识) {
                实体.次序 = 次序 as u32;
            }
        }
    }

    /// 在兄弟间上移一位。已在最前时返回 `false`。
    pub fn 上移(&mut self, 标识: 实体标识) -> bool {
        let Some(父) = self.取(标识).and_then(|实体| 实体.父实体) else {
            // 顶层实体走同样的逻辑，只是父为 None，需要区分"不存在"与"顶层"。
            return self.在兄弟间移动(标识, None, -1);
        };
        self.在兄弟间移动(标识, Some(父), -1)
    }

    /// 在兄弟间下移一位。已在最后时返回 `false`。
    pub fn 下移(&mut self, 标识: 实体标识) -> bool {
        match self.取(标识).and_then(|实体| 实体.父实体) {
            Some(父) => self.在兄弟间移动(标识, Some(父), 1),
            None => self.在兄弟间移动(标识, None, 1),
        }
    }

    /// 通用兄弟间移动。`方向` 为 `-1` 上移、`1` 下移。
    fn 在兄弟间移动(&mut self, 标识: 实体标识, 父: Option<实体标识>, 方向: i32) -> bool {
        let 列表 = self.子实体列表(父);
        let Some(下标) = 列表.iter().position(|项| 项 == &标识) else {
            return false;
        };
        let 目标 = 下标 as i32 + 方向;
        if 目标 < 0 || 目标 as usize >= 列表.len() {
            return false;
        }
        self.把子实体移到(父, 下标, 目标 as usize);
        true
    }

    /// 重命名实体。
    pub fn 重命名(&mut self, 标识: 实体标识, 新名称: impl Into<String>) -> bool {
        match self.实体表.get_mut(&标识) {
            Some(实体) => {
                实体.名称 = 新名称.into();
                true
            }
            None => false,
        }
    }

    /// 复制一个实体（不含子树）为新的顶层实体，返回新标识。
    ///
    /// 名称自动加 `_副本` 后缀并去重，避免场景里出现两个同名实体
    /// 让用户在场景树里无法区分。
    pub fn 复制实体(&mut self, 源: 实体标识) -> Option<实体标识> {
        let 源实体 = self.取(源)?.clone();
        let 新名称 = self.去重名称(&format!("{}_副本", 源实体.名称));
        let mut 新实体 = 源实体;
        新实体.标识 = 实体标识::新建();
        新实体.名称 = 新名称;
        新实体.父实体 = None;
        新实体.次序 = self.下一个次序(None);
        let 新标识 = 新实体.标识;
        self.插入实体(新实体);
        Some(新标识)
    }

    /// 生成一个场景内不重复的名称。
    pub fn 去重名称(&self, 期望: &str) -> String {
        let 已用: BTreeSet<&str> = self.实体表.values().map(|实体| 实体.名称.as_str()).collect();
        if !已用.contains(期望) {
            return 期望.to_string();
        }
        for 序号 in 2..=u32::MAX {
            let 候选 = format!("{期望}{序号}");
            if !已用.contains(候选.as_str()) {
                return 候选;
            }
        }
        期望.to_string()
    }

    /// 登记一个资源引用，并返回登记后的记录。
    pub fn 登记资源(&mut self, 路径: 项目相对路径, 唯一标识: Option<资源唯一标识>) -> 资源引用 {
        let 记录 = self
            .资源表
            .entry(路径.clone())
            .or_insert_with(|| 资源引用 {
                路径: 路径.clone(),
                唯一标识,
            });
        if 记录.唯一标识.is_none() {
            记录.唯一标识 = 唯一标识;
        }
        记录.clone()
    }

    /// 扫描所有组件属性，把内嵌的资源引用补登到资源表。
    ///
    /// 返回新登记的资源数量。这个方法是"资源表不需要用户手工维护"的关键：
    /// 只要组件引用了资源，保存前跑一遍就能保证资源表完整。
    pub fn 同步资源表(&mut self) -> usize {
        let mut 收集到: Vec<(项目相对路径, Option<资源唯一标识>)> = Vec::new();
        for 实体 in self.实体表.values() {
            for 组件 in &实体.组件 {
                for 值 in 组件.属性.values() {
                    收集资源引用(值, &mut 收集到);
                }
            }
        }
        let 原有数量 = self.资源表.len();
        for (路径, 唯一标识) in 收集到 {
            self.登记资源(路径, 唯一标识);
        }
        self.资源表.len().saturating_sub(原有数量)
    }

    /// 移除资源表中已不再被任何组件引用的条目。
    pub fn 清理资源表(&mut self) {
        let mut 在用: HashSet<String> = HashSet::new();
        let mut 收集到 = Vec::new();
        for 实体 in self.实体表.values() {
            for 组件 in &实体.组件 {
                for 值 in 组件.属性.values() {
                    收集资源引用(值, &mut 收集到);
                }
            }
        }
        for (路径, _) in 收集到 {
            在用.insert(路径.文本().to_string());
        }
        self.资源表
            .retain(|路径, _| 在用.contains(路径.文本()));
    }

    /// 把资源路径改指向新位置。返回受影响的资源表条目数。
    ///
    /// 用途：在资源浏览器里重命名或移动文件后，**同步更新所有引用**。
    /// 这是"不得随意移动文件后破坏场景引用"这条要求的实现基础。
    pub fn 改资源路径(&mut self, 旧路径: &项目相对路径, 新路径: &项目相对路径) -> usize {
        let mut 数量 = 0;

        // 资源表
        if let Some(记录) = self.资源表.remove(旧路径) {
            let 唯一标识 = 记录.唯一标识;
            self.资源表.insert(新路径.clone(), 资源引用 {
                路径: 新路径.clone(),
                唯一标识,
            });
            数量 += 1;
        }

        // 组件属性里的内嵌引用。
        //
        // `含路径` 返回 `Option<()>`，这里只关心"是否存在"，因此直接用
        // `is_some()` 判断，而不是把返回值绑定到变量上再丢弃 ——
        // 后者会留下一个没有任何效果的表达式。
        let mut 受影响实体 = Vec::new();
        for (标识, 实体) in &self.实体表 {
            '实体: for 组件 in &实体.组件 {
                for 值 in 组件.属性.values() {
                    if 含路径(值, 旧路径).is_some() {
                        受影响实体.push(*标识);
                        // 一个实体只要有一处引用就够了，跳到下一个实体。
                        break '实体;
                    }
                }
            }
        }
        受影响实体.sort();
        受影响实体.dedup();

        for 标识 in &受影响实体 {
            if let Some(实体) = self.实体表.get_mut(标识) {
                for 组件 in &mut 实体.组件 {
                    for 值 in 组件.属性.values_mut() {
                        替换路径(值, 旧路径, 新路径);
                    }
                }
            }
        }

        // 改写完成后必须重建资源表。
        //
        // 上面第 1 步删掉了旧路径的登记并插入了新路径的登记，
        // 但组件里的引用是在第 2 步才改写完的；如果此时不重建，
        // 资源表里会残留"新路径已登记"与"组件引用已指向新路径"之间的
        // 不一致状态（例如同一文件被登记两次、旧路径仍挂着孤儿条目），
        // 表现为保存出来的场景校验不过、且资源引用追踪错乱。
        self.同步资源表();

        数量 + 受影响实体.len()
    }

    /// 校验文档一致性。
    pub fn 校验(&self) -> 文档校验报告 {
        let mut 报告 = 文档校验报告::default();

        // 父实体存在性
        for (标识, 实体) in &self.实体表 {
            if let Some(父) = 实体.父实体
                && !self.实体表.contains_key(&父) {
                    报告.悬空父实体.push(*标识);
                }
        }

        // 循环层级：从每个实体向上走，若步数超过实体总数则必然处于环中。
        //
        // 用 `while let` 串起两次查找，比在 `loop` 里写两个 `let ... else break`
        // 更直白：条件本身就是"还能往上走一步"。
        let 上限 = self.实体表.len() + 1;
        for 标识 in self.实体表.keys() {
            let mut 当前 = *标识;
            let mut 步数 = 0;
            while let Some(父) = self.取(当前).and_then(|实体| 实体.父实体) {
                当前 = 父;
                步数 += 1;
                if 步数 > 上限 {
                    报告.循环层级.push(*标识);
                    break;
                }
            }
        }
        报告.循环层级.sort();
        报告.循环层级.dedup();

        // 次序冲突
        //
        // 注意分组键用"实际父实体"（顶层实体的父实体是 `None`），
        // 而不是"是否出现在顶层顺序表里"。否则一个父实体为 `None`
        // 但漏登记在 `顶层顺序` 的实体，会被错分成独立一组，
        // 既漏报它与其他顶层实体的次序冲突，又无法与兄弟比对次序。
        let mut 按父分组: HashMap<Option<实体标识>, HashMap<u32, Vec<实体标识>>> = HashMap::new();
        for 实体 in self.实体表.values() {
            if 实体.父实体.is_none() && !self.顶层顺序.contains(&实体.标识) {
                // 没有父实体却没登记在顶层顺序里，等同于悬空。
                报告.悬空父实体.push(实体.标识);
            }
            按父分组
                .entry(实体.父实体)
                .or_default()
                .entry(实体.次序)
                .or_default()
                .push(实体.标识);
        }
        for 分组 in 按父分组.values() {
            for 标识列表 in 分组.values() {
                if 标识列表.len() > 1 {
                    报告.次序冲突.extend(标识列表.iter().copied());
                }
            }
        }
        报告.次序冲突.sort();
        报告.次序冲突.dedup();
        报告.悬空父实体.sort();
        报告.悬空父实体.dedup();

        // 资源引用是否登记
        let mut 收集到 = Vec::new();
        for 实体 in self.实体表.values() {
            for 组件 in &实体.组件 {
                for 值 in 组件.属性.values() {
                    收集资源引用(值, &mut 收集到);
                }
            }
        }
        let mut 未登记: BTreeSet<项目相对路径> = BTreeSet::new();
        for (路径, _) in 收集到 {
            if !self.资源表.contains_key(&路径) {
                未登记.insert(路径);
            }
        }
        报告.未登记资源 = 未登记.into_iter().collect();

        报告
    }

    /// 用另一份文档的内容整体覆盖自己。撤销/重做与运行时隔离都依赖它。
    pub fn 覆盖为(&mut self, 其他: &场景文档) {
        self.格式版本 = 其他.格式版本;
        self.元数据 = 其他.元数据.clone();
        self.实体表 = 其他.实体表.clone();
        self.顶层顺序 = 其他.顶层顺序.clone();
        self.资源表 = 其他.资源表.clone();
    }

    /// 按场景树的显示顺序，深度优先遍历所有实体。
    ///
    /// 返回 `(标识, 深度)` 列表。用于场景树扁平渲染与"选择所有子实体"等操作。
    pub fn 深度优先遍历(&self) -> Vec<(实体标识, usize)> {
        let mut 结果 = Vec::new();
        let mut 已访问 = HashSet::new();
        // 显式栈，元素为 (标识, 深度)。压栈时反向压入以保持兄弟顺序。
        let mut 栈: Vec<(实体标识, usize)> = self
            .子实体列表(None)
            .into_iter()
            .rev()
            .map(|标识| (标识, 0))
            .collect();
        while let Some((标识, 深度)) = 栈.pop() {
            if !已访问.insert(标识) {
                continue;
            }
            结果.push((标识, 深度));
            for 子 in self.子实体列表(Some(标识)).into_iter().rev() {
                栈.push((子, 深度 + 1));
            }
        }
        结果
    }
}

/// 递归收集属性值里内嵌的资源引用。
fn 收集资源引用(值: &属性值, 输出: &mut Vec<(项目相对路径, Option<资源唯一标识>)>) {
    match 值 {
        属性值::资源(引用) => {
            输出.push((引用.路径.clone(), 引用.唯一标识.map(资源唯一标识::从)));
        }
        属性值::列表(列表) => {
            for 项 in 列表 {
                收集资源引用(项, 输出);
            }
        }
        属性值::结构(字段) => {
            for (_, 项) in 字段 {
                收集资源引用(项, 输出);
            }
        }
        属性值::枚举(枚举) => {
            if let Some(负载) = &枚举.负载 {
                收集资源引用(负载, 输出);
            }
        }
        _ => {}
    }
}

/// 判断属性值里是否含有指向指定路径的资源引用。
fn 含路径(值: &属性值, 目标: &项目相对路径) -> Option<()> {
    match 值 {
        属性值::资源(引用) if &引用.路径 == 目标 => Some(()),
        属性值::资源(_) => None,
        属性值::列表(列表) => 列表.iter().find_map(|项| 含路径(项, 目标)),
        属性值::结构(字段) => 字段.iter().find_map(|(_, 项)| 含路径(项, 目标)),
        属性值::枚举(枚举) => 枚举.负载.as_deref().and_then(|负载| 含路径(负载, 目标)),
        _ => None,
    }
}

/// 把属性值里指向旧路径的资源引用改写为新路径。
fn 替换路径(值: &mut 属性值, 旧路径: &项目相对路径, 新路径: &项目相对路径) {
    match 值 {
        属性值::资源(引用) => {
            if &引用.路径 == 旧路径 {
                引用.路径 = 新路径.clone();
            }
        }
        属性值::列表(列表) => {
            for 项 in 列表 {
                替换路径(项, 旧路径, 新路径);
            }
        }
        属性值::结构(字段) => {
            for (_, 项) in 字段 {
                替换路径(项, 旧路径, 新路径);
            }
        }
        属性值::枚举(枚举) => {
            if let Some(负载) = &mut 枚举.负载 {
                替换路径(负载, 旧路径, 新路径);
            }
        }
        _ => {}
    }
}
