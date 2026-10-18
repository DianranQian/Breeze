#!/usr/bin/env python3
"""由词条数据文件生成 Rust 翻译词表，并核对界面文案的覆盖情况。

用法（在仓库根目录执行）::

    python tools/gen_messages.py            # 生成 + 核对
    python tools/gen_messages.py --check    # 只核对，不写文件

# 为什么要脚本生成

词表有几百条，且必须按 UTF-8 字节序严格排序（查找用二分）。
手工维护一个几百行、还要严格有序的表，迟早会出错，
而顺序错了**不会报错，只会静默查不到**。把它生成出来，
顺序就永远是对的。

# 数据来源

`crates/editor_core/i18n/messages.tsv` —— 每行三列，用制表符分隔::

    键<TAB>中文<TAB>英文

它是翻译的唯一真相来源。新增界面文案的流程是：

1. 在代码里写 `语言.取("新的键")`；
2. 在 messages.tsv 里加一行；
3. 跑本脚本重新生成词表；
4. 跑 `python tools/gen_messages.py --check` 确认没有漏。

# 核对什么

* 词表里的键唯一、非空、是规范的小写 ASCII 标识符；
* 中英两列都非空；
* **扫描范围内源码出现的每一句界面中文，都能在词表里找到对应词条**
  （按中文文案反向匹配）—— 漏翻译会被当场报出来；
* 报告"词表里有、但源码里已经不再使用"的僵尸词条（只提示，不算错）。

扫描范围与排除规则见 `扫描目标` 与 `应排除`。
"""

from __future__ import annotations

import io
import os
import re
import sys

仓库根 = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
数据文件 = os.path.join(仓库根, "crates", "editor_core", "i18n", "messages.tsv")
输出文件 = os.path.join(仓库根, "crates", "editor_core", "src", "i18n", "table.rs")

# 参与"中文覆盖核对"的源码文件。
#
# 只列**已经完成国际化改造**的文件：核对的目标是"改过的文件不许漏"，
# 而不是"全仓库不许出现中文"—— 后者会把用户数据、日志、
# 尚未迁移的模块全部误报，最后只能靠加一堆例外来让它变绿，
# 那样的检查没有价值。
扫描目标 = [
    "crates/editor_ui/src/menu_bar.rs",
    "crates/editor_ui/src/inspector_panel.rs",
    "crates/editor_ui/src/assets_panel.rs",
    "crates/editor_ui/src/history_panel.rs",
    "crates/editor_ui/src/hierarchy_panel.rs",
    "crates/editor_ui/src/viewport_panel.rs",
    "crates/editor_ui/src/console_panel.rs",
    "crates/editor_ui/src/build_panel.rs",
    "crates/editor_ui/src/timeline_panel.rs",
    "crates/editor_ui/src/lib.rs",
    "crates/editor_ui/src/docks.rs",
    "crates/editor_ui/src/state.rs",
    "crates/editor_core/src/command.rs",
    "crates/editor_app/src/scene_bridge.rs",
]

# 这些中文串**不是**界面文案，不参与核对。
#
# 每条都要写清理由 —— 允许清单如果不写理由，很快就会变成
# "把报错的东西塞进来"的垃圾桶。
应排除 = {
    # ---- ① egui 控件 id ----
    #
    # 它们不是显示文本，而是控件的稳定标识。翻译它们会让
    # 切换语言时控件状态（展开/折叠、滚动位置、焦点）全部丢失。
    "二维-{字段名}",
    "三维-{字段名}",
    "四维-{字段名}",
    "通用二维-{字段名}",
    "通用三维-{字段名}",
    "通用四维-{字段名}",
    "枚举-{字段名}",
    "组件-{类型名}",
    "角度-{字段名}",
    "资源目录-{路径}",
    "变换区",
    "资源排序",
    # 场景树"行内空白区"与"空白区域"两个感应区的控件 id。
    #
    # 它们由 `ui.interact(..., egui::Id::new(..))` 显式指定：空白区不对应
    # 任何实体，没有可派生的天然 id，必须自带一个。翻译它会让切换语言后
    # 这块感应区的交互状态被当成"另一个控件"而丢失。
    "场景树行空白",
    "场景树空白区域",
    # ---- ② 纯格式化模板 ----
    #
    # 只做路径拼接、数字精度、单位与序号排版，本身不含可翻译文字；
    # 可见部分全部来自参数（数字、单位符号、用户数据）。
    "[{下标}]",
    "{字段名}-{下标}",
    "{前缀}/{名称}",
    "{字节} {}",
    "{值:.1} {}",
    "{值:.4}",
    "{类别:?}",
    # 时间轴的自绘文字：刻度值、"实体 · 属性"标签。
    # 可见部分全是用户数据与数字，编辑器不生成任何可翻译文字。
    "{时间:.1}",
    "{时间:.2}",
    "{时长:.2}",
    "{实体名} · {属性名}",
    # 时间轴标尺刻度值：纯数字排版，不含可翻译文字。
    "{刻度时间:.1}",
    # 打包失败项：可见部分全是文件路径与系统错误文本，不含编辑器文案。
    "{文件}：{原因}",
    # ---- ③ 用户数据拼接 ----
    #
    # 可见部分是 emoji 与用户自己的资源名，编辑器不生成任何文字。
    "📁 {名称}",
    # ---- ④ 数据与序列化名 ----
    #
    # 场景文件里的字段名 / 枚举判别值，翻译会破坏格式兼容性。
    "未命名场景",
    "实体标识::新建",
    "{}_副本",
    "{期望}{序号}",
    # ---- ⑤ 非界面文案的技术说明 ----
    "RON（场景与项目清单的序列化格式）",
    "egui 0.36 + egui_dock 0.21（界面与可停靠布局）",
    # 语言自称：各语言用自己的写法显示，不属于翻译范围。
    "简体中文",
    # ---- ⑥ 本轮明确排除：错误文案 ----
    #
    # `scene_doc` / `command.rs` 的错误消息有意留在本轮之外。
    # 已确认的下一轮方案是**让错误携带键与参数、由界面渲染**，
    # 而不是引入一个全局"当前语言"。
    # 在完成那一步之前，英文模式下这些错误仍是中文 ——
    # 这是已知缺口，记录在 docs/工作日志.md；这里如实排除，
    # 而不是把它当成"已经覆盖"来自欺。
    "实体 {标识} 不存在",
    "实体 {标识} 已有组件 `{}`",
    "实体 {标识} 没有组件 `{类型名}`",
    "父实体 {父} 不存在",
}


def 反转义(s):
    """把 TSV 里的转义序列还原成真实字符。

    为什么需要这一步：TSV 是纯文本、一行一条，因此换行只能用 `\\n`
    这两个字符来表示。若直接把这两个字符交给 [`转义`]，
    反斜杠会被再转义一次，Rust 源码里就变成 `\\\\n`，
    运行时渲染出的是**可见的 `\\n` 两个字符**而不是换行。

    这个 bug 真的发生过：7 条文案（含空状态提示）在界面上显示成
    `没有资源。\\n把图片拖进来`。因此这里先还原成真实换行，
    再由 [`转义`] 统一按 Rust 字面量规则重新转义。
    """
    return s.replace("\\n", "\n").replace("\\t", "\t").replace('\\"', '"')


def 读取词条():
    """读 messages.tsv，返回 [(键, 中文, 英文)]。"""
    行列表 = []
    with io.open(数据文件, encoding="utf-8") as f:
        for 序号, 原始 in enumerate(f, 1):
            行 = 原始.rstrip("\n").rstrip("\r")
            if not 行.strip() or 行.lstrip().startswith("#"):
                continue
            列 = 行.split("\t")
            if len(列) != 3:
                raise SystemExit(
                    f"{数据文件}:{序号} 应当有 3 列（键/中文/英文，制表符分隔），"
                    f"实际 {len(列)} 列：{行!r}"
                )
            # 只对**键**做 strip。
            #
            # 文案列的首尾空白是**有意义**的：界面里有些文案靠前导空格做对齐
            # （例如菜单项未选中时用 "   " 占住勾选标记的位置）。
            # 早先这里对三列一律 strip，结果是"源码里的文案与词表里的对不上"，
            # 覆盖核对会误报成漏翻译 —— 这个 bug 也真的发生过。
            键 = 列[0].strip()
            中 = 反转义(列[1])
            英 = 反转义(列[2])
            行列表.append((键, 中, 英))
    return 行列表


def 校验词条(行列表):
    错 = []
    已见 = set()
    for 键, 中, 英 in 行列表:
        if not 键:
            错.append("存在空键")
        elif not re.fullmatch(r"[a-z][a-z0-9_]*", 键):
            错.append(f"键 `{键}` 不是规范的小写 ASCII 标识符")
        elif 键.endswith("_"):
            错.append(f"键 `{键}` 不应以下划线结尾")
        if 键 in 已见:
            错.append(f"键 `{键}` 重复")
        已见.add(键)
        if not 中:
            错.append(f"`{键}` 中文为空")
        if not 英:
            错.append(f"`{键}` 英文为空")
    return 错


def 扫描源码中文():
    """扫描目标文件，返回 {中文文案: [文件]}（不含测试与注释）。

    # 关键：必须支持**跨行字符串字面量**

    Rust 允许用行尾反斜杠续行，于是一句界面文案可能横跨好几行源码::

        egui::RichText::new(
            "本版本已实现：项目建立/打开、场景编辑与保存、场景树、检查器、\\
             2D 视口、资源浏览、撤销重做、控制台。",
        )

    这里早先用 `[^"\\\\\\n]*` 匹配字面量内容，**遇到换行就停**，
    因此这类文案被整段漏掉 —— 既没进词表、也没被检查发现，
    于是它们在英文模式下一直是中文。实际漏了 5 条（含关于窗口、
    历史与性能面板的空状态说明）。

    现在用 `(?:[^"\\\\]|\\\\.)*` 配 `re.S`：`\\\\.` 能吃掉
    "反斜杠 + 换行"的续行，`[^"\\\\]` 也能匹配普通换行。
    """
    结果 = {}
    字面量 = re.compile(r'"((?:[^"\\]|\\.)*)"', re.S)
    for 相对 in 扫描目标:
        路径 = os.path.join(仓库根, 相对.replace("/", os.sep))
        if not os.path.isfile(路径):
            continue
        正文 = io.open(路径, encoding="utf-8").read()
        i = 正文.find("#[cfg(test)]")
        if i > 0:
            正文 = 正文[:i]
        正文 = re.sub(r"//[^\n]*", "", 正文)
        for m in 字面量.finditer(正文):
            s = m.group(1)
            if any("\u4e00" <= c <= "\u9fff" for c in s) and len(s) < 400:
                结果.setdefault(s, set()).add(相对)
    return 结果


def 转义(s):
    """按 Rust 字符串字面量的规则转义。

    顺序很重要：先把真实反斜杠转义掉，再把换行/制表符变成转义序列。
    反过来做的话，`\\n` 里的反斜杠会被再转义一次，
    运行时渲染出可见的 `\\n` 而不是换行。
    """
    return (
        s.replace("\\", "\\\\")
        .replace("\n", "\\n")
        .replace("\t", "\\t")
        .replace('"', '\\"')
    )


def 生成(行列表):
    # 按 UTF-8 字节序排序，与 Rust 的 str 序一致（二分查找的前提）
    排序后 = sorted(行列表, key=lambda r: r[0].encode("utf-8"))
    行 = [
        "//! 翻译词表。**本文件由脚本生成，请勿手工编辑。**",
        "//!",
        "//! 真相来源：`crates/editor_core/i18n/messages.tsv`",
        "//! 重新生成：`python tools/gen_messages.py`",
        "//!",
        "//! 排序方式：按 `键` 的 UTF-8 字节序升序 —— 查找用二分，",
        "//! 顺序错了会静默查不到，因此由脚本保证而不是靠人维护。",
        "",
        "use super::词条;",
        "",
        "/// 全部词条，按 `键` 的字节序升序。",
        "///",
        "/// `#[rustfmt::skip]`：让 rustfmt 不要重排这几百行。",
        "/// 否则 `cargo fmt` 会改动生成物，`--check` 与数据文件比对时就永远"
        "不一致，",
        "/// 而那个检查本来是用来防止有人手改生成物的 —— 一个总是失败的检查等于没有检查。",
        "#[rustfmt::skip]",
        "pub static 词表: &[词条] = &[",
    ]
    for 键, 中, 英 in 排序后:
        行.append(
            f'    词条 {{ 键: "{转义(键)}", 中文: "{转义(中)}", 英文: "{转义(英)}" }},'
        )
    行.append("];")
    行.append("")
    return "\n".join(行)


def 扫描调用点键():
    """扫描全部 Rust 源码，取出所有 `语言.取("键")` / `语言.填充("键"` 用到的键。

    返回 {键: {文件}}。

    这是**迁移完成之后**真正有用的那个方向：
    源里已经没有中文了，但可能出现"调用了一个词表里不存在的键"
    （拼错、或改名时漏改一处）。`取` 查不到会返回键本身，
    界面上显示 `menu_file_save_scene` —— 能看出来，但最好在测试阶段就挡住。
    """
    结果 = {}
    模式 = re.compile(r'语言\s*\.\s*(?:取|填充)\(\s*"([a-z0-9_]+)"')
    根目录 = os.path.join(仓库根, "crates")
    for 当前, _, 文件列表 in os.walk(根目录):
        for 名 in 文件列表:
            if not 名.endswith(".rs"):
                continue
            路径 = os.path.join(当前, 名)
            相对 = os.path.relpath(路径, 仓库根).replace(os.sep, "/")
            if 相对.endswith("i18n/table.rs"):
                continue
            正文 = io.open(路径, encoding="utf-8").read()
            for m in 模式.finditer(正文):
                结果.setdefault(m.group(1), set()).add(相对)
    return 结果


def 核对调用点(行列表):
    表内 = {键 for 键, _, _ in 行列表}
    调用 = 扫描调用点键()
    未定义 = {k: v for k, v in 调用.items() if k not in 表内}
    if 未定义:
        print(
            f"以下 {len(未定义)} 个键被调用但**不在词表里**"
            "（`取` 会把键名原样显示到界面上，请检查拼写或补词条）：",
            file=sys.stderr,
        )
        for k in sorted(未定义):
            print(f"  - {k}   [{', '.join(sorted(未定义[k]))}]", file=sys.stderr)
        return 1
    print(f"调用点核对通过：源码共引用 {len(调用)} 个键，全部在词表里")

    未引用 = 表内 - set(调用)
    if 未引用:
        print(f"提示：{len(未引用)} 条词条当前没有任何调用点（不算错，供清理参考）：")
        for k in sorted(未引用)[:12]:
            print(f"  · {k}")
    return 0


def 核对裸键(行列表):
    """找"显示函数收到裸键字面量"的位置。

    # 为什么需要这条

    批量迁移时最容易犯的错不是漏翻，而是**把中文字面量换成了键、
    却忘了套 `语言.取()`**。于是界面直接显示 `save_scene_2` 这样的键名。

    这种错误：
    * 编译通过；
    * 词表完整、调用点核对也通过（因为压根没有"调用"）；
    * 只有真的打开窗口才看得见。

    界面上的键名是最典型的"迁移漏了包装"，所以专门扫一遍：
    在显示函数的参数位置出现了**词表里的键**，说明这里应当是
    `语言.取("键")` 而不是裸的 `"键"`。
    """
    键集 = {键 for 键, _, _ in 行列表}
    # 允许 `ui` 与 `.button` 之间出现换行 —— rustfmt 经常把链式调用拆行。
    # 早先的正则要求两者紧邻，于是漏掉了
    #     ui
    #         .button("new_entity_3")
    # 这种写法，界面上就真的出现了一个裸键 `new entity_3`。
    展示函数 = (
        r"(?:ui\s*\.\s*(?:label|button|small_button|heading))",
        r"(?:egui::)?(?:RichText|Button)::new",
        r"\w+\s*\.\s*on_hover_text",
    )
    模式 = re.compile(
        r"\b(" + "|".join(展示函数) + r')\s*\(\s*"([a-z0-9_]+)"'
    )

    命中 = []
    根目录 = os.path.join(仓库根, "crates")
    for 当前, _, 文件列表 in os.walk(根目录):
        for 名 in 文件列表:
            if not 名.endswith(".rs"):
                continue
            路径 = os.path.join(当前, 名)
            相对 = os.path.relpath(路径, 仓库根).replace(os.sep, "/")
            正文 = io.open(路径, encoding="utf-8").read()
            for m in 模式.finditer(正文):
                if m.group(2) in 键集:
                    行号 = 正文[: m.start()].count("\n") + 1
                    命中.append(f"{相对}:{行号}  {m.group(0)}")

    if 命中:
        print(
            f"以下 {len(命中)} 处显示函数收到的是**裸键字面量**，"
            "界面会直接显示键名。应当写成 `语言.取(\"键\")`：",
            file=sys.stderr,
        )
        for 位置 in 命中:
            print(f"  - {位置}", file=sys.stderr)
        return 1
    print("裸键核对通过：显示函数没有收到未包装的键字面量")
    return 0


def 主流程():
    只核对 = "--check" in sys.argv
    行列表 = 读取词条()
    print(f"词条数据：{len(行列表)} 条")

    错 = 校验词条(行列表)
    if 错:
        print("词条数据有问题：", file=sys.stderr)
        for e in 错:
            print("  -", e, file=sys.stderr)
        return 1

    新内容 = 生成(行列表)
    if 只核对:
        旧 = io.open(输出文件, encoding="utf-8").read() if os.path.isfile(输出文件) else ""
        if 旧 != 新内容:
            print(
                "词表与 messages.tsv 不一致：请运行 `python tools/gen_messages.py` 重新生成",
                file=sys.stderr,
            )
            return 1
        print("词表与数据文件一致")
    else:
        os.makedirs(os.path.dirname(输出文件), exist_ok=True)
        io.open(输出文件, "w", encoding="utf-8", newline="\n").write(新内容)
        print(f"已生成 {os.path.relpath(输出文件, 仓库根)}")

    # ---- 中文覆盖核对 ----
    表内中文 = {中 for _, 中, _ in 行列表}
    源码 = 扫描源码中文()
    未覆盖 = {s: fs for s, fs in 源码.items() if s not in 表内中文 and s not in 应排除}
    if 未覆盖:
        print(
            f"以下 {len(未覆盖)} 句界面中文在词表里没有对应词条"
            "（新增文案后请在 messages.tsv 补一行）：",
            file=sys.stderr,
        )
        for s in sorted(未覆盖):
            print(f"  - {s[:90]}   [{', '.join(sorted(未覆盖[s]))}]", file=sys.stderr)
        return 1

    僵尸 = 表内中文 - set(源码) - 应排除
    if 僵尸:
        print(f"提示：{len(僵尸)} 条词条在当前扫描范围内已不再使用（不算错）：")
        for s in sorted(僵尸)[:10]:
            print(f"  · {s[:80]}")

    print(f"中文覆盖核对通过：扫描 {len(源码)} 句中文，全部有词条")

    # 迁移之后，更重要的方向是"调用点引用的键是否存在"。
    # 源里已经没有中文了，但键可能拼错或改名时漏改，
    # 那种情况下界面会显示键名。
    if 核对调用点(行列表) != 0:
        return 1
    return 核对裸键(行列表)


if __name__ == "__main__":
    sys.exit(主流程())
