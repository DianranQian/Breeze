//! 中文名：示例模板安装
//! 示例项目的**交付契约**验收：模板发现 → 复制到仓库外 → 隐藏模板后仍能打开。
//!
//! # 为什么这些断言必须在这种形状下做
//!
//! 上一轮的"打开示例项目"直接读 `CARGO_MANIFEST_DIR/../../examples/<名称>`。
//! 那个实现只要仓库在就永远能跑通，于是任何测试都会绿，而产品交付层面
//! 仍然是"编辑器依赖源码树"。真正要证明的是：
//!
//! 1. 模板可以从**可配置目录**发现（不依赖仓库路径）；
//! 2. 复制到**仓库外**之后，`project.ron`、`assets/`、`scenes/` 都在副本里；
//! 3. **把原始模板目录整个删掉之后**，副本仍然能打开、资源仍能解析；
//! 4. 目标目录已有内容时**默认拒绝覆盖**，且用户文件一个字节都没变。
//!
//! 因此本文件的所有路径都落在 `std::env::temp_dir()` 下，仓库的 `examples/`
//! 只作为**复制来源**被读取，绝不写入、也不作为打开路径。
//!
//! # 为什么所有断言挤在一个测试函数里
//!
//! 三个环境变量（模板目录、安装目录、配置目录）是**进程全局**的。
//! 拆成多个 `#[test]` 会在同一个二进制里并行执行并互相改写环境变量，
//! 表现为随机的"有时找不到模板"。全部串行放在一条测试里最稳，
//! 而且这条链本来就是一条端到端故事。

use std::path::{Path, PathBuf};

use editor_app::actions::{准备示例项目, 示例安装目录, 示例安装目录环境变量};
use editor_ui::dev_modes::示例项目;

/// 仓库根目录（只用于读取示例作为**复制来源**）。
fn 仓库根() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .to_path_buf()
}

/// 造一个唯一的临时目录（避免并行测试互相踩）。
fn 唯一临时目录(名: &str) -> PathBuf {
    let 目录 = std::env::temp_dir().join(format!(
        "breeze_demo_{名}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|值| 值.as_millis())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&目录);
    std::fs::create_dir_all(&目录).expect("应当能创建临时目录");
    目录
}

/// 递归统计文件数与总字节数。
fn 统计(根: &Path) -> (usize, u64) {
    let mut 文件数 = 0;
    let mut 字节 = 0;
    let mut 栈 = vec![根.to_path_buf()];
    while let Some(当前) = 栈.pop() {
        let Ok(条目) = std::fs::read_dir(&当前) else {
            continue;
        };
        for 条目 in 条目.flatten() {
            let 路径 = 条目.path();
            match 条目.file_type() {
                Ok(类型) if 类型.is_dir() => 栈.push(路径),
                Ok(类型) if 类型.is_file() => {
                    文件数 += 1;
                    字节 += 条目.metadata().map(|元| 元.len()).unwrap_or(0);
                }
                _ => {}
            }
        }
    }
    (文件数, 字节)
}

/// 端到端：模板目录可配置 → 复制到仓库外 → 删掉模板后副本仍可打开。
#[test]
fn 示例项目能复制到仓库外且在模板消失后仍可打开() {
    let 仓库示例 = 仓库根().join("examples");
    assert!(
        仓库示例.join("2d_demo").join("project.ron").is_file(),
        "前置条件：仓库里应当有 2d_demo 示例（作为复制来源）：{}",
        仓库示例.display()
    );

    // ---- ① 模板目录放在仓库**外面**，指向从仓库复制的两个示例 ----
    let 模板根 = 唯一临时目录("templates");
    for 名称 in ["2d_demo", "3d_demo"] {
        复制目录(&仓库示例.join(名称), &模板根.join(名称));
    }
    let 安装根 = 唯一临时目录("installed");
    let 配置根 = 唯一临时目录("config");

    // SAFETY（unsafe 无关）：`set_var` 在 2024 edition 下是 unsafe 的，
    // 但本测试是单线程串行执行的（见文件头说明），不存在并发读写环境变量。
    unsafe {
        std::env::set_var(
            editor_core::template::模板目录环境变量,
            模板根.display().to_string(),
        );
        std::env::set_var(示例安装目录环境变量, 安装根.display().to_string());
        std::env::set_var(
            editor_app::config::配置目录环境变量,
            配置根.display().to_string(),
        );
    }
    assert_eq!(示例安装目录(), 安装根, "安装目录应当被环境变量接管");

    // ---- ② 两个示例都能从模板目录发现 ----
    let 模板列表 = editor_core::template::发现模板().expect("应当能发现模板");
    let 名字: Vec<&str> = 模板列表.iter().map(|项| 项.名称.as_str()).collect();
    for 必须 in ["2d_demo", "3d_demo"] {
        assert!(名字.contains(&必须), "应当发现 `{必须}`，实际：{名字:?}");
    }

    // ---- ③ 安装（复制）到仓库外的安装根 ----
    for (示例, 名称) in [(示例项目::二维, "2d_demo"), (示例项目::三维, "3d_demo")] {
        let 目标 = 准备示例项目(示例).expect("示例项目应当能安装");
        let 期望 = 安装根.join(名称);
        assert_eq!(目标, 期望, "安装位置应当在仓库外的安装根下");
        assert!(
            !目标.starts_with(仓库根()),
            "安装位置绝不能落在仓库里：{}",
            目标.display()
        );
        assert!(
            目标.join("project.ron").is_file(),
            "复制结果必须包含 project.ron：{}",
            目标.display()
        );
        assert!(
            目标.join("assets").is_dir(),
            "复制结果必须包含 assets/（否则资源路径会打成仓库耦合）：{}",
            目标.display()
        );
        assert!(
            目标.join("scenes").is_dir(),
            "复制结果必须包含 scenes/：{}",
            目标.display()
        );
        // 副本的体量应当与模板一致（证明素材真的被复制，而不是只复制了清单）。
        let (模板文件, _) = 统计(&模板根.join(名称));
        let (副本文件, _) = 统计(&目标);
        assert_eq!(
            副本文件, 模板文件,
            "`{名称}` 的副本文件数必须与模板一致（{副本文件} vs {模板文件}）"
        );
    }

    // ---- ④ 资源路径相对项目根解析 ----
    let 二维 = 安装根.join("2d_demo");
    let 场景 = editor_core::template::解析资源路径(&二维, "scenes/qinglan_valley.scene.ron")
        .expect("场景应当是项目内的真实文件");
    assert!(场景.starts_with(&二维), "解析结果必须落在项目根内");
    assert!(
        editor_core::template::解析资源路径(&二维, "../外面.txt").is_err(),
        "目录穿越必须被拒绝"
    );

    // ---- ⑤ 真的能打开（走编辑器自己的打开流程）----
    let 清单 = editor_core::template::打开已有项目目录(&二维).expect("副本应当是一个合法项目");
    assert_eq!(清单.场景数, 1, "2d_demo 应当有 1 个场景");
    let mut 编辑器 = editor_core::state::编辑器状态::新建();
    编辑器
        .打开项目(&二维)
        .expect("编辑器应当能打开仓库外的副本");
    assert_eq!(编辑器.会话.文档.实体数量(), 46, "2D 示例实体数应与预期一致");

    // 3D 副本同样必须能打开（两个示例都要验，不能只验 2D）。
    let 三维 = 安装根.join("3d_demo");
    let 三维清单 = editor_core::template::打开已有项目目录(&三维).expect("3D 副本应当是合法项目");
    assert_eq!(三维清单.场景数, 1, "3d_demo 应当有 1 个场景");
    let mut 编辑器3 = editor_core::state::编辑器状态::新建();
    编辑器3
        .打开项目(&三维)
        .expect("编辑器应当能打开仓库外的 3D 副本");
    assert!(编辑器3.会话.文档.实体数量() > 0, "3D 副本打开后应当有实体");

    // ---- ⑥ 把原始模板目录**整个删掉**，副本仍然可用 ----
    std::fs::remove_dir_all(&模板根).expect("应当能删除模板目录");
    assert!(!模板根.exists(), "前置条件：模板目录必须已经不在了");

    // 再次"打开示例项目"：已安装 → 直接返回，且**不读模板**。
    let 再次 = 准备示例项目(示例项目::二维).expect("模板消失后，已安装的副本仍必须能打开");
    assert_eq!(再次, 二维);
    let mut 编辑器 = editor_core::state::编辑器状态::新建();
    编辑器
        .打开项目(&再次)
        .expect("模板消失后，副本仍必须能被编辑器打开");
    assert_eq!(编辑器.会话.文档.实体数量(), 46);
    assert!(
        editor_core::template::解析资源路径(&再次, "scenes/qinglan_valley.scene.ron").is_ok(),
        "模板消失后，副本内的资源路径仍必须解析得到"
    );

    // ---- ⑦ 覆盖保护：目标已有用户文件时默认拒绝，且文件不被改动 ----
    let 受保护 = 唯一临时目录("protected");
    let 目标 = 受保护.join("2d_demo");
    std::fs::create_dir_all(&目标).expect("应当能创建目标目录");
    let 用户文件 = 目标.join("我的笔记.txt");
    std::fs::write(&用户文件, "用户自己的内容，绝不能被覆盖").expect("应当能写文件");

    // 把安装根指到受保护目录，模板也重新造一份（上一步已删掉）。
    let 模板根2 = 唯一临时目录("templates2");
    复制目录(&仓库示例.join("2d_demo"), &模板根2.join("2d_demo"));
    unsafe {
        std::env::set_var(
            editor_core::template::模板目录环境变量,
            模板根2.display().to_string(),
        );
        std::env::set_var(示例安装目录环境变量, 受保护.display().to_string());
    }
    let 错误 = 准备示例项目(示例项目::二维).expect_err("目标非空时必须拒绝覆盖");
    assert!(
        错误.contains("失败") || 错误.contains("已存在") || 错误.contains("非空"),
        "错误信息应当说明拒绝原因：{错误}"
    );
    assert_eq!(
        std::fs::read_to_string(&用户文件).expect("用户文件必须仍然存在"),
        "用户自己的内容，绝不能被覆盖",
        "拒绝覆盖时用户文件的内容必须一个字节都没变"
    );
    let (文件数, _) = 统计(&目标);
    assert_eq!(文件数, 1, "拒绝覆盖时不应写入任何新文件，实际 {文件数} 个");

    // ---- 收尾 ----
    let _ = std::fs::remove_dir_all(&安装根);
    let _ = std::fs::remove_dir_all(&受保护);
    let _ = std::fs::remove_dir_all(&模板根2);
    let _ = std::fs::remove_dir_all(&配置根);
}

/// 递归复制目录（测试辅助；不依赖第三方 crate 的递归复制语义）。
fn 复制目录(源: &Path, 目标: &Path) {
    std::fs::create_dir_all(目标).expect("应当能创建目录");
    for 条目 in std::fs::read_dir(源).expect("应当能读目录").flatten() {
        let 来源 = 条目.path();
        let 去向 = 目标.join(条目.file_name());
        if 条目.file_type().map(|类型| 类型.is_dir()).unwrap_or(false) {
            复制目录(&来源, &去向);
        } else {
            std::fs::copy(&来源, &去向).expect("应当能复制文件");
        }
    }
}
