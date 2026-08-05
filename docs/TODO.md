## Quick wins

- The `mcp` gem is locked at 0.4.0 while upstream is at 0.25.x, so the gemspec
  pins `~> 0.4.0` to match what the tests cover. Run the suite against current
  `mcp` and widen the constraint.

## Bugs

## Refactoring

- Is there a way to check for the vector extension *in sqlite*? I think the
checking of a predefined list of install locations might be brittle. Maybe just
remove that check, db manager seems to handle lack of the extension fine anyway.

## Improvements / new Features"

- I would like to integrate time as a factor that influences ranking of search
results by preferring more recent memories over past ones. this 'aging-factor'
should be configurable so it can be fine-tuned depending on how it goes.

- [maybe not really important?] Web UI for promoting Memories from project to
global and vice versa (imlies storing the source project along with global
memories which might be good context anyway)
