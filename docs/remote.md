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
- **Bootstrapping:** if the remote machine has no `hof`, or one of another
  protocol version, Hof copies a static build of itself over the same ssh
  connection into `~/.local/share/hof/bin/`.

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
protocol. By default that is `ssh -T <ssh> -- <hof> agent --stdio`, where
`ssh` is the host's `ssh` key (or its name) and `hof` is the remote
`hof`'s path. `command` replaces the whole thing, for a container, a
serial line, or a test:

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
       X server or Xwayland.
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
  - Hof rewrites the client's first packet, its authorization. It puts in
    the local cookie, if there is one, and takes out the offer to share
    memory, which two machines can't. The sound then travels in the
    packets.

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
   - the host picker, for a tab or a split.

   Still to do: bootstrapping.
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
- **The remote binary.** `hof` links the GUI libraries, which a server may
  not have. Bootstrapping a static, agent-only build answers that.
- **Probing for a running agent** connects to it. That connection counts
  as one while it lasts, which is why the agent waits a while before it
  goes, rather than going the moment it is idle.
