//! 中文名：标识符约束
//! BSN 的标识符约束：**中文命名与 `bsn!` 语法的冲突及其绕法**。
//!
//! # 这是一条实测发现，不是猜测
//!
//! 项目约定"自定义 Rust 类型、函数、字段、枚举和变量使用中文命名"。
//! 但 `bsn!` 宏区分"这是类型"还是"这是值/函数"的方式是
//! **看标识符首字符是否大写**：
//!
//! ```text
//! // bevy_macro_utils-0.20.0/src/path_type.rs:30
//! if last_ident_first_char.is_uppercase() { PathType::Type } else { PathType::Function }
//! ```
//!
//! `char::is_uppercase()` 对汉字的返回是 `false`（汉字没有大小写）。
//! 于是 `护甲` 这类中文类型名会被判为 `PathType::Function`，
//! `bsn!` 把它当成"从同名变量取值"，编译期报
//! `cannot find value 护甲 in this scope`。
//!
//! 更隐蔽的是后续症状：写 `护甲 { 值: 99 }` 时，`护甲` 已被当成表达式消费掉，
//! 紧跟的 `{ 值: 99 }` 会被当成**新的场景条目**去解析，
//! 于是报出与中文毫不相关的 `expected identifier` ——
//! 很容易被误判成"中文字段名不支持"。
//!
//! 本文件把这两条绕法都钉成可执行测试，避免以后有人重新踩一遍：
//!
//! 1. **字段名可以是中文**，只有**类型名**受启发式影响；
//! 2. 中文命名的组件类型可以**通过变量插入**（`let 实例 = ...; bsn! { 实例 }`）。

use bevy::prelude::*;

use bsn_bridge::BSN宿主;

/// 绕法一：`ASCII` 类型名 + 中文字段名。
///
/// 这条能成立，说明限制只在**类型名**上；字段名照旧可以用中文，
/// 与项目的中文命名约定并不冲突。
#[derive(Component, Default, Clone, Debug, PartialEq, Eq)]
struct Armor {
    值: i64,
    等级: i64,
}

/// 绕法二的被插入对象：完全中文命名。
#[derive(Component, Default, Clone, Debug, PartialEq, Eq)]
struct 护甲 {
    值: i64,
}

#[test]
fn 字段名可以是中文而类型名必须是_ascii() {
    let mut 宿主 = BSN宿主::新建();
    let 根 = 宿主
        .实例化(bsn! {
            Armor { 值: 99 }
        })
        .expect("ASCII 类型名 + 中文字段名应当可用");

    assert_eq!(
        宿主.取组件::<Armor>(根),
        Some(&Armor { 值: 99, 等级: 0 }),
        "中文字段必须被正确解析，未提到的字段取 Default"
    );
}

#[test]
fn 中文命名的组件可以经变量插入() {
    let mut 宿主 = BSN宿主::新建();
    // 形状仍由编译期决定：这里只有"一个实体、带 护甲 组件"。
    // 变化的是组件实例本身来自一个普通 Rust 变量。
    let 实例 = 护甲 { 值: 7 };
    let 根 = 宿主
        .实例化(bsn! {
            实例
        })
        .expect("通过变量插入中文命名组件应当可用");

    assert_eq!(宿主.取组件::<护甲>(根), Some(&护甲 { 值: 7 }));
}

#[test]
fn 中文命名的组件类型不能直接写在_bsn_里() {
    // 这条测试守护的是"约束确实存在"这件事本身。
    //
    // 编译器不会让我们把非法的 `bsn!` 放进来（那是编译错误，不是运行期错误），
    // 所以这里改用一个等价的编码检查：断言 `护甲` 的首字符不是大写，
    // 从而它一定会被 `PathType::new` 判为 Function。
    // 一旦哪天 Bevy 改了这条启发式，本测试与 `官方场景原型.rs` 里的人话说明
    // 就需要一起复核。
    let 类型名 = std::any::type_name::<护甲>();
    let 短名 = 类型名.rsplit("::").next().unwrap();
    assert_eq!(短名, "护甲");
    let 首字符 = 短名.chars().next().unwrap();
    assert!(
        !首字符.is_uppercase(),
        "前置条件失效：Bevy 若改用别的判定方式，中文命名约束需要重新评估"
    );
}
