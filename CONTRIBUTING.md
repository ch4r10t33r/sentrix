# Contributing to Borgkit

## Commit message convention

Borgkit uses **[Conventional Commits](https://www.conventionalcommits.org/)**.
Every commit message must follow this format:

```
<type>(<optional scope>): <short description>

[optional body]

[optional footer(s)]
```

### Types and their effect on versioning

| Type | Description | Version bump |
|---|---|---|
| `feat` | New feature | **minor** (0.x.0) |
| `fix` | Bug fix | **patch** (0.0.x) |
| `perf` | Performance improvement | **patch** |
| `refactor` | Code change (no feature/fix) | **patch** |
| `docs` | Documentation only | none |
| `test` | Adding/updating tests | none |
| `chore` | Tooling, deps, CI | none |
| `feat!` / `fix!` / any `BREAKING CHANGE:` footer | Breaking change | **major** (x.0.0) |

### Examples

```bash
# Patch release
git commit -m "fix(discovery): handle empty DHT query response"

# Minor release
git commit -m "feat(zig): add Libp2pDiscovery sidecar bridge"

# Major release (breaking change)
git commit -m "feat!: rename AgentRequest.from to AgentRequest.callerId

BREAKING CHANGE: callers must update AgentRequest construction to use callerId"

# No release
git commit -m "docs: update libp2p architecture diagram"
git commit -m "chore(deps): bump typescript to 5.4"
```

## Release process

Releases are **fully automated** — no manual tagging or npm publish needed.

1. Merge a PR into `main`/`master`
2. GitHub Actions runs the `Release` workflow
3. `semantic-release` analyses commits since the last release
4. If any `feat`, `fix`, or `perf` commits exist it:
   - Bumps the version in `package.json`
   - Generates / prepends to `CHANGELOG.md`
   - Creates a GitHub Release with release notes
   - Publishes to [npmjs.com](https://www.npmjs.com/package/borgkit-cli)
   - Commits `package.json` + `CHANGELOG.md` back to `main`

## Setup (first time)

```bash
npm ci          # install deps including semantic-release devDeps
npm run build   # compile TypeScript → dist/
```

## Required secrets (repo admins only)

| Secret | Where to set | Description |
|---|---|---|
| `NPM_TOKEN` | GitHub → Settings → Secrets → Actions | npm automation token with `publish` scope |

The `GITHUB_TOKEN` is provided automatically by GitHub Actions.
