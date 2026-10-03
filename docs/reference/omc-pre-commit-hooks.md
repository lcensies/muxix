# oh-my-claudecode: Pre-Commit & Git Hook Configurations

## Summary

The repository has **no active pre-commit or git hook enforcement**. All `.git/hooks/` entries are the default Git sample files (`.sample` suffix, never executed).

## Findings

| Mechanism | File/Location | Status |
|---|---|---|
| pre-commit framework | `.pre-commit-config.yaml` | **Not present** |
| Husky | `.husky/` directory | **Not present** |
| lint-staged | `package.json["lint-staged"]` or `.lintstagedrc*` | **Not present** |
| lefthook / simple-git-hooks | `package.json["devDependencies"]` | **Not present** |
| Active `.git/hooks/` scripts | `.git/hooks/` (no `.sample` suffix) | **None** — only default `.sample` files |

## `.git/hooks/` Contents

All 15 files present are Git's default sample hooks (read-only templates, not executed):

- `applypatch-msg.sample`
- `commit-msg.sample`
- `fsmonitor-watchman.sample`
- `post-update.sample`
- `pre-applypatch.sample`
- `pre-commit.sample`
- `pre-merge-commit.sample`
- `prepare-commit-msg.sample`
- `pre-push.sample`
- `pre-rebase.sample`
- `pre-receive.sample`
- `push-to-checkout.sample`
- `sendemail-validate.sample`
- `update.sample`

None are active (the `.sample` extension prevents Git from executing them).

## `package.json` Scripts (hook-adjacent)

The closest quality-adjacent npm lifecycle hook is:

| Script | Trigger stage | Quality tool invoked |
|---|---|---|
| `prepublishOnly` | Before `npm publish` | `npm run build && npm run compose-docs` (build validation only) |
| `version` | After `npm version` bump | `bash scripts/sync-version.sh` (version sync, not quality) |

Neither `prepare` (which husky uses) nor any `pre-commit`/`pre-push` lifecycle script is defined.

## Conclusion

oh-my-claudecode does **not** enforce code quality at commit time. There are no pre-commit hooks, no husky setup, no lint-staged configuration, and no active custom git hooks. Quality enforcement is limited to CI pipelines and manual developer invocation of `npm run lint`, `npm run format`, and `npm test`.
