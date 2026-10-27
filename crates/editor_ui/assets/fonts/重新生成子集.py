#!/usr/bin/env python3
"""重新生成内置中文字体子集。

中文名：重新生成子集

把上游的可变字体固定到 wght=400，再按字符集裁剪出
`crates/editor_ui/assets/fonts/NotoSansSC-Subset.ttf`。

# 这份子集负责哪些字

只负责**中文排版**：汉字、中文标点、全角字符、ASCII，以及 `·` 与 `×`。
拉丁字母、箭头、数学符号与图标（`↶` `≠` `●` `✎` 之类）来自 egui 内置的
Latin 字体与图标字体，不在本子集的职责内 —— 硬塞进中文字体只会得到
"上游本来就没有这个字形"的空结果。

字符集是三部分的并集：

1. **GB2312 全部汉字**（6763 个）—— 实体名、场景名是用户输入，
   只收源码里出现过的字，会让用户给实体起名时看到方框；
2. **界面源里的中文排版字符** —— 扫描 `crates/editor_ui/src/**/*.rs`、
   `crates/editor_core/i18n/**/*.tsv`（全部界面文案的词条源）与仓库根
   `README.md`，只保留汉字、U+2000–U+206F（中文引号/破折号/省略号）、
   U+3000–U+303F（CJK 标点）、U+FF00–U+FFEF（全角）与 `·` `×`；
3. **ASCII 可见字符** —— 数字、路径分隔符、括号，运行时会拼进界面。

只扫界面源而不是全仓库：测试里的日文样例、文档注释里的箭头图标都不是
界面文案，把它们算进字符集只会让"缺失"报告充满噪音。

重建时还会**尽量**带上 GB2312 符号区（箭头、几何图形等），避免重新生成
反而丢掉已有字形；上游没有的区位自动跳过，也不计入 `--check` 的必保集。

# 用法

    python crates/editor_ui/assets/fonts/重新生成子集.py
    python crates/editor_ui/assets/fonts/重新生成子集.py --out target/verify/x.ttf
    python crates/editor_ui/assets/fonts/重新生成子集.py --check

`--check` 只读现有字体，列出目标字符集里缺字的部分；不重建、不需要上游字体。
重建需要本机装有上游可变字体（默认 `C:\\Windows\\Fonts\\NotoSansSC-VF.ttf`，
可用 `--source` 或环境变量 `BEVY_STUDIO_FONT_SOURCE` 覆盖）。

依赖：`fonttools`（`pip install fonttools`）。
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

字体文件 = "NotoSansSC-Subset.ttf"
默认上游 = Path(r"C:\Windows\Fonts\NotoSansSC-VF.ttf")
静止字重 = 400
# 脚本位于 <仓库根>/crates/editor_ui/assets/fonts/。
仓库根 = Path(__file__).resolve().parents[4]
字体目录 = Path(__file__).resolve().parent
# 界面文案的真正来源：绘制代码 + i18n 词条。测试与文档注释不参与。
界面源 = (
    "crates/editor_ui/src/**/*.rs",
    "crates/editor_core/i18n/**/*.tsv",
)
额外说明 = "README.md"


def GB2312区(起区: int, 止区: int) -> str:
    """按 GB2312 的区位枚举一个区段，跳过未定义区位。"""
    结果 = []
    for 区 in range(起区, 止区 + 1):
        for 位 in range(0xA1, 0xFF):
            try:
                结果.append(bytes((区, 位)).decode("gb2312"))
            except UnicodeDecodeError:
                continue
    return "".join(结果)


def GB2312汉字() -> str:
    """GB2312 的汉字区（B0–F7，6763 字）。"""
    return GB2312区(0xB0, 0xF7)


def GB2312符号() -> str:
    """GB2312 的符号区（A1–A9：标点、箭头、几何图形、拼音等）。

    这一区按**尽力而为**处理：上游 Noto Sans SC 本身并不含全部区位
    （例如带声调的拼音、西里尔字母），缺的字形无从补出。它们不列入
    `--check` 的必保集，因为界面上这类符号本来就由 egui 的 Latin 字体兜底。
    """
    return GB2312区(0xA1, 0xA9)


def 是中文排版字符(字符: str) -> bool:
    """汉字、中文标点、全角字符，以及界面里当作中文标点用的 `·` 与 `×`。"""
    码位 = ord(字符)
    return (
        0x3400 <= 码位 <= 0x4DBF  # CJK 扩展 A
        or 0x4E00 <= 码位 <= 0x9FFF  # CJK 基本区
        or 0x2000 <= 码位 <= 0x206F  # 常用标点（— … “ ” 等）
        or 0x3000 <= 码位 <= 0x303F  # CJK 标点（、。「」等）
        or 0xFF00 <= 码位 <= 0xFFEF  # 全角字符
        or 码位 in (0x00B7, 0x00D7)  # 间隔号 · / 乘号 ×
    )


def 界面源字符() -> set[str]:
    """收集界面绘制代码、i18n 词条与仓库 README 里的中文排版字符。"""
    文件表: list[Path] = []
    for 模式 in 界面源:
        文件表.extend(sorted(仓库根.glob(模式)))
    根说明 = 仓库根 / 额外说明
    if 根说明.is_file():
        文件表.append(根说明)
    if not 文件表:
        raise SystemExit(f"在 {仓库根} 下找不到任何界面源文件，检查脚本位置")
    字符 = set()
    for 路径 in 文件表:
        try:
            字符.update(路径.read_text(encoding="utf-8"))
        except (UnicodeDecodeError, OSError) as 错误:
            raise SystemExit(f"读不了 {路径}：{错误}")
    return {字符 for 字符 in 字符 if 字符.isprintable() and 是中文排版字符(字符)}


def 必保字符集() -> tuple[str, int]:
    """返回（必须覆盖的字符集, 其中界面源字符的个数）。`--check` 认的就是这一份。"""
    源码 = 界面源字符()
    字集 = (
        源码
        | set(GB2312汉字())
        | {chr(码位) for 码位 in range(0x20, 0x7F)}
    )
    return "".join(sorted(字集)), len(源码)


def 重建字符集(必保: str) -> str:
    """重建时多带上 GB2312 符号区，让子集不因重新生成而丢掉已有的箭头/几何图形。"""
    return "".join(sorted(set(必保) | set(GB2312符号())))


def 固定字重(字体) -> None:
    from fontTools.varLib import instancer

    if not any(轴.axisTag == "wght" for 轴 in 字体["fvar"].axes):
        return
    instancer.instantiateVariableFont(字体, {"wght": 静止字重}, inplace=True)


def 裁剪(字体, 字符集: str) -> None:
    from fontTools import subset

    选项 = subset.Options()
    选项.layout_features = ["*"]
    选项.name_IDs = ["*"]
    选项.name_legacy = True
    选项.name_languages = ["*"]
    选项.notdef_outline = True
    选项.recalc_bounds = True
    选项.recalc_timestamp = False
    选项.glyph_names = False
    裁剪器 = subset.Subsetter(options=选项)
    裁剪器.populate(unicodes=sorted({ord(字符) for 字符 in 字符集}))
    裁剪器.subset(字体)


def 缺字(字体路径: Path, 字符集: str) -> list[str]:
    from fontTools.ttLib import TTFont

    with TTFont(str(字体路径), lazy=True) as 字体:
        覆盖 = set()
        for 表 in 字体["cmap"].tables:
            覆盖.update(表.cmap)
    return [字符 for 字符 in 字符集 if ord(字符) not in 覆盖]


def 执行检查(字体路径: Path) -> int:
    if not 字体路径.is_file():
        print(f"找不到字体：{字体路径}", file=sys.stderr)
        return 2
    必保, _ = 必保字符集()
    缺失 = 缺字(字体路径, 必保)
    if 缺失:
        print(
            f"缺失 {len(缺失)} 个字符：{''.join(缺失)}\n"
            "重新生成：python " + str(Path(__file__).relative_to(仓库根)),
            file=sys.stderr,
        )
        return 1
    print(f"{字体路径.name} 覆盖全部 {len(必保)} 个必保字符")
    return 0


def 重建(上游: Path, 输出: Path) -> int:
    from fontTools.ttLib import TTFont

    if not 上游.is_file():
        print(
            f"找不到上游可变字体：{上游}\n"
            "用 --source 指定，或设置 BEVY_STUDIO_FONT_SOURCE。",
            file=sys.stderr,
        )
        return 2
    必保, 源码数 = 必保字符集()
    字符集 = 重建字符集(必保)
    print(
        f"必保字符：{len(必保)}（界面源汉字与标点 {源码数}，"
        f"GB2312 汉字 6763 + ASCII）；含 GB2312 符号区后共 {len(字符集)}"
    )

    字体 = TTFont(str(上游))
    固定字重(字体)
    裁剪(字体, 字符集)
    输出.parent.mkdir(parents=True, exist_ok=True)
    字体.save(str(输出))
    字体.close()

    缺失 = 缺字(输出, 必保)
    if 缺失:
        print(f"重建结果仍缺 {len(缺失)} 个必保字符：{''.join(缺失)}", file=sys.stderr)
        return 1
    大小 = 输出.stat().st_size / 1024 / 1024
    print(f"已写出 {输出}（{大小:.2f} MB，覆盖全部必保字符）")
    return 0


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")

    解析器 = argparse.ArgumentParser(description="重建内置中文字体子集")
    解析器.add_argument(
        "--source",
        type=Path,
        default=Path(os.environ.get("BEVY_STUDIO_FONT_SOURCE") or 默认上游),
        help="上游可变字体（默认 %(default)s）",
    )
    解析器.add_argument(
        "--out",
        type=Path,
        default=字体目录 / 字体文件,
        help="输出字体（默认 %(default)s）",
    )
    解析器.add_argument(
        "--check",
        action="store_true",
        help="只校验 --out 指向的字体是否覆盖目标字符集，不重建",
    )
    参数 = 解析器.parse_args()
    if 参数.check:
        return 执行检查(参数.out)
    return 重建(参数.source, 参数.out)


if __name__ == "__main__":
    raise SystemExit(main())
