#!/bin/sh
# The quality gate's behaviour eval (docs/quality-gate.md): real members on
# small fixed tasks, run with equipe at a baseline (A) and with this checkout
# (B), graded, and compared. It spends tokens on your own Claude Code login —
# see the cost note in the doc before a full run.
#
#   scripts/quality-gate.sh --baseline origin/main             # all tasks, 3 runs each, A and B
#   scripts/quality-gate.sh --baseline v0.103.50 --tasks backend-fix-failing-test,pm-vague-ask --runs 1
#   scripts/quality-gate.sh --baseline-binary ~/Downloads/equipe --tasks designer-landing-page
#   scripts/quality-gate.sh --tasks backend-fix-failing-test --runs 1   # B alone: a smoke run, no verdict
set -eu

usage() {
	cat <<'EOF'
usage: scripts/quality-gate.sh [--baseline <git ref> | --baseline-binary <path>] [--tasks <a,b,…>] [--runs <n>] [--jobs <n>] [--out <dir>]
  --baseline         the ref A is built from; left out, only B (this checkout) runs
  --baseline-binary  an equipe already built (a release's, say) to run as A, instead of building one
  --tasks            task names, comma-separated (default: all)
  --runs             runs of each task per variant (default 3)
  --jobs             runs at once (default 1)
  --out              the report's folder (default <target>/quality-gate/<time>)
Builds go to $CARGO_TARGET_DIR when it is set, else this checkout's target/.
EOF
}

baseline="" baseline_binary="" tasks="" runs=3 jobs=1 out=""
while [ $# -gt 0 ]; do
	case "$1" in
	--baseline) baseline=$2; shift 2 ;;
	--baseline-binary) baseline_binary=$2; shift 2 ;;
	--tasks) tasks=$2; shift 2 ;;
	--runs) runs=$2; shift 2 ;;
	--jobs) jobs=$2; shift 2 ;;
	--out) out=$2; shift 2 ;;
	-h | --help) usage; exit 0 ;;
	*) usage >&2; exit 2 ;;
	esac
done
if [ -n "$baseline" ] && [ -n "$baseline_binary" ]; then
	echo "--baseline or --baseline-binary, not both" >&2
	exit 2
fi

root=$(git rev-parse --show-toplevel)
cd "$root"
target=${CARGO_TARGET_DIR:-$root/target}
out=${out:-$target/quality-gate/$(date +%Y%m%d-%H%M%S)}
mkdir -p "$out"
df -h / | tail -1 | awk '{ print "free disk: " $4 }'

if [ -n "$baseline_binary" ]; then
	cp "$baseline_binary" "$out/equipe-A"
	EQUIPE_EVAL_A="$out/equipe-A"
	export EQUIPE_EVAL_A
elif [ -n "$baseline" ]; then
	sha=$(git rev-parse --verify "$baseline^{commit}")
	tree=$(mktemp -d "${TMPDIR:-/tmp}/equipe-baseline.XXXXXX")
	git worktree add --detach -q "$tree" "$sha"
	trap 'git -C "$root" worktree remove --force "$tree" >/dev/null 2>&1 || true' EXIT
	echo "building A at $sha"
	# The same build folder: the dependencies are built once for both.
	(cd "$tree" && CARGO_TARGET_DIR="$target" cargo build -q -p equipe --bin equipe)
	cp "$target/debug/equipe" "$out/equipe-A"
	EQUIPE_EVAL_A="$out/equipe-A"
	export EQUIPE_EVAL_A
	# Cargo names a workspace crate's build by its path inside the workspace,
	# so A's crates and this checkout's share names in the build folder, and
	# it tells them apart only by time: A's, built just now, look newer than
	# this checkout's files. Make this checkout's newer, or B is built with A's crates.
	find crates -type f -exec touch {} +
fi
echo "building B (this checkout)"
CARGO_TARGET_DIR="$target" cargo build -q -p equipe --bin equipe
cp "$target/debug/equipe" "$out/equipe-B"

# The eval runs for hours: build it, then run it on its own, so the build
# folder isn't held for others meanwhile.
CARGO_TARGET_DIR="$target" cargo test -q -p equipe --test quality_gate --no-run
exe=$(CARGO_TARGET_DIR="$target" cargo test -q -p equipe --test quality_gate --no-run --message-format=json 2>/dev/null | grep '"name":"quality_gate"' | sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' | tail -1)
cp "$exe" "$out/quality_gate"

EQUIPE_EVAL_B="$out/equipe-B" EQUIPE_EVAL_TASKS=$tasks EQUIPE_EVAL_RUNS=$runs EQUIPE_EVAL_JOBS=$jobs EQUIPE_EVAL_OUT=$out \
	"$out/quality_gate" --ignored --nocapture behaviour_eval
