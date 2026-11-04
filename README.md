# Ljós Hof

*Ljós* is Icelandic for light; a *hof* is a shrine. Hof is a terminal for
Linux and macOS. Its UI is QML, built on [Ljós UI](https://github.com/hal-c2/ljos-ui),
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
- **Keys as each desktop has them.** On a Mac, Command: Cmd+T, Cmd+W,
  Cmd+D, Cmd+1 to 9, Cmd+C and Cmd+V, and every Ctrl and Option chord is
  the terminal's. Elsewhere Ctrl+Shift: Ctrl+Shift+T, Ctrl+Shift+W, Alt+1
  to 9, Ctrl+Shift+C, so plain Ctrl chords (fzf's Ctrl+T, say) reach the
  shell. Any of it can be rebound in `[keys]`.
- **One TOML file** where every key is optional. A key Hof doesn't know is
  a warning, not an error. `reload-config` applies the file without a
  restart. See [`hof.example.toml`](crates/hof_config/hof.example.toml).
- **Commands.** `[commands.<name>]` runs a program in a tab, a split or the
  background, or types text into the focused pane, at a key you choose.
- **Hooks.** Shell snippets run on pane-opened, pane-closed,
  command-started, command-finished, bell, notification, directory-changed,
  title-changed and agent-changed.
- **Panes that know their agents.** Hof sees Claude Code, Codex, Gemini,
  Aider and opencode in a pane, and whether each is working, waiting for
  you or idle. It tells by the program in the foreground and the bottom of
  its screen (`[agents.<name>]` tables), or by the agent saying so with
  `hof ctl agent-state` from its hooks. A waiting pane gets a frame, its
  tab a dot, and a desktop notification when you can't see it
  (`[notify]`). `next-waiting` jumps to it.
- **Plugins.** A plugin is `plugins/<name>.toml` or
  `plugins/<name>/plugin.toml` beside the config, holding commands and
  hooks. Your own config wins any conflict.
- **`hof ctl`** drives the running Hof: list panes, read a screen or its
  scrollback, send text, run a command and wait for its exit code, open
  tabs and splits, focus or close panes, and say or wait for an agent's
  state. Inside a pane it needs no arguments, because Hof sets
  `HOF_SOCKET` and `HOF_PANE_ID` there.
- **The clipboard from programs.** Programs may put text on the clipboard
  with OSC 52. When one asks to read it, Hof asks you first, as
  `clipboard-read` says. `hof copy`, `hof paste` and `hof open URL|FILE` do
  the same from the shell. They speak to the pane's terminal, so they work
  in remote panes too.
- **Links.** A program's own links (OSC 8) and URLs in the text are
  underlined under the pointer, which names them in the pane's corner.
  Ctrl+click (Cmd+click on a Mac) opens one. A URL that wraps onto the next row opens whole.
- **MCP.** `hof mcp` is a stdio MCP server for the running Hof. Agents can
  read panes and agents' states, and say or wait for those states, always;
  typing into panes and opening or closing them are separate opt-ins in
  `[mcp]`.
- **Optional session restore.** `[session] restore = true` reopens the
  windows, tabs, splits and directories, and optionally the scrollback.
- **Themes.** Nine are built in, and you can add your own as
  `themes/<name>.toml`.

## Building

You need [mise](https://mise.jdx.dev). It installs Rust and Zig;
libghostty-vt's build script needs Zig to build Ghostty's parser.

```sh
mise install          # Rust and Zig, at the versions mise.toml pins
mise setup            # links your Ljós UI checkout ($LJOS_UI_DIR) into third_party/,
                      # and fetches the dependencies
mise hof              # builds Hof and opens a window; `mise hof --dropdown`
```

| task                     | what it does                                        |
|--------------------------|-----------------------------------------------------|
| `mise hof [ARGS]`        | builds and runs Hof, with ARGS                      |
| `mise build`             | an optimised build, `target/release/hof`            |
| `mise test [ARGS]`       | every test, or what ARGS pick (`mise test -p hof_vt`) |
| `mise check`             | what a change must pass: fmt, clippy and every test |
| `mise check-macos`       | that it compiles for macOS, checked from Linux      |
| `mise types`             | regenerates the QML types (see below)               |
| `mise fire [SECONDS]`    | the stress test (see below)                         |
| `mise agents`            | the static agents for remote machines               |
| `mise remote-test [IMG]` | bootstrapping a container over ssh, with Docker     |

`mise <task>` is short for `mise run <task>`.

The QML is compiled ahead of time against generated types. After changing a
Rust type that QML can see, regenerate them:

```sh
mise types            # LJOS_TYPES=write cargo test -p hof --test types
```

Remote machines with no Hof get a static, agent-only build. Build it,
for each musl target rustup has, and try it on a container reached over
ssh:

```sh
mise agents                      # target/release/hof-agent-<arch>-linux
mise remote-test alpine:3.22     # Docker; debian:bookworm-slim by default
```

A release of Hof ships those agents beside `hof`, or in `../lib/hof`.

## The stress test

`mise fire` builds a fire animation: 200×60 cells of 24-bit colour,
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
the clipboard and files across. See [`docs/remote.md`](docs/remote.md).

Remote panes work now. Name a machine under `[hosts]`:

```toml
[hosts.dev]
ssh = "dev.example.com"
```

Then Ctrl+Shift+H (Cmd+Shift+H on a Mac), or `hof ctl tab --host dev`, opens a tab there, and a
split of that tab stays on `dev`. A machine with no Hof gets a small,
static `hof` agent, copied over ssh the first time. When the connection drops, the pane says it is
reconnecting and picks up where it was. When Hof quits with
`session.restore` on, the remote programs keep running and are attached
again at the next start. `hof copy`, `hof paste` and `hof open URL` there
reach the clipboard and the browser where you sit. With `x11 = true` and
`sound = true` on the host, programs there open their windows on your
display and play through your speakers, as with `ssh -X`, carried over
Hof's own connection.

## Not yet

- Opening a remote machine's `file:` links, which waits for its files to
  be reached from here.

## Licence

MIT. See [LICENSE](LICENSE).
