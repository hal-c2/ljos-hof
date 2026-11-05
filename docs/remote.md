# Remote panes, and panes that know their agents

Where Hof is going. Two goals:

- **A pane can live on any machine.** Tabs and splits can mix machines. A
  dropped connection, a laptop lid or a new network loses nothing.
  Programs on the remote machine can open X11 windows, play sound, use
  the clipboard and open files on the machine you are sitting at, as if
  they ran there.
- **Hof knows which panes run agents** (Claude Code, Codex and the like),
  and whether each one is working, waiting for you or done. That holds on
  every machine.

## The shape of it

```
 your machine                                  a remote machine
 ┌───────────────────────────────┐   ssh     ┌──────────────────────────────┐
 │ hof (the window)              │  stdio    │ hof agent --stdio  (bridge)  │
 │  pane ── Terminal (external)  │◄─────────►│        │                     │
 │  pane ── local Session        │  one mux  │        ▼                     │
 │                               │ channels: │ hof agent  (daemon, outlives │
 │  forwards: X11, sound,        │  panes    │   the connection)            │
 │   clipboard, files, open      │  control  │  pane ── Session (pty + vt)  │
 └───────────────────────────────┘  x11 ...  │  pane ── Session             │
                                             └──────────────────────────────┘
```

### The agent on the remote machine

- **`hof agent` is the Hof binary, run on the remote machine** (the
  `hof_agent` crate). It owns that machine's ptys, through the same
  `hof_vt::Session` a local pane uses, so it keeps the screen's whole
  state: it parses what it forwards.
- **It answers the programs' queries itself** (cursor position, device
  attributes, colours), next to the program, so a query never waits a
  round trip or goes unanswered while the connection is down.
- **It is a daemon on a unix socket**, `$HOF_AGENT_SOCKET`, else
  `$XDG_RUNTIME_DIR/hof-agent.sock`. It is made mode 0600, under a umask,
  so only its user can ever connect. It outlives any connection, and goes
  once it has had no pane and no connection for ten seconds. A socket left
  by an agent that died is taken over; one an agent still answers on is
  not.
- **`hof agent --stdio`**, which is what ssh runs, only bridges ssh's stdio
  to that socket. If no agent answers, it starts one with
  `hof agent --detach`, in a session of its own so the ssh session's end
  does not end it. `hof mcp` bridges the same way.
- **Each agent has an instance number**, sent in `Hello`. A client that
  finds a different one knows the agent restarted, and its pane numbers
  mean nothing now; a pane the agent doesn't have is answered with `Gone`.
- **One connection watches a pane at a time.** Attaching from another
  takes it over, and the first is sent `Detach`.
- **Bootstrapping.** A machine with no Hof gets one, copied over ssh:
  - The command ssh runs there is one `sh -c '...'`, with no quote or
    backslash inside, so any login shell runs it. It runs the agent Hof
    copied for this version and wire, in
    `${XDG_DATA_HOME:-~/.local/share}/hof/<version>-<wire>/hof`, else the
    host's own `hof`. With neither, it says
    `Hof is not on this machine (Linux x86_64)` and exits 127.
  - The connection after that copies `hof-agent-<arch>-<os>` there,
    through a second ssh command. The copy is a static, agent-only build
    of a few MiB: `hof agent`, `copy`, `paste` and `open`, with no GUI
    libraries. Hof finds it in `$HOF_AGENT_DIR`, beside `hof`, in
    `../lib/hof`, or in `~/.local/share/hof/agents`.
  - The copy runs `--version` there before it takes its place, so one
    that can't run there never does. After three failures Hof stops
    copying and says why.
  - The agent puts its own directory last on its panes' `PATH`, so they
    find `hof copy` and the rest. A login shell's profile may set a
    `PATH` of its own, so the copy also links `~/.local/bin/hof` to
    itself, unless something else is there.
  - `bootstrap = false` on a host turns this off, and `command` does too.

### The local side of a remote pane

A remote pane is a `Terminal` whose `host` names a `[hosts]` entry:

- Its session is a `hof_vt::Session::remote`. The connection's thread
  writes the agent's bytes straight into the session's parser, through a
  `Feed`, without passing through QML.
- What the user types, pastes and resizes leaves through an `Outlet`,
  already encoded by the local libghostty.
- The local copy does not answer queries, since the agent already has.
- `remotePane` is the agent's number for the program. A restored pane
  sets it and attaches; if the agent no longer has that pane, a new
  program starts under the screen the old one left.
- `connection` (`connecting`, `up`, `down`, `gone`) and
  `connectionMessage` say how the link is. The pane shows a banner while
  it reconnects, and keeps its screen.
- Every pane on one host shares one connection (`hof_remote::client`),
  which the first pane on that host opens.
- When Hof quits with a session to restore, remote panes detach rather
  than close, so their programs run on.

Search, selection, links, shell integration, hooks and painting are the
same code a local pane runs.

### Reaching a host

A host is reached through any command whose stdin and stdout speak the
protocol. By default that is `ssh -T <ssh-options> <ssh>`, then the
script that runs the agent there (see bootstrapping, above). `ssh` is the
host's `ssh` key (or its name), `ssh-options` are more words for ssh,
and `hof` is the remote `hof`'s path, if it has one. `command` replaces
the whole thing, for a container, a serial line, or a test, and copies
nothing:

```toml
[hosts.box]
command = ["podman", "exec", "-i", "box", "hof", "agent", "--stdio"]
```

### The connection: resumable streams over ssh

- **Every pane's output is a numbered byte stream.** The agent keeps the last
  few MiB of each stream.
- **On reconnect, the client says how far it got.** The agent sends the rest,
  or, if that has been dropped, a `Session::snapshot()` (the screen and
  scrollback as VT sequences, which Hof already uses for session restore)
  and then the live stream.
- **Input is numbered too**, so a keystroke is never typed twice or lost.

That gives the mosh experience for drops, suspend and roaming, using ssh's
authentication, `~/.ssh/config`, jump hosts and agent forwarding as they
are. Each reconnect pays for an ssh handshake.

Later there can be a UDP transport (QUIC), with keys handed over through
ssh. That makes roaming instant and removes head-of-line blocking. Typing
over slow links can get local echo, predicted from the local copy of the
screen.

### One connection, many channels

The ssh connection carries length-prefixed frames, each tagged with a
channel:

| channel   | carries                                                    |
|-----------|------------------------------------------------------------|
| control   | open, close and resize panes, hello and resume, version    |
| pane N    | output from the agent, input to the agent, and what runs   |
|           | in the pane's foreground                                   |
| files     | 9P: the remote machine's files, served by the `p9` crate   |
| carried N | one connection the agent passes on: an X11 client, or a    |
|           | PulseAudio native-protocol client                          |

A carried connection works like this:

- **Offering.** After the hello, Hof sends `Offer` with the services it
  can serve: `x11`, `sound`, or both. The agent starts listening for each
  the first time one is offered. Panes opened while a connection offers
  them get `DISPLAY`, `XAUTHORITY` and `PULSE_SERVER`. The pane's own
  environment, sent by Hof, still wins.
- **Connecting.** When a program there connects, the agent picks a
  channel number and sends `Connect`, with the service and what the
  program opened with. It goes to the newest connection that offers the
  service. Hof connects to the server here and answers with data, or
  with `Hangup`.
- **Flowing.** `Data` carries the bytes both ways, in pieces of 64 KiB.
  Each end acknowledges what it has written with `Ack`, and stops reading
  from its socket while 1 MiB is unacknowledged. A program that floods
  the display can't starve the panes' output.
- **Ending.** `Hangup` closes the channel once what was sent before it
  has been written. When the connection drops, every channel on it is
  closed, and the programs see their server go, as with `ssh -X`.

Each channel past control is switched on per host, in config, and some
need a prompt:

```toml
[hosts.dev]
ssh = "dev.example.com"        # anything `ssh` takes; empty means the name
hof = "~/.local/bin/hof"       # the remote `hof`, if it is not on the PATH
ssh-options = ["-p", "2222"]   # more for ssh, before the destination
bootstrap = true               # with no Hof there, copy Hof's agent over
x11 = true
sound = true
files = true
clipboard = "ask"              # ask, allow or deny: the remote machine reading yours
```

### Forwarding, piece by piece

- **Clipboard:**
  - OSC 52 from any pane, local or remote, is parsed locally. Writes are
    allowed unless `terminal.clipboard-write` is off. A read asks the
    user, unless `terminal.clipboard-read` (or, for a remote pane, its
    host's `clipboard`) says to allow or deny it. The answer goes back as
    the program's input, through the agent for a remote pane.
  - `hof copy` and `hof paste` write OSC 52 to `/dev/tty`. They need no
    channel of their own: the sequences travel in the pane's output, so
    they work in local and remote panes alike, and in other terminals
    that take OSC 52.
  - X11 clients on the remote machine get the clipboard through the X
    tunnel.
- **Open here:**
  - `hof open URL` writes Hof's OSC 7702 to `/dev/tty`, and Hof opens the
    URL locally, asking first unless `terminal.open-urls` says otherwise.
  - `hof open FILE` in a local pane opens the file. In a remote pane it is
    not opened yet. It should fetch the file into a local cache, open it
    with the local handler, and write it back when it changes.
  - Once the files channel is there, the remote machine's tree is mounted
    at `~/.cache/hof/hosts/<name>`. That needs a local FUSE 9P client of our
    own; `p9` is server-only.
  - The `p9` server runs in the agent and reads frames off the channel
    directly.
- **X11, in two steps:**
  1. **A tunnel, like `ssh -X` but over our mux.** Done.
     - The agent takes the first display from `:10` that has no lock
       file and no socket. It listens on the abstract socket
       `@/tmp/.X11-unix/X<n>` on Linux, or on TCP port `6000+n`
       elsewhere.
     - It writes an Xauthority file with a cookie of its own into a
       private directory, and remote panes get `XAUTHORITY` for it. A
       client that doesn't bring that cookie is refused there, before
       anything reaches Hof.
     - Hof takes the agent's cookie out of the client's setup, and puts
       in the local display's own, read from `$XAUTHORITY` or
       `~/.Xauthority` for each connection. Then it connects to the local
       X server or Xwayland. A `DISPLAY` from macOS's launchd
       (`/private/tmp/com.apple.launchd.…/org.xquartz:0`) names its socket
       whole, `:0` and all.
     - Where yserver is installed (on a Mac, or named by `HOF_YSERVER`),
       Hof uses it instead: it starts
       `yserver --rootless :N -auth FILE -terminate` the first time a
       window opens, with a cookie of its own in that file, and each X
       window becomes a window of its own here. yserver ends with its last
       window, and is started again for the next.
     - When the server here can't be reached, or yserver won't start, the
       client there is refused with the reason (`Hof: no X server on …`),
       so the program prints why rather than only "Can't open display".
     - Apps die when the connection drops, as with `ssh -X`.
  2. **Apps that survive a dropped connection, as with xpra.**
     [yserver](https://github.com/joske/yserver)'s core is backend-agnostic:
     `yserver-core` drives a `dyn Backend`, and its rendering lives in
     `yserver-render`.
     - Run it headless on the remote machine, with a backend that ships each
       top-level window's damage, compressed, to Hof.
     - Hof shows each window as a window of its own, rootless, as yserver's
       macOS mode already does with Mac windows.
     - Input goes back the same way, so the apps keep running while the
       connection is down.
     - This needs yserver to export drawables. That is work to do upstream,
       not to copy.
- **Sound:** done.
  - Remote panes get `PULSE_SERVER` for a socket of the agent's, in its
    private directory.
  - Each connection is relayed to the local PulseAudio server:
    `$PULSE_SERVER` if it names a socket, else
    `$XDG_RUNTIME_DIR/pulse/native`. pipewire-pulse speaks the same
    protocol.
  - With no server running here, as on a Mac, Hof serves the connection
    itself (`hof_remote::sound`). It speaks enough of PulseAudio's native
    protocol for programs to play: one sink, `hof`, with its volume and
    mute; playback streams in any of PulseAudio's sample formats, rates
    and channel maps, mixed to stereo and resampled to the output's rate;
    corking, flushing, draining and latency. It asks each stream for what
    its buffer (300 ms by default) is short of, as PulseAudio does.
    Recording, uploaded samples and modules are not served. On a Mac it
    plays through CoreAudio's default output (with cpal), opened when
    something first sounds and closed after ten quiet seconds, and
    following the output the user picks. Nothing needs installing.
  - When relaying, Hof rewrites the client's first packet, its
    authorization. It puts in the local cookie, if there is one, and
    takes out the offer to share memory, which two machines can't. The
    sound then travels in the packets.

### Mixing machines in tabs and splits

- **Model:**
  - Every pane gets a `host`: `None` for this machine, or a name from
    `[hosts]`.
  - Session restore saves the host, and on restore remote panes reattach to
    their agent. Their programs never stopped, unlike local ones.
- **Doing it:**
  - New tab on a host, and split on a host, both from the keyboard:
    `new-tab-on-host` (Ctrl+Shift+H), `split-right-on-host` and
    `split-down-on-host` show a list of this machine and the `[hosts]`,
    which typing narrows.
  - A split inherits its pane's host by default.
  - `hof ctl tab --host dev`, and `split --host`.
- **Seeing it:**
  - A host badge on the pane and the tab.
  - A pane whose connection is down shows that it is reconnecting, and
    keeps its screen.
- `hof ctl` and MCP list panes with their host. An agent orchestrating
  others sees every machine.

## Panes that know their agents

The idea comes from herdr. What follows is Hof's own design.

- **States:** each pane has an agent state: `none`, `working`, `waiting`
  (it wants you), or `idle` (done).
  - Tabs, windows and the dropdown roll these up, so a tab with a waiting
    agent shows it.
- **Telling the states, by three means:**
  1. **The foreground process.** `tcgetpgrp` on the pty, then
     `/proc/<pid>/comm` and the arguments, gives the agent's name. For a
     remote pane the agent daemon sends it (`Foreground`) on attach, and
     when it has changed by the next output, ahead of that output, so the
     tables work there too.
  2. **The screen.** A plugin can add an `[agents.<name>]` table with the
     process it matches and the text that means waiting or working. The
     screen is already parsed, and only its bottom rows need reading when
     it changes. Hof ships tables for the common agents.
  3. **The agent saying so.** `ESC ] 7701 ; <state> [; <name>] BEL`, which
     `hof ctl agent-state` writes to the terminal, so it reaches Hof from
     a remote pane too; MCP `set_agent_state`; or the `reportAgent`
     method from QML. What is said holds while the program that was in
     the foreground then still is, and wins over the screen. With no
     foreground to see, as for a remote pane, it holds until the next
     report. Claude Code's hooks can call `hof ctl agent-state`.
- **Using them:**
  - A hook, `agent-changed`, with `HOF_AGENT` and `HOF_AGENT_STATE`.
  - A desktop notification when an agent starts waiting and its pane is
    out of sight.
  - `hof ctl wait-agent --state idle`, and MCP `wait_for_agent`, so
    scripts and agents can chain agents.
  - The states in MCP `list_panes`.
  - A key, `next-waiting`, that jumps to the next waiting agent, on any
    machine.
- **Looking costs little.** A pane looks a moment (300 ms) after its
  screen changes, and only reads the screen when a table's process is in
  the foreground. A quiet pane costs nothing.

## Order of work

1. **Hosts in the pane model.** Done: `[hosts]` in config, and `host` and
   the agent's pane number in the saved session. A split inherits its
   pane's host.
2. **`hof_remote`: the wire and the resumable stream.** Done: framing,
   numbered streams, the backlog, snapshot fallback. Pure and testable,
   with no ssh.
3. **`hof agent`.** Done: the daemon and the `--stdio` bridge, panes on
   the remote machine, resume, takeover. Tested over socket pairs and a
   socket in a temporary directory.
4. **The client and the ssh transport.** Done:
   - the connection in Hof, with reconnect and backoff;
   - a remote pane as a `Terminal` with a `host`;
   - the reconnecting banner and host badges;
   - `hof ctl tab --host` and `split --host`, and the same in MCP;
   - the host picker, for a tab or a split;
   - bootstrapping: copying the agent to a machine with no Hof. Tested
     in containers of Debian, Ubuntu and Alpine with
     `mise remote-test [image]`.
5. **Agent awareness.** Done locally: the foreground process, the
   tables, the escape sequence, the frame, badge and tab dot, the hook,
   notifications, `next-waiting`, `hof ctl agent-state` and `wait-agent`,
   and the MCP tools. Remote panes carry the escape sequence, and the
   agent daemon sends each pane's foreground process, so the same means
   work on any machine.
6. **Clipboard and open-here.** Done: OSC 52 writes, and reads behind a
   prompt, in local and remote panes; `hof copy`, `hof paste` and
   `hof open URL`. Still to do: opening a remote machine's files, which
   waits for the files channel.
7. **The X11 tunnel**, then **sound**. Done: carried connections with
   their own flow control, the display and sound sockets in the agent,
   and the cookies put in by Hof. Tested against stand-in servers, and
   with `xdpyinfo` and `pactl` against a real desktop (an ignored test).
8. **Files over 9P**, then the FUSE mount.
9. **X11 apps that survive a dropped connection**, built on yserver, and the
   **UDP transport**.

## Known gaps

- **The theme.** The agent's copy of a pane answers colour queries with
  its default theme, not the one the user sees. `Open` should carry the
  client's theme, and a theme change should be sent on.
- **Large snapshots.** A snapshot is one frame, and a big scrollback can
  pass the frame limit. It should be sent in pieces, or cut to fit.
- **Agents for other machines.** `mise agents` builds one for each
  musl target rustup has, which is x86_64 here. aarch64 needs
  `rustup target add aarch64-unknown-linux-musl`, and macOS needs a
  build on a Mac.
- **Login shells on Alpine** and others whose profile neither keeps the
  agent's `PATH` nor has `~/.local/bin` on it don't find `hof` by name.
- **Old copies stay.** A new version copies itself beside the old one,
  which nothing removes.
- **Probing for a running agent** connects to it. That connection counts
  as one while it lasts, which is why the agent waits a while before it
  goes, rather than going the moment it is idle.
