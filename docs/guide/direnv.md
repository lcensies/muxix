---
description: Automatically set up direnv in new worktrees
---

# direnv integration

If your project uses [direnv](https://direnv.net/) for environment management, you can configure muxix to automatically set it up in new worktrees:

```yaml
# .muxix.yaml
post_create:
  - direnv allow

files:
  symlink:
    - .envrc
```

See also [Using direnv for port isolation](/guide/monorepos#using-direnv) in monorepos.
