# Sourced by each case's setup.sh. Makes the run's workspace belong to
# project fera, puts the staged embedding model where recollect looks for it
# in the run's home, and offers `seed` to store memories before the run.
set -euo pipefail

# The harness gives each run a fresh home; an existing database means this
# script runs outside it, where seeding would write into real memories.
if [ -e "$HOME/.recollect/memories.db" ]; then
  echo "refusing to seed: $HOME/.recollect already holds a database" >&2
  exit 1
fi

stage="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$HOME/.recollect"
cp -r "$stage/.eval-model" "$HOME/.recollect/models"
echo "fera" > .recollect-project

# seed <type> <tags> <content>: stores one memory in project fera.
seed() {
  recollect store -p fera -t "$1" -T "$2" "$3" > /dev/null
}

# Ten notes and todos, so the session-start index shows only these.
seed_recent_notes() {
  seed note decision,invoices "Decision: invoices are immutable once sent; corrections go out as credit notes."
  seed note learning,staging "The staging database is reset every Sunday night, so test data there does not survive the week."
  seed todo customers "Add a VAT ID check to the customer form."
  seed note decision,api "The public API stays on JSON:API; no GraphQL endpoint."
  seed note learning,ci "CI caches gems per Gemfile.lock hash; a stale cache shows up as missing native extensions."
  seed todo reports "Monthly revenue report should exclude test customers."
  seed note decision,auth "Sessions expire after 8 hours of inactivity; the refresh token covers longer work."
  seed note learning,deploy "Deploys run migrations before restarting the web servers."
  seed todo invoices "Show the payment due date on the invoice list."
  seed note decision,email "Transactional email goes through Postmark, newsletters through a separate account."
}
