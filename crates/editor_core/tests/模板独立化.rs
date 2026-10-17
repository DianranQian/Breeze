//! 示例项目模板独立化的集成测试。
//!
//! # 这些测试守的是什么
//!
//! 「打开示例」过去依赖编译期写死的仓库路径，这条链路只有在开发者本机、
//! 且仓库目录结构不变时才成立。这里的每条用例都在验证替代方案的**边界**：
//!
//! * 模板发现只认"目录里有 `project.ron`"，与仓库位置无关；
//! * 复制默认拒绝覆盖，失败时**一个字节都不写**（用户文件不能被毁）；
//! * 复制出的项目与模板**彻底断奶**：把源模板整个删掉，它照样能打开；
//! * 资源路径不允许绝对路径、不允许 `..` 逃出项目根。
//!
//! 所有夹具都建在 `tempfile` 造的临时目录里（系统临时目录，必然在仓库之外），
//! 测试既不读也不写仓库里的 `examples/` —— 唯一碰仓库示例的是最后那条
//! 「真实示例」用例，而且它只做**复制出去**这件事。
//!
//! 测试名统一带 `模板` 前缀，`cargo test -p editor_core 模板` 一次跑全。

// 与 `tests/编辑闭环.rs` 保持一致的例外：测试名用中文 + 大写字母分段，
// 可读性优先于 `non_snake_case` 的字面要求。
#![allow(non_snake_case)]

use std::path::{Path, PathBuf};

use editor_core::模板::{
    从根集合发现, 从环境值发现, 发现模板, 复制模板到, 复制选项, 打开已有项目目录, 模板来源,
    模板根候选, 模板目录环境变量, 模板错误, 解析资源路径, 项目摘要,
};

// ======================================================================
// 夹具
// ======================================================================

/// 在一个目录下造一个最小但**真实**的模板，返回模板根。
///
/// 内容覆盖四种复制时必须小心处理的东西：
/// * 清单文件（判定模板的依据）；
/// * 中文目录名与中文文件名（Windows 上最容易翻车的编码场景）；
/// * 二进制资源（必须逐字节复制，不能当文本处理）；
/// * 空目录（递归复制要保留它，否则资源浏览器里会少一层分类）。
fn 造模板(父目录: &Path, 目录名: &str, 项目名: &str) -> PathBuf {
    let 根 = 父目录.join(目录名);
    std::fs::create_dir_all(根.join("assets").join("贴图")).expect("建 assets/贴图");
    std::fs::create_dir_all(根.join("assets").join("空目录")).expect("建空目录");
    std::fs::create_dir_all(根.join("scenes")).expect("建 scenes");

    std::fs::write(
        根.join("project.ron"),
        format!(
            "(\n    格式版本: 1,\n    名称: \"{项目名}\",\n    说明: \"\",\n    \
             启动场景: Some(\"scenes/主关卡.scene.ron\"),\n)\n"
        ),
    )
    .expect("写 project.ron");
    std::fs::write(
        根.join("scenes").join("主关卡.scene.ron"),
        "(\n    格式版本: 1,\n    名称: \"主关卡\",\n)\n",
    )
    .expect("写场景");
    // 假 PNG 头 + 非法 UTF-8 字节：用来证明复制是按字节而不是按文本。
    std::fs::write(
        根.join("assets").join("贴图").join("主角.png"),
        [
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0xFF, 0xFE, 0x80,
        ],
    )
    .expect("写二进制资源");
    std::fs::write(根.join("README.md"), "模板说明\n").expect("写 README");
    根
}

/// 递归统计 (文件数, 总字节数)。目录本身不计。
fn 统计文件(目录: &Path) -> (usize, u64) {
    let mut 文件数 = 0usize;
    let mut 字节数 = 0u64;
    let mut 栈 = vec![目录.to_path_buf()];
    while let Some(当前) = 栈.pop() {
        let Ok(读取) = std::fs::read_dir(&当前) else {
            continue;
        };
        for 项 in 读取.flatten() {
            let 路径 = 项.path();
            let Ok(元数据) = 项.metadata() else {
                continue;
            };
            if 元数据.is_dir() {
                栈.push(路径);
            } else if 元数据.is_file() {
                文件数 += 1;
                字节数 += 元数据.len();
            }
        }
    }
    (文件数, 字节数)
}

/// 递归收集全部文件（用于"从复制出的项目里随机挑一个真实资源"）。
fn 收集文件(目录: &Path) -> Vec<PathBuf> {
    let mut 结果 = Vec::new();
    let mut 栈 = vec![目录.to_path_buf()];
    while let Some(当前) = 栈.pop() {
        let Ok(读取) = std::fs::read_dir(&当前) else {
            continue;
        };
        for 项 in 读取.flatten() {
            let 路径 = 项.path();
            let Ok(元数据) = 项.metadata() else {
                continue;
            };
            if 元数据.is_dir() {
                栈.push(路径);
            } else if 元数据.is_file() {
                结果.push(路径);
            }
        }
    }
    结果.sort();
    结果
}

/// 把绝对路径转成项目相对路径文本（统一用 `/`）。
fn 相对文本(根: &Path, 文件: &Path) -> String {
    let 相对 = 文件.strip_prefix(根).expect("文件应当位于项目内");
    相对
        .components()
        .map(|组件| 组件.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// 断言路径不带 Windows 的 `\\?\` 扩展前缀。
///
/// `canonicalize` 在 Windows 上返回的正是这种"verbatim"路径，它显示给用户很难看，
/// 也会让上层用 `starts_with` 做的前缀比较莫名失败；模块内部应当已经统一去掉。
fn 断言无扩展前缀(路径: &Path) {
    let 文本 = 路径.to_string_lossy();
    assert!(
        !文本.starts_with(r"\\?\"),
        "返回给上层的路径不应带 Windows 扩展前缀：{文本}"
    );
}

/// 让编译器确认 `项目摘要` 这个类型名没有被拼错（同时避免导入被判未使用）。
#[allow(dead_code)]
fn _类型占位(_: 项目摘要) {}

// ======================================================================
// 发现模板：不依赖真实仓库路径
// ======================================================================

#[test]
fn 模板_从注入根集合发现_完全不读真实仓库() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 模板根 = 临时.path().join("示例库");
    std::fs::create_dir_all(&模板根).unwrap();
    造模板(&模板根, "alpha_demo", "甲示例");
    造模板(&模板根, "beta_demo", "乙示例");
    // 没有 project.ron 的目录不是模板
    std::fs::create_dir_all(模板根.join("not_a_template")).unwrap();

    let 列表 = 从根集合发现(std::slice::from_ref(&模板根));
    assert_eq!(列表.len(), 2, "只应发现带 project.ron 的目录：{列表:?}");
    // 顺序稳定（按名称排序），否则界面每次刷新都会跳
    assert_eq!(列表[0].名称, "alpha_demo");
    assert_eq!(列表[1].名称, "beta_demo");
    // 显示名来自清单里的项目名，而不是目录名
    assert_eq!(列表[0].显示名, "甲示例");
    assert_eq!(列表[1].显示名, "乙示例");
    assert!(列表.iter().all(|模板| 模板.来源 == 模板来源::注入));
    assert!(列表.iter().all(|模板| 模板.根目录.is_absolute()));
    assert!(列表[0].清单路径().is_file());

    // 候选根也可以直接是一个模板目录
    let 单个 = 从根集合发现(&[模板根.join("beta_demo")]);
    assert_eq!(单个.len(), 1);
    assert_eq!(单个[0].名称, "beta_demo");

    // 重复根去重；不存在的根安静跳过（发现过程不该 panic）
    let 去重 = 从根集合发现(&[
        模板根.clone(),
        模板根.clone(),
        临时.path().join("这个目录不存在"),
    ]);
    assert_eq!(去重.len(), 2, "重复候选不应产生重复模板");

    // 空候选集合 → 空结果
    assert!(从根集合发现(&[]).is_empty());
}

#[test]
fn 模板_环境变量指向模板根时列出全部模板() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 根 = 临时.path().join("模板库");
    std::fs::create_dir_all(&根).unwrap();
    造模板(&根, "alpha_demo", "甲示例");
    造模板(&根, "beta_demo", "乙示例");

    let 列表 = 从环境值发现(Some(&根)).expect("指向模板根应当能发现模板");
    assert_eq!(列表.len(), 2);
    assert!(列表.iter().all(|模板| 模板.来源 == 模板来源::环境变量));

    // 没有配置环境变量（None）→ 空列表而不是错误
    assert!(从环境值发现(None).expect("未配置不是错误").is_empty());
}

#[test]
fn 模板_环境变量指向单个模板时可用且三种失败原因互不相同() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 单个模板 = 造模板(临时.path(), "solo_demo", "单个示例");

    // 场景一：变量直接指向某个模板 —— 必须能用
    let 列表 = 从环境值发现(Some(&单个模板)).expect("指向单个模板也应当可用");
    assert_eq!(列表.len(), 1);
    assert_eq!(列表[0].名称, "solo_demo");
    assert_eq!(列表[0].显示名, "单个示例");
    assert_eq!(
        列表[0].来源,
        模板来源::环境变量单模板,
        "来源必须能区分「变量指向单个模板」与「变量指向模板根」，否则出错时说不清"
    );

    // 场景二：路径不存在
    let 不存在 = 从环境值发现(Some(&临时.path().join("并没有这个目录"))).unwrap_err();
    assert!(
        matches!(&不存在, 模板错误::环境变量目录不存在 { .. }),
        "实际：{不存在:?}"
    );
    let 不存在文案 = 不存在.to_string();
    assert!(
        不存在文案.contains("模板目录不存在"),
        "文案要说清是模板目录不存在：{不存在文案}"
    );

    // 场景三：路径存在但是个文件
    let 文件 = 临时.path().join("一个文件.txt");
    std::fs::write(&文件, "x").unwrap();
    let 不是目录 = 从环境值发现(Some(&文件)).unwrap_err();
    assert!(
        matches!(&不是目录, 模板错误::环境变量不是目录 { .. }),
        "实际：{不是目录:?}"
    );

    // 场景四：目录存在，但里面没有任何模板
    let 空目录 = 临时.path().join("空目录");
    std::fs::create_dir_all(&空目录).unwrap();
    let 没有模板 = 从环境值发现(Some(&空目录)).unwrap_err();
    assert!(
        matches!(&没有模板, 模板错误::环境变量无模板 { .. }),
        "实际：{没有模板:?}"
    );

    // 三种失败的文案必须互不相同：用户看到错误才知道该改路径的哪一部分
    let 文案 = [不存在文案, 不是目录.to_string(), 没有模板.to_string()];
    for 左 in 0..文案.len() {
        for 右 in (左 + 1)..文案.len() {
            assert_ne!(文案[左], 文案[右], "不同原因必须有不同文案");
        }
    }
}

#[test]
fn 模板_开发期兜底能发现仓库示例且环境变量能完全绕过它() {
    if std::env::var_os(模板目录环境变量).is_some() {
        // 环境变量一旦设置就完全接管候选列表（这本身就是"可被绕过"的证明），
        // 此时不该再断言仓库兜底路径，否则测试就依赖了外部环境。
        return;
    }

    let 列表 = 发现模板().expect("开发期兜底应当能找到仓库里的示例");
    let 名称: Vec<&str> = 列表.iter().map(|模板| 模板.名称.as_str()).collect();
    for 期望 in ["2d_demo", "3d_demo"] {
        assert!(名称.contains(&期望), "应当发现 {期望}，实际：{名称:?}");
    }
    assert!(
        列表
            .iter()
            .all(|模板| matches!(模板.来源, 模板来源::开发期兜底 | 模板来源::可执行文件同级)),
        "开发期只可能命中兜底两级来源：{列表:?}"
    );

    // 候选列表里至少有一条真实存在的路径
    assert!(模板根候选().iter().any(|路径| 路径.is_dir()));
}

// ======================================================================
// 复制模板：安全边界
// ======================================================================

#[test]
fn 模板_复制到仓库外目标成功且文件数与源一致() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 源 = 造模板(临时.path(), "源模板", "源示例");
    // 目标父目录也不存在：模拟用户选了"新建文件夹/我的游戏"
    let 目标 = 临时.path().join("安装位置").join("我的游戏");

    let 结果 = 复制模板到(&源, &目标, 复制选项::default()).expect("复制应当成功");

    let (源文件数, 源字节数) = 统计文件(&源);
    assert_eq!(源文件数, 4, "夹具应当有 4 个文件");
    assert_eq!(结果.文件数, 源文件数, "复制的文件数必须与源一致");
    assert_eq!(结果.总字节数, 源字节数, "复制的总字节数必须与源一致");
    assert_eq!(结果.目录数, 4, "assets、assets/贴图、assets/空目录、scenes");
    assert!(结果.目标.is_absolute(), "目标必须是绝对路径");
    assert!(结果.源.is_absolute());
    断言无扩展前缀(&结果.目标);
    断言无扩展前缀(&结果.源);

    // 落地校验
    assert!(目标.join("project.ron").is_file(), "目标缺少 project.ron");
    assert!(
        目标.join("assets").join("空目录").is_dir(),
        "空目录也必须被复制，否则项目结构会缺一层"
    );

    // 二进制资源逐字节一致（不是按文本搬运）
    let 源图 = std::fs::read(源.join("assets").join("贴图").join("主角.png")).unwrap();
    let 目标图 = std::fs::read(目标.join("assets").join("贴图").join("主角.png")).unwrap();
    assert_eq!(源图, 目标图, "二进制资源必须逐字节一致");

    // 目标里的文件数与源一致，且没有多出别的东西
    assert_eq!(统计文件(&目标).0, 源文件数);
}

#[test]
fn 模板_目标非空时默认拒绝且原文件不被改动() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 源 = 造模板(临时.path(), "源模板", "源示例");
    let 目标 = 临时.path().join("用户的目录");
    std::fs::create_dir_all(&目标).unwrap();
    std::fs::write(目标.join("我的重要文件.txt"), "不要动我").unwrap();

    let 错误 = 复制模板到(&源, &目标, 复制选项::default()).unwrap_err();
    assert!(
        matches!(&错误, 模板错误::目标已存在 { 数量: 1, .. }),
        "非空目标必须默认拒绝，实际：{错误:?}"
    );
    assert!(
        错误.to_string().contains("已存在且非空"),
        "文案要说明原因：{错误}"
    );

    // 关键：拒绝时目标必须一个字节都没被改
    assert_eq!(
        std::fs::read_to_string(目标.join("我的重要文件.txt")).unwrap(),
        "不要动我"
    );
    assert!(
        !目标.join("project.ron").exists(),
        "拒绝时不允许留下半个项目"
    );
    assert_eq!(
        std::fs::read_dir(&目标).unwrap().count(),
        1,
        "目标目录里不该多出任何文件"
    );
}

#[test]
fn 模板_允许覆盖时确实覆盖并清掉旧文件() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 源 = 造模板(临时.path(), "源模板", "新项目");
    let 目标 = 临时.path().join("旧项目");
    std::fs::create_dir_all(&目标).unwrap();
    std::fs::write(目标.join("旧文件.txt"), "旧内容").unwrap();
    std::fs::write(目标.join("project.ron"), "(格式版本: 1, 名称: \"旧项目\")").unwrap();

    // 用 `..Default::default()` 的写法，确认选项结构体可以这样扩展字段
    let 选项 = 复制选项 {
        允许覆盖: true,
        ..Default::default()
    };
    let 结果 = 复制模板到(&源, &目标, 选项.clone()).expect("允许覆盖时应当成功");

    assert!(
        !目标.join("旧文件.txt").exists(),
        "覆盖必须先清空目标，否则新旧文件会混成一个假项目"
    );
    let 源清单 = std::fs::read_to_string(源.join("project.ron")).unwrap();
    let 目标清单 = std::fs::read_to_string(目标.join("project.ron")).unwrap();
    assert_eq!(目标清单, 源清单, "清单必须被真正覆盖");
    assert!(目标清单.contains("新项目"));

    let (源文件数, _) = 统计文件(&源);
    assert_eq!(结果.文件数, 源文件数);
    assert_eq!(统计文件(&目标).0, 源文件数, "覆盖后目标应与模板逐文件一致");

    // 便捷构造器等价于上面的字面量
    assert_eq!(复制选项::覆盖(), 选项);
}

#[test]
fn 模板_目标为空目录时允许写入() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 源 = 造模板(临时.path(), "源模板", "空目录目标");
    let 目标 = 临时.path().join("已经建好的空目录");
    std::fs::create_dir_all(&目标).unwrap();

    复制模板到(&源, &目标, 复制选项::default()).expect("空目录应当允许写入");
    assert!(目标.join("project.ron").is_file());
    assert!(目标.join("assets").is_dir());
}

#[test]
fn 模板_可以只复制可见项() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 源 = 造模板(临时.path(), "源模板", "隐藏项");
    std::fs::write(源.join(".gitignore"), "/target\n").unwrap();
    std::fs::create_dir_all(源.join(".git")).unwrap();
    std::fs::write(源.join(".git").join("config"), "[core]\n").unwrap();

    let 目标 = 临时.path().join("目标");
    let 选项 = 复制选项 {
        复制隐藏项: false,
        ..Default::default()
    };
    let 结果 = 复制模板到(&源, &目标, 选项).expect("复制应当成功");

    assert!(
        !目标.join(".gitignore").exists(),
        "跳过隐藏项时不该复制点文件"
    );
    assert!(!目标.join(".git").exists(), "跳过隐藏项时不该复制点目录");
    assert!(目标.join("project.ron").is_file());
    assert_eq!(结果.文件数, 统计文件(&源).0 - 2, "两个隐藏文件应当被跳过");
}

#[test]
fn 模板_拒绝复制到模板自身或自身内部() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 源 = 造模板(临时.path(), "源模板", "源示例");
    let (复制前文件数, _) = 统计文件(&源);

    // 目标就是自己：即使允许覆盖也必须拒绝，否则等于先删自己再复制自己
    let 自身 = 复制模板到(&源, &源, 复制选项::覆盖()).unwrap_err();
    assert!(matches!(&自身, 模板错误::目标即模板(_)), "实际：{自身:?}");

    // 目标在模板内部：递归复制会把自己套娃进去
    let 内部 = 复制模板到(&源, &源.join("子项目"), 复制选项::default()).unwrap_err();
    assert!(
        matches!(&内部, 模板错误::目标在模板内 { .. }),
        "实际：{内部:?}"
    );

    // 反过来，目标包含了模板：允许覆盖时"清空目标"会把模板本身删掉
    let 包含 = 复制模板到(&源, 临时.path(), 复制选项::覆盖()).unwrap_err();
    assert!(
        matches!(&包含, 模板错误::目标包含模板 { .. }),
        "实际：{包含:?}"
    );

    assert_eq!(统计文件(&源).0, 复制前文件数, "拒绝时模板必须原封不动");
    assert!(源.join("project.ron").is_file());
    assert!(!源.join("子项目").exists());
    assert!(临时.path().join("源模板").is_dir(), "模板不能被清空");
}

#[test]
fn 模板_源目录不是模板时给出明确错误() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 空目录 = 临时.path().join("不是模板");
    std::fs::create_dir_all(&空目录).unwrap();

    let 错误 = 复制模板到(&空目录, &临时.path().join("目标"), 复制选项::default()).unwrap_err();
    assert!(
        matches!(&错误, 模板错误::缺少清单(_)),
        "没有 project.ron 的目录不能当模板，实际：{错误:?}"
    );
    assert!(错误.to_string().contains("project.ron"));

    let 不存在 = 复制模板到(
        &临时.path().join("根本没这个模板"),
        &临时.path().join("目标"),
        复制选项::default(),
    )
    .unwrap_err();
    assert!(
        matches!(&不存在, 模板错误::模板目录不存在(_)),
        "实际：{不存在:?}"
    );
}

// ======================================================================
// 打开已有项目 + 资源路径
// ======================================================================

#[test]
fn 模板_复制后的项目能打开且资源路径能解析到真实文件() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 源 = 造模板(临时.path(), "源模板", "复制后可打开");
    let 目标 = 临时.path().join("用户项目");
    复制模板到(&源, &目标, 复制选项::default()).expect("复制");

    let 摘要 = 打开已有项目目录(&目标).expect("复制出的项目应当能打开");
    assert_eq!(摘要.名称, "复制后可打开");
    assert_eq!(摘要.场景数, 1);
    assert!(摘要.根目录.is_absolute());
    断言无扩展前缀(&摘要.根目录);
    assert!(摘要.根目录.join("project.ron").is_file());

    // assets 相对路径必须解析到真实文件，且内容与模板一致
    let 图 = 解析资源路径(&摘要.根目录, "assets/贴图/主角.png").expect("资源应当能解析");
    assert!(图.is_file(), "解析结果必须是真实文件：{}", 图.display());
    断言无扩展前缀(&图);
    assert_eq!(
        std::fs::read(&图).unwrap(),
        std::fs::read(源.join("assets").join("贴图").join("主角.png")).unwrap()
    );
}

#[test]
fn 模板_删除源模板后复制出的项目仍然可以打开() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 源 = 造模板(临时.path(), "源模板", "断奶项目");
    let 目标 = 临时.path().join("独立项目");
    复制模板到(&源, &目标, 复制选项::default()).expect("复制");

    // 把模板整个删掉 —— 复制出的项目绝不能还依赖它
    std::fs::remove_dir_all(&源).expect("删除源模板");
    assert!(!源.exists());

    let 摘要 = 打开已有项目目录(&目标).expect("源模板删掉后，复制出的项目必须照旧能打开");
    assert_eq!(摘要.名称, "断奶项目");
    assert_eq!(摘要.场景数, 1);
    assert!(摘要.根目录.join("project.ron").is_file());

    let 图 = 解析资源路径(&摘要.根目录, "assets/贴图/主角.png").expect("资源必须随项目一起落地");
    assert!(图.is_file(), "资源必须真实存在：{}", 图.display());
    assert!(
        解析资源路径(&摘要.根目录, "scenes/主关卡.scene.ron")
            .expect("场景文件应当能被解析")
            .is_file()
    );
}

#[test]
fn 模板_打开已有项目目录会区分各种无效情况() {
    let 临时 = tempfile::tempdir().expect("临时目录");

    // 目录不存在
    let 不存在 = 打开已有项目目录(&临时.path().join("没有这个目录")).unwrap_err();
    assert!(
        matches!(&不存在, 模板错误::项目目录不存在(_)),
        "实际：{不存在:?}"
    );

    // 目录存在但缺清单
    let 空目录 = 临时.path().join("空目录");
    std::fs::create_dir_all(&空目录).unwrap();
    let 缺清单 = 打开已有项目目录(&空目录).unwrap_err();
    assert!(
        matches!(&缺清单, 模板错误::项目清单缺失(_)),
        "实际：{缺清单:?}"
    );
    assert!(缺清单.to_string().contains("project.ron"));

    // 清单存在但完全不是 RON
    let 坏项目 = 临时.path().join("坏项目");
    std::fs::create_dir_all(&坏项目).unwrap();
    std::fs::write(坏项目.join("project.ron"), "这不是 RON，也没有名称字段").unwrap();
    let 坏清单 = 打开已有项目目录(&坏项目).unwrap_err();
    assert!(
        matches!(&坏清单, 模板错误::项目清单无效 { .. }),
        "实际：{坏清单:?}"
    );
    assert!(坏清单.to_string().contains("项目清单无效"));

    // 版本过新：兼容读取不能变成"什么都放行"
    let 未来 = 临时.path().join("未来项目");
    std::fs::create_dir_all(&未来).unwrap();
    std::fs::write(
        未来.join("project.ron"),
        "(格式版本: 999, 名称: \"来自未来\")",
    )
    .unwrap();
    let 过新 = 打开已有项目目录(&未来).unwrap_err();
    assert!(
        matches!(&过新, 模板错误::项目清单无效 { .. }),
        "版本过新必须拒绝，实际：{过新:?}"
    );
    assert!(
        过新.to_string().contains("999") || 过新.to_string().contains("格式版本"),
        "文案要说明是版本问题：{过新}"
    );

    // 清单里本版本解析不了的内容：仍要能取到项目名（否则用户的项目彻底打不开）
    let 兼容 = 临时.path().join("兼容项目");
    std::fs::create_dir_all(&兼容).unwrap();
    std::fs::write(
        兼容.join("project.ron"),
        "(\n    格式版本: 1,\n    名称: \"兼容项目\",\n    启动场景: Some(12345),\n)\n",
    )
    .unwrap();
    let 摘要 = 打开已有项目目录(&兼容).expect("只是某个字段类型不认识，不该让项目打不开");
    assert_eq!(摘要.名称, "兼容项目");
    assert_eq!(摘要.场景数, 0, "没有 scenes 目录时场景数为 0");
}

#[test]
fn 模板_资源路径拒绝绝对路径与目录穿越() {
    let 临时 = tempfile::tempdir().expect("临时目录");
    let 根 = 造模板(临时.path(), "项目", "资源路径");
    // 项目外的文件：任何解析都不许把它带进来
    let 外面 = 临时.path().join("外面的秘密.txt");
    std::fs::write(&外面, "secret").unwrap();

    // 正常路径
    let 图 = 解析资源路径(&根, "assets/贴图/主角.png").expect("正常相对路径应当解析成功");
    assert!(图.is_file());
    assert!(
        解析资源路径(&根, r"assets\贴图\主角.png")
            .expect("反斜杠写法同样可用")
            .is_file()
    );
    assert!(
        解析资源路径(&根, "./assets/贴图/主角.png")
            .expect("./ 前缀应当被忽略")
            .is_file()
    );
    // 不逃出项目根的 `..` 是允许的（`a/../b` 语义上就在项目内）
    assert!(
        解析资源路径(&根, "assets/贴图/../贴图/主角.png")
            .expect("层内回退应当允许")
            .is_file()
    );
    assert!(
        解析资源路径(&根, "project.ron")
            .expect("根下的文件")
            .is_file()
    );

    // 逃逸：先报逃逸，而不是"文件不存在" —— 顺序很重要，
    // 否则用户会以为文件丢了，实际是路径被拒。
    match 解析资源路径(&根, "../外面的秘密.txt") {
        Err(模板错误::路径逃逸 { 相对, .. }) => assert_eq!(相对, "../外面的秘密.txt"),
        其它 => panic!("应当拒绝逃逸路径，实际：{其它:?}"),
    }
    assert!(matches!(
        解析资源路径(&根, "assets/../../外面的秘密.txt").unwrap_err(),
        模板错误::路径逃逸 { .. }
    ));
    assert!(
        matches!(
            解析资源路径(&根, "../并不存在.txt").unwrap_err(),
            模板错误::路径逃逸 { .. }
        ),
        "不存在的逃逸目标也必须先报逃逸"
    );

    // 绝对路径
    let 绝对 = 根.join("project.ron").to_string_lossy().into_owned();
    let 绝对错误 = 解析资源路径(&根, &绝对).unwrap_err();
    assert!(
        matches!(&绝对错误, 模板错误::绝对路径(_)),
        "实际：{绝对错误:?}"
    );
    for 形态 in [
        r"C:\Windows\win.ini",
        r"\\server\share\x.png",
        r"//server/share/x.png",
        "/etc/passwd",
    ] {
        assert!(
            matches!(解析资源路径(&根, 形态).unwrap_err(), 模板错误::绝对路径(_)),
            "应当拒绝绝对路径：{形态}"
        );
    }

    // 存在性
    let 缺 = 解析资源路径(&根, "assets/没有这个.png").unwrap_err();
    assert!(matches!(&缺, 模板错误::资源不存在 { .. }), "实际：{缺:?}");
    assert!(缺.to_string().contains("assets/没有这个.png"));

    // 空路径
    assert!(matches!(
        解析资源路径(&根, "   ").unwrap_err(),
        模板错误::相对路径为空
    ));

    // 项目根本身不存在
    assert!(matches!(
        解析资源路径(&临时.path().join("没有这个项目"), "assets/x.png").unwrap_err(),
        模板错误::项目目录不存在(_)
    ));
}

// ======================================================================
// 真实示例：2d_demo / 3d_demo（只复制出去，绝不修改仓库里的 examples）
// ======================================================================

#[test]
fn 模板_真实示例复制到临时目录后可以打开() {
    let 仓库示例目录 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples");
    assert!(
        仓库示例目录.is_dir(),
        "仓库示例目录应当存在：{}",
        仓库示例目录.display()
    );

    let 模板列表 = 从根集合发现(&[仓库示例目录]);
    let 名称: Vec<&str> = 模板列表.iter().map(|模板| 模板.名称.as_str()).collect();
    for 期望 in ["2d_demo", "3d_demo"] {
        assert!(名称.contains(&期望), "应当发现示例 {期望}，实际：{名称:?}");
    }

    for 模板 in &模板列表 {
        let (源文件数前, _) = 统计文件(&模板.根目录);

        // 落到系统临时目录下的唯一目录；TempDir 在用例结束（含 panic）时自动清理。
        let 工作区 = tempfile::Builder::new()
            .prefix("微风示例模板_")
            .tempdir_in(std::env::temp_dir())
            .expect("应当能在系统临时目录里建目录");
        let 目标 = 工作区.path().join(&模板.名称);

        let 结果 = 复制模板到(&模板.根目录, &目标, 复制选项::default())
            .unwrap_or_else(|错误| panic!("复制示例 {} 失败：{错误}", 模板.名称));
        assert!(结果.文件数 > 0, "示例 {} 复制了 0 个文件", 模板.名称);
        assert!(目标.join("project.ron").is_file());

        // 删掉与项目本身无关的附属文件，确认项目不靠它们撑着
        let _ = std::fs::remove_file(目标.join("README.md"));
        let _ = std::fs::remove_file(目标.join("ASSETS.md"));

        let 摘要 = 打开已有项目目录(&目标)
            .unwrap_or_else(|错误| panic!("复制出的示例 {} 应当能打开：{错误}", 模板.名称));
        assert!(!摘要.名称.trim().is_empty(), "项目名不能为空");
        assert!(摘要.场景数 >= 1, "示例 {} 应当至少有一个场景", 模板.名称);

        // 资源必须真的随复制落地，并且能被相对路径解析到
        let 资源文件 = 收集文件(&目标.join("assets"));
        assert!(
            !资源文件.is_empty(),
            "示例 {} 的 assets 不应为空",
            模板.名称
        );
        let 相对 = 相对文本(&目标, &资源文件[0]);
        let 解析 = 解析资源路径(&摘要.根目录, &相对)
            .unwrap_or_else(|错误| panic!("示例 {} 的资源 {相对} 解析失败：{错误}", 模板.名称));
        assert!(解析.is_file());

        // 这个测试不允许改动仓库里的示例：文件数必须与复制前一致
        assert_eq!(
            统计文件(&模板.根目录).0,
            源文件数前,
            "测试期间不得修改仓库里的示例 {}",
            模板.名称
        );
        assert!(模板.根目录.join("project.ron").is_file());
    }
}
