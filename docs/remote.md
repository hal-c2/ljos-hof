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

### This machine's panes, under the agent here

With `[session] restore` and `keep-running` on, a pane of this machine
is a remote pane too. Its host is the agent here, reached at its socket
(`$XDG_RUNTIME_DIR/hof-agent.sock`) with no ssh. So reattaching, the
saved `remotePane` and detaching on quit work as they do for any host,
and closing Hof leaves the programs running.

- `hof_remote::client::Local` is the transport. When nothing answers at
  the socket, it starts `hof agent --detach`, in a `systemd-run --user
  --scope` of its own where systemd runs the session, else as a plain
  child that leaves Hof's session. Either way the agent keeps Hof's
  environment.
- The program is started as Hof would start it itself:
  - with Hof's variables (but `TERM` and its kin), then the pane's own;
  - in Hof's directory when the pane names none.
- The pane's `host` stays empty. Nothing keyed by a host's name reaches
  the agent here, and its files are never mounted or copied: a file it
  opens is a file here.
- Its directory, for a shell that never says, is where its foreground
  process works, as the system says.
- The window that keeps a session listens for `hof ctl` at
  `hof-session.sock` (`hof-session-NAME.sock` for a named one), where
  the next window to keep it listens too. So a program that ran on still
  reaches the Hof that attached it.
- A window holds its session with a lock, `hof-session[-NAME].lock` in
  the runtime directory, for as long as it runs. A plain `hof` takes the
  first of `default`, `2`, `3` … that no other Hof holds, and `hof attach
  NAME` takes `NAME` or says another Hof has it. So two windows never
  attach to one program or save over each other's session.
- A named session is kept in `sessions/NAME/` beside `session.json`,
  with its own `scrollback/`. Closing every pane of it removes that
  directory.
- Not yet:
  - A program asking the terminal's colours (OSC 10/11) is answered by
    the agent, which knows only the default theme. The same is true on
    any host.
  - The pane's `HOF_OPEN_KEY` is new at each start, so `hof open` in a
    program that ran on is asked about like printed text.
  - An agent left by an older Hof runs on until its panes close. A new
    agent can't take over its programs yet.
- `hof forget NAME` ends what a session kept running and forgets it. It
  attaches to each of its panes, here and on its hosts, and closes the
  ones an agent still has. It never starts the agent here, which would
  have nothing of the session's, and it won't touch a session a Hof has
  open.

### A session kept on another machine

`hof attach --host desktop work` opens the session `work` that is kept
on `desktop`, a `[hosts]` name. `hof sessions --host desktop` lists that
machine's sessions, and `hof forget --host desktop work` ends one's
programs and forgets it there.

To keep them there always, set it in the config or in Settings
(Window → Session):

```toml
[session]
host = "desktop"   # a plain `hof` opens its session on desktop
name = "mine"      # this machine's, not `default`

[hosts.desktop]
session = "work"   # desktop's: each host may name its own
```

A plain `hof` then opens `work` on `desktop` as `hof attach --host
desktop work` would. A host that names no session of its own takes
`[session] name`. Settings has the name beside the host it is kept on,
and on each host's page. A second window, while one has it open, keeps the
first of `default`, `2`, `3` there that none has. `hof attach`, `hof
sessions` and `hof forget` take that host too; `--here` is for this
machine's. A config shared by both machines can name either: on
`desktop` itself (its short host name) the sessions are its own, and
the session is the one `[hosts.desktop]` names. When the
host can't be reached in 30 seconds, the window opens with a session of
this machine's, and says why on its standard error.

- The agent there serves a `sessions` channel
  (`hof_session::service`): one JSON object a line each way, to list,
  hold, save and forget. The scrollback the file keeps apart is carried
  with the session.
- The agent holds the session, with the same lock a window there takes,
  for as long as the channel lasts. So a session open on one machine is
  open on the other too, and a Hof on either says so.
- The connection is kept up for the channel while the window runs
  (`Host::keep_connected`), though no pane may want it yet.
- A pane's host is as the machine it is saved on names it. A pane of
  `desktop`'s own (no host there) is `desktop` here; a pane here is
  saved as running on this machine's name. Hosts both know by the same
  name stay as they are. So `hof attach work` back on `desktop` opens it
  as it was left.
- New tabs, and the first pane of a session not saved yet, run on
  `desktop`. A split runs where the pane it splits does.
- Each save as Hof runs is sent and not waited for; a newer one takes the
  place of one not sent yet. The last, as Hof goes, is waited for a few
  seconds. Only if it was kept are the programs left running.
- When the connection drops, the agent there sees the channel end and
  lets the session go. The next save holds it again on a new channel.
- The window keeps no `hof ctl` socket for it: the name here may be a
  session of this machine's.
- Not yet: a session another Hof takes in the moment the connection was
  down stays that one's, and this window's saves are refused with a
  warning.

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
| files N   | 9P2000.L: the remote machine's files, which Hof asks for  |
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
- **Taking turns.** Each end sends in two lanes. Keystrokes, resizes,
  pings, pane output and acknowledgements go first. What the channels
  carry goes after, 64 KiB at a time. A keystroke or a pane's output waits
  for one piece of a file or an X11 image, not for everything queued
  ahead of it. A paste goes in 64 KiB pieces, so it neither passes a
  frame's limit nor holds up the keys typed after it.
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
  - Each pane's program gets `HOF_OPEN_KEY`, random and new per pane, and
    `hof open` puts it in the request (`ESC ] 7702 ; key=KEY ; URL ST`).
    A request with the pane's key is the user's own and is not asked
    about; one without it, or with another key, is a program's and is.
    `open-urls = "deny"` refuses both. A remote pane attached again after
    Hof restarts keeps the key it started with, which Hof no longer
    knows, so there it asks.
  - `hof open FILE` in a local pane opens the file. In a remote pane, as
    for a click on a program's `file:` link there, Hof copies the file
    here over a files channel and opens the copy with the local handler.
    The pane shows "Opening … from <host>…" meanwhile, and why, if it
    could not.
  - `hof open DIR` in a remote pane opens the folder where that machine's
    files are mounted here (below). Where they can't be (off Linux, or
    with no FUSE), it copies the folder here as it is then, level by level, at most 2000 files and 256 MiB; what is past that,
    links, and anything not a file or folder are left out, and the pane
    says how many. The copy's files are read-only and not watched. Copied
    again, what went there goes here too. `hof open FILE` on one of its
    files makes that one a watched, writable copy like any other.
  - The copy goes in the cache, at its path there:
    `$XDG_CACHE_HOME/hof/hosts/<name>/<path>`, else `~/.cache/hof/...`, or
    `~/Library/Caches/hof/...` on a Mac. Copies are readable by this user
    only, and never executable. A path with `..` in it, or a relative one,
    is refused.
  - While a copy is watched, a save here is written back once it has been
    the same for half a second, and the file there is looked at every
    three seconds and copied again when it changed. When both changed,
    the save here wins. A copy is watched until it is deleted here, a day
    after it last changed, or Hof quits, 64 at most.
  - The files channel is the first Hof opens itself: its `Connect` goes
    from Hof to the agent, with a channel number that has the top bit set
    so it never meets one the agent picked. Each channel carries one
    9P2000.L session. The agent serves its user's files on it, as that
    user, with nothing outside what that user may reach. An older agent
    hangs up a channel it doesn't know, and Hof says to update Hof there.
  - The 9P2000.L codec, client and server are Hof's own
    (`hof_remote::files`), no more than this needs: walk, open, create,
    read, write, readdir, getattr and setattr, mkdir, rename, unlink,
    readlink, fsync and statfs.
  - **The mount** (Linux): the remote machine's whole tree at
    `$XDG_RUNTIME_DIR/hof/hosts/<name>`, made the first time a folder
    there is opened (a file opened is copied, and mounts nothing), or
    as soon as the host is reached with `files = true`. It is a FUSE file system served in Hof, through
    `fusermount3` with no libfuse (`fuser`, its pure-Rust mount), and
    every request on it is a 9P request on a files channel of its own.
    Nothing is copied: the page cache is dropped each time a file is
    opened, and what the kernel knows of a file is trusted for a second.
  - Each file is known by its path there. A file moved through the
    mount is known by its new path, and so is everything in it; one
    moved there is, here, a file gone and another made.
  - Every file shows as this user's; what may be done with it is what
    the agent's user may do there, which the agent answers. Changing
    whose a file is, links (`ln`, `ln -s`), devices and
    `rename(RENAME_NOREPLACE)` are refused.
  - When the channel goes, the next request opens another, and files
    open through the mount are opened again on it. While the files
    can't be reached, requests fail at once with `EIO` for five
    seconds before Hof tries again.
  - Hof unmounts when it quits, or when the host's config changes,
    lazily: a program still in it keeps what it has open. A mount left
    by a Hof that ended is taken away the next time; one a running Hof
    has there is used as it is.
  - A mount is the other machine's own files, so `rm -r` in it removes
    them there, and so does anything that walks into it removing a tree
    (`std::fs::remove_dir_all` and `TempDir` do not stop at a mount).
    A session can be attached at a folder rather than `/` (`aname`):
    the agent then walks no higher than it, `..` included. The tests
    mount only a folder they made, and unmount before it is removed.
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
       With no X server here at all, it is told what to install (on a
       Mac, XQuartz or yserver).
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
   `hof open URL`, and `hof open FILE` and `file:` links in remote panes,
   through the files channel.
7. **The X11 tunnel**, then **sound**. Done: carried connections with
   their own flow control, the display and sound sockets in the agent,
   and the cookies put in by Hof. Tested against stand-in servers, and
   with `xdpyinfo` and `pactl` against a real desktop (an ignored test).
8. **Files over 9P**, then the FUSE mount. Done: the channel, the
   agent's server, copies kept in sync, and the mount on Linux. Tested
   through the kernel against a real agent.
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
