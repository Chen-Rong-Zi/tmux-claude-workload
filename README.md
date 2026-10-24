# tmux-claude-workload

Reproducible **workload image** for Docker Sandboxes (`sbx`, `docker/tap/sbx`): a
`docker/sandbox-templates:claude-code-docker`-based image that bakes in
`tmux`, the `chikuwa` agent tracker (compiled for linux/arm64 from source), and
the Claude Code configuration (settings, hooks, statusline, tmux.conf).

## Image

- Tag: `tmux-claude:base` (also registered in the sbx template store as `claude:full`)
- User: `agent`, entrypoint `tini --`, default cmd `claude --dangerously-skip-permissions`

## Build

```sh
docker build -t tmux-claude:base .
```

Register it for sbx (`docker build` alone does NOT put the image into the sbx
template store):

```sh
docker tag tmux-claude:base claude:full
docker save claude:full -o /tmp/t.tar
sbx template load /tmp/t.tar
rm -f /tmp/t.tar
```

## Contents

- `Dockerfile` — the recipe (tmux via apt; chikuwa compiled from source)
- `chikuwa-src/` — chikuwa source, needed by the Dockerfile build
- `home/` — Claude Code config files baked into `/home/agent/` (no API keys)

> Note: `docker build` needs network access to apt, crates.io, and github.com
> (chikuwa builds from source). Use a proxy in restricted networks.
