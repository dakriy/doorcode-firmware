# Documentation

Project documentation that ships with the repository. It describes this
firmware on its own terms: hardware target, architecture, protocol, and
operational decisions. Working notes that only make sense on one machine
live outside the repository.

- `security.md`: the security posture and why.
- `firmware-signing.md`: the signature block on firmware images and the key
  that checks it.
- `host.md`: the desktop host binary, its flags, and its control API.
- `testing.md`: the integration test that runs a desktop door against a
  development server, and the load tool that runs many.
