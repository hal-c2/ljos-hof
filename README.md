# Ljós Hof

*Ljós* is Icelandic for light; a *hof* is a shrine. Hof is a terminal for
Linux. Its UI is QML, built on [Ljós UI](https://github.com/hal-c2/ljos-ui),
and it uses Ghostty's `libghostty-vt` (through
[libghostty-rs](https://github.com/uzaaft/libghostty-rs)) as a library to
parse what programs write.

It aims to be fast, configured in one small file, and easy to extend from
the shell: your own commands at a key, hooks on terminal events, plugins,
`hof ctl` for scripts and an MCP server for agents.

## What it does

- **Tabs and splits**, each of which can be zoomed and resized. Search, and
  jumping between prompts when the shell has shell integration.
- **A dropdown.** `hof --dropdown` slides in from the top or bottom edge,
  and `hof --toggle` shows or hides it (bind that in your desktop).
  `hof --tab -- cmd` opens a tab in it.
- **One TOML file** where every key is optional. A key Hof doesn't know is
  a warning, not an error. `reload-config` applies the file without a
  restart. See [`hof.example.toml`](crates/hof_config/hof.example.toml).
- **Commands.** `[commands.<name>]` runs a program in a tab, a split or the
  background, or types text into the focused pane, at a key you choose.
- **Hooks.** Shell snippets run on pane-opened, pane-closed,
  command-started, command-finished, bell, notification, directory-changed
  and title-changed.
- **Plugins.** A plugin is `plugins/<name>.toml` or
  `plugins/<name>/plugin.toml` beside the config, holding commands and
  hooks. Your own config wins any conflict.
- **`hof ctl`** drives the running Hof: list panes, read a screen or its
  scrollback, send text, run a command and wait for its exit code, open
  tabs and splits, and focus or close panes. Inside a pane it needs no
  arguments, because Hof sets `HOF_SOCKET` and `HOF_PANE_ID` there.
- **MCP.** `hof mcp` is a stdio MCP server for the running Hof. Agents can
  read panes always; typing into panes and opening or closing them are
  separate opt-ins in `[mcp]`.
- **Optional session restore.** `[session] restore = true` reopens the
  windows, tabs, splits and directories, and optionally the scrollback.
- **Themes.** Nine are built in, and you can add your own as
  `themes/<name>.toml`.

## Building

You need [mise](https://mise.jdx.dev). It installs Rust and Zig;
libghostty-vt's build script needs Zig to build Ghostty's parser.

```sh
mise install
mise run setup        # links your Ljós UI checkout ($LJOS_UI_DIR) into third_party/
cargo build --release
./target/release/hof
```

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```

The QML is compiled ahead of time against generated types. After changing a
Rust type that QML can see, regenerate them:

```sh
LJOS_TYPES=write cargo test -p hof --test types
```

## The stress test

`mise run fire` builds a fire animation: 200×60 cells of 24-bit colour,
about 370 kB a frame. It measures the fire in three ways:

- through the parser and the grid with no window;
- painted in an offscreen window;
- drawn in the terminal you run it from.

Pass a number to set how many seconds the last part lasts.

On the machine Hof is developed on, in a release build:

| what                                  | frames/s |
|---------------------------------------|---------:|
| parsing                               |  ~950    |
| parsing and updating the grid         |  ~670    |
| parsing, the grid and painting        |  ~480    |

`libghostty-vt-sys` builds Ghostty's Zig in Debug mode whenever cargo builds
with any debug info, and that makes the parser dozens of times slower. The
workspace's `Cargo.toml` turns debug info off for that one package, in both
profiles. Keep it off.

## The crates

| crate          | what it is                                                      |
|----------------|-----------------------------------------------------------------|
| `hof`          | the application: the QML window, the CLI, `hof ctl`, IPC        |
| `hof_qml`      | the `Terminal` item: paints a session and takes its input       |
| `hof_vt`       | a session: a program on a pty, parsed on its own thread         |
| `hof_config`   | the config file, its commands, hooks and plugins                |
| `hof_mcp`      | the MCP server and the client `hof ctl` and `hof mcp` use       |
| `hof_session`  | the saved session, without the UI                               |
| `hof_remote`   | the wire to `hof agent` on other machines, and resumable streams |
| `hof_agent`    | `hof agent`: remote panes' programs, kept running between connections |

## Where it is going

Remote panes on any machine, mixed in tabs and splits. They survive
dropped connections the way mosh sessions do, and bring X11 apps, sound,
the clipboard and files across. Panes also know the agents running in
them. See [`docs/remote.md`](docs/remote.md).

Remote panes work now. Name a machine under `[hosts]`:

```toml
[hosts.dev]
ssh = "dev.example.com"
```

Then `hof ctl tab --host dev` opens a tab there, and a split of that tab
stays on `dev`. The machine needs `hof` on its `PATH`; Hof doesn't copy
itself over yet. When the connection drops, the pane says it is
reconnecting and picks up where it was. When Hof quits with
`session.restore` on, the remote programs keep running and are attached
again at the next start.

## Not yet

- Links: OSC 8 hyperlinks, and detecting URLs in plain text.
- OSC 52 clipboard access, behind a permission prompt.

## Licence

MIT. See [LICENSE](LICENSE).
