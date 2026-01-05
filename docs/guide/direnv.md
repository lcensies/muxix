# direnv integration

If your project uses [direnv](https://direnv.net/) for environment management, you can configure workmux to automatically set it up in new worktrees:

```yaml
# .workmux.yaml
post_create:
  - direnv allow

files:
  symlink:
    - .envrc
```
