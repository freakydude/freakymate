#!/usr/bin/env bash
# Runs all checks of a work package in one go: formatting, Clippy, tests, app build, UI previews and the
# skill check. Every step runs even when an earlier one fails; the exit code is 1 if any step failed.
# WSL2/Linux only. Usage (repo root): scripts/verify.sh [--snapshot]
#   --snapshot  also renders the UI snapshot scenes against a fake Moonraker (slower, needs a free port)
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1
log_dir=$(mktemp -d "${TMPDIR:-/tmp}/freakymate-verify.XXXXXX")
with_snapshot=false
[ "${1:-}" = "--snapshot" ] && with_snapshot=true
failed=()

run_step() {
    local name=$1
    shift
    local log="$log_dir/${name// /_}.log"
    local start=$SECONDS
    if "$@" > "$log" 2>&1; then
        echo "OK    $name ($((SECONDS - start)) s)"
    else
        echo "FAIL  $name after $((SECONDS - start)) s (last lines in $log)"
        tail -n 15 "$log" | sed 's/^/        /'
        failed+=("$name")
    fi
}

run_step "cargo fmt --check" cargo fmt --all -- --check
run_step "cargo clippy" cargo clippy --workspace --all-targets -- -D warnings
run_step "cargo test" cargo test --workspace
# The whole workspace, not only -p freakymate: the same features as the test run, so the UI crate is not compiled again.
run_step "cargo build" cargo build --workspace --bins
run_step "ui previews" scripts/ui-preview.sh
# The AI skills are local (.claude/ is not in the repository); check them only where they exist.
skill_check=.claude/skills/session-handover/scripts/check-skills.sh
if [ -x "$skill_check" ]; then
    run_step "skill check" "$skill_check"
fi
if [ "$with_snapshot" = true ]; then
    run_step "ui snapshot" scripts/ui-snapshot.sh
fi

echo
if [ "${#failed[@]}" -eq 0 ]; then
    echo "All checks passed. Logs: $log_dir"
    exit 0
fi
echo "Failed: ${failed[*]}. Logs: $log_dir"
exit 1
