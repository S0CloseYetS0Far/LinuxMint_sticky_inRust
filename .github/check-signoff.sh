#!/bin/bash
# Usage: check-signoff.sh BASE HEAD
# Fails if a non-merge commit in BASE..HEAD has no "Signed-off-by:" line
# with the commit author's email.
set -euo pipefail

base=$1
head=$2
missing=0
count=0

for sha in $(git rev-list --no-merges "$base..$head"); do
    count=$((count + 1))
    email=$(git log -1 --format='%ae' "$sha")
    subject=$(git log -1 --format='%s' "$sha")
    signoffs=$(git log -1 --format='%(trailers:key=Signed-off-by,valueonly)' "$sha")
    # Email addresses compare case-insensitively.
    if [[ "${signoffs,,}" == *"<${email,,}>"* ]]; then
        echo "ok       ${sha:0:12} $subject"
    else
        echo "MISSING  ${sha:0:12} $subject (author: $email)"
        missing=$((missing + 1))
    fi
done

if [ "$missing" -gt 0 ]; then
    echo
    echo "$missing of $count commit(s) have no Signed-off-by line with the author's email."
    echo "Signing off means you take responsibility for the code, including any part written by an AI."
    echo "Fix it with:  git rebase --signoff $base && git push --force-with-lease"
    echo "See https://github.com/S0CloseYetS0Far/LinuxMint_sticky_inRust#contributing"
    exit 1
fi

echo "All $count commit(s) are signed off."
