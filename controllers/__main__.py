"""`python3 -m controllers <approve|queue|briefs|preset> [args...]`.

Every mode first checks `ach contract` (protocol 2) and exits 2 before any change on a mismatch.
"""
import sys

from . import approve, briefs, preset, queue
from .common import Ach, ContractError, EXIT_CONTRACT, preflight

MODES = {'approve': approve.main, 'queue': queue.main, 'briefs': briefs.main, 'preset': preset.main}
DAEMONS = {'approve', 'queue'}  # these preflight inside `serve`, before their lock and every cycle


def main(argv=None, ach=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if not argv or argv[0] not in MODES:
        print(f"usage: python3 -m controllers <{'|'.join(MODES)}> [args...]", file=sys.stderr)
        return 2
    mode, rest = argv[0], argv[1:]
    ach = ach or Ach()
    if mode not in DAEMONS and not {'-h', '--help'} & set(rest):
        try:
            preflight(ach)
        except ContractError as error:
            print(f'contract check failed: {error}', file=sys.stderr)
            return EXIT_CONTRACT
    return MODES[mode](rest, ach=ach)


if __name__ == '__main__':
    sys.exit(main())
