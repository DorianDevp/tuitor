FROM php:8.4-cli

# Xdebug through PIE, as https://xdebug.org/docs/install describes; PIE needs unzip.
RUN --mount=type=bind,from=ghcr.io/php/pie:bin,source=/pie,target=/usr/local/bin/pie \
    set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends unzip; \
    pie install --no-cache xdebug/xdebug; \
    apt-get purge -y --auto-remove unzip; \
    rm -rf /var/lib/apt/lists/*
