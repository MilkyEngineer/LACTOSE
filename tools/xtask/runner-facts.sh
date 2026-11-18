#!/usr/bin/env bash
# The facts of a `shaders` CI runner (tools/xtask/README.md, "GPU runs"): its system, and on
# macOS its Xcode, its `metal`, its `swift` and its Metal device (its name, its argument
# buffers tier, its highest Apple GPU family and whether it is of the Metal 3 family), written
# to the file named by the last argument and to the job's summary. Each fact is recorded
# before any is judged, and a missing one fails the script with a message that names it,
# since what the hosted images offer is not assumed. With `--metal-optional`, for a runner
# that loads cooked code and compiles none, a missing `metal` is recorded and not judged. A
# device that lacks what the engine shaders need is the Metal step's to fail, naming it.
#
#   bash tools/xtask/runner-facts.sh [--metal-optional] <file>
set -u

usage="usage: runner-facts.sh [--metal-optional] <file>"
metal_optional=false
if [ "${1:-}" = --metal-optional ]; then
    metal_optional=true
    shift
fi
out=${1:?$usage}
mkdir -p "$(dirname "$out")"
: > "$out"
missing=()

# One fact: its name, then the command whose output it is. A command that fails or prints
# nothing records the fact as missing.
fact() {
    local name=$1
    shift
    local value
    if value=$("$@" 2>&1) && [ -n "$value" ]; then
        printf '%s: %s\n' "$name" "$(printf '%s' "$value" | tr '\n' ' ' | sed 's/ *$//')" >> "$out"
    else
        printf '%s: MISSING (%s)\n' "$name" "$*" >> "$out"
        missing+=("$name")
    fi
}

fact system uname -srm
case "$(uname -s)" in
Darwin)
    fact macos sw_vers -productVersion
    fact macos-build sw_vers -buildVersion
    fact xcode xcodebuild -version
    fact developer-dir xcode-select -p
    fact sdk xcrun -sdk macosx --show-sdk-version
    fact metal xcrun -sdk macosx --find metal
    fact metal-version xcrun -sdk macosx metal --version
    fact swift swift --version
    fact metal-device swift "$(dirname "$0")/metal/pipelines.swift" --device
    ;;
Linux)
    fact distribution sh -c '. /etc/os-release && printf %s "$PRETTY_NAME"'
    ;;
esac

cat "$out"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    { echo '```'; cat "$out"; echo '```'; } >> "$GITHUB_STEP_SUMMARY"
fi
failed=false
for name in ${missing[@]+"${missing[@]}"}; do
    case $name in
    metal | metal-version)
        if $metal_optional; then
            echo "::notice::the runner has no working \`metal\` ($name), which this job does not run"
            continue
        fi
        echo "::error::the runner has no working \`metal\` ($name): the cook compiles each metallib with it, and from Xcode 26 it is the separate Metal Toolchain component (\`xcodebuild -downloadComponent MetalToolchain\`)"
        ;;
    metal-device)
        echo "::error::the runner has no Metal device the Metal script can read: see $out"
        ;;
    *)
        echo "::error::the runner's $name is missing: see $out"
        ;;
    esac
    failed=true
done
if $failed; then
    exit 1
fi
