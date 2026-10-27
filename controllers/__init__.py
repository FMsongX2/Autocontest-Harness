"""Managed contest controllers run as `python3 -m controllers <approve|queue|briefs|preset>`.

The controllers only drive the `ach` CLI (argv lists, never a shell). Their state stays under
`<root>/data/` (`data/auto-approve`, `data/queue-master`); this package holds no operator data.
"""

PROTOCOL = 2
VERSION = '0.8.0'
