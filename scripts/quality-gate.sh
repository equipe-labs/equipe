#!/bin/sh
# The quality gate's behaviour eval (docs/quality-gate.md): real members on
# small fixed tasks, run with equipe at a baseline ref (A) and with this
# checkout (B), graded, and compared. It spends tokens on your own Claude Code
# login — see the cost note in the doc before a full run.
#
#   scripts/quality-gate.sh --baseline origin/main             # all tasks, 3 runs each, A and B
#   scripts/quality-gate.sh --baseline v0.103.50 --tasks backend-fix-failing-test,pm-vague-ask --runs 1
#   scripts/quality-gate.sh --tasks backend-fix-failing-test --runs 1   # B alone: a smoke run, no verdict
set -eu

usage() {
	cat <<'EOF'
usage: scripts/quality-gate.sh [--baseline <git ref>] [--tasks <a,b,…>] [--runs <n>] [--jobs <n>] [--out <dir>]
  --baseline  the ref A is built from; left out, only B (this checkout) runs
  --tasks     task names, comma-separated (default: all ten)
  --runs      runs of each task per variant (default 3)
  --jobs      runs at once (default 1)
  --out       the report's folder (default target/quality-gate/<time>)
EOF
}

baseline="" tasks="" runs=3 jobs=1 out=""
while [ $# -gt 0 ]; do
	case "$1" in
	--baseline) baseline=$2; shift 2 ;;
	--tasks) tasks=$2; shift 2 ;;
	--runs) runs=$2; shift 2 ;;
	--jobs) jobs=$2; shift 2 ;;
	--out) out=$2; shift 2 ;;
	-h | --help) usage; exit 0 ;;
	*) usage >&2; exit 2 ;;
	esac
done

root=$(git rev-parse --show-toplevel)
cd "$root"
out=${out:-$root/target/quality-gate/$(date +%Y%m%d-%H%M%S)}
mkdir -p "$out"
df -h / | tail -1 | awk '{ print "free disk: " $4 }'

if [ -n "$baseline" ]; then
	sha=$(git rev-parse --verify "$baseline^{commit}")
	tree=$(mktemp -d "${TMPDIR:-/tmp}/equipe-baseline.XXXXXX")
	git worktree add --detach -q "$tree" "$sha"
	trap 'git -C "$root" worktree remove --force "$tree" >/dev/null 2>&1 || true' EXIT
	echo "building A at $sha"
	# This checkout's build folder: the dependencies are built once for both.
	(cd "$tree" && CARGO_TARGET_DIR="$root/target" cargo build -q -p equipe --bin equipe)
	cp "$root/target/debug/equipe" "$out/equipe-A"
	EQUIPE_EVAL_A="$out/equipe-A"
	export EQUIPE_EVAL_A
	# The binary A's build put in the shared folder: B's is built again over it.
	touch crates/cli/src/main.rs
fi
echo "building B (this checkout)"
cargo build -q -p equipe --bin equipe
cp target/debug/equipe "$out/equipe-B"

EQUIPE_EVAL_B="$out/equipe-B" EQUIPE_EVAL_TASKS=$tasks EQUIPE_EVAL_RUNS=$runs EQUIPE_EVAL_JOBS=$jobs EQUIPE_EVAL_OUT=$out \
	cargo test -q -p equipe --test quality_gate -- --ignored --nocapture behaviour_eval
