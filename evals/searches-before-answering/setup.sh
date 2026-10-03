#!/usr/bin/env bash
# An old solved bug, then enough newer notes that the bug is not in the
# session-start index: the agent has to look it up in recollect.
source "$(dirname "$0")/../scaffold.sh"
seed note bug,pdf,invoices "Bug: invoice PDF export failed with Prawn::Errors::UnknownFont: DejaVuSans.
Cause: the background worker loads the font registry once at boot, so fonts deployed later are unknown to it.
Fix: restart the worker (systemctl restart fera-worker) after every font deploy."
seed_recent_notes
