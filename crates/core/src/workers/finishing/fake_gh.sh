#!/bin/sh
# A stand-in for `gh` over a local bare repository, for tests.
# usage: fake_gh.sh STATE_DIR BARE_REPOSITORY <the arguments of gh>
set -eu
state=$1
bare=$2
shift 2
echo "$*" >>"$state/calls"

verb="$1 $2"
shift 2
number= head= base= match= body=
case $verb in
"pr view" | "pr checks" | "pr merge" | "pr edit")
	number=$1
	shift
	;;
esac
while [ $# -gt 0 ]; do
	case $1 in
	--head) head=$2 ;;
	--base) base=$2 ;;
	--match-head-commit) match=$2 ;;
	--body) body=$2 ;;
	esac
	case $1 in
	--merge | --squash | --rebase | --log-failed) shift ;;
	--*) shift 2 ;;
	*) shift ;;
	esac
done

of() { git -C "$bare" rev-parse --verify -q "refs/heads/$1" || true; }

case $verb in
"pr list")
	if [ -f "$state/branch" ] && [ ! -f "$state/merged" ] &&
		[ "$(cat "$state/branch")" = "$head" ]; then
		echo '[{"number":1,"url":"https://forge.example/pull/1"}]'
	else
		echo '[]'
	fi
	;;
"pr create")
	echo "$head" >"$state/branch"
	echo "$base" >"$state/base"
	echo "$body" >"$state/body"
	;;
"pr edit")
	echo "$body" >"$state/body"
	;;
"pr view")
	if [ -f "$state/merged" ]; then
		printf '{"state":"MERGED","headRefOid":"%s","baseRefName":"%s","mergeCommit":{"oid":"%s"}}\n' \
			"$(cat "$state/merged")" "$(cat "$state/base")" "$(cat "$state/merge-commit")"
	else
		printf '{"state":"OPEN","headRefOid":"%s","baseRefName":"%s","mergeCommit":null}\n' \
			"$(of "$(cat "$state/branch")")" "$(cat "$state/base")"
	fi
	;;
"pr checks")
	bucket=$(cat "$state/checks" 2>/dev/null || echo pass)
	printf '[{"name":"ci","bucket":"%s","link":"https://forge.example/actions/runs/7/job/1"}]\n' "$bucket"
	[ "$bucket" = pass ] || exit 1
	;;
"run view")
	echo "the log of the failed check"
	;;
"pr merge")
	branch=$(cat "$state/branch")
	at=$(of "$branch")
	[ "$at" = "$match" ] || {
		echo "head commit does not match" >&2
		exit 1
	}
	base=$(cat "$state/base")
	merge=$(GIT_AUTHOR_NAME=forge GIT_AUTHOR_EMAIL=forge@example.com \
		GIT_COMMITTER_NAME=forge GIT_COMMITTER_EMAIL=forge@example.com \
		git -C "$bare" commit-tree "$at^{tree}" -p "$(of "$base")" -p "$at" -m "Merge pull request #$number")
	git -C "$bare" update-ref "refs/heads/$base" "$merge"
	echo "$merge" >"$state/merge-commit"
	echo "$at" >"$state/merged"
	;;
*)
	echo "unknown: $verb" >&2
	exit 2
	;;
esac
