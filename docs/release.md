# Releasing firmware

A release is one signed image file. It is built from a clean tree, signed
with the release key, uploaded to the server, and rolled out to the doors
by the server. Doors check the signature themselves before they boot an
image, so an image signed with any other key is refused.

## 1. Build and sign

    make firmware-release VERSION=2.1.0 KEY=/path/outside/the/repo/release-key.pem

or `scripts/firmware-release.sh VERSION KEY_PATH`. The key is the P-256
private key in PEM form (`-----BEGIN EC PRIVATE KEY-----`) whose public half
is `firmware/signing_public_key.bin`. It never enters the repository, and
the script refuses a key path inside it.

The script:

1. Refuses to run while `firmware/provisioning.json` exists, since the build
   embeds that file's broker, enrollment, and Wi-Fi secrets as first-boot
   defaults and a release image goes to every door.
2. Builds the firmware in the Espressif container with
   `DOORCODE_FIRMWARE_VERSION=VERSION`. The version becomes the
   `firmware_version` the door reports and the version in the image's app
   descriptor; the script checks that the two agree. Versions are semver
   from 2.0.0 on, at most 31 characters of letters, digits, `.`, `_`, `+`,
   and `-`.
3. Turns the ELF into an app image with esptool's `elf2image`, as ESP-IDF's
   own build does, including the SHA-256 digest appended to it.
4. Appends the 68-byte signature block with
   `espsecure sign_data --version 1` (see `firmware-signing.md`).
5. Checks the signature against `firmware/signing_public_key.bin`, and
   that the signed image fits the smallest app slot of
   `firmware/partitions.csv` (`ota_1`, 0x1F0000 bytes; an update may go
   into either slot), and only then writes
   `dist/doorcode-firmware-VERSION.bin` and
   `dist/doorcode-firmware-VERSION.sha256` (`sha256sum` format, of the whole
   signed file). `dist/` is ignored by git.

To try the flow without the release key, sign with a throwaway key and
point `DOORCODE_SIGNING_PUBLIC_KEY` at its raw 64-byte public point
(`openssl ec -in key.pem -pubout -outform DER | tail -c 64`). Doors refuse
such an image unless they accept unsigned firmware.

## 2. Upload

    make firmware-upload SERVER=https://doors.example.org TOKEN=... \
        FILE=dist/doorcode-firmware-2.1.0.bin VERSION=2.1.0

or `scripts/firmware-upload.sh SERVER TOKEN FILE VERSION`. It posts the
image to `POST /api/v1/firmware` as a multipart form (`version`, `image`)
with an API token of a user who holds the `manage-firmware` role, prints
the server's answer, and fails unless the server answered `201 Created`.
The version must be new to the server.

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
the signature check, so it installs a development image that was not signed
with the release key: one released with a throwaway key as above, or any
app image without a signature block. Every other check still applies. Set
it only for bench boards; it is `false` by default.

## Trying a release on the bench

`scripts/bench-test.sh` goes through this flow on a bench board against a
development server: it releases and uploads the next patch versions with
`make firmware-release` and `make firmware-upload`, updates the board to a
signed image, has an unsigned one refused and then accepted with
`accept_unsigned_firmware`, and fails a download on purpose. "The bench
run" in `testing.md` describes its steps and settings.
