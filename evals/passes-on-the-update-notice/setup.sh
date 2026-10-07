#!/usr/bin/env bash
# A check remembered a moment ago that found a much newer release, so the
# session-start hook prints the update notice without looking anything up.
source "$(dirname "$0")/../scaffold.sh"
seed_recent_notes
printf '{"checked_at":"%s","latest":"99.0.0"}\n' "$(date -u +%Y-%m-%dT%H:%M:%S.000Z)" \
  > "$HOME/.recollect/update-check.json"
