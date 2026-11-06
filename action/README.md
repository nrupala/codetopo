# codetopo PR impact (GitHub Action)

Posts one comment on each pull request showing its **structural change impact**:
- the symbols and call/import edges the PR adds, removes or changes;
- each touched symbol's **blast radius** before and after, worst change first.

It updates the same comment on every push, and attaches the full diff JSON as a workflow artifact.

**Free on public repositories.** On private repositories it needs a codetopo Team key (US$19 per repo per month).

```yaml
# .github/workflows/codetopo.yml
name: codetopo
on: pull_request
permissions:
  contents: read
  pull-requests: write
jobs:
  impact:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: nrupala/codetopo/action@main
        with:
          max-rows: 10
          fail-above: 0          # Guardrails: set e.g. 200 to fail PRs that push any blast radius above 200
          # codetopo-key: ${{ secrets.CODETOPO_KEY }}   # private repos only
```

Notes:
- **Blast radius** is the count of symbols reachable from a symbol over calls and imports edges, the same measure as `codetopo descendants`. These are measurements only; nothing is scored or judged.
- Runs on `pull_request`, never `pull_request_target`. On PRs from forks, GitHub gives the token read-only access, so the comment step cannot post there. The diff is still uploaded as an artifact.
- The first run installs the CLI with `cargo install` (a few minutes); later runs are cached.
