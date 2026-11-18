#!/usr/bin/env bash
# The facts of a `shaders` CI runner (tools/xtask/README.md, "GPU runs"): its system, and on
# macOS its Xcode, its `metal` and its `swift`, written to the file named by the one argument
# and to the job's summary. Each fact is recorded before any is judged, and a missing one
# fails the script with a message that names it, since what the hosted images offer is not
# assumed. Whether a runner has a Metal device is the Metal step's to record.
#
#   bash tools/xtask/runner-facts.sh <file>
set -u

out=${1:?usage: runner-facts.sh <file>}
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
    ;;
Linux)
    fact distribution sh -c '. /etc/os-release && printf %s "$PRETTY_NAME"'
    ;;
esac

cat "$out"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    { echo '```'; cat "$out"; echo '```'; } >> "$GITHUB_STEP_SUMMARY"
fi
if [ ${#missing[@]} -gt 0 ]; then
    for name in "${missing[@]}"; do
        case $name in
        metal | metal-version)
            echo "::error::the runner has no working \`metal\` ($name): the cook compiles each metallib with it, and from Xcode 26 it is the separate Metal Toolchain component (\`xcodebuild -downloadComponent MetalToolchain\`)"
            ;;
        *)
            echo "::error::the runner's $name is missing: see $out"
            ;;
        esac
    done
    exit 1
fi
