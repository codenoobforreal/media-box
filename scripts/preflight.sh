#!/usr/bin/env bash
# 开工前检查 + 建作业分支。用法： ./preflight.sh <repo-dir> [branch]
# 关键前提：远端 origin/<base> 必须与本地 HEAD 一致（远端是唯一恢复点）。
set -euo pipefail

REPO="${1:-.}"
BRANCH="${2:-agent/run-$(date +%Y%m%d-%H%M)}"

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

git fetch origin --quiet || { echo "FAIL: git fetch origin 失败（网络或凭证），无法确认远端为恢复点。" >&2; exit 8; }
if ! git rev-parse --verify --quiet "origin/$BASE" >/dev/null; then
  echo "FAIL: 远端没有 origin/$BASE。先 git push -u origin $BASE。" >&2; exit 9
fi

COUNTS="$(git rev-list --left-right --count "origin/$BASE...HEAD")"
# rev-list --left-right --count A...B 输出 "A独有 B独有"
# A = origin/<base>，B = HEAD → 第 1 列是落后数，第 2 列是领先数
BEHIND="$(printf '%s' "$COUNTS" | awk '{print $1}')"
AHEAD="$(printf '%s' "$COUNTS" | awk '{print $2}')"
if [ "$AHEAD" != "0" ] || [ "$BEHIND" != "0" ]; then
  echo "FAIL: 本地与 origin/$BASE 不同步（本地领先 $AHEAD，落后 $BEHIND）。" >&2
  echo "      远端此刻不等于你的工作状态，回滚会丢东西。先 push 或 pull 对齐。" >&2
  exit 10
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
