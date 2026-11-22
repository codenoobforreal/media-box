#!/usr/bin/env bash
# 开工前检查 + 建作业分支。
# 用法：
#   ./preflight.sh <repo-dir> [branch]        # 检查通过后建作业分支（RW 态）
#   ./preflight.sh --check-only <repo-dir>   # 只做只读检查，不建分支、不写配置
#   ./preflight.sh --force <repo-dir> [branch] # 强制覆盖已有 agent.base/agent.branch
# 关键前提：远端 origin/<base> 必须与本地 HEAD 一致（远端是唯一恢复点）。
set -euo pipefail

CHECK_ONLY=0
FORCE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --check-only|-c) CHECK_ONLY=1; shift ;;
    --force|-f) FORCE=1; shift ;;
    --) shift; break ;;
    -*) echo "未知选项: $1" >&2; exit 64 ;;
    *) break ;;
  esac
done

REPO="${1:-.}"
BRANCH="${2:-agent/run-$(date +%Y%m%d-%H%M%S)}"

cd "$REPO"

BASE="$(git rev-parse --abbrev-ref HEAD)"
if [ "$BASE" = "HEAD" ]; then
  echo "FAIL: 处于 detached HEAD，先切到一个分支。" >&2; exit 2
fi

if [ -n "$(git status --porcelain)" ]; then
  echo "FAIL: 工作区不干净，先 commit 或 stash。" >&2
  git status --short >&2
  exit 3
fi

if ! git remote get-url origin >/dev/null 2>&1; then
  echo "FAIL: 没有 origin 远端。远端是本方案的唯一恢复点，先配好并 push。" >&2; exit 7
fi

if ! git fetch origin --quiet 2>/dev/null; then
  echo "FAIL: git fetch origin 失败。请检查：1) 网络是否通畅；2) 凭证是否过期（git credential）；3) origin 是否可达。" >&2; exit 8
fi

if ! git rev-parse --verify --quiet "origin/$BASE" >/dev/null; then
  echo "FAIL: 远端没有 origin/$BASE。先 git push -u origin $BASE。" >&2; exit 9
fi

COUNTS="$(git rev-list --left-right --count "origin/$BASE...HEAD")"
BEHIND="$(printf '%s' "$COUNTS" | awk '{print $1}')"
AHEAD="$(printf '%s' "$COUNTS" | awk '{print $2}')"
if [ "$AHEAD" != "0" ] || [ "$BEHIND" != "0" ]; then
  echo "FAIL: 本地与 origin/$BASE 不同步（本地领先 $AHEAD，落后 $BEHIND）。" >&2
  echo "      远端此刻不等于你的工作状态，回滚会丢东西。先 push 或 pull 对齐。" >&2
  exit 10
fi

# 检查是否已有 agent 配置
EXISTING_BASE="$(git config --get agent.base 2>/dev/null || true)"
EXISTING_BRANCH="$(git config --get agent.branch 2>/dev/null || true)"
if [ -n "$EXISTING_BASE" ] || [ -n "$EXISTING_BRANCH" ]; then
  if [ "$FORCE" = "0" ]; then
    echo "FAIL: 已存在 agent 配置（base=$EXISTING_BASE branch=$EXISTING_BRANCH）。" >&2
    echo "      如需重新初始化，加 --force 覆盖；或先手动 git config --unset agent.base / agent.branch。" >&2
    exit 11
  fi
  echo "WARN: 覆盖已有 agent 配置（base=$EXISTING_BASE branch=$EXISTING_BRANCH）。" >&2
fi

# 所有检查通过
if [ "$CHECK_ONLY" = "1" ]; then
  echo "OK   check-only: base=$BASE  remote-sync=0/0（未建分支、未改配置）"
  exit 0
fi

if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
  echo "FAIL: 分支已存在：$BRANCH（换名字或先删除）" >&2; exit 4
fi

git config --local agent.base "$BASE"
git config --local agent.branch "$BRANCH"
git switch -c "$BRANCH"

echo
echo "OK   base=$BASE  branch=$BRANCH  remote-sync=0/0"
echo
echo "会话期间守三条："
echo "  1) 每块 cargo check 通过后自行提交，一块一个 commit"
echo "  2) 只用 git add <明确路径>，禁止 add -A；只读远端，不 push"
echo "  3) 收尾用 ff-merge.sh；回滚用 git revert，不用 reset --hard"
