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

- **Tabs and splits**, each of which can be zoomed and resized. The panes
  of a split are framed, the focused one in a highlight colour; how thick
  the line is and how round its corners are is in the settings. Search,
  and jumping between prompts when the shell has shell integration.
- **A dropdown.** `hof --dropdown` slides in from the top or bottom edge,
  and `hof --toggle` shows or hides it (bind that in your desktop).
  `hof --tab -- cmd` opens a tab in it.
- **Keys as each desktop has them.** On a Mac, Command: Cmd+T, Cmd+W,
  Cmd+D, Cmd+1 to 9, Cmd+C and Cmd+V, and every Ctrl and Option chord is
  the terminal's. Elsewhere Ctrl+Shift: Ctrl+Shift+T, Ctrl+Shift+W, Alt+1
  to 9, Ctrl+Shift+C, so plain Ctrl chords (fzf's Ctrl+T, say) reach the
  shell. Any of it can be rebound in `[keys]`.
- **Menus with the keys beside them.** On a Mac the actions are in the
  menu bar. Elsewhere they're behind a button at the right of the tab bar,
  and a right click in a pane opens a menu of its own (unless the program
  there has the mouse; Shift+right click then). `[window] menu` chooses
  the bar, the button or neither.
- **A settings window** (Ctrl+Shift+, or Cmd+,) for the theme, font,
  opacity, the frames round split panes, cursor, clipboard, menu and
  session, and the remote hosts: add one, set its address, user and port,
  what it may share, or remove it.
  Each change is written into the config file at once, keeping your
  comments, and applies straight away. The controls take their colours
  from the terminal's theme, and the test suite holds the settings, the
  menus and the host picker to the Ljós design audit (contrast, names,
  target sizes, text that is cut off).
- **One TOML file** where every key is optional. A key Hof doesn't know is
  a warning, not an error. Saving the file applies it without a restart.
  See [`hof.example.toml`](crates/hof_config/hof.example.toml).
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
  `clipboard-read` says. `hof copy`, `hof paste` and `hof open URL|FILE|DIR`
  do the same from the shell. They speak to the pane's terminal, so they
  work in remote panes too. A program asking to open something is asked
  about, as `open-urls` says; your own `hof open` is not.
- **Links.** A program's own links (OSC 8) and URLs in the text are
  underlined under the pointer, which names them in the pane's corner.
  Ctrl+click (Cmd+click on a Mac) opens one. A URL that wraps onto the next row opens whole.
- **MCP.** `hof mcp` is a stdio MCP server for the running Hof. Agents can
  read panes and agents' states, and say or wait for those states, always;
  typing into panes and opening or closing them are separate opt-ins in
  `[mcp]`.
- **Optional session restore.** `[session] restore = true` reopens the
  windows, tabs, splits and directories, and optionally the scrollback.
  The session is kept as Hof runs: a moment after the tabs and splits
  change, and every half minute. So Hof ended by Ctrl+C, a signal or a
  crash opens again as it was. A shell that never says where it is (bash
  sends no OSC 7) is found where the system says it works. Closing every
  pane forgets the session.
- **Programs that outlive Hof.** With `restore` on, each pane's program
  runs under `hof agent` on this machine, not under Hof itself
  (`[session] keep-running`, on by default). Closing Hof, or starting a
  new build of it, leaves the programs running; the next start attaches
  to them where they were, as tmux would. Hof starts the agent the first
  time a pane needs it, in a systemd user scope of its own where systemd
  runs the session. It is the same agent a Hof on another machine reaches
  over ssh, and it goes once it has no panes left.
- **Sessions by name.** `hof attach work` opens the session `work` as it
  was left, or starts it. `hof attach` alone opens the one saved last
  that no Hof has open, and `hof sessions` lists them. `hof forget work`
  ends the programs it kept running and forgets it. Only one Hof at a
  time has a session open: a second plain `hof` keeps a session of its
  own (`2`, then `3`), not the first one's. A window started for a
  command (`hof -e htop`) keeps none.
- **Sessions on other machines.** `hof attach --host desktop work`
  opens the session `work` kept on `desktop` (a `[hosts]` name), its
  programs still running there, and keeps it there as it changes. Back
  at `desktop`, `hof attach work` opens it as it was left. `hof sessions
  --host desktop` and `hof forget --host desktop work` do the same there.
  `[session] host = "desktop"`, or Window → Session in Settings, makes
  that where a plain `hof` keeps its session; `[hosts.desktop] session`
  names it there, and `[session] name` names this machine's.
  It needs this Hof on both machines.
- **Themes.** Nine are built in, and you can add your own as
  `themes/<name>.toml`.

## Getting started

Install [mise](https://mise.jdx.dev/getting-started.html) and
[rustup](https://rustup.rs), then:

```sh
mise run setup        # Rust and Zig at mise.toml's versions, with the targets the
                      # tasks build for; Ljós UI in third_party/ at the pinned
                      # revision; the crates; then what it skipped and what the
                      # system lacks
mise run dev          # builds Hof and opens a window; `mise run dev --dropdown`
mise run install      # puts hof on your PATH (cargo install; --root DIR, --agents)
```

Hof builds against a checkout of [Ljós UI](https://github.com/hal-c2/ljos-ui)
at `third_party/ljos-ui`, at the revision `third_party/ljos-ui.rev` names:
the one `Cargo.lock` was made against. `setup` links the checkout
`--ljos-ui DIR` names, else keeps the one there, else links `$LJOS_UI_DIR`
when that is set, and else clones that revision there (`--no-clone` fails
instead). It moves its own clone to the pinned revision; a linked checkout
is yours, so it only says when it differs, and `mise run check` fails until
it matches. To build against a newer Ljós UI, move the checkout, let cargo
update `Cargo.lock`, and commit both with the new revision in
`third_party/ljos-ui.rev`.

Zig is for libghostty-vt, whose build script builds Ghostty's parser. The
system must have a C compiler and pkg-config, and on Linux the ALSA and
fontconfig headers; `setup` says which are missing, and never installs them.

| task                            | what it does                                        |
|---------------------------------|-----------------------------------------------------|
| `mise run setup`                | the first run after cloning; safe to run again      |
| `mise run dev [ARGS]`           | builds and runs Hof, with ARGS                      |
| `mise run hof [ARGS]`           | the same as `dev`                                   |
| `mise run build [ARGS]`         | an optimised build, `target/release/hof`            |
| `mise run install`              | `cargo install` of hof; `--root DIR`, `--agents`, `--dry-run` |
| `mise run check`                | what a change must pass: the pinned Ljós UI, fmt and clippy |
| `mise run test [ARGS]`          | every test, or what ARGS pick (`mise run test -p hof_vt`) |
| `mise run fmt`                  | formats the code; `--check` only says whether it is |
| `mise run docs`                 | the API docs of Hof's crates, `target/doc/hof/index.html`; `--open` |
| `mise run check-macos`          | that it compiles for macOS, checked from Linux with Zig |
| `mise run types`                | regenerates the QML types (see below); `--check`    |
| `mise run fire [SECONDS]`       | the stress test (see below)                         |
| `mise run agents`               | the static agents for remote machines; `--only x86_64` |
| `mise run remote-test [IMG]`    | bootstrapping a container over ssh, with Docker     |
| `mise run tasks:smoke`          | runs every task quickly, as CI does; `--list`, `--only TASK` |

A change must pass `mise run check` and `mise run test`; CI runs both, then
`mise run tasks:smoke`. `mise run <task> --help` says what a task takes, and
`mise <task>` is short for `mise run <task>`, except for `install` and `fmt`,
which are mise's own commands. There is nothing to deploy: Hof is installed
from a checkout.

The QML is compiled ahead of time against generated types. After changing a
Rust type that QML can see, regenerate them:

```sh
mise types            # LJOS_TYPES=write cargo test -p hof --test types
mise types --check    # only says whether the committed types are current
```

Remote machines with no Hof get a static, agent-only build, for x86_64 and
aarch64 Linux with musl (`setup` adds both targets). Build them, and try one
on a container reached over ssh:

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
reach the clipboard and the browser where you sit. `hof open FILE` there,
or a click on a program's `file:` link, copies the file here and opens
the copy with what opens such files here. Each save goes back to the
machine it came from, and a change there is copied again. `hof open DIR`
there opens the folder itself: on Linux, Hof mounts that machine's files
at `$XDG_RUNTIME_DIR/hof/hosts/<name>` over the same connection, so what
you change in it changes there. Elsewhere, or with no FUSE, it opens a
read-only copy of the folder as it is then. `files = true` on the host
mounts them as soon as it is reached. Both ends need
this Hof; an older agent says to update it. With `x11 = true` and
`sound = true` on the host, programs there open their windows on your
display and play through your speakers, as with `ssh -X`, carried over
Hof's own connection. On a Mac they open through
[yserver](https://github.com/joske/yserver) when it is installed (on the
`PATH`, in Applications, or named by `HOF_YSERVER`): Hof starts it
rootless the first time a window opens, so each X window is a Mac window
of its own. Without yserver, the X server `$DISPLAY` names (XQuartz) is
used. With neither, programs there say what to install. Sound goes to the PulseAudio or PipeWire server running here. A
Mac has none, so Hof serves the sound itself and plays it through the
Mac's own output, the one chosen in its Sound settings. Nothing needs
installing on either end: programs there that play through PulseAudio
or PipeWire (most do, from `paplay` to `mpv` and browsers) just play.

## Licence

MIT. See [LICENSE](LICENSE).

The controls in `crates/hof/qml/Hof/Bricks` began as HAL-C2's. Their icons
are [Lucide](https://lucide.dev)'s, under the ISC licence.
