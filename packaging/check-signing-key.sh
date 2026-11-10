#!/usr/bin/env bash
# Fails when the published release key can't sign for DAYS more days: its primary key, or every
# signing subkey, has expired, is revoked or expires before then. ci.yml's dry run runs it, since a
# dry run no longer signs with the key itself (docs/developer/releasing.md says how to renew it).
#
#   packaging/check-signing-key.sh <public key (.asc)> <days>
set -euo pipefail

limit=$(($(date +%s) + $2 * 86400))
primary=0
subkey=0
lasts() { [[ $1 != [erd] ]] && { [ -z "$2" ] || [ "$2" -gt "$limit" ]; }; }
while IFS=: read -r type validity _ _ id _ expires _ _ _ _ usage _; do
    [[ $type == pub || ( $type == sub && $usage == *s* ) ]] || continue
    when=never
    [ -z "$expires" ] || when=$(date -u -d "@$expires" +%F)
    echo "$type $id: expires $when"
    if lasts "$validity" "$expires"; then
        [ "$type" = pub ] && primary=1 || subkey=1
    fi
done < <(gpg --show-keys --with-colons "$1")
if [ "$primary" = 0 ] || [ "$subkey" = 0 ]; then
    echo "::error::The release key can't sign for $2 more days: renew it (docs/developer/releasing.md)"
    exit 1
fi
