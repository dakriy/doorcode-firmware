# Documentation

Project documentation that ships with the repository. It describes this
firmware on its own terms: hardware target, architecture, protocol, and
operational decisions. Working notes that only make sense on one machine
live outside the repository.

- `architecture.md`: the event loop, the connectors and what each target
  implements, how tests enter, the desktop binaries, and the firmware
  workspace.
- `protocol.md`: how the firmware follows the server's controller
  contract: the configuration fields it honors, their defaults and
  effects, and the telemetry it sends.
- `security.md`: the security posture and why.
- `firmware-signing.md`: the signature block on firmware images and the key
  that checks it.
- `release.md`: building, signing, uploading, and rolling out a firmware
  release.
- `host.md`: the desktop host binary, its flags, and its control API.
- `mesh.md`: how doors far from an access point hop through other doors,
  and the telemetry that shows it.
- `testing.md`: the integration test that runs a desktop door against a
  development server, and the load tool that runs many.

The firmware workspace's own README, `../firmware/README.md`, covers
building and flashing in the container, provisioning, the partition table,
and what each connector does on the board. `../AGENTS.md` is the contract
for changing the code: its rules, gates, and commands.
