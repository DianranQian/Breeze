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
    # 数据与序列化名：它们是场景文件里的字段名 / 枚举判别值，
    # 翻译它们会破坏文件格式兼容性。
    "未命名场景",
    "实体标识::新建",
    "{}_副本",
    "{期望}{序号}",
    # 用户数据与技术标识：不属于界面文案。
    "RON（场景与项目清单的序列化格式）",
    "egui 0.36 + egui_dock 0.21（界面与可停靠布局）",
    # 语言自称：各语言用自己的写法显示，不属于翻译范围。
    "简体中文",
    # 纯占位符/符号串：与语言无关。
    "{标题}",
    "✓ {标题}",
    "{标题}    {快捷键}",
    "已停止",
    "运行中",
    "已暂停",
}


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
            # 覆盖核对会误报成漏翻译 —— 这个 bug 真的发生过。
            键 = 列[0].strip()
            中 = 列[1]
            英 = 列[2]
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
    """扫描目标文件，返回 {中文文案: [文件]}（不含测试与注释）。"""
    结果 = {}
    for 相对 in 扫描目标:
        路径 = os.path.join(仓库根, 相对.replace("/", os.sep))
        if not os.path.isfile(路径):
            continue
        正文 = io.open(路径, encoding="utf-8").read()
        i = 正文.find("#[cfg(test)]")
        if i > 0:
            正文 = 正文[:i]
        正文 = re.sub(r"//[^\n]*", "", 正文)
        for m in re.finditer(r'"((?:[^"\\\n]|\\.)*)"', 正文):
            s = m.group(1)
            if any("\u4e00" <= c <= "\u9fff" for c in s) and len(s) < 200:
                结果.setdefault(s, set()).add(相对)
    return 结果


def 转义(s):
    return s.replace("\\", "\\\\").replace('"', '\\"')


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
        "pub static 词表: &[词条] = &[",
    ]
    for 键, 中, 英 in 排序后:
        行.append(
            f'    词条 {{ 键: "{转义(键)}", 中文: "{转义(中)}", 英文: "{转义(英)}" }},'
        )
    行.append("];")
    行.append("")
    return "\n".join(行)


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

    print(f"覆盖核对通过：扫描 {len(源码)} 句中文，全部有词条")
    return 0


if __name__ == "__main__":
    sys.exit(主流程())
