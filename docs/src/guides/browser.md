# Debug JavaScript in Chrome from the terminal

js-debug starts Chrome or Chromium and debugs the page. Works for plain pages and frameworks like
Angular, through source maps.

```sh
tuitor install js-debug
tuitor "ng serve"           # a pwa-chrome / chrome configuration from launch.json
```

```jsonc
{
  "name": "ng serve",
  "type": "chrome",
  "request": "launch",
  "url": "http://localhost:4200",
  "webRoot": "${workspaceFolder}"
}
```

- Start your dev server yourself (`ng serve`); `preLaunchTask` is not run.
- Without `runtimeExecutable`, js-debug looks for Google Chrome only. Configurations tuitor creates
  use the first of `google-chrome`, `chromium`, `chromium-browser`, `microsoft-edge` on `PATH`.
- Headless works over SSH: the examples take `HEADLESS=1`.
- `console.log` output shows in OUTPUT.

![The Angular example stopped in app.ts; STACK folds Angular's frames](../assets/screenshots/tabs.png)

## Try it

```sh
cd examples/web && ./run.sh          # a page from disk, no server needed
cd examples/angular && ./run.sh      # starts ng serve, then tuitor
```
