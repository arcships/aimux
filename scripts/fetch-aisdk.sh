#!/usr/bin/env bash
# Fetch the pinned AI SDK package sources into target/aisdk/<pkg>@<ver>/.
#
# The pinned packages come only from fixtures/aisdk/VERSIONS.json (override
# the file with AISDK_VERSIONS=<path>, the destination with AISDK_DEST=<dir>;
# relative paths are taken from the repository root).
# Every entry must be the object form
#
#     "<package>": {"version": "<x.y.z>", "integrity": "sha512-<base64>"}
#
# and the whole file is checked before anything is downloaded. Each tarball's
# sha512 must equal the pinned integrity, and its package.json must name the
# pinned package and version.
#
# Two phases: every package is fetched, verified and extracted into a staging
# directory first; only when all of them pass are the destinations replaced.
# Any failure (network, missing version, integrity or version mismatch)
# leaves the destination exactly as it was. Running it again replaces each
# destination with identical content.
#
# Usage: bash scripts/fetch-aisdk.sh

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

die() {
    printf 'fetch-aisdk: %s\n' "$*" >&2
    exit 1
}

for tool in jq npm tar openssl; do
    command -v "$tool" >/dev/null 2>&1 || die "required tool not found: $tool"
done

versions_file="${AISDK_VERSIONS:-fixtures/aisdk/VERSIONS.json}"
dest_root="${AISDK_DEST:-target/aisdk}"

[[ -f "$versions_file" ]] || die "versions file not found: $versions_file"
jq -e 'type == "object" and length > 0' "$versions_file" >/dev/null 2>&1 ||
    die "$versions_file must be a non-empty JSON object"

# Every entry, before any download: the object form, a plain npm package name
# (it becomes a path under $dest_root), a plain version and a sha512 SRI.
bad="$(jq -r '
    to_entries[]
    | select(
        (.key | test("^(@[a-z0-9][a-z0-9._-]*/)?[a-z0-9][a-z0-9._-]*$") | not)
        or (.value | type) != "object"
        or (.value.version | type) != "string"
        or (.value.version | test("^[0-9]+\\.[0-9]+\\.[0-9]+(-[0-9A-Za-z.-]+)?$") | not)
        or (.value.integrity | type) != "string"
        or (.value.integrity | test("^sha512-[A-Za-z0-9+/]{86}==$") | not))
    | .key' "$versions_file")"
[[ -z "$bad" ]] ||
    die "$versions_file: not {\"version\", \"integrity\": \"sha512-...\"}: ${bad//$'\n'/ }"

mkdir -p "$dest_root"
work="$(mktemp -d "$dest_root/.work.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# pkg<TAB>version<TAB>integrity
entries="$(jq -r 'to_entries[] | [.key, .value.version, .value.integrity] | @tsv' "$versions_file")"

# Phase 1: fetch, verify and stage every package; nothing is placed yet.
specs=()
while IFS=$'\t' read -r pkg ver integrity; do
    spec="$pkg@$ver"
    echo "fetch-aisdk: $spec"

    pack_dir="$work/pack"
    rm -rf "$pack_dir" && mkdir -p "$pack_dir"
    tarball_name="$(npm pack "$spec" --silent --ignore-scripts --pack-destination "$pack_dir" 2>/dev/null | tail -n1)" ||
        die "npm pack failed for $spec (network error or no such version)"
    tarball="$pack_dir/$tarball_name"
    [[ -n "$tarball_name" && -f "$tarball" ]] || die "npm pack produced no tarball for $spec"

    actual="sha512-$(openssl dgst -sha512 -binary "$tarball" | openssl base64 -A)"
    [[ "$actual" == "$integrity" ]] ||
        die "$spec: integrity mismatch (expected $integrity, got $actual)"

    unpack="$work/unpack"
    rm -rf "$unpack" && mkdir -p "$unpack"
    tar -xzf "$tarball" -C "$unpack" --no-same-owner --no-same-permissions
    [[ -f "$unpack/package/package.json" ]] || die "$spec: tarball has no package/package.json"

    got="$(jq -r '.name + "@" + .version' "$unpack/package/package.json")"
    [[ "$got" == "$spec" ]] || die "version mismatch: wanted $spec, tarball is $got"

    staging="$work/stage/$spec"
    mkdir -p "$(dirname "$staging")"
    mv "$unpack/package" "$staging"
    specs+=("$spec")
done <<<"$entries"

# Phase 2: every package verified. Move the old trees aside, place the new
# ones, and put the old ones back if a move fails. (The ${a[@]+...} form keeps
# an empty array safe under `set -u` in bash 3.2.)
placed=()
restore() {
    local spec
    for spec in ${placed[@]+"${placed[@]}"}; do
        rm -rf "${dest_root:?}/$spec"
    done
    for spec in "${specs[@]}"; do
        if [[ -e "$work/old/$spec" ]]; then
            mkdir -p "$(dirname "$dest_root/$spec")"
            mv "$work/old/$spec" "$dest_root/$spec"
        fi
    done
}
for spec in "${specs[@]}"; do
    if [[ -e "$dest_root/$spec" ]]; then
        mkdir -p "$(dirname "$work/old/$spec")"
        mv "$dest_root/$spec" "$work/old/$spec" || { restore; die "could not move $dest_root/$spec aside"; }
    fi
done
for spec in "${specs[@]}"; do
    mkdir -p "$(dirname "$dest_root/$spec")"
    mv "$work/stage/$spec" "$dest_root/$spec" || { restore; die "could not place $spec"; }
    placed+=("$spec")
done

echo "fetch-aisdk: done ($(jq length "$versions_file") packages in $dest_root)"
