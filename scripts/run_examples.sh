#!/usr/bin/env bash
set -uo pipefail

domain="all"
fail_fast=0
verify=1
expected_file="examples/expected_warnings.tsv"
targets=()

usage() {
  cat <<'USAGE'
Usage:
  scripts/run_examples.sh [--domain all|internval|nullptr|sign] [--fail-fast] [--no-verify] [path ...]

Runs MIRSA over example .rs files in batch.
By default, compares emitted warning codes against examples/expected_warnings.tsv.

If no path is given, runs:
  examples/safe4u_final_numeric
  examples/safe4u_final_pointer

Paths may be files or directories. Files under an original/ directory and
*.orig.rs files are skipped.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --domain)
      if [[ $# -lt 2 ]]; then
        echo "error: --domain requires a value" >&2
        exit 2
      fi
      domain="$2"
      shift 2
      ;;
    --domain=*)
      domain="${1#--domain=}"
      shift
      ;;
    --fail-fast)
      fail_fast=1
      shift
      ;;
    --no-verify)
      verify=0
      shift
      ;;
    --expected)
      if [[ $# -lt 2 ]]; then
        echo "error: --expected requires a file path" >&2
        exit 2
      fi
      expected_file="$2"
      shift 2
      ;;
    --expected=*)
      expected_file="${1#--expected=}"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      while [[ $# -gt 0 ]]; do
        targets+=("$1")
        shift
      done
      ;;
    -*)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      targets+=("$1")
      shift
      ;;
  esac
done

case "$domain" in
  all|internval|nullptr|sign) ;;
  *)
    echo "error: unsupported domain '$domain'" >&2
    exit 2
    ;;
esac

if [[ ${#targets[@]} -eq 0 ]]; then
  targets=(
    "examples/safe4u_final_numeric"
    "examples/safe4u_final_pointer"
  )
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root" || exit 1

echo "Building driver..."
if ! cargo build -q -p driver; then
  echo "error: failed to build driver" >&2
  exit 1
fi

driver="$repo_root/target/debug/driver"
sysroot="$(rustc --print sysroot)"
export LD_LIBRARY_PATH="$sysroot/lib:${LD_LIBRARY_PATH:-}"

files=()
for target in "${targets[@]}"; do
  if [[ -f "$target" ]]; then
    case "$target" in
      */original/*|*.orig.rs) ;;
      *.rs) files+=("$target") ;;
      *) echo "skip non-rust file: $target" >&2 ;;
    esac
  elif [[ -d "$target" ]]; then
    while IFS= read -r file; do
      files+=("$file")
    done < <(find "$target" -type f -name '*.rs' ! -name '*.orig.rs' ! -path '*/original/*' | sort)
  else
    echo "warning: path not found: $target" >&2
  fi
done

if [[ ${#files[@]} -eq 0 ]]; then
  echo "error: no example .rs files found" >&2
  exit 1
fi

passed=0
failed=0
failures=()
declare -A expected

if [[ "$verify" -eq 1 ]]; then
  if [[ ! -f "$expected_file" ]]; then
    echo "error: expected warnings file not found: $expected_file" >&2
    exit 1
  fi
  while IFS=$'\t' read -r path codes extra; do
    [[ -z "${path:-}" || "${path:0:1}" == "#" ]] && continue
    if [[ -n "${extra:-}" ]]; then
      echo "error: malformed expected warnings row for $path" >&2
      exit 1
    fi
    expected["$path"]="$codes"
  done < "$expected_file"
fi

echo "Running ${#files[@]} example(s) with --domain $domain"
for file in "${files[@]}"; do
  printf '[%03d/%03d] %s ... ' "$((passed + failed + 1))" "${#files[@]}" "$file"
  if ! "$driver" --domain "$domain" "$file" >/tmp/mirsa-example.out 2>/tmp/mirsa-example.err; then
    echo "FAILED"
    failed=$((failed + 1))
    failures+=("$file")
    sed 's/^/    /' /tmp/mirsa-example.err >&2
    if [[ "$fail_fast" -eq 1 ]]; then
      break
    fi
    continue
  fi

  if [[ "$verify" -eq 1 ]]; then
    if [[ ! -v "expected[$file]" ]]; then
      echo "MISSING EXPECTED"
      failed=$((failed + 1))
      failures+=("$file")
      echo "    no row for $file in $expected_file" >&2
      if [[ "$fail_fast" -eq 1 ]]; then
        break
      fi
      continue
    fi

    actual="$(
      sed -n 's/.*warning\[\([^]]*\)\].*/\1/p' /tmp/mirsa-example.out /tmp/mirsa-example.err \
        | paste -sd, -
    )"
    if [[ -z "$actual" ]]; then
      actual="-"
    fi
    want="${expected[$file]}"
    if [[ "$actual" == "$want" ]]; then
      echo "ok"
      passed=$((passed + 1))
    else
      echo "MISMATCH"
      failed=$((failed + 1))
      failures+=("$file")
      echo "    expected: $want" >&2
      echo "    actual:   $actual" >&2
      if [[ "$fail_fast" -eq 1 ]]; then
        break
      fi
    fi
  else
    echo "ok"
    passed=$((passed + 1))
  fi
done

rm -f /tmp/mirsa-example.out /tmp/mirsa-example.err

echo
echo "Summary: $passed passed, $failed failed"
if [[ "$failed" -ne 0 ]]; then
  echo "Failed examples:"
  for file in "${failures[@]}"; do
    echo "  $file"
  done
  exit 1
fi
