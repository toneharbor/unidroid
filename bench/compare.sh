#!/usr/bin/env bash
# Runs unidbg and minidbg on the same samples, checks the outputs match,
# and prints peak memory (max RSS) and wall time for each.
#
# Usage: bench/compare.sh [instances...]   (default: 1 4)
set -euo pipefail
cd "$(dirname "$0")/.."

export JAVA_HOME="${JAVA_HOME:-/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home}"
BASE=bench/unidbg-baseline
if (($#)); then COUNTS=("$@"); else COUNTS=(1 4); fi

if [[ ! -f $BASE/cp.txt || ! -d $BASE/target/classes ]]; then
    (cd $BASE && mvn -q -B compile dependency:build-classpath -Dmdep.outputFile=cp.txt)
fi
cargo build -q --release --example bench

UNIDBG=("$JAVA_HOME/bin/java" --add-opens java.base/java.util=ALL-UNNAMED -cp "$BASE/target/classes:$(cat $BASE/cp.txt)" baseline.Baseline)
MINIDBG=(target/release/examples/bench)

# Runs a command with its stdout saved to $1, prints "<seconds> <MB>".
measure() {
    local out=$1 err
    shift
    err=$(mktemp)
    if ! /usr/bin/time -l "$@" > "$out" 2> "$err"; then
        cat "$err" >&2
        : > "$out"
    fi
    awk '/ real /{r=$1} /maximum resident/{m=$1} END{printf "%.2f %.1f", r, m/1048576}' "$err"
    rm -f "$err"
}

uo=$(mktemp)
mo=$(mktemp)
trap 'rm -f "$uo" "$mo"' EXIT
printf "%-11s %4s | %16s | %16s | %s\n" sample n "unidbg" "minidbg" "same output"
for sample in jna tmessages jna32 ttencrypt tmessages32 signutil qdreader kanxue; do
    case "$sample" in
        jna|tmessages) SAMPLES=tests/fixtures/arm64-v8a ;;
        *) SAMPLES=tests/fixtures/armeabi-v7a ;;
    esac
    for n in "${COUNTS[@]}"; do
        read -r ut um <<< "$(measure "$uo" "${UNIDBG[@]}" "$sample" "$SAMPLES" "$n")"
        read -r mt mm <<< "$(measure "$mo" "${MINIDBG[@]}" "$sample" "$SAMPLES" "$n")"
        same=no
        if [[ -s $uo && -s $mo ]] && diff <(grep -v '^pbkdf2_ms=' "$uo") <(grep -v '^pbkdf2_ms=' "$mo") > /dev/null; then same=yes; fi
        printf "%-11s %4d | %6.1f MB %6.2fs | %6.1f MB %6.2fs | %s\n" "$sample" "$n" "$um" "$ut" "$mm" "$mt" "$same"
    done
done
