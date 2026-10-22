#!/usr/bin/env bash
#
# R15 资源预览的真实 GUI 验收脚本（可重跑）。
#
# 它做四件事，全部基于**真实程序**与**真实资源**：
#   1. 建临时验收项目 `.verify_r15/project`，资源全部从真实项目复制
#      （来源见 `.verify_r15/SOURCES.md`）；
#   2. 编译 `bevy-studio` 与 `gui_probe`；
#   3. 用 `gui_probe drag` 注入真实鼠标点击（SendInput），由应用自己
#      回读渲染目标写出 PNG + 证据 JSON；
#   4. 汇总证据生成机器可读报告 `.verify_r15/acceptance.json`。
#
# 用法（仓库根目录）：
#   bash tools/asset_preview_acceptance.sh
#
# 前置：Windows + 可交互桌面（真实输入注入需要前台窗口）。

set -uo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT" || exit 1

VERIFY=.verify_r15
# 窗口尺寸必须小于屏幕：系统把新窗口层叠放置，窗口一旦比屏幕大，
# 连续启动几次就会有一部分移到屏幕外，注入的点击随之落到屏幕外
# （证据里是 `received_pointer=false`）。
WIN_SIZE=${BEVY_STUDIO_WINDOW_SIZE:-1440x900}
PROJECT="$VERIFY/project"
CFG="$ROOT/.tmp/资产预览验收配置"


echo "== 1/4 准备临时验收项目 =="
rm -rf "$PROJECT"
mkdir -p "$PROJECT/assets" "$PROJECT/scenes" "$CFG" "$PROJECT/assets/tps/characters" "$PROJECT/assets/tps/environment"

copy_asset() { # 源 → 目标
  if [ ! -f "$1" ]; then
    echo "缺少真实资源：$1" >&2
    return 1
  fi
  cp "$1" "$2"
}
copy_asset examples/3d_demo/assets/tps/environment/floor.png              "$PROJECT/assets/floor.png"
copy_asset examples/3d_demo/assets/tps/environment/wall.png               "$PROJECT/assets/wall.png"
copy_asset examples/3d_demo/assets/tps/environment/Crate.gltf              "$PROJECT/assets/Crate.gltf"
copy_asset examples/3d_demo/assets/tps/characters/Character_Soldier.gltf   "$PROJECT/assets/Character_Soldier.gltf"
copy_asset examples/3d_demo/scenes/tps_arena.scene.ron                     "$PROJECT/scenes/tps_arena.scene.ron"
copy_asset crates/editor_ui/assets/fonts/NotoSansSC-Subset.ttf             "$PROJECT/assets/NotoSansSC-Subset.ttf"
copy_asset .verify/runs/audio_demo/assets/audio/beep.wav                   "$PROJECT/assets/beep.wav"
copy_asset examples/3d_demo/ASSETS.md                                      "$PROJECT/assets/NOTES.md"
# 场景预览用的真实场景会引用 `assets/tps/...` 下的资产。若这些文件缺失，
# 打开场景会被标脏并在退出时写恢复点，下次启动弹出"恢复自动保存"对话框，
# 把资源列表整个盖住 —— 那时自动点击会落在对话框上（received_pointer=false）。
# 因此连同这些真实资产一起复制，让工程自洽（仍然全部是真实文件的副本）。
copy_asset examples/3d_demo/assets/tps/characters/Character_Enemy.gltf   "$PROJECT/assets/tps/characters/Character_Enemy.gltf"
copy_asset examples/3d_demo/assets/tps/characters/Character_Hazmat.gltf  "$PROJECT/assets/tps/characters/Character_Hazmat.gltf"
copy_asset examples/3d_demo/assets/tps/characters/Character_Soldier.gltf "$PROJECT/assets/tps/characters/Character_Soldier.gltf"
for 文件 in Barrier_Single Container_Long Container_Small Crate ExplodingBarrel SackTrench TrafficCone; do
  copy_asset "examples/3d_demo/assets/tps/environment/$文件.gltf" "$PROJECT/assets/tps/environment/$文件.gltf"
done
copy_asset examples/3d_demo/assets/tps/environment/floor.png            "$PROJECT/assets/tps/environment/floor.png"
copy_asset examples/3d_demo/assets/tps/environment/wall.png             "$PROJECT/assets/tps/environment/wall.png"
printf '\x00\x01\x02\x03binary' > "$PROJECT/assets/unknown.bin"
cat > "$PROJECT/project.ron" << 'RON'
(
    格式版本: 1,
    名称: "R15 资源预览验收",
    说明: "临时验收项目：资源全部从真实项目复制，来源见 .verify_r15/SOURCES.md。",
    启动场景: None,
)
RON

echo "== 2/4 构建 =="
cargo build -p editor_app --bin bevy-studio || exit 1
cargo build -p gui_probe || exit 1

# PowerShell 不认 MSYS 的 `/c/...` 路径；用相对路径最稳
# （它的当前工作目录就是仓库根）。
EXE="./target/debug/bevy-studio.exe"
PROBE="./target/debug/gui_probe.exe"
PROJECT_ABS=$(cd "$PROJECT" && pwd -W 2>/dev/null || pwd)
CFG_ABS=$(cd "$CFG" && pwd -W 2>/dev/null || pwd)
REAL_EXAMPLE=$(cd examples/3d_demo && pwd -W 2>/dev/null || pwd)

# 当前会话要打开的项目：默认是临时验收项目，real3d 会话会临时改成真实示例。
CURRENT_PROJECT="$PROJECT_ABS"

LOG="$VERIFY/gui_gate.log"
: > "$LOG"

# 等上一次启动的编辑器真的退出。
#
# 不等待时下一次会话会在"旧进程还没释放前台"的窗口上注入，
# 表现是 `received_pointer=false`、退出码 3 —— 点到的其实是一个正在关闭的窗口。
等编辑器退出() {
  local count=0
  while [ "$count" -lt 40 ]; do
    if ! tasklist 2>/dev/null | grep -qi "bevy-studio"; then
      sleep 1
      return 0
    fi
    sleep 0.5
    count=$((count + 1))
  done
  echo "上一个编辑器实例在 20 秒内没有退出，继续尝试下一会话" >> "$LOG"
  return 1
}

# 跑一次会话：名字 + gui_probe 参数。stdout 进总日志，证据留在会话目录。
run_session() {
  local name=$1
  shift
  等编辑器退出
  rm -rf "$VERIFY/$name"
  mkdir -p "$VERIFY/$name"
  echo "-------- run_session $name --------" >> "$LOG"
  local attempt=0
  local rc=1
  # 真实输入注入对"前台窗口"敏感：残留的旧编辑器会抢走前台，
  # 表现为 `received_pointer=false`。失败后清掉残留实例重试一次，
  # 并在日志里如实记录"重试过"，不把首次失败藏起来。
  while [ "$attempt" -lt 2 ]; do
    # 恢复点会让启动时弹出"恢复自动保存"对话框，把资源列表整个盖住；
    # 那时点击落在对话框上，表现就是 `received_pointer=false`。
    # 每次尝试前都清掉，包括重试（第一次被中断时也会留下恢复点）。
    rm -rf "$PROJECT/.breeze/recovery"
    # 每个会话用独立配置目录：配置会持久化窗口位置，跨会话复用会让窗口越挪越偏。
    rm -rf "$CFG_ABS/$name"
    mkdir -p "$CFG_ABS/$name"
    # 每个会话必须在**独立进程**里启动。
    #
    # 实测：在同一个 bash 进程里连续 spawn `gui_probe`，从第二个会话起
    # `received_pointer=false` —— 注入的点击没到达客户区，而坐标、前台
    # 激活与截图都正常；改成每条命令一个新 `bash -c` 后连续会话全部通过。
    # 这与"人工逐条执行"的输入上下文一致，因此用它而不是依赖 PowerShell。
    ( cd "$ROOT" && bash -c "$PROBE drag --exe $EXE --out $VERIFY/$name --name $name --timeout 240 $* --env BEVY_STUDIO_OPEN_PROJECT=$CURRENT_PROJECT --env BEVY_STUDIO_WORKSPACE=assets --env BEVY_STUDIO_LANGUAGE=zh --env BEVY_STUDIO_CONFIG_DIR=$CFG_ABS/$name --env BEVY_STUDIO_WINDOW_SIZE=${WIN_SIZE:-1440x900} --allow-no-change" ) >> "$LOG" 2>&1
    rc=$?
    if [ "$rc" -eq 0 ]; then
      break
    fi
    attempt=$((attempt + 1))
    echo "run_session $name 第 $attempt 次失败（退出码 $rc），等待实例退出后重试" >> "$LOG"
    等编辑器退出
    sleep 4
  done
  echo "run_session $name 退出码=$rc 尝试次数=$((attempt + 1))" >> "$LOG"
  # 等本次实例**正常**退出，而不是立刻强杀：
  # 强杀会让窗口在前台锁上留下"正在销毁"的状态，下一次会话的
  # `SetForegroundWindow` 拿不到前台，注入的点击就落到别处。
  等编辑器退出
  sleep 4
}

echo "== 3/4 真实鼠标点击各类型资源 =="
run_session img         --frame 150 --frame2 360 --case asset:floor.png --case2 asset:wall.png
run_session model       --frame 150 --frame2 560 --case asset:Character_Soldier.gltf
run_session scene       --frame 150 --frame2 340 --case asset:tps_arena.scene.ron
run_session audio       --frame 150 --frame2 340 --frame3 560 --case asset:beep.wav --case3 click:asset_preview_audio
run_session font        --frame 150 --frame2 380 --case asset:NotoSansSC-Subset.ttf
run_session unsupported --frame 150 --frame2 320 --case asset:unknown.bin
run_session switch      --frame 150 --frame2 320 --frame3 500 --case asset:floor.png --case2 asset:Crate.gltf --case3 asset:wall.png

# 真实示例项目：scenes/ 在第一层，可一次点击命中。
CURRENT_PROJECT="$REAL_EXAMPLE"
run_session real3d --frame 150 --frame2 340 --case asset:tps_arena.scene.ron
CURRENT_PROJECT="$PROJECT_ABS"

# 不打开项目：验证没有旧预览残留。
等编辑器退出
rm -rf "$VERIFY/noproject"; mkdir -p "$VERIFY/noproject"
echo "-------- run_session noproject --------" >> "$LOG"
"$PROBE" drag --exe "$EXE" --out "$VERIFY/noproject" --name noproject --timeout 200 \
  --frame 150 --frame2 300 --case click:menubar \
  --env BEVY_STUDIO_WORKSPACE=assets --env BEVY_STUDIO_LANGUAGE=zh \
  --env BEVY_STUDIO_CONFIG_DIR="$CFG_ABS" \
  --allow-no-change >> "$LOG" 2>&1
echo "run_session noproject 退出rc=$?" >> "$LOG"

echo "== 4/4 汇总证据 =="
PYTHONIOENCODING=utf-8 python "$VERIFY/make_acceptance_report.py" || exit 1
PYTHONIOENCODING=utf-8 python - "$VERIFY/acceptance.json" << 'PY'
import json, sys
d = json.load(open(sys.argv[1], encoding='utf-8'))
for case in d['cases']:
    print(('通过' if case['passed'] else '失败'), case['id'], '—', case['title'])
print('总计：', d['summary'])
sys.exit(0 if d['passed'] else 2)
PY
