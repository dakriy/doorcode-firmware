# Mesh

A door out of good reach of the building's Wi-Fi can reach the network
through another door. The doors form a Wi-Fi mesh with Espressif's
Mesh-Lite: each door runs a station and a soft-AP, and a door whose access
point is too weak joins a neighbouring door's soft-AP instead. Every door
keeps an IP address and a broker connection of its own, so the server and
the application see a meshed door exactly as any other.

## Configuration

The mesh is configured per door, in the configuration message:

| Field | Default | Meaning |
| --- | --- | --- |
| `mesh_enabled` | `false` | Whether the door may join or offer the mesh. |
| `mesh_ssid` | empty | The name of every door's soft-AP; the mesh is off while it is empty. |
| `mesh_password` | empty | The soft-AP's WPA2 password, 8 to 63 characters; a shorter one leaves the soft-AP open. |
| `mesh_min_router_rssi` | -75 | Below this signal, in dBm, the door joins another door rather than the access point. |
| `mesh_max_layer` | 4 | The most hops from the access point a door may sit at. |

Every door of a site should be given the same mesh settings. The door reads
them when it starts and chooses its network then, so a configuration that
changes any of them restarts the door. Under the mesh a change of
`wifi_11b_disabled` restarts the door too, and the switch covers only the
door's station; its soft-AP keeps the 802.11b rates (`protocol.md`).

## Behavior

With the mesh off, the door is a Wi-Fi station only, as the firmware's
README describes: it joins the strongest access point of `wifi_ssid`, falls
back to the network that last associated, and roams to an access point at
least 8 dB stronger while its signal stays below `wifi_roam_rssi`.

With the mesh on:

- **Root.** When an access point of `wifi_ssid` is at least
  `mesh_min_router_rssi`, the door joins the strongest one, as without the
  mesh, and is a root: its soft-AP, named `mesh_ssid`, is on and offers the
  mesh to other doors. Sites with many access points have many roots.
- **Hopping.** When no access point is that strong, the door joins the
  soft-AP of a door already in the mesh instead, as long as that puts it at
  most `mesh_max_layer` hops from the access point. Its own soft-AP then
  offers the mesh one hop further, unless it is at the deepest layer.
- **Addresses.** Each soft-AP hands out addresses to the doors joined to it
  and forwards their traffic onward with network address translation, so a
  door several hops away still opens its own TCP connection to the broker.
- **Fallback.** When the door has had no address for 30 seconds, it
  switches between the credentials it was told and those that last joined
  the access point, as the plain station does.
- **Roaming.** A root whose signal has been below `wifi_roam_rssi` for
  five readings in a row, a second apart, scans at most once a minute; when
  another access point of the same SSID is at least 8 dB stronger than the
  average of those readings, it leaves and joins again, picking the
  strongest access point, or a neighbouring door if none is strong enough.
  The access point it has joined is never a candidate, so with one access
  point it stays, and readings taken before the link last came up or of an
  access point joined earlier do not count. Doors hopping through it find
  their way back on their own.

Mesh-Lite recovers on its own when a parent door goes away: the doors below
it look for another parent or, if their access point is strong enough
again, join it.

## Hidden network

Every door's soft-AP is hidden, always; no setting shows it. Its beacons
leave the name out, so phones and laptops in the building do not list the
mesh among the networks they offer to join. A door looking for a parent
already knows `mesh_ssid`: Mesh-Lite finds the doors of its mesh by their
beacons, joins by name with probes that ask for it, and remembers the
parent it joined.

Hiding the name keeps the mesh out of everyday network lists, and nothing
more. It is not access control:

- The beacons still go out, carrying the soft-AP's MAC address and
  Mesh-Lite's own information, so a Wi-Fi scanner sees that a network is
  there and which door offers it.
- A door's probes for a parent carry `mesh_ssid` in the clear, and so does
  the soft-AP's answer, so anyone listening nearby learns the name.
- What keeps others off the mesh is WPA2 with `mesh_password`, so give
  every site a long random one.

## Telemetry

Every telemetry report carries where the door sits in the mesh, also when
the mesh is off:

| Field | Type | Meaning |
| --- | --- | --- |
| `mesh_layer` | uint | Hops to the access point: 0 when the door joined it directly. |
| `mesh_parent` | text | The hardware id of the door this one hops through, 12 lowercase hex digits; empty when it joined the access point directly. |
| `mesh_children` | uint | Doors joined to this door's soft-AP. |

`signal` is the signal of whatever the door joined: the access point, or
its parent door's soft-AP. The parent's hardware id is its station MAC,
which is the address of its soft-AP less one, since an ESP32 gives its
soft-AP the MAC after its station's.

The desktop host reports whatever `POST /radio` set (`docs/host.md`).
