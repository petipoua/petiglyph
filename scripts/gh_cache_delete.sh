#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/gh_cache_delete.sh [options]

Manual GitHub Actions cache cleanup helper for this repository.

Options:
  --repo <owner/name>   Target repository (default: infer from git remote origin)
  --all                 Delete all caches (optionally scoped by --ref)
  --key <cache-key>     Delete one cache by exact key
  --prefix <key-prefix> Delete all caches whose key starts with this prefix
  --ref <git-ref>       Scope delete/list to ref (refs/heads/<branch> or refs/pull/<n>/merge)
  --list                List matching caches and exit (no deletion)
  --dry-run             Print delete commands but do not execute them
  -h, --help            Show this help

Examples:
  ./scripts/gh_cache_delete.sh --prefix linux-rust-quality-
  ./scripts/gh_cache_delete.sh --all --ref refs/heads/main
  ./scripts/gh_cache_delete.sh --key windows-rust-quality-ci-<hash>
  ./scripts/gh_cache_delete.sh --prefix rust-quality --list
EOF
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

infer_repo_from_git() {
  local remote url repo
  remote="$(git remote get-url origin 2>/dev/null || true)"
  if [[ -z "$remote" ]]; then
    echo "error: cannot infer repo; pass --repo owner/name" >&2
    exit 1
  fi
  url="${remote%.git}"
  repo="${url##*:}"
  repo="${repo##*/github.com/}"
  if [[ "$repo" != */* ]]; then
    echo "error: cannot parse repo from origin remote '$remote'; pass --repo owner/name" >&2
    exit 1
  fi
  printf '%s\n' "$repo"
}

run_or_echo() {
  if [[ "$dry_run" == "1" ]]; then
    printf 'dry-run:'
    for token in "$@"; do
      printf ' %q' "$token"
    done
    printf '\n'
    return 0
  fi
  "$@"
}

require_cmd gh
require_cmd git

repo=""
ref=""
key=""
prefix=""
all="0"
list_only="0"
dry_run="0"

while (($# > 0)); do
  case "$1" in
    --repo)
      repo="${2:-}"
      shift 2
      ;;
    --ref)
      ref="${2:-}"
      shift 2
      ;;
    --key)
      key="${2:-}"
      shift 2
      ;;
    --prefix)
      prefix="${2:-}"
      shift 2
      ;;
    --all)
      all="1"
      shift
      ;;
    --list)
      list_only="1"
      shift
      ;;
    --dry-run)
      dry_run="1"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$repo" ]]; then
  repo="$(infer_repo_from_git)"
fi

if [[ "$all" == "1" && ( -n "$key" || -n "$prefix" ) ]]; then
  echo "error: --all cannot be combined with --key or --prefix" >&2
  exit 1
fi

if [[ -n "$key" && -n "$prefix" ]]; then
  echo "error: choose either --key or --prefix (not both)" >&2
  exit 1
fi

if [[ "$all" != "1" && -z "$key" && -z "$prefix" ]]; then
  echo "error: choose one of --all, --key, or --prefix" >&2
  exit 1
fi

echo "repo: $repo"
if [[ -n "$ref" ]]; then
  echo "ref:  $ref"
fi

if [[ "$all" == "1" ]]; then
  args=(cache delete --all --succeed-on-no-caches --repo "$repo")
  if [[ -n "$ref" ]]; then
    args+=(--ref "$ref")
  fi
  run_or_echo gh "${args[@]}"
  exit 0
fi

if [[ -n "$key" ]]; then
  args=(cache delete "$key" --repo "$repo")
  if [[ -n "$ref" ]]; then
    args+=(--ref "$ref")
  fi
  run_or_echo gh "${args[@]}"
  exit 0
fi

# Prefix mode: list matching cache IDs, then delete by ID.
list_args=(cache list --repo "$repo" --limit 100 --key "$prefix" --json id,key,ref)
if [[ -n "$ref" ]]; then
  list_args+=(--ref "$ref")
fi

if [[ "$list_only" == "1" ]]; then
  gh "${list_args[@]}"
  exit 0
fi

mapfile -t ids < <(gh "${list_args[@]}" --jq '.[].id')
if [[ "${#ids[@]}" -eq 0 ]]; then
  echo "No caches matched prefix '$prefix'."
  exit 0
fi

echo "Deleting ${#ids[@]} cache entr$( [[ "${#ids[@]}" -eq 1 ]] && echo "y" || echo "ies" ) with prefix '$prefix'."
for id in "${ids[@]}"; do
  run_or_echo gh cache delete "$id" --repo "$repo"
done
