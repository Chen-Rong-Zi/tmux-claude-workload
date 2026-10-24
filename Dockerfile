# workload:recipe — reproducible workload image for Docker Sandboxes (sbx)
# Based on the sbx claude base template; bakes in:
#   - tmux                (apt)
#   - chikuwa TUI         (compiled from source for linux/arm64)
#   - claude config files (kit home/ -> /home/agent/)
FROM docker.io/docker/sandbox-templates:claude-code-docker

USER root

# tmux + Rust toolchain (apt; no rustup needed)
RUN apt-get update \
    && apt-get install -y --no-install-recommends tmux rustc cargo \
    && rm -rf /var/lib/apt/lists/*

# Build chikuwa from source (requires crates.io + github.com during build)
COPY chikuwa-src/ /tmp/chikuwa-src/
RUN cd /tmp/chikuwa-src \
    && cargo build --release \
    && install -Dm755 target/release/chikuwa /home/agent/.local/bin/chikuwa \
    && chown agent:agent /home/agent/.local/bin/chikuwa \
    && rm -rf /tmp/chikuwa-src /root/.cargo/registry /root/.cargo/git

# Bake in claude config (settings.local.json, CLAUDE.md, hooks, statusline, tmux.conf)
COPY home/ /home/agent/
RUN chown -R agent:agent /home/agent/.claude /home/agent/.config /home/agent/.local

USER agent
