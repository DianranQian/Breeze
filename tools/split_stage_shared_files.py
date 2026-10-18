#!/usr/bin/env python3
"""把"本轮改动"与"其他工作线的未提交改动"从同一个文件里分离出来再暂存。

# 为什么需要这个脚本

`viewport_panel.rs` 与 `viewport3d.rs` 同时含有：
* 本轮新增的操纵器驱动/绘制代码；
* **另一条工作线**尚未提交的改动（瓦片地形/几何体尺寸、3D 自动适应窗口公式）。

直接用 `git add <文件>` 会把别人的改动一起提交；而 `git add -p` 需要交互。
这里的做法是：合成一份"HEAD + 仅本轮改动"的内容，写成 git 对象后只更新索引
（`git update-index --cacheinfo`）。**工作区文件不动**，因此另一条工作线的
改动原样留在工作区，只是没有被暂存 —— 这正是我们要的状态。
"""

from __future__ import annotations

import pathlib
import subprocess

仓库 = pathlib.Path(__file__).resolve().parents[1]
输出 = 仓库 / ".verify_v2" / "runs" / "synth"

# (路径, 其他工作线改动区间的起止标记)；止标记为 None 表示"到文件末尾"。
区间 = [
    (
        "crates/editor_ui/src/viewport_panel.rs",
        "pub fn 计算实体框",
        "/// 命中测试：返回世界坐标点下最靠上的实体。",
    ),
    (
        "crates/editor_ui/src/viewport_panel.rs",
        "mod 测试 {",
        None,
    ),
    (
        "crates/editor_ui/src/viewport3d.rs",
        "/// 把相机对准整个场景。",
        "/// 屏幕空间最近的实体。",
    ),
]


def 取_head(路径: str) -> str:
    结果 = subprocess.run(
        ["git", "show", f"HEAD:{路径}"],
        cwd=仓库,
        capture_output=True,
        check=True,
    )
    return 结果.stdout.decode("utf-8")


def 区间切片(文本: str, 起: str, 止: str | None) -> tuple[int, int]:
    开头 = 文本.index(起)
    结尾 = len(文本) if 止 is None else 文本.index(止, 开头)
    return 开头, 结尾


# 同一文件可能有多段：先按文件分组，逐段替换后再统一写对象。
按文件: dict[str, list[tuple[str, str | None]]] = {}
for 路径, 起, 止 in 区间:
    按文件.setdefault(路径, []).append((起, 止))

for 路径, 段列表 in 按文件.items():
    合成 = (仓库 / 路径).read_text(encoding="utf-8")
    head = 取_head(路径)
    for 起, 止 in 段列表:
        当前起, 当前止 = 区间切片(合成, 起, 止)
        head起, head止 = 区间切片(head, 起, 止)
        # 把当前内容里"别人的那一段"换回 HEAD 的版本。
        合成 = 合成[:当前起] + head[head起:head止] + 合成[当前止:]

    输出.mkdir(parents=True, exist_ok=True)
    临时 = 输出 / pathlib.Path(路径).name
    临时.write_text(合成, encoding="utf-8", newline="\n")

    # 写入 git 对象并只更新索引。
    blob = subprocess.run(
        ["git", "hash-object", "-w", f"--path={路径}", str(临时)],
        cwd=仓库,
        capture_output=True,
        check=True,
        text=True,
    ).stdout.strip()
    subprocess.run(
        ["git", "update-index", "--cacheinfo", f"100644,{blob},{路径}"],
        cwd=仓库,
        check=True,
    )
    print(f"已暂存「HEAD + 本轮改动」：{路径}  blob={blob[:12]}")
