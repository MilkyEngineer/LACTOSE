#!/usr/bin/env bash
# The line-ending rule (architecture section 20, D148): every tracked file against
# `.gitattributes`, through git's own account of its line endings in the index and in the
# working tree and of its attributes. `bash tools/xtask/line-endings.sh` from anywhere in the
# repository; tools/xtask/README.md says what it accepts and what each remedy means.
#
# A copy checked out with CRLF, on a host that converts once a line takes this file out of
# the rule, must still run and refuse what git lists rather than stop on a syntax error. In
# such a copy the next line leaves a carriage return in `probe`, and the one after runs this
# file again without them. Nothing above them may be a blank line or a command.
probe=x
[ "$probe" = x ] || { source_text=$(<"$0"); exec "$BASH" -c "${source_text//$'\r'/}" "$0"; } #

set -u

rule='text=auto eol=lf'

# The fields are separated by git's own `%x09`: a tab written here would carry a carriage
# return in a CRLF copy run without the line above. `:/` and `--full-name` list the whole
# tree, with paths from its root, whatever directory this runs in. The atoms need git 2.38.
if ! listing=$(git ls-files --full-name \
  --format='%(eolinfo:index)%x09%(eolinfo:worktree)%x09%(eolattr)%x09%(path)' :/); then
  echo "line-endings: git ls-files failed: run inside the repository, with git 2.38 or later" >&2
  exit 2
fi
if [ -z "$listing" ]; then
  echo "line-endings: git lists no tracked file" >&2
  exit 2
fi

# The three shapes of section 3 of the decision; everything else is refused.
accepted() {
  if [ "$attribute" = "$rule" ]; then
    case "$index|$worktree" in
      'lf|lf' | 'lf|none' | 'none|lf' | 'none|none') return 0 ;;
    esac
  elif [ "$attribute" = -text ]; then
    [ "$vendored" = yes ] && return 0
    [ "$index" = -text ] && return 0
  fi
  return 1
}

say() {
  echo "  $1"
  said=yes
}

# Every remedy that applies, the attribute's first: `git add --renormalize` stores LF only
# under the rule. The last one is what no other remedy covers.
remedies() {
  said=no
  if [ "$attribute" = -text ] && [ "$vendored" = no ] && [ "$index" != -text ]; then
    say "a \`-text\` or \`binary\` line of .gitattributes reaches this text file: narrow or remove the line"
  elif [ "$attribute" != "$rule" ] && [ "$attribute" != -text ]; then
    say "a line of .gitattributes takes this file out of the rule, or the rule's line is gone; on a host that converts, that is why its working tree is CRLF"
  fi
  case "$index" in
    crlf | mixed) say "the index holds CR: git add --renormalize $path (a plain git add keeps the CR)" ;;
  esac
  if [ "$attribute" = "$rule" ] && [ "$index" = lf ]; then
    case "$worktree" in
      crlf | mixed) say "the checkout predates the rule, or a CRLF edit was committed: with nothing uncommitted and no merge in progress, refresh it by git rm --cached -r -q :/ then git reset --hard. In CI, whose checkout is fresh, the rule does not hold on this host, and the decision returns to the planner (D148)" ;;
    esac
  fi
  if [ "$attribute" = "$rule" ] && [ "$index" = -text ]; then
    say "git reads this file as binary: remove a stray CR or NUL from a text file, or name a kind that is not text \`binary\` in .gitattributes"
  fi
  if [ -z "$index" ] || [ -z "$worktree" ]; then
    say "git lists no line endings: a symbolic link or submodule, which the rule does not admit, or a file missing from the checkout, which is restored"
  fi
  if [ "$said" = no ]; then
    say "the working file holds an uncommitted edit, which is reverted; committed instead, it is refused again by the remedy its new line matches"
  fi
}

read_count=0
refused=0
while IFS= read -r line; do
  read_count=$((read_count + 1))
  index=${line%%$'\t'*}
  line=${line#*$'\t'}
  worktree=${line%%$'\t'*}
  line=${line#*$'\t'}
  attribute=${line%%$'\t'*}
  path=${line#*$'\t'}
  # git quotes a path with unusual characters, opening quote included.
  case "$path" in
    vendor/* | '"vendor/'*) vendored=yes ;;
    *) vendored=no ;;
  esac
  accepted && continue
  refused=$((refused + 1))
  echo "$path: i/$index w/$worktree attr/$attribute"
  remedies
done <<<"$listing"

if [ "$refused" -ne 0 ]; then
  echo "line-endings: $refused of $read_count files refused"
  exit 1
fi
echo "line-endings: $read_count files read, none refused"
