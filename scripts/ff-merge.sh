#!/usr/bin/env bash
# 收尾：校验线性历史后用 ff-only 合并，并可选自动推送 base。
# 用法：
#   ./ff-merge.sh <repo-dir> [branch]        # 合并到 base，不 push
#   ./ff-merge.sh --push <repo-dir> [branch] # 合并后自动 git push origin <base>
set -euo pipefail

PUSH=0
while [ $# -gt 0 ]; do
  case "$1" in
    --push|-p) PUSH=1; shift ;;
    --) shift; break ;;
    -*) echo "未知选项: $1" >&2; exit 64 ;;
    *) break ;;
  esac
done

REPO="${1:?repo dir}"
cd "$REPO"

BRANCH="${2:-$(git config --get agent.branch || true)}"
BASE="$(git config --get agent.base || true)"
if [ -z "$BASE" ] || [ -z "$BRANCH" ]; then
  echo "FAIL: 找不到合并目标。先跑 preflight.sh，或手动 git config --local agent.base <分支>" >&2
  exit 2
fi
if [ "$BRANCH" = "$BASE" ]; then
  echo "FAIL: 作业分支与合并目标相同（$BRANCH），没有可合并的东西。检查是否已在主分支上。" >&2
  exit 16
fi

if [ -n "$(git status --porcelain)" ]; then
  echo "FAIL: 当前工作区不干净，还有未提交改动。" >&2
  git status --short >&2
  exit 3
fi

BASE_SHA="$(git rev-parse "$BASE")"
BRANCH_SHA="$(git rev-parse "$BRANCH")"
if ! git merge-base --is-ancestor "$BASE_SHA" "$BRANCH_SHA"; then
  echo "FAIL: $BASE 不是 $BRANCH 的祖先 —— 会话期间主分支被改动过，无法 ff-only。" >&2
  echo "     建议：先在本地 git pull origin $BASE 对齐，或在作业分支上 git revert 对应提交后重试。" >&2
  exit 5
fi

echo "== 待合并提交 =="
git log --oneline "$BASE..$BRANCH"
echo

git switch "$BASE"
git merge --ff-only "$BRANCH"

echo "== 本次会话变更（合并前 base..合并后 HEAD）=="
git diff --stat "$BASE_SHA..HEAD"
echo

echo "== 合并后校验 =="
if [ "$(git rev-parse --abbrev-ref HEAD)" != "$BASE" ]; then
  echo "WARN: 合并后不在 $BASE 分支上，请人工确认。" >&2
fi
if [ -n "$(git status --porcelain)" ]; then
  echo "WARN: 合并后工作区仍有改动，请人工确认。" >&2
  git status --short >&2
fi

if [ "$PUSH" = "1" ]; then
  echo "== 推送 base 到远端 =="
  if ! git push origin "$BASE"; then
    echo "FAIL: git push origin $BASE 失败（鉴权/网络/远端拒绝）。" >&2
    echo "      本地合并已完成，请手动排查后再 push；不得 force push。" >&2
    exit 6
  fi
  echo "OK   已推送 origin/$BASE"
fi

echo
echo "OK 已快进合并到 $BASE。分支 $BRANCH 保留（便于 git revert 定位）。"
if [ "$PUSH" = "0" ]; then
  echo "本次会话未 push。如需推送：git push origin $BASE"
fi
echo "若结果不满意：由人执行 git reset --hard origin/$BASE（本地会回到远端那份）"
