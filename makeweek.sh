#!/bin/bash
#
# List the weekly archives composable from the HF bucket.
#
# Usage:
#   makeweek.sh    List the weeks composable from the HF bucket.
#
# A week N is composed of the latest full_*.db taken before the week starts
# (the base) plus every inc_*.db inside the week. Weeks run Wednesday 00:00Z
# to Wednesday 00:00Z (DateHours::week() = hours since 2025-01-01T00:00Z / 168).

set -euo pipefail

BUCKET="hf://buckets/Hugi-R/wplace-archives"
EPOCH_SECONDS=1735689600 # 2025-01-01T00:00:00Z, the DateHours epoch

FULL_FILES=() FULL_DH=() # full/ snapshots and their datehours
INC_FILES=() INC_DH=()   # incremental/ snapshots and their datehours
WEEK_BASE=""             # set by select_week: base full_*.db of the week
WEEK_INC_FILES=()        # set by select_week: inc_*.db of the week

usage() {
    cat >&2 <<'EOF'
Usage: makeweek.sh          list the weeks composable from the HF bucket
EOF
}

# snapshot_datehours <filename>
# Print the datehours (hours since 2025-01-01T00:00:00Z, floored) encoded in a
# snapshot filename like "full_2026-08-04T01-36-27Z.db".
snapshot_datehours() {
    local ts="${1#*_}" # strip "full_" or "inc_"
    ts="${ts%.db}"
    local d="${ts%%T*}" # 2026-08-04
    local t="${ts#*T}"  # 01-36-27Z
    t="${t%Z}"          # 01-36-27
    t="${t//-/:}"       # 01:36:27
    local epoch
    epoch=$(date -u -d "$d $t" +%s)
    echo $(( (epoch - EPOCH_SECONDS) / 3600 ))
}

# fetch_listings
# Fill FULL_FILES/FULL_DH and INC_FILES/INC_DH from the bucket listing.
fetch_listings() {
    local tmp_full tmp_inc
    tmp_full=$(mktemp)
    tmp_inc=$(mktemp)
    trap "rm -f '$tmp_full' '$tmp_inc'" EXIT

    uvx hf buckets list "$BUCKET/full" -q > "$tmp_full"
    uvx hf buckets list "$BUCKET/incremental" -q > "$tmp_inc"

    local f
    while IFS= read -r f; do
        [[ -n "$f" ]] || continue
        f="${f#full/}"
        FULL_FILES+=("$f")
        FULL_DH+=("$(snapshot_datehours "$f")")
    done < <(sort -u "$tmp_full")

    while IFS= read -r f; do
        [[ -n "$f" ]] || continue
        f="${f#incremental/}"
        INC_FILES+=("$f")
        INC_DH+=("$(snapshot_datehours "$f")")
    done < <(sort -u "$tmp_inc")

    if (( ${#FULL_FILES[@]} == 0 || ${#INC_FILES[@]} == 0 )); then
        echo "error: empty bucket listing from $BUCKET" >&2
        exit 1
    fi
}

# select_week <week>
# Set WEEK_BASE to the latest full_*.db taken before the week starts (empty if
# none) and WEEK_INC_FILES to every inc_*.db inside the week, in order.
# Week <w> covers datehours [168*w, 168*(w+1)), same as DateHours::week().
select_week() {
    local w="$1"
    local start=$(( w * 168 ))
    local end=$(( (w + 1) * 168 ))
    local i dh best_dh=-1

    WEEK_BASE=""
    for i in "${!FULL_FILES[@]}"; do
        dh=${FULL_DH[i]}
        if (( dh < start && dh > best_dh )); then
            WEEK_BASE=${FULL_FILES[i]}
            best_dh=$dh
        fi
    done

    WEEK_INC_FILES=()
    for i in "${!INC_FILES[@]}"; do
        dh=${INC_DH[i]}
        if (( dh >= start && dh < end )); then
            WEEK_INC_FILES+=("${INC_FILES[i]}")
        fi
    done
}

# list_weeks
# Print one block per week (ascending) that has at least one snapshot, with
# its base and every composing inc file.
list_weeks() {
    local -A seen=()
    local w i
    for i in "${!INC_FILES[@]}"; do
        seen[$(( INC_DH[i] / 168 ))]=1
    done

    local -a weeks=()
    mapfile -t weeks < <(printf '%s\n' "${!seen[@]}" | sort -n)

    for w in "${weeks[@]}"; do
        select_week "$w"
        local start_s=$(( EPOCH_SECONDS + w * 168 * 3600 ))
        local end_s=$(( EPOCH_SECONDS + (w + 1) * 168 * 3600 ))
        local range
        range="$(date -u -d "@${start_s}" +%Y-%m-%d) -> $(date -u -d "@${end_s}" +%Y-%m-%d)"
        local status
        if [[ -z "$WEEK_BASE" ]]; then
            status="[no base]"
        elif (( ${#WEEK_INC_FILES[@]} == 7 )); then
            status="[complete]"
        else
            status="[incomplete: ${#WEEK_INC_FILES[@]}/7 incs]"
        fi
        printf 'week %-3d %s   %s\n' "$w" "$range" "$status"
        if [[ -n "$WEEK_BASE" ]]; then
            printf '  base: %s\n' "$WEEK_BASE"
        else
            printf '  base: (none)\n'
        fi
        if (( ${#WEEK_INC_FILES[@]} > 0 )); then
            local inc
            for inc in "${WEEK_INC_FILES[@]}"; do
                if [[ "$inc" == "${WEEK_INC_FILES[0]}" ]]; then
                    printf '  incs: %s\n' "$inc"
                else
                    printf '        %s\n' "$inc"
                fi
            done
        else
            printf '  incs: (none)\n'
        fi
    done
}

main() {
    if (( $# == 0 )); then
        fetch_listings
        list_weeks
    else
        usage
        exit 2
    fi
}

main "$@"
