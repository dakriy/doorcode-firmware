#!/bin/sh
# Uploads a release image to a doorcode server as VERSION
# (POST /api/v1/firmware, multipart fields `version` and `image`) with an
# API token of a user who holds the manage-firmware role, and prints the
# server's answer. SERVER is the server's base address, such as
# https://doors.example.org.
#
# Usage: scripts/firmware-upload.sh SERVER TOKEN FILE VERSION
set -eu

if [ $# -ne 4 ] || [ -z "$1" ] || [ -z "$2" ] || [ -z "$3" ] || [ -z "$4" ]; then
    echo "usage: $0 SERVER TOKEN FILE VERSION" >&2
    exit 2
fi
server=${1%/}
token=$2
file=$3
version=$4

if [ ! -f "$file" ]; then
    echo "no image at $file" >&2
    exit 1
fi

answer=$(mktemp)
trap 'rm -f "$answer"' EXIT

# The token goes in a header read from stdin, so it never shows in the
# process list.
status=$(printf 'Authorization: Bearer %s\n' "$token" | curl --silent --show-error \
    --output "$answer" --write-out '%{http_code}' \
    --header @- \
    --form-string "version=$version" \
    --form "image=@$file;type=application/octet-stream" \
    "$server/api/v1/firmware")
cat "$answer"
echo
echo "HTTP $status"
[ "$status" = 201 ]
