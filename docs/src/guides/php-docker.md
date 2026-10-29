# Debug PHP in Docker

Xdebug in the container connects out to tuitor on the host. Add this to the service:

```yaml
services:
  php:
    extra_hosts: ["host.docker.internal:host-gateway"]   # needed on Linux
    environment:
      XDEBUG_MODE: ${XDEBUG_MODE:-off}                    # XDEBUG_MODE=debug docker compose up -d
      XDEBUG_CONFIG: client_host=host.docker.internal client_port=9003
```

Start tuitor with the path mapping (path in the container = path here):

```sh
tuitor listen --map /var/www/app=./api
```

Then send a request with the Xdebug trigger, or set `XDEBUG_TRIGGER=1` for a CLI script.

## Keep the mapping

| command | does |
|---|---|
| `/config map /var/www/app=./api` | saves the mapping in a tuitor configuration |
| `/config mappings-from-compose` (`M` in CONFIG) | takes the mappings from the service's bind mounts, via `docker compose config` or `compose.yaml` |

## Try it

```sh
cd examples/php
tuitor "Listen for Xdebug in Docker"
docker compose run --rm php          # in another terminal
```

Xdebug traces work in the container too: `mkdir -p traces`, then
`XDEBUG_MODE=debug,trace docker compose run --rm php`.
