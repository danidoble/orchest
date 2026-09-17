#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

usage() {
  echo 'Usage: ./publish.sh [--prepare-only | --resume | --retry]' >&2
  exit 2
}

mode="publish"
if [[ $# -gt 1 ]]; then usage; fi
if [[ $# -eq 1 ]]; then
  case "$1" in
    --prepare-only) mode="prepare" ;;
    --resume) mode="resume" ;;
    --retry) mode="retry" ;;
    *) usage ;;
  esac
fi

confirm() {
  local answer
  read -r -p "$1 [y/N] " answer
  [[ "$answer" == [yY] || "$answer" == [yY][eE][sS] || "$answer" == [sS] || "$answer" == [sS][iI] ]]
}

for tool in git cargo python3; do
  command -v "$tool" >/dev/null || { echo "Missing command: $tool" >&2; exit 1; }
done
if [[ "$mode" != "resume" && -n "$(git status --porcelain --untracked-files=no)" ]]; then
  echo 'Tracked files have changes. Commit or save them before publishing.' >&2
  git status --short --untracked-files=no
  exit 1
fi

run_checks() {
  cargo metadata --offline --locked --no-deps --format-version 1 >/dev/null
  echo 'Running local release checks...'
  cargo fmt --all --check
  cargo test --workspace --locked
  cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
}

branch=$(git branch --show-current)
[[ -n "$branch" ]] || { echo 'Detached HEAD is not supported.' >&2; exit 1; }
current=$(python3 - <<'PY'
import pathlib, tomllib
print(tomllib.loads(pathlib.Path('crates/orchest-cli/Cargo.toml').read_text())['package']['version'])
PY
)

if [[ "$mode" == "publish" || "$mode" == "prepare" ]]; then
  [[ "$current" =~ ^([0-9]+\.[0-9]+\.[0-9]+)-alpha\.([1-9][0-9]*)$ ]] || {
    echo "Current version is not an alpha version: $current" >&2
    exit 1
  }
  suggested="${BASH_REMATCH[1]}-alpha.$((BASH_REMATCH[2] + 1))"
  read -r -p "New version [$suggested]: " version
  version="${version:-$suggested}"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+-alpha\.[1-9][0-9]*$ ]] || {
    echo 'Version must look like 0.1.0-alpha.3' >&2
    exit 1
  }
  read -r -p 'Release summary: ' summary
  [[ -n "$summary" ]] || { echo 'Release summary is required.' >&2; exit 1; }
  echo "Prepare Orchest v$version from $current on branch $branch"
  confirm 'Update version files and release notes?' || exit 0

  OLD_VERSION="$current" NEW_VERSION="$version" RELEASE_SUMMARY="$summary" python3 - <<'PY'
import os, pathlib, re, tomllib

old = os.environ['OLD_VERSION']
new = os.environ['NEW_VERSION']
summary = os.environ['RELEASE_SUMMARY']
def key(value):
    match = re.fullmatch(r'(\d+)\.(\d+)\.(\d+)-alpha\.(\d+)', value)
    if not match:
        raise SystemExit(f'invalid alpha version: {value}')
    return tuple(map(int, match.groups()))
if key(new) <= key(old):
    raise SystemExit(f'new version must be greater than {old}')

members = tomllib.loads(pathlib.Path('Cargo.toml').read_text())['workspace']['members']
names = set()
for member in members:
    path = pathlib.Path(member, 'Cargo.toml')
    content = path.read_text()
    package = tomllib.loads(content)['package']
    if package['version'] != old:
        raise SystemExit(f'{path}: expected {old}, found {package["version"]}')
    names.add(package['name'])
    lines = content.splitlines(keepends=True)
    in_package = False
    changed = 0
    for index, line in enumerate(lines):
        if line.strip() == '[package]':
            in_package = True
        elif line.startswith('['):
            in_package = False
        if in_package and line.startswith('version = '):
            lines[index] = f'version = "{new}"\n'
            changed += 1
    if changed != 1:
        raise SystemExit(f'{path}: expected one package version')
    path.write_text(''.join(lines))

lock = pathlib.Path('Cargo.lock')
blocks = re.split(r'(?=^\[\[package\]\]$)', lock.read_text(), flags=re.MULTILINE)
seen = set()
for index, block in enumerate(blocks):
    match = re.search(r'^name = "([^"]+)"$', block, re.MULTILINE)
    if match and match.group(1) in names:
        name = match.group(1)
        blocks[index], count = re.subn(
            r'^version = "' + re.escape(old) + r'"$',
            f'version = "{new}"', block, count=1, flags=re.MULTILINE)
        if count != 1:
            raise SystemExit(f'Cargo.lock: expected {old} for {name}')
        seen.add(name)
if seen != names:
    raise SystemExit(f'Cargo.lock: missing workspace packages: {sorted(names - seen)}')
lock.write_text(''.join(blocks))

notes = pathlib.Path('RELEASE_NOTES.md')
notes.write_text(f'# Orchest v{new}\n\n{summary}\n')
print(f'Updated {len(names)} Cargo packages, Cargo.lock and RELEASE_NOTES.md')
PY

  git diff --stat
  git status --short
  echo 'If checks fail, fix the issue and run ./publish.sh --resume.'
  run_checks
  if [[ "$mode" == "prepare" ]]; then
    echo 'Prepared locally. Review and commit the changed files before publishing.'
    exit 0
  fi
  confirm "Commit v$version and its release notes?" || exit 0
  git add Cargo.lock crates/*/Cargo.toml RELEASE_NOTES.md
  git commit -m "chore: prepare v$version prerelease"
elif [[ "$mode" == "resume" ]]; then
  version="$current"
  [[ -f RELEASE_NOTES.md ]] || { echo 'RELEASE_NOTES.md is missing.' >&2; exit 1; }
  grep -Fqx "# Orchest v$version" RELEASE_NOTES.md || {
    echo "RELEASE_NOTES.md does not describe v$version" >&2
    exit 1
  }
  run_checks
  confirm "Commit v$version and its release notes?" || exit 0
  git add Cargo.lock crates/*/Cargo.toml RELEASE_NOTES.md
  git commit -m "chore: prepare v$version prerelease"
  [[ -z "$(git status --porcelain --untracked-files=no)" ]] || {
    echo 'Other tracked changes remain. Commit or save them, then run ./publish.sh --retry.' >&2
    exit 1
  }
else
  version="$current"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+-alpha\.[1-9][0-9]*$ ]] || {
    echo "Current version is not an alpha version: $version" >&2
    exit 1
  }
  [[ -f RELEASE_NOTES.md ]] || { echo 'RELEASE_NOTES.md is missing.' >&2; exit 1; }
  grep -Fqx "# Orchest v$version" RELEASE_NOTES.md || {
    echo "RELEASE_NOTES.md does not describe v$version" >&2
    exit 1
  }
fi

remote=$(git remote get-url origin)
if [[ "$remote" =~ github\.com[:/]([^/]+)/([^/]+)$ ]]; then
  repo="${BASH_REMATCH[1]}/${BASH_REMATCH[2]%.git}"
else
  echo "Cannot identify a GitHub origin from $remote" >&2
  exit 1
fi
command -v gh >/dev/null || { echo 'Missing command: gh' >&2; exit 1; }

echo "Ready to push $branch to $repo. This makes the release source available online."
confirm 'Push the branch?' || exit 0
git push origin "$branch"

echo 'The release workflow builds Linux and Windows on GitHub Actions and consumes runner minutes.'
confirm "Start the manual workflow for v$version?" || {
  echo "To resume later, run ./publish.sh --retry"
  exit 0
}

if gh release view "v$version" --repo "$repo" >/dev/null 2>&1; then
  echo "Release v$version already exists." >&2
  exit 1
fi
previous=$(gh run list --repo "$repo" --workflow release-orchest.yml --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId // 0')
gh workflow run release-orchest.yml --repo "$repo" --ref "$branch" -f "version=$version"
run_id=''
for _ in {1..20}; do
  candidate=$(gh run list --repo "$repo" --workflow release-orchest.yml --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId // 0')
  if [[ "$candidate" != "$previous" && "$candidate" != 0 ]]; then
    run_id="$candidate"
    break
  fi
  sleep 3
done
[[ -n "$run_id" ]] || { echo 'Workflow started; find its run in GitHub Actions.' >&2; exit 1; }
echo "Watching GitHub Actions run $run_id..."
gh run watch "$run_id" --repo "$repo" --exit-status
gh release view "v$version" --repo "$repo" --json url,isPrerelease,assets \
  --jq '"Release: \(.url) | prerelease: \(.isPrerelease) | assets: \([.assets[].name] | join(", "))"'
