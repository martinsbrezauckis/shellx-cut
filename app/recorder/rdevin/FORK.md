# ShellX Cut rdevin maintenance fork

This directory starts from the published [`rdevin` 0.1.0 crate](https://crates.io/crates/rdevin/0.1.0), released under the MIT License. Its upstream repository is <https://github.com/justdeeevin/rdevin>.

The imported crate archive has SHA-256:

```text
0e24531bad5877ada9f6464949e63433ffb9d24fb3ac6405f810c66ac18b9f82
```

ShellX Cut keeps the crate name and version, event types, key mapping, coordinate behavior, and passive `ListenOnly` / hook forwarding behavior. The maintained delta is deliberately narrow:

- add one owned, process-global passive listener API with explicit stop and join;
- make Windows unhook its two low-level hooks on the listener thread;
- make macOS stop, wake, remove, and release its own run-loop event-tap resources;
- make X11 disable, flush, free, and close its owned RECORD resources;
- retain only the event/key conversion surface and the owned passive-listener API;
- make legacy fire-and-forget listener entry points module-private so they cannot bypass the owned singleton;
- deliberately omit upstream grab, simulation, examples, and OS-injecting tests because Cut only consumes passive input.

The fork is a product build input. `LICENSE` remains the upstream MIT license; this attribution and the source are intended to travel with any public source export.
