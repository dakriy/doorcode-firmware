# Releasing firmware

A release is one image file. It is built from a clean tree, uploaded to
the server, and rolled out to the doors by the server. The server signs
the image each time it serves it, with the site's signing key, and sends
the public half of that key to every door in its configuration; doors
check the signature themselves before they boot an image, so an image the
server did not sign is refused (`firmware-signing.md`). No signing key is
needed to release.

## 1. Build

Push a tag `vVERSION`, such as `v2.1.0`, and GitHub builds the release
(`.github/workflows/release.yml`). The workflow:

1. Asks `scripts/firmware-release.sh --check VERSION` whether VERSION is
   one it would release, and stops there if not.
2. Runs the same verification as a push (`verify.yml`).
3. Runs `make firmware-release VERSION=VERSION`, described below.
4. Publishes the GitHub Release named VERSION on the tag, holding
   `doorcode-firmware-VERSION.bin` and `doorcode-firmware-VERSION.sha256`.
   A VERSION with a `-` part, such as `2.1.0-rc1`, is marked a pre-release.

Nothing is published when any step fails. Download the `.bin` from the
release's page and upload it to the server as below.

To build a release locally instead:

    make firmware-release VERSION=2.1.0

or `scripts/firmware-release.sh VERSION`. The script:

1. Refuses to run while `firmware/provisioning.json` exists, since the build
   embeds that file's broker, enrollment, and Wi-Fi secrets as first-boot
   defaults and a release image goes to every door.
2. Builds the firmware in the Espressif container with
   `DOORCODE_FIRMWARE_VERSION=VERSION`. The version becomes the
   `firmware_version` the door reports and the version in the image's app
   descriptor; the script checks that the two agree. Versions are semver
   from 2.0.0 on, at most 31 characters of letters, digits, `.`, `_`, `+`,
   and `-`, and never end in `-migrate`, which marks the stepping-stone
   image (`migration.md`).
3. Turns the ELF into an app image with esptool's `elf2image`, as ESP-IDF's
   own build does, including the SHA-256 digest appended to it.
4. Checks that the image with the server's 68-byte signature block fits
   the smallest app slot of `firmware/partitions.csv` (`ota_1`, 0x1F0000
   bytes; an update may go into either slot), that is, that the image is
   at most 2031548 bytes, which is also the largest upload the server
   takes. Only then it writes the unsigned image as
   `dist/doorcode-firmware-VERSION.bin` and
   `dist/doorcode-firmware-VERSION.sha256` (`sha256sum` format). `dist/`
   is ignored by git.

The SHA-256 the script writes is of the unsigned file. The server reports
the `sha256` and `size` of the image it serves, signature block included,
which is what the `update` command carries and the door checks; so its
`sha256` differs from the one in `dist/`, and its `size` is 68 bytes
larger.

## 2. Upload

    make firmware-upload SERVER=https://doors.example.org TOKEN=... \
        FILE=dist/doorcode-firmware-2.1.0.bin VERSION=2.1.0

or `scripts/firmware-upload.sh SERVER TOKEN FILE VERSION`. It posts the
image to `POST /api/v1/firmware` as a multipart form (`version`, `image`)
with an API token of a user who holds the `manage-firmware` role, prints
the server's answer, and fails unless the server answered `201 Created`.
The version must be new to the server. `TOKEN=-` reads the token from the
first line of standard input instead, so that it stays out of the process
list:

    printf '%s\n' "$TOKEN" | make firmware-upload SERVER=https://doors.example.org \
        TOKEN=- FILE=dist/doorcode-firmware-2.1.0.bin VERSION=2.1.0

## 3. Roll out

The server sends an image to the doors that should run it: every door once
it is made the default version, or one door through its firmware override.
Rollout happens in the update window, a few doors at a time, or at once from
a door's page. For each door the server sends an `update` command naming the
version, the image's SHA-256, and a one-time download address.

The door downloads the image straight into its inactive OTA slot, erasing
the slot a sector at a time as the image arrives so the keypad stays
responsive. When the download completes it checks the SHA-256 and the
signature, has ESP-IDF check the image's structure, sets the slot to boot,
and restarts. Any failure leaves the running image in place and is
acknowledged to the server with the reason, such as `checksum mismatch` or
`bad signature`.

The new image boots on trial. Once it has connected to the broker as its
door it is marked valid and kept; the server sees the new version and
counts the update done. If it has not connected within
`update_connect_timeout_s` (default 300 s), or it resets for any reason
before then, the bootloader goes back to the previous image, which reports
the old version and acknowledges the update as failed with
`reverted: could not connect`.

## Boards on the standard partition table

Boards whose partition table has ESP-IDF's standard 1 MB app slots cannot
hold a release image. They first take a stepping-stone image, built with
`scripts/firmware-release.sh --migrate`, from the previous update
server; it moves them to this firmware's table, and the release then
reaches them as above. `migration.md` describes both steps.

## Bench boards and unsigned images

A door whose configuration sets `accept_unsigned_firmware` to `true` skips
the signature check, so it installs an image whatever its last 68 bytes
hold, and with no signing key in its configuration. Every other check still
applies. Set it only for bench and development boards; it is `false` by
default.

## Trying a release on the bench

`scripts/tests/bench-test.sh` goes through this flow on a bench board against
a development server: it releases the next two patch versions with
`make firmware-release`, uploads them with `make firmware-upload`, updates
the board to the first and back to its other slot with the second, both
signed by the server, and fails a download on purpose. "The bench run" in
`testing.md` describes its steps and settings.
